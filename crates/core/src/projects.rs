//! Read-only project presentation and confirmed operations shared by desktop callers.
//! Existing activation/adoption/unuse planners remain the only filesystem writers.
use crate::{
    Error, Result, activate, activation_transaction::Snapshot, adopt, index, lockfile,
    project_registry, scan, status, unuse, vault::Vault,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU8, Ordering},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "lowercase", deny_unknown_fields)]
pub enum Request {
    Use {
        project: PathBuf,
        selections: Vec<String>,
        agent: String,
        mode: String,
        #[serde(default)]
        include_secondary: Vec<String>,
    },
    Unuse {
        project: PathBuf,
        ids: Vec<String>,
        agent: String,
    },
    Adopt {
        project: PathBuf,
        agent: String,
    },
}
impl Request {
    pub fn project(&self) -> &Path {
        match self {
            Self::Use { project, .. }
            | Self::Unuse { project, .. }
            | Self::Adopt { project, .. } => project,
        }
    }
}
#[derive(Debug, Serialize)]
pub struct Project {
    pub path: PathBuf,
    pub name: String,
    pub state: String,
    pub bundle: Option<String>,
    pub activations: Vec<Activation>,
    pub local: Vec<Local>,
    pub warnings: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct Activation {
    pub id: String,
    pub name: String,
    pub agent: String,
    pub mode: String,
    pub invocation: String,
    pub target: PathBuf,
    pub state: String,
    pub message: String,
    pub reason_code: String,
}
#[derive(Debug, Serialize)]
pub struct Local {
    pub agent: String,
    pub entry: crate::ScanEntry,
}
/// Explicitly selected roots plus the local registry only; never scan the user's home.
pub fn list(vault: &Vault, selected: &[PathBuf]) -> Result<Vec<Project>> {
    let root = index::validate_path(&vault.root)?;
    let mut paths = project_registry::read(vault)?
        .projects
        .into_iter()
        .filter(|p| p.vault == root)
        .map(|p| p.path)
        .collect::<BTreeSet<_>>();
    for path in selected {
        paths.insert(index::validate_path(path)?);
    }
    let records = vault.records()?;
    paths
        .into_iter()
        .map(|path| {
            let observation = status::status(vault, Some(&path))?.projects.remove(0);
            let mut project = Project {
                path: path.clone(),
                name: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                state: observation.state,
                bundle: None,
                activations: vec![],
                local: vec![],
                warnings: observation.warnings,
            };
            if !matches!(project.state.as_str(), "ok" | "no_lock") {
                return Ok(project);
            }
            if let Some(lock) = lockfile::read(&path)? {
                project.bundle = lock.bundle;
                for observed in observation.items {
                    if let Some(skill) = lock
                        .active
                        .iter()
                        .find(|s| s.id == observed.id && s.agent == observed.agent)
                    {
                        project.activations.push(Activation {
                            id: skill.id.clone(),
                            name: skill.name.clone(),
                            agent: skill.agent.clone(),
                            mode: skill.mode.clone(),
                            invocation: skill.invocation.clone(),
                            target: observed.target,
                            state: observed.state,
                            message: observed.message,
                            reason_code: observed.reason_code,
                        });
                    }
                }
            }
            for (directory, agent) in [
                (".claude/skills", "claude"),
                (".agents/skills", "codex"),
                (".codex/skills", "codex"),
                (".cursor/skills", "cursor"),
            ] {
                let parent = path.join(directory);
                if !parent.try_exists()? {
                    continue;
                }
                let children = (|| -> Result<Vec<PathBuf>> {
                    let parent = index::validate_path(&parent)?;
                    let mut paths = fs::read_dir(parent)?
                        .map(|e| e.map(|e| e.path()))
                        .collect::<std::io::Result<Vec<_>>>()?;
                    paths.sort();
                    Ok(paths)
                })();
                let children = match children {
                    Ok(paths) => paths,
                    Err(error) => {
                        project.warnings.push(error.to_string());
                        continue;
                    }
                };
                for child in children {
                    // Managed targets are already inspected through the lock, including broken links.
                    if project.activations.iter().any(|a| a.target == child) {
                        continue;
                    }
                    let metadata = fs::symlink_metadata(&child)?;
                    if metadata.file_type().is_symlink() {
                        project.warnings.push(format!(
                            "Unmanaged symbolic link preserved without following it: {}",
                            child.display()
                        ));
                        continue;
                    }
                    if !metadata.is_dir() {
                        continue;
                    }
                    match scan::scan(&[child], &records) {
                        Ok(report) => {
                            project.warnings.extend(report.warnings);
                            project
                                .local
                                .extend(report.entries.into_iter().map(|entry| Local {
                                    agent: agent.into(),
                                    entry,
                                }));
                        }
                        Err(error) => project.warnings.push(error.to_string()),
                    }
                }
            }
            Ok(project)
        })
        .collect()
}
#[derive(Debug, Serialize)]
pub struct Change {
    pub target: PathBuf,
    pub source: Option<PathBuf>,
    pub id: Option<String>,
    pub agent: Option<String>,
    pub mode: Option<String>,
    pub invocation: Option<String>,
    pub changed: bool,
}
#[derive(Debug, Serialize)]
pub struct Preview {
    pub request: Request,
    pub token: String,
    pub changes: Vec<Change>,
    pub candidates: Vec<crate::AdoptionCandidate>,
    pub risky: Vec<crate::activation_risk::Risk>,
    pub warnings: Vec<String>,
    pub gitignore_suggestions: Vec<String>,
    pub changed: bool,
}
fn use_options(request: &Request, home: &Path, dry_run: bool) -> crate::ActivateOptions {
    let Request::Use {
        project,
        selections,
        agent,
        mode,
        include_secondary,
    } = request
    else {
        unreachable!()
    };
    crate::ActivateOptions {
        project: project.clone(),
        home: home.into(),
        selections: selections.clone(),
        agent: agent.clone(),
        mode: mode.clone(),
        include_secondary: include_secondary.clone(),
        dry_run,
    }
}
fn unuse_options(request: &Request, dry_run: bool) -> crate::UnuseOptions {
    let Request::Unuse {
        project,
        ids,
        agent,
    } = request
    else {
        unreachable!()
    };
    crate::UnuseOptions {
        project: project.clone(),
        ids: ids.clone(),
        agent: if agent == "all" {
            None
        } else {
            Some(agent.clone())
        },
        dry_run,
    }
}
fn adopt_options(request: &Request, home: &Path, dry_run: bool) -> crate::AdoptOptions {
    let Request::Adopt { project, agent } = request else {
        unreachable!()
    };
    crate::AdoptOptions {
        project: project.clone(),
        home: home.into(),
        agent: agent.clone(),
        dry_run,
        confirmed: !dry_run,
    }
}
fn changes(report: &crate::ActivationReport) -> Vec<Change> {
    report
        .items
        .iter()
        .map(|i| Change {
            target: i.target.clone(),
            source: Some(i.source.clone()),
            id: Some(i.id.clone()),
            agent: Some(i.agent.clone()),
            mode: Some(i.mode.clone()),
            invocation: Some(i.invocation.clone()),
            changed: i.changed,
        })
        .collect()
}
pub fn preview(vault: &Vault, home: &Path, request: &Request) -> Result<Preview> {
    build_preview(vault, home, request).map(|(preview, _)| preview)
}

fn build_preview(
    vault: &Vault,
    home: &Path,
    request: &Request,
) -> Result<(Preview, Option<activate::Plan>)> {
    status::validate_mutation_project(vault, request.project(), home)?;
    let mut plan = Preview {
        request: request.clone(),
        token: String::new(),
        changes: vec![],
        candidates: vec![],
        risky: vec![],
        warnings: vec![],
        gitignore_suggestions: vec![],
        changed: false,
    };
    let mut activation = None;
    let raw = match request {
        Request::Use { .. } => {
            let prepared = activate::prepare(vault, &use_options(request, home, true))?;
            let r = &prepared.report;
            plan.changes = changes(r);
            plan.changed = r.changed;
            plan.warnings = r.warnings.clone();
            plan.gitignore_suggestions = r.gitignore_suggestions.clone();
            let raw = serde_json::to_value(r);
            activation = Some(prepared);
            raw
        }
        Request::Unuse { .. } => {
            let r = unuse::unuse(vault, &unuse_options(request, true))?;
            let mut paths = r.removed.clone();
            paths.extend(r.removed_directories.clone());
            if r.lock_removed {
                paths.push(r.project.join(".kinana.lock"));
            }
            plan.changes = paths
                .into_iter()
                .map(|target| Change {
                    target,
                    source: None,
                    id: None,
                    agent: None,
                    mode: None,
                    invocation: None,
                    changed: true,
                })
                .collect();
            plan.changed = r.changed;
            plan.warnings = r.warnings.clone();
            serde_json::to_value(r)
        }
        Request::Adopt { .. } => {
            let (r, prepared) = adopt::prepare(vault, &adopt_options(request, home, true))?;
            activation = prepared;
            if let Some(activation) = &r.activation {
                plan.changes = changes(activation);
                plan.changed = activation.changed;
                plan.warnings = activation.warnings.clone();
                plan.gitignore_suggestions = activation.gitignore_suggestions.clone();
            }
            plan.candidates = r.candidates.clone();
            serde_json::to_value(r)
        }
    }
    .map_err(|e| Error::new("JSON_ERROR", e.to_string()))?;
    // Bind confirmation to plan content AND observed node identities, not paths alone.
    let mut observed = Vec::new();
    let mut targets = plan
        .changes
        .iter()
        .map(|c| c.target.clone())
        .collect::<BTreeSet<_>>();
    targets.insert(request.project().join(".kinana.lock"));
    for path in targets {
        // Removal of a recorded parent directory uses a directory observation rather than
        // recursively following managed skill symlinks inside it.
        let stamp = if fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir()) {
            crate::activation_transaction::directory_identity(&path)?
        } else {
            Snapshot::read(&path)?.map(|s| s.ownership())
        };
        observed.push((path, stamp));
    }
    let fingerprint = if let Some(prepared) = &activation {
        prepared.validate_staging(vault)?;
        plan.risky = prepared.risky.clone();
        Some(prepared.fingerprint(request)?)
    } else {
        None
    };
    let bytes = serde_json::to_vec(&(request, raw, observed, fingerprint))
        .map_err(|e| Error::new("JSON_ERROR", e.to_string()))?;
    plan.token = blake3::hash(&bytes).to_hex().to_string();
    Ok((plan, activation))
}
/// Cancellation is accepted through read-only planning. Publication is atomic and cannot
/// be interrupted after this boundary; the existing transaction owns rollback on failure.
#[derive(Default)]
pub struct Control(AtomicU8);
impl Control {
    pub fn cancel(&self) -> bool {
        self.0
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
            || self.0.load(Ordering::SeqCst) == 1
    }
    pub fn is_committing(&self) -> bool {
        self.0.load(Ordering::SeqCst) == 2
    }
    fn commit(&self) -> Result<()> {
        self.0
            .compare_exchange(0, 2, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| ())
            .map_err(|_| {
                Error::new(
                    "PROJECT_CANCELLED",
                    "Project operation cancelled before publication",
                )
            })
    }
}
pub fn apply(
    vault: &Vault,
    home: &Path,
    request: &Request,
    token: &str,
    control: &Control,
) -> Result<Preview> {
    apply_desktop(vault, home, request, token, false, control)
}

