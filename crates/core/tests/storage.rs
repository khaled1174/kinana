use kinana_core::{ListFilter, SkillRecord, SourceInfo, index, vault::Vault};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "kinana-storage-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed),
            ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn vault(&self) -> Vault {
        Vault::new(self.0.join("vault"), self.0.join("cache").join("index.db"))
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _cleanup = fs::remove_dir_all(&self.0);
    }
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, at: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(at).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            if metadata.file_type().is_symlink() {
                files.insert(
                    relative,
                    fs::read_link(&path)
                        .unwrap()
                        .as_os_str()
                        .as_encoded_bytes()
                        .to_vec(),
                );
            } else if metadata.is_dir() {
                files.insert(relative, b"directory".to_vec());
                walk(root, &path, files);
            } else {
                files.insert(relative, fs::read(path).unwrap());
            }
        }
    }
    let mut files = BTreeMap::new();
    walk(root, root, &mut files);
    files
}

fn seed_skill(vault: &Vault) {
    let skill = vault.root.join("skills/api-review");
    fs::create_dir(&skill).unwrap();
    fs::write(skill.join("SKILL.md"), "---\nname: api-review\ndescription: Reviews API contracts.\n---\nRaw <script>do_not_run()</script>\n").unwrap();
    fs::write(
        vault.root.join("meta/api-review.toml"),
        r#"
id = "api-review"
content_hash = "blake3:recorded"
added_at = "2026-11-13T09:00:00Z"
[source]
kind = "scan"
found_in = ["/known/project/.agents/skills/api-review"]
[classification]
category = "backend-api"
role = "Review contracts"
tags = ["rest", "review"]
[description]
original = "Reviews API contracts."
[audit]
level = "review"
findings = ["Script present"]
"#,
    )
    .unwrap();
}

#[test]
fn dry_run_is_a_plan_with_zero_filesystem_mutations() {
    let temp = Temp::new();
    let vault = temp.vault();
    let before = snapshot(&temp.0);
    let report = vault.init(true).unwrap();
    assert!(report.dry_run);
    assert!(!report.created);
    assert!(!report.planned_actions.is_empty());
    assert_eq!(snapshot(&temp.0), before);
    assert!(!vault.root.exists());
    assert!(!vault.index_path.exists());
}

#[test]
fn init_creates_requested_layout_and_is_idempotent() {
    let temp = Temp::new();
    let vault = temp.vault();
    assert!(vault.init(false).unwrap().created);
    for directory in ["skills", "meta", "bundles", "dist", ".git"] {
        assert!(vault.root.join(directory).is_dir());
    }
    assert_eq!(
        fs::read_to_string(vault.root.join(".gitignore")).unwrap(),
        "dist/\n.DS_Store\n"
    );
    assert!(vault.index_path.is_file());
    assert!(!vault.root.join("index.db").exists());
    index::check(&vault.index_path).unwrap();
    seed_skill(&vault);
    let config_path = vault.root.join("box.toml");
    let config = fs::read_to_string(&config_path).unwrap();
    fs::write(&config_path, format!("# User-owned comment\n{config}")).unwrap();
    let before = snapshot(&temp.0);
    let report = vault.init(false).unwrap();
    assert!(!report.created);
    assert!(report.planned_actions.is_empty());
    assert_eq!(snapshot(&temp.0), before);
    vault.init(true).unwrap();
    assert_eq!(snapshot(&temp.0), before);
}

