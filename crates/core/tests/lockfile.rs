mod support;

use kinana_core::{LockedSkill, ProjectLock, frontmatter, lockfile};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

struct Temp {
    root: PathBuf,
    project: PathBuf,
    vault: PathBuf,
}
impl Temp {
    fn new(staging: bool) -> Self {
        let root = support::temp_dir().unwrap();
        let project = root.join("project");
        let vault = root.join("vault");
        fs::create_dir(&project).unwrap();
        fs::create_dir(&vault).unwrap();
        if staging {
            for prefix in [".claude/skills", ".agents/skills"] {
                fs::create_dir_all(project.join(prefix)).unwrap();
            }
        }
        Self {
            root,
            project,
            vault,
        }
    }
    fn lock(&self) -> ProjectLock {
        ProjectLock {
            restoration: None,
            vault: self.vault.clone(),
            bundle: Some("web-app".into()),
            active: vec![
                skill("api", "claude", "API Review"),
                skill("frontend", "codex", "Frontend Review"),
            ],
        }
    }
    fn path(&self) -> PathBuf {
        self.project.join(".kinana.lock")
    }
    fn stages(&self) -> Vec<PathBuf> {
        fs::read_dir(self.project.join(".claude/skills"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".skillsbox-lock-")
            })
            .collect()
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _cleanup = fs::remove_dir_all(&self.root);
    }
}

fn skill(id: &str, agent: &str, name: &str) -> LockedSkill {
    LockedSkill {
        target_root: None,
        generation_hash: None,
        ownership: None,
        id: id.into(),
        agent: agent.into(),
        mode: "link".into(),
        name: name.into(),
        invocation: "auto".into(),
        content_hash: format!("blake3:{}", "a".repeat(64)),
    }
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, directory: &Path, entries: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            let metadata = fs::symlink_metadata(&path).unwrap();
            if metadata.file_type().is_symlink() {
                entries.insert(
                    relative,
                    fs::read_link(&path)
                        .unwrap()
                        .as_os_str()
                        .as_encoded_bytes()
                        .to_vec(),
                );
            } else if metadata.is_dir() {
                entries.insert(relative, b"directory".to_vec());
                visit(root, &path, entries);
            } else if metadata.is_file() {
                entries.insert(relative, fs::read(&path).unwrap());
            } else {
                entries.insert(relative, b"special file".to_vec());
            }
        }
    }
    let mut entries = BTreeMap::new();
    visit(root, root, &mut entries);
    entries
}

#[test]
fn lock_roundtrip_uses_the_fixed_file_and_never_creates_activation_targets() {
    let temp = Temp::new(true);
    let mut lock = temp.lock();
    lock.active[0].mode = "copy".into();
    lock.active[0].invocation = "manual".into();
    assert!(lockfile::read(&temp.project).unwrap().is_none());
    let report = lockfile::write(&temp.project, &temp.vault, &lock, "claude", false).unwrap();
    assert_eq!(report.path, temp.path());
    assert_eq!(report.lock, lock);
    assert!(report.changed && !report.dry_run && report.warnings.is_empty());
    assert_eq!(lockfile::read(&temp.project).unwrap(), Some(lock.clone()));
    lockfile::validate_for_vault(&temp.project, &temp.vault, &lock).unwrap();
    assert!(temp.stages().is_empty());
    assert!(!temp.project.join(".claude/skills/API Review").exists());
    assert!(!temp.project.join(".agents/skills/Frontend Review").exists());
    assert!(!temp.project.join(".git").exists());
    assert!(!temp.project.join(".gitignore").exists());
    assert!(fs::read_dir(&temp.vault).unwrap().next().is_none());
}

#[test]
fn previews_and_semantic_noops_preserve_exact_bytes_and_metadata() {
    let temp = Temp::new(true);
    let lock = temp.lock();
    let before = snapshot(&temp.root);
    let report = lockfile::write(&temp.project, &temp.vault, &lock, "codex", true).unwrap();
    assert!(report.dry_run && report.changed);
    assert_eq!(snapshot(&temp.root), before);
    let text = format!(
        "# Owner formatting\r\n{}",
        toml::to_string_pretty(&lock)
            .unwrap()
            .replace("invocation = \"auto\"\n", "")
            .replace('\n', "\r\n")
    );
    fs::write(temp.path(), &text).unwrap();
    let modified = fs::metadata(temp.path()).unwrap().modified().unwrap();
    let before = snapshot(&temp.root);
    assert_eq!(lockfile::read(&temp.project).unwrap(), Some(lock.clone()));
    let report = lockfile::write(&temp.project, &temp.vault, &lock, "claude", false).unwrap();
    assert!(!report.changed);
    assert_eq!(snapshot(&temp.root), before);
    assert_eq!(fs::read(temp.path()).unwrap(), text.as_bytes());
    assert_eq!(
        fs::metadata(temp.path()).unwrap().modified().unwrap(),
        modified
    );
    let mut changed = lock;
    changed.bundle = None;
    assert!(
        lockfile::write(&temp.project, &temp.vault, &changed, "claude", true)
            .unwrap()
            .changed
    );
    assert_eq!(snapshot(&temp.root), before);
}

