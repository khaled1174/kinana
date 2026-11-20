//! Stable, read-only hashing of complete skill directories.
//!
//! The encoding includes entry types, path component counts and lengths, and
//! content lengths. Empty directories are included; the outer directory name,
//! timestamps, permissions, and enumeration order are excluded.

use crate::{Error, Result};
use std::collections::BTreeSet;
use std::fs::{self, File, Metadata};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

struct Entry {
    relative: PathBuf,
    is_directory: bool,
}

/// Hash all names and file contents without accepting symbolic links.
///
/// A link anywhere in the supplied path or directory tree is an explicit error,
/// including dangling links. Special files are rejected rather than opened.
pub fn hash_directory(path: &Path) -> Result<String> {
    hash_with_repository_metadata(path, false, None)
}

/// Git's private database is not part of a root-level skill's source payload.
pub(crate) fn hash_payload_directory(path: &Path) -> Result<String> {
    hash_with_repository_metadata(path, true, None)
}

/// What the directory hashes to with the named files left out, under the same
/// definition. Used only to ask whether those files are a skill's only
/// difference from its recorded hash.
pub(crate) fn hash_directory_without(path: &Path, leave_out: &BTreeSet<PathBuf>) -> Result<String> {
    hash_with_repository_metadata(path, false, Some(leave_out))
}

fn hash_with_repository_metadata(
    path: &Path,
    skip_git: bool,
    leave_out: Option<&BTreeSet<PathBuf>>,
) -> Result<String> {
    let root = checked_absolute_path(path)?;
    if !checked_metadata(&root)?.is_dir() {
        return Err(Error::new(
            "NOT_A_DIRECTORY",
            format!("Skill path is not a directory: {}", root.display()),
        ));
    }

    let mut entries = Vec::new();
    collect_entries(&root, Path::new(""), &mut entries, skip_git)?;
    if let Some(leave_out) = leave_out {
        entries.retain(|entry| entry.is_directory || !leave_out.contains(&entry.relative));
    }
    entries.sort_by(|left, right| left.relative.cmp(&right.relative));

    let mut hasher = blake3::Hasher::new();
    hasher.update(b"skillsbox-directory-v1\0");
    for entry in entries {
        let absolute = root.join(&entry.relative);
        checked_absolute_path(&absolute)?;
        let metadata = checked_metadata(&absolute)?;
        if metadata.is_dir() != entry.is_directory {
            return Err(changed(&absolute));
        }
        hasher.update(if entry.is_directory { b"d" } else { b"f" });
        let components = entry.relative.components().collect::<Vec<_>>();
        hasher.update(&(components.len() as u64).to_le_bytes());
        for component in components {
            let bytes = component.as_os_str().as_encoded_bytes();
            hasher.update(&(bytes.len() as u64).to_le_bytes());
            hasher.update(bytes);
        }
        if !entry.is_directory {
            if !metadata.is_file() {
                return Err(unsupported(&absolute));
            }
            let expected_length = metadata.len();
            hasher.update(&expected_length.to_le_bytes());
            let mut file = File::open(&absolute).map_err(|error| io_error(&absolute, error))?;
            let mut buffer = [0_u8; 16 * 1024];
            let mut actual_length = 0_u64;
            loop {
                let read = file
                    .read(&mut buffer)
                    .map_err(|error| io_error(&absolute, error))?;
                if read == 0 {
                    break;
                }
                actual_length += read as u64;
                hasher.update(&buffer[..read]);
            }
            if actual_length != expected_length {
                return Err(changed(&absolute));
            }
        }
    }
    Ok(format!("blake3:{}", hasher.finalize().to_hex()))
}

fn collect_entries(
    root: &Path,
    relative: &Path,
    entries: &mut Vec<Entry>,
    skip_git: bool,
) -> Result<()> {
    let directory = root.join(relative);
    // Recheck directory components before opening a directory reached earlier.
    checked_absolute_path(&directory)?;
    let children = fs::read_dir(&directory).map_err(|error| io_error(&directory, error))?;
    for child in children {
        let child = child.map_err(|error| io_error(&directory, error))?;
        if skip_git && relative.as_os_str().is_empty() && child.file_name() == ".git" {
            continue;
        }
        let child_relative = relative.join(child.file_name());
        let path = root.join(&child_relative);
        let metadata = checked_metadata(&path)?;
        if !metadata.is_file() && !metadata.is_dir() {
            return Err(unsupported(&path));
        }
        let is_directory = metadata.is_dir();
        entries.push(Entry {
            relative: child_relative.clone(),
            is_directory,
        });
        if is_directory {
            collect_entries(root, &child_relative, entries, skip_git)?;
        }
    }
    Ok(())
}

/// Make a path absolute only after checking every existing component for links.
/// This intentionally avoids `canonicalize`, which follows symbolic links.
pub(crate) fn checked_absolute_path(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut checked = PathBuf::new();
    for component in absolute.components() {
        checked.push(component.as_os_str());
        checked_metadata(&checked)?;
    }
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    Ok(normalized)
}

pub(crate) fn checked_metadata(path: &Path) -> Result<Metadata> {
    let metadata = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
    if metadata.file_type().is_symlink() {
        return Err(Error::new(
            "SYMLINK_REJECTED",
            format!("Rejected symbolic link: {}", path.display()),
        ));
    }
    Ok(metadata)
}

fn changed(path: &Path) -> Error {
    Error::new(
        "FILE_CHANGED",
        format!("Path changed while hashing: {}", path.display()),
    )
}

fn unsupported(path: &Path) -> Error {
    Error::new(
        "UNSUPPORTED_FILE",
        format!(
            "Only regular files and directories can be hashed: {}",
            path.display()
        ),
    )
}

fn io_error(path: &Path, error: std::io::Error) -> Error {
    Error::new("IO_ERROR", format!("{}: {error}", path.display()))
}
