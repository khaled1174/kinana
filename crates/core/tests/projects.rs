mod support;
use kinana_core::{
    ImportOptions, import,
    projects::{self, Control, Request},
    vault::Vault,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
struct Fixture {
    root: PathBuf,
    vault: Vault,
    project: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = support::temp_dir().unwrap();
        let vault = Vault::new(root.join("vault"), root.join("state/index.db"));
        vault.init(false).unwrap();
        let source = root.join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("SKILL.md"),"---\nname: test-reference\ndescription: Review local design\n---\nRead this local reference.\n").unwrap();
        import::add(
            &vault,
            source.to_str().unwrap(),
            &root.join("scratch"),
            &ImportOptions::default(),
        )
        .unwrap();
        let project = root.join("project");
        fs::create_dir(&project).unwrap();
        fs::write(project.join("README.md"), "unchanged\n").unwrap();
        Self {
            root,
            vault,
            project,
        }
    }
    fn request(&self, agent: &str, mode: &str) -> Request {
        Request::Use {
            project: self.project.clone(),
            selections: vec!["test-reference".into()],
            agent: agent.into(),
            mode: mode.into(),
            include_secondary: vec![],
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, path: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for e in fs::read_dir(path).unwrap() {
            let path = e.unwrap().path();
            let m = fs::symlink_metadata(&path).unwrap();
            let key = path.strip_prefix(root).unwrap().to_path_buf();
            if m.file_type().is_symlink() {
                out.insert(
                    key,
                    fs::read_link(path)
                        .unwrap()
                        .as_os_str()
                        .as_encoded_bytes()
                        .to_vec(),
                );
            } else if m.is_dir() {
                out.insert(key, b"directory".to_vec());
                walk(root, &path, out);
            } else {
                out.insert(key, fs::read(path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}
#[test]
fn preview_confirmation_and_cancel_are_read_only_then_all_agents_restore_exact_tree() {
    for agent in ["claude", "codex", "cursor"] {
        for mode in ["link", "copy"] {
            let f = Fixture::new();
            let before = tree(&f.root);
            let request = f.request(agent, mode);
            let preview = projects::preview(&f.vault, &f.root, &request).unwrap();
            assert!(preview.changed);
            assert_eq!(tree(&f.root), before);
            assert_eq!(
                projects::apply(&f.vault, &f.root, &request, "", &Control::default())
                    .unwrap_err()
                    .code,
                "PROJECT_CONFIRMATION_REQUIRED"
            );
            let control = Control::default();
            assert!(control.cancel());
            assert_eq!(
                projects::apply(&f.vault, &f.root, &request, &preview.token, &control)
                    .unwrap_err()
                    .code,
                "PROJECT_CANCELLED"
            );
            assert_eq!(tree(&f.root), before);
            projects::apply(
                &f.vault,
                &f.root,
                &request,
                &preview.token,
                &Control::default(),
            )
            .unwrap();
            let listed = projects::list(&f.vault, &[]).unwrap();
            assert_eq!(listed.len(), 1);
            assert_eq!(listed[0].activations[0].state, "current");
            assert_eq!(listed[0].activations[0].mode, mode);
            let request = Request::Unuse {
                project: f.project.clone(),
                ids: vec!["test-reference".into()],
                agent: "all".into(),
            };
            let before_unuse = tree(&f.root);
            let plan = projects::preview(&f.vault, &f.root, &request).unwrap();
            assert_eq!(tree(&f.root), before_unuse);
            projects::apply(
                &f.vault,
                &f.root,
                &request,
                &plan.token,
                &Control::default(),
            )
            .unwrap();
            assert_eq!(
                tree(&f.project),
                BTreeMap::from([(PathBuf::from("README.md"), b"unchanged\n".to_vec())])
            );
        }
    }
}
#[test]
fn stale_preview_and_changed_or_foreign_targets_are_preserved() {
    let f = Fixture::new();
    let request = f.request("claude", "copy");
    let preview = projects::preview(&f.vault, &f.root, &request).unwrap();
    let request = f.request("claude", "link");
    let before = tree(&f.project);
    assert_eq!(
        projects::apply(
            &f.vault,
            &f.root,
            &request,
            &preview.token,
            &Control::default()
        )
        .unwrap_err()
        .code,
        "PROJECT_PREVIEW_CHANGED"
    );
    assert_eq!(tree(&f.project), before);
}
#[test]
fn selected_project_inspection_finds_local_skills_without_writes_or_registration() {
    let f = Fixture::new();
    let target = f.project.join(".claude/skills/foreign");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("SKILL.md"),
        "---\nname: foreign\ndescription: Local only\n---\nRead locally.\n",
    )
    .unwrap();
    let before = tree(&f.root);
    let listed = projects::list(&f.vault, std::slice::from_ref(&f.project)).unwrap();
    assert_eq!(listed[0].local.len(), 1);
    assert_eq!(listed[0].state, "no_lock");
    assert_eq!(tree(&f.root), before);
    assert!(projects::list(&f.vault, &[]).unwrap().is_empty());
    let request = f.request("claude", "link");
    let plan = projects::preview(&f.vault, &f.root, &request).unwrap();
    projects::apply(
        &f.vault,
        &f.root,
        &request,
        &plan.token,
        &Control::default(),
    )
    .unwrap();
    assert!(target.exists());
    let missing = f.root.join("missing");
    assert_eq!(
        projects::list(&f.vault, &[missing]).unwrap()[0].state,
        "IO_ERROR"
    );
}
#[test]
fn approval_changing_generated_content_invalidates_the_previous_preview() {
    let f = Fixture::new();
    let request = f.request("claude", "link");
    let plan = projects::preview(&f.vault, &f.root, &request).unwrap();
    let mut metadata = kinana_core::edit::read_metadata(&f.vault, "test-reference").unwrap();
    metadata.description.improved =
        Some("Review interface design and accessibility when building components.".into());
    metadata.description.active = "improved".into();
    metadata.description.approved_at = Some("2026-11-15T00:00:00Z".into());
    kinana_core::edit::apply(
        &f.vault,
        &BTreeMap::from([("test-reference".into(), metadata)]),
        "Accept fixture description",
        false,
    )
    .unwrap();
    let before = tree(&f.project);
    assert_eq!(
        projects::apply(
            &f.vault,
            &f.root,
            &request,
            &plan.token,
            &Control::default()
        )
        .unwrap_err()
        .code,
        "PROJECT_PREVIEW_CHANGED"
    );
    assert_eq!(tree(&f.project), before);
}
#[cfg(unix)]
#[test]
fn project_inspection_never_follows_an_unmanaged_skill_directory_link() {
    let f = Fixture::new();
    let outside = f.root.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("SKILL.md"), "not a skill to read").unwrap();
    fs::create_dir(f.project.join(".claude")).unwrap();
    std::os::unix::fs::symlink(&outside, f.project.join(".claude/skills")).unwrap();
    let before = tree(&f.root);
    let result = projects::list(&f.vault, std::slice::from_ref(&f.project)).unwrap();
    assert!(result[0].local.is_empty());
    assert!(!result[0].warnings.is_empty());
    assert_eq!(tree(&f.root), before);
}

#[test]
fn only_the_exact_desktop_preview_can_confirm_risk() {
    let f = Fixture::new();
    let source = f.root.join("risky-source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("SKILL.md"), "---\nname: risky\ndescription: Test risk\n---\nrm -rf \"$TEST_ROOT\"\n").unwrap();
    import::add(&f.vault, source.to_str().unwrap(), &f.root.join("scratch"),
        &ImportOptions { allow_risky: true, ..ImportOptions::default() }).unwrap();
    let request = Request::Use { project: f.project.clone(), selections: vec!["risky".into()], agent: "claude".into(), mode: "copy".into(), include_secondary: vec![] };
    let before = tree(&f.project);
    let preview = projects::preview(&f.vault,&f.root,&request).unwrap();
    assert_eq!(preview.risky.len(),1);
    assert!(preview.risky[0].details.iter().any(|detail| detail.line==Some(5)));
    assert_eq!(projects::apply(&f.vault,&f.root,&request,&preview.token,&Control::default()).unwrap_err().code,"RISKY_ACTIVATION_REQUIRES_CONFIRMATION");
    assert_eq!(projects::apply_desktop(&f.vault,&f.root,&request,"wrong",true,&Control::default()).unwrap_err().code,"PROJECT_PREVIEW_CHANGED");
    let cancelled=Control::default();
    cancelled.cancel();
    assert_eq!(projects::apply_desktop(&f.vault,&f.root,&request,&preview.token,true,&cancelled).unwrap_err().code,"PROJECT_CANCELLED");
    assert_eq!(tree(&f.project),before);
    projects::apply_desktop(&f.vault,&f.root,&request,&preview.token,true,&Control::default()).unwrap();
    assert!(f.project.join(".claude/skills/risky/SKILL.md").is_file());
    let next=projects::preview(&f.vault,&f.root,&request).unwrap();
    assert!(next.risky.is_empty());
    assert!(!next.changed);
}

#[cfg(unix)]
#[test]
fn approved_risky_links_materialize_missing_automatic_and_manual_caches() {
    for manual in [false, true] {
        let f = Fixture::new();
        let source = f.root.join("risky-source");
        fs::create_dir(&source).unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\nname: risky\ndescription: Test risk\n---\nrm -rf \"$TEST_ROOT\"\n",
        )
        .unwrap();
        import::add(
            &f.vault,
            source.to_str().unwrap(),
            &f.root.join("scratch"),
            &ImportOptions {
                allow_risky: true,
                ..Default::default()
            },
        )
        .unwrap();
        if manual {
            let metadata = f.vault.root.join("meta/risky.toml");
            let mut text = fs::read_to_string(&metadata).unwrap();
            text.push_str("\n[invocation]\nmode = \"manual\"\n");
            fs::write(&metadata, text).unwrap();
        }
        fs::remove_dir_all(f.vault.root.join("dist/risky")).unwrap();
        let request = Request::Use {
            project: f.project.clone(),
            selections: vec!["risky".into()],
            agent: "claude".into(),
            mode: "link".into(),
            include_secondary: vec![],
        };
        let before = tree(&f.project);
        let preview = projects::preview(&f.vault, &f.root, &request).unwrap();
        assert_eq!(preview.risky.len(), 1);
        assert_eq!(tree(&f.project), before);
        projects::apply_desktop(
            &f.vault,
            &f.root,
            &request,
            &preview.token,
            true,
            &Control::default(),
        )
        .unwrap();
        let active = f.project.join(".claude/skills/risky");
        let target = f.vault.root.join(if manual {
            "dist/.manual/risky"
        } else {
            "dist/risky"
        });
        assert_eq!(fs::read_link(&active).unwrap(), target);
        assert!(active.join("SKILL.md").is_file());
        assert!(fs::read_dir(&f.project).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".skillsbox-activation-")
        }));
    }
}
