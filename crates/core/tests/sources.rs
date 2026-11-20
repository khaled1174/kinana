use kinana_core::{Result, sources};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self> {
        let root = std::env::temp_dir().canonicalize()?.join(format!(
            "kinana-sources-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root)?;
        Ok(Self(root))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _cleanup = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn local_folder_is_read_only_and_never_owned_by_cleanup() -> Result<()> {
    let temp = Scratch::new()?;
    let original = temp.0.join("user skill");
    let state = temp.0.join("not-created");
    fs::create_dir(&original)?;
    fs::write(original.join("SKILL.md"), "untrusted source data\n")?;
    fs::write(original.join("script.sh"), "#!/bin/sh\ntouch SCRIPT_RAN\n")?;
    let prepared = sources::prepare(&original.to_string_lossy(), &state)?;
    assert_eq!(prepared.root, original);
    assert_eq!(prepared.provenance.kind, "local");
    assert_eq!(prepared.provenance.found_in, vec![original.clone()]);
    assert_eq!(prepared.provenance.url, None);
    assert_eq!(prepared.label, "user skill");
    assert!(!state.exists());
    drop(prepared);
    assert_eq!(
        fs::read_to_string(original.join("SKILL.md"))?,
        "untrusted source data\n"
    );
    assert!(original.join("script.sh").is_file());
    assert!(!original.join("SCRIPT_RAN").exists());
    assert!(!state.exists());
    Ok(())
}

#[test]
fn local_files_and_parent_components_are_rejected_without_scratch() -> Result<()> {
    let temp = Scratch::new()?;
    let file = temp.0.join("data.txt");
    fs::write(&file, "data")?;
    let state = temp.0.join("not-created");
    assert!(sources::prepare(&file.to_string_lossy(), &state).is_err());
    let traversal = temp.0.join("../").join(
        temp.0
            .file_name()
            .ok_or_else(|| std::io::Error::other("missing temp name"))?,
    );
    assert!(sources::prepare(&traversal.to_string_lossy(), &state).is_err());
    assert!(!state.exists());
    assert_eq!(fs::read_to_string(file)?, "data");
    Ok(())
}

#[test]
fn malformed_remote_urls_are_rejected_before_creating_scratch() -> Result<()> {
    let temp = Scratch::new()?;
    let state = temp.0.join("not-created");
    for input in [
        "http://github.com/acme/skills",
        "ssh://github.com/acme/skills",
        "git@github.com:acme/skills.git",
        "file:///tmp/repository",
        "https://example.com/acme/skills",
        "https://github.com.attacker.invalid/acme/skills",
        "https://github.com:443/acme/skills",
        "https://user:secret@github.com/acme/skills",
        "https://github.com/acme/skills?x=y",
        "https://github.com/acme/skills#fragment",
        "https://github.com/acme/skills/../outside",
        "https://github.com/acme/skills/tree/main/../../outside",
        "https://github.com/acme/skills/tree/main/%2e%2e/outside",
        "https://github.com/acme/skills/tree/feature%2Fbranch/skill",
        "https://github.com/acme/skills/tree/main/.git",
        "https://github.com/acme/skills/tree/-branch/skill",
        "https://github.com/acme/skills/tree/main.lock/skill",
        "https://github.com/acme/skills/tree/main\\..\\outside",
        "https://github.com/acme/skills/tree/main//skill",
        "https://github.com/acme/.git",
        "https://github.com/acme/skills/tree",
    ] {
        assert!(
            sources::is_remote(input),
            "not identified as remote: {input}"
        );
        let error = sources::prepare(input, &state).expect_err(input);
        assert_eq!(error.code, "INVALID_SOURCE_URL", "{input}: {error}");
    }
    assert!(!sources::is_remote("./local-directory"));
    assert!(!state.exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn local_and_scratch_symlink_roots_are_rejected() -> Result<()> {
    use std::os::unix::fs::symlink;
    let temp = Scratch::new()?;
    let original = temp.0.join("original");
    fs::create_dir(&original)?;
    fs::write(original.join("KEEP"), "owned by user")?;
    let link = temp.0.join("link");
    symlink(&original, &link)?;
    assert!(sources::prepare(&link.to_string_lossy(), &temp.0.join("scratch")).is_err());
    let descendant = original.join("nested");
    fs::create_dir(&descendant)?;
    assert!(
        sources::prepare(
            &link.join("nested").to_string_lossy(),
            &temp.0.join("scratch")
        )
        .is_err()
    );
    assert!(sources::prepare("https://github.com/acme/skills", &link).is_err());
    assert_eq!(fs::read_to_string(original.join("KEEP"))?, "owned by user");
    assert!(!temp.0.join("scratch").exists());
    Ok(())
}

fn stored_zip(path: &Path, entries: &[(&str, &[u8], u32)]) -> Result<()> {
    fn push16(bytes: &mut Vec<u8>, value: u16) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    fn push32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = u32::MAX;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ if crc & 1 == 1 { 0xedb8_8320 } else { 0 };
            }
        }
        !crc
    }
    let mut bytes = Vec::new();
    let mut central = Vec::new();
    for (name, payload, mode) in entries {
        let offset = u32::try_from(bytes.len()).map_err(std::io::Error::other)?;
        let size = u32::try_from(payload.len()).map_err(std::io::Error::other)?;
        let name_size = u16::try_from(name.len()).map_err(std::io::Error::other)?;
        let crc = crc32(payload);
        bytes.extend_from_slice(b"PK\x03\x04");
        for value in [20, 0x800, 0, 0, 0] {
            push16(&mut bytes, value);
        }
        for value in [crc, size, size] {
            push32(&mut bytes, value);
        }
        for value in [name_size, 0] {
            push16(&mut bytes, value);
        }
        bytes.extend_from_slice(name.as_bytes());
        bytes.extend_from_slice(payload);
        central.extend_from_slice(b"PK\x01\x02");
        for value in [0x314, 20, 0x800, 0, 0, 0] {
            push16(&mut central, value);
        }
        for value in [crc, size, size] {
            push32(&mut central, value);
        }
        for value in [name_size, 0, 0, 0, 0] {
            push16(&mut central, value);
        }
        push32(&mut central, *mode << 16);
        push32(&mut central, offset);
        central.extend_from_slice(name.as_bytes());
    }
    let central_start = u32::try_from(bytes.len()).map_err(std::io::Error::other)?;
    let central_size = u32::try_from(central.len()).map_err(std::io::Error::other)?;
    bytes.extend_from_slice(&central);
    bytes.extend_from_slice(b"PK\x05\x06");
    let count = u16::try_from(entries.len()).map_err(std::io::Error::other)?;
    for value in [0, 0, count, count] {
        push16(&mut bytes, value);
    }
    push32(&mut bytes, central_size);
    push32(&mut bytes, central_start);
    push16(&mut bytes, 0);
    fs::write(path, bytes)?;
    Ok(())
}

#[test]
fn zip_stored_and_deflated_sources_are_bounded_local_and_owned() -> Result<()> {
    use std::io::Write;
    let temp = Scratch::new()?;
    let state = temp.0.join("state");
    for compression in [
        zip::CompressionMethod::Stored,
        zip::CompressionMethod::Deflated,
    ] {
        let path = temp.0.join(format!("{compression:?}.zip"));
        let mut archive = zip::ZipWriter::new(fs::File::create(&path)?);
        let options = zip::write::SimpleFileOptions::default().compression_method(compression);
        archive
            .add_directory("skill/", options)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        archive
            .start_file("skill/SKILL.md", options)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        archive.write_all(b"---\nname: sample\ndescription: ZIP source\n---\n")?;
        archive
            .start_file("skill/run", options.unix_permissions(0o755))
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        archive.write_all(b"#!/bin/sh\ntouch UNTRUSTED_RAN\n")?;
        archive
            .finish()
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let original = fs::read(&path)?;
        sources::validate_input(&path.to_string_lossy())?;
        assert!(!state.exists());
        let prepared = sources::prepare(&path.to_string_lossy(), &state)?;
        assert!(prepared.root.starts_with(&state));
        assert_eq!(prepared.provenance.kind, "local");
        assert_eq!(prepared.provenance.found_in, vec![path.clone()]);
        assert_eq!(prepared.provenance.url, None);
        assert_eq!(prepared.provenance.commit, None);
        assert_eq!(prepared.label, format!("{compression:?}"));
        assert_eq!(
            fs::read(prepared.root.join("skill/SKILL.md"))?,
            b"---\nname: sample\ndescription: ZIP source\n---\n"
        );
        assert!(!prepared.root.join("UNTRUSTED_RAN").exists());
        assert!(!prepared.root.join("skill/UNTRUSTED_RAN").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_ne!(
                fs::metadata(prepared.root.join("skill/run"))?
                    .permissions()
                    .mode()
                    & 0o111,
                0
            );
        }
        let root = prepared.root.clone();
        drop(prepared);
        assert!(!root.exists());
        assert_eq!(fs::read(&path)?, original);
        assert_eq!(fs::read_dir(&state)?.count(), 0);
        fs::remove_dir(&state)?;
    }
    Ok(())
}

#[test]
fn zip_traversal_and_platform_aliases_fail_before_any_extraction() -> Result<()> {
    let temp = Scratch::new()?;
    let path = temp.0.join("unsafe.zip");
    let state = temp.0.join("never-created");
    fs::write(temp.0.join("KEEP"), "user file")?;
    for name in [
        "../escape",
        "a/../../escape",
        "/absolute",
        "//server/share",
        "C:/escape",
        "C:\\escape",
        "\\\\server\\share",
        "a\\..\\escape",
        "a//b",
        "a/./b",
        "a/..",
        "a:stream",
        "a/trailing.",
        "a/trailing ",
        "a/CON.txt",
        "a/NUL",
        "a/\0name",
    ] {
        stored_zip(
            &path,
            &[
                ("SKILL.md", b"harmless", 0o100644),
                (name, b"unsafe", 0o100644),
            ],
        )?;
        assert!(
            sources::validate_input(&path.to_string_lossy()).is_err(),
            "accepted dry-run: {name:?}"
        );
        assert!(
            sources::prepare(&path.to_string_lossy(), &state).is_err(),
            "accepted: {name:?}"
        );
        assert!(!state.exists(), "created scratch for {name:?}");
        assert_eq!(fs::read_to_string(temp.0.join("KEEP"))?, "user file");
    }
    Ok(())
}

#[test]
fn zip_symlinks_special_files_and_duplicate_paths_are_rejected() -> Result<()> {
    let temp = Scratch::new()?;
    let path = temp.0.join("unsafe.zip");
    let state = temp.0.join("never-created");
    for mode in [0o120777, 0o010644, 0o020644, 0o060644, 0o140644, 0o040755] {
        stored_zip(
            &path,
            &[
                ("SKILL.md", b"harmless", 0o100644),
                ("special", b"/etc/passwd", mode),
            ],
        )?;
        assert!(
            sources::validate_input(&path.to_string_lossy()).is_err(),
            "accepted mode {mode:o}"
        );
        assert!(sources::prepare(&path.to_string_lossy(), &state).is_err());
        assert!(!state.exists());
    }
    for entries in [
        vec![
            ("SKILL.md", b"one".as_slice(), 0o100644),
            ("SKILL.md", b"two".as_slice(), 0o100644),
        ],
        vec![
            ("Skill/SKILL.md", b"one".as_slice(), 0o100644),
            ("skill/other", b"two".as_slice(), 0o100644),
        ],
        vec![
            ("skill", b"one".as_slice(), 0o100644),
            ("skill/SKILL.md", b"two".as_slice(), 0o100644),
        ],
        vec![
            ("skill/SKILL.md", b"one".as_slice(), 0o100644),
            ("skill", b"two".as_slice(), 0o100644),
        ],
        vec![
            ("skill/", b"".as_slice(), 0o040755),
            ("skill/", b"".as_slice(), 0o040755),
        ],
        vec![
            ("skill", b"one".as_slice(), 0o100644),
            ("skill/", b"".as_slice(), 0o040755),
        ],
    ] {
        stored_zip(&path, &entries)?;
        assert!(
            sources::validate_input(&path.to_string_lossy()).is_err(),
            "accepted colliding metadata"
        );
        assert!(sources::prepare(&path.to_string_lossy(), &state).is_err());
        assert!(!state.exists());
    }
    Ok(())
}

#[test]
fn zip_encryption_unsupported_compression_and_size_metadata_are_rejected() -> Result<()> {
    let temp = Scratch::new()?;
    let path = temp.0.join("unsafe.zip");
    let state = temp.0.join("never-created");
    stored_zip(&path, &[("SKILL.md", b"payload", 0o100644)])?;
    let original = fs::read(&path)?;
    let central = original
        .windows(4)
        .position(|bytes| bytes == b"PK\x01\x02")
        .ok_or_else(|| std::io::Error::other("missing central header"))?;
    for change in ["encrypted", "compression", "size", "count"] {
        let mut bytes = original.clone();
        match change {
            "encrypted" => {
                bytes[6] |= 1;
                bytes[central + 8] |= 1;
            }
            "compression" => {
                bytes[8..10].copy_from_slice(&12u16.to_le_bytes());
                bytes[central + 10..central + 12].copy_from_slice(&12u16.to_le_bytes());
            }
            "size" => {
                bytes[central + 24..central + 28]
                    .copy_from_slice(&(32u32 * 1024 * 1024 + 1).to_le_bytes());
            }
            _ => {
                let end = bytes.len() - 22;
                bytes[end + 8..end + 12].copy_from_slice(&[0xff; 4]);
            }
        }
        fs::write(&path, bytes)?;
        assert!(
            sources::validate_input(&path.to_string_lossy()).is_err(),
            "accepted {change}"
        );
        assert!(
            sources::prepare(&path.to_string_lossy(), &state).is_err(),
            "accepted {change}"
        );
        assert!(!state.exists());
    }
    Ok(())
}

#[test]
fn zip_corrupt_payload_cleans_owned_scratch_and_keeps_archive_and_siblings() -> Result<()> {
    let temp = Scratch::new()?;
    let path = temp.0.join("corrupt.zip");
    let state = temp.0.join("state");
    fs::create_dir_all(state.join("unrelated"))?;
    fs::write(state.join("unrelated/KEEP"), "keep")?;
    stored_zip(&path, &[("SKILL.md", b"payload", 0o100644)])?;
    let mut bytes = fs::read(&path)?;
    bytes[30 + "SKILL.md".len()] ^= 1;
    fs::write(&path, &bytes)?;
    // Metadata-only dry-run cannot verify payload CRCs without decompressing.
    sources::validate_input(&path.to_string_lossy())?;
    assert!(sources::prepare(&path.to_string_lossy(), &state).is_err());
    assert_eq!(fs::read(&path)?, bytes);
    assert_eq!(fs::read_dir(&state)?.count(), 1);
    assert_eq!(fs::read_to_string(state.join("unrelated/KEEP"))?, "keep");
    Ok(())
}

#[test]
fn zip_replayed_deflate_streams_are_rejected_before_scratch_writes() -> Result<()> {
    use std::io::Write;
    let temp = Scratch::new()?;
    let path = temp.0.join("overlap.zip");
    let state = temp.0.join("never-created");
    let mut archive = zip::ZipWriter::new(fs::File::create(&path)?);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for name in ["first.md", "other.md"] {
        archive
            .start_file(name, options)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        archive.write_all(b"one shared deflated source payload\n")?;
    }
    archive
        .finish()
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let mut bytes = fs::read(&path)?;
    let headers = bytes
        .windows(4)
        .enumerate()
        .filter_map(|(offset, bytes)| (bytes == b"PK\x01\x02").then_some(offset))
        .collect::<Vec<_>>();
    assert_eq!(headers.len(), 2);
    let first_payload_header = bytes[headers[0] + 42..headers[0] + 46].to_vec();
    bytes[headers[1] + 42..headers[1] + 46].copy_from_slice(&first_payload_header);
    fs::write(&path, &bytes)?;
    let error =
        sources::validate_input(&path.to_string_lossy()).expect_err("accepted overlapping data");
    assert_eq!(error.code, "INVALID_ARCHIVE");
    assert!(error.message.contains("overlap"), "{error}");
    assert!(sources::prepare(&path.to_string_lossy(), &state).is_err());
    assert!(!state.exists());
    assert_eq!(fs::read(&path)?, bytes);
    Ok(())
}

#[test]
fn zip_unicode_directory_aliases_follow_filesystem_identity_rules() -> Result<()> {
    let temp = Scratch::new()?;
    let composed = "\u{e9}";
    let decomposed = "e\u{301}";
    let probe = temp.0.join("normalization-probe");
    fs::create_dir(&probe)?;
    fs::create_dir(probe.join(composed))?;
    let filesystem_aliases_names = probe.join(decomposed).is_dir();
    let path = temp.0.join("unicode.zip");
    let state = temp.0.join("state");
    fs::create_dir_all(state.join("unrelated"))?;
    fs::write(state.join("unrelated/KEEP"), "user file")?;
    let first = format!("{composed}/SKILL.md");
    let reused_parent = format!("{composed}/reference.md");
    let alias = format!("{decomposed}/other.md");
    stored_zip(
        &path,
        &[
            (&first, b"first source", 0o100644),
            (&reused_parent, b"same exact logical parent", 0o100644),
            (&alias, b"distinct logical parent", 0o100644),
        ],
    )?;
    let original = fs::read(&path)?;
    // A read-only metadata check cannot discover filesystem normalization rules.
    sources::validate_input(&path.to_string_lossy())?;
    let prepared = sources::prepare(&path.to_string_lossy(), &state);
    if filesystem_aliases_names {
        let error = prepared.expect_err("merged distinct logical ZIP directories");
        assert_eq!(error.code, "ZIP_PATH_COLLISION");
    } else {
        let prepared = prepared?;
        assert_eq!(fs::read(prepared.root.join(&first))?, b"first source");
        assert_eq!(
            fs::read(prepared.root.join(&reused_parent))?,
            b"same exact logical parent"
        );
        assert_eq!(
            fs::read(prepared.root.join(&alias))?,
            b"distinct logical parent"
        );
        drop(prepared);
    }
    assert_eq!(fs::read_dir(&state)?.count(), 1);
    assert_eq!(
        fs::read_to_string(state.join("unrelated/KEEP"))?,
        "user file"
    );
    assert_eq!(fs::read(&path)?, original);
    Ok(())
}

#[test]
fn input_validation_is_read_only_for_local_and_remote_inputs() -> Result<()> {
    let temp = Scratch::new()?;
    sources::validate_input(&temp.0.to_string_lossy())?;
    sources::validate_input("https://github.com/acme/skills")?;
    sources::validate_input("https://github.com/acme/skills/tree/main/skills/demo")?;
    for input in [
        "",
        "https://github.com/acme/skills?token=x",
        "https://github.com/acme/skills/tree/main/../other",
    ] {
        assert!(sources::validate_input(input).is_err());
    }
    assert_eq!(fs::read_dir(&temp.0)?.count(), 0);
    Ok(())
}

fn git(root: &Path, arguments: &[&str]) -> Result<Output> {
    let mut command = Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    let output = command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "user.name=Source Test",
            "-c",
            "user.email=source@test.invalid",
            "-C",
        ])
        .arg(root)
        .args(arguments)
        .output()?;
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(output)
}

