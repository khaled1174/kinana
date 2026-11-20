//! `kinana doctor --repair` (owner decisions 2026-11-17, fourth and sixth
//! rounds): a preview that writes nothing, then, with its token, a backup of
//! everything replaced in `~/.kinana/repair-backups/<timestamp>/` and the
//! repair itself, with paths passed to Git byte for byte.
#[allow(dead_code)]
mod support;
use serde_json::Value;
use std::{fs, path::Path, process::Command};
use support::{Sandbox, snapshot};

fn json(s: &Sandbox, args: &[&str]) -> Value {
    let output = s.run(&[args, &["--json"]].concat());
    assert!(
        output.status.success(),
        "{args:?}: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn refused(s: &Sandbox, args: &[&str]) -> (i32, String, String) {
    let output = s.run(&[args, &["--json"]].concat());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    (
        output.status.code().unwrap(),
        value["error"]["code"].as_str().unwrap().to_owned(),
        value["error"]["message"].as_str().unwrap().to_owned(),
    )
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

fn strings(value: &Value) -> Vec<String> {
    serde_json::from_value(value.clone()).unwrap()
}

/// A vault with one skill `api` holding files, a nested asset and a recorded
/// empty directory.
fn setup() -> Sandbox {
    let s = Sandbox::new();
    s.initialize();
    let source = s.root.join("source-api");
    fs::create_dir_all(source.join("assets/empty/deep")).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: api\ndescription: Review API contracts\n---\n# Review\n",
    )
    .unwrap();
    fs::write(source.join("assets/a.txt"), "asset\n").unwrap();
    json(&s, &["add", source.to_str().unwrap()]);
    json(&s, &["build"]);
    s
}

fn preview(s: &Sandbox) -> Value {
    json(s, &["doctor", "--repair"])
}

fn confirm(s: &Sandbox, report: &Value) -> Value {
    json(
        s,
        &[
            "doctor",
            "--repair",
            "--confirm",
            report["preview_token"].as_str().unwrap(),
        ],
    )
}

fn interrupted_passes(report: &Value) -> bool {
    report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["name"] == "interrupted_change")
        .unwrap()["passed"]
        .as_bool()
        .unwrap()
}

#[test]
fn preview_writes_nothing_and_confirm_backs_up_then_restores_exactly() {
    let s = setup();
    let skill = s.vault.join("skills/api");
    let original = fs::read(skill.join("assets/a.txt")).unwrap();
    fs::write(skill.join("assets/a.txt"), "changed in place\n").unwrap();
    fs::remove_file(skill.join("SKILL.md")).unwrap();
    fs::remove_dir_all(skill.join("assets/empty")).unwrap();
    fs::write(s.vault.join("bundles/staged.toml"), "name = \"one\"\n").unwrap();
    git(&s.vault, &["add", "--", "bundles/staged.toml"]);
    fs::write(s.vault.join("notes.txt"), "the user's own file\n").unwrap();

    let before = snapshot(&s.root);
    let plan = preview(&s);
    let dry = json(&s, &["doctor", "--repair", "--dry-run"]);
    assert_eq!(snapshot(&s.root), before, "the preview wrote something");
    assert_eq!(plan["dry_run"], true);
    assert_eq!(dry["restore"], plan["restore"]);
    assert_eq!(
        strings(&plan["restore"]),
        ["skills/api/SKILL.md", "skills/api/assets/a.txt"]
    );
    assert_eq!(strings(&plan["unstage"]), ["bundles/staged.toml"]);
    assert_eq!(strings(&plan["untracked"]), ["notes.txt"]);
    assert_eq!(
        strings(&plan["recreate_directories"]),
        ["skills/api/assets/empty/deep"]
    );
    assert!(plan["after"].is_null());
    let folder = plan["backup_directory"].as_str().unwrap();
    assert!(
        folder.starts_with(s.home.join(".kinana/repair-backups/").to_str().unwrap()),
        "{folder}"
    );
    // --dry-run with the token is still a preview.
    let token = plan["preview_token"].as_str().unwrap();
    json(&s, &["doctor", "--repair", "--confirm", token, "--dry-run"]);
    assert_eq!(snapshot(&s.root), before);

    let done = confirm(&s, &plan);
    assert_eq!(done["dry_run"], false);
    assert_eq!(done["backup_directory"], plan["backup_directory"]);
    assert_eq!(fs::read(skill.join("assets/a.txt")).unwrap(), original);
    assert!(skill.join("SKILL.md").is_file());
    assert!(skill.join("assets/empty/deep").is_dir());
    // Unstaged, and the bytes stay on disk; the user's file is untouched.
    assert_eq!(
        fs::read_to_string(s.vault.join("bundles/staged.toml")).unwrap(),
        "name = \"one\"\n"
    );
    assert!(git(&s.vault, &["diff", "--cached", "--name-only"]).is_empty());
    assert_eq!(
        fs::read_to_string(s.vault.join("notes.txt")).unwrap(),
        "the user's own file\n"
    );

    // The backup: the file changed in place, with its bytes, private.
    let backup = Path::new(folder);
    assert_eq!(strings(&done["backed_up"]), ["skills/api/assets/a.txt"]);
    assert_eq!(
        fs::read_to_string(backup.join("vault/skills/api/assets/a.txt")).unwrap(),
        "changed in place\n"
    );
    assert!(backup.join("git-index").is_file());
    assert!(
        fs::read_to_string(backup.join("MANIFEST.txt"))
            .unwrap()
            .contains("vault/skills/api/assets/a.txt")
    );
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(backup), 0o700);
        assert_eq!(mode(&backup.join("vault/skills/api")), 0o700);
        assert_eq!(mode(&backup.join("vault/skills/api/assets/a.txt")), 0o600);
    }
    // Only the user's own files are left for doctor to name.
    fs::remove_file(s.vault.join("bundles/staged.toml")).unwrap();
    fs::remove_file(s.vault.join("notes.txt")).unwrap();
    assert!(interrupted_passes(&json(&s, &["doctor"])));
    // Kinana never deletes the backup.
    assert!(backup.join("vault/skills/api/assets/a.txt").is_file());
}

