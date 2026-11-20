#[allow(dead_code)]
mod support;
use serde_json::Value;
use std::{fs, path::Path, process::Command};
use support::{Sandbox, snapshot as tree};

fn json(s: &Sandbox, args: &[&str]) -> Value {
    let output = s.run(&[args, &["--json"]].concat());
    assert!(
        output.status.success(),
        "{:?}: {}",
        args,
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn seed(s: &Sandbox, name: &str) {
    let source = s.root.join(name);
    fs::create_dir(&source).unwrap();
    fs::write(source.join("SKILL.md"),format!("---\nname: {name}\ndescription: Review API contracts\n---\n# API review\nReview request and response contracts before implementation.\n")).unwrap();
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
#[test]
fn favorite_roundtrip_is_metadata_only_idempotent_and_undoable() {
    let s = Sandbox::new();
    s.initialize();
    seed(&s, "api");
    json(&s, &["classify", "api", "--agent", "keywords"]);
    let skills = tree(&s.vault.join("skills"));
    let dist = tree(&s.vault.join("dist"));
    let db = fs::read(s.home.join(".kinana/index-v2.db")).unwrap();
    let path = s.vault.join("meta/api.toml");
    let before: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let first = head(&s.vault);
    let changed = json(&s, &["pin", "api"]);
    assert_eq!(
        changed,
        serde_json::json!({"id":"api","pinned":true,"dry_run":false,"changed":true,"committed":true})
    );
    let pinned_head = head(&s.vault);
    assert_ne!(first, pinned_head);
    let mut after: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    after["pinned"] = before["pinned"].clone();
    assert_eq!(after, before);
    assert_eq!(json(&s, &["list"])[0]["pinned"], true);
    assert_eq!(json(&s, &["pin", "api"])["changed"], false);
    assert_eq!(head(&s.vault), pinned_head);
    assert_eq!(tree(&s.vault.join("skills")), skills);
    assert_eq!(tree(&s.vault.join("dist")), dist);
    assert_eq!(fs::read(s.home.join(".kinana/index-v2.db")).unwrap(), db);
    assert_eq!(
        json(&s, &["search", "contracts"])["items"][0]["pinned"],
        true
    );
    let snapshot = tree(&s.root);
    assert_eq!(
        json(&s, &["pin", "api", "--unset", "--dry-run"])["changed"],
        true
    );
    assert_eq!(tree(&s.root), snapshot);
    json(&s, &["undo"]);
    assert_eq!(json(&s, &["list"])[0]["pinned"], false);
    assert_eq!(tree(&s.vault.join("skills")), skills);
    assert_eq!(tree(&s.vault.join("dist")), dist);
    json(&s, &["pin", "api"]);
    json(&s, &["pin", "api", "--unset"]);
    let unset_head = head(&s.vault);
    assert_eq!(json(&s, &["pin", "api", "--unset"])["committed"], false);
    assert_eq!(head(&s.vault), unset_head);
}
#[test]
fn legacy_absence_is_false_without_writes_and_favorites_sort_then_id() {
    let s = Sandbox::new();
    s.initialize();
    seed(&s, "a");
    seed(&s, "z");
    seed(&s, "m");
    let p = s.vault.join("meta/a.toml");
    let legacy = fs::read_to_string(&p)
        .unwrap()
        .replace("pinned = false\n", "");
    fs::write(&p, legacy).unwrap();
    // Commit this old-format fixture before invoking a mutating command.
    kinana_core::gitlog::commit_paths(
        &s.vault,
        "Fixture: legacy metadata",
        &["meta/a.toml".into()],
    )
    .unwrap();
    let before = tree(&s.root);
    assert_eq!(json(&s, &["list"])[0]["pinned"], false);
    assert_eq!(json(&s, &["pin", "a", "--unset"])["changed"], false);
    assert_eq!(tree(&s.root), before);
    json(&s, &["pin", "z"]);
    json(&s, &["pin", "m"]);
    let ids = |v: Value| {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x["id"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(json(&s, &["list"])), ["m", "z", "a"]);
    assert_eq!(
        ids(json(
            &s,
            &[
                "list",
                "--pinned",
                "--category",
                "other",
                "--audit",
                "clean",
                "--agent",
                "codex"
            ]
        )),
        ["m", "z"]
    );
    assert_eq!(
        json(&s, &["list", "--pinned", "--category", "security"]),
        serde_json::json!([])
    );
}
#[test]
fn favorite_rejects_dirty_or_foreign_metadata_without_payload_changes() {
    let s = Sandbox::new();
    s.initialize();
    seed(&s, "api");
    let file = s.vault.join("meta/api.toml");
    fs::write(
        &file,
        format!("{}\n# owner change\n", fs::read_to_string(&file).unwrap()),
    )
    .unwrap();
    let before = tree(&s.root);
    let output = s.run(&["pin", "api", "--json"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(tree(&s.root), before);
    #[cfg(unix)]
    {
        fs::remove_file(&file).unwrap();
        let outside = s.root.join("outside.toml");
        fs::write(&outside, "owner").unwrap();
        std::os::unix::fs::symlink(&outside, &file).unwrap();
        assert!(!s.run(&["pin", "api", "--json"]).status.success());
        assert_eq!(fs::read_to_string(outside).unwrap(), "owner");
    }
}
#[cfg(unix)]
#[test]
fn failed_commit_restores_metadata_and_preserves_unknown_fields_and_payload() {
    use std::os::unix::fs::PermissionsExt;
    let s = Sandbox::new();
    s.initialize();
    seed(&s, "api");
    let file = s.vault.join("meta/api.toml");
    fs::write(
        &file,
        format!(
            "{}\n[future]\nowner = 'preserve me'\n",
            fs::read_to_string(&file).unwrap()
        ),
    )
    .unwrap();
    kinana_core::gitlog::commit_paths(&s.vault, "Fixture: future field", &["meta/api.toml".into()])
        .unwrap();
    let metadata = fs::read(&file).unwrap();
    let skills = tree(&s.vault.join("skills"));
    let dist = tree(&s.vault.join("dist"));
    let index = fs::read(s.home.join(".kinana/index-v2.db")).unwrap();
    let before_head = head(&s.vault);
    let bin = s.root.join("bin");
    fs::create_dir(&bin).unwrap();
    let git = bin.join("git");
    fs::write(&git, "#!/bin/sh\nfor arg do\n  if [ \"$arg\" = commit ]; then exit 97; fi\ndone\nexec /usr/bin/git \"$@\"\n").unwrap();
    fs::set_permissions(&git, fs::Permissions::from_mode(0o755)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_kinana"))
        .env("HOME", &s.home)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .args(["--vault", s.vault.to_str().unwrap(), "pin", "api", "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(fs::read(&file).unwrap(), metadata);
    assert_eq!(tree(&s.vault.join("skills")), skills);
    assert_eq!(tree(&s.vault.join("dist")), dist);
    assert_eq!(fs::read(s.home.join(".kinana/index-v2.db")).unwrap(), index);
    assert_eq!(head(&s.vault), before_head);
    assert!(fs::read_dir(s.vault.join(".git")).unwrap().all(|e| {
        !e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".skillsbox-meta-")
    }));
    assert!(json(&s, &["pin", "api"])["committed"].as_bool().unwrap());
    let parsed: toml::Value = toml::from_str(&fs::read_to_string(file).unwrap()).unwrap();
    assert_eq!(parsed["future"]["owner"].as_str(), Some("preserve me"));
}

#[cfg(unix)]
#[test]
fn favorite_never_executes_git_from_a_recorded_untrusted_source() {
    use std::os::unix::fs::PermissionsExt;
    let s = Sandbox::new();
    s.initialize();
    seed(&s, "api");
    let source = s.root.join("api");
    let git = source.join("git");
    let marker = s.root.join("must-not-run");
    fs::write(
        &git,
        format!("#!/bin/sh\ntouch '{}'\nexit 1\n", marker.display()),
    )
    .unwrap();
    fs::set_permissions(&git, fs::Permissions::from_mode(0o755)).unwrap();
    let before = fs::read(s.vault.join("meta/api.toml")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_kinana"))
        .env("HOME", &s.home)
        .env("PATH", format!("{}:/usr/bin:/bin", source.display()))
        .args(["--vault", s.vault.to_str().unwrap(), "pin", "api", "--json"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(!marker.exists());
    assert_eq!(fs::read(s.vault.join("meta/api.toml")).unwrap(), before);
}