#[test]
fn init_rejects_foreign_directories_and_files_without_changes() {
    let temp = Temp::new();
    let vault = temp.vault();
    fs::create_dir(&vault.root).unwrap();
    fs::write(vault.root.join("keep.txt"), "user data").unwrap();
    let before = snapshot(&temp.0);
    let conflict = vault.init(false).unwrap_err();
    assert_eq!(conflict.code, "VAULT_CONFLICT");
    assert!(conflict.needs_decision);
    assert!(vault.init(true).unwrap_err().needs_decision);
    assert_eq!(snapshot(&temp.0), before);

    let other = Vault::new(temp.0.join("new-vault"), temp.0.join("foreign.db"));
    fs::write(&other.index_path, "Not a Kinana index").unwrap();
    let before = snapshot(&temp.0);
    let conflict = other.init(false).unwrap_err();
    assert_eq!(conflict.code, "INDEX_CONFLICT");
    assert!(conflict.needs_decision);
    assert!(!other.root.exists());
    assert_eq!(snapshot(&temp.0), before);
}

#[test]
fn files_are_authoritative_for_list_and_raw_show() {
    let temp = Temp::new();
    let vault = temp.vault();
    vault.init(false).unwrap();
    seed_skill(&vault);
    assert_eq!(vault.records().unwrap().len(), 1);
    let detail = vault.show("api-review").unwrap();
    assert!(
        detail
            .raw_skill_md
            .contains("<script>do_not_run()</script>")
    );
    assert_eq!(
        detail.locations,
        vec![PathBuf::from("/known/project/.agents/skills/api-review")]
    );
    assert!(detail.variants.is_empty());
    assert_eq!(detail.metadata.audit.findings, ["Script present"]);

    let filter = ListFilter {
        pinned: false,
        category: Some("backend-api".into()),
        stack: None,
        stage: None,
        audit: Some("review".into()),
        agent: Some("codex".into()),
    };
    assert_eq!(vault.list(&filter).unwrap().len(), 1);
    assert!(
        vault
            .list(&ListFilter {
                agent: Some("unknown".into()),
                ..ListFilter::default()
            })
            .is_err()
    );
    assert!(
        vault
            .list(&ListFilter {
                category: Some("mobile".into()),
                ..ListFilter::default()
            })
            .unwrap()
            .is_empty()
    );

    let metadata_path = vault.root.join("meta/api-review.toml");
    let metadata = fs::read_to_string(&metadata_path)
        .unwrap()
        .replace("category = \"backend-api\"", "category = \"security\"");
    fs::write(&metadata_path, metadata).unwrap();
    // The index is still empty, and a manual edit appears immediately.
    assert_eq!(vault.records().unwrap()[0].category, "security");
    fs::remove_file(&vault.index_path).unwrap();
    assert_eq!(
        vault.show("api-review").unwrap().skill.content_hash,
        "blake3:recorded"
    );
}

#[test]
fn finder_inventory_artifacts_are_ignored_and_preserved() {
    let temp = Temp::new();
    let vault = temp.vault();
    vault.init(false).unwrap();
    seed_skill(&vault);
    for directory in ["skills", "meta"] {
        fs::write(
            vault.root.join(directory).join(".DS_Store"),
            "Finder metadata",
        )
        .unwrap();
    }
    let before = snapshot(&temp.0);
    assert_eq!(vault.records().unwrap().len(), 1);
    assert_eq!(vault.list(&ListFilter::default()).unwrap().len(), 1);
    assert_eq!(vault.show("api-review").unwrap().skill.id, "api-review");
    assert_eq!(snapshot(&temp.0), before);
}

// Invoked by the parent regression test with a child-only PATH. The ordinary
// parallel test run leaves this helper inactive and never changes process env.
#[cfg(unix)]
#[test]
fn initialization_failure_child() {
    let Some(root) = std::env::var_os("KINANA_INIT_CHILD_ROOT") else {
        return;
    };
    let index = std::env::var_os("KINANA_INIT_CHILD_INDEX").unwrap();
    let vault = Vault::new(root.into(), index.into());
    if std::env::var_os("KINANA_INIT_CHILD_SUCCEED").is_some() {
        assert!(vault.init(false).unwrap().created);
    } else {
        assert_eq!(vault.init(false).unwrap_err().code, "GIT_ERROR");
    }
}

