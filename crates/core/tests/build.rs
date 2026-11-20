mod support;

use kinana_core::{
    Audit, BoxConfig, Classification, Description, SkillMetadata, SourceInfo, build, frontmatter,
    hash, vault::Vault,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = support::temp_dir().unwrap();
        Self(path)
    }

    fn vault(&self) -> Vault {
        let root = self.0.join("vault");
        fs::create_dir(&root).unwrap();
        for name in ["skills", "meta", "dist"] {
            fs::create_dir(root.join(name)).unwrap();
        }
        fs::write(
            root.join("box.toml"),
            toml::to_string(&BoxConfig::default()).unwrap(),
        )
        .unwrap();
        Vault::new(root, self.0.join("unused-index.db"))
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _cleanup = fs::remove_dir_all(&self.0);
    }
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut result = BTreeMap::new();
    fn walk(root: &Path, at: &Path, result: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for child in fs::read_dir(at).unwrap() {
            let child = child.unwrap();
            let path = child.path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            let metadata = fs::symlink_metadata(&path).unwrap();
            if metadata.file_type().is_symlink() {
                result.insert(
                    relative,
                    fs::read_link(&path)
                        .unwrap()
                        .as_os_str()
                        .as_encoded_bytes()
                        .to_vec(),
                );
            } else if metadata.is_dir() {
                result.insert(relative, b"directory".to_vec());
                walk(root, &path, result);
            } else {
                result.insert(relative, fs::read(&path).unwrap());
            }
        }
    }
    walk(root, root, &mut result);
    result
}

fn seed(vault: &Vault, id: &str, skill: &str) -> SkillMetadata {
    let root = vault.root.join("skills").join(id);
    fs::create_dir(&root).unwrap();
    fs::write(root.join("SKILL.md"), skill.as_bytes()).unwrap();
    fs::create_dir_all(root.join("scripts/empty/deep")).unwrap();
    fs::create_dir(root.join("assets")).unwrap();
    fs::write(root.join("scripts/run.sh"), b"#!/bin/sh\nexit 0\n").unwrap();
    fs::write(root.join("assets/data.bin"), [0, 1, 2, 0xff]).unwrap();
    let metadata = SkillMetadata {
        proposal_source_changed: None,
        pinned: false,
        id: id.into(),
        content_hash: hash::hash_directory(&root).unwrap(),
        added_at: "2026-11-13T00:00:00Z".into(),
        source: SourceInfo::default(),
        classification: Classification::default(),
        description: Description {
            original: frontmatter::parse_skill(skill).unwrap().description,
            ..Description::default()
        },
        audit: Audit::default(),
        empty_directories: vec![],
        proposal: None,
    };
    save_metadata(vault, &metadata);
    metadata
}

fn basic(id: &str) -> String {
    format!(
        "---\nname: {id}\ndescription: Original description.\n---\n\n# Raw body\n<script>never_run()</script>\n"
    )
}

fn save_metadata(vault: &Vault, metadata: &SkillMetadata) {
    fs::write(
        vault
            .root
            .join("meta")
            .join(format!("{}.toml", metadata.id)),
        toml::to_string_pretty(metadata).unwrap(),
    )
    .unwrap();
}

fn approved(metadata: &mut SkillMetadata, description: &str) {
    metadata.description.active = "improved".into();
    metadata.description.improved = Some(description.into());
    metadata.description.approved_at = Some("2026-11-13T00:00:00Z".into());
}

#[test]
fn original_build_preserves_every_payload_byte_and_empty_directory() {
    let temp = Temp::new();
    let vault = temp.vault();
    let source = "\u{feff}---\r\nname: raw\r\ndescription: |\r\n  Original description.\r\ncustom: [one, two]\r\n---\r\n\r\n## Exact body\r\nbytes stay here\nlast line";
    seed(&vault, "raw", source);
    let originals = snapshot(&vault.root.join("skills"));
    let report = build::build(&vault, &[], false).unwrap();
    assert!(!report.dry_run);
    assert_eq!(report.items.len(), 1);
    assert!(report.items[0].changed);
    assert_eq!(report.items[0].description_source, "original");
    assert_eq!(
        snapshot(&vault.root.join("dist/raw")),
        snapshot(&vault.root.join("skills/raw"))
    );
    assert_eq!(snapshot(&vault.root.join("skills")), originals);
    assert!(!vault.index_path.exists());
    assert!(!vault.root.join(".git").exists());

    let dist_before = snapshot(&vault.root.join("dist"));
    let manifest_time = fs::metadata(vault.root.join("dist/.skillsbox-build.toml"))
        .unwrap()
        .modified()
        .unwrap();
    let skill_time = fs::metadata(vault.root.join("dist/raw/SKILL.md"))
        .unwrap()
        .modified()
        .unwrap();
    let report = build::build(&vault, &[], false).unwrap();
    assert!(!report.items[0].changed);
    assert_eq!(snapshot(&vault.root.join("dist")), dist_before);
    assert_eq!(
        fs::metadata(vault.root.join("dist/raw/SKILL.md"))
            .unwrap()
            .modified()
            .unwrap(),
        skill_time
    );
    assert_eq!(
        fs::metadata(vault.root.join("dist/.skillsbox-build.toml"))
            .unwrap()
            .modified()
            .unwrap(),
        manifest_time
    );
}

