mod support;

use kinana_core::{
    ActivateOptions, Bundle, BundleSkill, ImportOptions, activate, bundles, gitlog,
    hash::hash_directory, import, remove, vault::Vault,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        Self(support::temp_dir().unwrap())
    }

    fn vault(&self) -> Vault {
        let vault = Vault::new(self.0.join("vault"), self.0.join("cache/index.db"));
        vault.init(false).unwrap();
        for id in ["api", "frontend"] {
            let source = self.0.join(format!("source-{id}"));
            fs::create_dir_all(source.join("assets/empty/deep")).unwrap();
            fs::write(
                source.join("SKILL.md"),
                format!("---\r\nname: {id}\r\ndescription: Review API contracts\r\n---\r\nExact source bytes مرحبا\r\n"),
            )
            .unwrap();
            fs::write(source.join("assets/bytes.bin"), [0, 1, 255, 13, 10]).unwrap();
            import::add(
                &vault,
                source.to_str().unwrap(),
                &self.0.join("state"),
                &ImportOptions::default(),
            )
            .unwrap();
        }
        vault
    }

    fn activate(&self, vault: &Vault, id: &str) -> PathBuf {
        let project = self.0.join("project");
        fs::create_dir_all(&project).unwrap();
        activate::activate(
            vault,
            &ActivateOptions {
                project: project.clone(),
                home: self.0.clone(),
                selections: vec![id.into()],
                agent: "claude".into(),
                mode: "link".into(),
                include_secondary: vec![],
                dry_run: false,
            },
        )
        .unwrap();
        project
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _cleanup = fs::remove_dir_all(&self.0);
    }
}

fn bundle(vault: &Vault) -> Bundle {
    let bundle = Bundle {
        name: "web-app".into(),
        title: "تطبيق ويب".into(),
        agents: vec!["claude".into(), "codex".into()],
        skills: vec![
            BundleSkill {
                id: "frontend".into(),
                role: "primary".into(),
                group: Some("review".into()),
            },
            BundleSkill {
                id: "api".into(),
                role: "secondary".into(),
                group: Some("review".into()),
            },
        ],
    };
    bundles::save_definition(vault, None, bundle.clone(), false).unwrap();
    bundle
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, directory: &Path, entries: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for child in fs::read_dir(directory).unwrap() {
            let path = child.unwrap().path();
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
                walk(root, &path, entries);
            } else {
                entries.insert(relative, fs::read(&path).unwrap());
            }
        }
    }
    let mut entries = BTreeMap::new();
    walk(root, root, &mut entries);
    entries
}

/// Vault files only: Git's object store and the rebuildable SQLite cache are
/// compared through their own commands, not byte for byte.
fn vault_files(mut entries: BTreeMap<PathBuf, Vec<u8>>) -> BTreeMap<PathBuf, Vec<u8>> {
    entries.retain(|path, _| path.starts_with("vault") && !path.starts_with("vault/.git"));
    entries
}

