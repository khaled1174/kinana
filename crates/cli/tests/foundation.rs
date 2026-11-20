use kinana_core::gitlog;
mod support;
use serde_json::Value;
use std::{fs, path::Path, process::Command};
use support::{Sandbox, copy_tree, snapshot};
fn json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn init_dry_run_has_no_side_effects_and_init_is_idempotent() {
    let sandbox = Sandbox::new();
    let before = snapshot(&sandbox.root);
    let dry = sandbox.run(&["init", "--dry-run", "--json"]);
    assert!(dry.status.success());
    assert_eq!(json(&dry)["dry_run"], true);
    assert_eq!(snapshot(&sandbox.root), before);
    sandbox.initialize();
    assert!(sandbox.vault.join("box.toml").is_file());
    assert!(sandbox.home.join(".kinana/index-v2.db").is_file());
    let before = snapshot(&sandbox.root);
    let again = sandbox.run(&["init", "--json"]);
    assert!(again.status.success());
    assert_eq!(json(&again)["created"], false);
    assert_eq!(snapshot(&sandbox.root), before);
}
#[test]
fn init_path_selects_the_vault_without_requiring_vault_flag() {
    let sandbox = Sandbox::new();
    let output = Command::new(env!("CARGO_BIN_EXE_kinana"))
        .env("HOME", &sandbox.home)
        .env("USERPROFILE", &sandbox.home)
        .args(["init", "--path"])
        .arg(&sandbox.vault)
        .arg("--json")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(sandbox.vault.join("box.toml").exists());
}
#[test]
fn scan_reports_all_four_states_without_mutating_any_files() {
    let sandbox = Sandbox::new();
    sandbox.initialize();
    let project = sandbox.root.join("projects");
    copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/scan/projects"),
        &project,
    );
    // Git itself does not track directory names `.git`; recreate the ignored decoy.
    let decoy = project.join(".git/.claude/skills/ignored");
    fs::create_dir_all(&decoy).unwrap();
    fs::write(
        decoy.join("SKILL.md"),
        "---\nname: ignored\ndescription: Ignore me\n---\n",
    )
    .unwrap();
    let before = snapshot(&sandbox.root);
    let output = sandbox.run(&["scan", project.to_str().unwrap(), "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let value = json(&output);
    assert_eq!(
        value["counts"],
        serde_json::json!({"new":2,"duplicate":1,"variant":1,"broken":2})
    );
    assert_eq!(value["entries"].as_array().unwrap().len(), 6);
    assert_eq!(
        snapshot(&sandbox.root),
        before,
        "scan must not modify source projects, vault, or index"
    );
}
#[test]
fn list_and_show_read_source_files_and_preserve_raw_markdown() {
    let sandbox = Sandbox::new();
    sandbox.initialize();
    sandbox.seed("api-review", "api-review", "backend-api");
    let before = snapshot(&sandbox.root);
    let output = sandbox.run(&[
        "list",
        "--category",
        "backend-api",
        "--agent",
        "codex",
        "--json",
    ]);
    assert!(output.status.success());
    assert_eq!(json(&output)[0]["id"], "api-review");
    let output = sandbox.run(&["list", "--audit", "risky", "--json"]);
    assert!(output.status.success());
    assert_eq!(json(&output), serde_json::json!([]));
    let output = sandbox.run(&["show", "api-review", "--json"]);
    assert!(output.status.success());
    let value = json(&output);
    assert_eq!(value["metadata"]["description"]["active"], "original");
    assert!(
        value["raw_skill_md"]
            .as_str()
            .unwrap()
            .contains("<script>untrusted()</script>")
    );
    assert_eq!(snapshot(&sandbox.root), before);
}
#[test]
fn doctor_is_read_only_and_reports_missing_index_as_failure() {
    let sandbox = Sandbox::new();
    sandbox.initialize();
    let before = snapshot(&sandbox.root);
    let output = sandbox.run(&["doctor", "--json"]);
    assert!(output.status.success());
    assert_eq!(json(&output)["healthy"], true);
    assert_eq!(snapshot(&sandbox.root), before);
    fs::remove_file(sandbox.home.join(".kinana/index-v2.db")).unwrap();
    let before = snapshot(&sandbox.root);
    let output = sandbox.run(&["doctor", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(json(&output)["healthy"], false);
    assert_eq!(snapshot(&sandbox.root), before);
}
#[test]
fn invalid_cli_and_missing_skills_return_stable_json_errors() {
    let sandbox = Sandbox::new();
    sandbox.initialize();
    for args in [
        &["show", "missing", "--json"][..],
        &["show", "../escape", "--json"],
        &["scan", "--json"],
        &["list", "--agent", "unknown", "--json"],
        &["scan", "/definitely-missing-kinana-root", "--json"],
    ] {
        let output = sandbox.run(args);
        assert_eq!(output.status.code(), Some(1));
        let value = json(&output);
        assert!(value["error"]["code"].is_string());
        assert!(value["error"]["message"].is_string());
        assert!(value["error"]["reason_code"].is_string());
        assert_eq!(value["error"].as_object().unwrap().len(), 3);
    }
}
#[test]
fn init_refuses_a_foreign_directory_without_changing_it() {
    let sandbox = Sandbox::new();
    fs::create_dir(&sandbox.vault).unwrap();
    fs::write(sandbox.vault.join("keep.txt"), "owner data").unwrap();
    let before = snapshot(&sandbox.root);
    let output = sandbox.run(&["init", "--json"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(json(&output)["error"]["code"], "VAULT_CONFLICT");
    assert_eq!(snapshot(&sandbox.root), before);
}
#[cfg(unix)]
#[test]
fn scan_never_follows_a_symlink_or_runs_a_skill_script() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let sandbox = Sandbox::new();
    sandbox.initialize();
    let project = sandbox.root.join("project");
    let skills = project.join(".agents/skills");
    fs::create_dir_all(&skills).unwrap();
    let outside = sandbox.root.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(
        outside.join("SKILL.md"),
        "---\nname: outside\ndescription: Outside\n---\n",
    )
    .unwrap();
    symlink(&outside, skills.join("outside")).unwrap();
    let safe = skills.join("safe");
    fs::create_dir(&safe).unwrap();
    fs::write(
        safe.join("SKILL.md"),
        "---\nname: safe\ndescription: Safe\n---\n",
    )
    .unwrap();
    let script = safe.join("never.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\ntouch '{}'\n",
            sandbox.root.join("EXECUTED").display()
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    let output = sandbox.run(&["scan", project.to_str().unwrap(), "--json"]);
    assert!(output.status.success());
    let value = json(&output);
    assert_eq!(value["counts"]["new"], 1);
    assert_eq!(value["entries"].as_array().unwrap().len(), 1);
    assert!(!value["warnings"].as_array().unwrap().is_empty());
    assert!(!sandbox.root.join("EXECUTED").exists());
}

#[test]
fn scan_provenance_uses_an_actual_local_remote_of_any_name() {
    let sandbox = Sandbox::new();
    sandbox.initialize();
    let project = sandbox.root.join("project");
    let skill = project.join(".agents/skills/provenance");
    fs::create_dir_all(&skill).unwrap();
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: provenance\ndescription: Provenance\n---\n",
    )
    .unwrap();
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&project)
            .args(["init", "--quiet", "--template="])
            .status()
            .unwrap()
            .success()
    );
    let global = sandbox.root.join("global.gitconfig");
    fs::write(
        &global,
        "[remote \"origin\"]\n url = https://github.com/false/global-source.git\n",
    )
    .unwrap();
    let scan = || {
        Command::new(env!("CARGO_BIN_EXE_kinana"))
            .env("HOME", &sandbox.home)
            .env("USERPROFILE", &sandbox.home)
            .env("GIT_CONFIG_GLOBAL", &global)
            .arg("--vault")
            .arg(&sandbox.vault)
            .arg("scan")
            .arg(&project)
            .arg("--json")
            .output()
            .unwrap()
    };
    let output = scan();
    assert!(output.status.success());
    assert_eq!(json(&output)["entries"][0]["source"]["kind"], "unknown");
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&project)
            .args([
                "remote",
                "add",
                "upstream",
                "https://github.com/real/skills.git"
            ])
            .status()
            .unwrap()
            .success()
    );
    let output = scan();
    assert!(output.status.success());
    assert_eq!(
        json(&output)["entries"][0]["source"]["url"],
        "https://github.com/real/skills.git"
    );
}

