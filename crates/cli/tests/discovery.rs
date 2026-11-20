#[allow(dead_code)]
mod support;

use kinana_core::{index, vault::Vault};
use serde_json::{Value, json};
use std::fs;
use support::{Sandbox, snapshot};

fn fixture() -> Sandbox {
    let s = Sandbox::new();
    s.initialize();
    s.seed("review", "Contract Review", "web-frontend");
    let v = Vault::new(s.vault.clone(), s.home.join(".kinana/index-v2.db"));
    index::rebuild_for_vault(&v.index_path, &v.root, &v.records().unwrap()).unwrap();
    s
}

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

#[test]
fn search_finds_source_metadata_without_writes() {
    let s = fixture();
    let before = snapshot(&s.root);
    let result = success(&s, &["search", "contracts", "--json"]);
    assert_eq!(result["items"][0]["id"], "review");
    assert_eq!(snapshot(&s.root), before);
}

#[test]
fn suggest_reads_project_signals_and_matching_skills_without_writes() {
    let s = fixture();
    let project = s.root.join("project");
    fs::create_dir(&project).unwrap();
    fs::write(
        project.join("package.json"),
        r#"{"dependencies":{"react":"19"}}"#,
    )
    .unwrap();
    let before = snapshot(&s.root);
    let result = success(
        &s,
        &["suggest", "--project", project.to_str().unwrap(), "--json"],
    );
    assert_eq!(result["categories"], json!(["web-frontend"]));
    assert_eq!(result["skills"][0]["id"], "review");
    assert_eq!(snapshot(&s.root), before);
}

#[test]
fn discovery_errors_are_json_and_do_not_create_or_repair_anything() {
    let s = fixture();
    for args in [
        &["search", "", "--json"][..],
        &[
            "suggest",
            "--project",
            s.root.join("missing").to_str().unwrap(),
            "--json",
        ],
    ] {
        let before = snapshot(&s.root);
        let out = s.run(args);
        assert_eq!(out.status.code(), Some(1));
        let error: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert!(error["error"]["code"].is_string());
        assert_eq!(snapshot(&s.root), before);
    }
    fs::remove_file(s.home.join(".kinana/index-v2.db")).unwrap();
    let before = snapshot(&s.root);
    let out = s.run(&["search", "contracts", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["error"]["code"], "INDEX_MISSING");
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("doctor --rebuild-index")
    );
    assert_eq!(snapshot(&s.root), before);
}

#[test]
fn discovery_text_and_dry_run_use_the_same_readonly_behavior() {
    let s = fixture();
    let p = s.root.join("project");
    fs::create_dir(&p).unwrap();
    fs::write(p.join("package.json"), r#"{"dependencies":{"react":"19"}}"#).unwrap();
    let before = snapshot(&s.root);
    for args in [
        &["search", "contracts", "--dry-run"][..],
        &["suggest", "--project", p.to_str().unwrap(), "--dry-run"],
    ] {
        let out = s.run(args);
        assert!(out.status.success());
        assert!(String::from_utf8(out.stdout).unwrap().contains("review"));
    }
    assert_eq!(snapshot(&s.root), before);
}
