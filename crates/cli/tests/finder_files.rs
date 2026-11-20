//! Finder `.DS_Store` files that Git does not track and that are a skill's
//! only difference from its recorded hash: `build` and `undo` remove them and
//! say so (owner decision 2026-11-17). The hash definition is unchanged.
#[allow(dead_code)]
mod support;
use serde_json::Value;
use std::{fs, path::Path, process::Command};
use support::Sandbox;

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

fn refused(s: &Sandbox, args: &[&str]) -> String {
    let output = s.run(&[args, &["--json"]].concat());
    assert!(!output.status.success(), "{args:?} succeeded");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    value["error"]["code"].as_str().unwrap().to_owned()
}

fn add(s: &Sandbox, id: &str, with_finder_file: bool) {
    let source = s.root.join(format!("source-{id}"));
    fs::create_dir_all(source.join("assets")).unwrap();
    fs::write(
        source.join("SKILL.md"),
        format!("---\nname: {id}\ndescription: Review API contracts\n---\n# Review\nText.\n"),
    )
    .unwrap();
    fs::write(source.join("assets/a.txt"), "asset").unwrap();
    if with_finder_file {
        fs::write(source.join(".DS_Store"), "imported finder bytes").unwrap();
    }
    json(s, &["add", source.to_str().unwrap()]);
}

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}");
    String::from_utf8(output.stdout).unwrap()
}

fn finder(s: &Sandbox, relative: &str) -> std::path::PathBuf {
    let path = s.vault.join(relative);
    fs::write(&path, "\0\0\0\x01Bud1 written by Finder").unwrap();
    path
}

fn removed(value: &Value) -> Vec<String> {
    value
        .get("finder_files_removed")
        .map(|files| serde_json::from_value(files.clone()).unwrap())
        .unwrap_or_default()
}

#[test]
fn build_removes_finder_files_that_are_a_skills_only_difference_and_names_them() {
    let s = Sandbox::new();
    s.initialize();
    add(&s, "api", false);
    let head = git(&s.vault, &["rev-parse", "HEAD"]);
    let top = finder(&s, "skills/api/.DS_Store");
    let nested = finder(&s, "skills/api/assets/.DS_Store");

    let preview = json(&s, &["build", "--dry-run"]);
    assert_eq!(
        removed(&preview),
        ["skills/api/.DS_Store", "skills/api/assets/.DS_Store"]
    );
    assert!(top.exists() && nested.exists(), "a dry run removes nothing");

    let output = s.run(&["build"]);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains("Removed Finder .DS_Store file(s) that were the only difference in their skill: skills/api/.DS_Store, skills/api/assets/.DS_Store"),
        "{text}"
    );
    assert!(!top.exists() && !nested.exists());
    // Untracked files only: no commit, nothing for undo to revert.
    assert_eq!(git(&s.vault, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        git(&s.vault, &["status", "--porcelain", "--ignored"]),
        "!! dist/\n"
    );
    let again = json(&s, &["build"]);
    assert!(again.get("finder_files_removed").is_none(), "{again}");
}

#[test]
fn a_finder_file_beside_any_other_change_is_kept_and_build_still_refuses() {
    let s = Sandbox::new();
    s.initialize();
    add(&s, "api", false);
    let leftover = finder(&s, "skills/api/.DS_Store");
    fs::write(s.vault.join("skills/api/assets/a.txt"), "edited").unwrap();
    for args in [
        &["build", "--dry-run"][..],
        &["build"],
        &["undo"],
        &["undo", "--dry-run"],
    ] {
        let code = refused(&s, args);
        assert!(
            code == "SOURCE_HASH_MISMATCH" || code == "GIT_DIRTY_VAULT",
            "{args:?}: {code}"
        );
        assert!(
            leftover.exists(),
            "{args:?} removed a file it could not justify"
        );
    }
}

