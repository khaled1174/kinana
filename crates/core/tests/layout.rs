mod support;

use kinana_core::{SkillMetadata, gitlog, hash::hash_directory, layout, vault::Vault};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = support::temp_dir().unwrap();
        Self(path)
    }

    fn raw_vault(&self) -> PathBuf {
        let root = self.0.join("vault");
        fs::create_dir_all(root.join("skills")).unwrap();
        fs::create_dir(root.join("meta")).unwrap();
        root
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _cleanup = fs::remove_dir_all(&self.0);
    }
}

fn metadata(id: &str, directories: &[&str]) -> SkillMetadata {
    SkillMetadata {
        proposal_source_changed: None,
        classification_context: None,
        pinned: false,
        id: id.into(),
        content_hash: "blake3:recorded".into(),
        added_at: "2026-11-13T09:00:00Z".into(),
        source: Default::default(),
        classification: Default::default(),
        description: Default::default(),
        audit: Default::default(),
        empty_directories: directories.iter().map(PathBuf::from).collect(),
        proposal: None,
    }
}

fn write_metadata(root: &Path, filename_id: &str, metadata: &SkillMetadata) {
    fs::write(
        root.join("meta").join(format!("{filename_id}.toml")),
        toml::to_string_pretty(metadata).unwrap(),
    )
    .unwrap();
}

fn skill(root: &Path, id: &str) -> PathBuf {
    let path = root.join("skills").join(id);
    fs::create_dir(&path).unwrap();
    fs::write(
        path.join("SKILL.md"),
        format!("---\nname: {id}\ndescription: Test skill.\n---\nOriginal bytes\r\n"),
    )
    .unwrap();
    path
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    walkdir::WalkDir::new(root)
        .min_depth(1)
        .follow_links(false)
        .into_iter()
        .map(|entry| {
            let entry = entry.unwrap();
            let bytes = if entry.file_type().is_symlink() {
                let mut bytes = b"link\0".to_vec();
                bytes.extend_from_slice(
                    fs::read_link(entry.path())
                        .unwrap()
                        .as_os_str()
                        .as_encoded_bytes(),
                );
                bytes
            } else if entry.file_type().is_dir() {
                b"directory".to_vec()
            } else if entry.file_type().is_file() {
                let mut bytes = b"file\0".to_vec();
                bytes.extend(fs::read(entry.path()).unwrap());
                bytes
            } else {
                b"special".to_vec()
            };
            (
                entry.path().strip_prefix(root).unwrap().to_path_buf(),
                bytes,
            )
        })
        .collect()
}

#[test]
fn collects_only_empty_leaves_in_sorted_order_without_touching_originals() {
    let temp = Scratch::new();
    let source = temp.0.join("source");
    for directory in [
        "z-empty",
        "a/nested/leaf",
        "a/second",
        "assets",
        ".git/private/empty",
        "nested/.git/payload-empty",
    ] {
        fs::create_dir_all(source.join(directory)).unwrap();
    }
    fs::write(source.join("SKILL.md"), "Original\r\n").unwrap();
    fs::write(source.join("assets/data.bin"), [0, 255, 13, 10]).unwrap();
    fs::write(source.join(".git/HEAD"), "private repository data").unwrap();
    let before = snapshot(&temp.0);
    let directories = layout::empty_directories(&source).unwrap();
    assert_eq!(
        directories,
        [
            "a/nested/leaf",
            "a/second",
            "nested/.git/payload-empty",
            "z-empty",
        ]
        .map(PathBuf::from)
    );
    assert_eq!(layout::empty_directories(&source).unwrap(), directories);
    assert_eq!(snapshot(&temp.0), before);

    let empty_root = temp.0.join("empty-source");
    fs::create_dir(&empty_root).unwrap();
    assert!(layout::empty_directories(&empty_root).unwrap().is_empty());
    fs::write(temp.0.join("file"), "regular file").unwrap();
    assert!(layout::empty_directories(&temp.0.join("file")).is_err());
}

