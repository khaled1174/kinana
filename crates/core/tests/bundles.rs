mod support;

use kinana_core::{Bundle, BundleSkill, ImportOptions, bundles, gitlog, import, vault::Vault};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let root = support::temp_dir().unwrap();
        Self(root)
    }

    fn vault(&self) -> Vault {
        let vault = Vault::new(self.0.join("vault"), self.0.join("cache/index.db"));
        vault.init(false).unwrap();
        for id in ["api", "frontend"] {
            let source = self.0.join(format!("source-{id}"));
            fs::create_dir_all(source.join("assets/empty/deep")).unwrap();
            fs::write(
                source.join("SKILL.md"),
                format!("---\r\nname: {id}\r\ndescription: Review API contracts\r\n---\r\nExact source bytes مرحبا\r\n"),
            )
            .unwrap();
            fs::write(source.join("assets/bytes.bin"), [0, 1, 255, 13, 10]).unwrap();
            import::add(
                &vault,
                source.to_str().unwrap(),
                &self.0.join("state"),
                &ImportOptions::default(),
            )
            .unwrap();
        }
        vault
    }

    fn input(&self, bundle: &Bundle) -> PathBuf {
        let path = self.0.join("input.toml");
        fs::write(&path, toml::to_string_pretty(bundle).unwrap()).unwrap();
        path
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _cleanup = fs::remove_dir_all(&self.0);
    }
}

fn sample(name: &str) -> Bundle {
    Bundle {
        name: name.into(),
        title: "تطبيق ويب".into(),
        agents: vec!["claude".into(), "codex".into()],
        skills: vec![
            BundleSkill {
                id: "frontend".into(),
                role: "primary".into(),
                group: Some("review".into()),
            },
            BundleSkill {
                id: "api".into(),
                role: "secondary".into(),
                group: Some("review".into()),
            },
        ],
    }
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, directory: &Path, entries: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for child in fs::read_dir(directory).unwrap() {
            let path = child.unwrap().path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            let metadata = fs::symlink_metadata(&path).unwrap();
            if metadata.file_type().is_symlink() {
                entries.insert(
                    relative,
                    fs::read_link(&path)
                        .unwrap()
                        .as_os_str()
                        .as_encoded_bytes()
                        .to_vec(),
                );
            } else if metadata.is_dir() {
                entries.insert(relative, b"directory".to_vec());
                walk(root, &path, entries);
            } else if metadata.is_file() {
                entries.insert(relative, fs::read(&path).unwrap());
            } else {
                entries.insert(relative, b"special file".to_vec());
            }
        }
    }
    let mut entries = BTreeMap::new();
    walk(root, root, &mut entries);
    entries
}

fn without_git(mut entries: BTreeMap<PathBuf, Vec<u8>>) -> BTreeMap<PathBuf, Vec<u8>> {
    entries.retain(|path, _| !path.starts_with("vault/.git"));
    entries
}