fn repository(root: &Path) -> Result<String> {
    fs::create_dir(root)?;
    git(
        root,
        &["init", "--quiet", "--initial-branch=main", "--template="],
    )?;
    fs::create_dir_all(root.join("skills/demo"))?;
    fs::write(
        root.join("skills/demo/SKILL.md"),
        "---\nname: demo\ndescription: source fixture\n---\n",
    )?;
    fs::write(root.join(".gitattributes"), "* filter=evil\n")?;
    fs::write(
        root.join("skills/demo/payload.sh"),
        "#!/bin/sh\ntouch SCRIPT_RAN\n",
    )?;
    fs::write(
        root.join("git"),
        "#!/bin/sh\ntouch \"$SOURCE_TEST_RAN\"\nexit 97\n",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root.join("git"), fs::Permissions::from_mode(0o700))?;
    }
    git(root, &["add", "."])?;
    git(root, &["commit", "--quiet", "-m", "Source fixture"])?;
    git(
        root,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/acme/skills.git",
        ],
    )?;
    let commit = git(root, &["rev-parse", "HEAD"])?;
    Ok(String::from_utf8_lossy(&commit.stdout).trim().to_string())
}

#[test]
fn real_local_repository_keeps_local_kind_and_records_actual_provenance() -> Result<()> {
    let temp = Scratch::new()?;
    let repo = temp.0.join("repo");
    let commit = repository(&repo)?;
    git(&repo, &["pack-refs", "--all", "--prune"])?;
    let included = repo.join(".git/untrusted-config");
    git(
        &repo,
        &["config", "include.path", &included.to_string_lossy()],
    )?;
    // Activating this source configuration would fail parsing its include.
    // Provenance reads must treat local configuration as data, without includes.
    fs::write(&included, "[invalid configuration\n")?;
    let local = repo.join("skills/demo");
    let state = temp.0.join("not-created");
    let prepared = sources::prepare(&local.to_string_lossy(), &state)?;
    assert_eq!(prepared.provenance.kind, "local");
    assert_eq!(
        prepared.provenance.url.as_deref(),
        Some("https://github.com/acme/skills.git")
    );
    assert_eq!(prepared.provenance.commit.as_deref(), Some(commit.as_str()));
    assert_eq!(prepared.provenance.subpath.as_deref(), Some("skills/demo"));
    assert_eq!(prepared.provenance.found_in, vec![local.clone()]);
    drop(prepared);
    assert!(local.join("SKILL.md").is_file());
    assert!(!state.exists());
    Ok(())
}

