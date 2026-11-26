//! Pi as an experimental classifier (0.3, batch 4): the alternative gate,
//! tested only with a fake `pi` (owner decision 1(d), automatic part: zero
//! live calls). Every case also has a broken fake `claude` first on PATH.
#![cfg(unix)]
#[allow(dead_code)]
#[path = "support/json_contract.rs"]
mod contract;
#[allow(dead_code)]
mod support;
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};
use support::{Sandbox, snapshot};

const PROPOSAL: &str = r#"{"category":"web-frontend","category_reason":"Reviews browser components.","stack":["react"],"stage":["review"],"tags":["react"],"role":"Review UI","when_to_use":"Before UI changes","when_not":"Backend work","improved_description":"Review React component accessibility","overlaps_with":[],"confidence":0.9}"#;

struct Fixture {
    s: Sandbox,
    bin: PathBuf,
}

impl Fixture {
    fn new(enabled: bool) -> Self {
        let s = Sandbox::new();
        s.initialize();
        let source = s.root.join("source-ui");
        fs::create_dir(&source).unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\nname: ui\ndescription: Review React components\n---\n# Body\n",
        )
        .unwrap();
        assert!(
            s.run(&["add", source.to_str().unwrap(), "--json"])
                .status
                .success()
        );
        if enabled {
            let path = s.vault.join("box.toml");
            let mut text = fs::read_to_string(&path).unwrap();
            text.push_str(
                "\n[classification_pi]\nenabled = true\nprovider = \"test-provider\"\nmodel = \"test-model\"\n",
            );
            fs::write(&path, text).unwrap();
            let status = Command::new("git")
                .arg("-C")
                .arg(&s.vault)
                .args(["commit", "-qam", "Enable Pi for the test"])
                .status()
                .unwrap();
            assert!(status.success());
        }
        let bin = s.root.join("bin");
        fs::create_dir(&bin).unwrap();
        // Claude is present but proves nothing: `auto` must move on to Pi.
        write_script(&bin.join("claude"), "#!/bin/sh\nexit 1\n");
        Self { s, bin }
    }

    fn pi(&self, mode: &str) {
        self.pi_with(mode, &json!([]));
    }

    /// Like [`Fixture::pi`], with `extra` events after `turn_start`.
    fn pi_with(&self, mode: &str, extra: &Value) {
        self.pi_with_version("1.5.2", mode, extra);
    }

    fn pi_with_version(&self, version: &str, mode: &str, extra: &Value) {
        let trace = self.s.root.join("pi-trace.jsonl");
        let marker = self.s.root.join("MUST_NOT_EXIST");
        let script = r#"#!/usr/bin/python3
import json, os, sys, time
mode = @MODE@
record = {'argv': sys.argv[1:], 'offline': os.environ.get('PI_OFFLINE'), 'package_dir': os.environ.get('PI_PACKAGE_DIR'), 'cwd_entries': os.listdir('.')}
if sys.argv[1:] == ['--version']:
    with open(@TRACE@, 'a') as f: f.write(json.dumps(record) + '\n')
    print('1.6.0' if mode == 'version' else @VERSION@)
    sys.exit(0)
record['stdin'] = sys.stdin.read()
with open(@TRACE@, 'a') as f: f.write(json.dumps(record) + '\n')
def out(event):
    print(json.dumps(event), flush=True)
out({'type': 'session', 'version': 3, 'id': 'x', 'timestamp': 't', 'cwd': os.getcwd()})
if mode == 'fail_early':
    # Pi 1.5.2 without a sign-in or with an unknown model: the header, then
    # _preparePrompt throws before agent_start and Pi exits 1.
    sys.exit(1)
if mode == 'auth_missing':
    # Pi 1.5.2's own check before the run, printed by runPrintMode.
    sys.stderr.write('Warning: errors loading models.json: model not available\n')
    sys.stderr.write('Authentication failed for "test-provider". Credentials may have expired or network is unavailable. Run \'/login test-provider\' to re-authenticate.\n')
    sys.exit(1)
if mode == 'warning_then_other':
    sys.stderr.write('Warning: skipped model catalog for test-provider: model not available\n')
    sys.stderr.write('Error: something else broke\n')
    sys.exit(1)
if mode == 'model_missing':
    sys.stderr.write('Error: Model "test-model" not found. Use --list-models to see available models.\n')
    sys.exit(1)
out({'type': 'agent_start'})
if mode == 'opening_only':
    out({'type': 'turn_start'})
    sys.exit(0)
out({'type': 'turn_start'})
# Pi 1.5.2's first message (recorded against a fake provider, 2026-11-19):
# no text of its own, the --system-prompt text and the working directory.
# Real Pi always sends it; it is mandatory since item 2 of the same day.
if mode != 'no_system':
    prompt = sys.argv[sys.argv.index('--system-prompt') + 1]
    system = {'role': 'system', 'content': '', 'sections': {'preamble': prompt, 'cwd': '<cwd>\n' + os.getcwd() + '\n</cwd>'}, 'timestamp': 1}
    if mode == 'system_tools':
        system['toolsAdded'] = [{'name': 'read', 'description': 'Read a file', 'parameters': {}}]
    if mode == 'system_elsewhere':
        system['sections']['cwd'] = '<cwd>\n/\n</cwd>'
    out({'type': 'message_start', 'message': system})
    out({'type': 'message_end', 'message': system})
for event in json.loads(@EXTRA@):
    out(event)
if mode.startswith('lock_'):
    # Owner, 2026-11-19, round 7, item 1: a refresh in progress holds Pi's
    # sign-in lock; this Pi "finishes" it 3 s after the refused record by
    # removing the lock and leaving SAVED, unless it was stopped first.
    lock = os.path.join(os.environ['HOME'], '.pi', 'agent', 'auth.json.lock')
    open(@MARKER@ + '.pid', 'w').write(str(os.getpid()))
    tool = {'type': 'tool_execution_start', 'toolCallId': 'c', 'toolName': 'read', 'args': {}}
    if mode == 'lock_tool':
        out(tool)
    elif mode == 'lock_null_results':
        out({'type': 'turn_end', 'message': {'role': 'assistant', 'content': []}, 'toolResults': None})
    elif mode == 'lock_retry':
        out({'type': 'auto_retry_start', 'attempt': 1, 'maxAttempts': 3, 'delayMs': 2000, 'errorMessage': 'x'})
    else:
        out({'type': 'compaction_start', 'reason': 'threshold'})
    open(@MARKER@ + '.refused', 'w').write(repr(time.time()))
    if mode == 'lock_then_tool':
        time.sleep(1)
        out(tool)
    if mode != 'lock_stuck':
        time.sleep(3)
        if os.path.isdir(lock):
            os.rmdir(lock)
        open(@MARKER@ + '.saved', 'w').write('refresh saved')
    time.sleep(40)
    open(@MARKER@, 'w').write('ran on')
    sys.exit(0)
if mode == 'retry':
    # Pi 1.5.2 after a failed provider request (recorded: disconnect).
    out({'type': 'message_end', 'message': {'role': 'assistant', 'content': [], 'stopReason': 'error', 'errorMessage': 'Stream ended without finish_reason'}})
    out({'type': 'auto_retry_start', 'attempt': 1, 'maxAttempts': 3, 'delayMs': 2000, 'errorMessage': 'Stream ended without finish_reason'})
    time.sleep(20)
    open(@MARKER@, 'w').write('ran on')
    sys.exit(0)
if mode.startswith('system') or mode == 'two_users':
    user = {'role': 'user', 'content': [{'type': 'text', 'text': record['stdin']}], 'timestamp': 2}
    for _ in range(2 if mode == 'two_users' else 1):
        out({'type': 'message_start', 'message': user})
        out({'type': 'message_end', 'message': user})
if mode in ('system_tools', 'system_elsewhere', 'two_users'):
    time.sleep(20)
    open(@MARKER@, 'w').write('ran on')
    sys.exit(0)
if mode == 'detached':
    import subprocess
    subprocess.Popen(['/bin/sh', '-c', 'sleep 3; echo ran > ' + @MARKER@ + '.detached'], start_new_session=True)
    out({'type': 'tool_execution_start', 'toolCallId': 'c', 'toolName': 'bash', 'args': {}})
    time.sleep(20)
    sys.exit(0)
if mode == 'tool_event':
    out({'type': 'tool_execution_start', 'toolCallId': 'c', 'toolName': 'read', 'args': {'path': '/etc/hosts'}})
    time.sleep(20)
    open(@MARKER@, 'w').write('ran on')
    sys.exit(0)
if mode == 'entry_appended':
    out({'type': 'entry_appended', 'entry': {'type': 'usage', 'usageType': 'cache_warm'}})
    time.sleep(20)
    open(@MARKER@, 'w').write('ran on')
    sys.exit(0)
if mode == 'unknown_event':
    out({'type': 'compaction_start', 'reason': 'threshold'})
    time.sleep(20)
    open(@MARKER@, 'w').write('ran on')
    sys.exit(0)
if mode == 'toolcall_update':
    out({'type': 'message_update', 'assistantMessageEvent': {'type': 'toolcall_start', 'contentIndex': 0, 'id': 'c', 'toolName': 'read'}})
    time.sleep(20)
    open(@MARKER@, 'w').write('ran on')
    sys.exit(0)
if mode == 'detached_ok':
    import subprocess
    subprocess.Popen(['/bin/sh', '-c', 'sleep 3; echo ran > ' + @MARKER@ + '.orphan'], start_new_session=True, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(0.3)
# Known provider errors, as Pi 1.5.2 reports them in the answer.
errors = {
    'login_error': 'OAuth refresh failed for test-provider: token refresh failed (401): {"error": {"message": "Please try signing in again.", "code": "refresh_token_reused"}}',
    'usage_error': 'You have hit your ChatGPT usage limit (plus plan). Try again in ~42 min.',
    'unknown_error': 'Stream ended without finish_reason',
}
if mode in errors:
    sys.stderr.write('Warning: Model "test-model" not found for provider "test-provider". Using custom model id.\n')
    failed = {'role': 'assistant', 'content': [], 'stopReason': 'error', 'errorMessage': errors[mode]}
    out({'type': 'message_start', 'message': failed})
    out({'type': 'message_end', 'message': failed})
    out({'type': 'agent_end', 'messages': [], 'willRetry': False})
    sys.exit(0)
text = 'not json' if mode == 'invalid' else @PROPOSAL@
content = [{'type': 'text', 'text': text}]
if mode == 'toolcall_block':
    content.append({'type': 'toolCall', 'id': 'c', 'name': 'read', 'arguments': {}})
out({'type': 'message_update', 'usage': {}, 'assistantMessageEvent': {'type': 'text_delta', 'contentIndex': 0, 'delta': text}})
out({'type': 'message_end', 'message': {'role': 'assistant', 'content': content, 'stopReason': 'stop'}})
out({'type': 'turn_end', 'message': {'role': 'assistant', 'content': content}, 'toolResults': []})
out({'type': 'agent_end', 'messages': [], 'willRetry': False})
out({'type': 'agent_settled', 'aborted': False})
"#
        .replace("@MODE@", &serde_json::to_string(mode).unwrap())
        .replace("@EXTRA@", &serde_json::to_string(&extra.to_string()).unwrap())
        .replace("@VERSION@", &serde_json::to_string(version).unwrap())
        .replace("@TRACE@", &serde_json::to_string(&trace.to_string_lossy()).unwrap())
        .replace("@MARKER@", &serde_json::to_string(&marker.to_string_lossy()).unwrap())
        .replace("@PROPOSAL@", &serde_json::to_string(PROPOSAL).unwrap());
        // Laid out like the npm package: the version check reads the
        // package.json above the resolved script.
        let package = self.s.root.join("pi-package");
        fs::create_dir_all(package.join("dist")).unwrap();
        fs::write(
            package.join("package.json"),
            json!({
                "name": "@earendil-works/pi-coding-agent",
                "version": if mode == "package_version" { "1.6.0" } else { version }
            })
            .to_string(),
        )
        .unwrap();
        write_script(&package.join("dist/cli.js"), &script);
        let _ = fs::remove_file(self.bin.join("pi"));
        std::os::unix::fs::symlink(package.join("dist/cli.js"), self.bin.join("pi")).unwrap();
        self.pin();
    }

    /// Pins the fake package as it is now, like the reviewed 1.5.2 package
    /// is pinned in core. Debug builds only read this manifest.
    fn pin(&self) {
        let package = self.s.root.join("pi-package");
        let hash = |path: &str| {
            blake3::hash(&fs::read(package.join(path)).unwrap())
                .to_hex()
                .to_string()
        };
        let version: Value =
            serde_json::from_slice(&fs::read(package.join("package.json")).unwrap()).unwrap();
        fs::write(
            self.manifest(),
            json!({
                "version": version["version"],
                "bin": "dist/cli.js",
                "tree": "dist",
                "files": {"package.json": hash("package.json"), "dist/cli.js": hash("dist/cli.js")}
            })
            .to_string(),
        )
        .unwrap();
    }

    fn manifest(&self) -> PathBuf {
        self.s.root.join("pi-manifest.json")
    }

    fn run(&self, args: &[&str]) -> (Option<i32>, Value) {
        self.run_with(args, &[])
    }

    /// Like [`Fixture::run`], with PI_CODING_AGENT_DIR from `env` only.
    fn run_with(&self, args: &[&str], env: &[(&str, &Path)]) -> (Option<i32>, Value) {
        let path =
            std::env::join_paths([self.bin.as_path(), Path::new("/usr/bin"), Path::new("/bin")])
                .unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_kinana"))
            .env("HOME", &self.s.home)
            .env("PATH", path)
            // Must never reach Pi: it would change what `--version` reports.
            .env("PI_PACKAGE_DIR", "/nonexistent-pi-package")
            .env("KINANA_TEST_PI_PACKAGE_MANIFEST", self.manifest())
            .env_remove("PI_CODING_AGENT_DIR")
            .envs(env.iter().copied())
            .arg("--vault")
            .arg(&self.s.vault)
            .args(args)
            .arg("--json")
            .output()
            .unwrap();
        let value = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| {
            panic!(
                "{args:?}: {} {}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
        });
        (out.status.code(), value)
    }

    /// The CLI's stderr for a run without `--json`.
    fn stderr(&self, args: &[&str]) -> String {
        let path =
            std::env::join_paths([self.bin.as_path(), Path::new("/usr/bin"), Path::new("/bin")])
                .unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_kinana"))
            .env("HOME", &self.s.home)
            .env("PATH", path)
            .env("PI_PACKAGE_DIR", "/nonexistent-pi-package")
            .env("KINANA_TEST_PI_PACKAGE_MANIFEST", self.manifest())
            .env_remove("PI_CODING_AGENT_DIR")
            .arg("--vault")
            .arg(&self.s.vault)
            .args(args)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stderr).into_owned()
    }

    fn calls(&self) -> Vec<Value> {
        fs::read_to_string(self.s.root.join("pi-trace.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn prompts(&self) -> usize {
        self.calls()
            .iter()
            .filter(|c| c.get("stdin").is_some())
            .count()
    }
}

fn write_script(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn pi_is_off_by_default_and_never_started() {
    let f = Fixture::new(false);
    f.pi("ok");
    let before = snapshot(&f.s.vault);
    let (code, error) = f.run(&["classify", "ui", "--agent", "pi"]);
    assert_eq!(code, Some(2));
    assert_eq!(error["error"]["code"], "AGENT_PI_DISABLED");
    // `auto` lists Pi as disabled and stops after Claude without starting it.
    let (code, report) = f.run(&["classify", "ui"]);
    assert_eq!(code, Some(2), "{report}");
    assert_eq!(report["stopped"]["reason_code"], "AGENT_AUTO_UNAVAILABLE");
    let pi = &report["stopped"]["providers"][1];
    assert_eq!(
        (pi["agent"].as_str(), pi["status"].as_str()),
        (Some("pi"), Some("disabled"))
    );
    assert!(f.calls().is_empty(), "Pi was started while disabled");
    assert_eq!(snapshot(&f.s.vault), before);
}

#[test]
fn a_valid_pi_answer_is_used_with_every_isolating_flag_and_template_one() {
    let f = Fixture::new(true);
    f.pi("ok");
    let (code, preview) = f.run(&["classify", "ui", "--agent", "pi", "--dry-run"]);
    assert_eq!(code, Some(0));
    assert!(
        preview["estimated_tokens"]["calibration"].is_null(),
        "Pi is not calibrated"
    );
    assert!(f.calls().is_empty(), "a preview starts nothing");

    let (code, report) = f.run(&["classify", "ui", "--agent", "pi"]);
    assert_eq!(code, Some(0), "{report}");
    assert_eq!(report["agent_calls"], 1);
    assert_eq!(report["items"][0]["classified_by"], "pi");
    assert_eq!(report["items"][0]["proposal_created"], true);
    let calls = f.calls();
    assert_eq!(calls[0]["argv"], json!(["--version"]));
    let run = &calls[1];
    let argv = run["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect::<Vec<_>>();
    for flag in [
        "-p",
        "--no-tools",
        "--no-mcp",
        "--no-extensions",
        "--no-skills",
        "--no-context-files",
        "--no-prompt-templates",
        "--no-themes",
        "--no-session",
        "--no-approve",
        "--offline",
    ] {
        assert!(argv.contains(&flag), "missing {flag}: {argv:?}");
    }
    assert!(argv.windows(2).any(|w| w == ["--mode", "json"]));
    assert!(argv.windows(2).any(|w| w == ["--model", "test-model"]));
    assert_eq!(run["offline"], "1");
    assert!(run["package_dir"].is_null(), "PI_PACKAGE_DIR reached Pi");
    assert_eq!(run["cwd_entries"], json!([]));
    let prompt = run["stdin"].as_str().unwrap();
    // Template 1 until the evaluation gate passes (owner, 2026-11-18).
    assert!(!prompt.contains("<taxonomy-guide>"));
    assert!(prompt.contains("Review React components"));
    let shown = f.run(&["show", "ui"]).1;
    assert_eq!(shown["metadata"]["classification"]["classified_by"], "pi");
    assert_eq!(
        shown["metadata"]["classification_context"]["category_reason"],
        "Reviews browser components."
    );
}

#[test]
fn any_tool_activity_or_unknown_event_kills_pi_at_once_without_retry() {
    for mode in [
        "tool_event",
        "unknown_event",
        "toolcall_update",
        "toolcall_block",
    ] {
        let f = Fixture::new(true);
        f.pi(mode);
        let before = snapshot(&f.s.vault);
        let started = Instant::now();
        let (code, report) = f.run(&["classify", "ui", "--agent", "pi"]);
        assert!(
            started.elapsed().as_secs() < 15,
            "{mode}: Pi was not stopped at once"
        );
        assert_eq!(code, Some(2), "{mode}: {report}");
        assert_eq!(
            report["stopped"]["reason_code"], "AGENT_PI_UNAVAILABLE",
            "{mode}"
        );
        assert_eq!(
            report["stopped"]["providers"][0]["reason_code"], "AGENT_TOOLS_NOT_ISOLATED",
            "{mode}"
        );
        assert_eq!(report["agent_calls"], 1, "{mode}: no retry after a refusal");
        assert_eq!(f.prompts(), 1, "{mode}");
        assert!(
            !f.s.root.join("MUST_NOT_EXIST").exists(),
            "{mode}: the process kept running after the refusal"
        );
        assert_eq!(snapshot(&f.s.vault), before, "{mode}: nothing is saved");
    }
}

/// Owner, 2026-11-19, items 1 and 3: Pi's first message may be its system
/// message, word for word Kinana's prompt and Pi's own working directory,
/// with no tool; and one user message per attempt. The fake Pi writes the
/// directory it really runs in, so this checks what Kinana gives the gate.
#[test]
fn the_system_message_and_one_user_message_are_checked_in_a_run() {
    let f = Fixture::new(true);
    f.pi("system_ok");
    let (code, report) = f.run(&["classify", "ui", "--agent", "pi"]);
    assert_eq!(code, Some(0), "{report}");
    assert_eq!(report["items"][0]["classified_by"], "pi");
    for (mode, reason) in [
        ("system_tools", "the system message declared tools"),
        (
            "system_elsewhere",
            "the system message's cwd section is not Pi's working directory",
        ),
        ("two_users", "Pi sent more than one user message"),
    ] {
        let f = Fixture::new(true);
        f.pi(mode);
        let before = snapshot(&f.s.vault);
        let started = Instant::now();
        let (code, report) = f.run(&["classify", "ui", "--agent", "pi"]);
        assert!(
            started.elapsed().as_secs() < 15,
            "{mode}: Pi was not stopped at once"
        );
        assert_eq!(code, Some(2), "{mode}: {report}");
        let failed = &report["stopped"]["providers"][0];
        assert_eq!(failed["reason_code"], "AGENT_TOOLS_NOT_ISOLATED", "{mode}");
        let message = report["stopped"]["message"].as_str().unwrap();
        assert!(message.contains(reason), "{mode}: {message}");
        assert_eq!(report["agent_calls"], 1, "{mode}: no retry after a refusal");
        assert!(
            !f.s.root.join("MUST_NOT_EXIST").exists(),
            "{mode}: Pi ran on"
        );
        assert_eq!(snapshot(&f.s.vault), before, "{mode}: nothing is saved");
    }
}

/// Owner, 2026-11-19, item 2: Pi's system message is runtime evidence of
/// its tools (its `toolsAdded`), so a run that does not open with it is
/// refused at once, like a Claude session without its init event.
#[test]
fn a_run_without_its_system_message_is_refused_at_once() {
    let f = Fixture::new(true);
    f.pi("no_system");
    let before = snapshot(&f.s.vault);
    let (code, report) = f.run(&["classify", "ui", "--agent", "pi"]);
    assert_eq!(code, Some(2), "{report}");
    assert_eq!(
        report["stopped"]["providers"][0]["reason_code"],
        "AGENT_TOOLS_NOT_ISOLATED"
    );
    let message = report["stopped"]["message"].as_str().unwrap();
    assert!(
        message.contains("Pi did not open the run with its system message, which declares its tools: 'message_update' came first"),
        "{message}"
    );
    assert_eq!(report["agent_calls"], 1, "no retry after a refusal");
    assert_eq!(snapshot(&f.s.vault), before, "nothing is saved");
}

/// Owner, 2026-11-19, live test report, item 1d: Pi's known errors stop
/// the run with a fixed code, like Claude's, and Pi's text never appears;
/// an unknown error stays AGENT_EXIT_FAILED.
#[test]
fn known_pi_errors_stop_with_fixed_codes_and_no_raw_text() {
    for (mode, stopped, provider) in [
        ("login_error", "AGENT_NOT_LOGGED_IN", "AGENT_NOT_LOGGED_IN"),
        ("usage_error", "AGENT_USAGE_LIMIT", "AGENT_USAGE_LIMIT"),
        (
            "model_missing",
            "AGENT_MODEL_UNAVAILABLE",
            "AGENT_MODEL_UNAVAILABLE",
        ),
        ("auth_missing", "AGENT_NOT_LOGGED_IN", "AGENT_NOT_LOGGED_IN"),
        (
            "warning_then_other",
            "AGENT_PI_UNAVAILABLE",
            "AGENT_EXIT_FAILED",
        ),
        ("unknown_error", "AGENT_PI_UNAVAILABLE", "AGENT_EXIT_FAILED"),
    ] {
        let f = Fixture::new(true);
        f.pi(mode);
        let before = snapshot(&f.s.vault);
        let (code, report) = f.run(&["classify", "ui", "--agent", "pi"]);
        assert_eq!(code, Some(2), "{mode}: {report}");
        assert_eq!(
            report["stopped"]["reason_code"], stopped,
            "{mode}: {report}"
        );
        assert_eq!(report["stopped"]["providers"][0]["agent"], "pi", "{mode}");
        assert_eq!(
            report["stopped"]["providers"][0]["reason_code"], provider,
            "{mode}"
        );
        // The CLI's own stderr never relays Pi's text either (review L4).
        let stderr = f.stderr(&["classify", "ui", "--agent", "pi"]);
        for raw in [
            "refresh_token",
            "re-authenticate",
            "something else broke",
            "plus plan",
        ] {
            assert!(!stderr.contains(raw), "{mode}: {stderr}");
        }
        let text = report.to_string();
        for raw in [
            "refresh_token",
            "signing in",
            "plus plan",
            "finish_reason",
            "list-models",
            "custom model id",
            "re-authenticate",
            "something else broke",
            "models.json",
        ] {
            assert!(
                !text.contains(raw),
                "{mode}: Pi's text leaked: {raw}: {text}"
            );
        }
        assert_eq!(report["agent_calls"], 1, "{mode}: no retry");
        assert_eq!(snapshot(&f.s.vault), before, "{mode}: nothing is saved");
    }
    // Under auto (Claude broken here), the code is Pi's reason in the stop.
    let f = Fixture::new(true);
    f.pi("login_error");
    let (_, preview) = f.run(&["classify", "ui", "--dry-run"]);
    let token = preview["pi_confirmation"]["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let (code, report) = f.run(&["classify", "ui", "--confirm", &token]);
    assert_eq!(code, Some(2), "{report}");
    assert_eq!(report["stopped"]["reason_code"], "AGENT_AUTO_UNAVAILABLE");
    let pi = report["stopped"]["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["agent"] == "pi")
        .unwrap()
        .clone();
    assert_eq!(pi["reason_code"], "AGENT_NOT_LOGGED_IN", "{report}");
}

/// Review of 2a9ab86, M1: Pi failing before its run (header only, exit 1)
/// is an exit failure, not a tool breach; the same opening with exit 0 is
/// refused for the missing system message.
#[test]
fn pi_failing_before_its_run_is_an_exit_failure() {
    for (mode, code, text) in [
        (
            "fail_early",
            "AGENT_EXIT_FAILED",
            "Pi classification failed",
        ),
        (
            "opening_only",
            "AGENT_TOOLS_NOT_ISOLATED",
            "the stream ended without Pi's system message, which declares its tools",
        ),
    ] {
        let f = Fixture::new(true);
        f.pi(mode);
        let before = snapshot(&f.s.vault);
        let (status, report) = f.run(&["classify", "ui", "--agent", "pi"]);
        assert_eq!(status, Some(2), "{mode}: {report}");
        assert_eq!(
            report["stopped"]["providers"][0]["reason_code"], code,
            "{mode}"
        );
        let message = report["stopped"]["message"].as_str().unwrap();
        assert!(message.contains(text), "{mode}: {message}");
        assert_eq!(report["agent_calls"], 1, "{mode}: no retry");
        assert_eq!(snapshot(&f.s.vault), before, "{mode}: nothing is saved");
    }
}

/// Owner, 2026-11-19, item 3: Pi's automatic retry after a failed provider
/// request stops the run as before, with its own code.
#[test]
fn an_automatic_retry_stops_pi_as_a_failed_connection() {
    let f = Fixture::new(true);
    f.pi("retry");
    let before = snapshot(&f.s.vault);
    let started = Instant::now();
    let (code, report) = f.run(&["classify", "ui", "--agent", "pi"]);
    assert!(
        started.elapsed().as_secs() < 15,
        "Pi was not stopped at once"
    );
    assert_eq!(code, Some(2), "{report}");
    assert_eq!(report["stopped"]["reason_code"], "AGENT_PI_UNAVAILABLE");
    assert_eq!(
        report["stopped"]["providers"][0]["reason_code"],
        "AGENT_CONNECTION_FAILED"
    );
    let message = report["stopped"]["message"].as_str().unwrap();
    assert!(
        message
            .contains("Pi's request to the model provider failed and Pi began an automatic retry"),
        "{message}"
    );
    assert!(!message.contains("AGENT_TOOLS_NOT_ISOLATED"), "{message}");
    assert_eq!(report["agent_calls"], 1, "no retry after the stop");
    assert_eq!(f.prompts(), 1);
    assert!(!f.s.root.join("MUST_NOT_EXIST").exists(), "Pi ran on");
    assert_eq!(snapshot(&f.s.vault), before, "nothing is saved");
}

/// Owner, 2026-11-19, round 7, item 1: one refused run with Pi's sign-in
/// lock present (`lock`) or not, timed; the fake Pi's markers after it.
struct LockRun {
    seconds: f64,
    /// From the moment the fake Pi sent the refused record to the end of
    /// Kinana's run.
    after_refusal: f64,
    reason: Value,
    saved: bool,
    ran_on: bool,
    alive: bool,
    lock_left: bool,
}

fn refused_with_lock(mode: &str, lock: bool) -> LockRun {
    let f = Fixture::new(true);
    f.pi(mode);
    let lock_path = f.s.home.join(".pi/agent/auth.json.lock");
    if lock {
        fs::create_dir_all(&lock_path).unwrap();
    }
    let before = snapshot(&f.s.vault);
    let started = Instant::now();
    let (code, report) = f.run(&["classify", "ui", "--agent", "pi"]);
    let seconds = started.elapsed().as_secs_f64();
    let ended = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    assert_eq!(code, Some(2), "{mode}: {report}");
    assert_eq!(report["agent_calls"], 1, "{mode}: no retry after a refusal");
    assert_eq!(snapshot(&f.s.vault), before, "{mode}: nothing is saved");
    // Long enough for a Pi that was not stopped to leave SAVED.
    std::thread::sleep(std::time::Duration::from_secs(4));
    let marker = f.s.root.join("MUST_NOT_EXIST");
    let refused: f64 = fs::read_to_string(marker.with_extension("refused"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let pid = fs::read_to_string(marker.with_extension("pid")).unwrap();
    let alive = Command::new("/bin/kill")
        .args(["-0", pid.trim()])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success();
    LockRun {
        seconds,
        after_refusal: ended - refused,
        reason: report["stopped"]["providers"][0]["reason_code"].clone(),
        saved: marker.with_extension("saved").exists(),
        ran_on: marker.exists(),
        alive,
        lock_left: lock_path.exists(),
    }
}

/// At once: well under the 3 s after which the fake Pi would leave SAVED,
/// measured from the refused record to the end of Kinana's run (review of
/// 05023fa: a short fixed wait must fail too).
const AT_ONCE: f64 = 1.5;

#[test]
fn a_refusal_about_tools_stops_pi_at_once_even_while_its_sign_in_lock_exists() {
    // A tool event, and tool results that are not a list (review of
    // 05023fa, M1).
    for mode in ["lock_tool", "lock_null_results"] {
        let run = refused_with_lock(mode, true);
        assert_eq!(run.reason, "AGENT_TOOLS_NOT_ISOLATED", "{mode}");
        assert!(!run.saved, "{mode}: Pi was kept alive after a tool refusal");
        assert!(run.after_refusal < AT_ONCE, "{mode}: {}", run.after_refusal);
        assert!(!run.alive && !run.ran_on, "{mode}");
        assert!(run.lock_left, "{mode}: Kinana never touches the lock");
    }
}

#[test]
fn another_refusal_waits_for_pi_s_sign_in_lock_then_stops_pi() {
    for (mode, reason) in [
        ("lock_unknown", "AGENT_TOOLS_NOT_ISOLATED"),
        ("lock_retry", "AGENT_CONNECTION_FAILED"),
    ] {
        let run = refused_with_lock(mode, true);
        assert_eq!(run.reason, reason, "{mode}");
        assert!(run.saved, "{mode}: Pi was stopped before its lock was gone");
        assert!(
            run.after_refusal >= 3.0 && run.after_refusal < 8.0,
            "{mode}: {}",
            run.after_refusal
        );
        assert!(
            !run.alive && !run.ran_on,
            "{mode}: Pi ran on after its lock was gone"
        );
        assert!(!run.lock_left, "{mode}");
    }
}

#[test]
fn a_lock_that_stays_stops_pi_after_twenty_seconds() {
    let run = refused_with_lock("lock_stuck", true);
    assert_eq!(run.reason, "AGENT_TOOLS_NOT_ISOLATED");
    assert!(
        run.after_refusal >= 20.0 && run.after_refusal < 25.0,
        "{}",
        run.after_refusal
    );
    assert!(run.seconds < 35.0, "{}", run.seconds);
    assert!(!run.alive && !run.ran_on && !run.saved);
    assert!(run.lock_left, "Kinana never touches the lock");
}

#[test]
fn without_a_lock_another_refusal_stops_pi_at_once() {
    let run = refused_with_lock("lock_unknown", false);
    assert_eq!(run.reason, "AGENT_TOOLS_NOT_ISOLATED");
    assert!(!run.saved, "Pi was kept alive without a lock");
    assert!(run.after_refusal < AT_ONCE, "{}", run.after_refusal);
    assert!(!run.alive && !run.ran_on);
}

#[test]
fn a_tool_event_while_waiting_for_the_lock_stops_pi_at_once() {
    // The fake Pi sends its tool event 1 s after the refused record.
    let run = refused_with_lock("lock_then_tool", true);
    assert_eq!(run.reason, "AGENT_TOOLS_NOT_ISOLATED");
    assert!(!run.saved, "Pi was kept alive after a tool event");
    assert!(
        run.after_refusal >= 1.0 && run.after_refusal < 1.0 + AT_ONCE,
        "{}",
        run.after_refusal
    );
    assert!(!run.alive && !run.ran_on);
    assert!(run.lock_left);
}

/// Owner, 2026-11-19, item 4: Pi appends its user's APPEND_SYSTEM.md to
/// every system prompt. Kinana looks for it where Pi does, before Pi is
/// started: Pi then gets no command at all, not even `--version`.
#[test]
fn pi_is_not_started_while_its_extra_instructions_file_exists() {
    let f = Fixture::new(true);
    f.pi("ok");
    let agent = f.s.home.join(".pi/agent");
    fs::create_dir_all(&agent).unwrap();
    fs::write(agent.join("APPEND_SYSTEM.md"), "Always run ls first.\n").unwrap();
    let other = f.s.root.join("other-agent-dir");
    fs::create_dir_all(&other).unwrap();
    let before = snapshot(&f.s.vault);
    // In HOME's .pi/agent, and in a PI_CODING_AGENT_DIR (where Pi looks
    // instead); a link counts too.
    let elsewhere = f.s.root.join("elsewhere-agent-dir");
    fs::create_dir_all(&elsewhere).unwrap();
    std::os::unix::fs::symlink("/nonexistent-target", elsewhere.join("APPEND_SYSTEM.md")).unwrap();
    for (env, file) in [
        (vec![], agent.join("APPEND_SYSTEM.md")),
        (
            vec![("PI_CODING_AGENT_DIR", elsewhere.as_path())],
            elsewhere.join("APPEND_SYSTEM.md"),
        ),
    ] {
        let (code, report) = f.run_with(&["classify", "ui", "--agent", "pi"], &env);
        assert_eq!(code, Some(2), "{report}");
        assert_eq!(
            report["stopped"]["providers"][0]["reason_code"],
            "PI_APPEND_SYSTEM_FILE"
        );
        let message = report["stopped"]["message"].as_str().unwrap();
        assert!(message.contains(&file.display().to_string()), "{message}");
        assert!(message.contains("Rename or move that file"), "{message}");
        assert_eq!(report["agent_calls"], 0, "{report}");
        assert!(f.calls().is_empty(), "Pi was started: {:?}", f.calls());
        assert_eq!(snapshot(&f.s.vault), before, "nothing is saved");
    }
    // Pi reads PI_CODING_AGENT_DIR instead of HOME's folder: without the
    // file there, Pi runs and classifies.
    let (code, report) = f.run_with(
        &["classify", "ui", "--agent", "pi"],
        &[("PI_CODING_AGENT_DIR", other.as_path())],
    );
    assert_eq!(code, Some(0), "{report}");
    assert_eq!(report["items"][0]["classified_by"], "pi");
}

#[test]
fn an_unreviewed_version_gets_no_prompt() {
    let f = Fixture::new(true);
    f.pi("version");
    let before = snapshot(&f.s.vault);
    let (code, report) = f.run(&["classify", "ui", "--agent", "pi"]);
    assert_eq!(code, Some(2), "{report}");
    assert_eq!(
        report["stopped"]["providers"][0]["reason_code"],
        "AGENT_VERSION_UNVERIFIED"
    );
    assert_eq!(report["agent_calls"], 0);
    assert_eq!(f.prompts(), 0);
    assert!(
        report["stopped"]["message"]
            .as_str()
            .unwrap()
            .contains("is pending review; accepted versions: 1.5.2, 1.6.1.")
    );
    assert_eq!(snapshot(&f.s.vault), before);
}

#[test]
fn an_invalid_answer_is_retried_once_then_skips_the_skill() {
    let f = Fixture::new(true);
    f.pi("invalid");
    let (code, report) = f.run(&["classify", "ui", "--agent", "pi"]);
    assert_eq!(code, Some(2), "{report}");
    assert!(report.get("stopped").is_none(), "{report}");
    assert_eq!(
        report["skipped"],
        json!([{"id":"ui","agent":"pi","reason_code":"INVALID_AGENT_PROPOSAL","attempts":2}])
    );
    assert_eq!(report["committed"], false);
    assert_eq!(report["agent_calls"], 2);
    assert_eq!(f.prompts(), 2);
}

#[test]
fn auto_uses_claude_then_pi_then_stops_and_asks() {
    let f = Fixture::new(true);
    f.pi("ok");
    let (code, preview) = f.run(&["classify", "ui", "--dry-run"]);
    assert_eq!(code, Some(0));
    assert_eq!(
        preview["providers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| (p["agent"].as_str().unwrap(), p["status"].as_str().unwrap()))
            .collect::<Vec<_>>(),
        [
            ("claude", "planned"),
            ("pi", "fallback"),
            ("codex", "unsupported")
        ]
    );
    // The preview carries the Pi confirmation for exactly these skills.
    let confirmation = &preview["pi_confirmation"];
    assert_eq!(confirmation["model"], "test-model");
    // Contract 11 (owner, 2026-11-24): the model was chosen, not Pi's default.
    assert_eq!(confirmation["model_default"], false);
    assert_eq!(confirmation["provider"], "test-provider");
    assert_eq!(
        (
            confirmation["skills"].as_u64(),
            confirmation["max_calls"].as_u64()
        ),
        (Some(1), Some(2))
    );
    let token = confirmation["token"].as_str().unwrap().to_owned();
    assert!(token.starts_with("pc3."));
    assert!(f.calls().is_empty(), "a preview starts nothing");
    let (code, report) = f.run(&["classify", "ui", "--confirm", &token]);
    assert_eq!(code, Some(0), "{report}");
    assert_eq!(report["items"][0]["classified_by"], "pi");

    let f = Fixture::new(true);
    f.pi("tool_event");
    let before = snapshot(&f.s.vault);
    let token = f.run(&["classify", "ui", "--dry-run"]).1["pi_confirmation"]["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let (code, report) = f.run(&["classify", "ui", "--confirm", &token]);
    assert_eq!(code, Some(2), "{report}");
    let stopped = &report["stopped"];
    assert_eq!(stopped["reason_code"], "AGENT_AUTO_UNAVAILABLE");
    assert_eq!(stopped["providers"][0]["status"], "failed");
    assert_eq!(
        stopped["providers"][1]["reason_code"],
        "AGENT_TOOLS_NOT_ISOLATED"
    );
    assert_eq!(stopped["providers"][2]["status"], "unsupported");
    assert_eq!(snapshot(&f.s.vault), before);
}

/// Owner, 2026-11-18: before `auto` makes its first Pi call, the user
/// confirms the provider, the model and the most calls Pi could make. In a
/// non-interactive CLI that is exit 2 with a preview, until --confirm.
#[test]
fn auto_asks_before_its_first_pi_call_and_writes_nothing_until_confirmed() {
    let f = Fixture::new(true);
    f.pi("ok");
    let source = f.s.root.join("source-zz");
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: zz\ndescription: Write release notes\n---\n# Body\n",
    )
    .unwrap();
    assert!(
        f.s.run(&["add", source.to_str().unwrap(), "--json"])
            .status
            .success()
    );
    let before = snapshot(&f.s.vault);
    // Claude fails on the first skill: the run stops before Pi is started.
    let (code, report) = f.run(&["classify", "--all", "--yes"]);
    assert_eq!(code, Some(2), "{report}");
    let stopped = &report["stopped"];
    assert_eq!(stopped["reason_code"], "PI_CONFIRMATION_REQUIRED");
    assert_eq!(stopped["remaining"], json!(["ui", "zz"]));
    let message = stopped["message"].as_str().unwrap();
    assert!(message.contains("model test-model"), "{message}");
    assert!(message.contains("provider test-provider"), "{message}");
    assert!(message.contains("up to 4 calls"), "{message}");
    assert_eq!(stopped["providers"][0]["status"], "failed");
    assert_eq!(stopped["providers"][1]["status"], "fallback");
    assert_eq!(
        stopped["providers"][1]["reason_code"],
        "PI_CONFIRMATION_REQUIRED"
    );
    let confirmation = &report["pi_confirmation"];
    assert_eq!(
        (
            confirmation["skills"].as_u64(),
            confirmation["max_calls"].as_u64()
        ),
        (Some(2), Some(4))
    );
    assert!(
        f.calls().is_empty(),
        "Pi was started without a confirmation"
    );
    assert_eq!(report["committed"], false);
    assert_eq!(snapshot(&f.s.vault), before, "declining writes nothing");

    // A confirmation for other skills or settings is refused before any call.
    let token = confirmation["token"].as_str().unwrap().to_owned();
    let (code, error) = f.run(&["classify", "ui", "--confirm", &token]);
    assert_eq!(code, Some(2), "{error}");
    assert_eq!(error["error"]["code"], "PI_CONFIRMATION_CHANGED");
    let (code, error) = f.run(&[
        "classify",
        "ui",
        "zz",
        "--yes",
        "--agent",
        "claude",
        "--confirm",
        &token,
    ]);
    assert_eq!(code, Some(1), "{error}");
    assert_eq!(error["error"]["code"], "INVALID_ARGUMENTS");
    let path = f.s.vault.join("box.toml");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.replace("test-model", "other-model")).unwrap();
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&f.s.vault)
            .args(["commit", "-qam", "Change the Pi model"])
            .status()
            .unwrap()
            .success()
    );
    let (code, error) = f.run(&["classify", "ui", "zz", "--yes", "--confirm", &token]);
    assert_eq!(code, Some(2), "{error}");
    assert_eq!(error["error"]["code"], "PI_CONFIRMATION_CHANGED");
    assert!(f.calls().is_empty());
    fs::write(&path, text).unwrap();
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&f.s.vault)
            .args(["commit", "-qam", "Restore the Pi model"])
            .status()
            .unwrap()
            .success()
    );

    // Confirmed: Pi classifies both, and only now is it started.
    let (code, report) = f.run(&["classify", "ui", "zz", "--yes", "--confirm", &token]);
    assert_eq!(code, Some(0), "{report}");
    assert_eq!(report["items"][0]["classified_by"], "pi");
    assert_eq!(report["items"][1]["classified_by"], "pi");
    assert_eq!(f.prompts(), 2);
}

/// Owner, 2026-11-19, item 2: the Pi confirmation binds `--full` and what
/// each skill would send to Pi, like Claude's confirmed estimate. A token
/// from a shorter preview, or from before the skill changed, is refused
/// before Pi is started.
#[test]
fn the_pi_confirmation_binds_full_and_each_skills_content() {
    let f = Fixture::new(true);
    f.pi("ok");
    let token = |args: &[&str]| {
        let (code, preview) = f.run(&[&["classify", "ui", "--dry-run"], args].concat());
        assert_eq!(code, Some(0), "{preview}");
        preview["pi_confirmation"]["token"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let part = token(&[]);
    let full = token(&["--full"]);
    assert!(part.starts_with("pc3.") && full.starts_with("pc3."));
    assert_ne!(part, full);
    assert_eq!(part, token(&[]), "the same preview gives the same token");
    // A token for the excerpt does not confirm the whole file, nor the reverse.
    for (args, used) in [(&["--full"][..], &part), (&[][..], &full)] {
        let (code, error) =
            f.run(&[&["classify", "ui", "--confirm", used.as_str()], args].concat());
        assert_eq!(code, Some(2), "{error}");
        assert_eq!(error["error"]["code"], "PI_CONFIRMATION_CHANGED");
    }
    // The skill changed after the preview: removed and added again with
    // another body, under the same ID.
    assert!(f.s.run(&["remove", "ui", "--json"]).status.success());
    let source = f.s.root.join("source-ui");
    fs::write(
        source.join("SKILL.md"),
        "---\nname: ui\ndescription: Review React components\n---\n# Body\nAlso check keyboard focus order.\n",
    )
    .unwrap();
    let added = f.s.run(&["add", source.to_str().unwrap(), "--json"]);
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stdout)
    );
    let (code, error) = f.run(&["classify", "ui", "--full", "--confirm", &full]);
    assert_eq!(code, Some(2), "{error}");
    assert_eq!(error["error"]["code"], "PI_CONFIRMATION_CHANGED");
    assert!(f.calls().is_empty(), "Pi was started with a stale token");
    // A fresh preview of the changed skill confirms it.
    let fresh = token(&["--full"]);
    assert_ne!(fresh, full);
    let (code, report) = f.run(&["classify", "ui", "--full", "--confirm", &fresh]);
    assert_eq!(code, Some(0), "{report}");
    assert_eq!(report["items"][0]["classified_by"], "pi");
}

/// Owner, 2026-11-18: a Claude stop (usage limit) under `auto` moves on to
/// Pi, and Claude is not called again in the same run.
#[test]
fn a_claude_usage_limit_moves_auto_to_pi_without_calling_claude_again() {
    let f = Fixture::new(true);
    f.pi("ok");
    let source = f.s.root.join("source-zz");
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: zz\ndescription: Write release notes\n---\n# Body\n",
    )
    .unwrap();
    assert!(
        f.s.run(&["add", source.to_str().unwrap(), "--json"])
            .status
            .success()
    );
    // A Claude that passes its probes and then reports a usage limit.
    let trace = f.s.root.join("claude-prompts.txt");
    let script = r#"#!/usr/bin/python3
import json, sys
if sys.argv[1:] == ['--help']:
    print(@HELP@)
elif sys.argv[1:] == ['--version']:
    print('2.1.319 (Claude Code)')
else:
    sys.stdin.read()
    with open(@TRACE@, 'a') as f: f.write('prompt\n')
    print(json.dumps({'type':'system','subtype':'init','tools':['StructuredOutput']}))
    print(json.dumps({'type':'result','subtype':'success','is_error':True,'result':"You've hit your limit · resets 5pm (UTC)"}))
    sys.exit(1)
"#
    .replace(
        "@HELP@",
        &serde_json::to_string(include_str!("../../../tests/fixtures/classification/claude-help.txt"))
            .unwrap(),
    )
    .replace("@TRACE@", &serde_json::to_string(&trace.to_string_lossy()).unwrap());
    write_script(&f.bin.join("claude"), &script);
    let token = f.run(&["classify", "--all", "--dry-run"]).1["pi_confirmation"]["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let (code, report) = f.run(&["classify", "--all", "--yes", "--confirm", &token]);
    assert_eq!(code, Some(0), "{report}");
    assert_eq!(report["items"][0]["classified_by"], "pi");
    assert_eq!(report["items"][1]["classified_by"], "pi");
    let prompts = fs::read_to_string(&trace).unwrap();
    assert_eq!(
        prompts.lines().count(),
        1,
        "Claude was called again after its limit"
    );
    assert!(
        report["items"][1]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("AGENT_USAGE_LIMIT"))
    );
}

/// Owner, 2026-11-18 (pending item 9): the files of the reviewed package are
/// pinned; a changed, extra or relinked file stops classification before Pi
/// runs at all, not even its version probe.
#[test]
fn a_package_file_that_differs_from_the_pinned_one_stops_pi_before_it_starts() {
    let f = Fixture::new(true);
    let package = f.s.root.join("pi-package");
    let before = snapshot(&f.s.vault);
    let refused = |f: &Fixture, what: &str| {
        let (code, report) = f.run(&["classify", "ui", "--agent", "pi"]);
        assert_eq!(code, Some(2), "{what}: {report}");
        let reason = &report["stopped"]["providers"][0]["reason_code"];
        assert_eq!(reason, "AGENT_PACKAGE_UNVERIFIED", "{what}: {report}");
        assert!(f.calls().is_empty(), "{what}: Pi was started");
    };
    // One byte added to the bin after it was pinned.
    f.pi("ok");
    let mut bytes = fs::read(package.join("dist/cli.js")).unwrap();
    bytes.extend_from_slice(b"# changed\n");
    fs::write(package.join("dist/cli.js"), bytes).unwrap();
    refused(&f, "changed bin");
    // An extra file in the pinned tree.
    f.pi("ok");
    fs::write(package.join("dist/extra.js"), "x").unwrap();
    refused(&f, "extra file");
    fs::remove_file(package.join("dist/extra.js")).unwrap();
    // pi linked to another file of the package.
    f.pi("ok");
    write_script(&package.join("dist/other.js"), "#!/bin/sh\nexit 0\n");
    let _ = fs::remove_file(f.bin.join("pi"));
    std::os::unix::fs::symlink(package.join("dist/other.js"), f.bin.join("pi")).unwrap();
    let mut manifest: Value = serde_json::from_slice(&fs::read(f.manifest()).unwrap()).unwrap();
    manifest["files"]["dist/other.js"] =
        json!(blake3::hash(b"#!/bin/sh\nexit 0\n").to_hex().to_string());
    fs::write(f.manifest(), manifest.to_string()).unwrap();
    refused(&f, "other bin");
    assert_eq!(snapshot(&f.s.vault), before);
    // The untouched package runs.
    fs::remove_file(package.join("dist/other.js")).unwrap();
    f.pi("ok");
    let (code, report) = f.run(&["classify", "ui", "--agent", "pi"]);
    assert_eq!(code, Some(0), "{report}");
    assert_eq!(report["items"][0]["classified_by"], "pi");
}

#[test]
fn hand_edited_pi_settings_are_checked_before_pi_is_started() {
    let f = Fixture::new(true);
    f.pi("ok");
    let path = f.s.vault.join("box.toml");
    let original = fs::read_to_string(&path).unwrap();
    let commit = || {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&f.s.vault)
                .args(["commit", "-qam", "Hand edit"])
                .status()
                .unwrap()
                .success()
        )
    };
    // A flag-like model, or (owner, 2026-11-18) an empty provider, or an
    // empty model for a provider Pi has no default for (2026-11-24), with Pi
    // enabled: refused before any preview, confirmation or start.
    for edited in [
        original.replace("model = \"test-model\"", "model = \"--tools\""),
        original.replace("provider = \"test-provider\"\n", ""),
        original.replace("model = \"test-model\"\n", ""),
        original.replace("model = \"test-model\"", "model = \"\""),
    ] {
        assert_ne!(edited, original);
        fs::write(&path, &edited).unwrap();
        commit();
        let before = snapshot(&f.s.vault);
        for args in [
            &["classify", "ui", "--agent", "pi"][..],
            &["classify", "ui"],
            &["classify", "ui", "--dry-run"],
        ] {
            let (code, report) = f.run(args);
            assert_eq!(code, Some(1), "{args:?}: {report}");
            assert_eq!(report["error"]["code"], "INVALID_PI_SETTINGS", "{args:?}");
        }
        assert!(f.calls().is_empty(), "Pi was started with invalid settings");
        assert_eq!(snapshot(&f.s.vault), before);
        fs::write(&path, &original).unwrap();
        commit();
    }
    // Keywords chosen explicitly do not need Pi's settings.
    fs::write(
        &path,
        original.replace("provider = \"test-provider\"\n", ""),
    )
    .unwrap();
    commit();
    let (code, report) = f.run(&["classify", "ui", "--agent", "keywords"]);
    assert_eq!(code, Some(0), "{report}");
}

