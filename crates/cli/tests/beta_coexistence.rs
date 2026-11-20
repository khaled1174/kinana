//! The 0.1.0 beta and this version on one vault and one state directory,
//! driven through the real beta binary. Ignored by default because that
//! binary is not part of this workspace: build `kinana` from the 0.1.0
//! commit, point `KINANA_BETA_BIN` at it and run
//! `cargo test -p kinana --test beta_coexistence -- --ignored`.
#[allow(dead_code)]
mod support;

use kinana_core::{ClassificationProposal, edit, vault::Vault};
use rusqlite::Connection;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use support::Sandbox;

fn run(binary: &Path, s: &Sandbox, args: &[&str], exit: i32) -> Value {
    let output = Command::new(binary)
        .env("HOME", &s.home)
        .env("USERPROFILE", &s.home)
        .arg("--vault")
        .arg(&s.vault)
        .args(args)
        .arg("--json")
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(exit),
        "{} {args:?}: {} {}",
        binary.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn source(s: &Sandbox, id: &str, description: &str) -> String {
    let source = s.root.join(format!("source-{id}"));
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        format!("---\nname: {id}\ndescription: {description}\n---\n# Body\nText.\n"),
    )
    .unwrap();
    source.to_str().unwrap().to_owned()
}

fn version(index: &Path) -> i64 {
    Connection::open(index)
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap()
}

