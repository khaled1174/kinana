use kinana_core::gitlog;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "kinana-git-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn initial_commit_is_local_and_source_hint_is_not_guessed() {
    let temp = Scratch::new();
    for dir in ["skills", "meta", "bundles"] {
        fs::create_dir(temp.0.join(dir)).unwrap();
    }
    fs::write(temp.0.join("box.toml"), "version = 1\n").unwrap();
    fs::write(temp.0.join(".gitignore"), "dist/\n").unwrap();
    gitlog::init(&temp.0).unwrap();
    gitlog::commit(&temp.0, "Initialize Kinana vault").unwrap();
    let head = Command::new("git")
        .arg("-C")
        .arg(&temp.0)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert!(head.status.success());
    assert_eq!(gitlog::remote_hint(&temp.0).kind, "unknown");
    let remote = Command::new("git")
        .arg("-C")
        .arg(&temp.0)
        .args([
            "remote",
            "add",
            "origin",
            "https://github.com/example/skills.git",
        ])
        .status()
        .unwrap();
    assert!(remote.success());
    let hint = gitlog::remote_hint(&temp.0);
    assert_eq!(hint.kind, "scan");
    assert_eq!(
        hint.url.as_deref(),
        Some("https://github.com/example/skills.git")
    );
    assert_eq!(
        hint.commit.as_deref(),
        Some(String::from_utf8_lossy(&head.stdout).trim())
    );
}