#[test]
fn approved_description_preserves_unknown_yaml_values_and_exact_crlf_body() {
    let temp = Temp::new();
    let vault = temp.vault();
    let source = "---\r\nname: yaml\r\ndescription: >-\r\n  An original\r\n  multiline description.\r\nallowed-tools: [Read, Bash]\r\ndisable-model-invocation: true\r\ncustom:\r\n  nested: [1, null, false, '001']\r\n  text: |\r\n    unknown multiline\r\n    keep meaning\r\n---\r\n\r\n# Exact Markdown\r\n\tUnicode: مرحبا\r\n\nno trailing newline";
    let mut metadata = seed(&vault, "yaml", source);
    let improved = "قبل كتابة API: review contracts.\nPreserve quotes: \"yes\" and # punctuation.";
    approved(&mut metadata, improved);
    save_metadata(&vault, &metadata);
    let source_snapshot = snapshot(&vault.root.join("skills"));
    build::build(&vault, &[], false).unwrap();
    let rendered = fs::read_to_string(vault.root.join("dist/yaml/SKILL.md")).unwrap();
    assert_eq!(
        frontmatter::parse_skill(&rendered).unwrap().description,
        improved
    );
    assert_eq!(
        rendered.split_once("---\r\n\r\n").unwrap().1,
        source.split_once("---\r\n\r\n").unwrap().1
    );
    let parse_yaml = |text: &str| {
        let yaml = text
            .split_once('\n')
            .unwrap()
            .1
            .split_once("---")
            .unwrap()
            .0;
        yaml_rust2::YamlLoader::load_from_str(yaml)
            .unwrap()
            .remove(0)
    };
    let before = parse_yaml(source);
    let after = parse_yaml(&rendered);
    for key in [
        "name",
        "allowed-tools",
        "disable-model-invocation",
        "custom",
    ] {
        assert_eq!(before[key], after[key], "changed YAML value {key}");
    }
    assert_eq!(snapshot(&vault.root.join("skills")), source_snapshot);
    assert_eq!(
        fs::read(vault.root.join("dist/yaml/assets/data.bin")).unwrap(),
        [0, 1, 2, 0xff]
    );
}

#[test]
fn build_requires_approval_valid_description_hash_and_safe_ids_before_writes() {
    let temp = Temp::new();
    let vault = temp.vault();
    let original = seed(&vault, "checked", &basic("checked"));
    let mut metadata = original.clone();
    for description in [
        None,
        Some("".into()),
        Some(" ".into()),
        Some("x".repeat(1025)),
    ] {
        metadata.description.active = "improved".into();
        metadata.description.improved = description;
        metadata.description.approved_at = Some("2026-11-13".into());
        save_metadata(&vault, &metadata);
        let before = snapshot(&temp.0);
        assert_eq!(
            build::build(&vault, &[], false).unwrap_err().code,
            "INVALID_METADATA"
        );
        assert_eq!(snapshot(&temp.0), before);
    }
    approved(&mut metadata, "Unapproved text");
    for approval in [None, Some(" ".into())] {
        metadata.description.approved_at = approval;
        save_metadata(&vault, &metadata);
        let before = snapshot(&temp.0);
        assert!(build::build(&vault, &[], false).is_err());
        assert_eq!(snapshot(&temp.0), before);
    }
    metadata = original;
    save_metadata(&vault, &metadata);
    for id in [
        "..",
        "../checked",
        "/checked",
        "checked/sub",
        "checked\\sub",
        "",
    ] {
        let before = snapshot(&temp.0);
        assert_eq!(
            build::build(&vault, &[id.into()], false).unwrap_err().code,
            "INVALID_SKILL_ID"
        );
        assert_eq!(snapshot(&temp.0), before);
    }
    fs::write(vault.root.join("skills/checked/assets/data.bin"), "changed").unwrap();
    let before = snapshot(&temp.0);
    assert_eq!(
        build::build(&vault, &[], false).unwrap_err().code,
        "SOURCE_HASH_MISMATCH"
    );
    assert_eq!(snapshot(&temp.0), before);
}

