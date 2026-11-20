//! Explicit adoption of ordinary project directories matching immutable sources.
use crate::activation_transaction::Snapshot;
use crate::{
    ActivateOptions, AdoptOptions, AdoptReport, AdoptionCandidate, Error, Result, activate, agents,
    lockfile, status, vault::Vault,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

pub fn adopt(vault: &Vault, options: &AdoptOptions) -> Result<AdoptReport> {
    let _state_guard = crate::state::write_guard(&vault.index_path, options.dry_run)?;
    let normalized = Vault::new(
        crate::index::validate_path(&vault.root)?,
        crate::index::validate_path(&vault.index_path)?,
    );
    let vault = &normalized;
    if !matches!(
        options.agent.as_str(),
        "claude" | "codex" | "cursor" | "all"
    ) {
        return Err(Error::new(
            "INVALID_ARGUMENTS",
            "Choose claude/codex/cursor/all",
        ));
    }
    let project = status::validate_mutation_project(vault, &options.project, &options.home)?;
    let old_lock = lockfile::read(&project)?;
    if let Some(lock) = &old_lock {
        lockfile::validate_for_vault(&project, &vault.root, lock)?;
    }
    let records = vault.records()?;
    let mut report = AdoptReport {
        dry_run: options.dry_run,
        candidates: vec![],
        activation: None,
    };
    let mut targets = BTreeMap::new();
    let mut pairs = BTreeSet::new();
    let mut ids = BTreeSet::new();
    let agents = if options.agent == "all" {
        agents::SUPPORTED.to_vec()
    } else {
        vec![options.agent.as_str()]
    };
    for agent in agents {
        let parent = crate::index::validate_path(
            &project.join(agents::project_skills(agent).expect("validated agent")),
        )?;
        if !parent.try_exists()? {
            continue;
        }
        let mut children = fs::read_dir(parent)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        children.sort();
        for target in children {
            let mut candidate = AdoptionCandidate {
                target: target.clone(),
                agent: agent.into(),
                id: None,
                content_hash: None,
                link_to: None,
                eligible: false,
                reason_code: "ADOPT_NOT_IDENTICAL".into(),
                reason: "Only ordinary directories matching an original skill are eligible".into(),
            };
            if old_lock.as_ref().is_some_and(|lock| {
                lock.active.iter().any(|skill| {
                    lockfile::target_path(&project, skill).ok().as_ref() == Some(&target)
                })
            }) {
                candidate.reason_code = "ADOPT_ALREADY_MANAGED".into();
                candidate.reason = "Already recorded in the project lock; preserved".into();
            } else if fs::symlink_metadata(&target)?.is_dir() {
                match Snapshot::read(&target) {
                    Ok(Some(snapshot)) => {
                        candidate.content_hash = snapshot.digest().map(str::to_owned);
                        let matching = records
                            .iter()
                            .filter(|record| {
                                Some(record.content_hash.as_str()) == snapshot.digest()
                                    && target
                                        .file_name()
                                        .is_some_and(|name| name == record.name.as_str())
                            })
                            .collect::<Vec<_>>();
                        if matching.len() == 1 {
                            let id = matching[0].id.clone();
                            candidate.id = Some(id.clone());
                            candidate.eligible = true;
                            candidate.reason_code = "ADOPT_IDENTICAL".into();
                            candidate.reason="Byte-identical to an immutable original; explicit confirmation required".into();
                            ids.insert(id.clone());
                            pairs.insert((id, agent.into()));
                            targets.insert(target.clone(), snapshot);
                        } else if matching.len() > 1 {
                            candidate.reason_code = "ADOPT_AMBIGUOUS".into();
                            candidate.reason = "Ambiguous original match; preserved".into();
                        }
                    }
                    Err(_) => {
                        candidate.reason_code = "ADOPT_UNSAFE".into();
                        candidate.reason="Contains links, special files or unreadable data; preserved without following them".into();
                    }
                    Ok(None) => {
                        candidate.reason_code = "ADOPT_DISAPPEARED".into();
                        candidate.reason = "Candidate disappeared; preserved".into();
                    }
                }
            }
            // Codex and Cursor can request the same physical candidate. Keep
            // both logical activation pairs, but present that directory once.
            if !report
                .candidates
                .iter()
                .any(|entry| entry.target == candidate.target)
            {
                report.candidates.push(candidate);
            }
        }
    }
    if ids.is_empty() {
        return Ok(report);
    }
    let mut activation_options = ActivateOptions {
        project,
        home: options.home.clone(),
        selections: ids.into_iter().collect(),
        agent: options.agent.clone(),
        mode: "link".into(),
        include_secondary: vec![],
        dry_run: true,
    };
    let preview = activate::adopt_validated(vault, &activation_options, &targets, &pairs)?;
    for candidate in &mut report.candidates {
        if let Some(item) = preview
            .items
            .iter()
            .find(|item| item.target == candidate.target)
        {
            candidate.link_to = Some(item.source.clone());
        }
    }
    if options.dry_run {
        report.activation = Some(preview);
        return Ok(report);
    }
    if !options.confirmed {
        return Err(Error::decision(
            "ADOPT_CONFIRMATION_REQUIRED",
            format!(
                "Review adopt --dry-run, then confirm with --yes. Candidates: {}",
                serde_json::to_string(&report.candidates)
                    .map_err(|error| Error::new("JSON_ERROR", error.to_string()))?
            ),
        ));
    }
    activation_options.dry_run = false;
    report.activation = Some(activate::adopt_validated(
        vault,
        &activation_options,
        &targets,
        &pairs,
    )?);
    Ok(report)
}
