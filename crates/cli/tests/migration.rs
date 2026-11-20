#[allow(dead_code)]
mod support;
use serde_json::Value;
use std::{fs, path::PathBuf};
use support::{Sandbox, snapshot};

fn run(s: &Sandbox, args: &[&str], exit: i32) -> Value {
    let out = s.run(args);
    assert_eq!(
        out.status.code(),
        Some(exit),
        "{} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn legacy(s: &Sandbox) -> PathBuf {
    s.initialize();
    s.seed("probe", "probe", "other");
    let project = s.root.join("مشروع مرتبط");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("README.md"), "preserved\n").unwrap();
    run(
        s,
        &[
            "use",
            "probe",
            "--project",
            project.to_str().unwrap(),
            "--agent",
            "codex",
            "--json",
        ],
        0,
    );
    fs::rename(s.home.join(".kinana"), s.home.join(".skillsbox")).unwrap();
    // Legacy state only ever held the earlier version's schema 1 index.
    support::as_left_by_the_beta(&s.home.join(".skillsbox"));
    fs::rename(
        project.join(".kinana.lock"),
        project.join(".skillsbox.lock"),
    )
    .unwrap();
    fs::write(
        s.home.join(".skillsbox/future-data.txt"),
        b"unknown state bytes\0\xff",
    )
    .unwrap();
    let path = project.join(".skillsbox.lock");
    let mut doc: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    doc.as_table_mut()
        .unwrap()
        .insert("future".into(), toml::Value::String("preserve me".into()));
    doc["active"][0].as_table_mut().unwrap().insert(
        "future_member".into(),
        toml::Value::String("preserve member too".into()),
    );
    fs::write(path, toml::to_string_pretty(&doc).unwrap()).unwrap();
    project
}
#[test]
fn legacy_preview_confirm_rollback_preserves_bytes_and_target_identity() {
    let s = Sandbox::new();
    let project = legacy(&s);
    let p = project.to_str().unwrap();
    let before = snapshot(&s.root);
    let before_project = snapshot(&project);
    let target = fs::read_link(project.join(".agents/skills/probe")).unwrap();
    let plan = run(&s, &["migrate", "--project", p, "--dry-run", "--json"], 0);
    assert_eq!(plan["changed"], true);
    assert_eq!(snapshot(&s.root), before);
    run(&s, &["status", "--project", p, "--json"], 0);
    assert_eq!(snapshot(&s.root), before);
    assert_eq!(
        run(&s, &["pin", "probe", "--json"], 2)["error"]["code"],
        "MIGRATION_REQUIRED"
    );
    run(
        &s,
        &["migrate", "--project", p, "--confirm", "obsolete", "--json"],
        2,
    );
    assert_eq!(snapshot(&s.root), before);
    let lock = fs::read(project.join(".skillsbox.lock")).unwrap();
    run(
        &s,
        &[
            "migrate",
            "--project",
            p,
            "--confirm",
            plan["preview_token"].as_str().unwrap(),
            "--json",
        ],
        0,
    );
    assert_eq!(fs::read(project.join(".kinana.lock")).unwrap(), lock);
    assert!(!project.join(".skillsbox.lock").exists());
    assert_eq!(
        fs::read(s.home.join(".kinana/future-data.txt")).unwrap(),
        b"unknown state bytes\0\xff"
    );
    assert_eq!(
        run(&s, &["status", "--project", p, "--json"], 0)["projects"][0]["items"][0]["state"],
        "current"
    );
    assert_eq!(
        run(&s, &["migrate", "--dry-run", "--json"], 0)["changed"],
        false
    );
    let rollback = run(&s, &["migrate", "--rollback", "--dry-run", "--json"], 0);
    run(
        &s,
        &[
            "migrate",
            "--rollback",
            "--confirm",
            rollback["preview_token"].as_str().unwrap(),
            "--json",
        ],
        0,
    );
    assert_eq!(fs::read(project.join(".skillsbox.lock")).unwrap(), lock);
    assert!(!s.home.join(".kinana").exists());
    assert_eq!(snapshot(&project), before_project);
    assert_eq!(
        fs::read_link(project.join(".agents/skills/probe")).unwrap(),
        target
    );
}
#[test]
fn conflicting_lock_names_and_foreign_new_state_are_never_overwritten() {
    let s = Sandbox::new();
    let project = legacy(&s);
    fs::write(project.join(".kinana.lock"), "foreign").unwrap();
    let before = snapshot(&s.root);
    run(
        &s,
        &[
            "migrate",
            "--project",
            project.to_str().unwrap(),
            "--dry-run",
            "--json",
        ],
        2,
    );
    assert_eq!(snapshot(&s.root), before);
    fs::remove_file(project.join(".kinana.lock")).unwrap();
    fs::create_dir(s.home.join(".kinana")).unwrap();
    fs::write(s.home.join(".kinana/owner-file"), "foreign").unwrap();
    let before = snapshot(&s.root);
    run(&s, &["migrate", "--dry-run", "--json"], 2);
    assert_eq!(snapshot(&s.root), before);
}
#[test]
fn sqlite_wal_commits_survive_the_state_backup() {
    let s = Sandbox::new();
    let project = legacy(&s);
    let db = rusqlite::Connection::open(s.home.join(".skillsbox/index.db")).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE migration_probe(value TEXT); INSERT INTO migration_probe VALUES('WAL committed data');").unwrap();
    let plan = run(
        &s,
        &[
            "migrate",
            "--project",
            project.to_str().unwrap(),
            "--dry-run",
            "--json",
        ],
        0,
    );
    run(
        &s,
        &[
            "migrate",
            "--project",
            project.to_str().unwrap(),
            "--confirm",
            plan["preview_token"].as_str().unwrap(),
            "--json",
        ],
        0,
    );
    let new = rusqlite::Connection::open(s.home.join(".kinana/index.db")).unwrap();
    assert_eq!(
        new.query_row("SELECT value FROM migration_probe", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "WAL committed data"
    );
    assert_eq!(
        new.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
}

#[test]
fn compatibility_binary_and_new_binary_share_the_os_writer_lease() {
    let s = Sandbox::new();
    s.initialize();
    s.seed("probe", "probe", "other");
    kinana_core::gitlog::commit(&s.vault, "Fixture: seed skill for concurrent writers").unwrap();
    let _guard =
        kinana_core::state::write_guard(&s.home.join(".kinana/index-v2.db"), false).unwrap();
    let before = snapshot(&s.root);
    for executable in [
        env!("CARGO_BIN_EXE_kinana"),
        env!("CARGO_BIN_EXE_skillsbox"),
    ] {
        let output = std::process::Command::new(executable)
            .env("HOME", &s.home)
            .env("USERPROFILE", &s.home)
            .arg("--vault")
            .arg(&s.vault)
            .args(["pin", "probe", "--json"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        let json: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["error"]["code"], "STATE_BUSY");
    }
    assert_eq!(snapshot(&s.root), before);
}

#[test]
fn migration_preserves_copy_and_manual_targets_and_unuse_restores_project() {
    for (agent, mode, manual) in [("codex", "copy", false), ("claude", "link", true)] {
        let s = Sandbox::new();
        s.initialize();
        s.seed("probe", "probe", "other");
        if manual {
            let path = s.vault.join("meta/probe.toml");
            let original = fs::read_to_string(&path).unwrap();
            fs::write(path, format!("{original}\n[invocation]\nmode='manual'\n")).unwrap();
        }
        let project = s.root.join("project");
        fs::create_dir(&project).unwrap();
        fs::write(project.join("user.txt"), "untouched").unwrap();
        let before = snapshot(&project);
        let p = project.to_str().unwrap();
        run(
            &s,
            &[
                "use",
                "probe",
                "--project",
                p,
                "--agent",
                agent,
                "--mode",
                mode,
                "--json",
            ],
            0,
        );
        let active = snapshot(&project);
        fs::rename(s.home.join(".kinana"), s.home.join(".skillsbox")).unwrap();
        // Legacy state only ever held the earlier version's schema 1 index.
        support::as_left_by_the_beta(&s.home.join(".skillsbox"));
        fs::rename(
            project.join(".kinana.lock"),
            project.join(".skillsbox.lock"),
        )
        .unwrap();
        let plan = run(&s, &["migrate", "--project", p, "--dry-run", "--json"], 0);
        run(
            &s,
            &[
                "migrate",
                "--project",
                p,
                "--confirm",
                plan["preview_token"].as_str().unwrap(),
                "--json",
            ],
            0,
        );
        assert_eq!(snapshot(&project), active);
        run(
            &s,
            &["unuse", "probe", "--project", p, "--agent", agent, "--json"],
            0,
        );
        assert_eq!(snapshot(&project), before);
    }
}