/// Review of 0694cad, finding 2: `pi --version` alone is a label; the
/// installed package must say the same reviewed version.
#[test]
fn a_package_that_is_not_the_reviewed_version_gets_no_prompt() {
    let f = Fixture::new(true);
    f.pi("package_version");
    let (code, report) = f.run(&["classify", "ui", "--agent", "pi"]);
    assert_eq!(code, Some(2), "{report}");
    assert_eq!(
        report["stopped"]["providers"][0]["reason_code"],
        "AGENT_VERSION_UNVERIFIED"
    );
    assert_eq!(f.prompts(), 0);
}

/// Review of 0694cad, finding 3: a command a tool starts in its own process
/// group is stopped with Pi.
#[test]
fn a_detached_command_started_by_a_tool_is_stopped_too() {
    let f = Fixture::new(true);
    f.pi("detached");
    let (code, report) = f.run(&["classify", "ui", "--agent", "pi"]);
    assert_eq!(code, Some(2), "{report}");
    assert_eq!(
        report["stopped"]["providers"][0]["reason_code"],
        "AGENT_TOOLS_NOT_ISOLATED"
    );
    std::thread::sleep(std::time::Duration::from_secs(5));
    assert!(
        !f.s.root.join("MUST_NOT_EXIST.detached").exists(),
        "the detached command kept running"
    );
}

