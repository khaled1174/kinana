mod support;

use kinana_core::{
    ProjectLock, ProjectRegistration, ProjectRegistry, Result, gitlog, project_registry,
    vault::Vault,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Result<Self> {
        let root = support::temp_dir()?;
        Ok(Self(root))
    }

    fn vault(&self, name: &str) -> Result<Vault> {
        let vault = Vault::new(self.0.join(name), self.0.join("state/index.db"));
        vault.init(false)?;
        Ok(vault)
    }

    fn project(&self, name: &str, vault: &Vault) -> Result<PathBuf> {
        let project = self.0.join(name);
        fs::create_dir(&project)?;
        write_lock(&project, vault)?;
        Ok(project)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _cleanup = fs::remove_dir_all(&self.0);
    }
}

fn write_lock(project: &Path, vault: &Vault) -> Result<()> {
    let lock = ProjectLock {
        restoration: None,
        vault: vault.root.clone(),
        bundle: None,
        active: Vec::new(),
    };
    fs::write(project.join(".kinana.lock"), toml::to_string_pretty(&lock)?)?;
    Ok(())
}

fn write_registry(vault: &Vault, registrations: Vec<ProjectRegistration>) -> Result<()> {
    fs::write(
        project_registry::path(vault)?,
        toml::to_string_pretty(&ProjectRegistry {
            version: 1,
            projects: registrations,
        })?,
    )?;
    Ok(())
}

fn snapshot(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>> {
    fn visit(root: &Path, at: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) -> Result<()> {
        for entry in fs::read_dir(at)? {
            let path = entry?.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|error| std::io::Error::other(error.to_string()))?
                .to_path_buf();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                files.insert(
                    relative,
                    fs::read_link(&path)?
                        .as_os_str()
                        .as_encoded_bytes()
                        .to_vec(),
                );
            } else if metadata.is_dir() {
                files.insert(relative, b"directory".to_vec());
                visit(root, &path, files)?;
            } else if metadata.is_file() {
                files.insert(relative, fs::read(path)?);
            } else {
                files.insert(relative, b"special file".to_vec());
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files)?;
    Ok(files)
}

#[test]
fn missing_registry_reads_and_inspects_as_empty_without_creating_state_or_vault_paths() -> Result<()>
{
    let temp = Temp::new()?;
    let vault = Vault::new(
        temp.0.join("missing-vault"),
        temp.0.join("missing-state/index.db"),
    );
    let before = snapshot(&temp.0)?;
    assert_eq!(
        project_registry::path(&vault)?,
        temp.0.join("missing-state/projects.toml")
    );
    assert_eq!(project_registry::read(&vault)?, ProjectRegistry::default());
    assert!(project_registry::inspect(&vault)?.projects.is_empty());
    assert_eq!(snapshot(&temp.0)?, before);
    Ok(())
}

#[test]
fn dry_run_roundtrip_and_exact_noop_preserve_project_cache_git_and_registry_bytes() -> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.vault("vault")?;
    let project = temp.project("project", &vault)?;
    fs::write(project.join("user-file"), b"Untouched project content.")?;
    let before = snapshot(&temp.0)?;
    let head = gitlog::head(&vault.root)?;
    let preview = project_registry::record(&vault, &project, true)?;
    assert!(preview.dry_run && preview.changed);
    assert_eq!(preview.registration.path, project);
    assert_eq!(preview.registration.vault, vault.root);
    assert_eq!(snapshot(&temp.0)?, before);
    let project_before = snapshot(&project)?;
    let cache = fs::read(&vault.index_path)?;
    let saved = project_registry::record(&vault, &project, false)?;
    assert!(saved.changed && !saved.dry_run);
    assert!(saved.warnings.is_empty());
    assert_eq!(saved.path, temp.0.join("state/projects.toml"));
    assert_eq!(
        project_registry::read(&vault)?.projects,
        [ProjectRegistration {
            path: project.clone(),
            vault: vault.root.clone()
        }]
    );
    assert_eq!(snapshot(&project)?, project_before);
    assert_eq!(fs::read(&vault.index_path)?, cache);
    assert_eq!(gitlog::head(&vault.root)?, head);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&saved.path)?.permissions().mode() & 0o777,
            0o600
        );
    }
    let bytes = fs::read(&saved.path)?;
    fs::write(
        &saved.path,
        [b"# Retain user formatting\n".as_slice(), bytes.as_slice()].concat(),
    )?;
    let before = snapshot(&temp.0)?;
    for dry_run in [false, true] {
        let noop = project_registry::record(&vault, &project, dry_run)?;
        assert!(!noop.changed);
        assert_eq!(noop.dry_run, dry_run);
    }
    assert_eq!(snapshot(&temp.0)?, before);
    let inspected = project_registry::inspect(&vault)?;
    assert_eq!(inspected.projects.len(), 1);
    assert_eq!(inspected.projects[0].status, "ok");
    assert!(inspected.projects[0].message.contains("Only lock data"));
    assert!(inspected.projects[0].lock.is_some());
    assert_eq!(snapshot(&temp.0)?, before);
    assert!(!vault.root.join("projects.toml").exists());
    assert!(!project.join(".claude").exists());
    assert!(!project.join(".agents").exists());
    Ok(())
}

