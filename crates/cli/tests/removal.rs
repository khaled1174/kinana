#[allow(dead_code)]
mod support;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
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

/// Run a command that must fail; return its exit code, error code and message.
fn refused(s: &Sandbox, args: &[&str]) -> (i32, String, String) {
    let output = s.run(&[args, &["--json"]].concat());
    let context = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(!output.status.success(), "{args:?} succeeded: {context}");
    let value: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{args:?}: {error}: {context}"));
    (
        output.status.code().unwrap(),
        value["error"]["code"].as_str().unwrap().to_owned(),
        value["error"]["message"].as_str().unwrap().to_owned(),
    )
}

fn add(s: &Sandbox, id: &str) {
    let source = s.root.join(format!("source-{id}"));
    fs::create_dir_all(source.join("assets/empty/deep")).unwrap();
    fs::write(
        source.join("SKILL.md"),
        format!("---\nname: {id}\ndescription: Review API contracts\n---\n# Review\nCompare request and response contracts before implementation.\n"),
    )
    .unwrap();
    fs::write(source.join("assets/bytes.bin"), [0, 1, 255, 13, 10]).unwrap();
    json(s, &["add", source.to_str().unwrap()]);
}

fn setup() -> Sandbox {
    let s = Sandbox::new();
    s.initialize();
    add(&s, "api");
    add(&s, "alternate");
    s
}

/// `members` are (id, role, group) rows of one bundle named `api-kit`.
fn bundle(s: &Sandbox, members: &[(&str, &str, Option<&str>)]) {
    let mut text = String::from("name = \"api-kit\"\ntitle = \"عدة API\"\nagents = [\"claude\"]\n");
    if members.is_empty() {
        text.push_str("skills = []\n");
    }
    for (id, role, group) in members {
        text.push_str(&format!("[[skills]]\nid = \"{id}\"\nrole = \"{role}\"\n"));
        if let Some(group) = group {
            text.push_str(&format!("group = \"{group}\"\n"));
        }
    }
    let file = s.root.join("bundle-input.toml");
    fs::write(&file, text).unwrap();
    json(s, &["bundle", "create", "--file", file.to_str().unwrap()]);
}

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
fn head(root: &Path) -> String {
    git(root, &["rev-parse", "HEAD"])
}
fn commits(root: &Path) -> usize {
    git(root, &["rev-list", "--count", "HEAD"])
        .trim()
        .parse()
        .unwrap()
}
fn subject(root: &Path) -> String {
    git(root, &["log", "-1", "--format=%s"]).trim().to_owned()
}

/// Everything a command may touch: vault tree (with .git), index, registry, projects.
fn everything(s: &Sandbox) -> (Vec<(PathBuf, Vec<u8>)>, String) {
    (tree(&s.root), head(&s.vault))
}

fn no_staging_left(s: &Sandbox) {
    for directory in [s.vault.join("dist"), s.vault.join(".git"), s.vault.clone()] {
        for entry in fs::read_dir(directory).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            assert!(
                !name.starts_with(".skillsbox-") || name == ".skillsbox-build.toml",
                "leftover staging: {name}"
            );
        }
    }
}

fn project(s: &Sandbox, name: &str) -> PathBuf {
    let path = s.root.join(name);
    fs::create_dir(&path).unwrap();
    path
}

