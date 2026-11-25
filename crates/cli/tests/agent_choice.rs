//! 0.3, batch 3: Codex refused with its reason, and `auto` (the default) that
//! stops and asks instead of saving keywords. Only fake agents are started.
//!
//! Owner decision 2026-11-18, one test per case:
//! (a) `auto`, Claude not on this computer: stop with the reason shown,
//!     or ask for confirmation if Pi is enabled.
//! (b) `auto`, Claude found but failing: Pi if enabled (pi_gate.rs), else a
//!     stop with `AGENT_AUTO_UNAVAILABLE`; completed skills stay saved.
//! (c) a named agent never falls back to keywords: `--agent claude` stops
//!     with `AGENT_CLAUDE_UNAVAILABLE` (Pi: pi_gate.rs).
//! (d) keywords only when chosen explicitly.
#[allow(dead_code)]
mod support;
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};
use support::{Sandbox, snapshot};

fn import(s: &Sandbox, id: &str) {
    let source = s.root.join(format!("source-{id}"));
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        format!("---\nname: {id}\ndescription: Review React components\n---\n# Body\n"),
    )
    .unwrap();
    let out = s.run(&["add", source.to_str().unwrap(), "--json"]);
    assert!(out.status.success());
}

/// A fake `claude` on a private PATH. `mode`: "broken" fails every call
/// (capability unproven, no model call); "ok" returns a valid proposal;
/// "fail-b" answers for every skill except the one named `b`; "limit-b",
/// "credit-b" and "login-b" stop at `b` with a usage limit, no credit, or
/// no sign-in.
#[cfg(unix)]
fn fake_claude(s: &Sandbox, mode: &str) -> std::ffi::OsString {
    use std::os::unix::fs::PermissionsExt;
    let bin = s.root.join(format!("bin-{mode}"));
    fs::create_dir(&bin).unwrap();
    let trace = s.root.join(format!("calls-{mode}.txt"));
    let proposal = json!({"category":"web-frontend","category_reason":"Reviews browser components.","stack":["react"],"stage":["review"],"tags":["react"],"role":"Review UI","when_to_use":"Before UI changes","when_not":"Backend work","improved_description":"Review React component accessibility","overlaps_with":[],"confidence":0.9});
    let script = r#"#!/usr/bin/python3
import json, sys
with open(@TRACE@, 'a') as file: file.write(' '.join(sys.argv[1:2]) + '\n')
if @MODE@ == 'broken':
    sys.exit(1)
if sys.argv[1:] == ['--help']:
    print(@HELP@)
elif sys.argv[1:] == ['--version']:
    print('2.1.319 (Claude Code)')
else:
    text = sys.stdin.read()
    if @MODE@ in ('model-ok', 'model-unisolated'):
        assert sys.argv[sys.argv.index('--model') + 1] == 'haiku'
        assert sys.argv[sys.argv.index('--effort') + 1] == 'low'
    if @MODE@ == 'fail-b' and 'name: b\n' in text:
        sys.exit(1)
    if @MODE@ == 'vanish-after-a' and 'name: a\n' in text:
        import os
        os.remove(__file__)
    stops = {'limit-b': "You've hit your limit · resets 5pm (UTC)", 'credit-b': 'Credit balance is too low', 'login-b': 'Not logged in · Please run /login'}
    if @MODE@ in stops and 'name: b\n' in text:
        print(json.dumps({'type':'system','subtype':'init','tools':['StructuredOutput']}))
        print(json.dumps({'type':'result','subtype':'success','is_error':True,'result':stops[@MODE@]}))
        sys.exit(1)
    tools = ['StructuredOutput', 'Read'] if @MODE@ == 'model-unisolated' else ['StructuredOutput']
    print(json.dumps({'type':'system','subtype':'init','tools':tools,'model':'claude-fixture'}))
    if @MODE@ == 'refused':
        message = {'role':'assistant','stop_reason':'refusal','content':[{'type':'text','text':'private-fixture-text'}]}
        print(json.dumps({'type':'assistant','message':message}))
        print(json.dumps({'type':'result','subtype':'error_during_execution','is_error':True}))
        sys.exit(1)
    if @MODE@ == 'invalid-b' and 'name: b\n' in text:
        call = {'type':'tool_use','name':'StructuredOutput','id':'fixture-call','input':{'__unparsedToolInput':'fixture'}}
        print(json.dumps({'type':'assistant','message':{'content':[call]}}))
    print(json.dumps({'type':'result','subtype':'success','is_error':False,'terminal_reason':'completed','structured_output':@PROPOSAL@}))
"#
    .replace("@MODE@", &serde_json::to_string(mode).unwrap())
    .replace(
        "@HELP@",
        &serde_json::to_string(include_str!("../../../tests/fixtures/classification/claude-help.txt"))
            .unwrap(),
    )
    .replace("@TRACE@", &serde_json::to_string(&trace.to_string_lossy()).unwrap())
    .replace("@PROPOSAL@", &proposal.to_string());
    fs::write(bin.join("claude"), script).unwrap();
    fs::set_permissions(bin.join("claude"), fs::Permissions::from_mode(0o700)).unwrap();
    std::env::join_paths([bin.as_path(), Path::new("/usr/bin"), Path::new("/bin")]).unwrap()
}

