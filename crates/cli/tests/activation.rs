#[allow(dead_code)]
mod support;
use kinana_core::{Bundle, BundleSkill, edit, hash, lockfile, project_registry, vault::Vault};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use support::Sandbox;

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, at: &Path, values: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(at).unwrap() {
            let path = entry.unwrap().path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            if metadata.file_type().is_symlink() {
                values.insert(
                    relative,
                    fs::read_link(&path)
                        .unwrap()
                        .as_os_str()
                        .as_encoded_bytes()
                        .to_vec(),
                );
            } else if metadata.is_dir() {
                values.insert(relative, b"dir".to_vec());
                visit(root, &path, values);
            } else {
                values.insert(relative, fs::read(path).unwrap());
            }
        }
    }
    let mut values = BTreeMap::new();
    visit(root, root, &mut values);
    values
}
fn success(sandbox: &Sandbox, args: &[&str]) -> Value {
    let result = sandbox.run(args);
    assert!(
        result.status.success(),
        "{args:?}: {} {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}
fn vault(sandbox: &Sandbox) -> Vault {
    Vault::new(
        sandbox.vault.clone(),
        sandbox.home.join(".kinana/index-v2.db"),
    )
}
fn add(sandbox: &Sandbox, label: &str, name: &str) -> String {
    let source = sandbox.root.join(format!("source-{label}"));
    fs::create_dir_all(source.join("assets/empty")).unwrap();
    fs::write(source.join("assets/data.bin"), [0, 1, 255]).unwrap();
    fs::write(source.join("SKILL.md"), format!("---\r\nname: {}\r\ndescription: Review React interfaces\r\ncustom: [keep, fields]\r\n---\r\n\r\n# Original body\r\nمرحبا\r\n", serde_json::to_string(name).unwrap())).unwrap();
    let result = success(sandbox, &["add", source.to_str().unwrap(), "--json"]);
    result["imported"][0].as_str().unwrap().into()
}
fn project(sandbox: &Sandbox, name: &str) -> PathBuf {
    let path = sandbox.root.join(name);
    fs::create_dir(&path).unwrap();
    fs::write(path.join(".gitignore"), "keep-existing\n").unwrap();
    path
}
fn use_args<'a>(selection: &'a str, project: &'a str, agent: &'a str) -> Vec<&'a str> {
    vec![
        "use",
        selection,
        "--project",
        project,
        "--agent",
        agent,
        "--json",
    ]
}

