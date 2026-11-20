//! Storage integration only. No activation target is created or inspected here.
use kinana_core::{LockedSkill, ProjectLock, Result, lockfile, project_registry, vault::Vault};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().canonicalize()?.join(format!(
            "kinana-lock-registry-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn example(vault: &Vault) -> ProjectLock {
    ProjectLock {
        restoration: None,
        vault: vault.root.clone(),
        bundle: None,
        active: vec![LockedSkill {
            target_root: None,
            generation_hash: None,
            ownership: None,
            id: "data-only-fixture".into(),
            agent: "claude".into(),
            mode: "link".into(),
            name: "data-only-fixture".into(),
            invocation: "auto".into(),
            content_hash: format!("blake3:{}", "0".repeat(64)),
        }],
    }
}
fn project(root: &Path, name: &str) -> Result<PathBuf> {
    let project = root.join(name);
    fs::create_dir_all(project.join(".claude/skills"))?;
    Ok(project)
}
#[test]
fn registered_lock_data_survives_missing_cache_without_starting_activation() -> Result<()> {
    let temp = Temp::new()?;
    let vault = Vault::new(temp.0.join("vault"), temp.0.join("state/index.db"));
    vault.init(false)?;
    let project = project(&temp.0, "project")?;
    let lock = example(&vault);
    let report = lockfile::write(&project, &vault.root, &lock, "claude", false)?;
    assert!(report.changed);
    assert!(project_registry::record(&vault, &project, false)?.changed);
    let lock_before = fs::read(project.join(".kinana.lock"))?;
    let registry_path = project_registry::path(&vault)?;
    let registry_before = fs::read(&registry_path)?;
    assert_eq!(project_registry::read(&vault)?.projects.len(), 1);
    fs::remove_file(&vault.index_path)?;
    let inspection = project_registry::inspect(&vault)?;
    assert_eq!(inspection.projects.len(), 1);
    assert_eq!(inspection.projects[0].lock, Some(lock));
    assert_eq!(fs::read(project.join(".kinana.lock"))?, lock_before);
    assert_eq!(fs::read(registry_path)?, registry_before);
    assert!(!vault.index_path.exists());
    assert_eq!(fs::read_dir(project.join(".claude/skills"))?.count(), 0);
    assert!(!project.join(".agents").exists());
    Ok(())
}
#[test]
fn read_only_registry_inspection_reports_foreign_lock_without_rewriting_it() -> Result<()> {
    let temp = Temp::new()?;
    let first = Vault::new(temp.0.join("first"), temp.0.join("state/index.db"));
    first.init(false)?;
    let second = Vault::new(temp.0.join("second"), temp.0.join("state/index.db"));
    second.init(false)?;
    let project = project(&temp.0, "project")?;
    lockfile::write(&project, &first.root, &example(&first), "claude", false)?;
    project_registry::record(&first, &project, false)?;
    let registry_path = project_registry::path(&first)?;
    let before = fs::read(registry_path.clone())?;
    // Model a user's explicit manual change to the data file. Storage discovery
    // must report the mismatch and leave both files exactly as observed.
    fs::write(
        project.join(".kinana.lock"),
        toml::to_string_pretty(&example(&second))?,
    )?;
    let lock_before = fs::read(project.join(".kinana.lock"))?;
    let inspection = project_registry::inspect(&first)?;
    assert_eq!(inspection.projects.len(), 1);
    assert!(inspection.projects[0].status != "ok" && inspection.projects[0].status != "lock_valid");
    assert_eq!(fs::read(registry_path)?, before);
    assert_eq!(fs::read(project.join(".kinana.lock"))?, lock_before);
    Ok(())
}

#[test]
fn lock_finalization_rechecks_even_unchanged_publications_and_preserves_foreign_data() -> Result<()>
{
    let temp = Temp::new()?;
    let vault = Vault::new(temp.0.join("vault"), temp.0.join("state/index.db"));
    vault.init(false)?;
    for change in [false, true] {
        let project = project(&temp.0, if change { "changed" } else { "unchanged" })?;
        let before = example(&vault);
        lockfile::write(&project, &vault.root, &before, "claude", false)?;
        let original = fs::read(project.join(".kinana.lock"))?;
        let mut requested = before;
        if change {
            requested.bundle = Some("new-bundle".into());
        }
        let mut prepared = lockfile::prepare(&project, &vault.root, &requested, "claude")?;
        prepared.publish()?;
        fs::write(
            project.join(".kinana.lock"),
            "Concurrent user's replacement bytes",
        )?;
        assert_eq!(
            prepared
                .finish()
                .expect_err("late lock change must fail")
                .code,
            "PROJECT_LOCK_CHANGED"
        );
        assert_eq!(
            fs::read(project.join(".kinana.lock"))?,
            b"Concurrent user's replacement bytes"
        );
        if change {
            let mut backup_found = false;
            for entry in fs::read_dir(project.join(".claude/skills"))? {
                let backup = entry?.path().join("old.lock");
                if backup.is_file() && fs::read(backup)? == original {
                    backup_found = true;
                }
            }
            assert!(
                backup_found,
                "original lock backup must survive alongside foreign data"
            );
        }
    }
    Ok(())
}
