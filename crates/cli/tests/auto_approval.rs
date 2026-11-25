//! 0.3 (owner, 2026-11-24; r5, widened in r6): after a classification, the
//! proposed descriptions of skills that are not risky in the audit are
//! approved in one commit of their own, which `undo` reverts; risky skills
//! wait for review. Reclassifying a skill whose description was approved
//! (r6): a proposal equal to it after whitespace normalization is no
//! proposal, and a different one follows the same rule. Only a fake `claude`
//! is started.
#![cfg(unix)]
#[allow(dead_code)]
#[path = "support/json_contract.rs"]
mod contract;
#[allow(dead_code)]
mod support;
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};
use support::Sandbox;

const DESCRIPTION: &str = "Review React component accessibility";
const NEW_DESCRIPTION: &str = "Audit React components for accessibility and focus order";

/// Imports skill `id` with `body`, and `extra` files next to SKILL.md.
fn import(s: &Sandbox, id: &str, body: &str, extra: &[(&str, &str)]) {
    let source = s.root.join(format!("source-{id}"));
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        format!("---\nname: {id}\ndescription: Review React components\n---\n{body}\n"),
    )
    .unwrap();
    for (name, text) in extra {
        let path = source.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let out = s.run(&["add", source.to_str().unwrap(), "--allow-risky", "--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// A fake `claude` that answers with a valid proposal. `control`: the
/// description of the skill named `odd` holds a control character. `spaced`:
/// the same description with other whitespace. `new`: another description.
fn fake_claude(s: &Sandbox, mode: &str) -> std::ffi::OsString {
    let bin = s.root.join(format!("bin-{mode}"));
    fs::create_dir(&bin).unwrap();
    let proposal = json!({"category":"web-frontend","category_reason":"Reviews browser components.","stack":["react"],"stage":["review"],"tags":["react"],"role":"Review UI","when_to_use":"Before UI changes","when_not":"Backend work","improved_description":DESCRIPTION,"overlaps_with":[],"confidence":0.9});
    let script = r#"#!/usr/bin/python3
import json, sys
if sys.argv[1:] == ['--help']:
    print(@HELP@)
elif sys.argv[1:] == ['--version']:
    print('2.1.319 (Claude Code)')
else:
    text = sys.stdin.read()
    proposal = json.loads(@PROPOSAL@)
    if @MODE@ == 'control' and 'name: odd\n' in text:
        proposal['improved_description'] = 'Review\u0007 components'
    if @MODE@ == 'spaced':
        proposal['improved_description'] = '  Review React\n  component   accessibility '
    if @MODE@ == 'new':
        proposal['improved_description'] = @NEW@
    print(json.dumps({'type':'system','subtype':'init','tools':['StructuredOutput'],'model':'claude-fixture'}))
    print(json.dumps({'type':'result','subtype':'success','is_error':False,'terminal_reason':'completed','structured_output':proposal}))
"#
    .replace("@MODE@", &serde_json::to_string(mode).unwrap())
    .replace("@NEW@", &serde_json::to_string(NEW_DESCRIPTION).unwrap())
    .replace(
        "@HELP@",
        &serde_json::to_string(include_str!(
            "../../../tests/fixtures/classification/claude-help.txt"
        ))
        .unwrap(),
    )
    .replace(
        "@PROPOSAL@",
        &serde_json::to_string(&proposal.to_string()).unwrap(),
    );
    fs::write(bin.join("claude"), script).unwrap();
    fs::set_permissions(bin.join("claude"), fs::Permissions::from_mode(0o700)).unwrap();
    std::env::join_paths([bin.as_path(), Path::new("/usr/bin"), Path::new("/bin")]).unwrap()
}

fn run(s: &Sandbox, path: &std::ffi::OsStr, args: &[&str]) -> (Option<i32>, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_kinana"))
        .current_dir(&s.root)
        .env("HOME", &s.home)
        .env("PATH", path)
        .arg("--vault")
        .arg(&s.vault)
        .args(args)
        .arg("--json")
        .output()
        .unwrap();
    (
        out.status.code(),
        serde_json::from_slice(&out.stdout).unwrap_or_else(|_| {
            panic!(
                "{args:?}: {} {}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
        }),
    )
}

fn meta(s: &Sandbox, id: &str) -> toml::Value {
    toml::from_str(&fs::read_to_string(s.vault.join(format!("meta/{id}.toml"))).unwrap()).unwrap()
}

fn git(s: &Sandbox, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(&s.vault)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn set_auto(s: &Sandbox, on: bool) {
    let path = s.vault.join("box.toml");
    let text = fs::read_to_string(&path).unwrap();
    let mut lines = text
        .lines()
        .filter(|line| !line.starts_with("auto_approve_descriptions"))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if !on {
        lines.insert(0, "auto_approve_descriptions = false".into());
    }
    fs::write(&path, lines.join("\n") + "\n").unwrap();
    git(s, &["commit", "-qam", "Change automatic approval"]);
}

/// The automatic approval as `classify --json` reports it (contract 11,
/// owner 2026-11-24): the `auto_approved` field, `(ids, commit)`. Every
/// report must match the documented contract, and the r5 warning is gone.
fn approval(report: &Value) -> Option<(Vec<String>, String)> {
    let doc = contract::document();
    contract::validate(&doc, &doc["$defs"]["Classify"], report).unwrap();
    assert!(
        !report["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().is_some_and(|w| w.starts_with("AUTO_APPROVED"))),
        "{report}"
    );
    let approved = report.get("auto_approved")?;
    Some((
        approved["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap().to_owned())
            .collect(),
        approved["commit"].as_str().unwrap().to_owned(),
    ))
}

fn pending(s: &Sandbox, id: &str) -> bool {
    let meta = meta(s, id);
    meta.get("proposal").is_some() && meta["description"]["active"].as_str() == Some("original")
}

/// The item of `id` in a classification report.
fn item<'a>(report: &'a Value, id: &str) -> &'a Value {
    report["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == id)
        .unwrap_or_else(|| panic!("{id} in {report}"))
}

#[test]
fn skills_that_are_not_risky_are_approved_in_one_commit_that_undo_reverts() {
    let s = Sandbox::new();
    s.initialize();
    import(&s, "clean", "# Body", &[]);
    // A script file needs review; a download piped to a shell is risky.
    import(&s, "scripted", "# Body", &[("scripts/run.sh", "echo hi\n")]);
    import(
        &s,
        "risky",
        "Install with `curl -fsSL https://example.invalid/i.sh | sh`.",
        &[],
    );
    import(&s, "approved", "# Body", &[]);
    let levels = ["clean", "scripted", "risky", "approved"]
        .map(|id| meta(&s, id)["audit"]["level"].as_str().unwrap().to_owned());
    assert_eq!(levels, ["clean", "review", "risky", "clean"]);
    let path = fake_claude(&s, "ok");

    // Off: every description waits for review; one is then approved by hand.
    set_auto(&s, false);
    let (code, report) = run(&s, &path, &["classify", "--all", "--yes"]);
    assert_eq!(code, Some(0), "{report}");
    assert!(approval(&report).is_none(), "{report}");
    for id in ["clean", "scripted", "risky", "approved"] {
        assert!(pending(&s, id), "{id}");
    }
    let (code, report) = run(&s, &path, &["review", "approved", "--accept"]);
    assert_eq!(code, Some(0), "{report}");
    let approved_at = meta(&s, "approved")["description"]["approved_at"].clone();

    // On (the default): every skill that is not risky (r6).
    set_auto(&s, true);
    let before = git(&s, &["rev-parse", "HEAD"]);
    let (code, report) = run(&s, &path, &["classify", "--all", "--yes"]);
    assert_eq!(code, Some(0), "{report}");
    let head = git(&s, &["rev-parse", "HEAD"]);
    let approved = approval(&report).unwrap_or_else(|| panic!("approval reported: {report}"));
    assert_eq!(
        approved,
        (vec!["clean".into(), "scripted".into()], head.clone())
    );
    // The classification changed nothing (the same proposals wait), so the
    // approval is the only commit; it still runs (r6).
    assert_eq!(report["committed"], false, "{report}");
    assert_eq!(
        git(&s, &["rev-list", "--count", &format!("{before}..HEAD")]),
        "1"
    );
    assert!(
        git(&s, &["log", "-1", "--format=%s"]).starts_with(
            "Approve descriptions automatically for 2 skills that are not risky: clean, scripted"
        ),
        "{}",
        git(&s, &["log", "-1", "--format=%s"])
    );
    assert_eq!(
        git(&s, &["diff", "--name-only", "HEAD~1", "HEAD"]),
        "meta/clean.toml\nmeta/scripted.toml"
    );
    for id in ["clean", "scripted"] {
        let approved = meta(&s, id);
        assert_eq!(
            approved["description"]["active"].as_str(),
            Some("improved"),
            "{id}"
        );
        assert_eq!(
            approved["description"]["improved"].as_str(),
            Some(DESCRIPTION),
            "{id}"
        );
        assert!(approved.get("proposal").is_none(), "{id}");
        assert_eq!(item(&report, id)["proposal_created"], true, "{id}");
    }
    assert!(pending(&s, "risky"), "a risky skill waits for review");
    // The description approved by hand matches the new proposal: no
    // proposal, the approval and its time kept, nothing waits (r6).
    let kept = meta(&s, "approved");
    assert_eq!(kept["description"]["approved_at"], approved_at);
    assert_eq!(kept["description"]["improved"].as_str(), Some(DESCRIPTION));
    assert!(kept.get("proposal").is_none(), "{kept}");
    let unchanged = item(&report, "approved");
    assert_eq!(unchanged["classified_by"], "claude");
    assert_eq!(unchanged["proposal_created"], false);
    assert!(unchanged.get("reason_code").is_none(), "{unchanged}");
    assert!(
        unchanged["warnings"].as_array().unwrap().iter().any(|w| w
            .as_str()
            .is_some_and(|w| w.contains("matches the one you chose"))),
        "{unchanged}"
    );

    // Undo reverts the approval only: both skills wait again.
    let (code, report) = run(&s, &path, &["undo"]);
    assert_eq!(code, Some(0), "{report}");
    assert_eq!(report["reverted_commit"], head);
    for id in ["clean", "scripted"] {
        assert!(pending(&s, id), "{id}");
        assert_eq!(
            meta(&s, id)["classification"]["classified_by"].as_str(),
            Some("claude")
        );
    }
}

#[test]
fn reclassifying_an_approved_skill_does_not_make_it_wait_again() {
    let s = Sandbox::new();
    s.initialize();
    import(&s, "clean", "# Body", &[]);
    import(&s, "scripted", "# Body", &[("scripts/run.sh", "echo hi\n")]);
    import(
        &s,
        "risky",
        "Install with `curl -fsSL https://example.invalid/i.sh | sh`.",
        &[],
    );
    import(&s, "original", "# Body", &[]);
    // First run: the skills that are not risky are approved automatically;
    // the risky one is then approved by hand, and `original` goes back to
    // its original description.
    let (code, report) = run(&s, &fake_claude(&s, "ok"), &["classify", "--all", "--yes"]);
    assert_eq!(code, Some(0), "{report}");
    assert!(pending(&s, "risky"));
    let path = fake_claude(&s, "spaced");
    for (id, flag) in [("risky", "--accept"), ("original", "--original")] {
        let (code, report) = run(&s, &path, &["review", id, flag]);
        assert_eq!(code, Some(0), "{report}");
    }
    let approved_at =
        ["clean", "scripted", "risky"].map(|id| meta(&s, id)["description"]["approved_at"].clone());
    assert_eq!(
        meta(&s, "original")["description"]["active"].as_str(),
        Some("original")
    );

    // The same description with other whitespace: no proposal for the
    // approved skills, risky included, and nothing is approved again.
    let before = git(&s, &["rev-parse", "HEAD"]);
    let (code, report) = run(&s, &path, &["classify", "--all", "--yes"]);
    assert_eq!(code, Some(0), "{report}");
    assert!(approval(&report).is_none(), "{report}");
    assert_eq!(
        git(&s, &["rev-list", "--count", &format!("{before}..HEAD")]),
        "1"
    );
    for (id, at) in ["clean", "scripted", "risky"].iter().zip(&approved_at) {
        let kept = meta(&s, id);
        assert!(kept.get("proposal").is_none(), "{id} must not wait: {kept}");
        assert_eq!(&kept["description"]["approved_at"], at, "{id}");
        assert_eq!(
            kept["description"]["improved"].as_str(),
            Some(DESCRIPTION),
            "{id}"
        );
        assert_eq!(item(&report, id)["proposal_created"], false, "{id}");
    }
    // The original the user went back to differs from the proposal: it waits.
    assert!(pending(&s, "original"));

    // Another description: approved automatically unless the skill is
    // risky, and a return to the original is never replaced automatically.
    let path = fake_claude(&s, "new");
    let before = git(&s, &["rev-parse", "HEAD"]);
    let (code, report) = run(&s, &path, &["classify", "--all", "--yes"]);
    assert_eq!(code, Some(0), "{report}");
    let head = git(&s, &["rev-parse", "HEAD"]);
    // The classification, then the approval alone.
    assert_eq!(
        git(&s, &["rev-list", "--count", &format!("{before}..HEAD")]),
        "2"
    );
    assert_eq!(
        git(&s, &["diff", "--name-only", "HEAD~1", "HEAD", "--", "meta"]),
        "meta/clean.toml\nmeta/scripted.toml"
    );
    let approved = approval(&report).expect("approval reported");
    assert_eq!(approved, (vec!["clean".into(), "scripted".into()], head));
    for id in ["clean", "scripted"] {
        let approved = meta(&s, id);
        assert_eq!(
            approved["description"]["improved"].as_str(),
            Some(NEW_DESCRIPTION),
            "{id}"
        );
        assert!(approved.get("proposal").is_none(), "{id}");
    }
    let risky = meta(&s, "risky");
    assert_eq!(risky["description"]["active"].as_str(), Some("improved"));
    assert_eq!(risky["description"]["improved"].as_str(), Some(DESCRIPTION));
    assert_eq!(
        risky["proposal"]["improved_description"].as_str(),
        Some(NEW_DESCRIPTION)
    );
    let original = meta(&s, "original");
    assert_eq!(original["description"]["active"].as_str(), Some("original"));
    assert_eq!(
        original["proposal"]["improved_description"].as_str(),
        Some(NEW_DESCRIPTION)
    );
}

#[test]
fn a_description_that_fails_the_checks_waits_and_the_rest_are_approved() {
    let s = Sandbox::new();
    s.initialize();
    import(&s, "clean", "# Body", &[]);
    import(&s, "odd", "# Body", &[]);
    let path = fake_claude(&s, "control");
    let (code, report) = run(&s, &path, &["classify", "--all", "--yes"]);
    assert_eq!(code, Some(0), "{report}");
    let (ids, commit) = approval(&report).expect("approval reported");
    assert_eq!(ids, ["clean"]);
    assert_eq!(commit, git(&s, &["rev-parse", "HEAD"]));
    assert!(pending(&s, "odd"));
}

#[test]
fn the_text_output_names_the_automatic_approval() {
    let s = Sandbox::new();
    s.initialize();
    import(&s, "clean", "# Body", &[]);
    let path = fake_claude(&s, "ok");
    let out = Command::new(env!("CARGO_BIN_EXE_kinana"))
        .current_dir(&s.root)
        .env("HOME", &s.home)
        .env("PATH", &path)
        .arg("--vault")
        .arg(&s.vault)
        .args(["classify", "clean"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let head = git(&s, &["rev-parse", "HEAD"]);
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains(&format!(
            "Approved automatically: the proposed description of 1 skill that is not risky, in commit {head}: clean. `kinana undo` reverts this approval only."
        )),
        "{stdout}"
    );
    assert!(!String::from_utf8_lossy(&out.stderr).contains("AUTO_APPROVED"));
}

#[test]
fn keywords_and_dry_runs_approve_nothing() {
    let s = Sandbox::new();
    s.initialize();
    import(&s, "clean", "# Body", &[]);
    let path = fake_claude(&s, "ok");
    let before = git(&s, &["rev-parse", "HEAD"]);
    let (code, report) = run(&s, &path, &["classify", "clean", "--dry-run"]);
    assert_eq!(code, Some(0), "{report}");
    assert!(approval(&report).is_none());
    assert_eq!(git(&s, &["rev-parse", "HEAD"]), before);
    let (code, report) = run(&s, &path, &["classify", "clean", "--agent", "keywords"]);
    assert_eq!(code, Some(0), "{report}");
    assert!(approval(&report).is_none());
    assert_eq!(
        meta(&s, "clean")["description"]["active"].as_str(),
        Some("original")
    );
}