fn git(vault: &Vault, args: &[&str]) -> Output {
    let output = Command::new("git")
        .arg("-C")
        .arg(&vault.root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
fn lifecycle_preserves_membership_order_sources_and_commits_only_the_bundle_path() {
    let temp = Temp::new();
    let vault = temp.vault();
    let expected = sample("web-app");
    let input = temp.input(&expected);
    let input_before = fs::read(&input).unwrap();
    let sources = snapshot(&vault.root.join("skills"));
    let dist = snapshot(&vault.root.join("dist"));
    let head = gitlog::head(&vault.root).unwrap();
    let created = bundles::create(&vault, &input, false).unwrap();
    assert_eq!(created.operation, "create");
    assert!(created.changed && created.committed && !created.dry_run);
    assert_eq!(created.path, vault.root.join("bundles/web-app.toml"));
    assert_eq!(created.bundle, expected);
    assert!(created.warnings.is_empty());
    assert_eq!(fs::read(&input).unwrap(), input_before);
    assert_ne!(gitlog::head(&vault.root).unwrap(), head);
    assert_eq!(
        git(&vault, &["log", "-1", "--format=%s"]).stdout,
        b"Create bundle web-app\n"
    );
    assert_eq!(
        git(
            &vault,
            &["diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"]
        )
        .stdout,
        b"bundles/web-app.toml\n"
    );
    assert_eq!(bundles::read(&vault, "web-app").unwrap(), expected);
    assert_eq!(bundles::list(&vault).unwrap(), vec![expected.clone()]);

    let mut updated = expected;
    updated.title = "Updated bundle".into();
    updated.skills.swap(0, 1);
    let input = temp.input(&updated);
    let edited = bundles::edit(&vault, "web-app", &input, false).unwrap();
    assert_eq!(edited.operation, "edit");
    assert!(edited.changed && edited.committed);
    assert_eq!(bundles::read(&vault, "web-app").unwrap(), updated);
    assert_eq!(
        git(&vault, &["log", "-1", "--format=%s"]).stdout,
        b"Edit bundle web-app\n"
    );
    assert_eq!(
        git(
            &vault,
            &["diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"]
        )
        .stdout,
        b"bundles/web-app.toml\n"
    );
    assert_eq!(snapshot(&vault.root.join("skills")), sources);
    assert_eq!(snapshot(&vault.root.join("dist")), dist);
    assert_eq!(
        fs::read(&input).unwrap(),
        toml::to_string_pretty(&updated).unwrap().into_bytes()
    );
    assert_eq!(
        input_before,
        toml::to_string_pretty(&sample("web-app"))
            .unwrap()
            .into_bytes()
    );
    gitlog::ensure_clean(&vault.root).unwrap();
}

#[test]
fn dry_runs_create_and_edit_with_zero_writes_and_same_semantic_edit_preserves_bytes() {
    let temp = Temp::new();
    let vault = temp.vault();
    let bundle = sample("web-app");
    let input = temp.input(&bundle);
    let before = snapshot(&temp.0);
    let report = bundles::create(&vault, &input, true).unwrap();
    assert!(report.dry_run && report.changed && !report.committed);
    assert_eq!(snapshot(&temp.0), before);
    bundles::create(&vault, &input, false).unwrap();
    let target = vault.root.join("bundles/web-app.toml");
    let saved = format!(
        "# Owner formatting stays exactly\r\n{}",
        fs::read_to_string(&target).unwrap()
    );
    fs::write(&target, &saved).unwrap();
    gitlog::commit_paths(
        &vault.root,
        "Keep owner bundle formatting",
        &[PathBuf::from("bundles/web-app.toml")],
    )
    .unwrap();
    let head = gitlog::head(&vault.root).unwrap();
    let before = snapshot(&temp.0);
    let report = bundles::edit(&vault, "web-app", &input, false).unwrap();
    assert!(!report.changed && !report.committed);
    assert_eq!(fs::read(&target).unwrap(), saved.as_bytes());
    assert_eq!(gitlog::head(&vault.root).unwrap(), head);
    assert_eq!(snapshot(&temp.0), before);
    let mut changed = bundle;
    changed.title = "New title".into();
    let input = temp.input(&changed);
    let before = snapshot(&temp.0);
    let report = bundles::edit(&vault, "web-app", &input, true).unwrap();
    assert!(report.dry_run && report.changed && !report.committed);
    assert_eq!(snapshot(&temp.0), before);
}

#[test]
fn read_and_list_are_sorted_read_only_and_work_with_missing_or_invalid_cache() {
    let temp = Temp::new();
    let vault = temp.vault();
    for name in ["z-last", "a-first"] {
        bundles::create(&vault, &temp.input(&sample(name)), false).unwrap();
    }
    fs::remove_file(&vault.index_path).unwrap();
    for corrupt in [false, true] {
        if corrupt {
            fs::write(&vault.index_path, b"foreign cache bytes").unwrap();
        }
        let before = snapshot(&temp.0);
        assert_eq!(bundles::read(&vault, "a-first").unwrap(), sample("a-first"));
        let names: Vec<_> = bundles::list(&vault)
            .unwrap()
            .into_iter()
            .map(|bundle| bundle.name)
            .collect();
        assert_eq!(names, ["a-first", "z-last"]);
        assert_eq!(snapshot(&temp.0), before);
    }
}

#[test]
fn validation_rejects_invalid_definitions_before_any_writes() {
    let temp = Temp::new();
    let vault = temp.vault();
    let valid = sample("web-app");
    let mut invalid = Vec::new();
    for name in [
        "",
        "../outside",
        "bad/name",
        "bad\\name",
        "UPPER",
        ".hidden",
    ] {
        let mut bundle = valid.clone();
        bundle.name = name.into();
        invalid.push((bundle, "INVALID_BUNDLE_NAME"));
    }
    let mut bundle = valid.clone();
    bundle.title = " \n\t".into();
    invalid.push((bundle, "INVALID_BUNDLE"));
    for agents in [
        vec![],
        vec!["claude", "claude"],
        vec!["unknown"],
        vec!["Claude"],
    ] {
        let mut bundle = valid.clone();
        bundle.agents = agents.into_iter().map(str::to_owned).collect();
        invalid.push((bundle, "INVALID_BUNDLE"));
    }
    let mut bundle = valid.clone();
    bundle.skills[1].id = "frontend".into();
    invalid.push((bundle, "INVALID_BUNDLE"));
    let mut bundle = valid.clone();
    bundle.skills[1].id = "missing".into();
    invalid.push((bundle, "SKILL_NOT_FOUND"));
    let mut bundle = valid.clone();
    bundle.skills[1].id = "../api".into();
    invalid.push((bundle, "INVALID_SKILL_ID"));
    let mut bundle = valid.clone();
    bundle.skills[1].role = "fallback".into();
    invalid.push((bundle, "INVALID_BUNDLE"));
    let mut bundle = valid.clone();
    bundle.skills[0].role = "secondary".into();
    invalid.push((bundle, "INVALID_BUNDLE"));
    let mut bundle = valid.clone();
    bundle.skills[1].role = "primary".into();
    invalid.push((bundle, "INVALID_BUNDLE"));
    for group in ["", "../group", "bad/group", "group name"] {
        let mut bundle = valid.clone();
        bundle.skills[0].group = Some(group.into());
        invalid.push((bundle, "INVALID_BUNDLE"));
    }
    for (bundle, code) in invalid {
        let input = temp.input(&bundle);
        let before = snapshot(&temp.0);
        assert_eq!(bundles::validate(&vault, &bundle).unwrap_err().code, code);
        assert_eq!(
            bundles::create(&vault, &input, false).unwrap_err().code,
            code
        );
        assert_eq!(snapshot(&temp.0), before);
    }
    let mut singleton = valid.clone();
    singleton.skills.truncate(1);
    bundles::validate(&vault, &singleton).unwrap();
    let mut empty = valid;
    empty.skills.clear();
    bundles::validate(&vault, &empty).unwrap();
}

#[test]
fn strict_input_fields_names_and_existing_targets_are_preserved_on_errors() {
    let temp = Temp::new();
    let vault = temp.vault();
    let bundle = sample("web-app");
    let target = vault.root.join("bundles/web-app.toml");
    let input = temp.input(&bundle);
    for text in [
        format!("unknown = true\n{}", toml::to_string(&bundle).unwrap()),
        toml::to_string(&bundle).unwrap().replace(
            "role = \"secondary\"",
            "role = \"secondary\"\nunknown = true",
        ),
    ] {
        fs::write(&input, text).unwrap();
        let before = snapshot(&temp.0);
        assert_eq!(
            bundles::create(&vault, &input, false).unwrap_err().code,
            "INVALID_TOML"
        );
        assert_eq!(snapshot(&temp.0), before);
    }
    let input = temp.input(&bundle);
    fs::write(&target, b"foreign invalid TOML\r\n").unwrap();
    let before = snapshot(&temp.0);
    let error = bundles::create(&vault, &input, false).unwrap_err();
    assert_eq!(error.code, "BUNDLE_EXISTS");
    assert!(error.needs_decision);
    assert_eq!(snapshot(&temp.0), before);
    assert_eq!(
        bundles::edit(&vault, "web-app", &input, false)
            .unwrap_err()
            .code,
        "INVALID_TOML"
    );
    assert_eq!(snapshot(&temp.0), before);
    fs::remove_file(&target).unwrap();
    assert_eq!(
        bundles::read(&vault, "web-app").unwrap_err().code,
        "BUNDLE_NOT_FOUND"
    );
    assert_eq!(
        bundles::edit(&vault, "web-app", &input, false)
            .unwrap_err()
            .code,
        "BUNDLE_NOT_FOUND"
    );
    let renamed = temp.input(&sample("other"));
    assert_eq!(
        bundles::edit(&vault, "web-app", &renamed, false)
            .unwrap_err()
            .code,
        "INVALID_BUNDLE"
    );
}

#[test]
fn dirty_vault_and_missing_index_preflight_leave_all_files_untouched() {
    let temp = Temp::new();
    let vault = temp.vault();
    let input = temp.input(&sample("web-app"));
    fs::write(vault.root.join("skills/api/owner.txt"), "owner data").unwrap();
    let before = snapshot(&temp.0);
    assert_eq!(
        bundles::create(&vault, &input, false).unwrap_err().code,
        "GIT_DIRTY_VAULT"
    );
    assert_eq!(snapshot(&temp.0), before);
    fs::remove_file(vault.root.join("skills/api/owner.txt")).unwrap();
    fs::remove_file(&vault.index_path).unwrap();
    let before = snapshot(&temp.0);
    assert_eq!(
        bundles::create(&vault, &input, false).unwrap_err().code,
        "INDEX_MISSING"
    );
    assert_eq!(snapshot(&temp.0), before);
}

#[test]
fn edit_can_repair_an_existing_bundle_that_references_a_missing_skill() {
    let temp = Temp::new();
    let vault = temp.vault();
    let mut stale = sample("web-app");
    stale.skills[1].id = "removed-skill".into();
    let target = vault.root.join("bundles/web-app.toml");
    fs::write(&target, toml::to_string_pretty(&stale).unwrap()).unwrap();
    gitlog::commit_paths(
        &vault.root,
        "Fixture: existing bundle references a removed skill",
        &[PathBuf::from("bundles/web-app.toml")],
    )
    .unwrap();
    assert_eq!(
        bundles::read(&vault, "web-app").unwrap_err().code,
        "SKILL_NOT_FOUND"
    );
    let replacement = sample("web-app");
    let report = bundles::edit(&vault, "web-app", &temp.input(&replacement), false).unwrap();
    assert!(report.changed && report.committed);
    assert_eq!(bundles::read(&vault, "web-app").unwrap(), replacement);
    gitlog::ensure_clean(&vault.root).unwrap();
}

#[cfg(unix)]
#[test]
fn linked_input_target_and_parent_and_special_files_are_never_followed_or_opened() {
    use std::os::unix::{fs::symlink, net::UnixListener};
    let temp = Temp::new();
    let vault = temp.vault();
    let input = temp.input(&sample("web-app"));
    let link = temp.0.join("linked-input.toml");
    symlink(&input, &link).unwrap();
    let before = snapshot(&temp.0);
    assert_eq!(
        bundles::create(&vault, &link, false).unwrap_err().code,
        "SYMLINK_PATH"
    );
    assert_eq!(snapshot(&temp.0), before);
    let target = vault.root.join("bundles/web-app.toml");
    symlink(&input, &target).unwrap();
    let before = snapshot(&temp.0);
    assert_eq!(
        bundles::create(&vault, &input, false).unwrap_err().code,
        "SYMLINK_PATH"
    );
    assert_eq!(
        bundles::read(&vault, "web-app").unwrap_err().code,
        "SYMLINK_PATH"
    );
    assert_eq!(bundles::list(&vault).unwrap_err().code, "SYMLINK_PATH");
    assert_eq!(snapshot(&temp.0), before);
    fs::remove_file(&target).unwrap();
    let socket = temp.0.join("socket.toml");
    let _listener = UnixListener::bind(&socket).unwrap();
    let before = snapshot(&temp.0);
    assert_eq!(
        bundles::create(&vault, &socket, false).unwrap_err().code,
        "INVALID_BUNDLE"
    );
    assert_eq!(snapshot(&temp.0), before);
    fs::rename(vault.root.join("bundles"), temp.0.join("outside-bundles")).unwrap();
    symlink(temp.0.join("outside-bundles"), vault.root.join("bundles")).unwrap();
    let before = snapshot(&temp.0);
    assert_eq!(
        bundles::create(&vault, &input, false).unwrap_err().code,
        "SYMLINK_PATH"
    );
    assert_eq!(bundles::list(&vault).unwrap_err().code, "SYMLINK_PATH");
    assert_eq!(snapshot(&temp.0), before);
}

#[cfg(unix)]
fn failure_child(temp: &Temp, operation: &str, replacement: bool) {
    use std::os::unix::fs::PermissionsExt;
    let directory = temp.0.join("trusted-test-bin");
    fs::create_dir(&directory).unwrap();
    let real_git = kinana_core::agents::executable_in_path("git")
        .unwrap()
        .canonicalize()
        .unwrap();
    fs::write(
        directory.join("git"),
        r#"#!/bin/sh
set -eu
for argument do
    if test "$argument" = commit; then
        if test "$BUNDLE_TEST_REPLACE" = yes; then
            rm -f -- "$BUNDLE_TEST_TARGET"
            printf 'foreign replacement\n' > "$BUNDLE_TEST_TARGET"
        fi
        printf 'injected bundle commit failure\n' >&2
        exit 87
    fi
done
exec "$BUNDLE_TEST_REAL_GIT" "$@"
"#,
    )
    .unwrap();
    fs::set_permissions(directory.join("git"), fs::Permissions::from_mode(0o700)).unwrap();
    let inherited = std::env::var_os("PATH").unwrap();
    let path =
        std::env::join_paths(std::iter::once(directory).chain(std::env::split_paths(&inherited)))
            .unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "bundle_commit_failure_child", "--nocapture"])
        .env("PATH", path)
        .env("BUNDLE_TEST_ROOT", &temp.0)
        .env("BUNDLE_TEST_OPERATION", operation)
        .env("BUNDLE_TEST_REAL_GIT", real_git)
        .env(
            "BUNDLE_TEST_REPLACE",
            if replacement { "yes" } else { "no" },
        )
        .env(
            "BUNDLE_TEST_TARGET",
            temp.0.join("vault/bundles/web-app.toml"),
        )
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
}