#[test]
fn undo_previews_then_reverts_source_metadata_and_refreshes_index() {
    let sandbox = Sandbox::new();
    sandbox.initialize();
    sandbox.seed("review", "review", "backend-api");
    gitlog::commit(&sandbox.vault, "Import review").unwrap();
    let target = gitlog::head(&sandbox.vault).unwrap();
    let before = snapshot(&sandbox.root);
    let dry = sandbox.run(&["undo", "--dry-run", "--json"]);
    assert!(
        dry.status.success(),
        "{}",
        String::from_utf8_lossy(&dry.stdout)
    );
    let value: Value = serde_json::from_slice(&dry.stdout).unwrap();
    assert_eq!(value["reverted_commit"], target);
    assert_eq!(value["dry_run"], true);
    assert_eq!(snapshot(&sandbox.root), before);
    let undone = sandbox.run(&["undo", "--json"]);
    assert!(
        undone.status.success(),
        "{}",
        String::from_utf8_lossy(&undone.stdout)
    );
    assert!(!sandbox.vault.join("skills/review").exists());
    assert!(!sandbox.vault.join("meta/review.toml").exists());
    let list = sandbox.run(&["list", "--json"]);
    assert!(list.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&list.stdout).unwrap(),
        serde_json::json!([])
    );
    assert!(sandbox.run(&["doctor", "--json"]).status.success());
}
#[test]
fn undo_initial_or_dirty_vault_requires_a_decision_without_changes() {
    let sandbox = Sandbox::new();
    sandbox.initialize();
    let before = snapshot(&sandbox.root);
    let initial = sandbox.run(&["undo", "--json"]);
    assert_eq!(initial.status.code(), Some(2));
    assert_eq!(snapshot(&sandbox.root), before);
    sandbox.seed("review", "review", "backend-api");
    gitlog::commit(&sandbox.vault, "Import review").unwrap();
    fs::write(sandbox.vault.join("skills/review/owner.txt"), "owner data").unwrap();
    let before = snapshot(&sandbox.root);
    let dirty = sandbox.run(&["undo", "--json"]);
    assert_eq!(dirty.status.code(), Some(2));
    assert_eq!(snapshot(&sandbox.root), before);
}