#[cfg(unix)]
#[test]
fn default_link_previews_without_writes_then_publishes_all_state_and_is_a_noop() {
    let sandbox = Sandbox::new();
    sandbox.initialize();
    let id = add(&sandbox, "review", "UI Review");
    let project = project(&sandbox, "project");
    let project = project.to_str().unwrap();
    let before = snapshot(&sandbox.root);
    let mut args = use_args(&id, project, "claude");
    args.push("--dry-run");
    let preview = success(&sandbox, &args);
    assert_eq!(preview["dry_run"], true);
    assert_eq!(preview["items"][0]["name"], "UI Review");
    assert_eq!(snapshot(&sandbox.root), before);
    args.pop();
    let report = success(&sandbox, &args);
    let target = Path::new(project).join(".claude/skills/UI Review");
    assert_eq!(
        fs::read_link(&target).unwrap(),
        sandbox.vault.join("dist").join(&id)
    );
    assert_eq!(report["items"][0]["mode"], "link");
    assert_eq!(
        report["gitignore_suggestions"],
        json!(["/.claude/skills/UI\\ Review"])
    );
    assert!(
        report["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning.as_str().unwrap().contains("/reload-skills"))
    );
    assert_eq!(
        fs::read_to_string(Path::new(project).join(".gitignore")).unwrap(),
        "keep-existing\n"
    );
    assert!(!Path::new(project).join(".git").exists());
    let lock = lockfile::read(Path::new(project)).unwrap().unwrap();
    assert_eq!(lock.active[0].name, "UI Review");
    assert_eq!(
        lock.active[0].content_hash,
        hash::hash_directory(&sandbox.vault.join("dist").join(&id)).unwrap()
    );
    assert_eq!(
        project_registry::read(&vault(&sandbox)).unwrap().projects[0].path,
        Path::new(project)
    );
    let connection = rusqlite::Connection::open(sandbox.home.join(".kinana/index-v2.db")).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM activations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(connection);
    let before = snapshot(&sandbox.root);
    let repeated = success(&sandbox, &args);
    assert_eq!(repeated["changed"], false);
    assert_eq!(snapshot(&sandbox.root), before);
}

#[test]
fn copy_refreshes_owned_bytes_but_preserves_a_modified_target_and_unrequested_skills() {
    let sandbox = Sandbox::new();
    sandbox.initialize();
    let id = add(&sandbox, "alpha", "alpha");
    let other = add(&sandbox, "beta", "beta");
    let p = project(&sandbox, "project");
    let ptext = p.to_str().unwrap();
    let mut args = use_args(&id, ptext, "claude");
    args.extend(["--mode", "copy"]);
    success(&sandbox, &args);
    let target = p.join(".claude/skills/alpha");
    assert!(
        !fs::symlink_metadata(&target)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let v = vault(&sandbox);
    let source_before = snapshot(&v.root.join("skills"));
    let mut metadata = edit::read_metadata(&v, &id).unwrap();
    metadata.description.active = "improved".into();
    metadata.description.improved = Some("Review accessible React components".into());
    metadata.description.approved_at = Some("2026-11-13".into());
    edit::apply(
        &v,
        &BTreeMap::from([(id.clone(), metadata)]),
        "Approve fixture description",
        false,
    )
    .unwrap();
    success(&sandbox, &args);
    assert_eq!(
        hash::hash_directory(&target).unwrap(),
        hash::hash_directory(&v.root.join("dist").join(&id)).unwrap()
    );
    success(&sandbox, &use_args(&other, ptext, "claude"));
    assert_eq!(lockfile::read(&p).unwrap().unwrap().active.len(), 2);
    fs::write(target.join("user.txt"), "preserve user content").unwrap();
    let before = snapshot(&sandbox.root);
    assert_eq!(sandbox.run(&args).status.code(), Some(2));
    assert_eq!(snapshot(&sandbox.root), before);
    assert_eq!(snapshot(&v.root.join("skills")), source_before);
}

#[cfg(unix)]
#[test]
fn foreign_targets_and_linked_parents_stop_the_entire_batch_before_writes() {
    use std::os::unix::fs::symlink;
    let sandbox = Sandbox::new();
    sandbox.initialize();
    let a = add(&sandbox, "alpha", "alpha");
    let b = add(&sandbox, "beta", "beta");
    let p = project(&sandbox, "project");
    let foreign = p.join(".claude/skills/beta");
    fs::create_dir_all(&foreign).unwrap();
    fs::write(foreign.join("custom"), "keep").unwrap();
    let before = snapshot(&sandbox.root);
    let result = sandbox.run(&[
        "use",
        &a,
        &b,
        "--project",
        p.to_str().unwrap(),
        "--agent",
        "claude",
        "--json",
    ]);
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(snapshot(&sandbox.root), before);
    let outside = sandbox.root.join("outside");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, p.join(".agents")).unwrap();
    let before = snapshot(&sandbox.root);
    assert!(
        !sandbox
            .run(&use_args(&a, p.to_str().unwrap(), "codex"))
            .status
            .success()
    );
    assert_eq!(snapshot(&sandbox.root), before);
}

#[cfg(unix)]
#[test]
fn explicit_secondary_is_manual_only_for_claude_and_does_not_contaminate_auto_dist() {
    let sandbox = Sandbox::new();
    sandbox.initialize();
    let primary = add(&sandbox, "primary", "primary");
    let secondary = add(&sandbox, "secondary", "secondary");
    let p = project(&sandbox, "project");
    let bundle = Bundle {
        name: "web-app".into(),
        title: "Web app".into(),
        agents: vec!["claude".into(), "codex".into()],
        skills: vec![
            BundleSkill {
                id: primary.clone(),
                role: "primary".into(),
                group: Some("review".into()),
            },
            BundleSkill {
                id: secondary.clone(),
                role: "secondary".into(),
                group: Some("review".into()),
            },
        ],
    };
    let file = sandbox.root.join("bundle.toml");
    fs::write(&file, toml::to_string_pretty(&bundle).unwrap()).unwrap();
    success(
        &sandbox,
        &[
            "bundle",
            "create",
            "--file",
            file.to_str().unwrap(),
            "--json",
        ],
    );
    success(
        &sandbox,
        &use_args("web-app", p.to_str().unwrap(), "claude"),
    );
    assert!(!p.join(".claude/skills/secondary").exists());
    let original = snapshot(&sandbox.vault.join("skills"));
    let auto = fs::read(sandbox.vault.join("dist/secondary/SKILL.md")).unwrap();
    let mut args = use_args("web-app", p.to_str().unwrap(), "claude");
    args.extend(["--include-secondary", &secondary]);
    success(&sandbox, &args);
    let manual = sandbox.vault.join("dist/.manual/secondary");
    assert_eq!(
        fs::read_link(p.join(".claude/skills/secondary")).unwrap(),
        manual
    );
    let text = fs::read_to_string(manual.join("SKILL.md")).unwrap();
    assert!(text.contains("disable-model-invocation: true"));
    assert!(text.ends_with("# Original body\r\nمرحبا\r\n"));
    assert_eq!(
        fs::read(sandbox.vault.join("dist/secondary/SKILL.md")).unwrap(),
        auto
    );
    assert_eq!(snapshot(&sandbox.vault.join("skills")), original);
    let p2 = project(&sandbox, "second-project");
    success(
        &sandbox,
        &use_args(&secondary, p2.to_str().unwrap(), "claude"),
    );
    assert_eq!(
        fs::read_link(p2.join(".claude/skills/secondary")).unwrap(),
        sandbox.vault.join("dist/secondary")
    );
    let p3 = project(&sandbox, "third-project");
    let before = snapshot(&sandbox.root);
    let mut both = use_args("web-app", p3.to_str().unwrap(), "all");
    both.extend(["--include-secondary", &secondary]);
    let result = sandbox.run(&both);
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(
        serde_json::from_slice::<Value>(&result.stdout).unwrap()["error"]["code"],
        "CODEX_MANUAL_UNVERIFIED"
    );
    assert_eq!(snapshot(&sandbox.root), before);
}

#[test]
fn global_codex_names_foreign_locks_and_foreign_registry_are_preserved() {
    let sandbox = Sandbox::new();
    sandbox.initialize();
    let id = add(&sandbox, "alpha", "alpha");
    let p = project(&sandbox, "project");
    let global = sandbox.home.join(".agents/skills/alpha");
    fs::create_dir_all(&global).unwrap();
    fs::write(global.join("SKILL.md"), "foreign").unwrap();
    let before = snapshot(&sandbox.root);
    assert_eq!(
        sandbox
            .run(&use_args(&id, p.to_str().unwrap(), "all"))
            .status
            .code(),
        Some(2)
    );
    assert_eq!(snapshot(&sandbox.root), before);
    let foreign = sandbox.root.join("foreign-vault");
    fs::create_dir(&foreign).unwrap();
    fs::write(
        p.join(".kinana.lock"),
        toml::to_string(&kinana_core::ProjectLock {
            restoration: None,
            vault: foreign.clone(),
            bundle: None,
            active: vec![],
        })
        .unwrap(),
    )
    .unwrap();
    let before = snapshot(&sandbox.root);
    let result = sandbox.run(&use_args(&id, p.to_str().unwrap(), "claude"));
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(
        serde_json::from_slice::<Value>(&result.stdout).unwrap()["error"]["code"],
        "FOREIGN_PROJECT_LOCK"
    );
    assert_eq!(snapshot(&sandbox.root), before);
    let p2 = project(&sandbox, "another-project");
    let registry = kinana_core::ProjectRegistry {
        version: 1,
        projects: vec![kinana_core::ProjectRegistration {
            path: p2.clone(),
            vault: foreign,
        }],
    };
    fs::write(
        sandbox.home.join(".kinana/projects.toml"),
        toml::to_string(&registry).unwrap(),
    )
    .unwrap();
    let before = snapshot(&sandbox.root);
    assert_eq!(
        sandbox
            .run(&use_args(&id, p2.to_str().unwrap(), "claude"))
            .status
            .code(),
        Some(2)
    );
    assert_eq!(snapshot(&sandbox.root), before);
}

#[test]
fn sqlite_failure_happens_before_filesystem_publication() {
    let sandbox = Sandbox::new();
    sandbox.initialize();
    let id = add(&sandbox, "alpha", "alpha");
    let p = project(&sandbox, "project");
    let connection = rusqlite::Connection::open(sandbox.home.join(".kinana/index-v2.db")).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_activation BEFORE INSERT ON projects BEGIN SELECT RAISE(FAIL,'fixture rejection'); END;").unwrap();
    drop(connection);
    let before = snapshot(&sandbox.root);
    let result = sandbox.run(&use_args(&id, p.to_str().unwrap(), "claude"));
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(snapshot(&sandbox.root), before);
}

#[cfg(unix)]
#[test]
fn metadata_manual_mode_is_enforced_and_empty_invocation_defaults_to_auto() {
    let sandbox = Sandbox::new();
    sandbox.initialize();
    let id = add(&sandbox, "alpha", "alpha");
    let p = project(&sandbox, "project");
    let path = sandbox.vault.join("meta/alpha.toml");
    let original = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("{original}\n[invocation]\nmode='manual'\n")).unwrap();
    let report = success(&sandbox, &use_args(&id, p.to_str().unwrap(), "claude"));
    assert_eq!(report["items"][0]["invocation"], "manual");
    assert_eq!(
        fs::read_link(p.join(".claude/skills/alpha")).unwrap(),
        sandbox.vault.join("dist/.manual/alpha")
    );
    let before = snapshot(&sandbox.root);
    assert_eq!(
        sandbox
            .run(&use_args(&id, p.to_str().unwrap(), "all"))
            .status
            .code(),
        Some(2)
    );
    assert_eq!(snapshot(&sandbox.root), before);
    fs::write(&path, format!("{original}\n[invocation]\n")).unwrap();
    let report = success(&sandbox, &use_args(&id, p.to_str().unwrap(), "claude"));
    assert_eq!(report["items"][0]["invocation"], "auto");
    assert_eq!(
        fs::read_link(p.join(".claude/skills/alpha")).unwrap(),
        sandbox.vault.join("dist/alpha")
    );
}

