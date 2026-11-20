//! Delete one skill from the vault: whole-operation preflight, one Git commit.
//!
//! Nothing is unlinked in place. Each owned node is first moved into a private
//! stage inside the ignored `dist/` directory and compared with the snapshot
//! taken during preflight; the stage is only discarded after the commit. Links
//! are never followed, and project files are never read for writing.
use crate::activation_transaction::{
    self as tx, ActivationTransaction, Data, PlannedWrite, Snapshot,
};
use crate::{
    Bundle, Error, RemoveReport, Result, build, bundles, gitlog, index, lockfile, project_registry,
    vault::Vault,
};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub fn remove(vault: &Vault, id: &str, dry_run: bool) -> Result<RemoveReport> {
    let _state_guard = crate::state::write_guard(&vault.index_path, dry_run)?;
    // The ID is data. It must be one safe component before it joins any path.
    crate::vault::validate_id(id)?;
    let vault = Vault::new(
        index::validate_path(&vault.root)?,
        index::validate_path(&vault.index_path)?,
    );
    let vault = &vault;
    vault.config()?;
    let source = index::validate_path(&vault.root.join("skills").join(id))?;
    if absent(&source)? {
        return Err(Error::new(
            "SKILL_NOT_FOUND",
            format!("No skill with ID '{id}' exists in the vault"),
        ));
    }
    vault.show(id)?;
    let meta = index::validate_path(&vault.root.join("meta").join(format!("{id}.toml")))?;

    let using = projects_using(vault, id)?;
    if !using.is_empty() {
        return Err(Error::decision(
            "SKILL_IN_USE",
            format!(
                "Skill '{id}' is active in registered project(s): {}. Run unuse for each project first; no project file was changed",
                using
                    .iter()
                    .map(|project| project.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }

    let dist = index::validate_path(&vault.root.join("dist"))?;
    let bundle_directory = index::validate_path(&vault.root.join("bundles"))?;
    let mut bundle_writes = Vec::new();
    let mut bundles_changed = Vec::new();
    for bundle in bundles::list(vault)? {
        if !bundle.skills.iter().any(|member| member.id == id) {
            continue;
        }
        let mut updated = bundle.clone();
        updated.skills.retain(|member| member.id != id);
        bundles::validate(vault, &updated).map_err(|error| {
            Error::decision(
                "BUNDLE_WOULD_BE_INVALID",
                format!(
                    "Removing '{id}' would leave bundle '{}' invalid ({}); edit or delete that bundle first",
                    bundle.name, error.message
                ),
            )
        })?;
        let target = bundle_directory.join(format!("{}.toml", bundle.name));
        let before = Snapshot::read(&target)?;
        // The file may have been replaced since `list` validated it.
        let current = before
            .as_ref()
            .and_then(Snapshot::bytes)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .and_then(|text| toml::from_str::<Bundle>(text).ok());
        if current.as_ref() != Some(&bundle) {
            return Err(Error::decision(
                "BUNDLE_CHANGED",
                format!("Bundle '{}' changed during preflight", bundle.name),
            ));
        }
        let mode = before
            .as_ref()
            .and_then(Snapshot::file_mode)
            .unwrap_or(0o600);
        bundle_writes.push(PlannedWrite {
            target,
            stage_parent: dist.clone(),
            before,
            data: Data::FileWithMode(toml::to_string_pretty(&updated)?.into_bytes(), mode),
        });
        bundles_changed.push(bundle.name);
    }

    // Snapshots refuse links and special files anywhere in the owned trees.
    let cache = build::plan_removal(vault, id)?;
    let payload = build::source_payload(&source)?;
    let source_before = Snapshot::read(&source)?;
    let meta_before = Snapshot::read(&meta)?;
    if source_before.as_ref().and_then(Snapshot::digest).is_none()
        || meta_before.as_ref().and_then(Snapshot::bytes).is_none()
    {
        return Err(Error::new(
            "INVALID_VAULT",
            format!("Skill '{id}' must be a real directory with a regular metadata file"),
        ));
    }

    let mut report = RemoveReport {
        id: id.into(),
        dry_run,
        changed: true,
        committed: false,
        removed_paths: [format!("skills/{id}"), format!("meta/{id}.toml")]
            .into_iter()
            .chain(cache.removed)
            .collect(),
        bundles_changed,
    };
    // Moves run in this order. Every tracked node goes first, the metadata
    // file before the skill directory, so from the first move until the
    // commit Git sees an uncommitted change and every other command refuses
    // the dirty vault; `doctor` names it and the way back. The ignored build
    // output goes last. After the commit only the private stages in `dist/`
    // remain, which `doctor` also reports.
    let mut writes = vec![
        PlannedWrite {
            target: meta,
            stage_parent: dist.clone(),
            before: meta_before,
            data: Data::Remove,
        },
        PlannedWrite {
            target: source.clone(),
            stage_parent: dist.clone(),
            before: source_before,
            data: Data::Remove,
        },
    ];
    writes.extend(bundle_writes);
    writes.extend(cache.writes);

    let source_roots = vault
        .records()?
        .into_iter()
        .flat_map(|record| record.source.found_in)
        .collect::<Vec<_>>();
    gitlog::reject_source_executable(&vault.root, &source_roots)?;
    // Before the general cleanliness check, so that a file Git does not keep
    // inside this skill is named with its own code, not as a dirty vault.
    let lost = unrecoverable(vault, id, &source)?;
    if !lost.is_empty() {
        return Err(Error::decision(
            "SKILL_HAS_UNRECOVERABLE_CONTENT",
            format!(
                "Skill '{id}' holds content that undo could not bring back after removal: {}. Git does not keep a nested .git directory, ignored files or unrecorded empty directories. Move them out of skills/{id} first; nothing was changed",
                lost.join(", ")
            ),
        ));
    }
    gitlog::ensure_clean(&vault.root)?;
    index::check(&vault.index_path)?;
    let anchors = anchors(&[
        &vault.root,
        &vault.root.join("skills"),
        &vault.root.join("meta"),
        &bundle_directory,
        &dist,
    ])?;
    tx::preflight(&writes, &[], &anchors)?;
    if dry_run {
        return Ok(report);
    }

    let mut paths = payload
        .entries
        .iter()
        .filter(|entry| entry.bytes.is_some())
        .map(|entry| Path::new("skills").join(id).join(&entry.relative))
        .collect::<BTreeSet<_>>();
    paths.insert(Path::new("meta").join(format!("{id}.toml")));
    for name in &report.bundles_changed {
        paths.insert(Path::new("bundles").join(format!("{name}.toml")));
    }
    publish(
        vault,
        writes,
        anchors,
        &paths.into_iter().collect::<Vec<_>>(),
        &format!("Remove skill {id}"),
        &format!("Skill '{id}' was removed"),
    )?;
    report.committed = true;
    Ok(report)
}

/// Registered projects of this vault whose lock records an activation of `id`.
/// The registry and lock files are the authority; the SQLite cache is not read.
/// A lock that cannot be read or validated stops the removal: non-use is unproven.
pub(crate) fn projects_using(vault: &Vault, id: &str) -> Result<Vec<PathBuf>> {
    let mut projects = project_registry::read(vault)?
        .projects
        .into_iter()
        .filter(|registration| same_directory(&registration.vault, &vault.root))
        .map(|registration| registration.path)
        .collect::<Vec<_>>();
    projects.sort();
    projects.dedup();
    let mut using = Vec::new();
    for project in projects {
        let active = (|| -> Result<bool> {
            let path = index::validate_path(&project)?;
            if absent(&path)? {
                return Ok(false);
            }
            let Some(lock) = lockfile::read(&path)? else {
                return Ok(false);
            };
            if !same_directory(&lock.vault, &vault.root) {
                // The lock belongs to another vault; its entries are not ours.
                return Ok(false);
            }
            lockfile::validate_for_vault(&path, &lock.vault, &lock)?;
            Ok(lock.active.iter().any(|skill| skill.id == id))
        })();
        match active {
            Ok(true) => using.push(project),
            Ok(false) => {}
            Err(mut error) => {
                error.message = format!(
                    "Cannot verify that '{id}' is unused: the lock of registered project {} could not be read ({}). Repair or remove that project's lock, then retry",
                    project.display(),
                    error.message
                );
                return Err(error);
            }
        }
    }
    Ok(using)
}

/// A recorded vault path names this vault when it is the same directory on
/// disk. Other commands compare the spelling only; before deleting, a
/// different spelling of the same directory (case- or normalization-insensitive
/// filesystems) must still count as use. This only reads directory identity.
fn same_directory(recorded: &Path, root: &Path) -> bool {
    if recorded == root {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (fs::metadata(recorded), fs::metadata(root)) {
            (Ok(left), Ok(right)) => {
                left.is_dir()
                    && right.is_dir()
                    && left.dev() == right.dev()
                    && left.ino() == right.ino()
            }
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

pub(crate) fn anchors(paths: &[&Path]) -> Result<Vec<(PathBuf, fs::Metadata)>> {
    let mut anchors = Vec::new();
    for path in paths {
        let path = index::validate_path(path)?;
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_dir() {
            return Err(Error::new(
                "INVALID_VAULT",
                format!("Expected directory: {}", path.display()),
            ));
        }
        anchors.push((path, metadata));
    }
    Ok(anchors)
}

/// Move the planned nodes into private staging, commit exactly `paths`, then
/// discard the staging and refresh this vault's cache rows. Any failure up to
/// and including the commit puts every node back and restores Git's index.
pub(crate) fn publish(
    vault: &Vault,
    writes: Vec<PlannedWrite>,
    anchors: Vec<(PathBuf, fs::Metadata)>,
    paths: &[PathBuf],
    message: &str,
    done: &str,
) -> Result<()> {
    let mut filesystem = ActivationTransaction::prepare(writes, Vec::new(), anchors)?;
    if let Err(error) = filesystem.apply().and_then(|_| filesystem.verify()) {
        return Err(filesystem.abort(error));
    }
    if let Err(error) = gitlog::commit_paths(&vault.root, message, paths) {
        let error = filesystem.abort(error);
        let _ = gitlog::unstage_paths(&vault.root, paths);
        return Err(error);
    }
    let cleanup = filesystem.finish();
    let refreshed = vault
        .records()
        .and_then(|records| index::rebuild_for_vault(&vault.index_path, &vault.root, &records));
    if let Err(error) = refreshed {
        return Err(Error::new(
            "INDEX_REFRESH_FAILED",
            format!(
                "{done} and committed, but index refresh failed: {error}; run doctor --rebuild-index"
            ),
        ));
    }
    if !cleanup.is_empty() {
        return Err(Error::new(
            "REMOVAL_CLEANUP_FAILED",
            format!(
                "{done} and committed, but private staging was preserved: {}",
                cleanup.join("; ")
            ),
        ));
    }
    Ok(())
}

/// The most paths an unrecoverable-content refusal names.
const UNRECOVERABLE_SHOWN: usize = 10;

/// Paths under `skills/<id>` that `undo` could not restore once the removal is
/// committed: a `.git` directory at any depth, a file Git does not track (an
/// ignored one), and an empty directory the metadata does not record. Finder
/// inventory files are not content. Nothing is followed or read.
fn unrecoverable(vault: &Vault, id: &str, source: &Path) -> Result<Vec<String>> {
    let prefix = format!("skills/{id}/");
    let tracked = gitlog::tracked_under(&vault.root, &prefix)?;
    let recorded = crate::edit::read_metadata(vault, id)?
        .empty_directories
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut found = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        let mut entries = fs::read_dir(source.join(&relative))?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<std::io::Result<Vec<_>>>()?;
        entries.sort();
        // Finder inventory is deleted with the skill and never counts as
        // content; a folder holding only that is empty for undo too.
        let empty = entries.iter().all(|name| gitlog::is_inventory_name(name));
        if empty && !relative.as_os_str().is_empty() && !recorded.contains(&relative) {
            found.push(format!("{prefix}{}/", relative.display()));
        }
        for name in entries {
            let child = relative.join(&name);
            let metadata = fs::symlink_metadata(source.join(&child))?;
            if name == ".git" {
                found.push(format!("{prefix}{}", child.display()));
            } else if metadata.is_dir() {
                pending.push(child);
            } else if !gitlog::is_inventory_name(&name)
                && !tracked.contains(&format!("{prefix}{}", child.display()))
            {
                found.push(format!("{prefix}{}", child.display()));
            }
        }
    }
    found.sort();
    let more = found.len().saturating_sub(UNRECOVERABLE_SHOWN);
    found.truncate(UNRECOVERABLE_SHOWN);
    if more > 0 {
        found.push(format!("and {more} more"));
    }
    Ok(found)
}

fn absent(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error.into()),
    }
}