#[test]
fn import_undo_and_undo_again_restore_the_exact_directory_hash() {
    let temp = Scratch::new();
    let vault = Vault::new(temp.0.join("vault"), temp.0.join("index.db"));
    vault.init(false).unwrap();
    let original = temp.0.join("original");
    fs::create_dir_all(original.join("assets/nested/empty")).unwrap();
    fs::create_dir(original.join("references")).unwrap();
    let manifest = b"---\nname: sample\ndescription: Test skill.\n---\nOriginal\r\n";
    let binary = [0, 255, 13, 10, 128];
    fs::write(original.join("SKILL.md"), manifest).unwrap();
    fs::write(original.join("assets/raw.bin"), binary).unwrap();
    let original_hash = hash_directory(&original).unwrap();
    let original_snapshot = snapshot(&original);

    // Simulate the file and metadata layer of import using real Git history.
    let imported = vault.root.join("skills/sample");
    fs::create_dir_all(imported.join("assets/nested/empty")).unwrap();
    fs::create_dir(imported.join("references")).unwrap();
    fs::write(imported.join("SKILL.md"), manifest).unwrap();
    fs::write(imported.join("assets/raw.bin"), binary).unwrap();
    let mut metadata = metadata("sample", &[]);
    metadata.content_hash = original_hash.clone();
    metadata.empty_directories = layout::empty_directories(&original).unwrap();
    write_metadata(&vault.root, "sample", &metadata);
    gitlog::commit(&vault.root, "Import sample with empty directories").unwrap();
    assert_eq!(hash_directory(&imported).unwrap(), original_hash);

    vault.undo(false).unwrap();
    assert!(!imported.exists());
    assert!(!vault.root.join("meta/sample.toml").exists());
    assert!(vault.records().unwrap().is_empty());

    vault.undo(false).unwrap();
    assert_eq!(hash_directory(&imported).unwrap(), original_hash);
    assert!(imported.join("assets/nested/empty").is_dir());
    assert!(imported.join("references").is_dir());
    assert_eq!(vault.records().unwrap()[0].content_hash, original_hash);
    assert_eq!(snapshot(&original), original_snapshot);
    gitlog::ensure_clean(&vault.root).unwrap();
}

#[test]
fn old_metadata_is_compatible_and_existing_directories_are_untouched() {
    let temp = Scratch::new();
    let root = temp.raw_vault();
    let source = skill(&root, "legacy");
    let legacy_metadata = metadata("legacy", &[]);
    let text = toml::to_string_pretty(&legacy_metadata).unwrap();
    assert!(!text.contains("empty_directories"));
    let decoded: SkillMetadata = toml::from_str(&text).unwrap();
    assert!(decoded.empty_directories.is_empty());
    write_metadata(&root, "legacy", &decoded);
    let before = snapshot(&temp.0);
    layout::restore(&root).unwrap();
    assert_eq!(snapshot(&temp.0), before);

    fs::create_dir(source.join("assets")).unwrap();
    fs::write(source.join("assets/preserve.txt"), "user content").unwrap();
    write_metadata(&root, "legacy", &metadata("legacy", &["assets"]));
    let before = snapshot(&temp.0);
    layout::restore(&root).unwrap();
    assert_eq!(snapshot(&temp.0), before);
}

#[test]
fn unsafe_manifest_paths_fail_before_any_directory_is_created() {
    let temp = Scratch::new();
    let root = temp.raw_vault();
    skill(&root, "sample");
    let absolute = temp.0.join("external").to_string_lossy().into_owned();
    for unsafe_path in [
        "",
        ".",
        "..",
        "../external",
        "assets/../../external",
        "assets/./empty",
        "assets//empty",
        "assets/",
        "/external",
        "C:\\external",
        "C:external",
        "\\\\server\\share",
        ".git/empty",
        &absolute,
    ] {
        write_metadata(
            &root,
            "sample",
            &metadata("sample", &["first-safe/empty", unsafe_path]),
        );
        let before = snapshot(&temp.0);
        assert!(layout::restore(&root).is_err(), "accepted {unsafe_path:?}");
        assert_eq!(snapshot(&temp.0), before, "wrote for {unsafe_path:?}");
    }
}

