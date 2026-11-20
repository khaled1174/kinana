//! Local project locations. Project locks remain the authority for ownership.
use crate::{
    Error, ProjectRegistration, ProjectRegistry, RegistryInspectReport, RegistryObservation,
    RegistryWriteReport, Result, index, lockfile, vault::Vault,
};
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_STAGE: AtomicU64 = AtomicU64::new(0);

/// The registry lives next to the local index, outside the vault's Git history.
pub fn path(vault: &Vault) -> Result<PathBuf> {
    let index_path = if vault.index_path.is_absolute() {
        vault.index_path.clone()
    } else {
        std::env::current_dir()?.join(&vault.index_path)
    };
    if index_path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(conflict(
            "Parent components are not allowed in the local index path",
        ));
    }
    let parent = index_path
        .parent()
        .ok_or_else(|| conflict("The local index must have a parent directory"))?;
    index::validate_path(&parent.join("projects.toml"))
}

/// Read location data without opening any recorded project or vault path.
pub fn read(vault: &Vault) -> Result<ProjectRegistry> {
    match snapshot(&path(vault)?)? {
        Some(saved) => parse(&saved.bytes),
        None => Ok(ProjectRegistry::default()),
    }
}

/// Record a project only after it has a valid lock for this vault.
/// This utility does not create or verify activation targets.
pub fn record(vault: &Vault, project: &Path, dry_run: bool) -> Result<RegistryWriteReport> {
    let _state_guard = crate::state::write_guard(&vault.index_path, dry_run)?;
    let project = require_directory(project)?;
    let vault_root = require_directory(&vault.root)?;
    vault.config()?;
    let lock_path = lockfile::path(&project)?;
    let lock_snapshot = snapshot(&lock_path)?.ok_or_else(|| {
        Error::decision(
            "PROJECT_LOCK_REQUIRED",
            "A valid .kinana.lock for this vault is required before project registration",
        )
    })?;
    let lock = lockfile::read(&project)?.ok_or_else(|| changed("Project lock disappeared"))?;
    lockfile::validate_for_vault(&project, &vault_root, &lock)?;
    if !matches_snapshot(&lock_path, Some(&lock_snapshot))? {
        return Err(changed(
            "Project lock changed during registration validation",
        ));
    }

    let target = path(vault)?;
    require_directory(
        target
            .parent()
            .ok_or_else(|| conflict("Registry path has no state parent"))?,
    )?;
    let before = snapshot(&target)?;
    let mut registry = match &before {
        Some(saved) => parse(&saved.bytes)?,
        None => ProjectRegistry::default(),
    };
    let registration = ProjectRegistration {
        path: project,
        vault: vault_root,
    };
    let mut report = RegistryWriteReport {
        path: target.clone(),
        registration: registration.clone(),
        dry_run,
        changed: true,
        warnings: Vec::new(),
    };
    if let Some(existing) = registry
        .projects
        .iter()
        .find(|entry| entry.path == registration.path)
    {
        if existing.vault != registration.vault {
            return Err(Error::decision(
                "PROJECT_VAULT_CONFLICT",
                "The project is already registered to another vault; its existing registration was preserved",
            ));
        }
        report.changed = false;
        return Ok(report);
    }
    registry.projects.push(registration);
    validate(&registry)?;
    if dry_run {
        return Ok(report);
    }
    let encoded = serialize_preserving(&registry, before.as_ref().map(|s| s.bytes.as_slice()))?;
    let mut transaction = RegistrySave::new(target, before, encoded)?;
    let publication = (|| {
        if !matches_snapshot(&lock_path, Some(&lock_snapshot))? {
            return Err(changed("Project lock changed before registry publication"));
        }
        transaction.publish()?;
        if !matches_snapshot(&lock_path, Some(&lock_snapshot))? {
            return Err(changed("Project lock changed during registry publication"));
        }
        transaction.verify_published()
    })();
    if let Err(mut error) = publication {
        transaction.rollback();
        if let Some(warning) = transaction.cleanup() {
            error.message.push_str(&format!(". {warning}"));
        }
        return Err(error);
    }
    transaction.finished = true;
    if let Some(warning) = transaction.cleanup() {
        report.warnings.push(warning);
    }
    Ok(report)
}

