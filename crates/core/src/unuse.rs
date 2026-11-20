//! Remove only recorded, unchanged activation nodes. Whole-batch preflight.
use crate::activation_transaction::{
    self as tx, ActivationTransaction, Data, PlannedWrite, Snapshot,
};
use crate::{Error, Result, UnuseOptions, UnuseReport, index, lockfile, status, vault::Vault};
use std::{collections::BTreeSet, fs, path::PathBuf};

pub fn unuse(vault: &Vault, options: &UnuseOptions) -> Result<UnuseReport> {
    let _state_guard = crate::state::write_guard(&vault.index_path, options.dry_run)?;
    let normalized = Vault::new(
        index::validate_path(&vault.root)?,
        index::validate_path(&vault.index_path)?,
    );
    let vault = &normalized;
    if options.ids.is_empty()
        || options
            .agent
            .as_ref()
            .is_some_and(|agent| !matches!(agent.as_str(), "claude" | "codex" | "cursor" | "all"))
    {
        return Err(Error::new(
            "INVALID_ARGUMENTS",
            "Choose explicit skill IDs and an optional claude/codex/cursor/all agent",
        ));
    }
    let mut ids = BTreeSet::new();
    for id in &options.ids {
        crate::vault::validate_id(id)?;
        if !ids.insert(id.as_str()) {
            return Err(Error::new("INVALID_ARGUMENTS", "Repeated unuse ID"));
        }
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .ok_or_else(|| Error::new("HOME_NOT_FOUND", "Cannot resolve home"))?;
    let project = status::validate_mutation_project(vault, &options.project, &PathBuf::from(home))?;
    let mut report = UnuseReport {
        project: project.clone(),
        dry_run: options.dry_run,
        changed: false,
        removed: vec![],
        removed_directories: vec![],
        lock_removed: false,
        warnings: vec![],
    };
    let lock_path = lockfile::write_path(&project)?;
    let before = Snapshot::read(&lock_path)?;
    let Some(mut lock) = lockfile::read(&project)? else {
        return Ok(report);
    };
    lockfile::validate_for_vault(&project, &vault.root, &lock)?;
    if !tx::matches(&lock_path, before.as_ref())? {
        return Err(conflict("Project lock changed during preflight"));
    }
    let selected = lock
        .active
        .iter()
        .filter(|skill| {
            ids.contains(skill.id.as_str())
                && options
                    .agent
                    .as_deref()
                    .is_none_or(|agent| agent == "all" || agent == skill.agent)
        })
        .cloned()
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Ok(report);
    }
    // Back up project nodes on their own filesystem, including external drives.
    let stage = selected
        .iter()
        .map(|skill| project.join(lockfile::skill_prefix(skill).expect("validated agent")))
        .find(|parent| parent.is_dir())
        .unwrap_or_else(|| {
            project.join(lockfile::skill_prefix(&selected[0]).expect("validated agent"))
        });
    let mut writes = vec![];
    let mut observations = vec![];
    let mut removing = BTreeSet::new();
    let mut empty_directories = vec![];
    for skill in &selected {
        let target = lockfile::target_path(&project, skill)?;
        let snapshot = Snapshot::read(&target)
            .map_err(|_| conflict("Unsafe or modified target was preserved"))?;
        if let Some(snapshot) = &snapshot {
            if skill
                .ownership
                .as_ref()
                .is_none_or(|identity| *identity != snapshot.ownership())
            {
                return Err(conflict(
                    "Target has no matching ownership identity; modified, replaced or legacy target was preserved. Re-run use explicitly to establish a legacy identity.",
                ));
            }
            let owned = if skill.mode == "link" {
                snapshot.link() == Some(status::source(vault, skill).as_path())
            } else {
                snapshot.digest() == Some(skill.content_hash.as_str())
            };
            if !owned {
                return Err(conflict("Modified target was preserved"));
            }
            let retained = lock.active.iter().any(|other| {
                !selected
                    .iter()
                    .any(|selected| selected.id == other.id && selected.agent == other.agent)
                    && lockfile::target_path(&project, other).ok().as_ref() == Some(&target)
            });
            if retained {
                observations.push((target, Some(snapshot.clone())));
                continue;
            }
            if !removing.insert(target.clone()) {
                continue;
            }
            writes.push(PlannedWrite {
                target: target.clone(),
                stage_parent: target.parent().expect("target parent").to_path_buf(),
                before: snapshot.clone().into(),
                data: Data::Remove,
            });
            report.removed.push(target);
        } else {
            observations.push((target, None));
        }
    }
    lock.active.retain(|skill| {
        !selected
            .iter()
            .any(|selected| selected.id == skill.id && selected.agent == skill.agent)
    });
    if let Some(restoration) = &mut lock.restoration {
        let mut directories = restoration.directories.clone();
        directories.sort_by_key(|directory| std::cmp::Reverse(directory.path.len()));
        for directory in directories {
            let path = project.join(&directory.path);
            if !path.try_exists()? {
                continue;
            }
            if tx::directory_identity(&path)?.as_deref() != Some(directory.identity.as_str()) {
                report.warnings.push(format!(
                    "Replaced or modified directory preserved: {}",
                    path.display()
                ));
                continue;
            }
            let eligible = fs::read_dir(&path)?
                .map(|entry| entry.map(|entry| removing.contains(&entry.path())))
                .collect::<std::io::Result<Vec<_>>>()?
                .into_iter()
                .all(|owned| owned);
            if eligible {
                observations.push((path.clone(), Some(Snapshot::directory(&path)?)));
                empty_directories.push((path.clone(), directory.identity.clone()));
                removing.insert(path.clone());
                report.removed_directories.push(path.clone());
                restoration
                    .directories
                    .retain(|directory| project.join(&directory.path) != path);
            }
        }
    }
    let data = if lock.active.is_empty() {
        let restoration = lock.restoration.take();
        lock.bundle = None;
        if restoration
            .as_ref()
            .is_some_and(|restoration| restoration.created_lock)
        {
            report.lock_removed = true;
            Data::Remove
        } else if let Some(original) = restoration
            .as_ref()
            .and_then(|restoration| restoration.original_empty_lock.as_ref())
        {
            lock = toml::from_str(original)?;
            if let Some(mode) = restoration
                .as_ref()
                .and_then(|restoration| restoration.original_lock_mode)
            {
                Data::FileWithMode(original.as_bytes().to_vec(), mode)
            } else {
                Data::File(original.as_bytes().to_vec())
            }
        } else {
            Data::File(lockfile::serialize_preserving(&project, &lock)?)
        }
    } else {
        Data::File(lockfile::serialize_preserving(&project, &lock)?)
    };
    writes.push(PlannedWrite {
        target: lock_path,
        stage_parent: stage.clone(),
        before,
        data,
    });
    let mut anchors = vec![];
    let mut anchor_paths = BTreeSet::from([
        project.clone(),
        vault.root.clone(),
        vault.index_path.clone(),
    ]);
    for write in &writes {
        for path in write.stage_parent.ancestors() {
            anchor_paths.insert(path.to_path_buf());
        }
    }
    for path in &anchor_paths {
        index::validate_path(path)?;
        match fs::symlink_metadata(path) {
            Ok(metadata) => anchors.push((path.clone(), metadata)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    tx::preflight(&writes, &observations, &anchors)?;
    // Validate index and schema without writing, including dry-run.
    let _ = index::activation_connection(&vault.index_path, false)?;
    report.changed = true;
    if options.dry_run {
        return Ok(report);
    }
    let mut connection = index::activation_connection(&vault.index_path, true)?;
    let database =
        connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if report.lock_removed {
        index::remove_project_cache(&database, &project, &vault.root)?;
    } else {
        index::write_project_cache(&database, &project, &lock)?;
    }
    let mut filesystem = ActivationTransaction::prepare(writes, observations, anchors)?;
    if let Err(error) = filesystem.apply().and_then(|_| filesystem.verify()) {
        return Err(filesystem.abort(error));
    }
    if let Err(error) = database.commit() {
        return Err(filesystem.abort(error.into()));
    }
    report
        .warnings
        .extend(filesystem.finish_removal(empty_directories));
    report.removed_directories.retain(|path| !path.exists());
    Ok(report)
}

fn conflict(message: &str) -> Error {
    Error::decision("UNUSE_CONFLICT", message)
}