/// Automated security review of 9fe02c2: a process detached by the agent and
/// left behind after it exits normally was recorded while the agent ran, so
/// it is stopped although its parent chain is gone. (A process the agent
/// starts and leaves within one 20 ms snapshot interval can still escape;
/// only an OS sandbox closes that, which is a pending owner decision.)
#[test]
fn a_process_left_behind_after_a_normal_exit_is_stopped() {
    let f = Fixture::new(true);
    f.pi("detached_ok");
    let (code, report) = f.run(&["classify", "ui", "--agent", "pi"]);
    assert_eq!(code, Some(0), "{report}");
    std::thread::sleep(std::time::Duration::from_secs(5));
    assert!(
        !f.s.root.join("MUST_NOT_EXIST.orphan").exists(),
        "a detached process outlived the run"
    );
}

/// Review of 8d7a5a4 (coverage): Claude classifies the first skill, fails on
/// the second; the confirmation covers only the remaining skill, and the
/// confirmed rerun gives it to Pi.
#[test]
fn a_partial_run_asks_for_pi_only_for_the_remaining_skills() {
    let f = Fixture::new(true);
    f.pi("ok");
    let source = f.s.root.join("source-zz");
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: zz\ndescription: Write release notes\n---\n# Body\n",
    )
    .unwrap();
    assert!(
        f.s.run(&["add", source.to_str().unwrap(), "--json"])
            .status
            .success()
    );
    let script = r#"#!/usr/bin/python3