#[cfg(unix)]
#[test]
fn vault_commits_never_run_a_repository_hook() {
    use std::os::unix::fs::PermissionsExt;
    let temp = Scratch::new();
    for dir in ["skills", "meta", "bundles"] {
        fs::create_dir(temp.0.join(dir)).unwrap();
    }
    fs::write(temp.0.join("box.toml"), "version = 1\n").unwrap();
    fs::write(temp.0.join(".gitignore"), "dist/\n").unwrap();
    gitlog::init(&temp.0).unwrap();
    let hook = temp.0.join(".git/hooks/pre-commit");
    fs::create_dir_all(hook.parent().unwrap()).unwrap();
    fs::write(&hook, "#!/bin/sh\nprintf hook-ran > HOOK_RAN\nexit 1\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o700)).unwrap();
    gitlog::commit(&temp.0, "Safe local commit").unwrap();
    assert!(!temp.0.join("HOOK_RAN").exists());
}

fn initialized_vault() -> Scratch {
    let temp = Scratch::new();
    for dir in ["skills", "meta", "bundles", "dist"] {
        fs::create_dir(temp.0.join(dir)).unwrap();
    }
    fs::write(temp.0.join("box.toml"), "version = 1\n").unwrap();
    fs::write(temp.0.join(".gitignore"), "dist/\n").unwrap();
    gitlog::init(&temp.0).unwrap();
    gitlog::commit(&temp.0, "Initialize Kinana vault").unwrap();
    temp
}

fn git(root: &std::path::Path, args: &[&str]) -> std::process::Output {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn add_skill(root: &std::path::Path, name: &str, content: &[u8]) {
    let skill = root.join("skills").join(name);
    fs::create_dir_all(skill.join("assets")).unwrap();
    fs::write(skill.join("SKILL.md"), content).unwrap();
    fs::write(skill.join("assets/raw.bin"), [0, 255, 13, 10, 128]).unwrap();
    fs::write(
        root.join("meta").join(format!("{name}.toml")),
        format!("id = \"{name}\"\n"),
    )
    .unwrap();
}

fn snapshot(root: &std::path::Path) -> std::collections::BTreeMap<PathBuf, Option<Vec<u8>>> {
    walkdir::WalkDir::new(root)
        .min_depth(1)
        .into_iter()
        .map(|entry| {
            let entry = entry.unwrap();
            let data = entry
                .file_type()
                .is_file()
                .then(|| fs::read(entry.path()).unwrap());
            (entry.path().strip_prefix(root).unwrap().to_path_buf(), data)
        })
        .collect()
}

#[test]
fn undo_restores_previous_bytes_and_removes_imported_source_and_metadata() {
    let temp = initialized_vault();
    add_skill(&temp.0, "existing", b"original\r\nbytes\0\xff");
    gitlog::commit(&temp.0, "Import existing").unwrap();
    let previous = gitlog::head(&temp.0).unwrap();
    fs::write(temp.0.join("skills/existing/SKILL.md"), b"changed\n").unwrap();
    add_skill(&temp.0, "new", b"new skill\r\n");
    gitlog::commit(&temp.0, "Change and import").unwrap();
    let imported = gitlog::head(&temp.0).unwrap();
    let report = gitlog::undo(&temp.0, false).unwrap();
    assert_eq!(report.reverted_commit, imported);
    assert_eq!(
        report.resulting_commit.as_deref(),
        Some(gitlog::head(&temp.0).unwrap().as_str())
    );
    assert!(!report.dry_run);
    assert_eq!(
        fs::read(temp.0.join("skills/existing/SKILL.md")).unwrap(),
        b"original\r\nbytes\0\xff"
    );
    assert!(!temp.0.join("skills/new").exists());
    assert!(!temp.0.join("meta/new.toml").exists());
    let trees = git(
        &temp.0,
        &["rev-parse", &format!("{previous}^{{tree}}"), "HEAD^{tree}"],
    );
    let trees: Vec<_> = std::str::from_utf8(&trees.stdout)
        .unwrap()
        .lines()
        .collect();
    assert_eq!(trees[0], trees[1]);
    gitlog::ensure_clean(&temp.0).unwrap();
    // Undo means revert HEAD, including when HEAD is itself a revert.
    let reverted = gitlog::head(&temp.0).unwrap();
    let again = gitlog::undo(&temp.0, false).unwrap();
    assert_eq!(again.reverted_commit, reverted);
    assert_eq!(
        fs::read(temp.0.join("skills/existing/SKILL.md")).unwrap(),
        b"changed\n"
    );
    assert!(temp.0.join("skills/new/SKILL.md").exists());
    assert!(temp.0.join("meta/new.toml").exists());
    gitlog::ensure_clean(&temp.0).unwrap();
}

#[test]
fn undo_dry_run_reports_head_without_changing_any_vault_or_git_file() {
    let temp = initialized_vault();
    add_skill(&temp.0, "one", b"one\n");
    gitlog::commit(&temp.0, "Import one").unwrap();
    // Old vaults may predate the managed attributes guard. Preview must not
    // install it, create a directory, or refresh the Git index.
    fs::remove_file(temp.0.join(".git/info/attributes")).unwrap();
    fs::remove_dir(temp.0.join(".git/info")).unwrap();
    let before = snapshot(&temp.0);
    let head = gitlog::head(&temp.0).unwrap();
    let report = gitlog::undo(&temp.0, true).unwrap();
    assert_eq!(report.reverted_commit, head);
    assert_eq!(report.resulting_commit, None);
    assert!(report.dry_run);
    assert_eq!(snapshot(&temp.0), before);
}

#[test]
fn undo_only_import_preserves_initialized_vault_directories() {
    let temp = initialized_vault();
    add_skill(&temp.0, "one", b"one\n");
    fs::create_dir_all(temp.0.join("skills/one/assets/empty/deeper")).unwrap();
    gitlog::commit(&temp.0, "Import one").unwrap();
    gitlog::undo(&temp.0, false).unwrap();
    for name in ["skills", "meta", "bundles", "dist"] {
        assert!(temp.0.join(name).is_dir());
    }
    assert!(!temp.0.join("skills/one").exists());
    assert!(!temp.0.join("meta/one.toml").exists());
    gitlog::ensure_clean(&temp.0).unwrap();
}

#[test]
fn undo_prunes_removed_skill_empty_directories_and_keeps_existing_skill_directories() {
    let temp = initialized_vault();
    add_skill(&temp.0, "existing", b"existing\n");
    fs::create_dir_all(temp.0.join("skills/existing/assets/keep-empty")).unwrap();
    gitlog::commit(&temp.0, "Import existing").unwrap();
    add_skill(&temp.0, "new", b"new\n");
    fs::create_dir_all(temp.0.join("skills/new/assets/empty/deeper")).unwrap();
    gitlog::commit(&temp.0, "Import new with empty directories").unwrap();
    gitlog::undo(&temp.0, false).unwrap();
    assert!(!temp.0.join("skills/new").exists());
    assert!(!temp.0.join("meta/new.toml").exists());
    assert!(temp.0.join("skills/existing/assets/keep-empty").is_dir());
    assert_eq!(
        fs::read(temp.0.join("skills/existing/SKILL.md")).unwrap(),
        b"existing\n"
    );
    gitlog::ensure_clean(&temp.0).unwrap();
}

#[test]
fn initial_commit_cannot_be_undone_even_in_dry_run() {
    let temp = initialized_vault();
    let before = snapshot(&temp.0);
    for dry_run in [true, false] {
        let error = gitlog::undo(&temp.0, dry_run).unwrap_err();
        assert_eq!(error.code, "UNDO_INITIAL_COMMIT");
        assert!(error.needs_decision);
        assert_eq!(snapshot(&temp.0), before);
    }
}

#[test]
fn undo_refuses_staged_unstaged_untracked_and_ignored_dirty_files_without_changes() {
    for kind in ["unstaged", "staged", "untracked", "ignored"] {
        let temp = initialized_vault();
        add_skill(&temp.0, "one", b"one\n");
        gitlog::commit(&temp.0, "Import one").unwrap();
        match kind {
            "unstaged" => fs::write(temp.0.join("box.toml"), "version = 2\n").unwrap(),
            "staged" => {
                fs::write(temp.0.join("box.toml"), "version = 2\n").unwrap();
                git(&temp.0, &["add", "box.toml"]);
            }
            "untracked" => fs::write(temp.0.join("notes.txt"), "personal\n").unwrap(),
            "ignored" => {
                fs::write(temp.0.join(".git/info/exclude"), "private.txt\n").unwrap();
                fs::write(temp.0.join("private.txt"), "personal\n").unwrap();
            }
            _ => unreachable!(),
        }
        let before = snapshot(&temp.0);
        for dry_run in [true, false] {
            assert_eq!(
                gitlog::undo(&temp.0, dry_run).unwrap_err().code,
                "GIT_DIRTY_VAULT"
            );
            assert_eq!(snapshot(&temp.0), before, "{kind}");
        }
    }
}

#[test]
fn commits_refuse_foreign_staged_and_untracked_files_without_consuming_them() {
    for staged in [false, true] {
        let temp = initialized_vault();
        add_skill(&temp.0, "one", b"one\n");
        fs::write(temp.0.join("notes.txt"), "personal\n").unwrap();
        if staged {
            git(&temp.0, &["add", "notes.txt"]);
        }
        let before = snapshot(&temp.0);
        assert_eq!(
            gitlog::commit(&temp.0, "Import one").unwrap_err().code,
            "GIT_DIRTY_VAULT"
        );
        assert_eq!(snapshot(&temp.0), before);
    }
}

#[test]
fn full_skill_payload_is_tracked_while_finder_inventory_and_dist_are_tolerated() {
    let temp = initialized_vault();
    for path in [
        ".DS_Store",
        "skills/.DS_Store",
        "meta/.DS_Store",
        "bundles/.DS_Store",
        "dist/generated",
    ] {
        fs::write(temp.0.join(path), "local\n").unwrap();
    }
    gitlog::ensure_clean(&temp.0).unwrap();
    add_skill(&temp.0, "one", b"one\r\n");
    fs::write(temp.0.join("skills/one/.gitignore"), "assets/\n.DS_Store\n").unwrap();
    fs::write(
        temp.0.join("skills/one/.DS_Store"),
        "part of original payload",
    )
    .unwrap();
    gitlog::commit(&temp.0, "Import all original payload").unwrap();
    let tracked = git(&temp.0, &["ls-files", "-z"]);
    let tracked: Vec<_> = tracked.stdout.split(|byte| *byte == 0).collect();
    assert!(tracked.contains(&b"skills/one/assets/raw.bin".as_slice()));
    assert!(tracked.contains(&b"skills/one/.DS_Store".as_slice()));
    for path in [
        ".DS_Store",
        "skills/.DS_Store",
        "meta/.DS_Store",
        "bundles/.DS_Store",
        "dist/generated",
    ] {
        assert!(!tracked.contains(&path.as_bytes()));
    }
    gitlog::ensure_clean(&temp.0).unwrap();
}

#[test]
fn unstage_managed_preserves_foreign_staged_files_and_all_worktree_bytes() {
    let temp = initialized_vault();
    add_skill(&temp.0, "one", b"one\n");
    fs::write(temp.0.join("notes.txt"), "personal\n").unwrap();
    git(&temp.0, &["add", "skills", "meta", "notes.txt"]);
    gitlog::unstage_managed(&temp.0).unwrap();
    assert!(temp.0.join("skills/one/SKILL.md").exists());
    assert!(temp.0.join("meta/one.toml").exists());
    assert_eq!(fs::read(temp.0.join("notes.txt")).unwrap(), b"personal\n");
    let staged = git(&temp.0, &["diff", "--cached", "--name-only"]);
    assert_eq!(staged.stdout, b"notes.txt\n");
}

#[cfg(unix)]
#[test]
fn repository_symlinks_and_foreign_attributes_are_refused_without_writes() {
    use std::os::unix::fs::symlink;
    for kind in ["config", "hooks", "attributes"] {
        let temp = initialized_vault();
        let external = Scratch::new();
        match kind {
            "config" => {
                fs::rename(temp.0.join(".git/config"), external.0.join("config")).unwrap();
                symlink(external.0.join("config"), temp.0.join(".git/config")).unwrap();
            }
            "hooks" => {
                symlink(&external.0, temp.0.join(".git/hooks")).unwrap();
            }
            "attributes" => {
                fs::write(temp.0.join(".git/info/attributes"), "* filter=foreign\n").unwrap();
            }
            _ => unreachable!(),
        }
        let outside_before = snapshot(&external.0);
        assert_eq!(
            gitlog::verify_repository(&temp.0).unwrap_err().code,
            "GIT_UNSAFE_REPOSITORY"
        );
        assert!(gitlog::commit(&temp.0, "No write").is_err());
        assert!(gitlog::undo(&temp.0, true).is_err());
        assert_eq!(snapshot(&external.0), outside_before);
    }
}

#[cfg(unix)]
#[test]
fn hooks_filters_and_hostile_inherited_git_environment_cannot_execute() {
    use std::os::unix::fs::PermissionsExt;
    let temp = Scratch::new();
    let fake_git = temp.0.join("git");
    fs::write(
        &fake_git,
        "#!/bin/sh\nprintf ran > \"$HOME/SCRIPT_RAN\"\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&fake_git, fs::Permissions::from_mode(0o700)).unwrap();
    let inherited_path = std::env::var_os("PATH").unwrap();
    let path = std::env::join_paths(
        [PathBuf::from(".")]
            .into_iter()
            .chain(std::env::split_paths(&inherited_path)),
    )
    .unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "hostile_git_environment_child", "--nocapture"])
        .current_dir(&temp.0)
        .env_clear()
        .env("PATH", path)
        .env("HOME", &temp.0)
        .env("KINANA_GIT_SAFETY_CHILD", &temp.0)
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "core.worktree")
        .env("GIT_CONFIG_VALUE_0", "/definitely-not-the-vault")
        .env("GIT_CONFIG_PARAMETERS", "'core.bare=true'")
        .env("GIT_DIR", temp.0.join("wrong-repository"))
        .env("GIT_INDEX_FILE", temp.0.join("outside-index"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!temp.0.join("SCRIPT_RAN").exists());
    assert!(!temp.0.join("outside-index").exists());
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "missing_safe_git_environment_child",
            "--nocapture",
        ])
        .current_dir(&temp.0)
        .env_clear()
        .env("PATH", ".")
        .env("HOME", &temp.0)
        .env("KINANA_GIT_SAFETY_CHILD", &temp.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!temp.0.join("SCRIPT_RAN").exists());
}