fn run_with(s: &Sandbox, path: &std::ffi::OsStr, args: &[&str]) -> (Option<i32>, Value) {
    run_in(s, path, &s.root, args)
}

#[cfg(unix)]
#[test]
fn a_claude_model_and_effort_override_keep_the_same_init_gate() {
    for (mode, exit) in [("model-ok", 0), ("model-unisolated", 2)] {
        let s = Sandbox::new();
        s.initialize();
        import(&s, "ui");
        let path = fake_claude(&s, mode);
        let (code, report) = run_with(
            &s,
            &path,
            &[
                "classify", "ui", "--agent", "claude", "--model", "haiku", "--effort", "low",
            ],
        );
        assert_eq!(code, Some(exit), "{report}");
        assert_eq!(
            report["estimated_tokens"]["claude"],
            json!({"model":"haiku","effort":"low"})
        );
        assert_eq!(report["agent_calls"], 1);
        if exit == 0 {
            assert!(report.get("stopped").is_none());
            assert_eq!(report["items"][0]["classified_by"], "claude");
        } else {
            assert_eq!(
                report["stopped"]["providers"][0]["reason_code"],
                "AGENT_TOOLS_NOT_ISOLATED"
            );
            assert_eq!(report["committed"], false);
        }
    }
}

#[cfg(unix)]
#[test]
fn claude_reports_structured_refusal_without_retry_or_raw_text() {
    let s = Sandbox::new();
    s.initialize();
    import(&s, "ui");
    let path = fake_claude(&s, "refused");
    let (exit, report) = run_with(
        &s,
        &path,
        &["classify", "ui", "--agent", "claude", "--claude-events"],
    );
    assert_eq!(exit, Some(2));
    assert_eq!(report["agent_calls"], 1);
    assert_eq!(report["committed"], false);
    assert!(report.get("stopped").is_none());
    assert_eq!(report["skipped"][0]["reason_code"], "AGENT_REFUSED");
    assert_eq!(
        report["claude_events"][0],
        json!({
            "id":"ui","attempt":1,"model":"claude-fixture","stop_reason":"refusal",
            "event_type":"assistant.message","refused":true,"unparsed_tool_input":false
        })
    );
    assert!(!report.to_string().contains("private-fixture-text"));
}