#[test]
fn all_metadata_and_existing_components_are_preflighted_before_creation() {
    let temp = Scratch::new();
    let root = temp.raw_vault();
    let first = skill(&root, "a-first");
    skill(&root, "z-last");
    write_metadata(&root, "a-first", &metadata("a-first", &["new/empty"]));
    write_metadata(&root, "z-last", &metadata("different-id", &["empty"]));
    let before = snapshot(&temp.0);
    assert_eq!(
        layout::restore(&root).unwrap_err().code,
        "METADATA_ID_MISMATCH"
    );
    assert_eq!(snapshot(&temp.0), before);

    write_metadata(&root, "z-last", &metadata("z-last", &[]));
    fs::write(first.join("blocked"), "never overwrite").unwrap();
    write_metadata(
        &root,
        "a-first",
        &metadata("a-first", &["new/empty", "blocked/child"]),
    );
    let before = snapshot(&temp.0);
    assert!(layout::restore(&root).is_err());
    assert_eq!(snapshot(&temp.0), before);
}

#[test]
fn metadata_never_recreates_a_missing_skill_or_manifest() {
    let temp = Scratch::new();
    let root = temp.raw_vault();
    write_metadata(&root, "removed", &metadata("removed", &["assets/empty"]));
    let before = snapshot(&temp.0);
    assert!(layout::restore(&root).is_err());
    assert_eq!(snapshot(&temp.0), before);
    assert!(!root.join("skills/removed").exists());

    fs::create_dir(root.join("skills/removed")).unwrap();
    let before = snapshot(&temp.0);
    assert!(layout::restore(&root).is_err());
    assert_eq!(snapshot(&temp.0), before);
}

#[cfg(unix)]
#[test]
fn source_symlinks_components_and_special_files_are_rejected_without_writes() {
    use std::os::unix::{fs::symlink, net::UnixListener};

    let temp = Scratch::new();
    let root = temp.0.join("source");
    fs::create_dir_all(root.join("empty")).unwrap();
    symlink(&root, temp.0.join("source-link")).unwrap();
    assert!(layout::empty_directories(&temp.0.join("source-link")).is_err());
    assert!(layout::empty_directories(&temp.0.join("source-link/empty")).is_err());

    symlink(temp.0.join("missing"), root.join("dangling")).unwrap();
    let before = snapshot(&temp.0);
    assert!(layout::empty_directories(&root).is_err());
    assert_eq!(snapshot(&temp.0), before);
    fs::remove_file(root.join("dangling")).unwrap();

    let socket_path = root.join("socket");
    let _socket = UnixListener::bind(&socket_path).unwrap();
    let before = snapshot(&temp.0);
    assert_eq!(
        layout::empty_directories(&root).unwrap_err().code,
        "UNSUPPORTED_FILE"
    );
    assert_eq!(snapshot(&temp.0), before);
}

#[cfg(unix)]
#[test]
fn restored_paths_and_metadata_never_follow_symlinks_outside_the_vault() {
    use std::os::unix::fs::symlink;

    let temp = Scratch::new();
    let root = temp.raw_vault();
    let source = skill(&root, "sample");
    let external = temp.0.join("external");
    fs::create_dir(&external).unwrap();
    fs::write(external.join("keep.txt"), "external content").unwrap();
    symlink(&external, source.join("alias")).unwrap();
    write_metadata(
        &root,
        "sample",
        &metadata("sample", &["first-safe/empty", "alias/escaped"]),
    );
    let before = snapshot(&temp.0);
    assert!(layout::restore(&root).is_err());
    assert_eq!(snapshot(&temp.0), before);

    fs::remove_file(source.join("alias")).unwrap();
    let metadata_file = root.join("meta/sample.toml");
    let external_metadata = external.join("sample.toml");
    fs::rename(&metadata_file, &external_metadata).unwrap();
    symlink(&external_metadata, &metadata_file).unwrap();
    let before = snapshot(&temp.0);
    assert!(layout::restore(&root).is_err());
    assert_eq!(snapshot(&temp.0), before);

    fs::remove_file(&metadata_file).unwrap();
    fs::rename(&external_metadata, &metadata_file).unwrap();
    fs::remove_file(source.join("SKILL.md")).unwrap();
    symlink(external.join("keep.txt"), source.join("SKILL.md")).unwrap();
    let before = snapshot(&temp.0);
    assert!(layout::restore(&root).is_err());
    assert_eq!(snapshot(&temp.0), before);
}