/// Inspect only registered locations for the selected vault, with no scanning.
pub fn inspect(vault: &Vault) -> Result<RegistryInspectReport> {
    let selected_vault = absolute_lexical(&if vault.root.is_absolute() {
        vault.root.clone()
    } else {
        std::env::current_dir()?.join(&vault.root)
    })?;
    let mut registrations: Vec<_> = read(vault)?
        .projects
        .into_iter()
        .filter(|registration| registration.vault == selected_vault)
        .collect();
    registrations.sort_by(|left, right| left.path.cmp(&right.path));
    let projects = registrations
        .into_iter()
        .map(|registration| inspect_project(&registration.path, &selected_vault))
        .collect();
    Ok(RegistryInspectReport {
        projects,
        warnings: Vec::new(),
    })
}

fn inspect_project(project: &Path, vault: &Path) -> RegistryObservation {
    let observation = |status: &str, message: String, lock| RegistryObservation {
        project: project.to_path_buf(),
        status: status.into(),
        message,
        lock,
    };
    if let Err(error) = index::validate_path(project) {
        return observation("invalid_path", error.to_string(), None);
    }
    match fs::symlink_metadata(project) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return observation(
                "invalid_path",
                "Registered project path is not a directory".into(),
                None,
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return observation(
                "missing_project",
                "Registered project directory is missing".into(),
                None,
            );
        }
        Err(error) => return observation("invalid_path", error.to_string(), None),
    }
    let lock = match lockfile::read(project) {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            return observation(
                "missing_lock",
                "Registered project has no .kinana.lock".into(),
                None,
            );
        }
        Err(error) => {
            let status = if matches!(error.code.as_str(), "SYMLINK_PATH" | "INVALID_PATH") {
                "invalid_path"
            } else {
                "invalid_lock"
            };
            return observation(status, error.to_string(), None);
        }
    };
    if let Err(error) = lockfile::validate_for_vault(project, vault, &lock) {
        let status = match error.code.as_str() {
            "FOREIGN_PROJECT_LOCK" => "foreign_lock",
            "SYMLINK_PATH" | "INVALID_PATH" => "invalid_path",
            _ => "invalid_lock",
        };
        return observation(status, error.to_string(), Some(lock));
    }
    observation(
        "ok",
        "Only lock data was validated; activation targets and agent loading remain unchecked."
            .into(),
        Some(lock),
    )
}

fn absolute_lexical(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute()
        || path
            .to_str()
            .is_none_or(|text| text.chars().any(char::is_control))
    {
        return Err(conflict(
            "Registered project and vault paths must be absolute Unicode paths without control characters",
        ));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                return Err(conflict(
                    "Parent path components are not allowed in project registry paths",
                ));
            }
            Component::CurDir => {}
            _ => normalized.push(component.as_os_str()),
        }
    }
    Ok(normalized)
}

fn validate(registry: &ProjectRegistry) -> Result<()> {
    if registry.version != 1 {
        return Err(conflict("Unsupported project registry version"));
    }
    let mut paths = BTreeSet::new();
    for registration in &registry.projects {
        let project = absolute_lexical(&registration.path)?;
        absolute_lexical(&registration.vault)?;
        if !paths.insert(project) {
            return Err(conflict(
                "A project path occurs more than once in the registry",
            ));
        }
    }
    Ok(())
}

fn parse(bytes: &[u8]) -> Result<ProjectRegistry> {
    let text = std::str::from_utf8(bytes).map_err(|error| conflict(error.to_string()))?;
    let mut known: toml::Value = toml::from_str(text)
        .map_err(|error| conflict(format!("Invalid projects.toml: {error}")))?;
    if let Some(table) = known.as_table_mut() {
        table.retain(|key, _| matches!(key, "version" | "projects"));
    }
    if let Some(items) = known
        .get_mut("projects")
        .and_then(toml::Value::as_array_mut)
    {
        for item in items {
            if let Some(table) = item.as_table_mut() {
                table.retain(|key, _| matches!(key, "path" | "vault"));
            }
        }
    }
    let registry: ProjectRegistry = known
        .try_into()
        .map_err(|error| conflict(format!("Invalid or foreign projects.toml: {error}")))?;
    validate(&registry)?;
    Ok(registry)
}