#[cfg(unix)]
#[test]
fn initial_git_failures_roll_back_and_retry_without_global_path_changes() {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;
    let temp = Temp::new();
    let binary = std::env::current_exe().unwrap();
    let original_path = std::env::var_os("PATH").unwrap();
    let git = std::env::split_paths(&original_path)
        .map(|path| path.join("git"))
        .find(|path| path.is_file())
        .unwrap();
    let fake_bin = temp.0.join("bin");
    fs::create_dir(&fake_bin).unwrap();
    let fake_git = fake_bin.join("git");
    fs::write(
        &fake_git,
        r#"#!/bin/sh
previous=''
target=''
for argument in "$@"; do
    if [ "$previous" = '-C' ]; then target="$argument"; fi
    if [ "$argument" = 'init' ] && [ "$KINANA_GIT_FAIL" = 'init' ]; then
        printf '[core]\nrepositoryformatversion = 0\n' > "$target/.git/config"
        if [ "$KINANA_GIT_PRESERVE" = 'yes' ]; then
            printf 'concurrent user data' > "$target/keep.txt"
        fi
        exit 1
    fi
    if [ "$argument" = 'commit' ] && [ "$KINANA_GIT_FAIL" = 'commit' ]; then exit 1; fi
    previous="$argument"
done
exec "$KINANA_REAL_GIT" "$@"
"#,
    )
    .unwrap();
    fs::set_permissions(&fake_git, fs::Permissions::from_mode(0o755)).unwrap();

    // Since 2026-11-18 a trusted system Git from the fixed install locations
    // (e.g. /usr/bin via /etc/paths) is used when PATH has none, so "missing"
    // is only reachable on a machine without one. Rollback itself stays
    // covered by the init and commit failures.
    let known_only = kinana_core::executables::SearchPlan {
        path: vec![],
        ..kinana_core::executables::SearchPlan::current()
    };
    let system_git = kinana_core::executables::find_in(&known_only, "git", &[])
        .found
        .is_some();
    for failure in ["missing", "init", "commit"] {
        if failure == "missing" && system_git {
            continue;
        }
        let root = temp.0.join(failure);
        if failure != "missing" {
            fs::create_dir(&root).unwrap();
        }
        let index = temp.0.join(format!("{failure}-cache/index.db"));
        let before = snapshot(&temp.0);
        let child_path = if failure == "missing" {
            temp.0.join("no-git")
        } else {
            fake_bin.clone()
        };
        let status = Command::new(&binary)
            .args(["--exact", "initialization_failure_child", "--nocapture"])
            .env("PATH", child_path)
            .env("KINANA_REAL_GIT", &git)
            .env("KINANA_GIT_FAIL", failure)
            .env("KINANA_INIT_CHILD_ROOT", &root)
            .env("KINANA_INIT_CHILD_INDEX", &index)
            .status()
            .unwrap();
        assert!(status.success(), "failed rollback scenario: {failure}");
        assert_eq!(
            snapshot(&temp.0),
            before,
            "left outputs after {failure} failure"
        );
        let status = Command::new(&binary)
            .args(["--exact", "initialization_failure_child", "--nocapture"])
            .env("PATH", &original_path)
            .env("KINANA_INIT_CHILD_ROOT", &root)
            .env("KINANA_INIT_CHILD_INDEX", &index)
            .env("KINANA_INIT_CHILD_SUCCEED", "yes")
            .status()
            .unwrap();
        assert!(status.success(), "failed retry after {failure}");
    }

    let root = temp.0.join("preserve");
    fs::create_dir(&root).unwrap();
    let index = temp.0.join("preserve-cache/index.db");
    let status = Command::new(binary)
        .args(["--exact", "initialization_failure_child", "--nocapture"])
        .env("PATH", fake_bin)
        .env("KINANA_REAL_GIT", git)
        .env("KINANA_GIT_FAIL", "init")
        .env("KINANA_GIT_PRESERVE", "yes")
        .env("KINANA_INIT_CHILD_ROOT", &root)
        .env("KINANA_INIT_CHILD_INDEX", &index)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(
        snapshot(&root),
        BTreeMap::from([(PathBuf::from("keep.txt"), b"concurrent user data".to_vec())])
    );
    assert!(!index.exists());
}