#[test]
fn unicode_description_limit_counts_characters_and_never_rewrites_original() {
    let temp = Temp::new();
    let vault = temp.vault();
    let mut metadata = seed(&vault, "unicode", &basic("unicode"));
    let improved = "界".repeat(1024);
    approved(&mut metadata, &improved);
    save_metadata(&vault, &metadata);
    build::build(&vault, &[], false).unwrap();
    assert_eq!(
        frontmatter::read_skill(&vault.root.join("dist/unicode/SKILL.md"))
            .unwrap()
            .description,
        improved
    );
    assert_eq!(
        fs::read_to_string(vault.root.join("skills/unicode/SKILL.md")).unwrap(),
        basic("unicode")
    );
}

#[test]
fn changing_one_metadata_override_updates_only_that_target() {
    let temp = Temp::new();
    let vault = temp.vault();
    let mut first = seed(&vault, "first", &basic("first"));
    seed(&vault, "second", &basic("second"));
    build::build(&vault, &[], false).unwrap();
    let second_before = snapshot(&vault.root.join("dist/second"));
    let second_time = fs::metadata(vault.root.join("dist/second/SKILL.md"))
        .unwrap()
        .modified()
        .unwrap();
    let metadata_before = fs::read(vault.root.join("meta/first.toml")).unwrap();
    approved(&mut first, "Accepted replacement");
    let overrides = BTreeMap::from([("first".into(), first)]);
    let mut transaction = build::prepare(&vault, &["first".into()], &overrides).unwrap();
    assert!(transaction.report().items[0].changed);
    transaction.apply().unwrap();
    transaction.finish().unwrap();
    assert_eq!(snapshot(&vault.root.join("dist/second")), second_before);
    assert_eq!(
        fs::metadata(vault.root.join("dist/second/SKILL.md"))
            .unwrap()
            .modified()
            .unwrap(),
        second_time
    );
    assert_eq!(
        fs::read(vault.root.join("meta/first.toml")).unwrap(),
        metadata_before
    );
    assert_eq!(
        frontmatter::read_skill(&vault.root.join("dist/first/SKILL.md"))
            .unwrap()
            .description,
        "Accepted replacement"
    );
}

#[test]
fn dry_run_and_preview_perform_zero_writes_even_without_dist() {
    let temp = Temp::new();
    let vault = temp.vault();
    let mut metadata = seed(&vault, "preview", &basic("preview"));
    fs::remove_dir(vault.root.join("dist")).unwrap();
    let before = snapshot(&temp.0);
    let report = build::build(&vault, &[], true).unwrap();
    assert!(report.dry_run);
    assert!(report.items[0].changed);
    approved(&mut metadata, "Accepted preview");
    let overrides = BTreeMap::from([("preview".into(), metadata)]);
    let report = build::preview(&vault, &["preview".into()], &overrides).unwrap();
    assert!(report.dry_run);
    assert_eq!(snapshot(&temp.0), before);
}

#[test]
fn dropping_prepared_or_applied_batches_restores_old_targets_and_absence() {
    let temp = Temp::new();
    let vault = temp.vault();
    let mut first = seed(&vault, "first", &basic("first"));
    build::build(&vault, &[], false).unwrap();
    seed(&vault, "new", &basic("new"));
    approved(&mut first, "Replacement to roll back");
    let overrides = BTreeMap::from([("first".into(), first)]);
    let before = snapshot(&temp.0);
    {
        let _transaction = build::prepare(&vault, &[], &overrides).unwrap();
    }
    assert_eq!(snapshot(&temp.0), before);
    {
        let mut transaction = build::prepare(&vault, &[], &overrides).unwrap();
        transaction.apply().unwrap();
        assert!(vault.root.join("dist/new/SKILL.md").exists());
    }
    assert_eq!(snapshot(&temp.0), before);

    fs::remove_dir_all(vault.root.join("dist")).unwrap();
    let before = snapshot(&temp.0);
    {
        let mut transaction = build::prepare(&vault, &[], &BTreeMap::new()).unwrap();
        transaction.apply().unwrap();
    }
    assert_eq!(snapshot(&temp.0), before);
}