fn git(vault: &Vault, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(&vault.root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn remove_commits_exactly_the_owned_paths_and_undo_restores_hashes_and_empty_directories() {
    let temp = Temp::new();
    let vault = temp.vault();
    let definition = bundle(&vault);
    let before = vault_files(snapshot(&temp.0));
    let source_hash = hash_directory(&vault.root.join("skills/api")).unwrap();
    let generated_hash = hash_directory(&vault.root.join("dist/api")).unwrap();
    let recorded = vault.show("api").unwrap().skill.content_hash;
    assert_eq!(recorded, source_hash);
    let head = gitlog::head(&vault.root).unwrap();

    let report = remove::remove(&vault, "api", false).unwrap();
    assert_eq!(report.id, "api");
    assert!(report.changed && report.committed && !report.dry_run);
    assert_eq!(
        report.removed_paths,
        ["skills/api", "meta/api.toml", "dist/api"]
    );
    assert_eq!(report.bundles_changed, ["web-app"]);
    assert_eq!(
        git(&vault, &["log", "-1", "--format=%s"]),
        "Remove skill api\n"
    );
    assert_eq!(
        git(&vault, &["rev-parse", "HEAD~1"]).trim(),
        head,
        "one commit on top of the previous head"
    );
    assert_eq!(
        git(
            &vault,
            &["diff-tree", "--no-commit-id", "--name-status", "-r", "HEAD"]
        ),
        "M\tbundles/web-app.toml\nD\tmeta/api.toml\nD\tskills/api/SKILL.md\nD\tskills/api/assets/bytes.bin\n"
    );
    gitlog::ensure_clean(&vault.root).unwrap();
    assert!(!vault.root.join("skills/api").exists());
    assert!(!vault.root.join("dist/api").exists());
    assert_eq!(vault.records().unwrap().len(), 1);
    let mut expected = definition.clone();
    expected.skills.retain(|member| member.id != "api");
    assert_eq!(bundles::read(&vault, "web-app").unwrap(), expected);
    assert!(vault.show("api").is_err());
    // The other skill and its generated copy are untouched.
    let mut untouched = before.clone();
    untouched.retain(|path, _| {
        path.starts_with("vault/skills/frontend")
            || path.starts_with("vault/dist/frontend")
            || path.starts_with("vault/meta/frontend.toml")
    });
    let after = vault_files(snapshot(&temp.0));
    for (path, bytes) in &untouched {
        assert_eq!(after.get(path), Some(bytes), "{}", path.display());
    }

    vault.undo(false).unwrap();
    assert_eq!(vault_files(snapshot(&temp.0)), before);
    assert_eq!(
        hash_directory(&vault.root.join("skills/api")).unwrap(),
        source_hash
    );
    assert_eq!(
        hash_directory(&vault.root.join("dist/api")).unwrap(),
        generated_hash
    );
    assert_eq!(vault.show("api").unwrap().skill.content_hash, recorded);
    assert!(vault.root.join("skills/api/assets/empty/deep").is_dir());
    assert_eq!(bundles::read(&vault, "web-app").unwrap(), definition);
    gitlog::ensure_clean(&vault.root).unwrap();
}

#[test]
fn dry_run_reports_the_plan_with_zero_writes() {
    let temp = Temp::new();
    let vault = temp.vault();
    bundle(&vault);
    let before = snapshot(&temp.0);
    let report = remove::remove(&vault, "api", true).unwrap();
    assert!(report.dry_run && report.changed && !report.committed);
    assert_eq!(
        report.removed_paths,
        ["skills/api", "meta/api.toml", "dist/api"]
    );
    assert_eq!(report.bundles_changed, ["web-app"]);
    assert_eq!(snapshot(&temp.0), before);
    let report = bundles::delete(&vault, "web-app", true).unwrap();
    assert_eq!(report.name, "web-app");
    assert!(report.dry_run && report.changed && !report.committed);
    assert_eq!(snapshot(&temp.0), before);
}

#[test]
fn removing_a_groups_only_primary_is_refused_instead_of_writing_an_invalid_bundle() {
    let temp = Temp::new();
    let vault = temp.vault();
    bundle(&vault);
    let before = snapshot(&temp.0);
    for dry_run in [true, false] {
        let error = remove::remove(&vault, "frontend", dry_run).unwrap_err();
        assert_eq!(error.code, "BUNDLE_WOULD_BE_INVALID");
        assert!(error.needs_decision);
        assert!(error.message.contains("web-app"), "{}", error.message);
        assert_eq!(snapshot(&temp.0), before);
    }
}

#[test]
fn crafted_ids_never_reach_the_filesystem() {
    let temp = Temp::new();
    let vault = temp.vault();
    let outside = temp.0.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("SKILL.md"), "owner data").unwrap();
    fs::write(temp.0.join("outside.toml"), "owner data").unwrap();
    let before = snapshot(&temp.0);
    let absolute = outside.to_str().unwrap().to_owned();
    for id in [
        "",
        ".",
        "..",
        "../outside",
        "../../outside",
        "api/..",
        "api/../frontend",
        "api\\..\\frontend",
        "api\0",
        "API",
        ".git",
        "-api",
        absolute.as_str(),
    ] {
        for dry_run in [true, false] {
            assert_eq!(
                remove::remove(&vault, id, dry_run).unwrap_err().code,
                "INVALID_SKILL_ID",
                "{id:?}"
            );
            assert_eq!(
                bundles::delete(&vault, id, dry_run).unwrap_err().code,
                "INVALID_BUNDLE_NAME",
                "{id:?}"
            );
        }
    }
    assert_eq!(
        remove::remove(&vault, "absent", false).unwrap_err().code,
        "SKILL_NOT_FOUND"
    );
    assert_eq!(
        bundles::delete(&vault, "absent", false).unwrap_err().code,
        "BUNDLE_NOT_FOUND"
    );
    assert_eq!(snapshot(&temp.0), before);
}

#[cfg(unix)]
#[test]
fn links_are_refused_and_their_targets_survive() {
    use std::os::unix::fs::symlink;
    let temp = Temp::new();
    let vault = temp.vault();
    bundle(&vault);
    let outside = temp.0.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep.txt"), "owner data").unwrap();

    // The vault itself reached through a link.
    let alias = temp.0.join("alias");
    symlink(&vault.root, &alias).unwrap();
    let linked = Vault::new(alias.clone(), vault.index_path.clone());
    let before = snapshot(&temp.0);
    assert_eq!(
        remove::remove(&linked, "api", false).unwrap_err().code,
        "SYMLINK_PATH"
    );
    assert_eq!(
        bundles::delete(&linked, "web-app", false).unwrap_err().code,
        "SYMLINK_PATH"
    );
    assert_eq!(snapshot(&temp.0), before);
    fs::remove_file(&alias).unwrap();

    // The ignored generated copy replaced by a link out of the vault. Git
    // does not see dist/, so only the removal's own guards protect the target.
    let generated = vault.root.join("dist/api");
    let kept = temp.0.join("kept-dist");
    fs::rename(&generated, &kept).unwrap();
    symlink(&outside, &generated).unwrap();
    let before = snapshot(&temp.0);
    for dry_run in [true, false] {
        let error = remove::remove(&vault, "api", dry_run).unwrap_err();
        assert_eq!(error.code, "SYMLINK_PATH", "{}", error.message);
        assert_eq!(snapshot(&temp.0), before);
    }
    fs::remove_file(&generated).unwrap();

    // A link nested inside the generated copy.
    fs::rename(&kept, &generated).unwrap();
    symlink(&outside, generated.join("assets/link")).unwrap();
    let before = snapshot(&temp.0);
    for dry_run in [true, false] {
        remove::remove(&vault, "api", dry_run).unwrap_err();
        assert_eq!(snapshot(&temp.0), before);
    }
    fs::remove_file(generated.join("assets/link")).unwrap();

    // The private staging parent replaced by a link.
    let dist = vault.root.join("dist");
    let kept = temp.0.join("kept-dist-root");
    fs::rename(&dist, &kept).unwrap();
    symlink(&kept, &dist).unwrap();
    let before = snapshot(&temp.0);
    assert_eq!(
        remove::remove(&vault, "api", false).unwrap_err().code,
        "SYMLINK_PATH"
    );
    assert_eq!(
        bundles::delete(&vault, "web-app", false).unwrap_err().code,
        "SYMLINK_PATH"
    );
    assert_eq!(snapshot(&temp.0), before);
    fs::remove_file(&dist).unwrap();
    fs::rename(&kept, &dist).unwrap();

    assert!(remove::remove(&vault, "api", false).unwrap().committed);
    assert_eq!(
        fs::read_to_string(outside.join("keep.txt")).unwrap(),
        "owner data"
    );
}

#[test]
fn use_is_read_from_registered_locks_and_an_unreadable_lock_fails_closed() {
    let temp = Temp::new();
    let vault = temp.vault();
    let project = temp.activate(&vault, "api");
    let before = snapshot(&temp.0);
    for dry_run in [true, false] {
        let error = remove::remove(&vault, "api", dry_run).unwrap_err();
        assert_eq!(error.code, "SKILL_IN_USE");
        assert!(error.needs_decision);
        assert!(
            error.message.contains(project.to_str().unwrap()),
            "{}",
            error.message
        );
        assert_eq!(snapshot(&temp.0), before);
    }
    // A skill that is not active there is removable while the other stays in use.
    assert!(remove::remove(&vault, "frontend", true).unwrap().changed);

    // Non-use cannot be proven from a damaged lock, so nothing is deleted.
    let lock = project.join(".kinana.lock");
    let saved = fs::read(&lock).unwrap();
    fs::write(&lock, "not a lock [").unwrap();
    let before = snapshot(&temp.0);
    for id in ["api", "frontend"] {
        let error = remove::remove(&vault, id, false).unwrap_err();
        assert_ne!(error.code, "SKILL_IN_USE");
        assert!(
            error.message.contains(project.to_str().unwrap()),
            "{}",
            error.message
        );
        assert_eq!(snapshot(&temp.0), before);
    }
    fs::write(&lock, saved).unwrap();

    // A registered project that no longer exists records no activation.
    fs::remove_dir_all(&project).unwrap();
    let report = remove::remove(&vault, "api", false).unwrap();
    assert!(report.committed);
    assert!(!vault.root.join("skills/api").exists());
}

#[test]
fn another_spelling_of_the_same_vault_directory_still_sees_the_activation() {
    let temp = Temp::new();
    let vault = temp.vault();
    let project = temp.activate(&vault, "api");
    let alias = temp.0.join("VAULT");
    let same = fs::metadata(&alias).is_ok_and(|metadata| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            metadata.ino() == fs::metadata(&vault.root).unwrap().ino()
        }
        #[cfg(not(unix))]
        {
            let _ = metadata;
            false
        }
    });
    if !same {
        eprintln!(
            "SKIP another_spelling_of_the_same_vault_directory_still_sees_the_activation: filesystem is case-sensitive; the lexical comparison already covers it."
        );
        return;
    }
    let aliased = Vault::new(alias, vault.index_path.clone());
    let before = snapshot(&temp.0);
    for dry_run in [true, false] {
        let error = remove::remove(&aliased, "api", dry_run).unwrap_err();
        assert_eq!(error.code, "SKILL_IN_USE", "{}", error.message);
        assert!(error.message.contains(project.to_str().unwrap()));
        assert_eq!(snapshot(&temp.0), before);
    }
}