#[test]
fn registration_requires_an_existing_valid_same_vault_lock_before_creating_registry_data()
-> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.vault("vault")?;
    let foreign = temp.vault("foreign")?;
    let project = temp.0.join("project");
    fs::create_dir(&project)?;
    let before = snapshot(&temp.0)?;
    for dry_run in [false, true] {
        let error = project_registry::record(&vault, &project, dry_run).unwrap_err();
        assert_eq!(error.code, "PROJECT_LOCK_REQUIRED");
        assert!(error.needs_decision);
    }
    assert_eq!(snapshot(&temp.0)?, before);
    fs::write(project.join(".kinana.lock"), b"unknown='foreign data'\n")?;
    let before = snapshot(&temp.0)?;
    assert!(project_registry::record(&vault, &project, false).is_err());
    assert_eq!(snapshot(&temp.0)?, before);
    write_lock(&project, &foreign)?;
    let before = snapshot(&temp.0)?;
    let error = project_registry::record(&vault, &project, false).unwrap_err();
    assert_eq!(error.code, "FOREIGN_PROJECT_LOCK");
    assert!(error.needs_decision);
    assert_eq!(snapshot(&temp.0)?, before);
    assert!(!project_registry::path(&vault)?.exists());
    write_lock(&project, &vault)?;
    project_registry::record(&vault, &project, false)?;
    assert_eq!(project_registry::read(&vault)?.projects.len(), 1);
    Ok(())
}

#[test]
fn shared_state_preserves_other_vault_records_and_conflicts_instead_of_reassigning_projects()
-> Result<()> {
    let temp = Temp::new()?;
    let one = temp.vault("vault-one")?;
    let two = temp.vault("vault-two")?;
    let first = temp.project("first", &one)?;
    let second = temp.project("second", &two)?;
    project_registry::record(&one, &first, false)?;
    project_registry::record(&two, &second, false)?;
    assert_eq!(project_registry::read(&one)?, project_registry::read(&two)?);
    assert_eq!(project_registry::read(&one)?.projects.len(), 2);
    let first_view = project_registry::inspect(&one)?;
    let second_view = project_registry::inspect(&two)?;
    assert_eq!(first_view.projects.len(), 1);
    assert_eq!(first_view.projects[0].project, first);
    assert_eq!(second_view.projects.len(), 1);
    assert_eq!(second_view.projects[0].project, second);
    write_lock(&first, &two)?;
    let before = snapshot(&temp.0)?;
    for dry_run in [false, true] {
        let error = project_registry::record(&two, &first, dry_run).unwrap_err();
        assert_eq!(error.code, "PROJECT_VAULT_CONFLICT");
        assert!(error.needs_decision);
    }
    assert_eq!(snapshot(&temp.0)?, before);
    assert_eq!(
        project_registry::inspect(&one)?.projects[0].status,
        "foreign_lock"
    );
    Ok(())
}