#[test]
fn missing_staging_roots_never_create_agent_directories_even_for_dry_runs() {
    let temp = Temp::new(false);
    let lock = temp.lock();
    let before = snapshot(&temp.root);
    for agent in ["claude", "codex", "cursor"] {
        for dry in [true, false] {
            assert_eq!(
                lockfile::write(&temp.project, &temp.vault, &lock, agent, dry)
                    .unwrap_err()
                    .code,
                "LOCK_STAGING_ROOT_MISSING"
            );
            assert_eq!(snapshot(&temp.root), before);
        }
    }
    assert!(!temp.project.join(".claude").exists());
    assert!(!temp.project.join(".agents").exists());
}

#[test]
fn validation_rejects_traversal_bad_fields_duplicate_identities_and_duplicate_targets() {
    let temp = Temp::new(true);
    let valid = temp.lock();
    let mut cases = Vec::new();
    for vault in [PathBuf::from("relative"), temp.vault.join("../vault")] {
        let mut lock = valid.clone();
        lock.vault = vault;
        cases.push(lock);
    }
    for bundle in ["", "../outside", "bad/name"] {
        let mut lock = valid.clone();
        lock.bundle = Some(bundle.into());
        cases.push(lock);
    }
    for id in ["", "../api", "bad/id"] {
        let mut lock = valid.clone();
        lock.active[0].id = id.into();
        cases.push(lock);
    }
    for name in [
        "",
        ".",
        "..",
        "../outside",
        "bad/name",
        "bad\\name",
        "bad\nname",
    ] {
        let mut lock = valid.clone();
        lock.active[0].name = name.into();
        cases.push(lock);
    }
    for agent in ["unknown", "Claude", "../claude"] {
        let mut lock = valid.clone();
        lock.active[0].agent = agent.into();
        cases.push(lock);
    }
    for mode in ["", "move", "Link"] {
        let mut lock = valid.clone();
        lock.active[0].mode = mode.into();
        cases.push(lock);
    }
    for invocation in ["", "automatic", "Manual"] {
        let mut lock = valid.clone();
        lock.active[0].invocation = invocation.into();
        cases.push(lock);
    }
    for digest in [
        "blake3:short".to_string(),
        "a".repeat(64),
        format!("blake3:{}", "g".repeat(64)),
        format!("blake3:{}", "a".repeat(65)),
    ] {
        let mut lock = valid.clone();
        lock.active[0].content_hash = digest;
        cases.push(lock);
    }
    let mut duplicate = valid.clone();
    duplicate.active.push(duplicate.active[0].clone());
    cases.push(duplicate);
    let mut duplicate_target = valid.clone();
    let mut other = duplicate_target.active[0].clone();
    other.id = "different".into();
    duplicate_target.active.push(other);
    cases.push(duplicate_target);
    for lock in cases {
        let before = snapshot(&temp.root);
        assert!(lockfile::validate(&lock).is_err());
        assert!(lockfile::write(&temp.project, &temp.vault, &lock, "claude", false).is_err());
        assert_eq!(snapshot(&temp.root), before);
    }
    let mut manual = valid;
    manual.active[1].invocation = "manual".into();
    let error = lockfile::validate(&manual).unwrap_err();
    assert_eq!(error.code, "CODEX_MANUAL_UNVERIFIED");
    assert!(error.needs_decision);
}

#[test]
fn manifest_names_and_agent_prefixes_are_used_exactly_without_id_slug_substitution() {
    let temp = Temp::new(true);
    let entry = skill("safe-id", "claude", "Mixed Case [tools] مرحبا");
    let text = format!(
        "---\nname: '{}'\ndescription: Static data\n---\n",
        entry.name
    );
    assert_eq!(frontmatter::parse_skill(&text).unwrap().name, entry.name);
    assert_eq!(
        lockfile::target_path(&temp.project, &entry).unwrap(),
        temp.project.join(".claude/skills").join(&entry.name)
    );
    let mut second = entry.clone();
    second.agent = "codex".into();
    assert_eq!(
        lockfile::target_path(&temp.project, &second).unwrap(),
        temp.project.join(".agents/skills").join(&entry.name)
    );
    let lock = ProjectLock {
        restoration: None,
        vault: temp.vault.clone(),
        bundle: None,
        active: vec![entry, second],
    };
    lockfile::validate(&lock).unwrap();
}