#[test]
fn remove_then_undo_restores_tree_meta_bundles_dist_and_index() {
    let s = setup();
    bundle(
        &s,
        &[
            ("alternate", "primary", Some("contracts")),
            ("api", "secondary", Some("contracts")),
        ],
    );
    let before: Vec<_> = ["skills", "meta", "bundles", "dist"]
        .iter()
        .map(|name| tree(&s.vault.join(name)))
        .collect();
    let list = json(&s, &["list"]);
    let detail = json(&s, &["show", "api"]);
    let count = commits(&s.vault);

    let report = json(&s, &["remove", "api"]);
    assert_eq!(
        report,
        json!({
            "id": "api",
            "dry_run": false,
            "changed": true,
            "committed": true,
            "removed_paths": ["skills/api", "meta/api.toml", "dist/api"],
            "bundles_changed": ["api-kit"],
        })
    );
    assert_eq!(commits(&s.vault), count + 1, "exactly one vault commit");
    assert_eq!(subject(&s.vault), "Remove skill api");
    assert!(git(&s.vault, &["status", "--porcelain"]).is_empty());
    for gone in ["skills/api", "meta/api.toml", "dist/api"] {
        assert!(
            fs::symlink_metadata(s.vault.join(gone)).is_err(),
            "{gone} must be deleted"
        );
    }
    assert!(s.vault.join("skills/alternate/SKILL.md").is_file());
    assert!(s.vault.join("dist/alternate/SKILL.md").is_file());
    let manifest = fs::read_to_string(s.vault.join("dist/.skillsbox-build.toml")).unwrap();
    assert!(!manifest.contains("api ="), "{manifest}");
    assert!(manifest.contains("alternate ="), "{manifest}");
    no_staging_left(&s);
    let members = json(&s, &["bundle", "show", "api-kit"]);
    assert_eq!(
        members["skills"],
        json!([{"id": "alternate", "role": "primary", "group": "contracts", "audit_level": "review"}])
    );
    let remaining = json(&s, &["list"]);
    assert_eq!(remaining.as_array().unwrap().len(), 1);
    assert_eq!(remaining[0]["id"], "alternate");
    // The index rows are gone too: search validates the cache against files.
    let found = json(&s, &["search", "contracts"]);
    assert_eq!(found["items"].as_array().unwrap().len(), 1);
    let (exit, code, _) = refused(&s, &["show", "api"]);
    assert_eq!(exit, 1, "{code}");
    assert!(json(&s, &["doctor"])["healthy"].as_bool().unwrap());

    json(&s, &["undo"]);
    let after: Vec<_> = ["skills", "meta", "bundles", "dist"]
        .iter()
        .map(|name| tree(&s.vault.join(name)))
        .collect();
    assert_eq!(after, before, "undo restores every byte, including dist");
    assert_eq!(json(&s, &["list"]), list);
    let restored = json(&s, &["show", "api"]);
    assert_eq!(restored, detail);
    assert_eq!(
        kinana_core::hash::hash_directory(&s.vault.join("skills/api")).unwrap(),
        restored["skill"]["content_hash"].as_str().unwrap()
    );
    assert_eq!(
        json(&s, &["search", "contracts"])["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(json(&s, &["doctor"])["healthy"].as_bool().unwrap());
    no_staging_left(&s);
}

#[test]
fn remove_refuses_a_skill_active_in_a_registered_project_and_leaves_everything_untouched() {
    let s = setup();
    let first = project(&s, "project-one");
    let second = project(&s, "project-two");
    for (path, agent) in [(&first, "claude"), (&second, "codex")] {
        json(
            &s,
            &[
                "use",
                "api",
                "--project",
                path.to_str().unwrap(),
                "--agent",
                agent,
            ],
        );
    }
    let before = everything(&s);
    for arguments in [vec!["remove", "api"], vec!["remove", "api", "--dry-run"]] {
        let (exit, code, message) = refused(&s, &arguments);
        assert_eq!((exit, code.as_str()), (2, "SKILL_IN_USE"), "{message}");
        assert!(message.contains(first.to_str().unwrap()), "{message}");
        assert!(message.contains(second.to_str().unwrap()), "{message}");
        assert_eq!(everything(&s), before);
    }
    // Another skill is not blocked by those activations.
    assert_eq!(
        json(&s, &["remove", "alternate", "--dry-run"])["changed"],
        true
    );
    // The user must unuse first; project files are only ever changed by unuse.
    json(&s, &["unuse", "api", "--project", first.to_str().unwrap()]);
    let (_, code, message) = refused(&s, &["remove", "api"]);
    assert_eq!(code, "SKILL_IN_USE");
    assert!(!message.contains(first.to_str().unwrap()), "{message}");
    json(&s, &["unuse", "api", "--project", second.to_str().unwrap()]);
    let projects = (tree(&first), tree(&second));
    assert_eq!(json(&s, &["remove", "api"])["committed"], true);
    assert_eq!((tree(&first), tree(&second)), projects);
}

#[test]
fn remove_refuses_when_a_bundle_would_become_invalid_and_keeps_an_emptied_bundle() {
    let s = setup();
    bundle(
        &s,
        &[
            ("api", "primary", Some("contracts")),
            ("alternate", "secondary", Some("contracts")),
        ],
    );
    let before = everything(&s);
    for arguments in [vec!["remove", "api"], vec!["remove", "api", "--dry-run"]] {
        let (exit, code, message) = refused(&s, &arguments);
        assert_eq!(
            (exit, code.as_str()),
            (2, "BUNDLE_WOULD_BE_INVALID"),
            "{message}"
        );
        assert!(message.contains("api-kit"), "{message}");
        assert_eq!(everything(&s), before);
    }
    // Removing the secondary member keeps the group valid.
    let report = json(&s, &["remove", "alternate"]);
    assert_eq!(report["bundles_changed"], json!(["api-kit"]));
    assert_eq!(
        json(&s, &["bundle", "show", "api-kit"])["skills"],
        json!([{"id": "api", "role": "primary", "group": "contracts", "audit_level": "review"}])
    );
    let after_first = tree(&s.vault.join("bundles"));
    // Existing validation accepts a bundle without members: it is kept, empty.
    let report = json(&s, &["remove", "api"]);
    assert_eq!(report["bundles_changed"], json!(["api-kit"]));
    assert_eq!(
        json(&s, &["bundle", "show", "api-kit"])["skills"],
        json!([])
    );
    assert_eq!(json(&s, &["list"]), json!([]));
    // undo reverts the latest commit only: the emptied bundle gets its member back.
    json(&s, &["undo"]);
    assert_eq!(tree(&s.vault.join("bundles")), after_first);
    assert_eq!(json(&s, &["list"]).as_array().unwrap().len(), 1);
}

#[test]
fn dry_run_for_remove_and_bundle_delete_writes_nothing_at_all() {
    let s = setup();
    bundle(
        &s,
        &[("api", "primary", None), ("alternate", "secondary", None)],
    );
    let before = everything(&s);
    assert_eq!(
        json(&s, &["remove", "api", "--dry-run"]),
        json!({
            "id": "api",
            "dry_run": true,
            "changed": true,
            "committed": false,
            "removed_paths": ["skills/api", "meta/api.toml", "dist/api"],
            "bundles_changed": ["api-kit"],
        })
    );
    assert_eq!(everything(&s), before);
    assert_eq!(
        json(&s, &["bundle", "delete", "api-kit", "--dry-run"]),
        json!({"name": "api-kit", "dry_run": true, "changed": true, "committed": false})
    );
    assert_eq!(everything(&s), before);
    // Text mode is a preview too.
    assert!(s.run(&["remove", "api", "--dry-run"]).status.success());
    assert!(
        s.run(&["bundle", "delete", "api-kit", "--dry-run"])
            .status
            .success()
    );
    assert_eq!(everything(&s), before);
}

#[test]
fn unknown_and_unsafe_ids_are_refused_without_touching_anything() {
    let s = setup();
    bundle(&s, &[("api", "primary", None)]);
    let outside = s.root.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("SKILL.md"), "owner data").unwrap();
    fs::write(s.root.join("outside.toml"), "owner data").unwrap();
    let before = everything(&s);

    let (exit, code, _) = refused(&s, &["remove", "absent"]);
    assert_eq!((exit, code.as_str()), (1, "SKILL_NOT_FOUND"));
    let (exit, code, _) = refused(&s, &["bundle", "delete", "absent"]);
    assert_eq!((exit, code.as_str()), (1, "BUNDLE_NOT_FOUND"));
    let absolute = outside.to_str().unwrap().to_owned();
    for id in [
        "../outside",
        "../../outside",
        "..",
        ".",
        "api/../alternate",
        "api/",
        "API",
        ".git",
        "../meta",
        absolute.as_str(),
    ] {
        for dry_run in [false, true] {
            let mut arguments = vec!["remove", id];
            if dry_run {
                arguments.push("--dry-run");
            }
            let (exit, code, _) = refused(&s, &arguments);
            assert_eq!((exit, code.as_str()), (1, "INVALID_SKILL_ID"), "{id}");
            let mut arguments = vec!["bundle", "delete", id];
            if dry_run {
                arguments.push("--dry-run");
            }
            let (exit, code, _) = refused(&s, &arguments);
            assert_eq!((exit, code.as_str()), (1, "INVALID_BUNDLE_NAME"), "{id}");
        }
    }
    // A bundle name is not a skill ID, and the reverse.
    assert_eq!(refused(&s, &["remove", "api-kit"]).1, "SKILL_NOT_FOUND");
    assert_eq!(
        refused(&s, &["bundle", "delete", "api"]).1,
        "BUNDLE_NOT_FOUND"
    );
    assert_eq!(everything(&s), before);
}

#[cfg(unix)]
#[test]
fn symlinked_skill_metadata_or_bundle_is_refused_without_deleting_outside_the_vault() {
    use std::os::unix::fs::symlink;
    let s = setup();
    let outside = s.root.join("outside");
    fs::create_dir_all(outside.join("nested")).unwrap();
    fs::write(
        outside.join("SKILL.md"),
        "---\nname: evil\ndescription: Outside\n---\nowner data\n",
    )
    .unwrap();
    fs::write(outside.join("nested/keep.txt"), "owner data").unwrap();
    let outside_meta = s.root.join("outside-meta.toml");
    fs::write(
        &outside_meta,
        fs::read_to_string(s.vault.join("meta/api.toml"))
            .unwrap()
            .replace("id = \"api\"", "id = \"evil\""),
    )
    .unwrap();
    let outside_bundle = s.root.join("outside-bundle.toml");
    fs::write(
        &outside_bundle,
        "name = \"evil\"\ntitle = \"x\"\nagents = [\"claude\"]\nskills = []\n",
    )
    .unwrap();
    let survivors = || {
        (
            tree(&outside),
            fs::read(&outside_meta).unwrap(),
            fs::read(&outside_bundle).unwrap(),
        )
    };
    let expected = survivors();

    // A whole skill directory that is a link out of the vault.
    symlink(&outside, s.vault.join("skills/evil")).unwrap();
    fs::copy(&outside_meta, s.vault.join("meta/evil.toml")).unwrap();
    for arguments in [vec!["remove", "evil"], vec!["remove", "evil", "--dry-run"]] {
        let (exit, code, message) = refused(&s, &arguments);
        assert_eq!((exit, code.as_str()), (1, "SYMLINK_PATH"), "{message}");
        assert!(
            fs::symlink_metadata(s.vault.join("skills/evil"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(survivors(), expected);
    }
    fs::remove_file(s.vault.join("skills/evil")).unwrap();
    fs::remove_file(s.vault.join("meta/evil.toml")).unwrap();

    // A real skill whose metadata file is a link out of the vault.
    let real_meta = fs::read(s.vault.join("meta/api.toml")).unwrap();
    fs::remove_file(s.vault.join("meta/api.toml")).unwrap();
    fs::write(&outside_meta, &real_meta).unwrap();
    symlink(&outside_meta, s.vault.join("meta/api.toml")).unwrap();
    let (_, code, message) = refused(&s, &["remove", "api"]);
    assert_eq!(code, "SYMLINK_PATH", "{message}");
    assert_eq!(fs::read(&outside_meta).unwrap(), real_meta);
    assert!(s.vault.join("skills/api/SKILL.md").is_file());
    fs::remove_file(s.vault.join("meta/api.toml")).unwrap();
    fs::write(s.vault.join("meta/api.toml"), &real_meta).unwrap();
    fs::write(&outside_meta, &expected.1).unwrap();

    // A link inside the skill tree is never followed or removed.
    symlink(&outside, s.vault.join("skills/api/assets/link")).unwrap();
    let (_, code, message) = refused(&s, &["remove", "api"]);
    assert_eq!(code, "SYMLINK_PATH", "{message}");
    assert!(s.vault.join("skills/api/SKILL.md").is_file());
    assert!(s.vault.join("meta/api.toml").is_file());
    assert_eq!(survivors(), expected);
    fs::remove_file(s.vault.join("skills/api/assets/link")).unwrap();

    // A bundle file that is a link out of the vault.
    symlink(&outside_bundle, s.vault.join("bundles/evil.toml")).unwrap();
    for arguments in [
        vec!["bundle", "delete", "evil"],
        vec!["bundle", "delete", "evil", "--dry-run"],
    ] {
        let (exit, code, message) = refused(&s, &arguments);
        assert_eq!((exit, code.as_str()), (1, "SYMLINK_PATH"), "{message}");
        assert!(
            fs::symlink_metadata(s.vault.join("bundles/evil.toml"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(survivors(), expected);
    }
    fs::remove_file(s.vault.join("bundles/evil.toml")).unwrap();

    // With the links gone the vault is clean again and removal works.
    assert_eq!(json(&s, &["remove", "api"])["committed"], true);
    assert_eq!(survivors(), expected);
}

#[test]
fn bundle_delete_then_undo_restores_the_exact_bundle_file() {
    let s = setup();
    bundle(
        &s,
        &[
            ("api", "primary", Some("contracts")),
            ("alternate", "secondary", Some("contracts")),
        ],
    );
    let bundles = tree(&s.vault.join("bundles"));
    let untouched: Vec<_> = ["skills", "meta", "dist"]
        .iter()
        .map(|name| tree(&s.vault.join(name)))
        .collect();
    let shown = json(&s, &["bundle", "show", "api-kit"]);
    let count = commits(&s.vault);

    assert_eq!(
        json(&s, &["bundle", "delete", "api-kit"]),
        json!({"name": "api-kit", "dry_run": false, "changed": true, "committed": true})
    );
    assert_eq!(commits(&s.vault), count + 1, "exactly one vault commit");
    assert_eq!(subject(&s.vault), "Delete bundle api-kit");
    assert!(git(&s.vault, &["status", "--porcelain"]).is_empty());
    assert!(fs::symlink_metadata(s.vault.join("bundles/api-kit.toml")).is_err());
    assert!(s.vault.join("bundles").is_dir());
    assert_eq!(json(&s, &["bundle", "list"]), json!([]));
    assert_eq!(
        refused(&s, &["bundle", "show", "api-kit"]).1,
        "BUNDLE_NOT_FOUND"
    );
    assert_eq!(
        refused(&s, &["bundle", "delete", "api-kit"]).1,
        "BUNDLE_NOT_FOUND"
    );
    let after: Vec<_> = ["skills", "meta", "dist"]
        .iter()
        .map(|name| tree(&s.vault.join(name)))
        .collect();
    assert_eq!(after, untouched, "deleting a bundle never deletes skills");
    assert_eq!(json(&s, &["list"]).as_array().unwrap().len(), 2);
    assert!(json(&s, &["doctor"])["healthy"].as_bool().unwrap());
    no_staging_left(&s);

    json(&s, &["undo"]);
    assert_eq!(tree(&s.vault.join("bundles")), bundles);
    assert_eq!(json(&s, &["bundle", "show", "api-kit"]), shown);
    assert!(json(&s, &["doctor"])["healthy"].as_bool().unwrap());
}

#[test]
fn bundle_delete_keeps_project_activations_and_removes_an_invalid_bundle_file() {
    let s = setup();
    bundle(&s, &[("api", "primary", None)]);
    let p = project(&s, "project");
    json(
        &s,
        &[
            "use",
            "api-kit",
            "--project",
            p.to_str().unwrap(),
            "--agent",
            "claude",
        ],
    );
    let before = tree(&p);
    assert_eq!(
        json(&s, &["bundle", "delete", "api-kit"])["committed"],
        true
    );
    assert_eq!(tree(&p), before, "the project and its lock are untouched");
    let status = json(&s, &["status"]);
    assert_eq!(status["projects"][0]["items"][0]["state"], "current");
    // The skill itself is still in use, so it still cannot be removed.
    assert_eq!(refused(&s, &["remove", "api"]).1, "SKILL_IN_USE");

    // A committed but unreadable bundle can only be repaired by deleting it.
    fs::write(s.vault.join("bundles/broken.toml"), "not = [valid").unwrap();
    kinana_core::gitlog::commit_paths(
        &s.vault,
        "Fixture: broken bundle",
        &["bundles/broken.toml".into()],
    )
    .unwrap();
    assert!(!s.run(&["bundle", "list", "--json"]).status.success());
    assert_eq!(json(&s, &["bundle", "delete", "broken"])["committed"], true);
    assert_eq!(json(&s, &["bundle", "list"]), json!([]));
}

#[test]
fn modified_generated_copy_and_dirty_vault_stop_removal_before_any_write() {
    let s = setup();
    fs::write(s.vault.join("dist/api/SKILL.md"), "owner edit").unwrap();
    let before = everything(&s);
    let (exit, code, message) = refused(&s, &["remove", "api"]);
    assert_eq!((exit, code.as_str()), (2, "BUILD_CONFLICT"), "{message}");
    assert_eq!(everything(&s), before);

    let s = setup();
    bundle(&s, &[("api", "primary", None)]);
    fs::write(s.vault.join("skills/alternate/notes.txt"), "owner edit").unwrap();
    let before = everything(&s);
    for arguments in [vec!["remove", "api"], vec!["bundle", "delete", "api-kit"]] {
        let (exit, code, message) = refused(&s, &arguments);
        assert_eq!((exit, code.as_str()), (2, "GIT_DIRTY_VAULT"), "{message}");
        assert_eq!(everything(&s), before);
    }
}

#[test]
fn removal_also_drops_an_owned_manual_copy_and_undo_keeps_the_vault_consistent() {
    let s = setup();
    bundle(
        &s,
        &[("api", "primary", None), ("alternate", "secondary", None)],
    );
    let p = project(&s, "project");
    json(
        &s,
        &[
            "use",
            "api-kit",
            "--include-secondary",
            "alternate",
            "--project",
            p.to_str().unwrap(),
            "--agent",
            "claude",
        ],
    );
    assert!(s.vault.join("dist/.manual/alternate/SKILL.md").is_file());
    json(
        &s,
        &[
            "unuse",
            "api",
            "alternate",
            "--project",
            p.to_str().unwrap(),
        ],
    );
    // unuse keeps the owned manual copy in the cache; removal must not orphan it.
    assert!(s.vault.join("dist/.manual/alternate/SKILL.md").is_file());
    let manifest = s.vault.join("dist/.manual/.skillsbox-manual.toml");
    assert!(fs::read_to_string(&manifest).unwrap().contains("alternate"));
    let report = json(&s, &["remove", "alternate"]);
    assert_eq!(
        report["removed_paths"],
        json!([
            "skills/alternate",
            "meta/alternate.toml",
            "dist/alternate",
            "dist/.manual/alternate"
        ])
    );
    assert!(fs::symlink_metadata(s.vault.join("dist/.manual/alternate")).is_err());
    assert!(!fs::read_to_string(&manifest).unwrap().contains("alternate"));
    no_staging_left(&s);
    json(&s, &["undo"]);
    assert!(s.vault.join("dist/alternate/SKILL.md").is_file());
    assert_eq!(json(&s, &["list"]).as_array().unwrap().len(), 2);
    // The manual copy is a cache: it is generated again by the next manual use.
    json(
        &s,
        &[
            "use",
            "api-kit",
            "--include-secondary",
            "alternate",
            "--project",
            p.to_str().unwrap(),
            "--agent",
            "claude",
        ],
    );
    assert!(s.vault.join("dist/.manual/alternate/SKILL.md").is_file());
    let status = json(&s, &["status"]);
    for item in status["projects"][0]["items"].as_array().unwrap() {
        assert_eq!(item["state"], "current", "{item}");
    }
    // A modified manual copy is preserved, and so is the skill.
    json(
        &s,
        &[
            "unuse",
            "api",
            "alternate",
            "--project",
            p.to_str().unwrap(),
        ],
    );
    fs::write(
        s.vault.join("dist/.manual/alternate/SKILL.md"),
        "owner edit",
    )
    .unwrap();
    let before = everything(&s);
    let (exit, code, message) = refused(&s, &["remove", "alternate"]);
    assert_eq!((exit, code.as_str()), (2, "BUILD_CONFLICT"), "{message}");
    assert_eq!(everything(&s), before);
}

#[cfg(unix)]
#[test]
fn failed_commit_restores_the_skill_its_metadata_bundles_and_generated_copy() {
    use std::os::unix::fs::PermissionsExt;
    let s = setup();
    bundle(
        &s,
        &[("api", "primary", None), ("alternate", "secondary", None)],
    );
    let before: Vec<_> = ["skills", "meta", "bundles", "dist"]
        .iter()
        .map(|name| tree(&s.vault.join(name)))
        .collect();
    let index = fs::read(s.home.join(".kinana/index-v2.db")).unwrap();
    let before_head = head(&s.vault);
    let bin = s.root.join("bin");
    fs::create_dir(&bin).unwrap();
    let fake = bin.join("git");
    fs::write(
        &fake,
        "#!/bin/sh\nfor arg do\n  if [ \"$arg\" = commit ]; then exit 97; fi\ndone\nexec /usr/bin/git \"$@\"\n",
    )
    .unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    for arguments in [vec!["remove", "api"], vec!["bundle", "delete", "api-kit"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_kinana"))
            .env("HOME", &s.home)
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .args(["--vault", s.vault.to_str().unwrap()])
            .args(&arguments)
            .arg("--json")
            .output()
            .unwrap();
        assert!(!output.status.success(), "{arguments:?}");
        let after: Vec<_> = ["skills", "meta", "bundles", "dist"]
            .iter()
            .map(|name| tree(&s.vault.join(name)))
            .collect();
        assert_eq!(after, before, "{arguments:?}");
        assert_eq!(fs::read(s.home.join(".kinana/index-v2.db")).unwrap(), index);
        assert_eq!(head(&s.vault), before_head);
        assert!(git(&s.vault, &["status", "--porcelain"]).is_empty());
        no_staging_left(&s);
    }
    assert_eq!(json(&s, &["remove", "api"])["committed"], true);
    assert_eq!(
        json(&s, &["bundle", "delete", "api-kit"])["committed"],
        true
    );
}

#[test]
fn undo_refuses_to_take_away_a_skill_that_is_active_in_a_registered_project() {
    // The latest vault commit is the import of `alternate`.
    let s = setup();
    let p = project(&s, "project");
    let path = p.to_str().unwrap();
    // An activation of another skill does not block undoing that import.
    json(&s, &["use", "api", "--project", path, "--agent", "claude"]);
    json(&s, &["undo", "--dry-run"]);

    json(
        &s,
        &["use", "alternate", "--project", path, "--agent", "claude"],
    );
    let before = everything(&s);
    for arguments in [vec!["undo"], vec!["undo", "--dry-run"]] {
        let (exit, code, message) = refused(&s, &arguments);
        assert_eq!((exit, code.as_str()), (2, "SKILL_IN_USE"), "{message}");
        assert!(message.contains("'alternate'"), "{message}");
        assert!(message.contains(path), "{message}");
        assert!(message.contains("unuse"), "{message}");
        assert_eq!(everything(&s), before, "a refused undo changes nothing");
    }
    assert!(
        fs::symlink_metadata(p.join(".claude/skills/alternate")).is_ok(),
        "the project's activation is intact"
    );

    // After unuse the same undo goes through and the vault stays consistent.
    json(&s, &["unuse", "alternate", "--project", path]);
    assert!(json(&s, &["undo"])["resulting_commit"].is_string());
    assert!(!s.vault.join("skills/alternate").exists());
    assert!(!s.vault.join("meta/alternate.toml").exists());
    assert!(json(&s, &["doctor"])["healthy"].as_bool().unwrap());
    assert_eq!(
        json(&s, &["status"])["projects"][0]["items"][0]["id"],
        "api"
    );
}

/// Review finding M1: `remove` deleted a `.git` directory the user had made
/// inside a skill, and `undo` could not bring it back. Content Git does not
/// keep now stops the removal, in a preview too, before anything changes.
#[test]
fn remove_refuses_content_that_undo_could_not_bring_back() {
    let s = setup();
    let skill = s.vault.join("skills/api");
    // A repository the user initialized inside the skill: Git never records it.
    fs::create_dir_all(skill.join(".git/refs")).unwrap();
    fs::write(skill.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    // An empty directory the metadata does not record (recorded ones such as
    // assets/empty/deep come back with undo and do not count).
    fs::create_dir(skill.join("notes")).unwrap();
    let before = everything(&s);
    for arguments in [vec!["remove", "api"], vec!["remove", "api", "--dry-run"]] {
        let (exit, code, message) = refused(&s, &arguments);
        assert_eq!(
            (exit, code.as_str()),
            (2, "SKILL_HAS_UNRECOVERABLE_CONTENT"),
            "{message}"
        );
        assert!(message.contains("skills/api/.git"), "{message}");
        assert!(message.contains("skills/api/notes/"), "{message}");
        assert!(!message.contains("assets/empty"), "{message}");
        assert_eq!(everything(&s), before);
    }
    no_staging_left(&s);
    // Once the user moves that content out, the removal goes ahead and undo
    // restores everything that was there, the recorded empty directory too.
    fs::remove_dir_all(skill.join(".git")).unwrap();
    fs::remove_dir(skill.join("notes")).unwrap();
    let kept = tree(&skill);
    assert_eq!(json(&s, &["remove", "api"])["committed"], true);
    assert!(!skill.exists());
    json(&s, &["undo"]);
    assert_eq!(tree(&skill), kept);
}

/// Review finding M2: a `remove` killed after its moves and before its
/// commit left a half-removed vault that `doctor` called healthy. Doctor now
/// reports it at both points a kill can leave something behind (before the
/// commit, and after it but before the cleanup), names the way back, and is
/// healthy again once the user has followed it.
#[test]
fn doctor_reports_a_removal_killed_before_or_after_its_commit_until_it_is_resolved() {
    use std::os::unix::fs::PermissionsExt;
    let doctor = |s: &Sandbox| -> (bool, String) {
        let output = s.run(&["doctor", "--json"]);
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        let check = value["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|check| check["name"] == "interrupted_change")
            .unwrap()
            .clone();
        assert_eq!(check["reason_code"], "DOCTOR_INTERRUPTED_CHANGE");
        (
            check["passed"].as_bool().unwrap(),
            check["message"].as_str().unwrap().to_owned(),
        )
    };
    let killed_remove = |s: &Sandbox, script: &str| {
        let bin = s.root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let fake = bin.join("git");
        fs::write(&fake, script).unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_kinana"))
            .env("HOME", &s.home)
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .args([
                "--vault",
                s.vault.to_str().unwrap(),
                "remove",
                "api",
                "--json",
            ])
            .output()
            .unwrap();
        // SIGKILL: no output, no exit code.
        assert_eq!(output.status.code(), None, "{output:?}");
        fs::remove_dir_all(&bin).unwrap();
    };
    let stages = |s: &Sandbox| -> Vec<PathBuf> {
        fs::read_dir(s.vault.join("dist"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".skillsbox-activation-")
            })
            .collect()
    };

    // 1. Killed when Git is asked to commit: everything moved, nothing committed.
    let s = setup();
    assert!(doctor(&s).0);
    let before_head = head(&s.vault);
    killed_remove(
        &s,
        "#!/bin/sh\nfor arg do\n  if [ \"$arg\" = commit ]; then kill -9 $PPID; exit 1; fi\ndone\nexec /usr/bin/git \"$@\"\n",
    );
    assert_eq!(head(&s.vault), before_head);
    assert!(!s.vault.join("skills/api").exists());
    let (passed, message) = doctor(&s);
    assert!(!passed, "{message}");
    assert!(message.contains("meta/api.toml"), "{message}");
    assert!(message.contains("skills/api/SKILL.md"), "{message}");
    assert!(message.contains("private stages"), "{message}");
    assert!(
        message.contains("empty directories") && message.contains("skills/api/assets/empty/deep"),
        "{message}"
    );
    no_command_printed(&message);
    // The user returns the named files with Git, recreates the directory
    // the message names, then follows the rest of it.
    git(
        &s.vault,
        &[
            "restore",
            "--source=HEAD",
            "--staged",
            "--worktree",
            "--",
            "meta/api.toml",
            "skills/api",
        ],
    );
    fs::create_dir_all(s.vault.join("skills/api/assets/empty/deep")).unwrap();
    json(&s, &["build"]);
    let (passed, message) = doctor(&s);
    assert!(!passed && message.contains("private stages"), "{message}");
    assert!(
        !message.contains("differs from the last commit"),
        "{message}"
    );
    for stage in stages(&s) {
        fs::remove_dir_all(stage).unwrap();
    }
    assert!(doctor(&s).0);
    assert!(s.vault.join("skills/api/SKILL.md").is_file());

    // 2. Killed right after the commit: the removal is recorded, but the
    // private stages and their moved files remain.
    killed_remove(
        &s,
        "#!/bin/sh\nfor arg do\n  if [ \"$arg\" = commit ]; then /usr/bin/git \"$@\"; kill -9 $PPID; exit 1; fi\ndone\nexec /usr/bin/git \"$@\"\n",
    );
    assert_ne!(head(&s.vault), before_head);
    assert_eq!(subject(&s.vault), "Remove skill api");
    let (passed, message) = doctor(&s);
    assert!(!passed, "{message}");
    assert!(message.contains("private stages"), "{message}");
    assert!(
        !message.contains("differs from the last commit"),
        "{message}"
    );
    assert!(!stages(&s).is_empty());
    for stage in stages(&s) {
        fs::remove_dir_all(stage).unwrap();
    }
    let (passed, message) = doctor(&s);
    assert!(passed, "{message}");
}

/// A vault that was never built (a fresh clone has an empty `dist/`, which
/// Git ignores) is not an interrupted change; generated output left without
/// its skill is.
#[test]
fn doctor_tells_an_unbuilt_vault_from_output_left_without_its_skill() {
    let s = setup();
    let check = |s: &Sandbox| -> (bool, String) {
        let output = s.run(&["doctor", "--json"]);
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        let check = value["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|check| check["name"] == "interrupted_change")
            .unwrap()
            .clone();
        (
            check["passed"].as_bool().unwrap(),
            check["message"].as_str().unwrap().to_owned(),
        )
    };
    for entry in fs::read_dir(s.vault.join("dist")).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            fs::remove_dir_all(path).unwrap();
        }
    }
    let (passed, message) = check(&s);
    assert!(passed, "{message}");
    json(&s, &["build"]);
    fs::create_dir(s.vault.join("dist/ghost")).unwrap();
    fs::write(s.vault.join("dist/ghost/SKILL.md"), "left behind\n").unwrap();
    let (passed, message) = check(&s);
    assert!(!passed, "{message}");
    assert!(
        message.contains("ghost (missing from skills, meta)"),
        "{message}"
    );
}

/// Second review, finding 1, and owner decision 2026-11-17 (sixth round):
/// `doctor` prints no shell command at all. Empty directory names from the
/// skill's author that look like shell are named as they are, and nothing
/// in the message can run.
#[test]
fn names_that_look_like_shell_are_listed_and_no_command_is_printed() {
    use std::os::unix::fs::PermissionsExt;
    let s = Sandbox::new();
    s.initialize();
    let source = s.root.join("source-hostile");
    let names = [
        "x\"; touch PWNED1; echo \"",
        "it's $(touch PWNED2)",
        "`touch PWNED3`",
    ];
    for name in names {
        fs::create_dir_all(source.join("assets").join(name)).unwrap();
    }
    fs::write(
        source.join("SKILL.md"),
        "---\nname: hostile\ndescription: Names that look like shell\n---\n# Body\n",
    )
    .unwrap();
    json(&s, &["add", source.to_str().unwrap()]);
    let bin = s.root.join("bin");
    fs::create_dir(&bin).unwrap();
    fs::write(
        bin.join("git"),
        "#!/bin/sh\nfor arg do\n  if [ \"$arg\" = commit ]; then kill -9 $PPID; exit 1; fi\ndone\nexec /usr/bin/git \"$@\"\n",
    )
    .unwrap();
    fs::set_permissions(bin.join("git"), fs::Permissions::from_mode(0o755)).unwrap();
    let killed = Command::new(env!("CARGO_BIN_EXE_kinana"))
        .env("HOME", &s.home)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .args(["--vault", s.vault.to_str().unwrap(), "remove", "hostile"])
        .output()
        .unwrap();
    assert_eq!(killed.status.code(), None);
    fs::remove_dir_all(&bin).unwrap();
    let (passed, message) = interrupted_check(&s);
    assert!(!passed, "{message}");
    no_command_printed(&message);
    for name in names {
        assert!(
            message.contains(&format!("skills/hostile/assets/{name}")),
            "{name} not named: {message}"
        );
    }
    for marker in ["PWNED1", "PWNED2", "PWNED3"] {
        let found = support::snapshot(&s.root).into_iter().any(|(path, _)| {
            path.to_string_lossy().contains(marker) && !path.to_string_lossy().contains("assets")
        });
        assert!(!found, "{marker} was created");
    }
}

/// Second review, finding 3: a file Git does not keep inside the skill is
/// refused with the documented code, not as a dirty vault.
#[test]
fn an_untracked_file_in_the_skill_is_refused_with_its_own_code() {
    let s = setup();
    fs::write(s.vault.join("skills/api/notes.txt"), "kept by the user\n").unwrap();
    let before = everything(&s);
    for arguments in [vec!["remove", "api"], vec!["remove", "api", "--dry-run"]] {
        let (exit, code, message) = refused(&s, &arguments);
        assert_eq!(
            (exit, code.as_str()),
            (2, "SKILL_HAS_UNRECOVERABLE_CONTENT"),
            "{message}"
        );
        assert!(message.contains("skills/api/notes.txt"), "{message}");
        assert_eq!(everything(&s), before);
    }
}

/// Owner decision 2026-11-17: Finder's `.DS_Store`, by name only and at any
/// depth, is not a change for doctor or the cleanliness checks, and `remove`
/// deletes it with the skill instead of refusing. Any other hidden file is
/// still content that undo could not bring back.
#[test]
fn finder_files_inside_a_skill_neither_alarm_doctor_nor_block_its_removal() {
    let s = setup();
    let skill = s.vault.join("skills/api");
    let interrupted = |s: &Sandbox| -> (bool, String) {
        let value: Value = serde_json::from_slice(&s.run(&["doctor", "--json"]).stdout).unwrap();
        let check = value["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|check| check["name"] == "interrupted_change")
            .unwrap()
            .clone();
        (
            check["passed"].as_bool().unwrap(),
            check["message"].as_str().unwrap().to_owned(),
        )
    };
    let kept = tree(&skill);
    let finder = [
        skill.join(".DS_Store"),
        skill.join("assets/.DS_Store"),
        skill.join("assets/empty/deep/.DS_Store"),
    ];
    for path in &finder {
        fs::write(path, [0, 0, 0, 1, b'B', b'u', b'd', b'1']).unwrap();
    }
    let (passed, message) = interrupted(&s);
    assert!(passed, "{message}");

    // Only the name counts: another hidden file is still refused.
    fs::write(skill.join(".notes"), "kept by the user\n").unwrap();
    let (exit, code, message) = refused(&s, &["remove", "api"]);
    assert_eq!(
        (exit, code.as_str()),
        (2, "SKILL_HAS_UNRECOVERABLE_CONTENT"),
        "{message}"
    );
    assert!(message.contains("skills/api/.notes"), "{message}");
    assert!(!message.contains(".DS_Store"), "{message}");
    fs::remove_file(skill.join(".notes")).unwrap();

    let commits_before = commits(&s.vault);
    assert_eq!(json(&s, &["remove", "api"])["committed"], true);
    assert!(!skill.exists(), "the Finder files went with the skill");
    assert_eq!(commits(&s.vault), commits_before + 1);
    let tracked = git(&s.vault, &["ls-files"]);
    assert!(!tracked.contains(".DS_Store"), "{tracked}");
    no_staging_left(&s);
    let (passed, message) = interrupted(&s);
    assert!(passed, "{message}");

    // Undo brings back exactly what Git recorded, without the Finder files.
    json(&s, &["undo"]);
    assert_eq!(tree(&skill), kept);
    let (passed, message) = interrupted(&s);
    assert!(passed, "{message}");
}

/// Owner decisions 2026-11-17: `doctor` names every file that differs from
/// the last commit, the one changed in place too, and suggests returning
/// those files only, never the whole vault; since the sixth round it prints
/// no command to copy.
#[test]
fn doctor_names_each_differing_file_and_prints_no_command() {
    let s = setup();
    fs::remove_file(s.vault.join("skills/api/SKILL.md")).unwrap();
    fs::write(s.vault.join("meta/alternate.toml"), b"changed in place\n").unwrap();
    fs::write(s.vault.join("bundles/staged.toml"), b"name = \"staged\"\n").unwrap();
    git(&s.vault, &["add", "--", "bundles/staged.toml"]);
    fs::write(s.vault.join("notes.txt"), b"the user's own file\n").unwrap();

    let (passed, message) = interrupted_check(&s);
    assert!(!passed, "{message}");
    for path in [
        "skills/api/SKILL.md",
        "meta/alternate.toml",
        "bundles/staged.toml",
        "notes.txt",
    ] {
        assert!(message.contains(path), "{path} not named: {message}");
    }
    assert!(message.contains("not the whole vault"), "{message}");
    assert!(message.contains("kinana doctor --repair"), "{message}");
    assert!(message.contains("move them out of the vault"), "{message}");
    no_command_printed(&message);
    assert!(!message.contains(s.vault.to_str().unwrap()), "{message}");
    assert_eq!(
        fs::read(s.vault.join("meta/alternate.toml")).unwrap(),
        b"changed in place\n"
    );
}

/// More than ten files differ: the message says how many it does not list,
/// and no longer tells the user to restore "the files named above".
#[test]
fn a_long_list_says_how_many_it_leaves_out() {
    let s = setup();
    for index in 0..12 {
        fs::write(s.vault.join(format!("bundles/b{index:02}.toml")), "x\n").unwrap();
    }
    git(&s.vault, &["add", "--", "bundles"]);
    let (passed, message) = interrupted_check(&s);
    assert!(!passed, "{message}");
    assert!(message.contains("and 2 more"), "{message}");
    assert!(!message.contains("named above"), "{message}");
}

/// Fourth review, medium finding: direction marks and line separators passed
/// the control check, and the text output showed another name. Every name in
/// doctor's output now shares one escape form: backslashes doubled, and
/// control, direction and separator characters written as `\u{..}`.
#[test]
fn direction_marks_and_line_separators_in_a_name_are_escaped() {
    let s = Sandbox::new();
    s.initialize();
    let source = s.root.join("source-bidi");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: bidi\ndescription: Names that reorder text\n---\n# Body\n",
    )
    .unwrap();
    let names = [
        ("a\u{202E}b.txt", "a\\u{202e}b.txt"),
        ("c\u{2066}d.txt", "c\\u{2066}d.txt"),
        ("e\u{2028}f.txt", "e\\u{2028}f.txt"),
        ("g\u{200f}h.txt", "g\\u{200f}h.txt"),
        ("i\\u{1b}j.txt", "i\\\\u{1b}j.txt"),
    ];
    for (name, _) in names {
        fs::write(source.join(name), "x").unwrap();
    }
    fs::write(source.join("a b.txt"), "the look-alike").unwrap();
    json(&s, &["add", source.to_str().unwrap()]);
    for (name, _) in names {
        fs::remove_file(s.vault.join("skills/bidi").join(name)).unwrap();
    }
    let (passed, message) = interrupted_check(&s);
    assert!(!passed);
    for (_, shown) in names {
        assert!(message.contains(shown), "{shown} not in {message:?}");
    }
    assert!(!message.contains("a b.txt"), "{message}");
    for character in ['\u{202E}', '\u{2066}', '\u{2028}', '\u{200f}'] {
        assert!(!message.contains(character), "{message:?}");
    }
    let text = String::from_utf8(s.run(&["doctor"]).stdout).unwrap();
    for character in ['\u{202E}', '\u{2066}', '\u{2028}', '\u{200f}', '\u{1b}'] {
        assert!(!text.contains(character), "{text:?}");
    }
    assert!(text.contains("a\\u{202e}b.txt"), "{text}");
}

fn interrupted_check(s: &Sandbox) -> (bool, String) {
    let value: Value = serde_json::from_slice(&s.run(&["doctor", "--json"]).stdout).unwrap();
    let check = value["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["name"] == "interrupted_change")
        .unwrap()
        .clone();
    (
        check["passed"].as_bool().unwrap(),
        check["message"].as_str().unwrap().to_owned(),
    )
}

/// Owner decision 2026-11-17, sixth round: nothing in the message is a
/// command to copy.
fn no_command_printed(message: &str) {
    for command in ["git ", "git --", "restore --", "mkdir", " && ", " ; "] {
        assert!(!message.contains(command), "{command:?} in {message}");
    }
}

/// Third review, finding 1: a file name from a skill's author holding
/// terminal escapes could redraw doctor's line on screen. Listed names show
/// control characters as visible escapes and the text output carries none.
#[test]
fn a_name_with_terminal_escapes_cannot_redraw_the_doctor_line() {
    let s = Sandbox::new();
    s.initialize();
    let source = s.root.join("source-escapes");
    fs::create_dir_all(&source).unwrap();
    let evil = "a\u{1b}[2K\rFAKE";
    fs::write(
        source.join("SKILL.md"),
        "---\nname: escapes\ndescription: Names with terminal escapes\n---\n# Body\n",
    )
    .unwrap();
    fs::write(source.join(evil), "x").unwrap();
    fs::write(source.join("plain.txt"), "y").unwrap();
    json(&s, &["add", source.to_str().unwrap()]);
    fs::remove_file(s.vault.join("skills/escapes").join(evil)).unwrap();
    fs::remove_file(s.vault.join("skills/escapes/plain.txt")).unwrap();

    let (passed, message) = interrupted_check(&s);
    assert!(!passed);
    assert!(!message.chars().any(char::is_control), "{message:?}");
    assert!(message.contains("a\\u{1b}[2K\\u{d}FAKE"), "{message}");
    assert!(message.contains("skills/escapes/plain.txt"), "{message}");
    no_command_printed(&message);
    let text = s.run(&["doctor"]);
    assert!(
        !text.stdout.contains(&0x1b),
        "{:?}",
        String::from_utf8_lossy(&text.stdout)
    );
}

/// Third review, finding 2: a tracked file replaced by a directory, or a
/// tracked directory replaced by a file, is named as something to move out
/// first, because returning it to the last commit would delete it.
#[test]
fn what_replaced_a_tracked_path_is_named_to_move_out_first() {
    let s = setup();
    let skill = s.vault.join("skills/api");
    fs::remove_file(skill.join("SKILL.md")).unwrap();
    fs::create_dir(skill.join("SKILL.md")).unwrap();
    fs::write(skill.join("SKILL.md/notes.txt"), "precious\n").unwrap();
    fs::remove_dir_all(skill.join("assets")).unwrap();
    fs::write(skill.join("assets"), "a file where a folder was\n").unwrap();
    fs::write(s.vault.join("meta/alternate.toml"), "changed\n").unwrap();

    let (passed, message) = interrupted_check(&s);
    assert!(!passed);
    assert!(!message.contains("could not be read"), "{message}");
    assert!(message.contains("would delete it"), "{message}");
    assert!(message.contains("meta/alternate.toml"), "{message}");
    let blocked = message.split("would delete it: ").nth(1).unwrap();
    assert!(
        blocked.contains("skills/api/SKILL.md") && blocked.contains("skills/api/assets"),
        "{blocked}"
    );
    no_command_printed(&message);
    assert_eq!(
        fs::read_to_string(skill.join("SKILL.md/notes.txt")).unwrap(),
        "precious\n"
    );
    assert_eq!(
        fs::read_to_string(skill.join("assets")).unwrap(),
        "a file where a folder was\n"
    );
}

/// Third review, finding 3: a staged new file changed after staging is
/// named, and the message says it stays where it is.
#[test]
fn a_staged_new_file_changed_after_staging_is_named_and_kept() {
    let s = setup();
    let path = s.vault.join("bundles/staged.toml");
    fs::write(&path, "name = \"one\"\n").unwrap();
    git(&s.vault, &["add", "--", "bundles/staged.toml"]);
    fs::write(&path, "name = \"two\"\n").unwrap();
    let (passed, message) = interrupted_check(&s);
    assert!(!passed);
    assert!(message.contains("bundles/staged.toml"), "{message}");
    assert!(message.contains("stay where they are"), "{message}");
    no_command_printed(&message);
    assert_eq!(fs::read_to_string(&path).unwrap(), "name = \"two\"\n");
}