import json, sys
if sys.argv[1:] == ['--help']:
    print(@HELP@)
elif sys.argv[1:] == ['--version']:
    print('2.1.319 (Claude Code)')
else:
    text = sys.stdin.read()
    if 'name: zz\n' in text:
        sys.exit(1)
    print(json.dumps({'type':'system','subtype':'init','tools':['StructuredOutput']}))
    print(json.dumps({'type':'result','subtype':'success','is_error':False,'terminal_reason':'completed','structured_output':json.loads(@PROPOSAL@)}))
"#
    .replace(
        "@HELP@",
        &serde_json::to_string(include_str!("../../../tests/fixtures/classification/claude-help.txt"))
            .unwrap(),
    )
    .replace("@PROPOSAL@", &serde_json::to_string(PROPOSAL).unwrap());
    write_script(&f.bin.join("claude"), &script);
    let (code, report) = f.run(&["classify", "--all", "--yes"]);
    assert_eq!(code, Some(2), "{report}");
    assert_eq!(report["items"][0]["id"], "ui");
    assert_eq!(report["items"][0]["classified_by"], "claude");
    assert_eq!(report["committed"], true);
    assert_eq!(report["stopped"]["reason_code"], "PI_CONFIRMATION_REQUIRED");
    assert_eq!(report["stopped"]["remaining"], json!(["zz"]));
    let confirmation = &report["pi_confirmation"];
    assert_eq!(
        (
            confirmation["skills"].as_u64(),
            confirmation["max_calls"].as_u64()
        ),
        (Some(1), Some(2))
    );
    assert!(f.calls().is_empty());
    let token = confirmation["token"].as_str().unwrap().to_owned();
    let (code, report) = f.run(&["classify", "zz", "--confirm", &token]);
    assert_eq!(code, Some(0), "{report}");
    assert_eq!(report["items"][0]["classified_by"], "pi");
    assert_eq!(f.prompts(), 1);
}

