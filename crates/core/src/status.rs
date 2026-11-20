//! Read-only project observations; lock files remain authoritative.
use crate::activation_transaction::Snapshot;
use crate::{
    ActivationStatus, Error, LockedSkill, ProjectStatus, Result, StatusReport, agents, hash, index,
    lockfile, project_registry, vault::Vault,
};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn source(vault: &Vault, skill: &LockedSkill) -> PathBuf {
    if skill.invocation == "manual" {
        vault.root.join("dist/.manual").join(&skill.id)
    } else {
        vault.root.join("dist").join(&skill.id)
    }
}

pub(crate) fn inspect(
    vault: &Vault,
    project: &Path,
    skill: &LockedSkill,
) -> Result<ActivationStatus> {
    let target = lockfile::target_path(project, skill)?;
    let observation = |state: &str, reason_code: &str, message: &str| ActivationStatus {
        id: skill.id.clone(),
        agent: skill.agent.clone(),
        target: target.clone(),
        state: state.into(),
        message: message.into(),
        reason_code: reason_code.into(),
    };
    let snapshot = match Snapshot::read(&target) {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => {
            return Ok(observation(
                "broken",
                "ACTIVATION_TARGET_MISSING",
                "Activation target is missing",
            ));
        }
        Err(_) => {
            return Ok(observation(
                "modified",
                "ACTIVATION_TARGET_UNSAFE",
                "Target is unsafe or contains unsupported files; preserved without following links",
            ));
        }
    };
    if skill
        .ownership
        .as_ref()
        .is_some_and(|identity| *identity != snapshot.ownership())
    {
        return Ok(observation(
            "modified",
            "ACTIVATION_TARGET_MODIFIED",
            "Target identity, permissions or contents changed",
        ));
    }
    let generated = source(vault, skill);
    if skill.mode == "link" && snapshot.link() != Some(generated.as_path()) {
        return Ok(observation(
            "modified",
            "ACTIVATION_LINK_REPLACED",
            "Link no longer points to the recorded generated skill",
        ));
    }
    if skill.mode == "copy" && snapshot.digest() != Some(skill.content_hash.as_str()) {
        return Ok(observation(
            "modified",
            "ACTIVATION_COPY_MODIFIED",
            "Copy differs from its recorded activation hash",
        ));
    }
    let manifest = generated.join("SKILL.md");
    let valid_manifest = index::validate_path(&manifest).and_then(|path| {
        if !fs::symlink_metadata(&path)?.is_file() {
            return Err(Error::new(
                "INVALID_FRONTMATTER",
                "Generated SKILL.md must be a regular file",
            ));
        }
        crate::frontmatter::read_skill(&path)
    });
    if valid_manifest.is_err() {
        return Ok(observation(
            "broken",
            "ACTIVATION_MANIFEST_INVALID",
            "Generated skill has a missing, invalid or unsafe SKILL.md",
        ));
    }
    match hash::hash_directory(&generated) {
        Ok(digest)
            if digest == skill.content_hash
                && skill.generation_hash.as_ref().is_none_or(|generation| {
                    crate::build::source_payload(&generated)
                        .is_ok_and(|payload| payload.generation_hash() == *generation)
                }) =>
        {
            Ok(observation(
                "current",
                if skill.ownership.is_some() {
                    "ACTIVATION_CURRENT"
                } else {
                    "ACTIVATION_LEGACY_UNOWNED"
                },
                if skill.ownership.is_some() {
                    "Target and generated skill match the lock"
                } else {
                    "Content matches; legacy lock has no identity proof for deletion"
                },
            ))
        }
        Ok(_) => Ok(observation(
            "stale",
            "ACTIVATION_SOURCE_STALE",
            "Generated skill changed since activation",
        )),
        Err(_) => Ok(observation(
            "broken",
            "ACTIVATION_SOURCE_MISSING",
            "Generated skill is missing or unsafe",
        )),
    }
}

pub fn status(vault: &Vault, project: Option<&Path>) -> Result<StatusReport> {
    let normalized = Vault::new(index::validate_path(&vault.root)?, vault.index_path.clone());
    let vault = &normalized;
    let root = index::validate_path(&vault.root)?;
    let mut projects = if let Some(project) = project {
        vec![project.to_path_buf()]
    } else {
        project_registry::read(vault)?
            .projects
            .into_iter()
            .filter(|entry| entry.vault == root)
            .map(|entry| entry.path)
            .collect()
    };
    projects.sort();
    projects.dedup();
    let projects = projects
        .into_iter()
        .map(|project| {
            let mut report = ProjectStatus {
                project: project.clone(),
                state: "ok".into(),
                items: vec![],
                warnings: vec![],
            };
            let observed = (|| -> Result<()> {
                let project = index::validate_path(&project)?;
                if !fs::symlink_metadata(&project)?.is_dir() {
                    return Err(Error::new(
                        "INVALID_PROJECT",
                        "Project is not a real directory",
                    ));
                }
                let Some(lock) = lockfile::read(&project)? else {
                    report.state = "no_lock".into();
                    return Ok(());
                };
                lockfile::validate_for_vault(&project, &root, &lock)?;
                for skill in &lock.active {
                    report.items.push(inspect(vault, &project, skill)?);
                }
                Ok(())
            })();
            if let Err(error) = observed {
                report.state = error.code;
                report.warnings.push(error.message);
            }
            report
        })
        .collect();
    Ok(StatusReport { projects })
}

pub(crate) fn validate_mutation_project(
    vault: &Vault,
    project: &Path,
    home: &Path,
) -> Result<PathBuf> {
    let project = index::validate_path(project)?;
    let root = index::validate_path(&vault.root)?;
    let home = index::validate_path(home)?;
    if !fs::symlink_metadata(&project)?.is_dir()
        || project.starts_with(&root)
        || project == home
        || project.starts_with(home.join(".cursor/skills"))
        || agents::SUPPORTED.iter().any(|agent| {
            project.starts_with(home.join(agents::project_skills(agent).expect("known agent")))
        })
    {
        return Err(Error::decision(
            "PROTECTED_PROJECT",
            "Choose a real project outside the vault and global skill directories",
        ));
    }
    for record in vault.records()? {
        if record
            .source
            .found_in
            .iter()
            .any(|source| project.starts_with(source))
        {
            return Err(Error::decision(
                "PROTECTED_PROJECT",
                "Cannot modify an imported source tree",
            ));
        }
    }
    Ok(project)
}