#[cfg(unix)]
#[test]
fn missing_safe_git_environment_child() {
    let Some(root) = std::env::var_os("KINANA_GIT_SAFETY_CHILD") else {
        return;
    };
    let vault = PathBuf::from(root).join("no-safe-git");
    fs::create_dir(&vault).unwrap();
    let before = snapshot(&vault);
    // PATH is only ".": the script there is never a candidate. Since
    // 2026-11-18 a trusted system Git from the fixed install locations
    // (/etc/paths) is used instead when one exists; otherwise nothing runs.
    match kinana_core::executables::find("git", &[]).found {
        Some(git) => {
            assert!(!git.starts_with(PathBuf::from(
                std::env::var_os("KINANA_GIT_SAFETY_CHILD").unwrap()
            )));
            gitlog::init(&vault).unwrap();
        }
        None => {
            assert_eq!(gitlog::init(&vault).unwrap_err().code, "GIT_ERROR");
            assert_eq!(snapshot(&vault), before);
        }
    }
}

#[cfg(unix)]
#[test]
fn hostile_git_environment_child() {
    use std::os::unix::fs::PermissionsExt;
    let Some(root) = std::env::var_os("KINANA_GIT_SAFETY_CHILD") else {
        return;
    };
    let root = PathBuf::from(root);
    let vault = root.join("vault");
    fs::create_dir(&vault).unwrap();
    for dir in ["skills", "meta", "bundles", "dist"] {
        fs::create_dir(vault.join(dir)).unwrap();
    }
    fs::write(vault.join("box.toml"), "version = 1\n").unwrap();
    fs::write(vault.join(".gitignore"), "dist/\n").unwrap();
    // These are controlled test fixtures outside the imported skill payload.
    let script = root.join("fake-git-command");
    fs::write(
        &script,
        "#!/bin/sh\nprintf ran > \"$HOME/SCRIPT_RAN\"\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    let script = script.to_str().unwrap();
    fs::write(root.join(".gitconfig"), format!("[core]\n hooksPath = {script}\n fsmonitor = {script}\n attributesFile = {}\n[filter \"attack\"]\n clean = {script}\n smudge = {script}\n process = {script}\n required = true\n", root.join("global-attributes").display())).unwrap();
    fs::write(root.join("global-attributes"), "* filter=attack\n").unwrap();
    gitlog::init(&vault).unwrap();
    fs::create_dir_all(vault.join(".git/hooks")).unwrap();
    for hook in [
        "pre-commit",
        "prepare-commit-msg",
        "commit-msg",
        "post-commit",
        "post-rewrite",
        "post-index-change",
    ] {
        let hook = vault.join(".git/hooks").join(hook);
        fs::write(&hook, fs::read(script).unwrap()).unwrap();
        fs::set_permissions(hook, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut config = fs::read_to_string(vault.join(".git/config")).unwrap();
    config.push_str(&format!("\n[core]\n hooksPath = {}\n fsmonitor = {script}\n autocrlf = true\n[filter \"attack\"]\n clean = {script}\n smudge = {script}\n process = {script}\n required = true\n[merge \"attack\"]\n driver = {script}\n", vault.join(".git/hooks").display()));
    fs::write(vault.join(".git/config"), config).unwrap();
    gitlog::commit(&vault, "Initialize safely").unwrap();
    add_skill(&vault, "one", b"keep\r\nthese bytes\r\n");
    let attrs = b"* filter=attack merge=attack text eol=lf working-tree-encoding=UTF-16LE\n";
    fs::write(vault.join("skills/one/.gitattributes"), attrs).unwrap();
    gitlog::commit(&vault, "Import inert attributes").unwrap();
    assert_eq!(
        fs::read(vault.join("skills/one/SKILL.md")).unwrap(),
        b"keep\r\nthese bytes\r\n"
    );
    assert_eq!(
        fs::read(vault.join("skills/one/.gitattributes")).unwrap(),
        attrs
    );
    fs::write(vault.join("skills/one/SKILL.md"), b"changed\r\n").unwrap();
    gitlog::commit_paths(
        &vault,
        "Change inert data through exact path staging",
        &[PathBuf::from("skills/one/SKILL.md")],
    )
    .unwrap();
    // Exact rollback must install its guard before touching the index too.
    fs::remove_file(vault.join(".git/info/attributes")).unwrap();
    gitlog::unstage_paths(&vault, &[PathBuf::from("skills/one/SKILL.md")]).unwrap();
    assert!(vault.join(".git/info/attributes").is_file());
    assert!(!root.join("SCRIPT_RAN").exists());
    // An index-only reset must restore the guard before consulting attributes.
    fs::remove_file(vault.join(".git/info/attributes")).unwrap();
    gitlog::unstage_managed(&vault).unwrap();
    assert!(vault.join(".git/info/attributes").is_file());
    assert!(!root.join("SCRIPT_RAN").exists());
    // Preflight in an old vault without a guard remains read-only and never
    // runs a clean filter just to decide whether the worktree is clean.
    fs::remove_file(vault.join(".git/info/attributes")).unwrap();
    let before = snapshot(&vault);
    gitlog::ensure_clean(&vault).unwrap();
    gitlog::undo(&vault, true).unwrap();
    assert_eq!(snapshot(&vault), before);
    gitlog::undo(&vault, false).unwrap();
    assert_eq!(
        fs::read(vault.join("skills/one/SKILL.md")).unwrap(),
        b"keep\r\nthese bytes\r\n"
    );
    assert!(!root.join("SCRIPT_RAN").exists());
}

#[cfg(unix)]
#[test]
fn git_executable_inside_vault_and_outside_symlink_to_it_are_never_run() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let temp = initialized_vault();
    let outside = Scratch::new();
    add_skill(&temp.0, "good", b"ordinary sibling skill\n");
    add_skill(&temp.0, "hostile", b"inert payload\n");
    let binary = temp.0.join("skills/hostile/git");
    fs::write(
        &binary,
        "#!/bin/sh\nprintf ran > \"$KINANA_GIT_BINARY_MARKER\"\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    gitlog::commit(&temp.0, "Import inert executable fixture").unwrap();
    git(
        &temp.0,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/example/skills.git",
        ],
    );
    let link_directory = outside.0.join("bin");
    fs::create_dir(&link_directory).unwrap();
    symlink(&binary, link_directory.join("git")).unwrap();
    let inherited_path = std::env::var_os("PATH").unwrap();
    let marker = outside.0.join("SCRIPT_RAN");
    for untrusted_directory in [binary.parent().unwrap(), link_directory.as_path()] {
        for fallback in [true, false] {
            let path = std::env::join_paths(
                [untrusted_directory.to_path_buf()]
                    .into_iter()
                    .chain(std::env::split_paths(&inherited_path).filter(|_| fallback)),
            )
            .unwrap();
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "vault_git_executable_child", "--nocapture"])
                .current_dir(&temp.0)
                .env_clear()
                .env("PATH", path)
                .env("HOME", &outside.0)
                .env("KINANA_GIT_BINARY_TEST_ROOT", &temp.0)
                .env("KINANA_GIT_BINARY_MARKER", &marker)
                .env(
                    "KINANA_EXPECT_SAFE_GIT",
                    if fallback { "yes" } else { "no" },
                )
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(!marker.exists());
        }
    }
}