#[test]
fn strict_registry_schema_version_duplicate_and_unsafe_paths_refuse_overwrite() -> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.vault("vault")?;
    let project = temp.project("project", &vault)?;
    let path = project_registry::path(&vault)?;
    let valid = toml::to_string_pretty(&ProjectRegistry {
        version: 1,
        projects: vec![ProjectRegistration {
            path: project.clone(),
            vault: vault.root.clone(),
        }],
    })?;
    let duplicate = toml::to_string_pretty(&ProjectRegistry {
        version: 1,
        projects: vec![
            ProjectRegistration {
                path: project.clone(),
                vault: vault.root.clone(),
            },
            ProjectRegistration {
                path: project.clone(),
                vault: temp.0.join("other"),
            },
        ],
    })?;
    let cases = [
        b"not a registry".to_vec(),
        b"version=2\nprojects=[]\n".to_vec(),
        b"version='invalid'\nprojects=[]\n".to_vec(),
        format!("{valid}\npath=42\n").into_bytes(),
        duplicate.into_bytes(),
        b"version=1\n[[projects]]\npath='relative'\nvault='/safe/vault'\n".to_vec(),
        b"version=1\n[[projects]]\npath='/safe/../project'\nvault='/safe/vault'\n".to_vec(),
        b"version=1\n[[projects]]\npath='/safe/project'\nvault='relative'\n".to_vec(),
        b"version=1\n[[projects]]\npath='/safe/project'\nvault='/safe/../vault'\n".to_vec(),
        vec![0xff, 0xfe],
    ];
    for bytes in cases {
        fs::write(&path, bytes)?;
        let before = snapshot(&temp.0)?;
        let error = project_registry::read(&vault).unwrap_err();
        assert_eq!(error.code, "REGISTRY_CONFLICT");
        assert!(error.needs_decision);
        assert!(project_registry::inspect(&vault).is_err());
        for dry_run in [false, true] {
            assert!(project_registry::record(&vault, &project, dry_run).is_err());
        }
        assert_eq!(snapshot(&temp.0)?, before);
    }
    Ok(())
}

#[test]
fn registry_read_accepts_stale_locations_without_reading_them_and_inspect_reports_each_state()
-> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.vault("vault")?;
    let foreign = temp.vault("foreign-vault")?;
    let ok = temp.project("ok", &vault)?;
    let missing = temp.0.join("missing-project");
    let no_lock = temp.0.join("missing-lock");
    fs::create_dir(&no_lock)?;
    let corrupt = temp.0.join("invalid-lock");
    fs::create_dir(&corrupt)?;
    fs::write(corrupt.join(".kinana.lock"), b"invalid lock data")?;
    let other_lock = temp.project("foreign-lock", &foreign)?;
    let not_directory = temp.0.join("not-directory");
    fs::write(&not_directory, b"ordinary data")?;
    let other_vault_project = temp.0.join("ignored-other-vault-project");
    let stale_vault = temp.0.join("missing-vault");
    let mut registrations: Vec<_> = [
        &ok,
        &missing,
        &no_lock,
        &corrupt,
        &other_lock,
        &not_directory,
    ]
    .into_iter()
    .map(|path| ProjectRegistration {
        path: path.clone(),
        vault: vault.root.clone(),
    })
    .collect();
    registrations.push(ProjectRegistration {
        path: other_vault_project,
        vault: stale_vault,
    });
    write_registry(&vault, registrations)?;
    let before = snapshot(&temp.0)?;
    assert_eq!(project_registry::read(&vault)?.projects.len(), 7);
    let report = project_registry::inspect(&vault)?;
    let statuses: BTreeMap<_, _> = report
        .projects
        .iter()
        .map(|entry| (entry.project.clone(), entry.status.as_str()))
        .collect();
    assert_eq!(statuses.get(&ok), Some(&"ok"));
    assert_eq!(statuses.get(&missing), Some(&"missing_project"));
    assert_eq!(statuses.get(&no_lock), Some(&"missing_lock"));
    assert_eq!(statuses.get(&corrupt), Some(&"invalid_lock"));
    assert_eq!(statuses.get(&other_lock), Some(&"foreign_lock"));
    assert_eq!(statuses.get(&not_directory), Some(&"invalid_path"));
    assert_eq!(report.projects.len(), 6);
    assert!(
        report
            .projects
            .iter()
            .find(|entry| entry.project == other_lock)
            .unwrap()
            .lock
            .is_some()
    );
    assert_eq!(snapshot(&temp.0)?, before);
    Ok(())
}