fn serialize_preserving(registry: &ProjectRegistry, original: Option<&[u8]>) -> Result<Vec<u8>> {
    let mut document = match original {
        Some(bytes) => toml::from_str::<toml::Value>(
            std::str::from_utf8(bytes).map_err(|e| conflict(e.to_string()))?,
        )?,
        None => toml::Value::try_from(registry)?,
    };
    let previous = document.get("projects").and_then(toml::Value::as_array);
    let items = registry
        .projects
        .iter()
        .map(|registration| -> Result<_> {
            let mut item = toml::Value::try_from(registration)?;
            if let Some(old) = previous
                .and_then(|items| items.iter().find(|old| old.get("path") == item.get("path")))
                && let Some(table) = old.as_table()
            {
                for (key, value) in table {
                    if !matches!(key.as_str(), "path" | "vault") {
                        item.as_table_mut()
                            .expect("registration table")
                            .insert(key.clone(), value.clone());
                    }
                }
            }
            Ok(item)
        })
        .collect::<Result<Vec<_>>>()?;
    let table = document
        .as_table_mut()
        .ok_or_else(|| conflict("Expected registry table"))?;
    table.insert(
        "version".into(),
        toml::Value::Integer(registry.version.into()),
    );
    table.insert("projects".into(), toml::Value::Array(items));
    Ok(toml::to_string_pretty(&document)?.into_bytes())
}

fn require_directory(path: &Path) -> Result<PathBuf> {
    let path = index::validate_path(path)?;
    if !fs::symlink_metadata(&path)?.is_dir() {
        return Err(conflict(format!(
            "Expected an existing directory: {}",
            path.display()
        )));
    }
    Ok(path)
}

fn conflict(message: impl Into<String>) -> Error {
    Error::decision("REGISTRY_CONFLICT", message)
}

fn changed(message: impl Into<String>) -> Error {
    Error::decision("REGISTRY_CHANGED", message)
}

struct Snapshot {
    bytes: Vec<u8>,
    identity: fs::Metadata,
}