#[test]
fn a_change_after_the_preview_is_refused_without_writing() {
    let s = setup();
    let file = s.vault.join("skills/api/assets/a.txt");
    fs::write(&file, "one\n").unwrap();
    let plan = preview(&s);
    fs::write(&file, "two\n").unwrap();
    let before = snapshot(&s.root);
    let token = plan["preview_token"].as_str().unwrap();
    let (exit, code, _) = refused(&s, &["doctor", "--repair", "--confirm", token]);
    assert_eq!((exit, code.as_str()), (2, "DOCTOR_REPAIR_STALE"));
    assert_eq!(snapshot(&s.root), before);
    assert!(!s.home.join(".kinana/repair-backups").exists());
}

#[test]
fn a_token_that_is_not_a_preview_token_is_refused_without_writing() {
    let s = setup();
    fs::write(s.vault.join("skills/api/assets/a.txt"), "one\n").unwrap();
    let before = snapshot(&s.root);
    for token in [
        "../../escape.0000000000000000000000000000000000000000000000000000000000000000",
        "20261117T120000Z.not-hex",
        "",
    ] {
        let (exit, code, _) = refused(&s, &["doctor", "--repair", "--confirm", token]);
        assert_eq!((exit, code.as_str()), (2, "DOCTOR_REPAIR_STALE"), "{token}");
    }
    assert_eq!(snapshot(&s.root), before);
}

#[test]
fn more_than_ten_files_are_all_listed_and_all_restored() {
    let s = setup();
    for n in 0..14 {
        fs::write(
            s.vault.join(format!("bundles/b{n:02}.toml")),
            "name = \"b\"\n",
        )
        .unwrap();
    }
    git(&s.vault, &["add", "--", "bundles"]);
    git(&s.vault, &["commit", "-qm", "bundles"]);
    for n in 0..14 {
        fs::write(s.vault.join(format!("bundles/b{n:02}.toml")), "changed\n").unwrap();
    }
    let plan = preview(&s);
    assert_eq!(plan["restore"].as_array().unwrap().len(), 14);
    let text = String::from_utf8(s.run(&["doctor", "--repair"]).stdout).unwrap();
    assert!(text.contains("bundles/b13.toml"), "{text}");
    confirm(&s, &plan);
    for n in 0..14 {
        assert_eq!(
            fs::read_to_string(s.vault.join(format!("bundles/b{n:02}.toml"))).unwrap(),
            "name = \"b\"\n"
        );
    }
}

