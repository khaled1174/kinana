#[allow(dead_code)]
mod support;
use kinana_core::{ProjectLock, hash, lockfile};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use support::Sandbox;

fn success(s: &Sandbox, args: &[&str]) -> Value {
    let out = s.run(args);
    assert!(
        out.status.success(),
        "{args:?}: {} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn tree(root: &Path) -> BTreeMap<PathBuf, Value> {
    fn walk(root: &Path, path: &Path, out: &mut BTreeMap<PathBuf, Value>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                meta.permissions().mode()
            };
            #[cfg(not(unix))]
            let mode = meta.permissions().readonly() as u32;
            let value = if meta.file_type().is_symlink() {
                serde_json::json!(["link", mode, fs::read_link(&path).unwrap()])
            } else if meta.is_dir() {
                serde_json::json!(["directory", mode])
            } else {
                serde_json::json!(["file", mode, fs::read(&path).unwrap()])
            };
            out.insert(path.strip_prefix(root).unwrap().into(), value);
            if meta.is_dir() {
                walk(root, &path, out);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}
fn add(s: &Sandbox, id: &str) -> PathBuf {
    let path = s.root.join(format!("source-{id}"));
    fs::create_dir_all(path.join("empty/nested")).unwrap();
    fs::write(
        path.join("SKILL.md"),
        format!("---\nname: {id}\ndescription: Review React accessibility\n---\nBody\n"),
    )
    .unwrap();
    success(s, &["add", path.to_str().unwrap(), "--json"]);
    path
}
fn project(s: &Sandbox, name: &str) -> PathBuf {
    let p = s.root.join(name);
    fs::create_dir(&p).unwrap();
    fs::write(p.join("README.md"), "keep\n").unwrap();
    fs::write(p.join(".gitignore"), "keep-existing\n").unwrap();
    p
}
fn use_skill(s: &Sandbox, id: &str, p: &Path, agent: &str, mode: &str) -> Value {
    success(
        s,
        &[
            "use",
            id,
            "--project",
            p.to_str().unwrap(),
            "--agent",
            agent,
            "--mode",
            mode,
            "--json",
        ],
    )
}
fn copy(source: &Path, target: &Path) {
    fs::create_dir(target).unwrap();
    fs::set_permissions(target, fs::metadata(source).unwrap().permissions()).unwrap();
    for child in fs::read_dir(source).unwrap() {
        let child = child.unwrap();
        if child.file_type().unwrap().is_dir() {
            copy(&child.path(), &target.join(child.file_name()));
        } else {
            fs::copy(child.path(), target.join(child.file_name())).unwrap();
        }
    }
}

#[cfg(unix)]
#[test]
fn link_and_copy_unuse_restore_the_exact_original_project_tree_for_both_agents() {
    let s = Sandbox::new();
    s.initialize();
    add(&s, "roundtrip");
    for agent in ["claude", "codex", "cursor"] {
        for mode in ["link", "copy"] {
            let p = project(&s, &format!("{agent}-{mode}"));
            let baseline = tree(&p);
            use_skill(&s, "roundtrip", &p, agent, mode);
            let before = tree(&s.root);
            let status = success(&s, &["status", "--project", p.to_str().unwrap(), "--json"]);
            assert_eq!(status["projects"][0]["items"][0]["state"], "current");
            assert_eq!(tree(&s.root), before);
            let preview = success(
                &s,
                &[
                    "unuse",
                    "roundtrip",
                    "--project",
                    p.to_str().unwrap(),
                    "--dry-run",
                    "--json",
                ],
            );
            assert_eq!(preview["lock_removed"], true);
            assert_eq!(tree(&s.root), before);
            let removed = success(
                &s,
                &[
                    "unuse",
                    "roundtrip",
                    "--project",
                    p.to_str().unwrap(),
                    "--json",
                ],
            );
            assert_eq!(removed["warnings"], serde_json::json!([]));
            assert_eq!(tree(&p), baseline, "{agent}/{mode}");
            assert_eq!(
                success(
                    &s,
                    &[
                        "unuse",
                        "roundtrip",
                        "--project",
                        p.to_str().unwrap(),
                        "--json"
                    ]
                )["changed"],
                false
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn unuse_preserves_modified_and_identically_replaced_targets_before_any_batch_write() {
    let s = Sandbox::new();
    s.initialize();
    add(&s, "alpha");
    add(&s, "beta");
    for mode in ["link", "copy"] {
        let p = project(&s, mode);
        use_skill(&s, "alpha", &p, "claude", mode);
        use_skill(&s, "beta", &p, "claude", mode);
        let target = p.join(".claude/skills/alpha");
        let saved = s.root.join(format!("old-{mode}"));
        fs::rename(&target, &saved).unwrap();
        if mode == "link" {
            std::os::unix::fs::symlink(fs::read_link(&saved).unwrap(), &target).unwrap();
        } else {
            copy(&saved, &target);
        }
        let before = tree(&s.root);
        let result = s.run(&[
            "unuse",
            "alpha",
            "beta",
            "--project",
            p.to_str().unwrap(),
            "--json",
        ]);
        assert_eq!(result.status.code(), Some(2));
        assert_eq!(tree(&s.root), before);
        assert_eq!(
            success(&s, &["status", "--project", p.to_str().unwrap(), "--json"])["projects"][0]["items"]
                [0]["state"],
            "modified"
        );
    }
}

#[cfg(unix)]
#[test]
fn preexisting_directories_and_an_empty_lock_are_restored_byte_for_byte() {
    let s = Sandbox::new();
    s.initialize();
    add(&s, "restore");
    let p = project(&s, "existing");
    fs::create_dir_all(p.join(".claude/skills")).unwrap();
    let lock = ProjectLock {
        vault: s.vault.clone(),
        restoration: None,
        bundle: None,
        active: vec![],
    };
    let bytes = format!("# user formatting\n{}\n", toml::to_string(&lock).unwrap());
    fs::write(p.join(".kinana.lock"), bytes).unwrap();
    let baseline = tree(&p);
    use_skill(&s, "restore", &p, "claude", "link");
    success(
        &s,
        &[
            "unuse",
            "restore",
            "--project",
            p.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(tree(&p), baseline);
}

#[cfg(unix)]
#[test]
fn partial_unuse_keeps_other_agents_and_preserves_foreign_directory_contents() {
    let s = Sandbox::new();
    s.initialize();
    add(&s, "item");
    let p = project(&s, "partial");
    use_skill(&s, "item", &p, "all", "link");
    fs::write(p.join(".claude/skills/foreign"), "keep").unwrap();
    success(
        &s,
        &[
            "unuse",
            "item",
            "--project",
            p.to_str().unwrap(),
            "--agent",
            "claude",
            "--json",
        ],
    );
    assert!(p.join(".agents/skills/item").is_symlink());
    assert_eq!(
        fs::read_to_string(p.join(".claude/skills/foreign")).unwrap(),
        "keep"
    );
    let lock = lockfile::read(&p).unwrap().unwrap();
    assert_eq!(lock.active.len(), 2);
    assert_eq!(
        lock.active
            .iter()
            .map(|a| a.agent.as_str())
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from(["codex", "cursor"])
    );
    assert!(p.join(".agents/skills/item").is_symlink());
    success(
        &s,
        &["unuse", "item", "--project", p.to_str().unwrap(), "--json"],
    );
    assert!(!p.join(".agents").exists());
    assert!(!p.join(".cursor").exists());
    assert!(p.join(".claude/skills/foreign").exists());
}

#[cfg(unix)]
#[test]
fn status_reports_stale_modified_and_broken_without_writing_or_following_foreign_links() {
    let s = Sandbox::new();
    s.initialize();
    add(&s, "state");
    let p = project(&s, "status");
    use_skill(&s, "state", &p, "claude", "copy");
    let dist = s.vault.join("dist/state/SKILL.md");
    fs::write(
        &dist,
        format!("{}\nchanged cache\n", fs::read_to_string(&dist).unwrap()),
    )
    .unwrap();
    let before = tree(&s.root);
    let status = success(&s, &["status", "--json"]);
    assert_eq!(status["projects"][0]["items"][0]["state"], "stale");
    assert_eq!(tree(&s.root), before);
    let target = p.join(".claude/skills/state");
    fs::write(target.join("SKILL.md"), "modified").unwrap();
    assert_eq!(
        success(&s, &["status", "--project", p.to_str().unwrap(), "--json"])["projects"][0]["items"]
            [0]["state"],
        "modified"
    );
    fs::rename(&target, s.root.join("saved")).unwrap();
    assert_eq!(
        success(&s, &["status", "--project", p.to_str().unwrap(), "--json"])["projects"][0]["items"]
            [0]["state"],
        "broken"
    );
    std::os::unix::fs::symlink(s.root.join("saved"), &target).unwrap();
    let before = tree(&s.root);
    assert_eq!(
        success(&s, &["status", "--json"])["projects"][0]["items"][0]["state"],
        "modified"
    );
    assert_eq!(tree(&s.root), before);
}

#[cfg(unix)]
#[test]
fn adopt_previews_requires_confirmation_and_only_replaces_identical_ordinary_directories() {
    let s = Sandbox::new();
    s.initialize();
    let source = add(&s, "adopted");
    let p = project(&s, "adopt");
    for agent in [".claude", ".agents"] {
        fs::create_dir_all(p.join(agent).join("skills")).unwrap();
        copy(&source, &p.join(agent).join("skills/adopted"));
    }
    fs::create_dir(p.join(".claude/skills/foreign")).unwrap();
    fs::write(p.join(".claude/skills/foreign/user"), "keep").unwrap();
    std::os::unix::fs::symlink(&source, p.join(".claude/skills/unknown-link")).unwrap();
    let before = tree(&s.root);
    let preview = success(
        &s,
        &[
            "adopt",
            "--project",
            p.to_str().unwrap(),
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(
        preview["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|candidate| candidate["eligible"] == true)
            .count(),
        2
    );
    assert_eq!(tree(&s.root), before);
    assert_eq!(
        s.run(&["adopt", "--project", p.to_str().unwrap(), "--json"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(tree(&s.root), before);
    let actual = success(
        &s,
        &["adopt", "--project", p.to_str().unwrap(), "--yes", "--json"],
    );
    assert_eq!(actual["activation"]["items"].as_array().unwrap().len(), 3);
    assert_eq!(
        actual["activation"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["target"].as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        2
    );
    assert_eq!(
        fs::read_link(p.join(".claude/skills/adopted")).unwrap(),
        s.vault.join("dist/adopted")
    );
    assert!(p.join(".agents/skills/adopted").is_symlink());
    assert!(p.join(".claude/skills/unknown-link").is_symlink());
    assert_eq!(
        fs::read_to_string(p.join(".claude/skills/foreign/user")).unwrap(),
        "keep"
    );
    assert_eq!(
        hash::hash_directory(&source).unwrap(),
        hash::hash_directory(&s.vault.join("skills/adopted")).unwrap()
    );
}

#[cfg(unix)]
#[test]
fn sqlite_failure_preserves_adoption_copies_and_unuse_targets_with_all_state() {
    for operation in ["adopt", "unuse"] {
        let s = Sandbox::new();
        s.initialize();
        let source = add(&s, "failure");
        let p = project(&s, operation);
        if operation == "unuse" {
            use_skill(&s, "failure", &p, "claude", "link");
        } else {
            fs::create_dir_all(p.join(".claude/skills")).unwrap();
            copy(&source, &p.join(".claude/skills/failure"));
        }
        let c = rusqlite::Connection::open(s.home.join(".kinana/index-v2.db")).unwrap();
        if operation == "adopt" {
            c.execute_batch("CREATE TRIGGER refuse BEFORE INSERT ON activations BEGIN SELECT RAISE(FAIL,'injected'); END;").unwrap();
        } else {
            c.execute_batch("CREATE TRIGGER refuse BEFORE DELETE ON activations BEGIN SELECT RAISE(FAIL,'injected'); END;").unwrap();
        }
        drop(c);
        let before = tree(&s.root);
        let args = if operation == "adopt" {
            vec!["adopt", "--project", p.to_str().unwrap(), "--yes", "--json"]
        } else {
            vec![
                "unuse",
                "failure",
                "--project",
                p.to_str().unwrap(),
                "--json",
            ]
        };
        assert_eq!(s.run(&args).status.code(), Some(1));
        assert_eq!(tree(&s.root), before);
    }
}

#[cfg(unix)]
#[test]
fn legacy_locks_cannot_delete_existing_targets_and_unsafe_restoration_paths_are_refused() {
    let s = Sandbox::new();
    s.initialize();
    add(&s, "legacy");
    let p = project(&s, "legacy-project");
    use_skill(&s, "legacy", &p, "claude", "link");
    let path = p.join(".kinana.lock");
    let mut lock = lockfile::read(&p).unwrap().unwrap();
    lock.active[0].ownership = None;
    fs::write(&path, toml::to_string_pretty(&lock).unwrap()).unwrap();
    let before = tree(&s.root);
    assert_eq!(
        s.run(&[
            "unuse",
            "legacy",
            "--project",
            p.to_str().unwrap(),
            "--json"
        ])
        .status
        .code(),
        Some(2)
    );
    assert_eq!(tree(&s.root), before);
    use_skill(&s, "legacy", &p, "claude", "link");
    let mut lock = lockfile::read(&p).unwrap().unwrap();
    lock.restoration.as_mut().unwrap().directories[0].path = "../foreign".into();
    fs::write(&path, toml::to_string_pretty(&lock).unwrap()).unwrap();
    let before = tree(&s.root);
    assert!(
        !s.run(&[
            "unuse",
            "legacy",
            "--project",
            p.to_str().unwrap(),
            "--json"
        ])
        .status
        .success()
    );
    assert_eq!(tree(&s.root), before);
}
