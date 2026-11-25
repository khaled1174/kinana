#[allow(dead_code)]
mod support;

use kinana_core::{ClassificationProposal, edit, frontmatter, gitlog, vault::Vault};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::PathBuf, process::Output};
use support::{Sandbox, snapshot};

fn output_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "Invalid JSON ({error}): stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn success(sandbox: &Sandbox, args: &[&str]) -> Value {
    let output = sandbox.run(args);
    assert!(
        output.status.success(),
        "{args:?}: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output_json(&output)
}

fn import_skill(sandbox: &Sandbox, id: &str, description: &str) -> PathBuf {
    let source = sandbox.root.join(format!("source-{id}"));
    fs::create_dir_all(source.join("assets/empty/deep")).unwrap();
    fs::write(
        source.join("SKILL.md"),
        format!(
            "---\r\nname: {id}\r\ndescription: {description}\r\ncustom:\r\n  enabled: true\r\n  labels: [one, two]\r\n---\r\n\r\n# Raw Markdown\r\nUnicode مرحبا\r\nNo trailing newline"
        ),
    )
    .unwrap();
    fs::write(source.join("assets/payload.bin"), [0, 255, 1, 13, 10]).unwrap();
    let original = snapshot(&source);
    let value = success(sandbox, &["add", source.to_str().unwrap(), "--json"]);
    assert_eq!(value["imported"], json!([id]));
    assert_eq!(snapshot(&source), original);
    assert_eq!(snapshot(&sandbox.vault.join("skills").join(id)), original);
    assert_eq!(snapshot(&sandbox.vault.join("dist").join(id)), original);
    source
}

fn vault(sandbox: &Sandbox) -> Vault {
    Vault::new(
        sandbox.vault.clone(),
        sandbox.home.join(".kinana/index-v2.db"),
    )
}

fn plant_proposal(sandbox: &Sandbox, id: &str, description: &str) {
    let vault = vault(sandbox);
    let mut metadata = edit::read_metadata(&vault, id).unwrap();
    metadata.classification.category = "backend-api".into();
    metadata.classification.classified_by = "claude".into();
    metadata.proposal = Some(ClassificationProposal {
        category: "backend-api".into(),
        stack: vec![],
        stage: vec![],
        tags: vec!["api".into(), "review".into()],
        role: "Review API contracts".into(),
        when_to_use: "Before implementing endpoints".into(),
        when_not: "When reviewing visual design".into(),
        improved_description: description.into(),
        overlaps_with: vec![],
        confidence: 0.9,
        category_reason: String::new(),
    });
    let result = edit::apply(
        &vault,
        &BTreeMap::from([(id.into(), metadata)]),
        "Fixture: save a classification proposal",
        false,
    )
    .unwrap();
    assert!(result.committed);
}

fn dist_description(sandbox: &Sandbox, id: &str) -> String {
    frontmatter::read_skill(&sandbox.vault.join("dist").join(id).join("SKILL.md"))
        .unwrap()
        .description
}

