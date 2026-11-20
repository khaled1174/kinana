#[allow(dead_code)]
mod support;
use serde_json::Value;
use std::fs;
use support::{Sandbox, snapshot};

fn success(s: &Sandbox, args: &[&str]) -> Value {
    let out = s.run(args);
    assert!(
        out.status.success(),
        "{args:?}: {} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

/// `seed` writes the skill without a commit, as no command would leave it;
/// `doctor` reports such a vault as an uncommitted change, so commit it.
fn seed_committed(s: &Sandbox, id: &str) {
    s.seed(id, "Review", "web-frontend");
    for args in [
        vec!["add", "--all"],
        vec![
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@localhost",
            "commit",
            "-q",
            "-m",
            "Fixture: seeded skill",
        ],
    ] {
        let status = std::process::Command::new("/usr/bin/git")
            .arg("-C")
            .arg(&s.vault)
            .args(&args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }
}

#[test]
fn doctor_rebuild_previews_without_writes_and_restores_registered_projects_after_index_loss() {
    let s = Sandbox::new();
    s.initialize();
    seed_committed(&s, "review");
    let p = s.root.join("project");
    fs::create_dir(&p).unwrap();
    success(
        &s,
        &[
            "use",
            "review",
            "--project",
            p.to_str().unwrap(),
            "--agent",
            "claude",
            "--json",
        ],
    );
    let database = s.home.join(".kinana/index-v2.db");
    fs::remove_file(&database).unwrap();
    let before = snapshot(&s.root);
    let preview = success(&s, &["doctor", "--rebuild-index", "--dry-run", "--json"]);
    assert_eq!(preview["rebuild"]["dry_run"], true);
    assert_eq!(preview["rebuild"]["projects"], 1);
    assert_eq!(preview["rebuild"]["activations"], 1);
    assert_eq!(snapshot(&s.root), before);
    let source_before = snapshot(&s.vault);
    let project_before = snapshot(&p);
    let result = success(&s, &["doctor", "--rebuild-index", "--json"]);
    assert_eq!(result["rebuild"]["rebuilt"], true);
    assert_eq!(result["healthy"], true);
    let c = rusqlite::Connection::open(database).unwrap();
    for table in ["skills", "skills_fts", "projects", "activations"] {
        assert_eq!(
            c.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            1,
            "{table}"
        );
    }
    assert_eq!(snapshot(&s.vault), source_before);
    assert_eq!(snapshot(&p), project_before);
    let before = snapshot(&s.root);
    success(&s, &["doctor", "--json"]);
    assert_eq!(snapshot(&s.root), before);
}

#[test]
fn doctor_reports_a_broken_link_with_exit_one_without_repairing_or_deleting_it() {
    let s = Sandbox::new();
    s.initialize();
    s.seed("review", "Review", "web-frontend");
    let p = s.root.join("project");
    fs::create_dir(&p).unwrap();
    success(
        &s,
        &[
            "use",
            "review",
            "--project",
            p.to_str().unwrap(),
            "--agent",
            "claude",
            "--json",
        ],
    );
    fs::remove_file(s.vault.join("dist/review/SKILL.md")).unwrap();
    let before = snapshot(&s.root);
    let result = s.run(&["doctor", "--json"]);
    assert_eq!(result.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["healthy"], false);
    assert_eq!(report["projects"][0]["items"][0]["state"], "broken");
    assert_eq!(snapshot(&s.root), before);
}

#[test]
fn doctor_rebuild_refuses_a_foreign_database_with_exit_two_and_no_writes() {
    let s = Sandbox::new();
    s.initialize();
    let path = s.home.join(".kinana/index-v2.db");
    let c = rusqlite::Connection::open(path).unwrap();
    c.pragma_update(None, "application_id", 42).unwrap();
    drop(c);
    let before = snapshot(&s.root);
    for dry in [false, true] {
        let mut args = vec!["doctor", "--rebuild-index", "--json"];
        if dry {
            args.push("--dry-run");
        }
        let result = s.run(&args);
        assert_eq!(result.status.code(), Some(2));
        let report: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(report["error"]["code"], "INDEX_CONFLICT");
        assert_eq!(snapshot(&s.root), before);
    }
}

#[test]
fn missing_state_directory_dry_run_writes_nothing_and_real_rebuild_prints_scope_warning() {
    let s = Sandbox::new();
    s.initialize();
    seed_committed(&s, "review");
    let parent = s.home.join(".kinana");
    fs::remove_file(parent.join("index-v2.db")).unwrap();
    fs::remove_dir(&parent).unwrap();
    let before = snapshot(&s.root);
    let preview = success(&s, &["doctor", "--rebuild-index", "--dry-run", "--json"]);
    assert_eq!(preview["rebuild"]["mode"], "missing");
    assert_eq!(snapshot(&s.root), before);
    assert!(!parent.exists());
    let result = s.run(&["doctor", "--rebuild-index"]);
    assert!(result.status.success());
    let text = String::from_utf8(result.stdout).unwrap();
    assert!(text.contains("Rebuilt index (missing)"));
    assert!(text.contains("Warning: Only the selected vault"));
    assert!(text.contains("other vault scopes"));
}
