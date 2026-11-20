mod support;
use kinana_core::{
    ImportOptions, agents, import,
    projects::{self, Control, Request},
    vault::Vault,
};
use std::{fs, path::PathBuf, process::Command};
#[test]
fn cursor_detection_child() {
    let Some(mode) = std::env::var_os("KINANA_CURSOR_DETECTION_TEST") else {
        return;
    };
    let detected = agents::detect(&PathBuf::from("/test-home"));
    assert_eq!(
        detected.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
        ["claude", "codex", "cursor"]
    );
    let cursor = &detected[2];
    assert_eq!(cursor.project_skills, ".agents/skills");
    assert_eq!(
        cursor.global_skills,
        PathBuf::from("/test-home/.agents/skills")
    );
    assert_eq!(
        agents::executable_in_path("cursor-agent").is_some(),
        mode != "generic"
    );
    assert_eq!(
        agents::executable_in_path("cursor").is_some(),
        mode == "editor"
    );
    #[cfg(target_os = "macos")]
    let editor_installed =
        std::path::Path::new("/Applications/Cursor.app/Contents/MacOS/Cursor").is_file();
    #[cfg(not(target_os = "macos"))]
    let editor_installed = false;
    assert_eq!(cursor.available, mode == "editor" || editor_installed);
}
#[cfg(unix)]
#[test]
fn only_the_cursor_editor_launcher_or_app_establishes_editor_support_without_execution() {
    use std::os::unix::fs::PermissionsExt;
    let root = support::temp_dir().unwrap();
    let marker = root.join("executed");
    let bin = root.join("agent");
    fs::write(&bin, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
    for mode in ["generic", "cli-only", "editor"] {
        if mode == "cli-only" {
            fs::copy(&bin, root.join("cursor-agent")).unwrap();
        }
        if mode == "editor" {
            fs::copy(&bin, root.join("cursor")).unwrap();
        }
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "cursor_detection_child", "--nocapture"])
            .env("PATH", &root)
            .env("KINANA_CURSOR_DETECTION_TEST", mode)
            .status()
            .unwrap();
        assert!(status.success());
        assert!(!marker.exists());
    }
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn cursor_foreign_nodes_manual_mode_and_global_skill_roots_are_preserved() {
    let root = support::temp_dir().unwrap();
    let v = Vault::new(root.join("vault"), root.join("state/index.db"));
    v.init(false).unwrap();
    let source = root.join("source");
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: probe\ndescription: Check spacing\n---\nRead this reference.\n",
    )
    .unwrap();
    import::add(
        &v,
        source.to_str().unwrap(),
        &root.join("scratch"),
        &ImportOptions::default(),
    )
    .unwrap();
    let project = root.join("project");
    let foreign = project.join(".agents/skills/probe");
    fs::create_dir_all(&foreign).unwrap();
    fs::write(foreign.join("keep"), "user owned").unwrap();
    let request = Request::Use {
        project: project.clone(),
        selections: vec!["probe".into()],
        agent: "cursor".into(),
        mode: "link".into(),
        include_secondary: vec![],
    };
    assert_eq!(
        projects::preview(&v, &root, &request).unwrap_err().code,
        "ACTIVATION_TARGET_EXISTS"
    );
    assert_eq!(
        fs::read_to_string(foreign.join("keep")).unwrap(),
        "user owned"
    );
    assert!(!project.join(".kinana.lock").exists());
    fs::remove_dir_all(&foreign).unwrap();
    let meta = v.root.join("meta/probe.toml");
    let before = fs::read_to_string(&meta).unwrap();
    fs::write(&meta, format!("{before}\n[invocation]\nmode='manual'\n")).unwrap();
    // A controlled fixture commit isolates the manual preflight from dirty-vault validation.
    kinana_core::gitlog::commit_paths(
        &v.root,
        "Prepare invocation fixture",
        &["meta/probe.toml".into()],
    )
    .unwrap();
    assert_eq!(
        projects::preview(&v, &root, &request).unwrap_err().code,
        "CURSOR_MANUAL_UNVERIFIED"
    );
    let global = root.join(".cursor/skills");
    fs::create_dir_all(&global).unwrap();
    let protected = Request::Use {
        project: global.clone(),
        selections: vec!["probe".into()],
        agent: "cursor".into(),
        mode: "link".into(),
        include_secondary: vec![],
    };
    assert_eq!(
        projects::preview(&v, &root, &protected).unwrap_err().code,
        "PROTECTED_PROJECT"
    );
    assert!(fs::read_dir(global).unwrap().next().is_none());
    let control = Control::default();
    assert!(!control.is_committing());
    assert!(!project.join(".kinana.lock").exists());
    fs::remove_dir_all(root).unwrap();
}
