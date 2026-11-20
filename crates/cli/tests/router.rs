#[allow(dead_code)]
#[path = "support/router.rs"]
mod previous;
#[allow(dead_code)]
mod support;
use serde_json::Value;
use std::fs;
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

#[test]
fn init_router_previews_installs_both_agents_and_repeats_without_changes() {
    let s = Sandbox::new();
    let before = snapshot(&s.root);
    let preview = run(&s, &["init", "--install-router", "--dry-run", "--json"], 0);
    assert_eq!(preview["router"]["targets"].as_array().unwrap().len(), 2);
    assert_eq!(snapshot(&s.root), before);
    let actual = run(&s, &["init", "--install-router", "--json"], 0);
    assert_eq!(actual["router"]["changed"], true);
    for base in [".claude", ".agents"] {
        assert_eq!(
            fs::read_to_string(s.home.join(base).join("skills/kinana/SKILL.md")).unwrap(),
            installed_text()
        );
    }
    let before = snapshot(&s.root);
    assert_eq!(
        run(&s, &["init", "--install-router", "--json"], 0)["router"]["changed"],
        false
    );
    assert_eq!(snapshot(&s.root), before);
}

#[test]
fn existing_foreign_router_blocks_initialization_and_is_preserved() {
    let s = Sandbox::new();
    let path = s.home.join(".agents/skills/kinana");
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("SKILL.md"), "User's skill").unwrap();
    let before = snapshot(&s.root);
    assert_eq!(
        run(&s, &["init", "--install-router", "--json"], 2)["error"]["code"],
        "ROUTER_CONFLICT"
    );
    assert_eq!(snapshot(&s.root), before);
    assert!(!s.vault.exists());
}

#[test]
fn ordinary_init_keeps_router_absent_and_text_install_reports_fixed_paths() {
    let s = Sandbox::new();
    let value = run(&s, &["init", "--json"], 0);
    assert!(value.get("router").is_none());
    assert!(!s.home.join(".claude").exists());
    assert!(!s.home.join(".agents").exists());
    let output = s.run(&["init", "--install-router"]);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Router claude:"));
    assert!(text.contains("Router codex:"));
}

#[cfg(unix)]
#[test]
fn an_earlier_unmodified_router_is_previewed_as_an_upgrade_and_then_upgraded() {
    use previous::install_previous_router;
    let s = Sandbox::new();
    s.initialize();
    let targets = [".claude", ".agents"].map(|base| install_previous_router(&s.home, base));
    let before = snapshot(&s.root);

    let preview = run(&s, &["init", "--install-router", "--dry-run", "--json"], 0);
    assert_eq!(preview["router"]["dry_run"], true);
    assert_eq!(preview["router"]["changed"], true);
    for target in preview["router"]["targets"].as_array().unwrap() {
        assert_eq!(target["action"], "upgrade");
        assert_eq!(target["changed"], true);
    }
    let text = s.run(&["init", "--install-router", "--dry-run"]);
    assert!(text.status.success());
    let text = String::from_utf8(text.stdout).unwrap();
    assert_eq!(text.matches("(upgrade)").count(), 2, "{text}");
    assert_eq!(snapshot(&s.root), before, "a preview writes nothing");

    let actual = run(&s, &["init", "--install-router", "--json"], 0);
    assert_eq!(actual["router"]["changed"], true);
    assert_eq!(actual["router"]["warnings"].as_array().unwrap().len(), 1);
    for (target, report) in targets
        .iter()
        .zip(actual["router"]["targets"].as_array().unwrap())
    {
        assert_eq!(report["action"], "upgrade");
        assert_eq!(report["path"], target.to_str().unwrap());
        assert_eq!(
            fs::read_to_string(target.join("SKILL.md")).unwrap(),
            installed_text()
        );
        // The earlier copy is gone and nothing else is left beside the router.
        let siblings = fs::read_dir(target.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(siblings, ["kinana"]);
    }

    let before = snapshot(&s.root);
    let again = run(&s, &["init", "--install-router", "--json"], 0);
    assert_eq!(again["router"]["changed"], false);
    for target in again["router"]["targets"].as_array().unwrap() {
        assert_eq!(target["action"], "unchanged");
        assert_eq!(target["changed"], false);
    }
    assert_eq!(snapshot(&s.root), before);
}

#[cfg(unix)]
#[test]
fn an_edited_or_unowned_earlier_router_is_refused_and_preserved() {
    use previous::{PREVIOUS_ROUTER, install_previous_router};
    // The user added a rule of their own to an owned earlier copy.
    let s = Sandbox::new();
    install_previous_router(&s.home, ".claude");
    let edited = install_previous_router(&s.home, ".agents");
    fs::write(
        edited.join("SKILL.md"),
        format!("{PREVIOUS_ROUTER}\nAlways answer in French.\n"),
    )
    .unwrap();
    let before = snapshot(&s.root);
    for arguments in [
        &["init", "--install-router", "--dry-run", "--json"][..],
        &["init", "--install-router", "--json"][..],
    ] {
        assert_eq!(
            run(&s, arguments, 2)["error"]["code"],
            "ROUTER_CONFLICT",
            "{arguments:?}"
        );
        assert_eq!(snapshot(&s.root), before);
    }
    assert!(!s.vault.exists());

    // The earlier text placed there by someone else, without our marker.
    let s = Sandbox::new();
    let foreign = s.home.join(".claude/skills/kinana");
    fs::create_dir_all(&foreign).unwrap();
    fs::write(foreign.join("SKILL.md"), PREVIOUS_ROUTER).unwrap();
    let before = snapshot(&s.root);
    assert_eq!(
        run(&s, &["init", "--install-router", "--json"], 2)["error"]["code"],
        "ROUTER_CONFLICT"
    );
    assert_eq!(snapshot(&s.root), before);
}

#[test]
fn a_fresh_install_reports_its_action_for_both_agents() {
    let s = Sandbox::new();
    let preview = run(&s, &["init", "--install-router", "--dry-run", "--json"], 0);
    let actual = run(&s, &["init", "--install-router", "--json"], 0);
    for report in [&preview, &actual] {
        let targets = report["router"]["targets"].as_array().unwrap();
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0]["agent"], "claude");
        assert_eq!(targets[1]["agent"], "codex");
        assert!(targets.iter().all(|target| target["action"] == "install"));
    }
    let text = String::from_utf8(s.run(&["init", "--install-router"]).stdout).unwrap();
    assert_eq!(text.matches("(unchanged)").count(), 2, "{text}");
}