/// The desktop invokes this after the user acknowledges the named risks.
/// CLI activation never calls this path or accepts its preview token.
pub fn apply_desktop(
    vault: &Vault,
    home: &Path,
    request: &Request,
    token: &str,
    acknowledge_risk: bool,
    control: &Control,
) -> Result<Preview> {
    if token.is_empty() {
        return Err(Error::decision(
            "PROJECT_CONFIRMATION_REQUIRED",
            "Preview and explicitly confirm the project changes first",
        ));
    }
    let (mut plan, activation) = build_preview(vault, home, request)?;
    if token != plan.token {
        return Err(Error::decision(
            "PROJECT_PREVIEW_CHANGED",
            "Project or skill changed since preview; review a new preview",
        ));
    }
    if !plan.risky.is_empty() && !acknowledge_risk {
        return Err(crate::activation_risk::decision(&plan.risky));
    }
    control.commit()?;
    let warnings = match (request, activation) {
        (_, Some(prepared)) => activate::execute(vault, prepared, false)?.warnings,
        (Request::Unuse { .. }, None) => {
            unuse::unuse(vault, &unuse_options(request, false))?.warnings
        }
        (_, None) => Vec::new(),
    };
    plan.warnings.extend(warnings);
    plan.warnings.sort();
    plan.warnings.dedup();
    Ok(plan)
}