#[test]
fn classify_previews_json_and_text_then_uses_keyword_fallback_without_source_changes() {
    let sandbox = Sandbox::new();
    sandbox.initialize();
    let source = import_skill(&sandbox, "api-review", "Review REST API endpoints");
    let original = snapshot(&source);
    let source_bytes = fs::read(source.join("SKILL.md")).unwrap();
    let before = snapshot(&sandbox.root);
    let dry = success(
        &sandbox,
        &[
            "classify",
            "api-review",
            "--agent",
            "keywords",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(dry["dry_run"], true);
    assert_eq!(dry["selected"], 1);
    assert_eq!(dry["agent_calls"], 0);
    assert_eq!(dry["items"][0]["category"], "backend-api");
    assert_eq!(dry["items"][0]["classified_by"], "keywords");
    assert_eq!(dry["committed"], false);
    assert_eq!(snapshot(&sandbox.root), before);
    let text = sandbox.run(&["classify", "api-review", "--agent", "keywords", "--dry-run"]);
    assert!(text.status.success());
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("Selected: 1"));
    assert!(text.contains("api-review\tbackend-api\tkeywords"));
    assert_eq!(snapshot(&sandbox.root), before);
    let provider_preview = success(
        &sandbox,
        &[
            "classify",
            "api-review",
            "--agent",
            "claude",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(provider_preview["estimated_agent_calls"], 1);
    assert_eq!(provider_preview["agent_calls"], 0);
    assert_eq!(snapshot(&sandbox.root), before);

    let head = gitlog::head(&sandbox.vault).unwrap();
    let classified = success(
        &sandbox,
        &["classify", "api-review", "--agent", "keywords", "--json"],
    );
    assert_eq!(classified["selected"], 1);
    assert_eq!(classified["agent_calls"], 0);
    assert_eq!(classified["items"][0]["classified_by"], "keywords");
    assert_eq!(classified["items"][0]["proposal_created"], false);
    assert_eq!(classified["items"][0]["category"], "backend-api");
    assert_eq!(classified["committed"], true);
    assert_ne!(gitlog::head(&sandbox.vault).unwrap(), head);
    let shown = success(&sandbox, &["show", "api-review", "--json"]);
    assert_eq!(shown["metadata"]["description"]["active"], "original");
    assert!(shown["metadata"]["proposal"].is_null());
    assert_eq!(snapshot(&source), original);
    assert_eq!(snapshot(&sandbox.vault.join("skills/api-review")), original);
    assert_eq!(
        fs::read(sandbox.vault.join("dist/api-review/SKILL.md")).unwrap(),
        source_bytes
    );
}

#[test]
fn classify_ids_all_and_pending_require_batch_confirmation_and_select_exact_targets() {
    let sandbox = Sandbox::new();
    sandbox.initialize();
    import_skill(&sandbox, "api", "Review REST API endpoints");
    import_skill(&sandbox, "frontend", "Review React CSS frontend");
    let originals = snapshot(&sandbox.vault.join("skills"));
    for args in [
        &[
            "classify", "api", "frontend", "--agent", "keywords", "--json",
        ][..],
        &["classify", "--all", "--agent", "keywords", "--json"],
        &["classify", "--pending", "--agent", "keywords", "--json"],
    ] {
        let before = snapshot(&sandbox.root);
        let output = sandbox.run(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert_eq!(
            output_json(&output)["error"]["code"],
            "CLASSIFY_CONFIRMATION_REQUIRED"
        );
        assert_eq!(snapshot(&sandbox.root), before);
    }
    let dry = success(
        &sandbox,
        &[
            "classify",
            "--all",
            "--agent",
            "keywords",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(dry["selected"], 2);
    let one = success(
        &sandbox,
        &["classify", "api", "--agent", "keywords", "--json"],
    );
    assert_eq!(one["selected"], 1);
    assert_eq!(one["items"][0]["id"], "api");
    let pending = success(
        &sandbox,
        &["classify", "--pending", "--agent", "keywords", "--json"],
    );
    assert_eq!(pending["selected"], 1);
    assert_eq!(pending["items"][0]["id"], "frontend");
    assert_eq!(pending["items"][0]["category"], "web-frontend");
    let before = snapshot(&sandbox.root);
    let pending = success(
        &sandbox,
        &["classify", "--pending", "--agent", "keywords", "--json"],
    );
    assert_eq!(pending["selected"], 0);
    assert_eq!(pending["committed"], false);
    assert_eq!(snapshot(&sandbox.root), before);
    let all = success(
        &sandbox,
        &[
            "classify", "--all", "--agent", "keywords", "--yes", "--json",
        ],
    );
    assert_eq!(all["selected"], 2);
    let ids = success(
        &sandbox,
        &[
            "classify", "frontend", "api", "--agent", "keywords", "--yes", "--json",
        ],
    );
    assert_eq!(ids["selected"], 2);
    assert_eq!(ids["items"][0]["id"], "api");
    assert_eq!(ids["items"][1]["id"], "frontend");
    assert_eq!(snapshot(&sandbox.vault.join("skills")), originals);
}

#[test]
fn review_previews_accepts_rejects_and_restores_original_without_touching_source_bytes() {
    let sandbox = Sandbox::new();
    sandbox.initialize();
    let source = import_skill(&sandbox, "review", "Review REST API contracts");
    let originals = snapshot(&source);
    let original_md = fs::read(source.join("SKILL.md")).unwrap();
    let accepted = "Before implementing REST endpoints, review API contracts and compatibility.";
    plant_proposal(&sandbox, "review", accepted);
    assert_eq!(
        fs::read(sandbox.vault.join("dist/review/SKILL.md")).unwrap(),
        original_md
    );
    let before = snapshot(&sandbox.root);
    let preview = success(&sandbox, &["review", "review", "--json"]);
    assert_eq!(preview["decision"], "preview");
    assert_eq!(preview["original"], "Review REST API contracts");
    assert_eq!(preview["current"], "Review REST API contracts");
    assert_eq!(preview["proposed"], accepted);
    assert_eq!(preview["active"], "original");
    assert_eq!(preview["committed"], false);
    let text = sandbox.run(&["review", "review"]);
    assert!(text.status.success());
    let text = String::from_utf8(text.stdout).unwrap();
    for expected in [
        "Original:",
        "Current:",
        "Proposed:",
        accepted,
        "Active: original",
    ] {
        assert!(text.contains(expected));
    }
    for decision in ["--accept", "--reject", "--original"] {
        let dry = success(
            &sandbox,
            &["review", "review", decision, "--dry-run", "--json"],
        );
        assert_eq!(dry["dry_run"], true);
        assert_eq!(dry["committed"], false);
        assert_eq!(snapshot(&sandbox.root), before);
    }
    let head = gitlog::head(&sandbox.vault).unwrap();
    let reviewed = success(&sandbox, &["review", "review", "--accept", "--json"]);
    assert_eq!(reviewed["decision"], "accept");
    assert_eq!(reviewed["active"], "improved");
    assert_eq!(reviewed["committed"], true);
    assert_ne!(gitlog::head(&sandbox.vault).unwrap(), head);
    assert_eq!(dist_description(&sandbox, "review"), accepted);
    let shown = success(&sandbox, &["show", "review", "--json"]);
    assert_eq!(shown["metadata"]["description"]["improved"], accepted);
    assert!(shown["metadata"]["description"]["approved_at"].is_string());
    assert!(shown["metadata"]["proposal"].is_null());
    let built = fs::read_to_string(sandbox.vault.join("dist/review/SKILL.md")).unwrap();
    let original_text = String::from_utf8(original_md.clone()).unwrap();
    assert_eq!(
        built.split_once("---\r\n\r\n").unwrap().1,
        original_text.split_once("---\r\n\r\n").unwrap().1
    );

    plant_proposal(
        &sandbox,
        "review",
        "A newer proposal that should be rejected.",
    );
    let rejected = success(&sandbox, &["review", "review", "--reject", "--json"]);
    assert_eq!(rejected["decision"], "reject");
    assert_eq!(rejected["active"], "improved");
    assert_eq!(rejected["committed"], true);
    assert_eq!(dist_description(&sandbox, "review"), accepted);
    let shown = success(&sandbox, &["show", "review", "--json"]);
    assert!(shown["metadata"]["proposal"].is_null());
    let restored = success(&sandbox, &["review", "review", "--original", "--json"]);
    assert_eq!(restored["active"], "original");
    assert_eq!(restored["committed"], true);
    assert_eq!(
        fs::read(sandbox.vault.join("dist/review/SKILL.md")).unwrap(),
        original_md
    );
    assert_eq!(snapshot(&source), originals);
    assert_eq!(snapshot(&sandbox.vault.join("skills/review")), originals);
}

#[test]
fn build_id_and_all_preview_without_writes_and_rebuild_only_the_missing_owned_target() {
    let sandbox = Sandbox::new();
    sandbox.initialize();
    import_skill(&sandbox, "first", "Review REST API endpoints");
    import_skill(&sandbox, "second", "Review React CSS frontend");
    let originals = snapshot(&sandbox.vault.join("skills"));
    fs::remove_dir_all(sandbox.vault.join("dist/first")).unwrap();
    let head = gitlog::head(&sandbox.vault).unwrap();
    let before = snapshot(&sandbox.root);
    for args in [
        &["build", "first", "--dry-run", "--json"][..],
        &["build", "--dry-run", "--json"],
    ] {
        let report = success(&sandbox, args);
        assert_eq!(report["dry_run"], true);
        assert_eq!(report["items"][0]["id"], "first");
        assert_eq!(report["items"][0]["changed"], true);
        assert_eq!(snapshot(&sandbox.root), before);
    }
    let text = sandbox.run(&["build", "first", "--dry-run"]);
    assert!(text.status.success());
    assert!(
        String::from_utf8(text.stdout)
            .unwrap()
            .contains("first\toriginal\tchanged")
    );
    assert_eq!(snapshot(&sandbox.root), before);
    let second_before = snapshot(&sandbox.vault.join("dist/second"));
    let second_modified = fs::metadata(sandbox.vault.join("dist/second/SKILL.md"))
        .unwrap()
        .modified()
        .unwrap();
    let report = success(&sandbox, &["build", "first", "--json"]);
    assert_eq!(report["items"].as_array().unwrap().len(), 1);
    assert_eq!(report["items"][0]["changed"], true);
    assert_eq!(
        snapshot(&sandbox.vault.join("dist/first")),
        snapshot(&sandbox.vault.join("skills/first"))
    );
    assert_eq!(snapshot(&sandbox.vault.join("dist/second")), second_before);
    assert_eq!(
        fs::metadata(sandbox.vault.join("dist/second/SKILL.md"))
            .unwrap()
            .modified()
            .unwrap(),
        second_modified
    );
    assert_eq!(gitlog::head(&sandbox.vault).unwrap(), head);
    let before = snapshot(&sandbox.root);
    let report = success(&sandbox, &["build", "--json"]);
    assert_eq!(report["items"].as_array().unwrap().len(), 2);
    assert!(
        report["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["changed"] == false)
    );
    assert_eq!(snapshot(&sandbox.root), before);
    assert_eq!(gitlog::head(&sandbox.vault).unwrap(), head);
    assert_eq!(snapshot(&sandbox.vault.join("skills")), originals);
}

#[test]
fn undo_resynchronizes_approved_and_original_descriptions() {
    let sandbox = Sandbox::new();
    sandbox.initialize();
    let source = import_skill(&sandbox, "undo-review", "Review REST API contracts");
    let originals = snapshot(&source);
    let accepted = "Review REST API compatibility before implementation.";
    plant_proposal(&sandbox, "undo-review", accepted);
    success(&sandbox, &["review", "undo-review", "--accept", "--json"]);
    assert_eq!(dist_description(&sandbox, "undo-review"), accepted);
    let before = snapshot(&sandbox.root);
    let dry = success(&sandbox, &["undo", "--dry-run", "--json"]);
    assert_eq!(dry["dry_run"], true);
    assert_eq!(snapshot(&sandbox.root), before);
    success(&sandbox, &["undo", "--json"]);
    assert_eq!(
        fs::read(sandbox.vault.join("dist/undo-review/SKILL.md")).unwrap(),
        fs::read(source.join("SKILL.md")).unwrap()
    );
    assert_eq!(
        success(&sandbox, &["show", "undo-review", "--json"])["metadata"]["description"]["active"],
        "original"
    );
    success(&sandbox, &["undo", "--json"]);
    assert_eq!(dist_description(&sandbox, "undo-review"), accepted);
    success(&sandbox, &["review", "undo-review", "--original", "--json"]);
    assert_eq!(
        dist_description(&sandbox, "undo-review"),
        "Review REST API contracts"
    );
    success(&sandbox, &["undo", "--json"]);
    assert_eq!(dist_description(&sandbox, "undo-review"), accepted);
    assert_eq!(snapshot(&source), originals);
    assert_eq!(
        snapshot(&sandbox.vault.join("skills/undo-review")),
        originals
    );
}

#[test]
fn undo_import_prunes_owned_dist_and_preserves_foreign_entries_then_rebuilds_on_redo() {
    let sandbox = Sandbox::new();
    sandbox.initialize();
    let source = import_skill(&sandbox, "undo-import", "Review REST API contracts");
    let originals = snapshot(&source);
    fs::create_dir(sandbox.vault.join("dist/foreign")).unwrap();
    fs::write(sandbox.vault.join("dist/foreign/owner.txt"), "preserve").unwrap();
    success(&sandbox, &["undo", "--json"]);
    for path in [
        "skills/undo-import",
        "meta/undo-import.toml",
        "dist/undo-import",
    ] {
        assert!(!sandbox.vault.join(path).exists(), "left {path}");
    }
    assert_eq!(
        fs::read(sandbox.vault.join("dist/foreign/owner.txt")).unwrap(),
        b"preserve"
    );
    success(&sandbox, &["undo", "--json"]);
    assert_eq!(
        snapshot(&sandbox.vault.join("skills/undo-import")),
        originals
    );
    assert_eq!(snapshot(&sandbox.vault.join("dist/undo-import")), originals);
    assert_eq!(snapshot(&source), originals);
    assert_eq!(
        fs::read(sandbox.vault.join("dist/foreign/owner.txt")).unwrap(),
        b"preserve"
    );
}

#[test]
fn phase_three_selection_errors_and_missing_proposals_return_stable_json_without_writes() {
    let sandbox = Sandbox::new();
    sandbox.initialize();
    import_skill(&sandbox, "review", "Review REST API contracts");
    for args in [
        &["classify", "--json"][..],
        &["classify", "review", "--all", "--json"],
        &["classify", "--all", "--pending", "--json"],
        &["classify", "review", "--agent", "unknown", "--json"],
        &[
            "classify", "review", "review", "--agent", "keywords", "--json",
        ],
        &["review", "review", "--accept", "--reject", "--json"],
        &["build", "../escape", "--json"],
        &["build", "missing", "--json"],
    ] {
        let before = snapshot(&sandbox.root);
        let output = sandbox.run(args);
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        let error = output_json(&output);
        assert!(error["error"]["code"].is_string());
        assert!(error["error"]["message"].is_string());
        assert!(error["error"]["reason_code"].is_string());
        assert_eq!(error["error"].as_object().unwrap().len(), 3);
        assert_eq!(snapshot(&sandbox.root), before);
    }
    for decision in ["--accept", "--reject"] {
        let before = snapshot(&sandbox.root);
        let output = sandbox.run(&["review", "review", decision, "--json"]);
        assert_eq!(output.status.code(), Some(2));
        assert_eq!(
            output_json(&output)["error"]["code"],
            "NO_DESCRIPTION_PROPOSAL"
        );
        assert_eq!(snapshot(&sandbox.root), before);
    }
}