#[cfg(unix)]
#[test]
fn failed_commits_restore_create_absence_and_exact_read_only_old_file() {
    use std::os::unix::fs::PermissionsExt;
    for operation in ["create", "edit"] {
        let temp = Temp::new();
        let vault = temp.vault();
        if operation == "edit" {
            bundles::create(&vault, &temp.input(&sample("web-app")), false).unwrap();
            fs::set_permissions(
                vault.root.join("bundles/web-app.toml"),
                fs::Permissions::from_mode(0o444),
            )
            .unwrap();
        }
        let mut changed = sample("web-app");
        changed.title = "Changed fixture".into();
        temp.input(&changed);
        let before = without_git(snapshot(&temp.0));
        let head = gitlog::head(&vault.root).unwrap();
        failure_child(&temp, operation, false);
        let mut after = without_git(snapshot(&temp.0));
        after.retain(|path, _| !path.starts_with("trusted-test-bin"));
        assert_eq!(after, before, "{operation}");
        assert_eq!(gitlog::head(&vault.root).unwrap(), head);
        gitlog::ensure_clean(&vault.root).unwrap();
        assert!(
            git(&vault, &["diff", "--cached", "--name-only"])
                .stdout
                .is_empty()
        );
        if operation == "edit" {
            assert_eq!(
                fs::metadata(vault.root.join("bundles/web-app.toml"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o444
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn concurrent_foreign_replacement_is_preserved_and_old_bundle_backup_is_retained() {
    let temp = Temp::new();
    let vault = temp.vault();
    bundles::create(&vault, &temp.input(&sample("web-app")), false).unwrap();
    let old = fs::read(vault.root.join("bundles/web-app.toml")).unwrap();
    let mut changed = sample("web-app");
    changed.title = "Changed fixture".into();
    temp.input(&changed);
    let originals = snapshot(&vault.root.join("skills"));
    let head = gitlog::head(&vault.root).unwrap();
    failure_child(&temp, "edit", true);
    assert_eq!(
        fs::read(vault.root.join("bundles/web-app.toml")).unwrap(),
        b"foreign replacement\n"
    );
    assert_eq!(gitlog::head(&vault.root).unwrap(), head);
    assert!(
        git(&vault, &["diff", "--cached", "--name-only"])
            .stdout
            .is_empty()
    );
    assert_eq!(snapshot(&vault.root.join("skills")), originals);
    let backup = fs::read_dir(vault.root.join("dist"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".skillsbox-bundle-")
        })
        .unwrap();
    assert_eq!(fs::read(backup.join("old.toml")).unwrap(), old);
}

#[cfg(unix)]
#[test]
fn successful_commit_survives_cache_refresh_failure_with_an_explicit_warning() {
    use std::os::unix::fs::PermissionsExt;
    let temp = Temp::new();
    let vault = temp.vault();
    temp.input(&sample("web-app"));
    let source_before = snapshot(&vault.root.join("skills"));
    let head = gitlog::head(&vault.root).unwrap();
    let tools = temp.0.join("trusted-cache-test-bin");
    fs::create_dir(&tools).unwrap();
    fs::write(
        tools.join("git"),
        r#"#!/bin/sh
set -eu
for argument do
    if test "$argument" = commit; then
        "$BUNDLE_CACHE_REAL_GIT" "$@"
        printf 'foreign cache bytes\n' > "$BUNDLE_CACHE_PATH"
        exit 0
    fi
done
exec "$BUNDLE_CACHE_REAL_GIT" "$@"
"#,
    )
    .unwrap();
    fs::set_permissions(tools.join("git"), fs::Permissions::from_mode(0o700)).unwrap();
    let inherited = std::env::var_os("PATH").unwrap();
    let path =
        std::env::join_paths(std::iter::once(tools).chain(std::env::split_paths(&inherited)))
            .unwrap();
    let real_git = kinana_core::agents::executable_in_path("git")
        .unwrap()
        .canonicalize()
        .unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "bundle_cache_refresh_failure_child",
            "--nocapture",
        ])
        .env("PATH", path)
        .env("BUNDLE_CACHE_ROOT", &temp.0)
        .env("BUNDLE_CACHE_REAL_GIT", real_git)
        .env("BUNDLE_CACHE_PATH", &vault.index_path)
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
    assert_ne!(gitlog::head(&vault.root).unwrap(), head);
    assert_eq!(bundles::read(&vault, "web-app").unwrap(), sample("web-app"));
    assert_eq!(
        fs::read(&vault.index_path).unwrap(),
        b"foreign cache bytes\n"
    );
    assert_eq!(snapshot(&vault.root.join("skills")), source_before);
    assert!(fs::read_dir(vault.root.join("dist")).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".skillsbox-bundle-")
    }));
    gitlog::ensure_clean(&vault.root).unwrap();
}