#[test]
#[ignore = "needs KINANA_BETA_BIN, a kinana binary built from the 0.1.0 commit"]
fn the_beta_and_this_version_both_work_on_one_vault() {
    let beta = PathBuf::from(
        std::env::var_os("KINANA_BETA_BIN").expect("set KINANA_BETA_BIN to the 0.1.0 binary"),
    );
    let new = Path::new(env!("CARGO_BIN_EXE_kinana"));
    let s = Sandbox::new();
    let project = s.root.join("project");
    fs::create_dir(&project).unwrap();
    let project = project.to_str().unwrap();
    let beta_index = s.home.join(".kinana/index.db");
    let new_index = s.home.join(".kinana/index-v2.db");

    // The beta creates the vault and uses it.
    run(&beta, &s, &["init"], 0);
    run(
        &beta,
        &s,
        &["add", &source(&s, "api", "Review REST API contracts")],
        0,
    );
    run(
        &beta,
        &s,
        &["use", "api", "--project", project, "--agent", "claude"],
        0,
    );
    assert_eq!(
        run(&beta, &s, &["search", "contracts"], 0)["items"][0]["id"],
        "api"
    );
    let beta_bytes = fs::read(&beta_index).unwrap();
    assert_eq!(version(&beta_index), 1);

    // This version arrives: a preview builds nothing, the next command builds
    // its own index from the vault, and the beta's file is not touched.
    assert_eq!(
        run(new, &s, &["search", "contracts", "--dry-run"], 1)["error"]["code"],
        "INDEX_MISSING"
    );
    assert!(!new_index.exists());
    assert_eq!(
        run(new, &s, &["search", "contracts"], 0)["items"][0]["id"],
        "api"
    );
    assert_eq!(version(&new_index), 2);
    assert!(run(new, &s, &["doctor"], 0)["healthy"].as_bool().unwrap());

    // This version changes the vault, using what the beta does not know:
    // dimensions, and a waiting proposal that carries them.
    run(
        new,
        &s,
        &["add", &source(&s, "forms", "Build accessible React forms")],
        0,
    );
    run(
        new,
        &s,
        &[
            "dimensions",
            "forms",
            "--stack",
            "react",
            "--stage",
            "build",
        ],
        0,
    );
    let vault = Vault::new(s.vault.clone(), new_index.clone());
    let mut update = edit::read_metadata(&vault, "forms").unwrap();
    update.proposal = Some(ClassificationProposal {
        category: "web-frontend".into(),
        stack: vec!["react".into()],
        stage: vec!["build".into()],
        tags: vec!["forms".into()],
        role: "Builds accessible forms".into(),
        when_to_use: "When a screen collects input".into(),
        when_not: "For read-only pages".into(),
        improved_description: "Build accessible React forms with validation.".into(),
        overlaps_with: vec![],
        confidence: 0.9,
    });
    edit::apply(
        &vault,
        &BTreeMap::from([("forms".into(), update)]),
        "Fixture: a waiting proposal with dimensions",
        false,
    )
    .unwrap();
    run(
        new,
        &s,
        &["use", "forms", "--project", project, "--agent", "claude"],
        0,
    );
    assert_eq!(
        run(new, &s, &["search", "forms"], 0)["items"][0]["id"],
        "forms"
    );
    assert_eq!(
        fs::read(&beta_index).unwrap(),
        beta_bytes,
        "this version never writes the beta's index"
    );

    // The beta still reads everything, including that proposal, and refreshes
    // its own index the way it always did after an outside change.
    assert_eq!(run(&beta, &s, &["list"], 0).as_array().unwrap().len(), 2);
    assert_eq!(
        run(&beta, &s, &["show", "forms"], 0)["skill"]["id"],
        "forms"
    );
    assert_eq!(
        run(&beta, &s, &["review", "forms"], 0)["decision"],
        "preview"
    );
    assert_eq!(
        run(&beta, &s, &["status", "--project", project], 0)["projects"][0]["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        run(&beta, &s, &["search", "forms"], 1)["error"]["code"],
        "INDEX_STALE"
    );
    run(&beta, &s, &["doctor", "--rebuild-index"], 0);
    assert_eq!(
        run(&beta, &s, &["search", "forms"], 0)["items"][0]["id"],
        "forms"
    );
    assert!(run(&beta, &s, &["doctor"], 0)["healthy"].as_bool().unwrap());

    // Beta rewrites of that skill's metadata, of its classification table
    // too, keep what the beta does not know.
    run(&beta, &s, &["pin", "forms"], 0);
    run(&beta, &s, &["category", "forms", "design-ui"], 0);
    let listed = run(
        new,
        &s,
        &["list", "--stack", "react", "--stage", "build"],
        0,
    );
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["id"], "forms");
    assert_eq!(listed[0]["pinned"], true);
    assert_eq!(listed[0]["category"], "design-ui");
    let pending = edit::read_metadata(&vault, "forms")
        .unwrap()
        .proposal
        .unwrap();
    assert_eq!(
        (pending.stack, pending.stage),
        (vec!["react".to_owned()], vec!["build".to_owned()])
    );

    // The beta changes the vault; this version refreshes its own index.
    let new_bytes = fs::read(&new_index).unwrap();
    run(
        &beta,
        &s,
        &["add", &source(&s, "extra", "Plan database migrations")],
        0,
    );
    run(&beta, &s, &["unuse", "api", "--project", project], 0);
    assert_eq!(
        fs::read(&new_index).unwrap(),
        new_bytes,
        "the beta never writes this version's index"
    );
    assert_eq!(
        run(new, &s, &["search", "migrations"], 1)["error"]["code"],
        "INDEX_STALE"
    );
    run(new, &s, &["doctor", "--rebuild-index"], 0);
    assert_eq!(
        run(new, &s, &["search", "migrations"], 0)["items"][0]["id"],
        "extra"
    );
    assert!(run(new, &s, &["doctor"], 0)["healthy"].as_bool().unwrap());
    assert!(run(&beta, &s, &["doctor"], 0)["healthy"].as_bool().unwrap());
    assert_eq!((version(&beta_index), version(&new_index)), (1, 2));
}