#[test]
fn bundle_delete_commits_only_the_bundle_file_and_undo_restores_its_bytes() {
    let temp = Temp::new();
    let vault = temp.vault();
    bundle(&vault);
    let before = vault_files(snapshot(&temp.0));
    let report = bundles::delete(&vault, "web-app", false).unwrap();
    assert_eq!(report.name, "web-app");
    assert!(report.changed && report.committed && !report.dry_run);
    assert_eq!(
        git(&vault, &["log", "-1", "--format=%s"]),
        "Delete bundle web-app\n"
    );
    assert_eq!(
        git(
            &vault,
            &["diff-tree", "--no-commit-id", "--name-status", "-r", "HEAD"]
        ),
        "D\tbundles/web-app.toml\n"
    );
    assert!(bundles::list(&vault).unwrap().is_empty());
    assert_eq!(
        bundles::read(&vault, "web-app").unwrap_err().code,
        "BUNDLE_NOT_FOUND"
    );
    let mut expected = before.clone();
    expected.remove(Path::new("vault/bundles/web-app.toml"));
    assert_eq!(vault_files(snapshot(&temp.0)), expected);
    gitlog::ensure_clean(&vault.root).unwrap();

    vault.undo(false).unwrap();
    assert_eq!(vault_files(snapshot(&temp.0)), before);
    gitlog::ensure_clean(&vault.root).unwrap();
}