#[test]
fn writes_require_an_existing_state_parent_and_never_repair_or_create_other_paths() -> Result<()> {
    let temp = Temp::new()?;
    let initialized = temp.vault("vault")?;
    let project = temp.project("project", &initialized)?;
    let vault = Vault::new(
        initialized.root.clone(),
        temp.0.join("missing-state/index.db"),
    );
    let before = snapshot(&temp.0)?;
    for dry_run in [false, true] {
        assert!(project_registry::record(&vault, &project, dry_run).is_err());
    }
    assert_eq!(snapshot(&temp.0)?, before);
    assert!(!temp.0.join("missing-state").exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn linked_recorded_projects_and_locks_are_reported_without_following_or_mutating_them() -> Result<()>
{
    use std::os::unix::fs::symlink;
    let temp = Temp::new()?;
    let vault = temp.vault("vault")?;
    let real = temp.project("real", &vault)?;
    let linked_project = temp.0.join("linked-project");
    symlink(&real, &linked_project)?;
    let linked_lock = temp.0.join("linked-lock");
    fs::create_dir(&linked_lock)?;
    symlink(real.join(".kinana.lock"), linked_lock.join(".kinana.lock"))?;
    let linked_parent = temp.0.join("linked-parent");
    symlink(&temp.0, &linked_parent)?;
    let through_parent = linked_parent.join("real");
    write_registry(
        &vault,
        [&linked_project, &linked_lock, &through_parent]
            .into_iter()
            .map(|path| ProjectRegistration {
                path: path.clone(),
                vault: vault.root.clone(),
            })
            .collect(),
    )?;
    let before = snapshot(&temp.0)?;
    assert_eq!(project_registry::read(&vault)?.projects.len(), 3);
    let report = project_registry::inspect(&vault)?;
    assert_eq!(report.projects.len(), 3);
    assert!(
        report
            .projects
            .iter()
            .all(|entry| entry.status == "invalid_path")
    );
    for project in [&linked_project, &linked_lock, &through_parent] {
        assert!(project_registry::record(&vault, project, false).is_err());
    }
    assert_eq!(snapshot(&temp.0)?, before);
    Ok(())
}

#[cfg(unix)]
#[test]
fn linked_special_or_foreign_registry_state_is_refused_without_overwrite() -> Result<()> {
    use std::os::unix::fs::symlink;
    use std::os::unix::net::UnixListener;
    let temp = Temp::new()?;
    let vault = temp.vault("vault")?;
    let project = temp.project("project", &vault)?;
    let path = project_registry::path(&vault)?;
    let foreign = temp.0.join("foreign-file");
    fs::write(&foreign, b"Keep foreign bytes")?;
    symlink(&foreign, &path)?;
    let before = snapshot(&temp.0)?;
    assert!(project_registry::read(&vault).is_err());
    assert!(project_registry::record(&vault, &project, false).is_err());
    assert_eq!(snapshot(&temp.0)?, before);
    fs::remove_file(&path)?;
    fs::create_dir(&path)?;
    let before = snapshot(&temp.0)?;
    assert!(project_registry::read(&vault).is_err());
    assert!(project_registry::record(&vault, &project, false).is_err());
    assert_eq!(snapshot(&temp.0)?, before);
    fs::remove_dir(&path)?;
    let listener = UnixListener::bind(&path)?;
    let before = snapshot(&temp.0)?;
    assert!(project_registry::read(&vault).is_err());
    assert!(project_registry::record(&vault, &project, false).is_err());
    assert_eq!(snapshot(&temp.0)?, before);
    drop(listener);
    fs::remove_file(&path)?;
    let linked_state = temp.0.join("linked-state");
    symlink(temp.0.join("state"), &linked_state)?;
    let linked = Vault::new(vault.root.clone(), linked_state.join("index.db"));
    let before = snapshot(&temp.0)?;
    assert!(project_registry::path(&linked).is_err());
    assert!(project_registry::read(&linked).is_err());
    assert!(project_registry::inspect(&linked).is_err());
    assert!(project_registry::record(&linked, &project, false).is_err());
    assert_eq!(snapshot(&temp.0)?, before);
    Ok(())
}

#[test]
fn future_registry_extensions_survive_new_registration_without_interpreting_paths() -> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.vault("vault")?;
    let first = temp.project("first", &vault)?;
    let second = temp.project("second", &vault)?;
    project_registry::record(&vault, &first, false)?;
    let path = project_registry::path(&vault)?;
    let mut doc: toml::Value = toml::from_str(&fs::read_to_string(&path)?)?;
    doc.as_table_mut()
        .unwrap()
        .insert("future".into(), toml::Value::String("retained".into()));
    doc["projects"][0].as_table_mut().unwrap().insert(
        "future_path".into(),
        toml::Value::String("../outside".into()),
    );
    fs::write(&path, toml::to_string_pretty(&doc)?)?;
    let before = snapshot(&temp.0)?;
    project_registry::record(&vault, &second, true)?;
    assert_eq!(snapshot(&temp.0)?, before);
    project_registry::record(&vault, &second, false)?;
    let after: toml::Value = toml::from_str(&fs::read_to_string(&path)?)?;
    assert_eq!(after["future"], doc["future"]);
    assert_eq!(
        after["projects"][0]["future_path"],
        doc["projects"][0]["future_path"]
    );
    assert_eq!(project_registry::read(&vault)?.projects.len(), 2);
    Ok(())
}
