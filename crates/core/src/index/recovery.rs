//! Rebuild only the selected scope from authoritative files and registered locks.
use super::*;
use crate::{
    IndexRebuildReport, RegistryObservation, bundles, hash, project_registry, vault::Vault,
};
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
struct SavedFile {
    bytes: Vec<u8>,
    metadata: fs::Metadata,
}

fn same_entry(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    {
        a.file_type() == b.file_type() && a.created().ok() == b.created().ok()
    }
}
fn same_permissions(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        a.permissions().mode() == b.permissions().mode()
    }
    #[cfg(not(unix))]
    {
        a.permissions().readonly() == b.permissions().readonly()
    }
}
fn file(path: &Path) -> Result<Option<SavedFile>> {
    validate_path(path)?;
    let before = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => return Err(conflict("Index or state data must be a regular file")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let bytes = fs::read(path)?;
    validate_path(path)?;
    let after = fs::symlink_metadata(path)?;
    if !same_entry(&before, &after)
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || !same_permissions(&before, &after)
    {
        return Err(changed("File changed while being read"));
    }
    Ok(Some(SavedFile {
        bytes,
        metadata: after,
    }))
}
fn matches(path: &Path, expected: Option<&SavedFile>) -> Result<bool> {
    if expected.is_none() {
        validate_path(path.parent().ok_or_else(|| changed("File has no parent"))?)?;
        return match fs::symlink_metadata(path) {
            Ok(_) => Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
            Err(error) => Err(error.into()),
        };
    }
    Ok(match (file(path)?, expected) {
        (Some(actual), Some(expected)) => {
            same_entry(&actual.metadata, &expected.metadata)
                && actual.bytes == expected.bytes
                && actual.metadata.modified().ok() == expected.metadata.modified().ok()
                && same_permissions(&actual.metadata, &expected.metadata)
        }
        _ => false,
    })
}
fn conflict(message: impl Into<String>) -> Error {
    Error::decision("INDEX_CONFLICT", message)
}
fn changed(message: impl Into<String>) -> Error {
    Error::decision("INDEX_CHANGED", message)
}

struct Sources {
    directories: Vec<(PathBuf, String, fs::Metadata)>,
    config: Option<SavedFile>,
    registry: Option<SavedFile>,
    observations: Vec<RegistryObservation>,
}
impl Sources {
    fn begin(vault: &Vault) -> Result<Self> {
        let mut directories = vec![];
        for name in ["skills", "meta", "bundles"] {
            let path = validate_path(&vault.root.join(name))?;
            let metadata = fs::symlink_metadata(&path)?;
            directories.push((path.clone(), hash::hash_directory(&path)?, metadata));
        }
        Ok(Self {
            directories,
            config: file(&vault.root.join("box.toml"))?,
            registry: file(&project_registry::path(vault)?)?,
            observations: vec![],
        })
    }
    fn verify(&self, vault: &Vault) -> Result<()> {
        for (path, digest, identity) in &self.directories {
            validate_path(path)?;
            if !same_entry(identity, &fs::symlink_metadata(path)?)
                || hash::hash_directory(path)? != *digest
            {
                return Err(changed("Authoritative vault files changed during rebuild"));
            }
        }
        if !matches(&vault.root.join("box.toml"), self.config.as_ref())?
            || !matches(&project_registry::path(vault)?, self.registry.as_ref())?
        {
            return Err(changed(
                "Configuration or project registry changed during rebuild",
            ));
        }
        let now = project_registry::inspect(vault)?.projects;
        if encode(&now)? != encode(&self.observations)? {
            return Err(changed(
                "Registered project lock data changed during rebuild",
            ));
        }
        Ok(())
    }
}
fn encode(value: &impl serde::Serialize) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|error| Error::new("JSON_ERROR", error.to_string()))
}