#[test]
fn concurrent_target_changes_fail_without_removing_foreign_content() {
    let temp = Temp::new();
    let vault = temp.vault();
    let mut first = seed(&vault, "first", &basic("first"));
    seed(&vault, "last", &basic("last"));
    build::build(&vault, &[], false).unwrap();
    approved(&mut first, "Accepted replacement");
    let overrides = BTreeMap::from([("first".into(), first)]);
    let before = snapshot(&temp.0);
    let mut transaction = build::prepare(&vault, &["first".into()], &overrides).unwrap();
    fs::write(
        vault.root.join("dist/first/user-data"),
        "concurrent content",
    )
    .unwrap();
    assert_eq!(transaction.apply().unwrap_err().code, "BUILD_CONFLICT");
    drop(transaction);
    fs::remove_file(vault.root.join("dist/first/user-data")).unwrap();
    assert_eq!(snapshot(&temp.0), before);
}

#[test]
fn failure_after_the_first_swap_restores_the_whole_batch() {
    let temp = Temp::new();
    let vault = temp.vault();
    let mut first = seed(&vault, "first", &basic("first"));
    let mut last = seed(&vault, "last", &basic("last"));
    build::build(&vault, &[], false).unwrap();
    approved(&mut first, "First replacement");
    approved(&mut last, "Last replacement");
    let overrides = BTreeMap::from([("first".into(), first), ("last".into(), last)]);
    let before = snapshot(&temp.0);
    let mut transaction = build::prepare(&vault, &[], &overrides).unwrap();
    // Inject a rename failure after the first target is installed. This also
    // exercises restoring the second target after its old version was moved.
    let stage = fs::read_dir(vault.root.join("dist"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".skillsbox-stage-")
        })
        .unwrap();
    fs::remove_dir_all(stage.join("new/last")).unwrap();
    assert!(transaction.apply().is_err());
    drop(transaction);
    assert_eq!(snapshot(&temp.0), before);
}

#[test]
fn foreign_and_modified_dist_targets_are_never_overwritten() {
    let temp = Temp::new();
    let vault = temp.vault();
    seed(&vault, "owned", &basic("owned"));
    fs::create_dir(vault.root.join("dist/owned")).unwrap();
    fs::write(vault.root.join("dist/owned/user.txt"), "foreign").unwrap();
    let before = snapshot(&temp.0);
    assert_eq!(
        build::build(&vault, &[], false).unwrap_err().code,
        "BUILD_CONFLICT"
    );
    assert_eq!(snapshot(&temp.0), before);
    fs::remove_dir_all(vault.root.join("dist/owned")).unwrap();
    build::build(&vault, &[], false).unwrap();
    fs::write(vault.root.join("dist/owned/SKILL.md"), "user modified").unwrap();
    let before = snapshot(&temp.0);
    assert_eq!(
        build::build(&vault, &[], false).unwrap_err().code,
        "BUILD_CONFLICT"
    );
    assert_eq!(snapshot(&temp.0), before);
}

#[test]
fn build_all_prunes_only_removed_owned_unchanged_targets_and_rollback_restores_them() {
    let temp = Temp::new();
    let vault = temp.vault();
    seed(&vault, "keep", &basic("keep"));
    seed(&vault, "removed", &basic("removed"));
    build::build(&vault, &[], false).unwrap();
    fs::remove_dir_all(vault.root.join("skills/removed")).unwrap();
    fs::remove_file(vault.root.join("meta/removed.toml")).unwrap();
    fs::create_dir(vault.root.join("dist/foreign")).unwrap();
    fs::write(vault.root.join("dist/foreign/user"), "preserve").unwrap();
    let before = snapshot(&temp.0);
    let dry_run = build::build(&vault, &[], true).unwrap();
    assert_eq!(dry_run.removed, ["removed"]);
    assert_eq!(snapshot(&temp.0), before);
    {
        let mut transaction = build::prepare(&vault, &[], &BTreeMap::new()).unwrap();
        transaction.apply().unwrap();
        assert!(!vault.root.join("dist/removed").exists());
    }
    assert_eq!(snapshot(&temp.0), before);
    let report = build::build(&vault, &[], false).unwrap();
    assert_eq!(report.removed, ["removed"]);
    assert!(!vault.root.join("dist/removed").exists());
    assert_eq!(
        fs::read(vault.root.join("dist/foreign/user")).unwrap(),
        b"preserve"
    );
    assert!(!build::build(&vault, &[], false).unwrap().items[0].changed);
}