#[cfg(unix)]
#[test]
fn vault_git_executable_child() {
    let Some(root) = std::env::var_os("KINANA_GIT_BINARY_TEST_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    // Without the inherited PATH, a trusted Git may still come from the fixed
    // install locations (since 2026-11-18); the vault's own `git` never runs.
    let trusted_fallback = kinana_core::executables::git(&[root.canonicalize().unwrap()]).is_ok();
    if std::env::var("KINANA_EXPECT_SAFE_GIT").unwrap() == "yes" || trusted_fallback {
        gitlog::head(&root).unwrap();
        gitlog::ensure_clean(&root).unwrap();
        gitlog::commit(&root, "No changes").unwrap();
        let hint = gitlog::remote_hint(&root.join("skills/good"));
        assert_eq!(hint.kind, "scan");
        assert_eq!(
            hint.url.as_deref(),
            Some("https://github.com/example/skills.git")
        );
    } else {
        let before = snapshot(&root);
        assert_eq!(gitlog::head(&root).unwrap_err().code, "GIT_ERROR");
        assert_eq!(gitlog::init(&root).unwrap_err().code, "GIT_ERROR");
        assert_eq!(
            gitlog::commit(&root, "No changes").unwrap_err().code,
            "GIT_ERROR"
        );
        assert_eq!(
            gitlog::remote_hint(&root.join("skills/good")).kind,
            "unknown"
        );
        assert_eq!(snapshot(&root), before);
    }
}

#[test]
fn exact_commit_refuses_unrelated_managed_changes_without_touching_the_index() {
    for kind in ["staged", "untracked", "unstaged"] {
        let temp = initialized_vault();
        if kind == "unstaged" {
            fs::write(temp.0.join("bundles/unrelated.toml"), "before\n").unwrap();
            gitlog::commit(&temp.0, "Create existing unrelated bundle").unwrap();
        }
        fs::write(temp.0.join("meta/imported.toml"), "owned import\n").unwrap();
        fs::write(temp.0.join("bundles/unrelated.toml"), "user data\n").unwrap();
        if kind == "staged" {
            git(&temp.0, &["add", "bundles/unrelated.toml"]);
        }
        let before = snapshot(&temp.0);
        let error = gitlog::commit_paths(
            &temp.0,
            "Import only owned metadata",
            &[PathBuf::from("meta/imported.toml")],
        )
        .unwrap_err();
        assert_eq!(error.code, "GIT_DIRTY_VAULT", "{kind}");
        assert_eq!(snapshot(&temp.0), before, "{kind}");
    }
}

#[test]
fn exact_commit_uses_literal_file_paths_and_supports_staged_and_unstaged_deletions() {
    let temp = initialized_vault();
    fs::create_dir(temp.0.join("skills/one")).unwrap();
    fs::write(temp.0.join("skills/one/a.txt"), "existing sibling\n").unwrap();
    gitlog::commit(&temp.0, "Create sibling").unwrap();
    let literal = PathBuf::from("skills/one/[a]*.txt");
    fs::write(temp.0.join(&literal), b"literal payload\r\n\0\xff").unwrap();
    gitlog::commit_paths(
        &temp.0,
        "Import exact literal filename",
        std::slice::from_ref(&literal),
    )
    .unwrap();
    let payload = git(&temp.0, &["show", "HEAD:skills/one/[a]*.txt"]);
    assert_eq!(payload.stdout, b"literal payload\r\n\0\xff");
    assert_eq!(
        fs::read(temp.0.join("skills/one/a.txt")).unwrap(),
        b"existing sibling\n"
    );
    fs::remove_file(temp.0.join(&literal)).unwrap();
    gitlog::commit_paths(&temp.0, "Delete exact file", &[literal]).unwrap();
    let staged = PathBuf::from("skills/one/a.txt");
    fs::remove_file(temp.0.join(&staged)).unwrap();
    git(&temp.0, &["add", "--all", "--", "skills/one/a.txt"]);
    gitlog::commit_paths(&temp.0, "Commit already staged deletion", &[staged]).unwrap();
    assert!(git(&temp.0, &["ls-files", "skills/one"]).stdout.is_empty());
}

#[test]
fn exact_rollback_preserves_unrelated_managed_and_foreign_staged_data() {
    let temp = initialized_vault();
    fs::write(temp.0.join("meta/existing.toml"), "before\n").unwrap();
    gitlog::commit(&temp.0, "Create existing metadata").unwrap();
    fs::write(temp.0.join("meta/existing.toml"), "owned update\n").unwrap();
    fs::write(temp.0.join("meta/a[1].toml"), "owned addition\n").unwrap();
    fs::write(temp.0.join("meta/a1.toml"), "other managed data\n").unwrap();
    fs::write(temp.0.join("notes.txt"), "other foreign data\n").unwrap();
    git(&temp.0, &["add", "meta", "notes.txt"]);
    // The importer restores its own worktree entries before clearing its index entries.
    fs::write(temp.0.join("meta/existing.toml"), "before\n").unwrap();
    fs::remove_file(temp.0.join("meta/a[1].toml")).unwrap();
    gitlog::unstage_paths(
        &temp.0,
        &[
            PathBuf::from("meta/existing.toml"),
            PathBuf::from("meta/a[1].toml"),
        ],
    )
    .unwrap();
    let staged = git(&temp.0, &["diff", "--cached", "--name-only"]);
    assert_eq!(staged.stdout, b"meta/a1.toml\nnotes.txt\n");
    assert_eq!(
        git(&temp.0, &["show", ":meta/a1.toml"]).stdout,
        b"other managed data\n"
    );
    assert_eq!(
        git(&temp.0, &["show", ":notes.txt"]).stdout,
        b"other foreign data\n"
    );
    assert_eq!(
        fs::read(temp.0.join("meta/existing.toml")).unwrap(),
        b"before\n"
    );
    assert!(!temp.0.join("meta/a[1].toml").exists());
}

#[test]
fn exact_git_apis_reject_directories_and_non_normalized_or_unmanaged_paths() {
    let temp = initialized_vault();
    add_skill(&temp.0, "one", b"one\n");
    gitlog::commit(&temp.0, "Create source tree").unwrap();
    let mut invalid = [
        "",
        "skills",
        "skills/one",
        "skills/../box.toml",
        "./box.toml",
        "skills//one/SKILL.md",
        "skills/one/./SKILL.md",
        "skills/one/SKILL.md/",
        "dist/generated",
        ".DS_Store",
        "skills/.DS_Store",
        "meta/.DS_Store",
        ".git/config",
    ]
    .map(PathBuf::from)
    .to_vec();
    invalid.push(temp.0.join("box.toml"));
    for path in invalid {
        let before = snapshot(&temp.0);
        for error in [
            gitlog::commit_paths(&temp.0, "Invalid path", std::slice::from_ref(&path)).unwrap_err(),
            gitlog::unstage_paths(&temp.0, std::slice::from_ref(&path)).unwrap_err(),
        ] {
            assert_eq!(error.code, "INVALID_GIT_PATH", "{}", path.display());
        }
        assert_eq!(snapshot(&temp.0), before);
    }
    fs::remove_dir_all(temp.0.join("skills/one")).unwrap();
    git(&temp.0, &["rm", "--cached", "-r", "skills/one"]);
    let before = snapshot(&temp.0);
    let directory = [PathBuf::from("skills/one")];
    assert_eq!(
        gitlog::commit_paths(&temp.0, "Missing directory", &directory)
            .unwrap_err()
            .code,
        "INVALID_GIT_PATH"
    );
    assert_eq!(
        gitlog::unstage_paths(&temp.0, &directory).unwrap_err().code,
        "INVALID_GIT_PATH"
    );
    assert_eq!(snapshot(&temp.0), before);
}