#[test]
fn a_tracked_finder_file_is_imported_content_and_is_never_removed() {
    let s = Sandbox::new();
    s.initialize();
    add(&s, "api", true);
    let imported = s.vault.join("skills/api/.DS_Store");
    assert!(git(&s.vault, &["ls-files", "skills/api"]).contains("skills/api/.DS_Store"));
    let report = json(&s, &["build"]);
    assert!(report.get("finder_files_removed").is_none());
    assert_eq!(
        fs::read_to_string(&imported).unwrap(),
        "imported finder bytes"
    );
    // Finder rewriting a tracked one is a real change, never swept.
    fs::write(&imported, "rewritten by Finder").unwrap();
    assert_eq!(refused(&s, &["build"]), "SOURCE_HASH_MISMATCH");
    assert_eq!(
        fs::read_to_string(&imported).unwrap(),
        "rewritten by Finder"
    );
}

#[test]
fn undo_removes_them_only_when_the_undo_itself_can_go_ahead() {
    let s = Sandbox::new();
    s.initialize();
    add(&s, "api", false);
    add(&s, "alternate", false);
    json(&s, &["pin", "api"]);
    let pinned = git(&s.vault, &["rev-parse", "HEAD"]);
    let first = finder(&s, "skills/api/.DS_Store");
    let second = finder(&s, "skills/alternate/assets/.DS_Store");

    let preview = json(&s, &["undo", "--dry-run"]);
    assert_eq!(
        removed(&preview),
        ["skills/alternate/assets/.DS_Store", "skills/api/.DS_Store"]
    );
    assert!(first.exists() && second.exists());

    // A commit Kinana did not make cannot be undone, so nothing is removed.
    fs::write(s.vault.join("notes.txt"), "mine").unwrap();
    git(&s.vault, &["add", "notes.txt"]);
    git(
        &s.vault,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@localhost",
            "commit",
            "-qm",
            "notes",
        ],
    );
    assert_eq!(refused(&s, &["undo"]), "UNDO_UNMANAGED_COMMIT");
    assert!(first.exists() && second.exists());
    git(&s.vault, &["reset", "-q", "--hard", pinned.trim()]);

    let output = s.run(&["undo"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Removed Finder .DS_Store file(s)"), "{text}");
    assert!(!first.exists() && !second.exists());
    assert_eq!(
        git(&s.vault, &["log", "-1", "--format=%s"]).trim(),
        format!(
            "Revert \"{}\"",
            git(&s.vault, &["log", "-1", "--skip=1", "--format=%s"]).trim()
        )
    );
    assert_eq!(git(&s.vault, &["status", "--porcelain"]), "");
}

#[test]
fn undo_of_an_import_takes_the_skill_and_its_finder_file_away() {
    let s = Sandbox::new();
    s.initialize();
    add(&s, "api", false);
    finder(&s, "skills/api/.DS_Store");
    let report = json(&s, &["undo"]);
    assert_eq!(removed(&report), ["skills/api/.DS_Store"]);
    assert!(!s.vault.join("skills/api").exists());
    assert_eq!(
        git(&s.vault, &["status", "--porcelain", "--ignored"]),
        "!! dist/\n"
    );
}

fn refusal(s: &Sandbox, args: &[&str]) -> (String, String) {
    let output = s.run(&[args, &["--json"]].concat());
    assert!(!output.status.success(), "{args:?} succeeded");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    (
        value["error"]["code"].as_str().unwrap().to_owned(),
        value["error"]["message"].as_str().unwrap().to_owned(),
    )
}

/// Owner decision 2026-11-17, sixth round: `use` deletes nothing itself, but
/// when untracked Finder files are a skill's only difference its refusal
/// names them and suggests `kinana build`; with any other difference it
/// refuses as before, without the suggestion.
#[test]
fn use_suggests_build_only_when_finder_files_are_the_only_difference() {
    let s = Sandbox::new();
    s.initialize();
    add(&s, "api", false);
    let project = s.root.join("project");
    fs::create_dir(&project).unwrap();
    let project = project.to_str().unwrap();
    let args = ["use", "api", "--project", project, "--agent", "claude"];
    let leftover = finder(&s, "skills/api/assets/.DS_Store");
    let before = support::snapshot(&s.root);

    let (code, message) = refusal(&s, &args);
    assert_eq!(code, "SOURCE_HASH_MISMATCH", "{message}");
    assert!(
        message.contains("skills/api/assets/.DS_Store") && message.contains("kinana build api "),
        "{message}"
    );
    assert_eq!(support::snapshot(&s.root), before, "use wrote something");

    fs::write(s.vault.join("skills/api/assets/a.txt"), "edited").unwrap();
    let (code, message) = refusal(&s, &args);
    assert_eq!(code, "SOURCE_HASH_MISMATCH", "{message}");
    assert!(!message.contains("kinana build"), "{message}");
    assert!(!message.contains(".DS_Store"), "{message}");
    assert!(leftover.exists());

    git(&s.vault, &["checkout", "--", "skills/api/assets/a.txt"]);
    assert_eq!(
        removed(&json(&s, &["build"])),
        ["skills/api/assets/.DS_Store"]
    );
    json(&s, &args);
}

/// Automated security review of 7ffafd2: names in the `use` hint and in the
/// Finder note of `build` come from the skill's author and are shown in the
/// shared escape form, so a direction mark cannot make them look like
/// another path.
#[test]
fn finder_paths_in_messages_are_shown_escaped() {
    let s = Sandbox::new();
    s.initialize();
    let source = s.root.join("source-bidi");
    let folder = "x\u{202E}y";
    fs::create_dir_all(source.join(folder)).unwrap();
    fs::write(source.join(folder).join("a.txt"), "asset").unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: bidi\ndescription: Review API contracts\n---\n# Review\n",
    )
    .unwrap();
    json(&s, &["add", source.to_str().unwrap()]);
    finder(&s, &format!("skills/bidi/{folder}/.DS_Store"));
    let project = s.root.join("project");
    fs::create_dir(&project).unwrap();
    let (code, message) = refusal(
        &s,
        &[
            "use",
            "bidi",
            "--project",
            project.to_str().unwrap(),
            "--agent",
            "claude",
        ],
    );
    assert_eq!(code, "SOURCE_HASH_MISMATCH", "{message}");
    assert!(
        message.contains("skills/bidi/x\\u{202e}y/.DS_Store"),
        "{message:?}"
    );
    assert!(!message.contains('\u{202E}'), "{message:?}");

    let output = s.run(&["build"]);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains("skills/bidi/x\\u{202e}y/.DS_Store"),
        "{text:?}"
    );
    assert!(!text.contains('\u{202E}'), "{text:?}");
}

/// Review 2026-11-17 (sixth round), F2: the hint names the skills to build,
/// because a plain `kinana build` covers every skill and stops at another
/// skill's real difference.
#[test]
fn the_use_hint_names_the_skills_so_another_skills_change_does_not_block_it() {
    let s = Sandbox::new();
    s.initialize();
    add(&s, "api", false);
    add(&s, "web", false);
    finder(&s, "skills/api/.DS_Store");
    fs::write(s.vault.join("skills/web/assets/a.txt"), "edited").unwrap();
    let project = s.root.join("project");
    fs::create_dir(&project).unwrap();
    let (code, message) = refusal(
        &s,
        &[
            "use",
            "api",
            "--project",
            project.to_str().unwrap(),
            "--agent",
            "claude",
        ],
    );
    assert_eq!(code, "SOURCE_HASH_MISMATCH", "{message}");
    assert!(
        message.contains("Run kinana build api to remove"),
        "{message}"
    );
    assert_eq!(
        removed(&json(&s, &["build", "api"])),
        ["skills/api/.DS_Store"]
    );
}
