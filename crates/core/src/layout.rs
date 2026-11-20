//! Preserve empty source directories in the vault's separate metadata layer.
//!
//! Git records files, so reverting a previous revert cannot recreate an empty
//! source directory. The metadata records only empty leaves; their parents can
//! then be reconstructed without adding marker files to the original skill.

use crate::{Error, Result, SkillMetadata, index};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Collect sorted relative paths to empty leaf directories without writing.
/// The skill root itself and its root-level `.git` metadata are excluded.
pub fn empty_directories(skill_root: &Path) -> Result<Vec<PathBuf>> {
    let root = require_directory(skill_root)?;
    let mut directories = Vec::new();
    collect_empty(&root, Path::new(""), &mut directories)?;
    directories.sort();
    Ok(directories)
}

fn collect_empty(root: &Path, relative: &Path, empty: &mut Vec<PathBuf>) -> Result<()> {
    let directory = require_directory(&root.join(relative))?;
    let mut has_entries = false;
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        if relative.as_os_str().is_empty() && entry.file_name() == ".git" {
            continue;
        }
        has_entries = true;
        let child = relative.join(entry.file_name());
        let path = index::validate_path(&root.join(&child))?;
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() {
            collect_empty(root, &child, empty)?;
        } else if !metadata.is_file() {
            return Err(unsupported(&path));
        }
    }
    if !has_entries && !relative.as_os_str().is_empty() {
        validate_relative(relative)?;
        empty.push(relative.to_path_buf());
    }
    Ok(())
}

/// Recreate directories recorded by metadata after a successful Git undo.
///
/// All manifests and existing path components are checked before the first
/// directory is created. A missing skill or manifest never recreates a removed
/// skill. On failure, only this call's still-empty created directories are
/// eligible for rollback; no files are created, replaced, or executed.
pub fn restore(vault_root: &Path) -> Result<()> {
    let root = require_directory(vault_root)?;
    let skills = require_directory(&root.join("skills"))?;
    let meta = require_directory(&root.join("meta"))?;
    let mut metadata_paths = Vec::new();
    for entry in fs::read_dir(&meta)? {
        let entry = entry?;
        if entry.file_name() == ".DS_Store" {
            continue;
        }
        metadata_paths.push(entry.path());
    }
    metadata_paths.sort();

    let mut planned = BTreeSet::new();
    for path in metadata_paths {
        require_file(&path)?;
        let id = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".toml"))
            .ok_or_else(|| invalid(format!("Unexpected metadata file: {}", path.display())))?;
        validate_id(id)?;
        let metadata: SkillMetadata = toml::from_str(&fs::read_to_string(&path)?)?;
        if metadata.id != id {
            return Err(Error::new(
                "METADATA_ID_MISMATCH",
                format!("Metadata ID {} does not match filename {id}", metadata.id),
            ));
        }
        let skill = require_directory(&skills.join(id))?;
        require_file(&skill.join("SKILL.md"))?;
        for relative in &metadata.empty_directories {
            validate_relative(relative)?;
            let mut target = skill.clone();
            for component in relative.components() {
                target.push(component.as_os_str());
                let target = index::validate_path(&target)?;
                match fs::symlink_metadata(&target) {
                    Ok(metadata) if metadata.is_dir() => {}
                    Ok(_) => {
                        return Err(invalid(format!(
                            "Expected directory while restoring layout: {}",
                            target.display()
                        )));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        planned.insert(target);
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        }
    }

    let mut transaction = DirectoryTransaction::default();
    for directory in planned {
        transaction.create(&directory)?;
    }
    transaction.finished = true;
    Ok(())
}

pub(crate) fn validate_relative(path: &Path) -> Result<()> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(name)
                if !name
                    .as_encoded_bytes()
                    .iter()
                    .any(|byte| matches!(byte, b'\\' | b':' | 0)) =>
            {
                normalized.push(name);
            }
            _ => return Err(invalid_path(path)),
        }
    }
    if normalized.as_os_str().is_empty()
        || normalized.as_os_str() != path.as_os_str()
        || normalized.components().next() == Some(Component::Normal(".git".as_ref()))
    {
        return Err(invalid_path(path));
    }
    Ok(())
}