/// The fourth review's medium finding, closed at the source: a name with a
/// direction mark, a line separator, a quote, a newline or Arabic letters is
/// restored byte for byte, and the look-alike beside it is not touched.
#[test]
fn hostile_names_are_shown_escaped_and_the_right_file_is_restored() {
    let s = Sandbox::new();
    s.initialize();
    let source = s.root.join("source-names");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: names\ndescription: Names that reorder text\n---\n# Body\n",
    )
    .unwrap();
    let names = [
        "a\u{202E}b.txt",
        "c\u{2028}d.txt",
        "it's \"quoted\".txt",
        "line\nbreak.txt",
        "ملف عربي.txt",
        "soft\u{AD}hyphen.txt",
    ];
    for name in names {
        fs::write(source.join(name), format!("original {name}")).unwrap();
    }
    fs::write(source.join("a b.txt"), "the look-alike").unwrap();
    json(&s, &["add", source.to_str().unwrap()]);
    let skill = s.vault.join("skills/names");
    for name in names {
        fs::write(skill.join(name), "changed").unwrap();
    }
    fs::write(skill.join("a b.txt"), "the look-alike, changed by the user").unwrap();
    git(&s.vault, &["add", "--", "skills/names/a b.txt"]);
    git(&s.vault, &["commit", "-qm", "user change"]);

    let text = String::from_utf8(s.run(&["doctor", "--repair"]).stdout).unwrap();
    for character in ['\u{202E}', '\u{2028}', '\u{AD}'] {
        assert!(!text.contains(character), "{text:?}");
    }
    assert!(text.contains("a\\u{202e}b.txt"), "{text}");
    assert!(text.contains("line\\u{a}break.txt"), "{text}");
    assert!(text.contains("kinana doctor --repair --confirm "), "{text}");
    let plan = preview(&s);
    assert_eq!(plan["restore"].as_array().unwrap().len(), names.len());
    confirm(&s, &plan);
    for name in names {
        assert_eq!(
            fs::read_to_string(skill.join(name)).unwrap(),
            format!("original {name}"),
            "{name:?}"
        );
    }
    assert_eq!(
        fs::read_to_string(skill.join("a b.txt")).unwrap(),
        "the look-alike, changed by the user"
    );
}

#[test]
fn what_replaced_a_tracked_file_is_named_and_left_alone() {
    let s = setup();
    let skill = s.vault.join("skills/api");
    fs::remove_file(skill.join("SKILL.md")).unwrap();
    fs::create_dir(skill.join("SKILL.md")).unwrap();
    fs::write(skill.join("SKILL.md/notes.txt"), "precious\n").unwrap();
    fs::write(skill.join("assets/a.txt"), "changed\n").unwrap();
    let plan = preview(&s);
    assert_eq!(strings(&plan["blocked"]), ["skills/api/SKILL.md"]);
    assert_eq!(strings(&plan["restore"]), ["skills/api/assets/a.txt"]);
    confirm(&s, &plan);
    assert_eq!(
        fs::read_to_string(skill.join("SKILL.md/notes.txt")).unwrap(),
        "precious\n"
    );
    assert_eq!(
        fs::read_to_string(skill.join("assets/a.txt")).unwrap(),
        "asset\n"
    );
}

#[test]
fn private_stages_are_moved_into_the_backup_not_deleted() {
    let s = setup();
    let stage = s.vault.join("dist/.skillsbox-activation-1-2");
    fs::create_dir_all(stage.join("api")).unwrap();
    fs::write(stage.join("api/SKILL.md"), "moved by remove\n").unwrap();
    std::os::unix::fs::symlink("/etc/passwd", stage.join("api/link")).unwrap();
    let plan = preview(&s);
    assert_eq!(
        strings(&plan["staging_removed"]),
        ["dist/.skillsbox-activation-1-2"]
    );
    let done = confirm(&s, &plan);
    assert!(!stage.exists());
    let kept = Path::new(done["backup_directory"].as_str().unwrap())
        .join("stages/dist/.skillsbox-activation-1-2/api");
    assert_eq!(
        fs::read_to_string(kept.join("SKILL.md")).unwrap(),
        "moved by remove\n"
    );
    // The link is kept as a link, never followed.
    assert_eq!(
        fs::read_link(kept.join("link")).unwrap(),
        Path::new("/etc/passwd")
    );
    assert!(interrupted_passes(&done["after"]));
}