/// Finder files: this version's `build` and `undo` remove an untracked
/// `.DS_Store` that is a skill's only difference, and the hash both versions
/// compute stays the same, so the beta keeps working on the result.
#[test]
#[ignore = "needs KINANA_BETA_BIN, a kinana binary built from the 0.1.0 commit"]
fn finder_files_removed_by_this_version_leave_hashes_the_beta_agrees_with() {
    let beta = PathBuf::from(
        std::env::var_os("KINANA_BETA_BIN").expect("set KINANA_BETA_BIN to the 0.1.0 binary"),
    );
    let new = Path::new(env!("CARGO_BIN_EXE_kinana"));
    let s = Sandbox::new();
    run(&beta, &s, &["init"], 0);
    // A skill imported with a `.DS_Store` of its own: tracked payload.
    let imported = source(&s, "imported", "Review REST API contracts");
    fs::write(
        Path::new(&imported).join(".DS_Store"),
        "imported finder bytes",
    )
    .unwrap();
    run(&beta, &s, &["add", &imported], 0);
    run(
        &beta,
        &s,
        &["add", &source(&s, "plain", "Plan database migrations")],
        0,
    );
    let hash = |binary: &Path, id: &str| {
        run(binary, &s, &["show", id], 0)["skill"]["content_hash"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let recorded = (hash(&beta, "imported"), hash(&beta, "plain"));
    assert_eq!((hash(new, "imported"), hash(new, "plain")), recorded);

    // Finder writes into a skill folder: the beta refuses, as it always did.
    let leftover = s.vault.join("skills/plain/.DS_Store");
    fs::write(&leftover, "\0\0\0\x01Bud1").unwrap();
    assert_eq!(
        run(&beta, &s, &["build"], 1)["error"]["code"],
        "SOURCE_HASH_MISMATCH"
    );
    let report = run(new, &s, &["build"], 0);
    assert_eq!(
        report["finder_files_removed"],
        serde_json::json!(["skills/plain/.DS_Store"])
    );
    assert!(!leftover.exists());
    assert_eq!(
        fs::read(s.vault.join("skills/imported/.DS_Store")).unwrap(),
        b"imported finder bytes"
    );
    // Both versions still compute the recorded hashes and accept the vault.
    assert_eq!((hash(&beta, "imported"), hash(&beta, "plain")), recorded);
    assert_eq!((hash(new, "imported"), hash(new, "plain")), recorded);
    run(&beta, &s, &["build"], 0);
    assert!(run(&beta, &s, &["doctor"], 0)["healthy"].as_bool().unwrap());

    // The same through undo of a change the beta made.
    run(&beta, &s, &["pin", "plain"], 0);
    fs::write(&leftover, "\0\0\0\x01Bud1").unwrap();
    let undone = run(new, &s, &["undo"], 0);
    assert_eq!(
        undone["finder_files_removed"],
        serde_json::json!(["skills/plain/.DS_Store"])
    );
    assert!(!leftover.exists());
    assert_eq!((hash(&beta, "imported"), hash(&beta, "plain")), recorded);
    run(&beta, &s, &["doctor", "--rebuild-index"], 0);
    assert!(run(&beta, &s, &["doctor"], 0)["healthy"].as_bool().unwrap());
    run(&beta, &s, &["build"], 0);
}

/// The router with the CLI path (ownership marker version 2) is not one the
/// beta knows: it refuses to replace it and writes nothing, and this version
/// still owns it afterwards.
#[test]
#[ignore = "needs KINANA_BETA_BIN, a kinana binary built from the 0.1.0 commit"]
fn the_beta_leaves_a_router_with_the_cli_path_alone() {
    let beta = PathBuf::from(std::env::var("KINANA_BETA_BIN").expect("KINANA_BETA_BIN"));
    let current = Path::new(env!("CARGO_BIN_EXE_kinana"));
    let s = Sandbox::new();
    let installed = run(current, &s, &["init", "--install-router"], 0);
    assert_eq!(installed["router"]["changed"], true);
    let before = support::snapshot(&s.home);
    let output = Command::new(&beta)
        .env("HOME", &s.home)
        .env("USERPROFILE", &s.home)
        .arg("--vault")
        .arg(&s.vault)
        .args(["init", "--install-router", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["code"], "ROUTER_CONFLICT", "{value}");
    assert_eq!(support::snapshot(&s.home), before, "the beta wrote");
    let again = run(current, &s, &["init", "--install-router"], 0);
    assert_eq!(again["router"]["changed"], false, "{again}");
}