#[test]
fn foreign_and_corrupt_existing_locks_are_never_overwritten() {
    let temp = Temp::new(true);
    let lock = temp.lock();
    let mut foreign = lock.clone();
    foreign.vault = temp.root.join("other-vault");
    fs::write(temp.path(), toml::to_string(&foreign).unwrap()).unwrap();
    let before = snapshot(&temp.root);
    let error = lockfile::write(&temp.project, &temp.vault, &lock, "claude", false).unwrap_err();
    assert_eq!(error.code, "FOREIGN_PROJECT_LOCK");
    assert!(error.needs_decision);
    assert_eq!(snapshot(&temp.root), before);
    for text in [
        "not TOML".into(),
        toml::to_string(&lock)
            .unwrap()
            .replace("mode = \"link\"", "mode = \"unsafe\""),
        toml::to_string(&lock).unwrap().replace(
            "mode = \"link\"",
            "mode = \"link\"\ntarget_root = '../outside'",
        ),
    ] {
        fs::write(temp.path(), text).unwrap();
        let before = snapshot(&temp.root);
        assert_eq!(
            lockfile::read(&temp.project).unwrap_err().code,
            "INVALID_PROJECT_LOCK"
        );
        assert!(lockfile::write(&temp.project, &temp.vault, &lock, "claude", false).is_err());
        assert_eq!(snapshot(&temp.root), before);
    }
}

#[test]
fn future_extensions_survive_actual_updates_and_never_supply_activation_paths() {
    let temp = Temp::new(true);
    let lock = temp.lock();
    let mut doc = toml::Value::try_from(&lock).unwrap();
    doc.as_table_mut().unwrap().insert(
        "future".into(),
        toml::Value::String("keep this extension".into()),
    );
    doc["active"][0]
        .as_table_mut()
        .unwrap()
        .insert("path".into(), toml::Value::String("../outside".into()));
    fs::write(temp.path(), toml::to_string_pretty(&doc).unwrap()).unwrap();
    let outside = temp.root.join("outside");
    fs::write(&outside, "foreign content").unwrap();
    assert_eq!(lockfile::read(&temp.project).unwrap(), Some(lock.clone()));
    let mut updated = lock.clone();
    updated.bundle = Some("different-bundle".into());
    lockfile::write(&temp.project, &temp.vault, &updated, "claude", false).unwrap();
    let stored: toml::Value = toml::from_str(&fs::read_to_string(temp.path()).unwrap()).unwrap();
    assert_eq!(stored["future"].as_str(), Some("keep this extension"));
    assert_eq!(stored["active"][0]["path"].as_str(), Some("../outside"));
    assert_eq!(fs::read_to_string(outside).unwrap(), "foreign content");
    assert_eq!(
        lockfile::target_path(&temp.project, &updated.active[0]).unwrap(),
        temp.project.join(".claude/skills/API Review")
    );
}

#[test]
fn dropping_prepared_and_published_operations_restores_prior_lock_or_absence() {
    let temp = Temp::new(true);
    let lock = temp.lock();
    let before = snapshot(&temp.root);
    {
        let _prepared = lockfile::prepare(&temp.project, &temp.vault, &lock, "claude").unwrap();
    }
    assert_eq!(snapshot(&temp.root), before);
    {
        let mut prepared = lockfile::prepare(&temp.project, &temp.vault, &lock, "claude").unwrap();
        prepared.publish().unwrap();
        assert!(temp.path().exists());
    }
    assert_eq!(snapshot(&temp.root), before);
    lockfile::write(&temp.project, &temp.vault, &lock, "claude", false).unwrap();
    let before = snapshot(&temp.root);
    let mut replacement = lock;
    replacement.bundle = None;
    {
        let mut prepared =
            lockfile::prepare(&temp.project, &temp.vault, &replacement, "claude").unwrap();
        prepared.publish().unwrap();
    }
    assert_eq!(snapshot(&temp.root), before);
}

#[test]
fn late_replacement_is_rejected_even_when_the_foreign_inode_has_identical_bytes() {
    let temp = Temp::new(true);
    let lock = temp.lock();
    lockfile::write(&temp.project, &temp.vault, &lock, "claude", false).unwrap();
    let bytes = fs::read(temp.path()).unwrap();
    let mut replacement = lock;
    replacement.bundle = None;
    let mut prepared =
        lockfile::prepare(&temp.project, &temp.vault, &replacement, "claude").unwrap();
    fs::rename(temp.path(), temp.project.join("owner-old-lock")).unwrap();
    fs::write(temp.path(), &bytes).unwrap();
    assert_eq!(prepared.publish().unwrap_err().code, "PROJECT_LOCK_CHANGED");
    drop(prepared);
    assert_eq!(fs::read(temp.path()).unwrap(), bytes);
    assert!(temp.stages().is_empty());
    assert!(temp.project.join("owner-old-lock").is_file());
}