#[test]
fn invalid_metadata_and_path_traversal_are_rejected() {
    let temp = Temp::new();
    let vault = temp.vault();
    vault.init(false).unwrap();
    seed_skill(&vault);
    for id in [
        "..",
        "../api-review",
        "/api-review",
        "api/review",
        "api\\review",
        "",
    ] {
        assert!(vault.show(id).is_err(), "ID should be refused: {id}");
    }
    let metadata_path = vault.root.join("meta/api-review.toml");
    let metadata = fs::read_to_string(&metadata_path)
        .unwrap()
        .replace("id = \"api-review\"", "id = \"different\"");
    fs::write(&metadata_path, metadata).unwrap();
    assert_eq!(vault.records().unwrap_err().code, "METADATA_ID_MISMATCH");
}

#[test]
fn improved_description_requires_explicit_approval_metadata() {
    let temp = Temp::new();
    let vault = temp.vault();
    vault.init(false).unwrap();
    seed_skill(&vault);
    let path = vault.root.join("meta/api-review.toml");
    let metadata = fs::read_to_string(&path).unwrap().replace(
        "[description]",
        "[description]\nactive = \"improved\"\nimproved = \"Proposed description\"",
    );
    fs::write(&path, &metadata).unwrap();
    assert_eq!(
        vault.show("api-review").unwrap_err().code,
        "INVALID_METADATA"
    );
    assert_eq!(vault.records().unwrap_err().code, "INVALID_METADATA");
    let approved = metadata.replace(
        "[description]",
        "[description]\napproved_at = \"2026-11-13T09:05:00Z\"",
    );
    fs::write(&path, approved).unwrap();
    assert_eq!(
        vault.show("api-review").unwrap().skill.description,
        "Proposed description"
    );
}

fn record(id: &str, description: &str) -> SkillRecord {
    SkillRecord {
        stack: vec![],
        stage: vec![],
        when_to_use: String::new(),
        pinned: false,
        id: id.into(),
        name: id.into(),
        description: description.into(),
        category: "other".into(),
        audit_level: "unscanned".into(),
        content_hash: "blake3:fixture".into(),
        source: SourceInfo::default(),
        added_at: "2026-11-13T00:00:00Z".into(),
        role: "Reviewing".into(),
        tags: vec!["review".into()],
    }
}