#[test]
fn a_failed_backup_leaves_the_vault_untouched() {
    let s = setup();
    fs::write(s.vault.join("skills/api/assets/a.txt"), "changed\n").unwrap();
    fs::write(s.home.join(".kinana/repair-backups"), "not a directory").unwrap();
    let plan = preview(&s);
    let before = snapshot(&s.vault);
    let token = plan["preview_token"].as_str().unwrap();
    let (exit, code, message) = refused(&s, &["doctor", "--repair", "--confirm", token]);
    assert_eq!(
        (exit, code.as_str()),
        (2, "DOCTOR_REPAIR_BACKUP_FAILED"),
        "{message}"
    );
    assert_eq!(snapshot(&s.vault), before);
}

#[test]
fn a_healthy_vault_has_nothing_to_repair() {
    let s = setup();
    let plan = preview(&s);
    assert_eq!(plan["preview_token"], "");
    assert!(plan["backup_directory"].is_null());
    let text = String::from_utf8(s.run(&["doctor", "--repair"]).stdout).unwrap();
    assert_eq!(text.trim(), "Nothing to repair.");
}

/// Automated security review of 4de5068: a `.gitattributes` from a skill's
/// author could make the restore convert bytes (line endings, encoding).
/// The restore installs the same attribute guard as every other Git write,
/// so the file returns byte for byte as the last commit has it.
#[test]
fn a_gitattributes_from_a_skill_cannot_convert_restored_bytes() {
    let s = Sandbox::new();
    s.initialize();
    let source = s.root.join("source-attributes");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: attributes\ndescription: Line endings\n---\n# Body\n",
    )
    .unwrap();
    fs::write(source.join(".gitattributes"), "*.txt text eol=crlf\n").unwrap();
    fs::write(source.join("data.txt"), "one\ntwo\n").unwrap();
    json(&s, &["add", source.to_str().unwrap()]);
    let file = s.vault.join("skills/attributes/data.txt");
    let committed = git(
        &s.vault,
        &["cat-file", "blob", "HEAD:skills/attributes/data.txt"],
    );
    fs::write(&file, "changed\n").unwrap();
    // A vault whose guard is gone (written by another Git client, say).
    fs::remove_file(s.vault.join(".git/info/attributes")).unwrap();
    let plan = preview(&s);
    confirm(&s, &plan);
    assert_eq!(fs::read_to_string(&file).unwrap(), committed);
}

/// Review of 4de5068, F1 (high): a `dist` that is a link led stage listing
/// and deletion outside the vault. A stage counts only under a real
/// directory, and nothing outside the vault is listed, copied or deleted.
#[test]
fn a_dist_link_never_leads_the_repair_outside_the_vault() {
    let s = setup();
    let outside = s.root.join("outside");
    fs::create_dir_all(outside.join(".skillsbox-victim")).unwrap();
    fs::write(outside.join(".skillsbox-victim/data.txt"), "precious\n").unwrap();
    fs::remove_dir_all(s.vault.join("dist")).unwrap();
    std::os::unix::fs::symlink(&outside, s.vault.join("dist")).unwrap();
    fs::write(s.vault.join("skills/api/assets/a.txt"), "changed\n").unwrap();
    let plan = preview(&s);
    assert!(
        strings(&plan["staging_removed"]).is_empty(),
        "{}",
        plan["staging_removed"]
    );
    confirm(&s, &plan);
    assert_eq!(
        fs::read_to_string(outside.join(".skillsbox-victim/data.txt")).unwrap(),
        "precious\n"
    );
}