fn validate_id(id: &str) -> Result<()> {
    crate::vault::validate_id(id)
}

fn require_directory(path: &Path) -> Result<PathBuf> {
    let path = index::validate_path(path)?;
    if !fs::symlink_metadata(&path)?.is_dir() {
        return Err(invalid(format!("Expected directory: {}", path.display())));
    }
    Ok(path)
}

fn require_file(path: &Path) -> Result<()> {
    let path = index::validate_path(path)?;
    if !fs::symlink_metadata(&path)?.is_file() {
        return Err(invalid(format!(
            "Expected regular file: {}",
            path.display()
        )));
    }
    Ok(())
}

fn invalid_path(path: &Path) -> Error {
    invalid(format!(
        "Empty-directory paths must be normalized safe relative paths: {}",
        path.display()
    ))
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new("INVALID_LAYOUT", message)
}

fn unsupported(path: &Path) -> Error {
    Error::new(
        "UNSUPPORTED_FILE",
        format!(
            "Only regular files and directories can be preserved: {}",
            path.display()
        ),
    )
}

#[derive(Default)]
struct DirectoryTransaction {
    created: Vec<(PathBuf, fs::Metadata)>,
    finished: bool,
}

impl DirectoryTransaction {
    fn create(&mut self, path: &Path) -> Result<()> {
        // Recheck before each mutation, including ancestors that existed at
        // preflight. An existing directory is never owned by this transaction.
        let path = index::validate_path(path)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() => return Ok(()),
            Ok(_) => {
                return Err(invalid(format!(
                    "Expected directory while restoring layout: {}",
                    path.display()
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        fs::create_dir(&path)?;
        self.created
            .push((path.clone(), fs::symlink_metadata(&path)?));
        Ok(())
    }
}

impl Drop for DirectoryTransaction {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        for (path, original) in self.created.iter().rev() {
            if index::validate_path(path).is_err() {
                continue;
            }
            let Ok(current) = fs::symlink_metadata(path) else {
                continue;
            };
            if current.is_dir() && same_directory(original, &current) {
                // remove_dir refuses nonempty directories and never removes
                // unrecorded files that appeared after creation.
                let _cleanup = fs::remove_dir(path);
            }
        }
    }
}

#[cfg(unix)]
fn same_directory(original: &fs::Metadata, current: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    original.dev() == current.dev() && original.ino() == current.ino()
}

#[cfg(not(unix))]
fn same_directory(original: &fs::Metadata, current: &fs::Metadata) -> bool {
    original.file_type() == current.file_type()
        && original
            .created()
            .ok()
            .zip(current.created().ok())
            .is_some_and(|(original, current)| original == current)
}

#[cfg(test)]
mod tests {
    use super::DirectoryTransaction;
    use std::fs;

    #[test]
    fn rollback_removes_only_owned_empty_directories() {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "skillsbox-layout-rollback-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        fs::create_dir(&root).unwrap();
        let existing = root.join("existing");
        fs::create_dir(&existing).unwrap();
        let blocker = root.join("blocker");
        fs::write(&blocker, "existing bytes").unwrap();
        let created = root.join("created");
        {
            let mut transaction = DirectoryTransaction::default();
            transaction.create(&existing).unwrap();
            transaction.create(&created).unwrap();
            transaction.create(&created.join("empty")).unwrap();
            assert!(transaction.create(&blocker).is_err());
        }
        assert!(!created.exists());
        assert!(existing.is_dir());
        assert_eq!(fs::read(&blocker).unwrap(), b"existing bytes");

        {
            let mut transaction = DirectoryTransaction::default();
            transaction.create(&created).unwrap();
            fs::write(created.join("appeared.txt"), "preserve concurrent content").unwrap();
        }
        assert_eq!(
            fs::read_to_string(created.join("appeared.txt")).unwrap(),
            "preserve concurrent content"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
