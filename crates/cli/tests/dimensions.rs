//! CLI surface of the classification dimensions (8.3).
#[allow(dead_code)]
mod support;
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};
use support::{Sandbox, snapshot as tree};

fn json(s: &Sandbox, args: &[&str]) -> Value {
    let output = s.run(&[args, &["--json"]].concat());
    assert!(
        output.status.success(),
        "{:?}: {} {}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn failure(s: &Sandbox, args: &[&str], exit: i32) -> Value {
    let output = s.run(&[args, &["--json"]].concat());
    assert_eq!(
        output.status.code(),
        Some(exit),
        "{:?}: {}",
        args,
        String::from_utf8_lossy(&output.stdout)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    value["error"].clone()
}
fn seed(s: &Sandbox, name: &str, description: &str) {
    let source = s.root.join(format!("source-{name}"));
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {description}\n---\n# Body\nText.\n"),
    )
    .unwrap();
    json(s, &["add", source.to_str().unwrap()]);
}
fn head(root: &Path) -> String {
    let out = Command::new("git")
        .args(["-C", root.to_str().unwrap(), "rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap()
}
fn ids(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|skill| skill["id"].as_str().unwrap())
        .collect()
}

#[test]
fn dimensions_command_sets_previews_clears_and_undoes_with_a_fixed_json_shape() {
    let s = Sandbox::new();
    s.initialize();
    seed(&s, "api", "Review API contracts");
    let read_meta = || -> toml::Value {
        toml::from_str(&fs::read_to_string(s.vault.join("meta/api.toml")).unwrap()).unwrap()
    };
    let original = read_meta();
    assert_eq!(json(&s, &["list"])[0]["stack"], json!([]));
    assert_eq!(json(&s, &["list"])[0]["stage"], json!([]));

    // A preview reports the result and writes nothing anywhere.
    let before = tree(&s.root);
    let first = head(&s.vault);
    let preview = json(
        &s,
        &[
            "dimensions",
            "api",
            "--stack",
            "node",
            "--stack",
            "react",
            "--stage",
            "review",
            "--dry-run",
        ],
    );
    assert_eq!(
        preview,
        json!({"id":"api","stack":["react","node"],"stage":["review"],
               "dry_run":true,"changed":true,"committed":false})
    );
    assert_eq!(tree(&s.root), before);
    assert_eq!(head(&s.vault), first);

    let changed = json(
        &s,
        &[
            "dimensions",
            "api",
            "--stack",
            "node",
            "--stack",
            "react",
            "--stage",
            "review",
        ],
    );
    assert_eq!(
        changed,
        json!({"id":"api","stack":["react","node"],"stage":["review"],
               "dry_run":false,"changed":true,"committed":true})
    );
    let set = head(&s.vault);
    assert_ne!(set, first);
    for report in [
        json(&s, &["list"]),
        json(&s, &["search", "contracts"])["items"].clone(),
    ] {
        assert_eq!(report[0]["stack"], json!(["react", "node"]));
        assert_eq!(report[0]["stage"], json!(["review"]));
    }
    let shown = json(&s, &["show", "api"]);
    assert_eq!(shown["skill"]["stack"], json!(["react", "node"]));
    assert_eq!(
        shown["metadata"]["classification"]["stage"],
        json!(["review"])
    );
    assert_eq!(
        shown["metadata"]["classification"]["classified_by"],
        "pending"
    );

    // Repeating the same request commits nothing.
    let same = json(&s, &["dimensions", "api", "--stage", "review"]);
    assert_eq!(same["changed"], false);
    assert_eq!(same["committed"], false);
    assert_eq!(head(&s.vault), set);

    // One dimension can be cleared while the other is left alone.
    let cleared = json(&s, &["dimensions", "api", "--clear-stack"]);
    assert_eq!(cleared["stack"], json!([]));
    assert_eq!(cleared["stage"], json!(["review"]));
    assert_eq!(json(&s, &["show", "api"])["skill"]["stack"], json!([]));
    json(&s, &["undo"]);
    assert_eq!(json(&s, &["list"])[0]["stack"], json!(["react", "node"]));
    json(&s, &["dimensions", "api", "--clear-stack", "--clear-stage"]);
    // Cleared dimensions leave no key behind: the metadata is what it was.
    assert_eq!(read_meta(), original);
    assert!(read_meta()["classification"].get("stack").is_none());

    // Text output names the values as well.
    let text = s.run(&["dimensions", "api", "--stack", "rust", "--dry-run"]);
    assert!(text.status.success());
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("api") && text.contains("rust") && text.contains("preview"));
}

#[test]
fn out_of_list_values_and_empty_requests_are_refused_without_any_write() {
    let s = Sandbox::new();
    s.initialize();
    seed(&s, "api", "Review API contracts");
    let before = tree(&s.root);
    for dry_run in [&[][..], &["--dry-run"][..]] {
        let error = failure(
            &s,
            &[&["dimensions", "api", "--stack", "reactjs"], dry_run].concat(),
            1,
        );
        assert_eq!(error["code"], "INVALID_STACK");
        assert_eq!(error["reason_code"], "INVALID_STACK");
        assert!(error["message"].as_str().unwrap().contains("reactjs"));
        let error = failure(
            &s,
            &[
                &["dimensions", "api", "--stack", "react", "--stage", "deploy"],
                dry_run,
            ]
            .concat(),
            1,
        );
        assert_eq!(error["code"], "INVALID_STAGE");
    }
    assert_eq!(
        failure(&s, &["dimensions", "api"], 1)["code"],
        "INVALID_ARGUMENTS"
    );
    assert_eq!(
        failure(
            &s,
            &["dimensions", "api", "--stack", "react", "--clear-stack"],
            1
        )["code"],
        "INVALID_ARGUMENTS"
    );
    assert_eq!(
        failure(&s, &["dimensions", "../api", "--stack", "react"], 1)["code"],
        "INVALID_SKILL_ID"
    );
    assert_eq!(
        failure(&s, &["list", "--stack", "reactjs"], 1)["code"],
        "INVALID_STACK"
    );
    assert_eq!(
        failure(&s, &["list", "--stage", "deploy"], 1)["code"],
        "INVALID_STAGE"
    );
    assert_eq!(tree(&s.root), before);
}

#[test]
fn list_filters_by_stack_and_stage_and_combines_them_with_the_existing_filters() {
    let s = Sandbox::new();
    s.initialize();
    for (name, category, stack, stage) in [
        (
            "alpha",
            "web-frontend",
            &["react", "next"][..],
            &["build"][..],
        ),
        ("beta", "web-frontend", &["react"][..], &["review"][..]),
        (
            "gamma",
            "testing-qa",
            &["react"][..],
            &["test", "review"][..],
        ),
        ("delta", "web-frontend", &[][..], &[][..]),
    ] {
        seed(&s, name, "Describe the skill");
        json(&s, &["category", name, category]);
        let mut args = vec!["dimensions", name];
        for value in stack {
            args.extend(["--stack", value]);
        }
        for value in stage {
            args.extend(["--stage", value]);
        }
        if args.len() > 2 {
            json(&s, &args);
        }
    }
    json(&s, &["pin", "gamma"]);
    let before = tree(&s.root);
    assert_eq!(
        ids(&json(&s, &["list"])),
        ["gamma", "alpha", "beta", "delta"]
    );
    assert_eq!(
        ids(&json(&s, &["list", "--stack", "react"])),
        ["gamma", "alpha", "beta"]
    );
    assert_eq!(ids(&json(&s, &["list", "--stack", "next"])), ["alpha"]);
    assert_eq!(
        ids(&json(&s, &["list", "--stage", "review"])),
        ["gamma", "beta"]
    );
    assert_eq!(
        ids(&json(
            &s,
            &["list", "--stack", "react", "--category", "web-frontend"]
        )),
        ["alpha", "beta"]
    );
    assert_eq!(
        ids(&json(
            &s,
            &[
                "list",
                "--stack",
                "react",
                "--stage",
                "review",
                "--category",
                "web-frontend"
            ]
        )),
        ["beta"]
    );
    assert_eq!(
        ids(&json(
            &s,
            &["list", "--stage", "review", "--pinned", "--agent", "claude"]
        )),
        ["gamma"]
    );
    // A configured value nobody holds is an empty list, not an error.
    assert_eq!(json(&s, &["list", "--stack", "rust"]), json!([]));
    assert_eq!(tree(&s.root), before);
}

#[test]
fn search_finds_a_skill_by_tag_and_by_when_to_use_but_not_by_when_not() {
    use kinana_core::{edit, vault::Vault};
    use std::collections::BTreeMap;
    let s = Sandbox::new();
    s.initialize();
    seed(&s, "payments", "Take card payments");
    seed(&s, "notes", "Summarize meetings");
    let vault = Vault::new(s.vault.clone(), s.home.join(".kinana/index-v2.db"));
    let mut metadata = edit::read_metadata(&vault, "payments").unwrap();
    metadata.classification.tags = vec!["stripe".into()];
    metadata.classification.when_to_use = "Before wiring a subscription provider".into();
    metadata.classification.when_not = "Never for cryptocurrency wallets".into();
    edit::apply(
        &vault,
        &BTreeMap::from([("payments".into(), metadata)]),
        "Fixture: classified",
        false,
    )
    .unwrap();
    let before = tree(&s.root);
    for query in ["stripe", "subscription", "stripe subscription"] {
        let report = json(&s, &["search", query]);
        assert_eq!(ids(&report["items"]), ["payments"], "{query}");
        assert_eq!(
            report["items"][0]["when_to_use"],
            "Before wiring a subscription provider"
        );
    }
    assert_eq!(json(&s, &["search", "cryptocurrency"])["items"], json!([]));
    assert_eq!(ids(&json(&s, &["list", "--query", "stripe"])), ["payments"]);
    assert_eq!(tree(&s.root), before);
}

#[test]
fn suggest_puts_react_skills_first_for_a_react_project_and_explains_each_match() {
    let s = Sandbox::new();
    s.initialize();
    for (name, category, stack) in [
        ("aaa-generic", "web-frontend", None),
        ("react-hooks", "web-frontend", Some("react")),
        ("react-tests", "testing-qa", Some("react")),
        ("zzz-other", "data-db", Some("python")),
    ] {
        seed(&s, name, "Describe the skill");
        json(&s, &["category", name, category]);
        if let Some(stack) = stack {
            json(&s, &["dimensions", name, "--stack", stack]);
        }
    }
    let project = s.root.join("project");
    fs::create_dir(&project).unwrap();
    fs::write(
        project.join("package.json"),
        r#"{"dependencies":{"react":"19"}}"#,
    )
    .unwrap();
    let before = tree(&s.root);
    let report = json(&s, &["suggest", "--project", project.to_str().unwrap()]);
    assert_eq!(report["categories"], json!(["web-frontend"]));
    assert_eq!(report["stacks"], json!(["react", "node"]));
    assert_eq!(
        ids(&report["skills"]),
        ["react-hooks", "react-tests", "aaa-generic"]
    );
    assert_eq!(
        report["matches"],
        json!([
            {"id":"react-hooks","matched_stacks":["react"],"matched_category":true},
            {"id":"react-tests","matched_stacks":["react"],"matched_category":false},
            {"id":"aaa-generic","matched_stacks":[],"matched_category":true},
        ])
    );
    let signals = report["stack_signals"].as_array().unwrap();
    assert_eq!(signals.len(), 2);
    assert!(signals.iter().all(
        |signal| signal["path"].as_str().unwrap().ends_with("package.json")
            && signal["reason"].is_string()
    ));
    assert_eq!(tree(&s.root), before);
    let text = s.run(&["suggest", "--project", project.to_str().unwrap()]);
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("Stacks: react, node"), "{text}");
}

#[test]
fn keyword_classification_reports_named_stacks_through_the_cli() {
    let s = Sandbox::new();
    s.initialize();
    seed(
        &s,
        "native",
        "Build screens with React Native and Next.js docs",
    );
    json(&s, &["classify", "native", "--agent", "keywords"]);
    let skill = json(&s, &["show", "native"])["skill"].clone();
    assert_eq!(skill["stack"], json!(["next", "react-native"]));
    assert_eq!(skill["stage"], json!([]));
}