#[test]
fn index_fts_and_scoped_atomic_rebuild_preserve_other_vaults() {
    let temp = Temp::new();
    let path = temp.0.join("index.db");
    let first = temp.0.join("first");
    let second = temp.0.join("second");
    index::initialize(&path).unwrap();
    index::rebuild_for_vault(&path, &first, &[record("same-id", "SQLite transactions")]).unwrap();
    index::rebuild_for_vault(&path, &second, &[record("same-id", "API review")]).unwrap();
    index::check(&path).unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    let count: i64 = connection
        .query_row(
            "SELECT count(*) FROM skills_fts WHERE skills_fts MATCH 'transactions'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    let count: i64 = connection
        .query_row("SELECT count(*) FROM skills", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 2);
    drop(connection);

    // Duplicate IDs must roll back the entire scoped replacement.
    assert!(
        index::rebuild_for_vault(
            &path,
            &first,
            &[record("duplicate", "one"), record("duplicate", "two")]
        )
        .is_err()
    );
    index::check(&path).unwrap();
    index::rebuild_for_vault(&path, &first, &[]).unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    let remaining: String = connection
        .query_row("SELECT description FROM skills", [], |row| row.get(0))
        .unwrap();
    assert_eq!(remaining, "API review");
    drop(connection);
    let before = snapshot(&temp.0);
    index::check(&path).unwrap();
    index::initialize(&path).unwrap();
    assert_eq!(snapshot(&temp.0), before);
}

#[test]
fn index_check_detects_corruption_without_repairing() {
    let temp = Temp::new();
    let path = temp.0.join("index.db");
    assert_eq!(index::check(&path).unwrap_err().code, "INDEX_MISSING");
    assert!(!path.exists());
    index::rebuild_for_vault(
        &path,
        &temp.0.join("vault"),
        &[record("review", "contracts")],
    )
    .unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute("DELETE FROM skills_fts", []).unwrap();
    drop(connection);
    let before = snapshot(&temp.0);
    assert_eq!(index::check(&path).unwrap_err().code, "INDEX_CORRUPT");
    assert_eq!(snapshot(&temp.0), before);
    index::rebuild_for_vault(
        &path,
        &temp.0.join("vault"),
        &[record("review", "Restored from files")],
    )
    .unwrap();
    index::check(&path).unwrap();
}

#[test]
fn index_creation_preserves_foreign_sidecars() {
    let temp = Temp::new();
    let path = temp.0.join("index.db");
    fs::write(temp.0.join("index.db-journal"), "foreign data").unwrap();
    let before = snapshot(&temp.0);
    assert_eq!(index::initialize(&path).unwrap_err().code, "INDEX_BUSY");
    assert_eq!(snapshot(&temp.0), before);
}

#[cfg(unix)]
#[test]
fn symlink_components_prevent_vault_and_index_writes() {
    use std::os::unix::fs::symlink;
    let temp = Temp::new();
    let outside = temp.0.join("outside");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, temp.0.join("linked")).unwrap();
    let vault = Vault::new(temp.0.join("linked/vault"), temp.0.join("cache/index.db"));
    let before = snapshot(&temp.0);
    assert_eq!(vault.init(false).unwrap_err().code, "SYMLINK_PATH");
    assert_eq!(snapshot(&temp.0), before);

    let vault = Vault::new(temp.0.join("vault"), temp.0.join("linked/index.db"));
    assert_eq!(vault.init(false).unwrap_err().code, "SYMLINK_PATH");
    assert!(!vault.root.exists());
    assert_eq!(snapshot(&temp.0), before);

    let path = temp.0.join("actual.db");
    index::initialize(&path).unwrap();
    symlink(&path, temp.0.join("linked.db")).unwrap();
    let before = snapshot(&temp.0);
    assert!(index::initialize(&temp.0.join("linked.db")).is_err());
    assert!(
        index::rebuild_for_vault(&temp.0.join("linked.db"), &temp.0.join("vault"), &[]).is_err()
    );
    assert_eq!(snapshot(&temp.0), before);
}

#[cfg(unix)]
#[test]
fn symlink_source_and_metadata_are_never_followed() {
    use std::os::unix::fs::symlink;
    let temp = Temp::new();
    let vault = temp.vault();
    vault.init(false).unwrap();
    seed_skill(&vault);
    let metadata = vault.root.join("meta/api-review.toml");
    let outside = temp.0.join("outside.toml");
    fs::rename(&metadata, &outside).unwrap();
    symlink(&outside, &metadata).unwrap();
    let before = snapshot(&temp.0);
    assert_eq!(vault.show("api-review").unwrap_err().code, "SYMLINK_PATH");
    assert_eq!(snapshot(&temp.0), before);
    fs::remove_file(&metadata).unwrap();
    fs::rename(&outside, &metadata).unwrap();
    let source = vault.root.join("skills/api-review/SKILL.md");
    let outside = temp.0.join("outside.md");
    fs::rename(&source, &outside).unwrap();
    symlink(&outside, &source).unwrap();
    let before = snapshot(&temp.0);
    assert_eq!(vault.records().unwrap_err().code, "SYMLINK_PATH");
    assert_eq!(snapshot(&temp.0), before);
}