#[test]
fn modified_removed_targets_are_preserved_instead_of_pruned() {
    let temp = Temp::new();
    let vault = temp.vault();
    seed(&vault, "removed", &basic("removed"));
    build::build(&vault, &[], false).unwrap();
    fs::remove_dir_all(vault.root.join("skills/removed")).unwrap();
    fs::remove_file(vault.root.join("meta/removed.toml")).unwrap();
    fs::write(vault.root.join("dist/removed/user-data"), "preserve").unwrap();
    let before = snapshot(&temp.0);
    assert_eq!(
        build::build(&vault, &[], false).unwrap_err().code,
        "BUILD_CONFLICT"
    );
    assert_eq!(snapshot(&temp.0), before);
}

#[cfg(unix)]
#[test]
fn executable_and_read_only_payloads_build_and_roll_back_without_leftover_stages() {
    use std::os::unix::fs::PermissionsExt;
    let temp = Temp::new();
    let vault = temp.vault();
    let mut metadata = seed(&vault, "permissions", &basic("permissions"));
    let scripts = vault.root.join("skills/permissions/scripts");
    fs::set_permissions(scripts.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(&scripts, fs::Permissions::from_mode(0o555)).unwrap();
    save_metadata(&vault, &metadata);
    build::build(&vault, &[], false).unwrap();
    assert_eq!(
        fs::metadata(vault.root.join("dist/permissions/scripts/run.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    assert_eq!(
        fs::metadata(vault.root.join("dist/permissions/scripts"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o555
    );
    approved(&mut metadata, "Replacement to roll back");
    let overrides = BTreeMap::from([("permissions".into(), metadata)]);
    let before = snapshot(&temp.0);
    {
        let mut transaction = build::prepare(&vault, &[], &overrides).unwrap();
        transaction.apply().unwrap();
    }
    assert_eq!(snapshot(&temp.0), before);
    // Restore source permissions so the fixture owner can remove its originals.
    fs::set_permissions(&scripts, fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(
        vault.root.join("dist/permissions/scripts"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
}

#[cfg(unix)]
#[test]
fn symlink_payload_dist_and_manifest_are_rejected_without_writes() {
    use std::os::unix::fs::symlink;
    let temp = Temp::new();
    let vault = temp.vault();
    seed(&vault, "links", &basic("links"));
    let outside = temp.0.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("content"), "preserve").unwrap();
    let link = vault.root.join("skills/links/auxiliary-link");
    symlink(outside.join("content"), &link).unwrap();
    let before = snapshot(&temp.0);
    assert!(build::build(&vault, &[], false).is_err());
    assert_eq!(snapshot(&temp.0), before);
    fs::remove_file(link).unwrap();
    symlink(&outside, vault.root.join("dist/links")).unwrap();
    let before = snapshot(&temp.0);
    assert!(build::build(&vault, &[], false).is_err());
    assert_eq!(snapshot(&temp.0), before);
    fs::remove_file(vault.root.join("dist/links")).unwrap();
    symlink(
        outside.join("content"),
        vault.root.join("dist/.skillsbox-build.toml"),
    )
    .unwrap();
    let before = snapshot(&temp.0);
    assert!(build::build(&vault, &[], false).is_err());
    assert_eq!(snapshot(&temp.0), before);
}

#[cfg(unix)]
#[test]
fn special_source_files_are_rejected_before_opening() {
    use std::os::unix::net::UnixListener;
    let temp = Temp::new();
    let vault = temp.vault();
    seed(&vault, "socket", &basic("socket"));
    let socket = vault.root.join("skills/socket/socket");
    let _listener = UnixListener::bind(&socket).unwrap();
    let report = build::build(&vault, &[], false).unwrap_err();
    assert_eq!(report.code, "UNSUPPORTED_FILE");
    assert!(
        fs::read_dir(vault.root.join("dist"))
            .unwrap()
            .next()
            .is_none()
    );
}