fn run_in(s: &Sandbox, path: &std::ffi::OsStr, cwd: &Path, args: &[&str]) -> (Option<i32>, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_kinana"))
        .current_dir(cwd)
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

#[test]
fn codex_is_refused_with_its_reason_and_writes_nothing() {
    let s = Sandbox::new();
    s.initialize();
    import(&s, "ui");
    let before = snapshot(&s.root);
    for extra in [&[][..], &["--dry-run"], &["--all", "--yes"]] {
        let mut args = vec!["classify", "--agent", "codex"];
        if !extra.contains(&"--all") {
            args.insert(1, "ui");
        }
        args.extend_from_slice(extra);
        args.push("--json");
        let out = s.run(&args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        let value: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(value["error"]["code"], "AGENT_CODEX_UNSUPPORTED");
        assert!(
            value["error"]["message"]
                .as_str()
                .unwrap()
                .contains("cannot guarantee that it does not read files")
        );
        assert_eq!(snapshot(&s.root), before);
    }
}

#[cfg(unix)]
#[test]
fn auto_stops_and_asks_when_claude_cannot_classify_and_saves_nothing_for_that_skill() {
    let s = Sandbox::new();
    s.initialize();
    import(&s, "ui");
    let path = fake_claude(&s, "broken");
    let before = snapshot(&s.vault);

    // The preview lists the providers in order; nothing is started.
    let (code, preview) = run_with(&s, &path, &["classify", "ui", "--dry-run"]);
    assert_eq!(code, Some(0));
    assert_eq!(preview["estimated_agent_calls"], 1);
    assert_eq!(
        preview["providers"],
        json!([
            {"agent":"claude","status":"planned","planned_calls":1},
            {"agent":"pi","status":"disabled","reason_code":"AGENT_PI_DISABLED","planned_calls":0},
            {"agent":"codex","status":"unsupported","reason_code":"AGENT_CODEX_UNSUPPORTED","planned_calls":0}
        ])
    );
    assert!(!s.root.join("calls-broken.txt").exists());
    // The preview shows the current classification, never a keyword guess.
    assert_eq!(preview["items"][0]["classified_by"], "pending");
    assert_eq!(preview["items"][0]["category"], "other");

    // `auto` is the default: Claude fails its probe, so the run stops and
    // asks. No keyword result is saved and no model call was made.
    let (code, report) = run_with(&s, &path, &["classify", "ui"]);
    assert_eq!(code, Some(2), "{report}");
    assert_eq!(report["agent_calls"], 0);
    assert_eq!(report["committed"], false);
    assert!(report["items"].as_array().unwrap().is_empty());
    let stopped = &report["stopped"];
    assert_eq!(stopped["reason_code"], "AGENT_AUTO_UNAVAILABLE");
    assert!(
        stopped["message"]
            .as_str()
            .unwrap()
            .contains("classify the remaining skills")
    );
    assert_eq!(stopped["classified"], 0);
    assert_eq!(stopped["remaining"], json!(["ui"]));
    assert_eq!(stopped["providers"][0]["agent"], "claude");
    assert_eq!(stopped["providers"][0]["status"], "failed");
    assert_eq!(
        stopped["providers"][0]["reason_code"],
        "AGENT_CAPABILITY_UNPROVEN"
    );
    assert_eq!(stopped["providers"][1]["status"], "disabled");
    assert_eq!(stopped["providers"][2]["status"], "unsupported");
    assert_eq!(snapshot(&s.vault), before);
    // Only the capability probe ran; never a prompt.
    let calls = fs::read_to_string(s.root.join("calls-broken.txt")).unwrap();
    assert!(
        calls
            .lines()
            .all(|line| line == "--help" || line == "--version"),
        "{calls}"
    );

    // An explicit keyword run remains the user's choice.
    let (code, keywords) = run_with(&s, &path, &["classify", "ui", "--agent", "keywords"]);
    assert_eq!(code, Some(0));
    assert_eq!(keywords["items"][0]["classified_by"], "keywords");
}

#[cfg(unix)]
#[test]
fn auto_uses_claude_when_it_works() {
    let s = Sandbox::new();
    s.initialize();
    import(&s, "ui");
    let path = fake_claude(&s, "ok");
    let (code, report) = run_with(&s, &path, &["classify", "ui"]);
    assert_eq!(code, Some(0), "{report}");
    assert_eq!(report["agent_calls"], 1);
    assert_eq!(report["items"][0]["classified_by"], "claude");
    assert!(report.get("stopped").is_none());
}

/// Without Claude or a confirmed Pi, `auto` stops and reports the absence.
#[cfg(unix)]
#[test]
fn auto_without_claude_on_this_computer_stops_with_a_visible_reason() {
    use std::os::unix::fs::PermissionsExt;
    let s = Sandbox::new();
    s.initialize();
    import(&s, "ui");
    // A `pi` that records any start: it must not be started.
    let bin = s.root.join("bin-pi-only");
    fs::create_dir(&bin).unwrap();
    let trace = s.root.join("pi-started.txt");
    fs::write(
        bin.join("pi"),
        format!("#!/bin/sh\necho started >> '{}'\nexit 1\n", trace.display()),
    )
    .unwrap();
    fs::set_permissions(bin.join("pi"), fs::Permissions::from_mode(0o700)).unwrap();
    let path =
        std::env::join_paths([bin.as_path(), Path::new("/usr/bin"), Path::new("/bin")]).unwrap();
    let (code, report) = run_with(&s, &path, &["classify", "ui"]);
    assert_eq!(code, Some(2), "{report}");
    assert!(report["items"].as_array().unwrap().is_empty());
    assert_eq!(report["stopped"]["reason_code"], "AGENT_AUTO_UNAVAILABLE");
    assert_eq!(
        report["stopped"]["providers"][0]["reason_code"],
        "AGENT_EXECUTABLE_NOT_FOUND"
    );
    assert_eq!(report["agent_calls"], 0);
    assert_eq!(report["committed"], false);
    assert!(!trace.exists(), "Pi was started");
    import(&s, "zz");
    let (code, both) = run_with(&s, &path, &["classify", "--all", "--yes"]);
    assert_eq!(code, Some(2), "{both}");
    assert!(both["items"].as_array().unwrap().is_empty(), "{both}");
    assert_eq!(both["stopped"]["remaining"], json!(["ui", "zz"]));
    // The text mode names the reason on stderr.
    let out = Command::new(env!("CARGO_BIN_EXE_kinana"))
        .env("HOME", &s.home)
        .env("PATH", &path)
        .arg("--vault")
        .arg(&s.vault)
        .args(["classify", "ui"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("AGENT_EXECUTABLE_NOT_FOUND"), "{stderr}");

    // Pi enabled: no keywords; the run asks to use Pi before starting it.
    let config = s.vault.join("box.toml");
    let mut text = fs::read_to_string(&config).unwrap();
    text.push_str(
        "\n[classification_pi]\nenabled = true\nprovider = \"test-provider\"\nmodel = \"test-model\"\n",
    );
    fs::write(&config, text).unwrap();
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&s.vault)
            .args(["commit", "-qam", "Enable Pi"])
            .status()
            .unwrap()
            .success()
    );
    let before = snapshot(&s.vault);
    let (code, report) = run_with(&s, &path, &["classify", "ui"]);
    assert_eq!(code, Some(2), "{report}");
    assert!(report.get("fallback_reason").is_none(), "{report}");
    assert!(report["items"].as_array().unwrap().is_empty(), "{report}");
    let stopped = &report["stopped"];
    assert_eq!(stopped["reason_code"], "PI_CONFIRMATION_REQUIRED");
    assert_eq!(stopped["providers"][0]["status"], "failed");
    assert_eq!(
        stopped["providers"][0]["reason_code"],
        "AGENT_EXECUTABLE_NOT_FOUND"
    );
    assert_eq!(stopped["providers"][1]["agent"], "pi");
    assert_eq!(report["pi_confirmation"]["provider"], "test-provider");
    assert_eq!(report["agent_calls"], 0);
    assert!(!trace.exists(), "Pi was started before the confirmation");
    assert_eq!(snapshot(&s.vault), before, "nothing is written");
}

/// Case (c): Claude named explicitly never becomes keywords, whether it is
/// missing or failing. Completed skills are saved; the rest are untouched.
#[cfg(unix)]
#[test]
fn claude_named_stops_instead_of_keywords_and_keeps_completed_skills() {
    let s = Sandbox::new();
    s.initialize();
    import(&s, "a");
    import(&s, "b");
    let before = snapshot(&s.vault);
    // Missing: a fixed code, exit 2, nothing written.
    let (code, report) = run_with(
        &s,
        std::ffi::OsStr::new("/usr/bin:/bin"),
        &["classify", "a", "--agent", "claude"],
    );
    assert_eq!(code, Some(2), "{report}");
    assert_eq!(report["stopped"]["reason_code"], "AGENT_CLAUDE_UNAVAILABLE");
    assert_eq!(
        report["stopped"]["providers"][0]["reason_code"],
        "AGENT_EXECUTABLE_NOT_FOUND"
    );
    assert!(report.get("fallback_reason").is_none());
    assert_eq!(snapshot(&s.vault), before);

    // Failing on the second skill: the first is saved, the second is not.
    let path = fake_claude(&s, "fail-b");
    let b_before = fs::read(s.vault.join("meta/b.toml")).unwrap();
    let (code, report) = run_with(
        &s,
        &path,
        &["classify", "a", "b", "--agent", "claude", "--yes"],
    );
    assert_eq!(code, Some(2), "{report}");
    assert_eq!(report["items"].as_array().unwrap().len(), 1);
    assert_eq!(report["items"][0]["id"], "a");
    assert_eq!(report["items"][0]["classified_by"], "claude");
    assert_eq!(report["committed"], true);
    let stopped = &report["stopped"];
    assert_eq!(stopped["reason_code"], "AGENT_CLAUDE_UNAVAILABLE");
    assert_eq!(stopped["classified"], 1);
    assert_eq!(stopped["remaining"], json!(["b"]));
    assert_eq!(stopped["providers"][0]["status"], "failed");
    assert_eq!(fs::read(s.vault.join("meta/b.toml")).unwrap(), b_before);
}

/// Case (b) without Pi, and (d): `auto` stops at the first skill Claude
/// cannot classify; the skills before it are saved, keywords are never used.
#[cfg(unix)]
#[test]
fn auto_keeps_completed_skills_and_stops_at_the_first_failure() {
    let s = Sandbox::new();
    s.initialize();
    import(&s, "a");
    import(&s, "b");
    let path = fake_claude(&s, "fail-b");
    let b_before = fs::read(s.vault.join("meta/b.toml")).unwrap();
    let (code, report) = run_with(&s, &path, &["classify", "--all", "--yes"]);
    assert_eq!(code, Some(2), "{report}");
    assert_eq!(report["items"][0]["id"], "a");
    assert_eq!(report["items"][0]["classified_by"], "claude");
    assert_eq!(report["stopped"]["reason_code"], "AGENT_AUTO_UNAVAILABLE");
    assert_eq!(report["stopped"]["remaining"], json!(["b"]));
    assert!(report.get("fallback_reason").is_none());
    assert_eq!(fs::read(s.vault.join("meta/b.toml")).unwrap(), b_before);
    // (d): the explicit choice is the only way to keywords for `b`.
    let (code, keywords) = run_with(&s, &path, &["classify", "b", "--agent", "keywords"]);
    assert_eq!(code, Some(0));
    assert_eq!(keywords["items"][0]["classified_by"], "keywords");
    assert!(keywords.get("fallback_reason").is_none());
}

/// Review of 8d7a5a4 (M1): a claude that exists but fails the trust rule is
/// present, not absent. `auto` stops instead of saving keywords.
#[cfg(unix)]
#[test]
fn auto_with_an_untrusted_claude_stops_instead_of_using_keywords() {
    use std::os::unix::fs::PermissionsExt;
    let s = Sandbox::new();
    s.initialize();
    import(&s, "ui");
    // The skill's own source folder: excluded by the trust rule.
    let source = s.root.join("source-ui");
    fs::write(source.join("claude"), "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(source.join("claude"), fs::Permissions::from_mode(0o700)).unwrap();
    let path =
        std::env::join_paths([source.as_path(), Path::new("/usr/bin"), Path::new("/bin")]).unwrap();
    let before = snapshot(&s.vault);
    let (code, report) = run_with(&s, &path, &["classify", "ui"]);
    assert_eq!(code, Some(2), "{report}");
    assert!(report.get("fallback_reason").is_none(), "{report}");
    assert_eq!(report["stopped"]["reason_code"], "AGENT_AUTO_UNAVAILABLE");
    assert_eq!(
        report["stopped"]["providers"][0]["reason_code"],
        "AGENT_EXECUTABLE_UNTRUSTED"
    );
    assert_eq!(snapshot(&s.vault), before);
}

/// Owner decision, 2026-11-18 (item 2): a `claude` reached only through a
/// relative `PATH` entry is untrusted, never "not on this computer": `auto`
/// stops with the untrusted code, and it is never started.
#[cfg(unix)]
#[test]
fn a_claude_behind_a_relative_path_entry_is_untrusted_not_absent() {
    use std::os::unix::fs::PermissionsExt;
    let s = Sandbox::new();
    s.initialize();
    import(&s, "ui");
    let cwd = s.root.join("cwd");
    let trace = s.root.join("relative-claude-started.txt");
    let fake = format!("#!/bin/sh\necho started >> '{}'\nexit 1\n", trace.display());
    for directory in [cwd.join("rel"), cwd.clone()] {
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("claude"), &fake).unwrap();
        fs::set_permissions(directory.join("claude"), fs::Permissions::from_mode(0o700)).unwrap();
    }
    let before = snapshot(&s.vault);
    // `rel` and an empty entry (the current directory, as for a shell).
    for path in ["rel:/usr/bin:/bin", ":/usr/bin:/bin", ".:/usr/bin:/bin"] {
        let (code, report) = run_in(&s, std::ffi::OsStr::new(path), &cwd, &["classify", "ui"]);
        assert_eq!(code, Some(2), "{path}: {report}");
        assert!(report.get("fallback_reason").is_none(), "{path}: {report}");
        assert_eq!(report["stopped"]["reason_code"], "AGENT_AUTO_UNAVAILABLE");
        assert_eq!(
            report["stopped"]["providers"][0]["reason_code"], "AGENT_EXECUTABLE_UNTRUSTED",
            "{path}"
        );
        let message = report["stopped"]["message"].as_str().unwrap();
        assert!(message.contains("relative PATH entry"), "{path}: {message}");
        let (code, named) = run_in(
            &s,
            std::ffi::OsStr::new(path),
            &cwd,
            &["classify", "ui", "--agent", "claude"],
        );
        assert_eq!(code, Some(2), "{path}: {named}");
        assert_eq!(
            named["stopped"]["providers"][0]["reason_code"],
            "AGENT_EXECUTABLE_UNTRUSTED"
        );
    }
    // The same program behind no PATH entry at all is simply absent.
    let (code, report) = run_in(
        &s,
        std::ffi::OsStr::new("/usr/bin:/bin"),
        &cwd,
        &["classify", "ui"],
    );
    assert_eq!(code, Some(2), "{report}");
    assert_eq!(
        report["stopped"]["providers"][0]["reason_code"],
        "AGENT_EXECUTABLE_NOT_FOUND"
    );
    assert!(!trace.exists(), "a relative claude was started");
    assert_eq!(snapshot(&s.vault), before);
}

#[cfg(unix)]
#[test]
fn invalid_structured_reply_skips_only_that_skill_after_one_retry() {
    for agent in ["claude", "auto"] {
        let s = Sandbox::new();
        s.initialize();
        for id in ["a", "b", "c"] {
            import(&s, id);
        }
        let head = kinana_core::gitlog::head(&s.vault).unwrap();
        let original = fs::read(s.vault.join("meta/b.toml")).unwrap();
        let path = fake_claude(&s, "invalid-b");
        let (exit, report) = run_with(&s, &path, &["classify", "--all", "--yes", "--agent", agent]);
        assert_eq!(exit, Some(2), "{report}");
        assert!(report.get("stopped").is_none(), "{report}");
        assert_eq!(report["agent_calls"], 4);
        assert_eq!(report["committed"], true);
        assert_eq!(
            report["skipped"],
            json!([{"id":"b","agent":"claude","reason_code":"INVALID_AGENT_PROPOSAL","attempts":2}])
        );
        let items = report["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        for (item, id) in items.iter().zip(["a", "c"]) {
            assert_eq!(item["id"], id);
            assert_eq!(item["classified_by"], "claude");
            assert_eq!(item["proposal_created"], true);
        }
        assert_eq!(fs::read(s.vault.join("meta/b.toml")).unwrap(), original);
        // One classification commit, then (owner 2026-11-24, r5/r6) the
        // automatic approval of the descriptions on its own (contract 11).
        assert_eq!(report["auto_approved"]["ids"], json!(["a", "c"]), "{report}");
        let parent = Command::new("git")
            .arg("-C")
            .arg(&s.vault)
            .args(["rev-parse", "HEAD~2"])
            .output()
            .unwrap();
        assert!(parent.status.success());
        assert_eq!(String::from_utf8(parent.stdout).unwrap().trim(), head);
    }
}

/// Owner decision, 2026-11-18 (item 3; was pending L2): a named agent that
/// reaches a usage limit, runs out of credit or is signed out stops with
/// that code, exit 2, and the window's choice; completed skills are saved.
#[cfg(unix)]
#[test]
fn a_named_claude_at_a_limit_stops_with_exit_2_and_keeps_completed_skills() {
    for (mode, code) in [
        ("limit-b", "AGENT_USAGE_LIMIT"),
        ("credit-b", "AGENT_CREDIT_EXHAUSTED"),
        ("login-b", "AGENT_NOT_LOGGED_IN"),
    ] {
        let s = Sandbox::new();
        s.initialize();
        import(&s, "a");
        import(&s, "b");
        import(&s, "c");
        let path = fake_claude(&s, mode);
        let b_before = fs::read(s.vault.join("meta/b.toml")).unwrap();
        let c_before = fs::read(s.vault.join("meta/c.toml")).unwrap();
        let (exit, report) = run_with(
            &s,
            &path,
            &["classify", "--all", "--agent", "claude", "--yes"],
        );
        assert_eq!(exit, Some(2), "{mode}: {report}");
        assert_eq!(report["items"].as_array().unwrap().len(), 1, "{mode}");
        assert_eq!(report["items"][0]["id"], "a");
        assert_eq!(report["items"][0]["classified_by"], "claude");
        assert_eq!(report["committed"], true, "{mode}: a is saved");
        let stopped = &report["stopped"];
        assert_eq!(stopped["reason_code"], code, "{mode}");
        assert_eq!(stopped["classified"], 1);
        assert_eq!(stopped["remaining"], json!(["b", "c"]));
        assert!(report.get("skipped").is_none());
        assert_eq!(report["agent_calls"], 2);
        assert_eq!(stopped["providers"][0]["agent"], "claude");
        assert_eq!(stopped["providers"][0]["status"], "failed", "{mode}");
        assert_eq!(stopped["providers"][0]["reason_code"], code, "{mode}");
        assert_eq!(
            fs::read(s.vault.join("meta/b.toml")).unwrap(),
            b_before,
            "{mode}: b is untouched"
        );
        assert_eq!(fs::read(s.vault.join("meta/c.toml")).unwrap(), c_before);
        if mode == "limit-b" {
            assert!(
                stopped["message"]
                    .as_str()
                    .unwrap()
                    .contains("(resets 5pm (utc))"),
                "{stopped}"
            );
        }
    }
}

/// Review of 5df6d58..aed5a39 (M1): once Claude answered in a run, a Claude
/// that disappears (an updater or an uninstall mid-run) is a failure, not
/// "not on this computer": `auto` with Pi off stops instead of saving
/// keywords for the rest.
#[cfg(unix)]
#[test]
fn auto_stops_when_claude_disappears_mid_run() {
    let s = Sandbox::new();
    s.initialize();
    import(&s, "a");
    import(&s, "b");
    let path = fake_claude(&s, "vanish-after-a");
    let b_before = fs::read(s.vault.join("meta/b.toml")).unwrap();
    let (code, report) = run_with(&s, &path, &["classify", "--all", "--yes"]);
    assert_eq!(code, Some(2), "{report}");
    assert_eq!(report["items"][0]["id"], "a");
    assert_eq!(report["items"][0]["classified_by"], "claude");
    assert!(report.get("fallback_reason").is_none(), "{report}");
    let stopped = &report["stopped"];
    assert_eq!(stopped["reason_code"], "AGENT_AUTO_UNAVAILABLE");
    assert_eq!(stopped["remaining"], json!(["b"]));
    assert_eq!(
        stopped["providers"][0]["reason_code"],
        "AGENT_EXECUTABLE_NOT_FOUND"
    );
    assert!(
        stopped["message"]
            .as_str()
            .unwrap()
            .contains("Claude is no longer found on this computer"),
        "{stopped}"
    );
    assert_eq!(fs::read(s.vault.join("meta/b.toml")).unwrap(), b_before);
}