#[cfg(unix)]
fn executable_git() -> Result<PathBuf> {
    let path = std::env::var_os("PATH").ok_or_else(|| std::io::Error::other("PATH is missing"))?;
    for directory in std::env::split_paths(&path) {
        let executable = directory.join("git");
        if executable.is_file() {
            return Ok(executable.canonicalize()?);
        }
    }
    Err(std::io::Error::other("git executable is missing").into())
}

#[cfg(unix)]
#[test]
fn github_clone_is_offline_isolated_and_cleans_up_only_its_own_scratch() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let temp = Scratch::new()?;
    let repo = temp.0.join("repo");
    let commit = repository(&repo)?;
    let binary = temp.0.join("bin");
    fs::create_dir(&binary)?;
    let wrapper = binary.join("git");
    fs::write(
        &wrapper,
        r#"#!/bin/sh
set -eu
test "$GIT_CONFIG_NOSYSTEM" = 1
test "$GIT_CONFIG_GLOBAL" = /dev/null
test "$GIT_CONFIG_SYSTEM" = /dev/null
test "$GIT_TERMINAL_PROMPT" = 0
test "$GIT_ATTR_NOSYSTEM" = 1
test "${GIT_DIR+x}" != x
test "${GIT_WORK_TREE+x}" != x
test "${GIT_INDEX_FILE+x}" != x
test "${GIT_OBJECT_DIRECTORY+x}" != x
test "${GIT_CONFIG_PARAMETERS+x}" != x
test "${GIT_CONFIG_COUNT+x}" != x
test "${GIT_CONFIG_KEY_0+x}" != x
test "${GIT_CONFIG_VALUE_0+x}" != x
test "${GIT_TEMPLATE_DIR+x}" != x
if read -r ignored; then exit 91; fi
printf '%s\n' "$@" >> "$SOURCE_TEST_LOG"
operation=
branch=
previous=
last=
for argument do
    if test "$argument" = clone; then operation=clone; fi
    if test "$previous" = --branch; then branch=$argument; fi
    previous=$argument
    last=$argument
done
if test "$operation" = clone; then
    if test -n "$branch"; then
        exec "$SOURCE_TEST_GIT" -c core.hooksPath=/dev/null -c protocol.file.allow=always clone --quiet --depth 1 --no-local --no-checkout --template= --branch "$branch" -- "$SOURCE_TEST_REPO" "$last"
    fi
    exec "$SOURCE_TEST_GIT" -c core.hooksPath=/dev/null -c protocol.file.allow=always clone --quiet --depth 1 --no-local --no-checkout --template= -- "$SOURCE_TEST_REPO" "$last"
fi
exec "$SOURCE_TEST_GIT" "$@"
"#,
    )?;
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700))?;
    let hook_dir = temp.0.join("hooks");
    fs::create_dir(&hook_dir)?;
    let hook = hook_dir.join("post-checkout");
    fs::write(&hook, "#!/bin/sh\ntouch \"$SOURCE_TEST_RAN\"\n")?;
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o700))?;
    let config = temp.0.join("hostile.gitconfig");
    fs::write(
        &config,
        "[filter \"evil\"]\nsmudge = touch \"$SOURCE_TEST_RAN\"; cat\nrequired = true\n[core]\nhooksPath = /should-not-be-used\n",
    )?;
    let state = temp.0.join("state");
    fs::create_dir_all(state.join("unrelated"))?;
    fs::write(state.join("unrelated/KEEP"), "keep")?;
    let log = temp.0.join("git-args.log");
    let mut path = vec![PathBuf::from("."), binary];
    let old_path =
        std::env::var_os("PATH").ok_or_else(|| std::io::Error::other("PATH is missing"))?;
    path.extend(std::env::split_paths(&old_path));
    let output = Command::new(std::env::current_exe()?)
        .args(["--exact", "github_fixture_child", "--nocapture"])
        .current_dir(&repo)
        .env(
            "PATH",
            std::env::join_paths(path).map_err(|error| std::io::Error::other(error.to_string()))?,
        )
        .env("SOURCE_TEST_CHILD", "1")
        .env("SOURCE_TEST_GIT", executable_git()?)
        .env("SOURCE_TEST_REPO", &repo)
        .env("SOURCE_TEST_STATE", &state)
        .env("SOURCE_TEST_COMMIT", &commit)
        .env("SOURCE_TEST_LOG", &log)
        .env("SOURCE_TEST_RAN", temp.0.join("UNTRUSTED_RAN"))
        .env("GIT_DIR", "/injected/gitdir")
        .env("GIT_WORK_TREE", "/injected/worktree")
        .env("GIT_INDEX_FILE", "/injected/index")
        .env("GIT_OBJECT_DIRECTORY", "/injected/objects")
        .env("GIT_CONFIG_PARAMETERS", "'core.hooksPath=/injected/hooks'")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "core.hooksPath")
        .env("GIT_CONFIG_VALUE_0", &hook_dir)
        .env("GIT_CONFIG_GLOBAL", &config)
        .env("GIT_CONFIG_SYSTEM", &config)
        .env("GIT_TEMPLATE_DIR", &hook_dir)
        .stdin(Stdio::null())
        .output()?;
    assert!(
        output.status.success(),
        "child failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!temp.0.join("UNTRUSTED_RAN").exists());
    assert_eq!(fs::read_to_string(state.join("unrelated/KEEP"))?, "keep");
    let arguments = fs::read_to_string(log)?;
    for expected in [
        "--depth\n1\n",
        "--no-checkout\n",
        "--branch\nmain\n",
        "--template=\n",
        "core.hooksPath=/dev/null\n",
        "core.fsmonitor=false\n",
        "protocol.ext.allow=never\n",
        "https://github.com/acme/skills.git\n",
    ] {
        assert!(
            arguments.contains(expected),
            "missing {expected:?} from {arguments}"
        );
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn github_fixture_child() -> Result<()> {
    if std::env::var_os("SOURCE_TEST_CHILD").is_none() {
        return Ok(());
    }
    let state = PathBuf::from(
        std::env::var_os("SOURCE_TEST_STATE")
            .ok_or_else(|| std::io::Error::other("missing state"))?,
    );
    let commit = std::env::var("SOURCE_TEST_COMMIT")
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let repo = PathBuf::from(
        std::env::var_os("SOURCE_TEST_REPO")
            .ok_or_else(|| std::io::Error::other("missing repository"))?,
    );
    let local = sources::prepare(&repo.to_string_lossy(), &state)?;
    assert_eq!(local.provenance.commit.as_deref(), Some(commit.as_str()));
    assert_eq!(local.provenance.kind, "local");
    drop(local);
    let prepared = sources::prepare(
        "https://github.com/acme/skills.git/tree/main/skills/demo",
        &state,
    )?;
    assert!(prepared.root.starts_with(&state));
    assert!(prepared.root.join("SKILL.md").is_file());
    assert!(!prepared.root.join("SCRIPT_RAN").exists());
    assert_eq!(prepared.provenance.kind, "github");
    assert_eq!(
        prepared.provenance.url.as_deref(),
        Some("https://github.com/acme/skills")
    );
    assert_eq!(prepared.provenance.subpath.as_deref(), Some("skills/demo"));
    assert_eq!(prepared.provenance.commit.as_deref(), Some(commit.as_str()));
    assert!(prepared.provenance.found_in.is_empty());
    assert_eq!(prepared.label, "acme");
    let root = prepared.root.clone();
    drop(prepared);
    assert!(!root.exists());
    let prepared = sources::prepare("https://github.com/acme/skills", &state)?;
    assert!(prepared.root.join(".git").is_dir());
    assert!(prepared.root.join("skills/demo/SKILL.md").is_file());
    let root = prepared.root.clone();
    drop(prepared);
    assert!(!root.exists());
    assert!(
        sources::prepare(
            "https://github.com/acme/skills/tree/main/not-present",
            &state
        )
        .is_err()
    );
    assert!(
        sources::prepare(
            "https://github.com/acme/skills/tree/no-such-branch/skills",
            &state
        )
        .is_err()
    );
    let remaining = fs::read_dir(&state)?.collect::<std::result::Result<Vec<_>, _>>()?;
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].file_name(), "unrelated");
    Ok(())
}