/// F5: a stage that is itself a link is moved as a link; nothing it points
/// to is read, hashed or listed.
#[test]
fn a_stage_that_is_a_link_is_not_followed() {
    let s = setup();
    let outside = s.root.join("outside");
    fs::create_dir_all(outside.join("locked")).unwrap();
    fs::write(outside.join("secret.txt"), "secret\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(outside.join("locked"), fs::Permissions::from_mode(0o000)).unwrap();
    }
    std::os::unix::fs::symlink(&outside, s.vault.join(".skillsbox-evil")).unwrap();
    let plan = preview(&s);
    let done = confirm(&s, &plan);
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(outside.join("locked"), fs::Permissions::from_mode(0o700)).unwrap();
    }
    assert!(fs::symlink_metadata(s.vault.join(".skillsbox-evil")).is_err());
    assert_eq!(
        fs::read_to_string(outside.join("secret.txt")).unwrap(),
        "secret\n"
    );
    let manifest = fs::read_to_string(
        Path::new(done["backup_directory"].as_str().unwrap()).join("MANIFEST.txt"),
    )
    .unwrap();
    assert!(!manifest.contains("secret.txt"), "{manifest}");
}

/// F2: content that exists only in Git's index (staged, then the file
/// removed) is copied to the backup before the index entry is reset.
#[test]
fn staged_only_content_is_in_the_backup() {
    let s = setup();
    let file = s.vault.join("skills/api/assets/a.txt");
    fs::write(&file, "STAGED ONLY CONTENT\n").unwrap();
    git(&s.vault, &["add", "--", "skills/api/assets/a.txt"]);
    fs::remove_file(&file).unwrap();
    fs::write(s.vault.join("bundles/new.toml"), "NEW STAGED FILE\n").unwrap();
    git(&s.vault, &["add", "--", "bundles/new.toml"]);
    fs::remove_file(s.vault.join("bundles/new.toml")).unwrap();
    let plan = preview(&s);
    let done = confirm(&s, &plan);
    let backup = Path::new(done["backup_directory"].as_str().unwrap());
    assert_eq!(
        fs::read_to_string(backup.join("index/skills/api/assets/a.txt")).unwrap(),
        "STAGED ONLY CONTENT\n"
    );
    assert_eq!(
        fs::read_to_string(backup.join("index/bundles/new.toml")).unwrap(),
        "NEW STAGED FILE\n"
    );
}

/// F3: a private stage at the vault root is moved, so its files are not also
/// reported as left where they are.
#[test]
fn files_in_a_stage_are_not_reported_as_left_in_place() {
    let s = setup();
    fs::create_dir(s.vault.join(".skillsbox-abc")).unwrap();
    fs::write(s.vault.join(".skillsbox-abc/f"), "moved\n").unwrap();
    let plan = preview(&s);
    assert_eq!(strings(&plan["staging_removed"]), [".skillsbox-abc"]);
    assert!(
        strings(&plan["untracked"]).is_empty(),
        "{}",
        plan["untracked"]
    );
}

/// F4: a failure after the vault was touched names the backup folder.
#[test]
fn a_failure_part_way_names_the_backup_folder() {
    use std::os::unix::fs::PermissionsExt;
    let s = setup();
    fs::write(s.vault.join("skills/api/assets/a.txt"), "changed\n").unwrap();
    let stage = s.vault.join(".skillsbox-stage/sub");
    fs::create_dir_all(&stage).unwrap();
    fs::write(stage.join("f"), "x\n").unwrap();
    fs::set_permissions(&stage, fs::Permissions::from_mode(0o500)).unwrap();
    let plan = preview(&s);
    let token = plan["preview_token"].as_str().unwrap();
    let (_, _, message) = refused(&s, &["doctor", "--repair", "--confirm", token]);
    fs::set_permissions(&stage, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        message.contains(plan["backup_directory"].as_str().unwrap()),
        "{message}"
    );
    assert!(message.contains("kinana doctor"), "{message}");
}

/// Low: no ~/.kinana yet; the confirmed repair creates it (private) for the
/// backup instead of failing with an opaque error.
#[test]
fn the_backup_creates_the_state_folder_when_it_is_missing() {
    let s = setup();
    fs::write(s.vault.join("skills/api/assets/a.txt"), "changed\n").unwrap();
    fs::remove_dir_all(s.home.join(".kinana")).unwrap();
    let plan = preview(&s);
    let done = confirm(&s, &plan);
    assert!(Path::new(done["backup_directory"].as_str().unwrap()).is_dir());
}