#[cfg(unix)]
#[test]
fn bundle_cache_refresh_failure_child() {
    let Some(root) = std::env::var_os("BUNDLE_CACHE_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let vault = Vault::new(root.join("vault"), root.join("cache/index.db"));
    let report = bundles::create(&vault, &root.join("input.toml"), false).unwrap();
    assert!(report.committed);
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("index refresh failed"))
    );
}

#[cfg(unix)]
#[test]
fn bundle_commit_failure_child() {
    let Some(root) = std::env::var_os("BUNDLE_TEST_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let vault = Vault::new(root.join("vault"), root.join("cache/index.db"));
    let result = if std::env::var("BUNDLE_TEST_OPERATION").unwrap() == "edit" {
        bundles::edit(&vault, "web-app", &root.join("input.toml"), false)
    } else {
        bundles::create(&vault, &root.join("input.toml"), false)
    };
    let error = result.unwrap_err();
    assert_eq!(error.code, "GIT_ERROR");
    assert!(error.message.contains("injected bundle commit failure"));
}

#[cfg(unix)]
#[test]
fn bundle_save_never_executes_git_from_a_known_original_source_folder() {
    use std::os::unix::fs::PermissionsExt;
    let temp = Temp::new();
    let vault = temp.vault();
    temp.input(&sample("web-app"));
    let original = temp.0.join("source-api");
    let marker = temp.0.join("SOURCE_GIT_RAN");
    fs::write(
        original.join("git"),
        "#!/bin/sh\nprintf executed > \"$BUNDLE_SOURCE_MARKER\"\nexit 91\n",
    )
    .unwrap();
    fs::set_permissions(original.join("git"), fs::Permissions::from_mode(0o700)).unwrap();
    let search = std::env::var_os("PATH").unwrap();
    let path =
        std::env::join_paths(std::iter::once(original).chain(std::env::split_paths(&search)))
            .unwrap();
    let before = snapshot(&temp.0);
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "bundle_unsafe_source_git_child", "--nocapture"])
        .env("PATH", path)
        .env("BUNDLE_SOURCE_ROOT", &temp.0)
        .env("BUNDLE_SOURCE_MARKER", &marker)
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
    assert!(!marker.exists());
    assert_eq!(snapshot(&temp.0), before);
    gitlog::ensure_clean(&vault.root).unwrap();
}