#[test]
fn ambiguous_bundle_selection_is_a_decision_and_other_vault_cache_rows_survive_use() {
    let sandbox = Sandbox::new();
    sandbox.initialize();
    let id = add(&sandbox, "alpha", "alpha");
    let p = project(&sandbox, "project");
    let connection = rusqlite::Connection::open(sandbox.home.join(".kinana/index-v2.db")).unwrap();
    connection
        .execute(
            "INSERT INTO projects(vault,path,bundle) VALUES ('other-vault','other-project','keep')",
            [],
        )
        .unwrap();
    connection.execute("INSERT INTO activations(vault,project_path,skill_id,agent,mode) VALUES ('other-vault','other-project','foreign','claude','copy')",[]).unwrap();
    drop(connection);
    success(&sandbox, &use_args(&id, p.to_str().unwrap(), "claude"));
    let connection = rusqlite::Connection::open(sandbox.home.join(".kinana/index-v2.db")).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT bundle FROM projects WHERE vault='other-vault'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "keep"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM activations WHERE vault='other-vault'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    drop(connection);
    let bundle = Bundle {
        name: id.clone(),
        title: "Ambiguous".into(),
        agents: vec!["claude".into()],
        skills: vec![BundleSkill {
            id: id.clone(),
            role: "primary".into(),
            group: None,
        }],
    };
    let file = sandbox.root.join("ambiguous.toml");
    fs::write(&file, toml::to_string(&bundle).unwrap()).unwrap();
    success(
        &sandbox,
        &[
            "bundle",
            "create",
            "--file",
            file.to_str().unwrap(),
            "--json",
        ],
    );
    let before = snapshot(&sandbox.root);
    let result = sandbox.run(&use_args(&id, p.to_str().unwrap(), "claude"));
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(
        serde_json::from_slice::<Value>(&result.stdout).unwrap()["error"]["code"],
        "AMBIGUOUS_SELECTION"
    );
    assert_eq!(snapshot(&sandbox.root), before);
}