fn foreign_rows(
    connection: &Connection,
    scope: &str,
) -> Result<Vec<Vec<Vec<rusqlite::types::Value>>>> {
    let mut tables = vec![];
    for table in [
        "skills",
        "skills_fts",
        "locations",
        "variants",
        "groups",
        "projects",
        "activations",
    ] {
        let count = connection
            .prepare(&format!("SELECT * FROM {table} LIMIT 0"))?
            .column_count();
        let order = (1..=count)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let mut statement = connection.prepare(&format!(
            "SELECT * FROM {table} WHERE vault<>?1 ORDER BY {order}"
        ))?;
        tables.push(
            statement
                .query_map([scope], |row| {
                    (0..count)
                        .map(|i| row.get(i))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?,
        );
    }
    Ok(tables)
}

pub fn rebuild(vault: &Vault, dry_run: bool) -> Result<IndexRebuildReport> {
    let _state_guard = crate::state::write_guard(&vault.index_path, dry_run)?;
    rebuild_with(vault, dry_run, || Ok(()))
}

fn rebuild_with(
    vault: &Vault,
    dry_run: bool,
    before_publish: impl FnOnce() -> Result<()>,
) -> Result<IndexRebuildReport> {
    let normalized = Vault::new(
        validate_path(&vault.root)?,
        validate_path(&vault.index_path)?,
    );
    let vault = &normalized;
    reject_sidecars(&vault.index_path)?;
    let before = file(&vault.index_path)?;
    let mode = match &before {
        None => "missing",
        Some(saved) => match validated_version(&vault.index_path) {
            Ok(Some(SCHEMA_VERSION)) => "healthy",
            // A file of the earlier version is that version's to rebuild.
            Ok(Some(version)) => return Err(require_current(version).unwrap_err()),
            _ => {
                // A damaged filename alone is not evidence of ownership. Keep the
                // SQLite format marker, application ID and schema version intact.
                if saved.bytes.len() < 100
                    || &saved.bytes[..16] != b"SQLite format 3\0"
                    || saved.bytes[68..72] != (APPLICATION_ID as u32).to_be_bytes()
                    || saved.bytes[60..64] != (SCHEMA_VERSION as u32).to_be_bytes()
                {
                    return Err(conflict(
                        "Cannot establish ownership/version of the damaged index; foreign or unknown data was preserved",
                    ));
                }
                "owned_corrupt"
            }
        },
    };
    let mut sources = Sources::begin(vault)?;
    vault.config()?;
    let records = vault.records()?;
    let definitions = bundles::list(vault)?;
    sources.observations = project_registry::inspect(vault)?.projects;
    sources.verify(vault)?;
    let locks = sources
        .observations
        .iter()
        .filter(|observation| observation.status == "ok")
        .filter_map(|observation| {
            observation
                .lock
                .as_ref()
                .map(|lock| (observation.project.clone(), lock.clone()))
        })
        .collect::<Vec<_>>();
    let mut report = IndexRebuildReport {
        vault: vault.root.clone(),
        index: vault.index_path.clone(),
        dry_run,
        rebuilt: false,
        mode: mode.into(),
        skills: records.len(),
        groups: definitions
            .iter()
            .map(|bundle| {
                bundle
                    .skills
                    .iter()
                    .filter(|member| member.group.is_some())
                    .count()
            })
            .sum(),
        projects: locks.len(),
        activations: locks.iter().map(|(_, lock)| lock.active.len()).sum(),
        other_vaults_preserved: mode == "healthy",
        other_vaults_recovery_required: mode != "healthy",
        observations: sources.observations.clone(),
        warnings: vec![],
        backup: None,
    };
    if mode != "healthy" {
        report.warnings.push("Only the selected vault can be rebuilt from these sources; other vault scopes require their own rebuild and are not claimed as recovered.".into());
    }
    for observation in &report.observations {
        if observation.status != "ok" {
            report.warnings.push(format!(
                "Skipped {}: {} ({})",
                observation.project.display(),
                observation.status,
                observation.message
            ));
        }
    }
    if dry_run {
        return Ok(report);
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    return Err(Error::new(
        "UNSUPPORTED_PLATFORM",
        "Exclusive index publication is unavailable",
    ));
    sources.verify(vault)?;
    reject_sidecars(&vault.index_path)?;
    if !matches(&vault.index_path, before.as_ref())? {
        return Err(changed("Index changed after planning"));
    }
    let mut publication = Publication::prepare(&vault.index_path, before, mode == "owned_corrupt")?;
    let result = (|| -> Result<()> {
        let staged = publication.stage.join("rebuilt.db");
        if mode == "healthy" {
            publication.write_clone()?;
        } else {
            initialize(&staged)?;
        }
        {
            let mut connection =
                Connection::open_with_flags(&staged, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
            let database =
                connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let scope = vault
                .root
                .to_str()
                .ok_or_else(|| Error::new("INVALID_PATH", "Vault must be Unicode"))?;
            let foreign_before = if mode == "healthy" {
                Some(foreign_rows(&database, scope)?)
            } else {
                None
            };
            write_vault_cache(&database, scope, &records, &definitions)?;
            database.execute("DELETE FROM activations WHERE vault=?1", [scope])?;
            database.execute("DELETE FROM projects WHERE vault=?1", [scope])?;
            for (project, lock) in &locks {
                write_project_cache(&database, project, lock)?;
            }
            if let Some(before) = foreign_before
                && foreign_rows(&database, scope)? != before
            {
                return Err(conflict(
                    "Rebuild would change another vault scope; the live index was preserved",
                ));
            }
            database.commit()?;
        }
        check(&staged)?;
        if let Some(before) = &publication.before {
            fs::set_permissions(&staged, before.metadata.permissions())?;
        }
        publication.after = file(&staged)?;
        before_publish()?;
        sources.verify(vault)?;
        reject_sidecars(&vault.index_path)?;
        publication.publish()?;
        // No project or vault file is ever published by index reconstruction.
        sources.verify(vault)?;
        publication.verify()
    })();
    if let Err(mut error) = result {
        publication.rollback();
        let warnings = publication.cleanup();
        if !warnings.is_empty() {
            error
                .message
                .push_str(&format!(". {}", warnings.join("; ")));
        }
        return Err(error);
    }
    publication.committed = true;
    if publication.keep_backup {
        report.backup = Some(publication.stage.join("original.db"));
    }
    report.warnings.extend(publication.cleanup());
    report.rebuilt = true;
    Ok(report)
}

struct Publication {
    target: PathBuf,
    stage: PathBuf,
    parent_identity: fs::Metadata,
    stage_identity: fs::Metadata,
    before: Option<SavedFile>,
    after: Option<SavedFile>,
    backed_up: bool,
    published: bool,
    committed: bool,
    keep_backup: bool,
    created: Vec<(PathBuf, fs::Metadata)>,
}
impl Publication {
    fn prepare(target: &Path, before: Option<SavedFile>, keep_backup: bool) -> Result<Self> {
        let parent = target
            .parent()
            .ok_or_else(|| changed("Index has no parent"))?;
        let mut created = vec![];
        fn ensure(path: &Path, created: &mut Vec<(PathBuf, fs::Metadata)>) -> Result<()> {
            validate_path(path)?;
            if let Ok(metadata) = fs::symlink_metadata(path) {
                if metadata.is_dir() {
                    return Ok(());
                }
                return Err(conflict("Index state parent is not a directory"));
            }
            ensure(
                path.parent()
                    .ok_or_else(|| changed("Directory has no parent"))?,
                created,
            )?;
            private_directory(path)?;
            created.push((path.into(), fs::symlink_metadata(path)?));
            Ok(())
        }
        if let Err(error) = ensure(parent, &mut created) {
            cleanup_created(&created);
            return Err(error);
        }
        let parent_identity = match fs::symlink_metadata(parent) {
            Ok(metadata) => metadata,
            Err(error) => {
                cleanup_created(&created);
                return Err(error.into());
            }
        };
        let stage = loop {
            let candidate = parent.join(format!(
                ".skillsbox-index-rebuild-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match private_directory(&candidate) {
                Ok(()) => break candidate,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    cleanup_created(&created);
                    return Err(error.into());
                }
            }
        };
        let stage_identity = fs::symlink_metadata(&stage)?;
        Ok(Self {
            target: target.into(),
            stage,
            parent_identity,
            stage_identity,
            before,
            after: None,
            backed_up: false,
            published: false,
            committed: false,
            keep_backup,
            created,
        })
    }
    fn check_stage(&self) -> Result<()> {
        validate_path(&self.stage)?;
        if !same_entry(
            &self.parent_identity,
            &fs::symlink_metadata(self.target.parent().expect("index parent"))?,
        ) || !same_entry(&self.stage_identity, &fs::symlink_metadata(&self.stage)?)
        {
            return Err(changed("Index staging or state parent was replaced"));
        }
        Ok(())
    }
    fn write_clone(&self) -> Result<()> {
        self.check_stage()?;
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(self.stage.join("rebuilt.db"))?;
        file.write_all(&self.before.as_ref().expect("healthy index snapshot").bytes)?;
        file.sync_all()?;
        Ok(())
    }
    fn publish(&mut self) -> Result<()> {
        self.publish_with(|| Ok(()))
    }
    fn publish_with(&mut self, after_backup: impl FnOnce() -> Result<()>) -> Result<()> {
        self.check_stage()?;
        reject_sidecars(&self.target)?;
        if !matches(&self.target, self.before.as_ref())? {
            return Err(changed("Index changed before publication"));
        }
        if self.before.is_some() {
            crate::safe_rename::rename_no_replace(&self.target, &self.stage.join("original.db"))?;
            self.backed_up = true;
            if !matches(&self.stage.join("original.db"), self.before.as_ref())? {
                return Err(changed("Concurrent index replacement preserved in staging"));
            }
        }
        after_backup()?;
        crate::safe_rename::rename_no_replace(&self.stage.join("rebuilt.db"), &self.target)?;
        self.published = true;
        self.verify()
    }
    fn verify(&self) -> Result<()> {
        self.check_stage()?;
        reject_sidecars(&self.target)?;
        if !self.published || !matches(&self.target, self.after.as_ref())? {
            return Err(changed("Published index changed"));
        }
        Ok(())
    }
    fn rollback(&mut self) {
        if self.check_stage().is_err() {
            return;
        }
        if self.published
            && matches(&self.target, self.after.as_ref()).unwrap_or(false)
            && fs::remove_file(&self.target).is_ok()
        {
            self.published = false;
        }
        if self.backed_up
            && matches(&self.stage.join("original.db"), self.before.as_ref()).unwrap_or(false)
            && crate::safe_rename::rename_no_replace(&self.stage.join("original.db"), &self.target)
                .is_ok()
        {
            self.backed_up = false;
        }
    }
    fn cleanup(&self) -> Vec<String> {
        let mut warnings = vec![];
        if !self.stage.exists() {
            return warnings;
        }
        if self.check_stage().is_ok() {
            if self.after.is_some()
                && matches(&self.stage.join("rebuilt.db"), self.after.as_ref()).unwrap_or(false)
            {
                let _ = fs::remove_file(self.stage.join("rebuilt.db"));
            }
            if self.backed_up
                && self.committed
                && !self.keep_backup
                && matches(&self.stage.join("original.db"), self.before.as_ref()).unwrap_or(false)
            {
                let _ = fs::remove_file(self.stage.join("original.db"));
            }
            if fs::remove_dir(&self.stage).is_ok() {
                return warnings;
            }
        }
        warnings.push(format!(
            "Index backup, partial or concurrently modified staging was preserved at {}",
            self.stage.display()
        ));
        warnings
    }
}
impl Drop for Publication {
    fn drop(&mut self) {
        if !self.committed {
            self.rollback();
        }
        let _ = self.cleanup();
        if !self.committed {
            cleanup_created(&self.created);
        }
    }
}
fn cleanup_created(created: &[(PathBuf, fs::Metadata)]) {
    for (path, identity) in created.iter().rev() {
        if validate_path(path).is_ok()
            && fs::symlink_metadata(path).is_ok_and(|now| same_entry(identity, &now))
        {
            let _ = fs::remove_dir(path);
        }
    }
}
fn private_directory(path: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().canonicalize().unwrap().join(format!(
                "skillsbox-recovery-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn vault(&self) -> Vault {
            let v = Vault::new(self.0.join("vault"), self.0.join("state/index.db"));
            v.init(false).unwrap();
            v
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn changed_sources_abort_before_publication_and_keep_the_live_index() {
        let t = Temp::new();
        let v = t.vault();
        let original = fs::read(&v.index_path).unwrap();
        let error = rebuild_with(&v, false, || {
            let path = v.root.join("box.toml");
            let mut text = fs::read_to_string(&path)?;
            text.push_str("\n# concurrent source edit\n");
            fs::write(path, text)?;
            Ok(())
        })
        .unwrap_err();
        assert_eq!(error.code, "INDEX_CHANGED");
        assert_eq!(fs::read(&v.index_path).unwrap(), original);
        assert_eq!(
            fs::read_dir(v.index_path.parent().unwrap())
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn an_identical_byte_index_replacement_is_not_overwritten() {
        let t = Temp::new();
        let v = t.vault();
        let bytes = fs::read(&v.index_path).unwrap();
        let saved = t.0.join("saved-original");
        let error = rebuild_with(&v, false, || {
            fs::rename(&v.index_path, &saved)?;
            fs::write(&v.index_path, &bytes)?;
            Ok(())
        })
        .unwrap_err();
        assert_eq!(error.code, "INDEX_CHANGED");
        assert_eq!(fs::read(&v.index_path).unwrap(), bytes);
        assert_eq!(fs::read(saved).unwrap(), bytes);
    }

    #[test]
    fn a_foreign_file_appearing_at_a_missing_index_path_is_preserved() {
        let t = Temp::new();
        let v = t.vault();
        fs::remove_file(&v.index_path).unwrap();
        let error = rebuild_with(&v, false, || {
            fs::write(&v.index_path, b"foreign replacement")?;
            Ok(())
        })
        .unwrap_err();
        assert_eq!(error.code, "INDEX_CHANGED");
        assert_eq!(fs::read(&v.index_path).unwrap(), b"foreign replacement");
    }

    #[test]
    fn rollback_restores_the_original_index_inode_after_publication_failure() {
        let t = Temp::new();
        let target = t.0.join("index.db");
        fs::write(&target, b"old").unwrap();
        let before = file(&target).unwrap();
        let mut publication = Publication::prepare(&target, before.clone(), false).unwrap();
        fs::write(publication.stage.join("rebuilt.db"), b"new").unwrap();
        publication.after = file(&publication.stage.join("rebuilt.db")).unwrap();
        publication.publish().unwrap();
        publication.rollback();
        assert!(matches(&target, before.as_ref()).unwrap());
        assert!(publication.cleanup().is_empty());
    }

    #[test]
    fn a_destination_appearing_after_backup_is_not_deleted_to_restore_the_old_index() {
        let t = Temp::new();
        let target = t.0.join("index.db");
        fs::write(&target, b"old").unwrap();
        let mut publication = Publication::prepare(&target, file(&target).unwrap(), false).unwrap();
        fs::write(publication.stage.join("rebuilt.db"), b"new").unwrap();
        publication.after = file(&publication.stage.join("rebuilt.db")).unwrap();
        assert!(
            publication
                .publish_with(|| {
                    fs::write(&target, b"foreign")?;
                    Ok(())
                })
                .is_err()
        );
        publication.rollback();
        assert_eq!(fs::read(&target).unwrap(), b"foreign");
        assert_eq!(
            fs::read(publication.stage.join("original.db")).unwrap(),
            b"old"
        );
        assert!(!publication.cleanup().is_empty());
    }
}