/// The router text this test binary installs: the template naming the
/// `kinana` executable that runs, links resolved.
fn installed_text() -> String {
    let cli = std::path::Path::new(env!("CARGO_BIN_EXE_kinana"))
        .canonicalize()
        .unwrap();
    kinana_core::router::render(&cli).unwrap()
}

/// Runs a copy of the `kinana` binary placed at `cli` (an app bundle path
/// with odd characters, say) against the sandbox.
fn run_copy(s: &Sandbox, cli: &std::path::Path, args: &[&str]) -> std::process::Output {
    if !cli.exists() {
        fs::create_dir_all(cli.parent().unwrap()).unwrap();
        fs::copy(env!("CARGO_BIN_EXE_kinana"), cli).unwrap();
    }
    std::process::Command::new(cli)
        .env("HOME", &s.home)
        .arg("--vault")
        .arg(&s.vault)
        .args(args)
        .output()
        .unwrap()
}

fn installed_word(s: &Sandbox) -> String {
    let text = fs::read_to_string(s.home.join(".claude/skills/kinana/SKILL.md")).unwrap();
    text.split_once("The Kinana CLI is `")
        .and_then(|(_, rest)| rest.split_once("`."))
        .unwrap()
        .0
        .to_owned()
}

/// Owner decision 2026-11-17: the CLI path in the router text is one shell
/// word, a `'` in it escaped; a path with a quote, spaces and Arabic letters
/// runs the CLI itself when the agent's command reaches `/bin/sh`.
#[test]
fn the_installed_path_runs_the_cli_from_a_folder_with_a_quote_spaces_and_arabic() {
    let s = Sandbox::new();
    let cli = s
        .root
        .join("مجلد it's here/Kinana.app/Contents/MacOS/kinana");
    let out = run_copy(&s, &cli, &["init", "--install-router", "--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let word = installed_word(&s);
    assert!(word.starts_with('\'') && word.contains("'\\''"), "{word}");
    let ran = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(format!("{word} --version"))
        .output()
        .unwrap();
    assert!(ran.status.success(), "{ran:?}");
    assert!(
        String::from_utf8_lossy(&ran.stdout).starts_with("kinana "),
        "{ran:?}"
    );
    let installed = fs::read_to_string(s.home.join(".claude/skills/kinana/SKILL.md")).unwrap();
    assert!(installed.len() <= kinana_core::router::MAX_INSTALLED_BYTES);
}

/// Option (b), owner decision 2026-11-17: when Kinana moved, the router is
/// previewed as `relocate` with both paths, written only when confirmed, and
/// a copy the user edited is still refused.
#[test]
fn a_moved_cli_is_previewed_as_relocate_and_written_only_when_run() {
    let s = Sandbox::new();
    let old = s.root.join("Applications/Kinana.app/Contents/MacOS/kinana");
    let new = s.root.join("Moved/Kinana.app/Contents/MacOS/kinana");
    assert!(
        run_copy(&s, &old, &["init", "--install-router", "--json"])
            .status
            .success()
    );
    let before = snapshot(&s.root.join("home"));
    let preview = run_copy(
        &s,
        &new,
        &["init", "--install-router", "--dry-run", "--json"],
    );
    assert!(preview.status.success());
    let preview: Value = serde_json::from_slice(&preview.stdout).unwrap();
    for target in preview["router"]["targets"].as_array().unwrap() {
        assert_eq!(target["action"], "relocate", "{preview}");
        assert_eq!(target["previous_cli_path"], old.to_str().unwrap());
    }
    assert_eq!(preview["router"]["cli_path"], new.to_str().unwrap());
    assert_eq!(snapshot(&s.root.join("home")), before, "the preview wrote");
    assert!(
        run_copy(&s, &new, &["init", "--install-router", "--json"])
            .status
            .success()
    );
    assert_eq!(installed_word(&s), format!("'{}'", new.display()));

    // An edited copy is never relocated.
    let skill = s.home.join(".claude/skills/kinana/SKILL.md");
    fs::write(&skill, fs::read_to_string(&skill).unwrap() + "\nMy note.\n").unwrap();
    let refused = run_copy(&s, &old, &["init", "--install-router", "--json"]);
    assert_eq!(refused.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&refused.stdout).unwrap();
    assert_eq!(value["error"]["code"], "ROUTER_CONFLICT");
    assert!(fs::read_to_string(&skill).unwrap().ends_with("My note.\n"));
}

/// A copy running from App Translocation (or a disk image) would name a path
/// that disappears: refused before any write.
#[test]
fn a_temporary_location_is_refused_without_writing() {
    let s = Sandbox::new();
    let cli = s
        .root
        .join("private/var/folders/x/AppTranslocation/ABC/d/Kinana.app/Contents/MacOS/kinana");
    fs::create_dir_all(cli.parent().unwrap()).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_kinana"), &cli).unwrap();
    let before = snapshot(&s.home);
    let out = run_copy(&s, &cli, &["init", "--install-router", "--json"]);
    assert_eq!(out.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["error"]["code"], "ROUTER_CLI_PATH_TEMPORARY");
    assert_eq!(snapshot(&s.home), before);
}

/// A path that cannot be written as one safe shell word is refused.
#[test]
fn a_path_with_a_backquote_or_a_line_break_is_refused_without_writing() {
    for folder in ["back`quote", "line\nbreak"] {
        let s = Sandbox::new();
        let cli = s.root.join(folder).join("kinana");
        let before = snapshot(&s.home);
        let out = run_copy(&s, &cli, &["init", "--install-router", "--json"]);
        assert_eq!(out.status.code(), Some(2), "{folder:?}");
        let value: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(
            value["error"]["code"], "ROUTER_CLI_PATH_INVALID",
            "{folder:?}"
        );
        assert_eq!(snapshot(&s.home), before);
    }
}

/// The app is not the CLI: run under another name, it names the `kinana`
/// beside it (Kinana.app/Contents/MacOS/kinana beside kinana-desktop).
#[test]
fn another_program_names_the_kinana_beside_it() {
    let s = Sandbox::new();
    let folder = s.root.join("Kinana.app/Contents/MacOS");
    let beside = folder.join("kinana");
    fs::create_dir_all(&folder).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_kinana"), &beside).unwrap();
    let other = folder.join("kinana-desktop");
    let out = run_copy(&s, &other, &["init", "--install-router", "--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(installed_word(&s), format!("'{}'", beside.display()));
}

/// Review of 5de14fb: the router's CLI path is needed only when a legacy
/// router is there to replace; `migrate` from a temporary location (or the
/// `skillsbox` name) without one works as before.
#[test]
fn migrate_needs_no_cli_path_without_a_legacy_router() {
    let s = Sandbox::new();
    let cli = s
        .root
        .join("x/AppTranslocation/ABC/d/Kinana.app/Contents/MacOS/kinana");
    let out = run_copy(&s, &cli, &["migrate", "--json"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
    let compat = s.root.join("bin/skillsbox");
    let out = run_copy(&s, &compat, &["init", "--install-router", "--dry-run", "--json"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["router"]["cli_path"], compat.to_str().unwrap());
}
