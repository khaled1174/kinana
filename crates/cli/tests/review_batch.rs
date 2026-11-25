//! CLI surface of the batch description accept (0.3, batch 1).
#[allow(dead_code)]
#[path = "support/json_contract.rs"]
mod contract;
#[allow(dead_code)]
mod support;
use kinana_core::{ClassificationProposal, edit, vault::Vault};
use serde_json::Value;
use std::{collections::BTreeMap, fs, path::Path, process::Command};
use support::{Sandbox, snapshot as tree};

fn run(s: &Sandbox, args: &[&str]) -> (Option<i32>, Value) {
    let output = s.run(&[args, &["--json"]].concat());
    let value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "{args:?}: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status.code(), value)
}
fn seed(s: &Sandbox, name: &str) {
    let source = s.root.join(format!("source-{name}"));
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: Original {name}\n---\n# Body\nText.\n"),
    )
    .unwrap();
    let (code, _) = run(s, &["add", source.to_str().unwrap()]);
    assert_eq!(code, Some(0));
}
fn propose(s: &Sandbox, ids: &[&str]) {
    let vault = Vault::new(
        s.vault.clone(),
        kinana_core::state::index_path(&s.home).unwrap(),
    );
    let mut updates = BTreeMap::new();
    for id in ids {
        let mut metadata = edit::read_metadata(&vault, id).unwrap();
        metadata.proposal = Some(ClassificationProposal {
            category: "backend-api".into(),
            stack: vec![],
            stage: vec![],
            tags: vec![],
            role: "Review".into(),
            when_to_use: "Before endpoints".into(),
            when_not: "Deployments".into(),
            improved_description: format!("Improved {id}"),
            overlaps_with: vec![],
            confidence: 0.8,
            category_reason: String::new(),
        });
        updates.insert(id.to_string(), metadata);
    }
    edit::apply(&vault, &updates, "Pending fixtures", false).unwrap();
}
fn head(root: &Path) -> String {
    let out = Command::new("git")
        .args(["-C", root.to_str().unwrap(), "rev-parse", "HEAD"])
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn batch_accept_previews_with_exit_two_then_confirms_one_commit_matching_the_contract() {
    let s = Sandbox::new();
    s.initialize();
    for name in ["one", "two", "three"] {
        seed(&s, name);
    }
    propose(&s, &["one", "two", "three"]);
    let doc = contract::document();
    let before = tree(&s.vault);
    let first = head(&s.vault);

    let (code, preview) = run(&s, &["review", "one", "three", "--accept"]);
    assert_eq!(code, Some(2), "{preview}");
    contract::validate(&doc, &doc["$defs"]["BatchReview"], &preview).unwrap();
    assert_eq!(preview["requires_confirmation"], true);
    assert_eq!(preview["committed"], false);
    assert_eq!(preview["items"][1]["proposed"], "Improved three");
    assert_eq!(tree(&s.vault), before);
    assert_eq!(head(&s.vault), first);
    let token = preview["preview_token"].as_str().unwrap().to_string();

    let (code, dry) = run(
        &s,
        &[
            "review",
            "one",
            "three",
            "--accept",
            "--confirm",
            &token,
            "--dry-run",
        ],
    );
    assert_eq!(code, Some(0));
    contract::validate(&doc, &doc["$defs"]["BatchReview"], &dry).unwrap();
    assert_eq!(dry["dry_run"], true);
    assert_eq!(tree(&s.vault), before);

    let (code, done) = run(
        &s,
        &["review", "one", "three", "--accept", "--confirm", &token],
    );
    assert_eq!(code, Some(0), "{done}");
    contract::validate(&doc, &doc["$defs"]["BatchReview"], &done).unwrap();
    assert_eq!(done["committed"], true);
    let log = Command::new("git")
        .args([
            "-C",
            s.vault.to_str().unwrap(),
            "rev-list",
            "--count",
            &format!("{}..HEAD", first.trim()),
        ])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&log.stdout).trim(), "1");
    let (_, two) = run(&s, &["show", "two"]);
    assert!(two["metadata"]["proposal"].is_object(), "{two}");

    // The token was spent: the state moved, so repeating it is refused.
    let (code, error) = run(
        &s,
        &["review", "one", "three", "--accept", "--confirm", &token],
    );
    assert_eq!(code, Some(2));
    assert_eq!(error["error"]["code"], "NO_DESCRIPTION_PROPOSAL");

    let (code, _) = run(&s, &["undo"]);
    assert_eq!(code, Some(0));
    assert_eq!(
        tree(&s.vault)
            .into_iter()
            .filter(|(p, _)| !p.starts_with(".git"))
            .collect::<Vec<_>>(),
        before
            .into_iter()
            .filter(|(p, _)| !p.starts_with(".git"))
            .collect::<Vec<_>>()
    );
}

#[test]
fn several_ids_need_accept_and_the_single_id_contract_is_unchanged() {
    let s = Sandbox::new();
    s.initialize();
    seed(&s, "one");
    seed(&s, "two");
    propose(&s, &["one", "two"]);
    let doc = contract::document();
    let (code, error) = run(&s, &["review", "one", "two", "--reject"]);
    assert_eq!(code, Some(1));
    assert_eq!(error["error"]["code"], "INVALID_ARGUMENTS");
    let (code, error) = run(&s, &["review", "one", "two"]);
    assert_eq!(code, Some(1));
    assert_eq!(error["error"]["code"], "INVALID_ARGUMENTS");

    let (code, single) = run(&s, &["review", "one", "--accept"]);
    assert_eq!(code, Some(0));
    contract::validate(&doc, &doc["$defs"]["Review"], &single).unwrap();
    assert_eq!(single["committed"], true);
}