#[cfg(unix)]
#[test]
fn bundle_unsafe_source_git_child() {
    let Some(root) = std::env::var_os("BUNDLE_SOURCE_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let vault = Vault::new(root.join("vault"), root.join("cache/index.db"));
    for dry_run in [true, false] {
        let error = bundles::create(&vault, &root.join("input.toml"), dry_run).unwrap_err();
        assert_eq!(error.code, "UNSAFE_GIT_EXECUTABLE");
    }
}

#[cfg(unix)]
#[test]
fn successful_edit_preserves_old_backup_modified_by_a_concurrent_writer_and_warns() {
    use std::os::unix::fs::PermissionsExt;
    let temp = Temp::new();
    let vault = temp.vault();
    bundles::create(&vault, &temp.input(&sample("web-app")), false).unwrap();
    let mut replacement = sample("web-app");
    replacement.title = "Replacement title".into();
    temp.input(&replacement);
    let sources = snapshot(&vault.root.join("skills"));
    let head = gitlog::head(&vault.root).unwrap();
    let tools = temp.0.join("trusted-backup-test-bin");
    fs::create_dir(&tools).unwrap();
    fs::write(
        tools.join("git"),
        r#"#!/bin/sh
set -eu
for argument do
    if test "$argument" = commit; then
        for backup in "$BUNDLE_BACKUP_DIST"/.skillsbox-bundle-*/old.toml; do
            if test -f "$backup"; then
                printf 'concurrent writer changed original\n' > "$backup"
            fi
        done
    fi
done
exec "$BUNDLE_BACKUP_REAL_GIT" "$@"
"#,
    )
    .unwrap();
    fs::set_permissions(tools.join("git"), fs::Permissions::from_mode(0o700)).unwrap();
    let real_git = kinana_core::agents::executable_in_path("git")
        .unwrap()
        .canonicalize()
        .unwrap();
    let search = std::env::var_os("PATH").unwrap();
    let path =
        std::env::join_paths(std::iter::once(tools).chain(std::env::split_paths(&search))).unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "bundle_modified_backup_child", "--nocapture"])
        .env("PATH", path)
        .env("BUNDLE_BACKUP_ROOT", &temp.0)
        .env("BUNDLE_BACKUP_DIST", vault.root.join("dist"))
        .env("BUNDLE_BACKUP_REAL_GIT", real_git)
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
    assert_ne!(gitlog::head(&vault.root).unwrap(), head);
    assert_eq!(bundles::read(&vault, "web-app").unwrap(), replacement);
    let stage = fs::read_dir(vault.root.join("dist"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".skillsbox-bundle-")
        })
        .unwrap();
    assert_eq!(
        fs::read(stage.join("old.toml")).unwrap(),
        b"concurrent writer changed original\n"
    );
    assert!(!stage.join("new.toml").exists());
    assert_eq!(snapshot(&vault.root.join("skills")), sources);
    gitlog::ensure_clean(&vault.root).unwrap();
}

#[cfg(unix)]
#[test]
fn bundle_modified_backup_child() {
    let Some(root) = std::env::var_os("BUNDLE_BACKUP_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let vault = Vault::new(root.join("vault"), root.join("cache/index.db"));
    let report = bundles::edit(&vault, "web-app", &root.join("input.toml"), false).unwrap();
    assert!(report.committed);
    let warning = report
        .warnings
        .iter()
        .find(|warning| warning.contains("preserved"))
        .unwrap();
    assert!(warning.contains(".skillsbox-bundle-"));
}