#[cfg(unix)]
#[test]
fn scanner_provenance_cannot_execute_a_sibling_skill_git_binary() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::new();
    sandbox.initialize();
    let project = sandbox.root.join("project");
    let skills = project.join(".agents/skills");
    for name in ["evil", "good"] {
        let skill = skills.join(name);
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: Static metadata\n---\n"),
        )
        .unwrap();
    }
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&project)
            .args(["init", "--quiet", "--template="])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&project)
            .args([
                "remote",
                "add",
                "origin",
                "https://github.com/owner/skills.git"
            ])
            .status()
            .unwrap()
            .success()
    );
    let executable = skills.join("evil/git");
    let marker = sandbox.root.join("EXECUTED");
    fs::write(
        &executable,
        format!("#!/bin/sh\nprintf ran > '{}'\nexit 1\n", marker.display()),
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let inherited = std::env::var_os("PATH").unwrap();
    let path = std::env::join_paths(
        [skills.join("evil")]
            .into_iter()
            .chain(std::env::split_paths(&inherited)),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_kinana"))
        .env("HOME", &sandbox.home)
        .env("USERPROFILE", &sandbox.home)
        .env("PATH", path)
        .arg("--vault")
        .arg(&sandbox.vault)
        .arg("scan")
        .arg(&project)
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(!marker.exists());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["entries"].as_array().unwrap().len(), 2);
    assert!(
        value["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["source"]["url"] == "https://github.com/owner/skills.git")
    );
}