fn snapshot(path: &Path) -> Result<Option<Snapshot>> {
    let path = index::validate_path(path)?;
    let before = match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => {
            return Err(conflict(format!(
                "Expected a regular state file: {}",
                path.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let bytes = fs::read(&path)?;
    index::validate_path(&path)?;
    let after = fs::symlink_metadata(&path)?;
    if !same_entry(&before, &after)
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return Err(changed("State file changed while it was read"));
    }
    Ok(Some(Snapshot {
        bytes,
        identity: after,
    }))
}

fn matches_snapshot(path: &Path, expected: Option<&Snapshot>) -> Result<bool> {
    Ok(match (snapshot(path)?, expected) {
        (None, None) => true,
        (Some(current), Some(expected)) => {
            same_entry(&current.identity, &expected.identity) && current.bytes == expected.bytes
        }
        _ => false,
    })
}

/// Guard one registry publication. Foreign replacements and changed backups
/// survive cleanup; a retained stage is reported with its recovery location.
struct RegistrySave {
    target: PathBuf,
    parent: PathBuf,
    parent_identity: fs::Metadata,
    before: Option<Snapshot>,
    after: Snapshot,
    stage: PathBuf,
    stage_identity: fs::Metadata,
    backed_up: bool,
    published: bool,
    finished: bool,
}

impl RegistrySave {
    fn new(target: PathBuf, before: Option<Snapshot>, bytes: Vec<u8>) -> Result<Self> {
        let parent = require_directory(
            target
                .parent()
                .ok_or_else(|| conflict("Registry path has no state parent"))?,
        )?;
        let parent_identity = fs::symlink_metadata(&parent)?;
        let stage = loop {
            let candidate = parent.join(format!(
                ".skillsbox-registry-{}-{}",
                std::process::id(),
                NEXT_STAGE.fetch_add(1, Ordering::Relaxed)
            ));
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&candidate) {
                Ok(()) => break candidate,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        };
        let stage_identity = fs::symlink_metadata(&stage)?;
        let mut transaction = Self {
            target,
            parent,
            parent_identity,
            before,
            after: Snapshot {
                bytes: Vec::new(),
                identity: stage_identity.clone(),
            },
            stage,
            stage_identity,
            backed_up: false,
            published: false,
            finished: false,
        };
        let temporary = transaction.stage.join("new.toml");
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        transaction.after.identity = file.metadata()?;
        // Record partial bytes too, so a failed write only removes its own data.
        let mut written = 0;
        while written < bytes.len() {
            let count = match file.write(&bytes[written..]) {
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            if count == 0 {
                return Err(std::io::Error::from(std::io::ErrorKind::WriteZero).into());
            }
            written += count;
            transaction.after.bytes = bytes[..written].to_vec();
        }
        transaction.after.identity = file.metadata()?;
        file.sync_all()?;
        Ok(transaction)
    }

    fn verify_stage(&self) -> Result<()> {
        require_directory(&self.parent)?;
        if !same_entry(&self.parent_identity, &fs::symlink_metadata(&self.parent)?) {
            return Err(changed("The registry state directory was replaced"));
        }
        require_directory(&self.stage)?;
        if !same_entry(&self.stage_identity, &fs::symlink_metadata(&self.stage)?) {
            return Err(changed("Private registry staging directory was replaced"));
        }
        Ok(())
    }

    fn publish(&mut self) -> Result<()> {
        self.verify_stage()?;
        if !matches_snapshot(&self.target, self.before.as_ref())? {
            return Err(changed("Registry changed during validation"));
        }
        if let Some(before) = &self.before {
            fs::rename(&self.target, self.stage.join("old.toml"))?;
            self.backed_up = true;
            if !matches_snapshot(&self.stage.join("old.toml"), Some(before))? {
                self.before = snapshot(&self.stage.join("old.toml"))?;
                return Err(changed(
                    "A concurrent registry replacement was captured and preserved",
                ));
            }
        }
        if !matches_snapshot(&self.stage.join("new.toml"), Some(&self.after))? {
            return Err(changed("Staged registry changed before publication"));
        }
        fs::hard_link(self.stage.join("new.toml"), &self.target).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                changed("A concurrent registry appeared before publication")
            } else {
                error.into()
            }
        })?;
        self.published = true;
        self.verify_published()
    }

    fn verify_published(&self) -> Result<()> {
        self.verify_stage()?;
        if !matches_snapshot(&self.target, Some(&self.after))? {
            return Err(changed("Published registry changed concurrently"));
        }
        Ok(())
    }

    fn rollback(&mut self) {
        if self.verify_stage().is_err() {
            return;
        }
        if self.published {
            if matches_snapshot(&self.target, Some(&self.after)).is_ok_and(|matches| matches) {
                if fs::remove_file(&self.target).is_ok() {
                    self.published = false;
                }
            } else {
                self.published = false;
            }
        }
        if self.backed_up && !self.published {
            let backup = self.stage.join("old.toml");
            if matches_snapshot(&backup, self.before.as_ref()).is_ok_and(|matches| matches)
                && fs::hard_link(&backup, &self.target).is_ok()
                && fs::remove_file(&backup).is_ok()
            {
                self.backed_up = false;
            }
        }
    }

    fn cleanup(&self) -> Option<String> {
        if fs::symlink_metadata(&self.stage)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        {
            return None;
        }
        if self.verify_stage().is_err() || (!self.finished && (self.backed_up || self.published)) {
            return Some(self.retained_warning());
        }
        for (name, expected) in [
            ("new.toml", Some(&self.after)),
            ("old.toml", self.before.as_ref()),
        ] {
            let path = self.stage.join(name);
            if let Some(expected) = expected
                && matches_snapshot(&path, Some(expected)).is_ok_and(|matches| matches)
            {
                let _cleanup = fs::remove_file(path);
            }
        }
        match fs::remove_dir(&self.stage) {
            Ok(()) => None,
            Err(_) => Some(self.retained_warning()),
        }
    }

    fn retained_warning(&self) -> String {
        format!(
            "Registry staging contains modified or unremovable data and was preserved at {}",
            self.stage.display()
        )
    }
}

impl Drop for RegistrySave {
    fn drop(&mut self) {
        if !self.finished {
            self.rollback();
        }
        let _cleanup = self.cleanup();
    }
}

#[cfg(unix)]
fn same_entry(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(not(unix))]
fn same_entry(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.file_type() == right.file_type()
        && left
            .created()
            .ok()
            .zip(right.created().ok())
            .is_some_and(|(left, right)| left == right)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Result<Self> {
            let root = std::env::temp_dir().canonicalize()?.join(format!(
                "skillsbox-registry-publication-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|error| std::io::Error::other(error.to_string()))?
                    .as_nanos(),
                NEXT_STAGE.fetch_add(1, Ordering::Relaxed),
            ));
            fs::create_dir(&root)?;
            Ok(Self(root))
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _cleanup = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn publication_rejects_equal_bytes_in_a_concurrent_new_inode() -> Result<()> {
        let temp = Temp::new()?;
        let target = temp.0.join("projects.toml");
        fs::write(&target, b"original")?;
        let before = snapshot(&target)?;
        let mut transaction = RegistrySave::new(target.clone(), before, b"updated".to_vec())?;
        let stage = transaction.stage.clone();
        fs::rename(&target, temp.0.join("concurrent-backup"))?;
        fs::write(&target, b"original")?;
        assert_eq!(transaction.publish().unwrap_err().code, "REGISTRY_CHANGED");
        transaction.rollback();
        assert!(transaction.cleanup().is_none());
        assert_eq!(fs::read(&target)?, b"original");
        assert_eq!(fs::read(temp.0.join("concurrent-backup"))?, b"original");
        assert!(!stage.exists());
        Ok(())
    }

    #[test]
    fn rollback_restores_original_identity_and_removes_only_owned_staging() -> Result<()> {
        let temp = Temp::new()?;
        let target = temp.0.join("projects.toml");
        fs::write(&target, b"original")?;
        let before = snapshot(&target)?.expect("original registry");
        let identity = before.identity.clone();
        let mut transaction = RegistrySave::new(target.clone(), Some(before), b"updated".to_vec())?;
        let stage = transaction.stage.clone();
        transaction.publish()?;
        assert_eq!(fs::read(&target)?, b"updated");
        transaction.rollback();
        assert!(transaction.cleanup().is_none());
        assert_eq!(fs::read(&target)?, b"original");
        assert!(same_entry(&identity, &fs::symlink_metadata(&target)?));
        assert!(!stage.exists());
        Ok(())
    }

    #[test]
    fn concurrent_target_and_backup_edits_survive_rollback_and_report_recovery_location()
    -> Result<()> {
        let temp = Temp::new()?;
        let target = temp.0.join("projects.toml");
        fs::write(&target, b"original")?;
        let before = snapshot(&target)?;
        let mut transaction = RegistrySave::new(target.clone(), before, b"updated".to_vec())?;
        transaction.publish()?;
        let stage = transaction.stage.clone();
        fs::write(stage.join("old.toml"), b"concurrent backup data")?;
        fs::write(&target, b"concurrent published data")?;
        transaction.rollback();
        let warning = transaction.cleanup().expect("modified data must remain");
        assert!(warning.contains(&stage.display().to_string()));
        drop(transaction);
        assert_eq!(fs::read(&target)?, b"concurrent published data");
        assert_eq!(fs::read(stage.join("old.toml"))?, b"concurrent backup data");
        assert_eq!(
            fs::read(stage.join("new.toml"))?,
            b"concurrent published data"
        );
        Ok(())
    }

    #[test]
    fn successful_publication_keeps_a_modified_backup_and_warns_instead_of_deleting_it()
    -> Result<()> {
        let temp = Temp::new()?;
        let target = temp.0.join("projects.toml");
        fs::write(&target, b"original")?;
        let before = snapshot(&target)?;
        let mut transaction = RegistrySave::new(target.clone(), before, b"updated".to_vec())?;
        transaction.publish()?;
        let stage = transaction.stage.clone();
        fs::write(stage.join("old.toml"), b"concurrent backup data")?;
        transaction.finished = true;
        let warning = transaction.cleanup().expect("modified backup must remain");
        assert!(warning.contains(&stage.display().to_string()));
        drop(transaction);
        assert_eq!(fs::read(&target)?, b"updated");
        assert_eq!(fs::read(stage.join("old.toml"))?, b"concurrent backup data");
        assert!(!stage.join("new.toml").exists());
        Ok(())
    }
}