#[test]
fn success_preserves_modified_old_backup_and_reports_its_path() {
    let temp = Temp::new(true);
    let lock = temp.lock();
    lockfile::write(&temp.project, &temp.vault, &lock, "claude", false).unwrap();
    let mut replacement = lock;
    replacement.bundle = None;
    let mut prepared =
        lockfile::prepare(&temp.project, &temp.vault, &replacement, "claude").unwrap();
    prepared.publish().unwrap();
    let stage = temp.stages().pop().unwrap();
    fs::write(stage.join("old.lock"), "concurrent writer data").unwrap();
    let report = prepared.finish().unwrap();
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains(stage.to_str().unwrap()))
    );
    assert_eq!(
        fs::read(stage.join("old.lock")).unwrap(),
        b"concurrent writer data"
    );
    assert_eq!(lockfile::read(&temp.project).unwrap(), Some(replacement));
    assert!(!stage.join("new.lock").exists());
}

#[cfg(unix)]
#[test]
fn read_only_old_lock_is_restored_by_inode_without_writing_through_it() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let temp = Temp::new(true);
    let lock = temp.lock();
    lockfile::write(&temp.project, &temp.vault, &lock, "claude", false).unwrap();
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o444)).unwrap();
    let metadata = fs::metadata(temp.path()).unwrap();
    let bytes = fs::read(temp.path()).unwrap();
    let mut replacement = lock;
    replacement.bundle = None;
    {
        let mut prepared =
            lockfile::prepare(&temp.project, &temp.vault, &replacement, "claude").unwrap();
        prepared.publish().unwrap();
    }
    let restored = fs::metadata(temp.path()).unwrap();
    assert_eq!(restored.ino(), metadata.ino());
    assert_eq!(restored.permissions().mode() & 0o777, 0o444);
    assert_eq!(fs::read(temp.path()).unwrap(), bytes);
    assert!(temp.stages().is_empty());
}

#[cfg(unix)]
#[test]
fn final_target_links_are_left_untouched_but_parent_and_lock_links_are_rejected() {
    use std::os::unix::fs::symlink;
    let temp = Temp::new(true);
    let lock = temp.lock();
    let outside = temp.root.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), "untouched").unwrap();
    let target = temp.project.join(".claude/skills/API Review");
    symlink(&outside, &target).unwrap();
    let before = snapshot(&temp.root);
    assert_eq!(
        lockfile::target_path(&temp.project, &lock.active[0]).unwrap(),
        target
    );
    lockfile::validate_for_vault(&temp.project, &temp.vault, &lock).unwrap();
    assert_eq!(snapshot(&temp.root), before);
    symlink(outside.join("keep"), temp.path()).unwrap();
    let before = snapshot(&temp.root);
    assert_eq!(
        lockfile::read(&temp.project).unwrap_err().code,
        "SYMLINK_PATH"
    );
    assert!(lockfile::write(&temp.project, &temp.vault, &lock, "claude", false).is_err());
    assert_eq!(snapshot(&temp.root), before);
    fs::remove_file(temp.path()).unwrap();
    fs::remove_dir_all(temp.project.join(".agents")).unwrap();
    symlink(&outside, temp.project.join(".agents")).unwrap();
    let before = snapshot(&temp.root);
    assert_eq!(
        lockfile::target_path(&temp.project, &lock.active[1])
            .unwrap_err()
            .code,
        "SYMLINK_PATH"
    );
    assert!(lockfile::write(&temp.project, &temp.vault, &lock, "claude", false).is_err());
    assert_eq!(snapshot(&temp.root), before);
    let linked_project = temp.root.join("project-link");
    symlink(&temp.project, &linked_project).unwrap();
    assert_eq!(
        lockfile::read(&linked_project).unwrap_err().code,
        "SYMLINK_PATH"
    );
}

#[cfg(unix)]
#[test]
fn special_lock_files_and_linked_vault_paths_are_rejected_without_opening_them() {
    use std::os::unix::{fs::symlink, net::UnixListener};
    let temp = Temp::new(true);
    let mut lock = temp.lock();
    let _listener = UnixListener::bind(temp.path()).unwrap();
    let before = snapshot(&temp.root);
    assert_eq!(
        lockfile::read(&temp.project).unwrap_err().code,
        "INVALID_PROJECT_LOCK"
    );
    assert!(lockfile::write(&temp.project, &temp.vault, &lock, "claude", false).is_err());
    assert_eq!(snapshot(&temp.root), before);
    let link = temp.root.join("vault-link");
    symlink(&temp.vault, &link).unwrap();
    lock.vault = link;
    assert_eq!(lockfile::validate(&lock).unwrap_err().code, "SYMLINK_PATH");
}
