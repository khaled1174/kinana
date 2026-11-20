//! After an upgrade from the 0.1.0 beta: this version builds its own index
//! file from the vault and never touches the beta's `index.db`.
#[allow(dead_code)]
mod support;

use rusqlite::Connection;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use support::{Sandbox, as_left_by_the_beta, snapshot};

fn version(index: &Path) -> i64 {
    Connection::open(index)
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap()
}

fn head(vault: &Path) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(vault)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap()
}

fn json(s: &Sandbox, args: &[&str], exit: i32) -> Value {
    let output = s.run(&[args, &["--json"]].concat());
    assert_eq!(
        output.status.code(),
        Some(exit),
        "{args:?}: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn source(s: &Sandbox, id: &str, description: &str) -> PathBuf {
    let source = s.root.join(format!("source-{id}"));
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        format!("---\nname: {id}\ndescription: {description}\n---\n# Body\nText.\n"),
    )
    .unwrap();
    source
}

/// A vault with two skills whose state directory is as the beta left it.
/// Returns the beta's index and this version's index path.
fn upgraded_from_the_beta(s: &Sandbox) -> (PathBuf, PathBuf) {
    s.initialize();
    for (id, description) in [
        ("api", "Review REST API contracts"),
        ("forms", "Build accessible forms"),
    ] {
        json(s, &["add", source(s, id, description).to_str().unwrap()], 0);
    }
    let state = s.home.join(".kinana");
    as_left_by_the_beta(&state);
    assert_eq!(version(&state.join("index.db")), 1);
    (state.join("index.db"), state.join("index-v2.db"))
}

#[test]
fn the_first_command_after_the_upgrade_builds_this_versions_index_from_the_vault() {
    let s = Sandbox::new();
    let (beta, index) = upgraded_from_the_beta(&s);
    let beta_bytes = fs::read(&beta).unwrap();
    let vault = snapshot(&s.vault);
    let commit = head(&s.vault);

    let found = json(&s, &["search", "contracts"], 0);
    assert_eq!(found["items"].as_array().unwrap().len(), 1);
    assert_eq!(found["items"][0]["id"], "api");
    assert_eq!(version(&index), 2);
    assert_eq!(fs::read(&beta).unwrap(), beta_bytes);

    // Only this version's cache appeared: vault files and Git are as they were.
    assert_eq!(snapshot(&s.vault), vault);
    assert_eq!(head(&s.vault), commit);
    let status = Command::new("git")
        .arg("-C")
        .arg(&s.vault)
        .args(["status", "--porcelain"])
        .output()
        .unwrap();
    assert!(status.stdout.is_empty(), "{:?}", status.stdout);
    let mut names = fs::read_dir(s.home.join(".kinana"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(names, ["index-v2.db", "index.db"]);
    assert!(json(&s, &["doctor"], 0)["healthy"].as_bool().unwrap());
}

#[test]
fn a_preview_builds_nothing_and_the_next_command_does() {
    let s = Sandbox::new();
    upgraded_from_the_beta(&s);
    let before = snapshot(&s.root);
    for arguments in [
        vec!["search", "contracts", "--dry-run"],
        vec!["remove", "api", "--dry-run"],
    ] {
        let report = json(&s, &arguments, 1);
        assert_eq!(report["error"]["code"], "INDEX_MISSING", "{arguments:?}");
        assert_eq!(snapshot(&s.root), before, "{arguments:?}");
    }
    // Reading the catalog needs no index; not being a preview, it builds one.
    assert_eq!(json(&s, &["list"], 0).as_array().unwrap().len(), 2);
    assert_eq!(
        json(&s, &["search", "contracts", "--dry-run"], 0)["items"][0]["id"],
        "api"
    );
    assert_eq!(
        json(&s, &["remove", "api", "--dry-run"], 0)["changed"],
        true
    );
}

#[test]
fn no_command_of_this_version_changes_the_beta_index() {
    let s = Sandbox::new();
    let (beta, index) = upgraded_from_the_beta(&s);
    let beta_bytes = fs::read(&beta).unwrap();
    let project = s.root.join("project");
    fs::create_dir(&project).unwrap();
    let project = project.to_str().unwrap();
    let extra = source(&s, "extra", "Plan database migrations");
    let commands: &[&[&str]] = &[
        &["add", extra.to_str().unwrap()],
        &["dimensions", "api", "--stack", "node", "--stage", "review"],
        &["pin", "forms"],
        &["use", "api", "--project", project, "--agent", "claude"],
        &["status", "--project", project],
        &["unuse", "api", "--project", project],
        &["remove", "forms"],
        &["undo"],
        &["doctor", "--rebuild-index"],
        &["search", "migrations"],
        &["doctor"],
    ];
    for arguments in commands {
        json(&s, arguments, 0);
        assert_eq!(fs::read(&beta).unwrap(), beta_bytes, "{arguments:?}");
    }
    assert_eq!((version(&beta), version(&index)), (1, 2));
}

#[test]
fn a_foreign_file_at_this_versions_index_path_is_refused_and_never_modified() {
    let s = Sandbox::new();
    let (_, index) = upgraded_from_the_beta(&s);
    // Another application's SQLite database, and bytes that are no database at all.
    for foreign in [None, Some(&b"not a database"[..])] {
        let _ = fs::remove_file(&index);
        match foreign {
            Some(bytes) => fs::write(&index, bytes).unwrap(),
            None => {
                let connection = Connection::open(&index).unwrap();
                connection
                    .execute_batch("PRAGMA application_id = 42; CREATE TABLE theirs (value TEXT);")
                    .unwrap();
            }
        }
        let before = snapshot(&s.root);
        assert_eq!(
            json(&s, &["search", "contracts"], 2)["error"]["code"],
            "INDEX_CONFLICT"
        );
        assert_eq!(snapshot(&s.root), before);
    }
}

fn spawn(s: &Sandbox, args: &[&str]) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_kinana"))
        .env("HOME", &s.home)
        .env("USERPROFILE", &s.home)
        .arg("--vault")
        .arg(&s.vault)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap()
}

#[test]
fn commands_that_start_together_after_the_upgrade_all_get_an_index() {
    let s = Sandbox::new();
    let (beta, index) = upgraded_from_the_beta(&s);
    let beta_bytes = fs::read(&beta).unwrap();
    let children = (0..6)
        .map(|_| spawn(&s, &["search", "contracts", "--json"]))
        .collect::<Vec<_>>();
    for child in children {
        let output = child.wait_with_output().unwrap();
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(output.status.success(), "{text}");
        let found: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(found["items"][0]["id"], "api");
    }
    assert_eq!(version(&index), 2);
    assert_eq!(fs::read(&beta).unwrap(), beta_bytes);
}

#[test]
fn a_first_build_that_cannot_run_is_reported_and_not_hidden() {
    let s = Sandbox::new();
    let (_, index) = upgraded_from_the_beta(&s);
    // Another writer holds the lease for as long as the command waits.
    let _guard = kinana_core::state::write_guard(&index, false).unwrap();
    let before = snapshot(&s.root);
    let output = spawn(&s, &["search", "contracts", "--json"])
        .wait_with_output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["error"]["code"], "INDEX_MISSING");
    let warning = String::from_utf8_lossy(&output.stderr);
    assert!(warning.starts_with("Warning: "), "{warning}");
    assert!(warning.contains("STATE_BUSY"), "{warning}");
    assert!(!index.exists());
    assert_eq!(snapshot(&s.root), before);
}
