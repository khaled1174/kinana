#[allow(dead_code)]
mod support;

use serde_json::{Value, json};
use std::{fs, path::PathBuf, process::Command};
use support::{Sandbox, snapshot};

fn output(sandbox: &Sandbox, args: &[&str]) -> Value {
    let output = sandbox.run(args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn import(sandbox: &Sandbox, id: &str, long: bool) -> PathBuf {
    let source = sandbox.root.join(format!("input-{id}"));
    fs::create_dir(&source).unwrap();
    let body = if long {
        format!(
            "{}{}# Far heading\nFinal setext\n===\n",
            "Visible text\n".repeat(300),
            "TAIL_BODY_UNIQUE\n".repeat(1000)
        )
    } else {
        "Short body\n".into()
    };
    fs::write(
        source.join("SKILL.md"),
        format!("---\nname: {id}\ndescription: Review React components\n---\n{body}"),
    )
    .unwrap();
    output(sandbox, &["add", source.to_str().unwrap(), "--json"]);
    source
}

#[test]
fn batch_preview_separates_measured_fixed_cost_from_actual_selected_skill_size() {
    let sandbox = Sandbox::new();
    sandbox.initialize();
    let long = import(&sandbox, "long", true);
    import(&sandbox, "short", false);
    let before = snapshot(&sandbox.root);
    let preview = output(
        &sandbox,
        &[
            "classify",
            "--all",
            "--agent",
            "claude",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(preview["estimated_agent_calls"], 2);
    assert_eq!(preview["agent_calls"], 0);
    let estimate = &preview["estimated_tokens"];
    assert_eq!(estimate["calibration"]["total_input_tokens"], 5691);
    assert!(
        estimate["calibration"]["source_log"]
            .as_str()
            .unwrap()
            .contains("phase-3-repair/claude-model")
    );
    assert_eq!(
        estimate["fixed_tokens_total"].as_u64().unwrap().div_ceil(2),
        estimate["fixed_tokens_per_call"].as_u64().unwrap()
    );
    assert_eq!(
        estimate["total_input_tokens"].as_u64().unwrap(),
        estimate["fixed_tokens_total"].as_u64().unwrap()
            + estimate["skill_tokens_total"].as_u64().unwrap()
    );
    assert_eq!(
        estimate["retry_limit_input_tokens"].as_u64().unwrap(),
        2 * estimate["total_input_tokens"].as_u64().unwrap()
    );
    assert_eq!(preview["items"][0]["classified_from"], "excerpt");
    assert_eq!(preview["items"][1]["classified_from"], "full");
    let excerpt = output(
        &sandbox,
        &[
            "classify",
            "long",
            "--agent",
            "claude",
            "--dry-run",
            "--json",
        ],
    );
    let full = output(
        &sandbox,
        &[
            "classify",
            "long",
            "--agent",
            "claude",
            "--full",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(full["items"][0]["classified_from"], "full");
    assert_eq!(
        full["estimated_tokens"]["fixed_tokens_per_call"],
        excerpt["estimated_tokens"]["fixed_tokens_per_call"]
    );
    assert!(
        full["estimated_tokens"]["skill_tokens_total"]
            .as_u64()
            .unwrap()
            > excerpt["estimated_tokens"]["skill_tokens_total"]
                .as_u64()
                .unwrap()
                + 3000
    );
    let text = sandbox.run(&["classify", "--all", "--agent", "claude", "--dry-run"]);
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(
        text.contains("fixed/call")
            && text.contains("skill text")
            && text.contains("retry ceiling")
    );
    assert_eq!(snapshot(&sandbox.root), before);
    assert!(
        fs::read_to_string(long.join("SKILL.md"))
            .unwrap()
            .contains("TAIL_BODY_UNIQUE")
    );
    let offline = output(
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
    assert_eq!(offline["estimated_tokens"]["total_input_tokens"], 0);
}

#[test]
fn full_requires_one_explicit_id_and_never_changes_a_file_on_invalid_arguments() {
    let sandbox = Sandbox::new();
    sandbox.initialize();
    let before = snapshot(&sandbox.root);
    for args in [
        vec!["classify", "--all", "--full", "--json"],
        vec!["classify", "--pending", "--full", "--json"],
        vec!["classify", "one", "two", "--full", "--json"],
        vec!["classify", "--full", "--json"],
    ] {
        assert_eq!(sandbox.run(&args).status.code(), Some(1));
        assert_eq!(snapshot(&sandbox.root), before);
    }
}

#[cfg(unix)]
#[test]
fn accepted_provider_results_record_full_or_excerpt_and_keep_source_bytes_unchanged() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::new();
    sandbox.initialize();
    // These cover classification and the manual review, not r5's
    // automatic approval (crates/cli/tests/auto_approval.rs).
    support::manual_review(&sandbox.vault);
    let source = import(&sandbox, "long", true);
    let original = snapshot(&source);
    let bin = sandbox.root.join("bin");
    fs::create_dir(&bin).unwrap();
    let trace = sandbox.root.join("prompt.txt");
    let proposal = json!({"category":"web-frontend","category_reason":"Reviews browser components.","stack":["react"],"stage":["review"],"tags":["react"],"role":"Review UI","when_to_use":"Before UI changes","when_not":"Backend work","improved_description":"Review React component accessibility","overlaps_with":[],"confidence":0.9});
    let script = r#"#!/usr/bin/python3
import json, sys
if sys.argv[1:] == ['--help']:
    print(@HELP@)
elif sys.argv[1:] == ['--version']:
    print('2.1.319 (Claude Code)')
else:
    with open(@TRACE@, 'w') as file: file.write(sys.stdin.read())
    print(json.dumps({'type':'system','subtype':'init','tools':['StructuredOutput']}))
    print(json.dumps({'type':'result','subtype':'success','is_error':False,'terminal_reason':'completed','structured_output':@PROPOSAL@}))
"#.replace("@HELP@", &serde_json::to_string(include_str!("../../../tests/fixtures/classification/claude-help.txt")).unwrap())
        .replace("@TRACE@", &serde_json::to_string(&trace.to_string_lossy()).unwrap())
        .replace("@PROPOSAL@", &proposal.to_string());
    fs::write(bin.join("claude"), script).unwrap();
    fs::set_permissions(bin.join("claude"), fs::Permissions::from_mode(0o700)).unwrap();
    let path = std::env::join_paths(
        std::iter::once(bin).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    for (full, expected) in [(false, "excerpt"), (true, "full")] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_kinana"));
        command
            .env("HOME", &sandbox.home)
            .env("PATH", &path)
            .arg("--vault")
            .arg(&sandbox.vault)
            .args(["classify", "long", "--agent", "claude", "--json"]);
        if full {
            command.arg("--full");
        }
        let result = command.output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let report: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(report["items"][0]["classified_by"], "claude", "{report}");
        assert_eq!(report["items"][0]["classified_from"], expected);
        assert_eq!(report["items"][0]["proposal_created"], true);
        let metadata: toml::Value =
            toml::from_str(&fs::read_to_string(sandbox.vault.join("meta/long.toml")).unwrap())
                .unwrap();
        assert_eq!(
            metadata["classification"]["classified_from"].as_str(),
            Some(expected)
        );
        // The agent's dimensions are adopted with the category and kept for the
        // proposal, beside its table rather than in it (the 0.1.0 beta rejects
        // a key it does not know inside `[proposal]`).
        assert!(metadata["proposal"].get("stack").is_none());
        assert!(metadata["proposal"].get("stage").is_none());
        for table in ["classification", "proposal_dimensions"] {
            assert_eq!(
                metadata[table]["stack"].as_array().map(Vec::len),
                Some(1),
                "{table}"
            );
            assert_eq!(metadata[table]["stack"][0].as_str(), Some("react"));
            assert_eq!(metadata[table]["stage"][0].as_str(), Some("review"));
        }
        let prompt = fs::read_to_string(&trace).unwrap();
        assert!(prompt.contains("\"allowed_stacks\":[\"react\",\"next\","));
        assert!(prompt.contains("\"allowed_stages\":[\"plan\",\"design\","));
        assert!(prompt.contains("# Far heading") && prompt.contains("Final setext"));
        assert_eq!(prompt.contains("TAIL_BODY_UNIQUE"), full);
        assert_eq!(snapshot(&source), original);
        assert_eq!(snapshot(&sandbox.vault.join("skills/long")), original);
    }
}

#[cfg(unix)]
#[test]
fn reclassification_keeps_approved_description_and_dist_until_the_next_review() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::new();
    sandbox.initialize();
    // These cover classification and the manual review, not r5's
    // automatic approval (crates/cli/tests/auto_approval.rs).
    support::manual_review(&sandbox.vault);
    let source = import(&sandbox, "approved", false);
    let bin = sandbox.root.join("proposal-provider");
    fs::create_dir(&bin).unwrap();
    let response = sandbox.root.join("proposal.json");
    let script = r#"#!/usr/bin/python3
import json, sys
if sys.argv[1:] == ['--help']:
    print(@HELP@)
elif sys.argv[1:] == ['--version']:
    print('2.1.319 (Claude Code)')
else:
    sys.stdin.read()
    with open(@RESPONSE@) as file:
        proposal = json.load(file)
    print(json.dumps({'type':'system','subtype':'init','tools':['StructuredOutput']}))
    print(json.dumps({'type':'result','subtype':'success','is_error':False,'structured_output':proposal}))
"#.replace("@HELP@", &serde_json::to_string(include_str!("../../../tests/fixtures/classification/claude-help.txt")).unwrap())
        .replace("@RESPONSE@", &serde_json::to_string(&response.to_string_lossy()).unwrap());
    fs::write(bin.join("claude"), script).unwrap();
    fs::set_permissions(bin.join("claude"), fs::Permissions::from_mode(0o700)).unwrap();
    let path = std::env::join_paths(
        std::iter::once(bin).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let classify = || {
        let result = Command::new(env!("CARGO_BIN_EXE_kinana"))
            .env("HOME", &sandbox.home)
            .env("PATH", &path)
            .arg("--vault")
            .arg(&sandbox.vault)
            .args(["classify", "approved", "--agent", "claude", "--json"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let report: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(report["items"][0]["proposal_created"], true);
        assert_eq!(report["items"][0]["classified_by"], "claude");
        report
    };
    let mut proposal = json!({"category":"web-frontend","category_reason":"Reviews browser components.","stack":["react"],"stage":["review"],"tags":["react"],"role":"Review UI","when_to_use":"Before UI changes","when_not":"Backend work","improved_description":"Approved React accessibility description","overlaps_with":[],"confidence":0.9});
    fs::write(&response, proposal.to_string()).unwrap();
    classify();
    output(&sandbox, &["review", "approved", "--accept", "--json"]);
    let metadata_path = sandbox.vault.join("meta/approved.toml");
    let approved: toml::Value =
        toml::from_str(&fs::read_to_string(&metadata_path).unwrap()).unwrap();
    let original = snapshot(&source);
    let dist = snapshot(&sandbox.vault.join("dist/approved"));
    proposal["improved_description"] = json!("A fresh unapproved description awaiting review");
    fs::write(&response, proposal.to_string()).unwrap();
    let report = classify();
    let after: toml::Value = toml::from_str(&fs::read_to_string(&metadata_path).unwrap()).unwrap();
    assert_eq!(after["description"], approved["description"]);
    assert_eq!(after["description"]["active"].as_str(), Some("improved"));
    // Accepting the description kept the dimensions the agent proposed.
    assert_eq!(
        approved["classification"]["stack"][0].as_str(),
        Some("react")
    );
    assert_eq!(
        approved["classification"]["stage"][0].as_str(),
        Some("review")
    );
    assert!(approved.get("proposal").is_none());
    assert_eq!(
        after["proposal"]["improved_description"].as_str(),
        proposal["improved_description"].as_str()
    );
    assert_eq!(snapshot(&sandbox.vault.join("dist/approved")), dist);
    assert_eq!(snapshot(&source), original);
    assert_eq!(snapshot(&sandbox.vault.join("skills/approved")), original);
    assert_eq!(report["build"]["items"][0]["changed"], false);
    let preview = output(&sandbox, &["review", "approved", "--json"]);
    assert_eq!(preview["active"], "improved");
    assert_eq!(
        preview["current"],
        "Approved React accessibility description"
    );
    assert_eq!(
        preview["proposed"],
        "A fresh unapproved description awaiting review"
    );
}

#[cfg(unix)]
#[test]
/// 0.3 (owner, 2026-11-18): Claude requested by name never falls back to
/// keywords. A failed model call stops the run with a fixed code and exit 2,
/// and nothing is written.
fn model_failure_with_claude_named_stops_without_keywords_in_text_and_json_modes() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::new();
    sandbox.initialize();
    import(&sandbox, "warning-fixture", false);
    let bin = sandbox.root.join("failed-model-bin");
    fs::create_dir(&bin).unwrap();
    let script = r#"#!/usr/bin/python3
import sys
if sys.argv[1:] == ['--help']:
    print(@HELP@)
elif sys.argv[1:] == ['--version']:
    print('2.1.319 (Claude Code)')
else:
    sys.stdin.read()
    sys.exit(17)
"#
    .replace(
        "@HELP@",
        &serde_json::to_string(include_str!(
            "../../../tests/fixtures/classification/claude-help.txt"
        ))
        .unwrap(),
    );
    fs::write(bin.join("claude"), script).unwrap();
    fs::set_permissions(bin.join("claude"), fs::Permissions::from_mode(0o700)).unwrap();
    let path = std::env::join_paths(
        std::iter::once(bin).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let meta = sandbox.vault.join("meta/warning-fixture.toml");
    let before = fs::read(&meta).unwrap();
    for json in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_kinana"));
        command
            .env("HOME", &sandbox.home)
            .env("PATH", &path)
            .arg("--vault")
            .arg(&sandbox.vault)
            .args(["classify", "warning-fixture", "--agent", "claude"]);
        if json {
            command.arg("--json");
        }
        let output = command.output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        let warning = String::from_utf8(output.stderr).unwrap();
        assert!(
            warning.contains("Classification stopped (AGENT_CLAUDE_UNAVAILABLE)"),
            "{warning}"
        );
        assert!(warning.contains("(claude: AGENT_"), "{warning}");
        assert!(!warning.contains("keyword fallback"), "{warning}");
        if json {
            let report: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(report["items"], serde_json::json!([]));
            assert_eq!(report["agent_calls"], 1);
            assert_eq!(report["committed"], false);
            assert_eq!(report["stopped"]["reason_code"], "AGENT_CLAUDE_UNAVAILABLE");
            assert_eq!(
                report["stopped"]["remaining"],
                serde_json::json!(["warning-fixture"])
            );
        }
        assert_eq!(fs::read(&meta).unwrap(), before, "nothing was written");
    }
}

/// Review 2026-11-17 (sixth round): a dirty vault made two model calls before
/// the commit refused it. It is now refused before any call.
#[test]
fn a_dirty_vault_is_refused_before_any_model_call() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::new();
    sandbox.initialize();
    import(&sandbox, "dirty", false);
    fs::write(sandbox.vault.join("notes.txt"), "the user's own file\n").unwrap();
    let bin = sandbox.root.join("bin");
    fs::create_dir(&bin).unwrap();
    let calls = sandbox.root.join("calls.txt");
    let script = r#"#!/usr/bin/python3
import sys
if sys.argv[1:] == ['--help']:
    print(@HELP@)
elif sys.argv[1:] == ['--version']:
    print('2.1.319 (Claude Code)')
else:
    with open(@CALLS@, 'a') as file: file.write('call\n')
"#
    .replace(
        "@HELP@",
        &serde_json::to_string(include_str!(
            "../../../tests/fixtures/classification/claude-help.txt"
        ))
        .unwrap(),
    )
    .replace(
        "@CALLS@",
        &serde_json::to_string(&calls.to_string_lossy()).unwrap(),
    );
    fs::write(bin.join("claude"), script).unwrap();
    fs::set_permissions(bin.join("claude"), fs::Permissions::from_mode(0o700)).unwrap();
    let path = std::env::join_paths(
        std::iter::once(bin).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_kinana"))
        .env("HOME", &sandbox.home)
        .env("PATH", &path)
        .arg("--vault")
        .arg(&sandbox.vault)
        .args(["classify", "dirty", "--agent", "claude", "--json"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    let value: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(value["error"]["code"], "GIT_DIRTY_VAULT", "{value}");
    assert!(!calls.exists(), "a model call was made");
}

/// Owner, round 8: a Claude usage limit stops "classify all" with a clear
/// error instead of a silent keyword fallback. Skills completed before it are
/// saved as proposals in one commit; the skill that hit the limit and every
/// later one stay as they were, and every model call is counted exactly.
#[cfg(unix)]
#[test]
fn a_usage_limit_saves_completed_skills_in_one_commit_and_stops_the_rest() {
    limit_batch(false);
}

#[cfg(unix)]
#[test]
fn credit_before_init_saves_only_completed_skills() {
    limit_batch(true);
}

#[cfg(unix)]
fn limit_batch(before_init: bool) {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::new();
    sandbox.initialize();
    // These cover classification and the manual review, not r5's
    // automatic approval (crates/cli/tests/auto_approval.rs).
    support::manual_review(&sandbox.vault);
    for id in ["a-first", "b-second", "c-third"] {
        import(&sandbox, id, false);
    }
    let bin = sandbox.root.join("limited-provider");
    fs::create_dir(&bin).unwrap();
    let calls = sandbox.root.join("calls.txt");
    let proposal = json!({"category":"web-frontend","category_reason":"Reviews browser components.","stack":["react"],"stage":["review"],"tags":["react"],"role":"Review UI","when_to_use":"Before UI changes","when_not":"Backend work","improved_description":"Review React component accessibility","overlaps_with":[],"confidence":0.9});
    let script = r#"#!/usr/bin/python3
import json, sys
if sys.argv[1:] == ['--help']:
    print(@HELP@)
elif sys.argv[1:] == ['--version']:
    print('2.1.319 (Claude Code)')
else:
    sys.stdin.read()
    with open(@CALLS@, 'a') as file: file.write('call\n')
    with open(@CALLS@) as file: count = len(file.readlines())
    if count > 1 and @EARLY@:
        print(json.dumps({'type':'result','is_error':True,'result':'Credit balance is too low'}))
        sys.exit(1)
    print(json.dumps({'type':'system','subtype':'init','tools':['StructuredOutput']}))
    if count == 1:
        print(json.dumps({'type':'result','subtype':'success','is_error':False,'structured_output':@PROPOSAL@}))
    else:
        print(json.dumps({'type':'assistant','error':'rate_limit','message':{'content':[{'type':'text','text':"You've hit your session limit"}]}}))
        print(json.dumps({'type':'result','subtype':'success','is_error':True,'result':"You've hit your session limit · resets 5pm (Asia/Riyadh)"}))
        sys.exit(1)
"#
    .replace(
        "@HELP@",
        &serde_json::to_string(include_str!("../../../tests/fixtures/classification/claude-help.txt"))
            .unwrap(),
    )
    .replace("@EARLY@", if before_init {"True"} else {"False"})
    .replace("@CALLS@", &serde_json::to_string(&calls.to_string_lossy()).unwrap())
    .replace("@PROPOSAL@", &proposal.to_string());
    fs::write(bin.join("claude"), script).unwrap();
    fs::set_permissions(bin.join("claude"), fs::Permissions::from_mode(0o700)).unwrap();
    let path = std::env::join_paths(
        std::iter::once(bin).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(&sandbox.vault)
            .args(args)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    };
    let commits_before = git(&["rev-list", "--count", "HEAD"]);
    let untouched: Vec<_> = ["b-second", "c-third"]
        .iter()
        .map(|id| fs::read(sandbox.vault.join(format!("meta/{id}.toml"))).unwrap())
        .collect();
    let result = Command::new(env!("CARGO_BIN_EXE_kinana"))
        .env("HOME", &sandbox.home)
        .env("PATH", &path)
        .arg("--vault")
        .arg(&sandbox.vault)
        .args(["classify", "--all", "--agent", "claude", "--yes", "--json"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);
    // A named agent's stop asks the user what next: exit 2 (owner, 2026-11-18).
    assert_eq!(result.status.code(), Some(2), "{stderr}");
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["stopped"]["providers"][0]["status"], "failed");
    // One accepted call, then one call that hit the limit; nothing after it.
    assert_eq!(fs::read_to_string(&calls).unwrap().lines().count(), 2);
    assert_eq!(report["agent_calls"], 2, "{report}");
    let stopped = &report["stopped"];
    let expected = if before_init {
        "AGENT_CREDIT_EXHAUSTED"
    } else {
        "AGENT_USAGE_LIMIT"
    };
    assert_eq!(stopped["reason_code"], expected, "{report}");
    assert_eq!(stopped["classified"], 1);
    assert_eq!(stopped["remaining"], json!(["b-second", "c-third"]));
    assert!(
        before_init
            || stopped["message"]
                .as_str()
                .unwrap()
                .contains("(resets 5pm (asia/riyadh))")
    );
    assert!(stderr.contains(expected), "{stderr}");
    assert!(!stderr.contains("keyword fallback"), "{stderr}");
    assert_eq!(report["committed"], true);
    assert_eq!(report["items"].as_array().unwrap().len(), 1);
    assert_eq!(report["items"][0]["id"], "a-first");
    assert_eq!(report["items"][0]["proposal_created"], true);
    // Exactly one new commit, holding only the completed skill.
    assert_eq!(
        git(&["rev-list", "--count", "HEAD"])
            .trim()
            .parse::<u32>()
            .unwrap(),
        commits_before.trim().parse::<u32>().unwrap() + 1
    );
    let changed = git(&["show", "--name-only", "--format=", "HEAD"]);
    assert!(changed.contains("meta/a-first.toml"), "{changed}");
    assert!(
        !changed.contains("b-second") && !changed.contains("c-third"),
        "{changed}"
    );
    let first: toml::Value =
        toml::from_str(&fs::read_to_string(sandbox.vault.join("meta/a-first.toml")).unwrap())
            .unwrap();
    assert_eq!(
        first["classification"]["classified_by"].as_str(),
        Some("claude")
    );
    assert!(first.get("proposal").is_some());
    for (id, before) in ["b-second", "c-third"].iter().zip(untouched) {
        assert_eq!(
            fs::read(sandbox.vault.join(format!("meta/{id}.toml"))).unwrap(),
            before,
            "{id}"
        );
    }
    assert_eq!(git(&["status", "--porcelain"]), "");
}