/// Pi live test, 2026-11-18: the stop names the event the gate refused, here
/// an `entry_appended` whose entry is not a message (owner decision: only a
/// message entry passes).
#[test]
fn the_stop_names_the_event_the_gate_refused() {
    let f = Fixture::new(true);
    f.pi("entry_appended");
    let (code, report) = f.run(&["classify", "ui", "--agent", "pi"]);
    assert_eq!(code, Some(2), "{report}");
    let message = report["stopped"]["message"].as_str().unwrap();
    assert!(
        message.contains("entry_appended carried a 'usage' entry"),
        "{message}"
    );
    assert!(
        !message.to_lowercase().contains("keywords without"),
        "{message}"
    );
    assert!(!f.s.root.join("MUST_NOT_EXIST").exists(), "Pi ran on");
}

/// Owner decision, 2026-11-18: `entry_appended` passes only with a message
/// entry that passes the content check, `queue_update` only with empty
/// queues. One fake Pi run per allowed case and per breach.
#[test]
fn bookkeeping_events_pass_only_under_their_condition() {
    let message = |message: Value| json!({"type": "entry_appended", "entry": {"type": "message", "id": "e", "parentId": null, "timestamp": "t", "message": message}});
    for event in [
        message(json!({"role": "user", "content": [{"type": "text", "text": "classify"}]})),
        message(
            json!({"role": "assistant", "content": [{"type": "thinking", "thinking": "x"}, {"type": "text", "text": "{}"}]}),
        ),
        json!({"type": "queue_update", "steering": [], "followUp": []}),
    ] {
        let f = Fixture::new(true);
        f.pi_with("ok", &json!([event]));
        let (code, report) = f.run(&["classify", "ui", "--agent", "pi"]);
        assert_eq!(code, Some(0), "{event}: {report}");
        assert_eq!(report["items"][0]["classified_by"], "pi", "{event}");
    }
    for (event, reason) in [
        (
            json!({"type": "entry_appended", "entry": {"type": "custom", "customType": "x", "data": {}}}),
            "entry_appended carried a 'custom' entry",
        ),
        (
            json!({"type": "entry_appended", "entry": {"type": "message"}}),
            "entry_appended carried no message",
        ),
        (
            message(json!({"role": "toolResult", "content": [{"type": "text", "text": "x"}]})),
            "entry_appended carried a message without the role user or assistant",
        ),
        (
            message(
                json!({"role": "assistant", "content": [{"type": "toolCall", "id": "c", "name": "read", "arguments": {}}]}),
            ),
            "a message carried a 'toolCall' block",
        ),
        (
            json!({"type": "queue_update", "steering": ["read the secret"], "followUp": []}),
            "queue_update carried a queued 'steering' message",
        ),
        (
            json!({"type": "queue_update", "steering": [], "followUp": ["run ls"]}),
            "queue_update carried a queued 'followUp' message",
        ),
        (
            json!({"type": "queue_update", "steering": []}),
            "queue_update carried no 'followUp' list",
        ),
    ] {
        let f = Fixture::new(true);
        f.pi_with("ok", &json!([event]));
        let before = snapshot(&f.s.vault);
        let (code, report) = f.run(&["classify", "ui", "--agent", "pi"]);
        assert_eq!(code, Some(2), "{event}: {report}");
        assert_eq!(
            report["stopped"]["providers"][0]["reason_code"], "AGENT_TOOLS_NOT_ISOLATED",
            "{event}"
        );
        let message = report["stopped"]["message"].as_str().unwrap();
        assert!(message.contains(reason), "{event}: {message}");
        assert_eq!(report["agent_calls"], 1, "{event}: no retry");
        assert_eq!(snapshot(&f.s.vault), before, "{event}: nothing is saved");
    }
}