#[test]
fn both_local_provenance_readers_remove_private_url_parts() -> Result<()> {
    let temp = Scratch::new()?;
    let repo = temp.0.join("repo");
    repository(&repo)?;
    git(
        &repo,
        &[
            "remote",
            "set-url",
            "origin",
            "https://reader:fixture@example.invalid/group/repo?fixture=1#section",
        ],
    )?;
    let local = repo.join("skills/demo");
    let prepared = sources::prepare(&local.to_string_lossy(), &temp.0.join("state"))?;
    for source in [
        prepared.provenance.clone(),
        kinana_core::gitlog::remote_hint(&local),
    ] {
        assert_eq!(
            source.url.as_deref(),
            Some("https://example.invalid/group/repo")
        );
    }
    use kinana_core::{ImportOptions, import, scan, vault::Vault};
    let public = "https://example.invalid/group/repo";
    let report = scan::scan(std::slice::from_ref(&local), &[])?;
    assert_eq!(report.entries[0].source.url.as_deref(), Some(public));
    for mode in ["add", "scan"] {
        let vault = Vault::new(temp.0.join(mode), temp.0.join(format!("index-{mode}.db")));
        vault.init(false)?;
        let options = ImportOptions {
            allow_risky: true,
            ..Default::default()
        };
        if mode == "scan" {
            import::import_scan(&vault, std::slice::from_ref(&local), &options)?;
        } else {
            import::add(
                &vault,
                &local.to_string_lossy(),
                &temp.0.join("state"),
                &options,
            )?;
        }
        let file = vault.root.join("meta/demo.toml");
        let metadata = fs::read_to_string(&file)?;
        assert!(metadata.contains(public) && !metadata.contains("reader:fixture"));
        let db = rusqlite::Connection::open(&vault.index_path)?;
        let indexed: String =
            db.query_row("SELECT source_url FROM skills WHERE id='demo'", [], |r| {
                r.get(0)
            })?;
        assert_eq!(indexed, public);
        fs::write(
            &file,
            metadata.replace(
                public,
                "https://user:old@example.invalid/group/repo?old=1#old",
            ),
        )?;
        kinana_core::gitlog::commit(&vault.root, "Legacy provenance fixture")?;
        assert_eq!(
            vault.show("demo")?.skill.source.url.as_deref(),
            Some(public)
        );
        import::add(
            &vault,
            &local.to_string_lossy(),
            &temp.0.join("state"),
            &options,
        )?;
        assert!(!fs::read_to_string(file)?.contains("user:old"));
    }
    Ok(())
}