#[cfg(unix)]
#[test]
fn owned_mode_changes_and_missing_generated_cache_recover_without_source_changes() {
    let sandbox = Sandbox::new();
    sandbox.initialize();
    let id = add(&sandbox, "alpha", "alpha");
    let p = project(&sandbox, "project");
    let original = snapshot(&sandbox.vault.join("skills"));
    let mut args = use_args(&id, p.to_str().unwrap(), "claude");
    success(&sandbox, &args);
    args.extend(["--mode", "copy"]);
    success(&sandbox, &args);
    assert!(
        fs::symlink_metadata(p.join(".claude/skills/alpha"))
            .unwrap()
            .is_dir()
    );
    args.truncate(args.len() - 2);
    success(&sandbox, &args);
    assert!(
        fs::symlink_metadata(p.join(".claude/skills/alpha"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    fs::remove_dir_all(sandbox.vault.join("dist/alpha")).unwrap();
    success(&sandbox, &args);
    assert!(sandbox.vault.join("dist/alpha/SKILL.md").exists());
    assert_eq!(snapshot(&sandbox.vault.join("skills")), original);
}

#[test]
fn two_selected_ids_cannot_claim_the_same_original_name() {
    let sandbox = Sandbox::new();
    sandbox.initialize();
    let first = add(&sandbox, "first", "same");
    let source = sandbox.root.join("variant");
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: same\ndescription: Another React review\n---\nVariant bytes\n",
    )
    .unwrap();
    let imported = success(
        &sandbox,
        &[
            "add",
            source.to_str().unwrap(),
            "--suffix",
            "variant",
            "--json",
        ],
    );
    let second = imported["imported"][0].as_str().unwrap();
    let p = project(&sandbox, "project");
    let before = snapshot(&sandbox.root);
    let result = sandbox.run(&[
        "use",
        &first,
        second,
        "--project",
        p.to_str().unwrap(),
        "--agent",
        "claude",
        "--json",
    ]);
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(
        serde_json::from_slice::<Value>(&result.stdout).unwrap()["error"]["code"],
        "ACTIVATION_NAME_CONFLICT"
    );
    assert_eq!(snapshot(&sandbox.root), before);
}

#[cfg(unix)]
#[test]
fn filesystem_name_aliases_trigger_full_rollback_instead_of_overwriting() {
    let sandbox = Sandbox::new();
    sandbox.initialize();
    let first = add(&sandbox, "first", "Upper");
    let source = sandbox.root.join("variant");
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: upper\ndescription: Another React review\n---\nVariant bytes\n",
    )
    .unwrap();
    let imported = success(
        &sandbox,
        &[
            "add",
            source.to_str().unwrap(),
            "--suffix",
            "variant",
            "--json",
        ],
    );
    let second = imported["imported"][0].as_str().unwrap();
    let p = project(&sandbox, "project");
    fs::write(p.join("AliasProbe"), "probe").unwrap();
    let aliases = p.join("aliasprobe").exists();
    fs::remove_file(p.join("AliasProbe")).unwrap();
    let before = snapshot(&sandbox.root);
    let result = sandbox.run(&[
        "use",
        &first,
        second,
        "--project",
        p.to_str().unwrap(),
        "--agent",
        "claude",
        "--json",
    ]);
    if aliases {
        assert_eq!(result.status.code(), Some(2));
        assert_eq!(snapshot(&sandbox.root), before);
    } else {
        assert!(result.status.success());
    }
}

#[test]
fn cursor_link_and_copy_cli_roundtrips_preserve_the_exact_project_tree() {
    for mode in ["link", "copy"] {
        let s = Sandbox::new();
        s.initialize();
        let id = add(&s, "cursor", "cursor-reference");
        let p = project(&s, "مشروع Cursor");
        let before = snapshot(&p);
        let mut args = use_args(&id, p.to_str().unwrap(), "cursor");
        args.extend(["--mode", mode]);
        let mut dry = args.clone();
        dry.push("--dry-run");
        success(&s, &dry);
        assert_eq!(snapshot(&p), before);
        success(&s, &args);
        let target = p.join(".agents/skills/cursor-reference");
        assert_eq!(target.is_symlink(), mode == "link");
        let report = success(&s, &["status", "--project", p.to_str().unwrap(), "--json"]);
        assert_eq!(report["projects"][0]["items"][0]["agent"], "cursor");
        assert_eq!(report["projects"][0]["items"][0]["state"], "current");
        assert_eq!(
            success(&s, &["list", "--agent", "cursor", "--json"])
                .as_array()
                .unwrap()
                .len(),
            1
        );
        success(
            &s,
            &[
                "unuse",
                &id,
                "--project",
                p.to_str().unwrap(),
                "--agent",
                "cursor",
                "--json",
            ],
        );
        assert_eq!(snapshot(&p), before);
        let rejected = s.run(&["classify", &id, "--agent", "cursor", "--json"]);
        assert_eq!(rejected.status.code(), Some(1));
        assert_eq!(
            serde_json::from_slice::<Value>(&rejected.stdout).unwrap()["error"]["code"],
            "INVALID_ARGUMENTS"
        );
    }
}
#[test]
fn all_agents_include_cursor_without_expanding_global_router_installation() {
    let s = Sandbox::new();
    s.initialize();
    let id = add(&s, "all-agents", "all-agents");
    let p = project(&s, "project");
    let before = snapshot(&p);
    let used = success(&s, &use_args(&id, p.to_str().unwrap(), "all"));
    assert_eq!(used["items"].as_array().unwrap().len(), 3);
    assert_eq!(lockfile::read(&p).unwrap().unwrap().active.len(), 3);
    success(
        &s,
        &[
            "unuse",
            &id,
            "--project",
            p.to_str().unwrap(),
            "--agent",
            "all",
            "--json",
        ],
    );
    assert_eq!(snapshot(&p), before);
    let report = success(&s, &["init", "--install-router", "--dry-run", "--json"]);
    assert_eq!(report["router"]["targets"].as_array().unwrap().len(), 2);
    assert!(!s.home.join(".cursor/skills/kinana").exists());
}

#[test]
fn codex_cursor_share_one_owned_target_and_release_it_only_after_the_last_owner() {
    for mode in ["link", "copy"] {
        for order in [["codex", "cursor"], ["cursor", "codex"]] {
            let s = Sandbox::new();
            s.initialize();
            let id = add(&s, "shared", "shared");
            let p = project(&s, "project");
            let baseline = snapshot(&p);
            for agent in order {
                success(
                    &s,
                    &[
                        "use",
                        &id,
                        "--project",
                        p.to_str().unwrap(),
                        "--agent",
                        agent,
                        "--mode",
                        mode,
                        "--json",
                    ],
                );
            }
            let lock = lockfile::read(&p).unwrap().unwrap();
            assert_eq!(lock.active.len(), 2);
            assert_eq!(lock.active[0].ownership, lock.active[1].ownership);
            let target = p.join(".agents/skills/shared");
            assert!(target.exists());
            assert!(!p.join(".cursor").exists());
            let before = snapshot(&p);
            let other = if mode == "link" { "copy" } else { "link" };
            let bad = s.run(&[
                "use",
                &id,
                "--project",
                p.to_str().unwrap(),
                "--agent",
                order[0],
                "--mode",
                other,
                "--json",
            ]);
            assert_eq!(bad.status.code(), Some(2));
            assert_eq!(snapshot(&p), before);
            success(
                &s,
                &[
                    "unuse",
                    &id,
                    "--project",
                    p.to_str().unwrap(),
                    "--agent",
                    order[0],
                    "--json",
                ],
            );
            assert!(target.exists());
            assert_eq!(lockfile::read(&p).unwrap().unwrap().active.len(), 1);
            success(
                &s,
                &[
                    "unuse",
                    &id,
                    "--project",
                    p.to_str().unwrap(),
                    "--agent",
                    order[1],
                    "--json",
                ],
            );
            assert_eq!(snapshot(&p), baseline);
        }
    }
}

#[test]
fn legacy_cursor_lock_keeps_its_historical_target_and_cannot_claim_a_shared_foreign_target() {
    let s = Sandbox::new();
    s.initialize();
    let id = add(&s, "legacy", "legacy");
    let p = project(&s, "project");
    success(&s, &use_args(&id, p.to_str().unwrap(), "cursor"));
    fs::rename(p.join(".agents"), p.join(".cursor")).unwrap();
    let mut lock = lockfile::read(&p).unwrap().unwrap();
    lock.active[0].target_root = None;
    for directory in &mut lock.restoration.as_mut().unwrap().directories {
        directory.path = directory.path.replace(".agents", ".cursor");
    }
    fs::write(p.join(".kinana.lock"), toml::to_string(&lock).unwrap()).unwrap();
    fs::create_dir_all(p.join(".agents/skills/legacy")).unwrap();
    fs::write(p.join(".agents/skills/legacy/keep"), "foreign").unwrap();
    let status = success(&s, &["status", "--project", p.to_str().unwrap(), "--json"]);
    assert_eq!(status["projects"][0]["items"][0]["state"], "current");
    let rejected = s.run(&use_args(&id, p.to_str().unwrap(), "cursor"));
    assert_eq!(rejected.status.code(), Some(2));
    success(
        &s,
        &[
            "unuse",
            &id,
            "--project",
            p.to_str().unwrap(),
            "--agent",
            "cursor",
            "--json",
        ],
    );
    assert!(!p.join(".cursor").exists());
    assert_eq!(
        fs::read_to_string(p.join(".agents/skills/legacy/keep")).unwrap(),
        "foreign"
    );
}

#[test]
fn cursor_can_share_a_legacy_codex_record_without_inventing_its_missing_generation() {
    let s = Sandbox::new();
    s.initialize();
    let id = add(&s, "legacy-codex", "legacy-codex");
    let p = project(&s, "project");
    success(&s, &use_args(&id, p.to_str().unwrap(), "codex"));
    let mut lock = lockfile::read(&p).unwrap().unwrap();
    lock.active[0].generation_hash = None;
    fs::write(p.join(".kinana.lock"), toml::to_string(&lock).unwrap()).unwrap();
    success(&s, &use_args(&id, p.to_str().unwrap(), "cursor"));
    let lock = lockfile::read(&p).unwrap().unwrap();
    assert_eq!(lock.active.len(), 2);
    assert!(
        lock.active
            .iter()
            .find(|s| s.agent == "codex")
            .unwrap()
            .generation_hash
            .is_none()
    );
    assert!(
        lock.active
            .iter()
            .find(|s| s.agent == "cursor")
            .unwrap()
            .generation_hash
            .is_some()
    );
    success(
        &s,
        &[
            "unuse",
            &id,
            "--project",
            p.to_str().unwrap(),
            "--agent",
            "cursor",
            "--json",
        ],
    );
    assert!(p.join(".agents/skills/legacy-codex").exists());
}

#[test]
fn visible_arabic_name_can_activate_but_hidden_formatting_cannot() {
    let sandbox = Sandbox::new();
    sandbox.initialize();
    let project = project(&sandbox, "project");
    let normal = add(&sandbox, "normal", "مراجعة");
    success(
        &sandbox,
        &use_args(&normal, project.to_str().unwrap(), "claude"),
    );
    let before = snapshot(&project);
    for (i, name) in ["مها\u{202e}رة", "de\u{200b}mo", "a\u{2060}b"]
        .iter()
        .enumerate()
    {
        let id = add(&sandbox, &format!("hidden-{i}"), name);
        let output = sandbox.run(&use_args(&id, project.to_str().unwrap(), "claude"));
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("UNSAFE_ACTIVATION_NAME")
                || String::from_utf8_lossy(&output.stdout).contains("UNSAFE_ACTIVATION_NAME")
        );
        assert_eq!(snapshot(&project), before);
    }
}