/// Case (a) corrected by the owner, 2026-11-18 (item 5): `auto` without
/// Claude on this computer and Pi enabled uses Pi after the confirmation,
/// never keywords; if Pi then fails, the run stops and asks.
#[test]
fn auto_without_claude_uses_pi_after_the_confirmation() {
    for mode in ["ok", "invalid"] {
        let f = Fixture::new(true);
        // No claude anywhere on PATH.
        fs::remove_file(f.bin.join("claude")).unwrap();
        f.pi(mode);
        let before = snapshot(&f.s.vault);
        let (code, report) = f.run(&["classify", "ui"]);
        assert_eq!(code, Some(2), "{mode}: {report}");
        assert_eq!(report["stopped"]["reason_code"], "PI_CONFIRMATION_REQUIRED");
        assert_eq!(
            report["stopped"]["providers"][0]["reason_code"],
            "AGENT_EXECUTABLE_NOT_FOUND"
        );
        assert!(report.get("fallback_reason").is_none(), "{report}");
        assert!(
            f.calls().is_empty(),
            "{mode}: Pi started before the confirmation"
        );
        assert_eq!(snapshot(&f.s.vault), before);
        let token = report["pi_confirmation"]["token"]
            .as_str()
            .unwrap()
            .to_owned();
        let (code, report) = f.run(&["classify", "ui", "--confirm", &token]);
        assert!(report.get("fallback_reason").is_none(), "{mode}: {report}");
        if mode == "ok" {
            assert_eq!(code, Some(0), "{report}");
            assert_eq!(report["items"][0]["classified_by"], "pi");
            assert!(
                report["items"][0]["warnings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|w| w.as_str().unwrap().contains("AGENT_EXECUTABLE_NOT_FOUND")),
                "{report}"
            );
            assert_eq!(f.prompts(), 1);
        } else {
            // Pi fails after the retry: a stop, never keywords.
            assert_eq!(code, Some(2), "{report}");
            assert_eq!(report["stopped"]["reason_code"], "AGENT_AUTO_UNAVAILABLE");
            assert!(
                report["stopped"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("Claude is not found on this computer and Pi did not classify"),
                "{report}"
            );
            assert!(report["items"].as_array().unwrap().is_empty());
            assert_eq!(f.prompts(), 2);
            assert_eq!(snapshot(&f.s.vault), before, "nothing is written");
        }
    }
}

/// Owner, 2026-11-24: the model is optional. "Pi default" is Pi 1.5.2's own
/// default for the provider, passed explicitly (1.5.2 refuses `--provider`
/// without `--model`), shown and bound by the confirmation.
#[test]
fn pi_default_passes_the_provider_default_and_the_confirmation_binds_it() {
    let f = Fixture::new(true);
    f.pi("ok");
    let path = f.s.vault.join("box.toml");
    let original = fs::read_to_string(&path).unwrap();
    let write = |text: &str| {
        fs::write(&path, text).unwrap();
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&f.s.vault)
                .args(["commit", "-qam", "Pi settings"])
                .status()
                .unwrap()
                .success()
        );
    };
    let default = original
        .replace(
            "provider = \"test-provider\"",
            "provider = \"openai-codex\"",
        )
        .replace("model = \"test-model\"\n", "");
    assert_ne!(default, original);
    write(&default);
    let (code, preview) = f.run(&["classify", "ui", "--dry-run"]);
    assert_eq!(code, Some(0), "{preview}");
    let confirmation = &preview["pi_confirmation"];
    assert_eq!(confirmation["provider"], "openai-codex");
    assert_eq!(confirmation["model"], "gpt-6.1-sol");
    // Contract 11 (owner, 2026-11-24): --json says the model is Pi's default.
    assert_eq!(confirmation["model_default"], true);
    let doc = contract::document();
    contract::validate(&doc, &doc["$defs"]["Classify"], &preview).unwrap();
    let token = confirmation["token"].as_str().unwrap().to_owned();
    assert!(token.starts_with("pc3."));
    // The same model chosen by name is another confirmation.
    write(&default.replace(
        "provider = \"openai-codex\"\n",
        "provider = \"openai-codex\"\nmodel = \"gpt-6.1-sol\"\n",
    ));
    let (_, chosen) = f.run(&["classify", "ui", "--dry-run"]);
    assert_eq!(chosen["pi_confirmation"]["model"], "gpt-6.1-sol");
    assert_eq!(chosen["pi_confirmation"]["model_default"], false);
    contract::validate(&doc, &doc["$defs"]["Classify"], &chosen).unwrap();
    assert_ne!(chosen["pi_confirmation"]["token"], token.as_str());
    let (code, refused) = f.run(&["classify", "ui", "--confirm", &token]);
    assert_eq!(code, Some(2), "{refused}");
    assert_eq!(refused["error"]["code"], "PI_CONFIRMATION_CHANGED");
    assert!(
        f.calls().is_empty(),
        "nothing started before a valid confirmation"
    );
    // A run with "Pi default" gives Pi the provider and its default model.
    write(&default);
    let (code, report) = f.run(&["classify", "ui", "--agent", "pi"]);
    assert_eq!(code, Some(0), "{report}");
    assert_eq!(report["items"][0]["classified_by"], "pi");
    let calls = f.calls();
    let argv = calls[1]["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(argv.windows(2).any(|w| w == ["--provider", "openai-codex"]));
    assert!(argv.windows(2).any(|w| w == ["--model", "gpt-6.1-sol"]));
}

#[test]
fn reviewed_versions_bind_their_own_defaults_before_any_provider_starts() {
    let versions = ["1.5.2", "1.6.1"];
    for (provider, models) in [
        ("azure", ["gpt-6.1-sol", "gpt-6.2-sol"]),
        ("openai", ["gpt-6.1-sol", "gpt-6.2-sol"]),
        ("openai-codex", ["gpt-6.1-sol", "gpt-6.2-sol"]),
        (
            "mistral",
            ["devstral-medium-latest", "mistral-medium-latest"],
        ),
    ] {
        let f = Fixture::new(true);
        let config = f.s.vault.join("box.toml");
        let original = fs::read_to_string(&config).unwrap();
        let provider_row = format!("provider = \"{provider}\"");
        let implicit = original
            .replace("provider = \"test-provider\"", &provider_row)
            .replace("model = \"test-model\"\n", "");
        let save = |text: &str| {
            fs::write(&config, text).unwrap();
            assert!(
                Command::new("git")
                    .current_dir(&f.s.vault)
                    .args(["commit", "-qam", "Change fixture provider settings"])
                    .status()
                    .unwrap()
                    .success()
            );
        };
        save(&implicit);
        let preview = || {
            let (code, report) = f.run(&["classify", "ui", "--dry-run"]);
            assert_eq!(code, Some(0), "{report}");
            report["pi_confirmation"].clone()
        };
        let mut tokens = vec![];
        for (version, model) in versions.iter().zip(models) {
            f.pi_with_version(version, "ok", &json!([]));
            let confirmation = preview();
            assert_eq!(confirmation["model"], model);
            assert_eq!(confirmation["model_default"], true);
            tokens.push(confirmation["token"].as_str().unwrap().to_owned());
        }
        for index in [1, 0] {
            let (version, model) = (versions[index], models[index]);
            f.pi_with_version(version, "ok", &json!([]));
            let before = snapshot(&f.s.vault);
            let (code, stale) = f.run(&["classify", "ui", "--confirm", &tokens[1 - index]]);
            assert_eq!(code, Some(2), "{stale}");
            assert_eq!(stale["error"]["code"], "PI_CONFIRMATION_CHANGED");
            assert_eq!(snapshot(&f.s.vault), before);
            let explicit = implicit.replace(
                &provider_row,
                &format!("{provider_row}\nmodel = \"{model}\""),
            );
            save(&explicit);
            let (code, stale) = f.run(&["classify", "ui", "--confirm", &tokens[index]]);
            assert_eq!(code, Some(2), "{stale}");
            assert_eq!(stale["error"]["code"], "PI_CONFIRMATION_CHANGED");
            assert!(f.calls().is_empty());
            save(&implicit);
        }
        f.pi_with_version("1.6.1", "ok", &json!([]));
        let (code, report) = f.run(&["classify", "ui", "--confirm", &tokens[1]]);
        assert_eq!(code, Some(0), "{report}");
        let trace = f.calls();
        let invocation = trace.last().unwrap()["argv"].as_array().unwrap();
        assert!(
            invocation
                .windows(2)
                .any(|pair| pair == [json!("--provider"), json!(provider)])
        );
        assert!(
            invocation
                .windows(2)
                .any(|pair| pair == [json!("--model"), json!(models[1])])
        );
    }
}

#[test]
fn changing_the_package_after_confirmation_stops_before_pi_starts() {
    let f = Fixture::new(true);
    let package = f.s.root.join("pi-package");
    let alternate = f.s.root.join("alternate");
    fs::create_dir(&alternate).unwrap();
    f.pi_with_version("1.6.1", "ok", &json!([]));
    fs::copy(package.join("dist/cli.js"), alternate.join("cli.js")).unwrap();
    fs::copy(package.join("package.json"), alternate.join("package.json")).unwrap();
    fs::copy(f.manifest(), alternate.join("manifest.json")).unwrap();
    f.pi_with_version("1.5.2", "ok", &json!([]));
    let token = f.run(&["classify", "ui", "--dry-run"]).1["pi_confirmation"]["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let script = format!(
        "#!/usr/bin/python3\nfrom shutil import copyfile\ncopyfile({:?}, {:?})\ncopyfile({:?}, {:?})\ncopyfile({:?}, {:?})\nraise SystemExit(1)\n",
        alternate.join("cli.js"),
        package.join("dist/cli.js"),
        alternate.join("package.json"),
        package.join("package.json"),
        alternate.join("manifest.json"),
        f.manifest()
    );
    write_script(&f.bin.join("claude"), &script);
    let before = snapshot(&f.s.vault);
    let (code, report) = f.run(&["classify", "ui", "--confirm", &token]);
    assert_eq!(code, Some(2), "{report}");
    let providers = report["stopped"]["providers"].as_array().unwrap();
    assert!(
        providers
            .iter()
            .any(|p| p["agent"] == "pi" && p["reason_code"] == "PI_CONFIRMATION_CHANGED"),
        "{report}"
    );
    assert!(f.calls().is_empty());
    assert_eq!(snapshot(&f.s.vault), before);
}

#[test]
fn candidate_package_checks_refuse_modified_missing_linked_extra_and_mismatched_versions() {
    for case in ["missing", "changed", "linked", "extra", "mismatch"] {
        let f = Fixture::new(true);
        f.pi_with_version(
            "1.6.1",
            if case == "mismatch" { "version" } else { "ok" },
            &json!([]),
        );
        let package = f.s.root.join("pi-package");
        let additional = package.join("dist/data.js");
        fs::write(&additional, "original fixture").unwrap();
        let manifest_path = f.manifest();
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["files"]["dist/data.js"] =
            json!(blake3::hash(b"original fixture").to_hex().to_string());
        fs::write(manifest_path, manifest.to_string()).unwrap();
        match case {
            "missing" => fs::remove_file(&additional).unwrap(),
            "changed" => fs::write(&additional, "modified").unwrap(),
            "linked" => {
                fs::remove_file(&additional).unwrap();
                let original = f.s.root.join("fixture-data");
                fs::write(&original, "original fixture").unwrap();
                std::os::unix::fs::symlink(original, &additional).unwrap();
            }
            "extra" => fs::write(package.join("dist/extra.js"), "extra fixture").unwrap(),
            _ => {}
        }
        let (_, report) = f.run(&["classify", "ui", "--agent", "pi"]);
        assert_eq!(
            report["stopped"]["providers"][0]["reason_code"],
            if case == "mismatch" {
                "AGENT_VERSION_UNVERIFIED"
            } else {
                "AGENT_PACKAGE_UNVERIFIED"
            },
            "{case}: {report}"
        );
        assert_eq!(f.calls().len(), usize::from(case == "mismatch"));
        assert_eq!(f.prompts(), 0);
    }
}
