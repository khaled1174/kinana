use crate::{
    DoctorCheck, DoctorReport, Result, agents, index, project_registry, status, vault::Vault,
};
use std::collections::BTreeSet;
use std::path::Path;

/// Diagnostics inspect only this vault and its locally registered projects.
pub fn check(vault: &Vault, home: &Path) -> Result<DoctorReport> {
    let agents = agents::detect(home);
    let config = vault.config();
    let records = vault.records();
    let index = index::check(&vault.index_path);
    let mut checks = vec![
        DoctorCheck {
            name: "git".into(),
            reason_code: "DOCTOR_GIT".into(),
            passed: crate::executables::git(&[]).is_ok(),
            message: "git must be installed to maintain vault history".into(),
        },
        DoctorCheck {
            name: "vault".into(),
            reason_code: "DOCTOR_VAULT".into(),
            passed: config.is_ok(),
            message: match config {
                Ok(_) => "box.toml is valid".into(),
                Err(e) => e.to_string(),
            },
        },
        DoctorCheck {
            name: "metadata".into(),
            reason_code: "DOCTOR_METADATA".into(),
            passed: records.is_ok(),
            message: match records {
                Ok(records) => format!("{} valid skill records", records.len()),
                Err(e) => e.to_string(),
            },
        },
        DoctorCheck {
            name: "index".into(),
            reason_code: "DOCTOR_INDEX".into(),
            passed: index.is_ok(),
            message: match index {
                Ok(()) => "SQLite index is healthy".into(),
                Err(e) => e.to_string(),
            },
        },
    ];
    checks.push(interrupted_change(vault));
    let mut warnings = vec![];
    let definitions = crate::bundles::list(vault);
    checks.push(DoctorCheck {
        name: "bundles".into(),
        reason_code: "DOCTOR_BUNDLES".into(),
        passed: definitions.is_ok(),
        message: match definitions {
            Ok(bundles) => format!("{} valid bundle definitions", bundles.len()),
            Err(error) => error.to_string(),
        },
    });
    let registrations = project_registry::inspect(vault);
    checks.push(DoctorCheck {
        name: "project_registry".into(),
        reason_code: "DOCTOR_PROJECT_REGISTRY".into(),
        passed: registrations.is_ok(),
        message: match &registrations {
            Ok(report) => format!(
                "{} registered locations for this vault; no home scan",
                report.projects.len()
            ),
            Err(error) => error.to_string(),
        },
    });
    if let Ok(registrations) = &registrations {
        for observation in &registrations.projects {
            if observation.status != "ok" {
                warnings.push(format!(
                    "{}: {} ({})",
                    observation.project.display(),
                    observation.status,
                    observation.message
                ));
            }
        }
        let cache = index::registered_projects_match(vault, &registrations.projects);
        checks.push(DoctorCheck {
            name: "project_index".into(),
            reason_code: "DOCTOR_PROJECT_INDEX".into(),
            passed: matches!(cache, Ok(true)),
            message: match cache {
                Ok(true) => "Project cache agrees with registered lock files".into(),
                Ok(false) => {
                    "Project cache differs from authoritative locks; use doctor --rebuild-index"
                        .into()
                }
                Err(error) => error.to_string(),
            },
        });
    }
    let projects = match status::status(vault, None) {
        Ok(report) => report.projects,
        Err(error) => {
            warnings.push(error.to_string());
            vec![]
        }
    };
    checks.push(DoctorCheck {
        name: "projects".into(),
        reason_code: "DOCTOR_PROJECTS".into(),
        passed: projects
            .iter()
            .all(|project| matches!(project.state.as_str(), "ok" | "no_lock")),
        message: format!(
            "{} registered projects inspected without following foreign links",
            projects.len()
        ),
    });
    let mut bad_targets = vec![];
    for project in &projects {
        warnings.extend(project.warnings.iter().cloned());
        for item in &project.items {
            if item.state != "current" {
                bad_targets.push(format!(
                    "{}: {} ({})",
                    item.target.display(),
                    item.state,
                    item.message
                ));
            }
        }
    }
    checks.push(DoctorCheck {
        name: "activation_targets".into(),
        reason_code: "DOCTOR_ACTIVATION_TARGETS".into(),
        passed: bad_targets.is_empty(),
        message: if bad_targets.is_empty() {
            "No broken, stale or modified recorded activation targets".into()
        } else {
            bad_targets.join("; ")
        },
    });
    Ok(DoctorReport {
        vault: vault.root.clone(),
        index: vault.index_path.clone(),
        agents,
        healthy: checks.iter().all(|c| c.passed),
        checks,
        projects,
        warnings,
        rebuild: None,
    })
}

pub fn run(vault: &Vault, home: &Path, rebuild_index: bool, dry_run: bool) -> Result<DoctorReport> {
    let rebuild = if rebuild_index {
        Some(index::rebuild(vault, dry_run)?)
    } else {
        None
    };
    let mut report = check(vault, home)?;
    if let Some(rebuild) = &rebuild {
        report.warnings.extend(rebuild.warnings.iter().cloned());
    }
    report.rebuild = rebuild;
    Ok(report)
}

/// Empty directories the last commit's metadata records for a skill that
/// differs from it, missing on disk now. Git does not record empty
/// directories; the metadata does, and a checkout alone would leave those
/// skills failing their hash. Only safe relative paths, the same ones undo
/// would restore, are returned; a path something else stands in is left to
/// the blocked paths.
pub(crate) fn missing_empty_directories(
    vault: &Vault,
    found: &crate::gitlog::Uncommitted,
) -> Vec<String> {
    let paths = found.all();
    let mut directories = Vec::new();
    let ids = paths
        .iter()
        .filter_map(|path| {
            path.strip_prefix("meta/")
                .and_then(|name| name.strip_suffix(".toml"))
                .or_else(|| {
                    path.strip_prefix("skills/")
                        .and_then(|rest| rest.split('/').next())
                })
        })
        .collect::<BTreeSet<_>>();
    for id in ids {
        if crate::vault::validate_id(id).is_err() {
            continue;
        }
        let Ok(Some(bytes)) = crate::gitlog::head_file(&vault.root, &format!("meta/{id}.toml"))
        else {
            continue;
        };
        let Some(metadata) = std::str::from_utf8(&bytes)
            .ok()
            .and_then(|text| toml::from_str::<crate::SkillMetadata>(text).ok())
        else {
            continue;
        };
        // The names come from the skill's author: only safe relative
        // paths, the same ones undo would restore, are named.
        for relative in metadata.empty_directories {
            let Some(text) = relative.to_str() else {
                continue;
            };
            if crate::layout::validate_relative(&relative).is_err() {
                continue;
            }
            // Something else in the way is named by the blocked
            // paths, not as a missing directory.
            let inside = format!("skills/{id}/{text}");
            if crate::gitlog::blocked_on_disk(&vault.root, &inside) {
                continue;
            }
            if !vault.root.join(&inside).is_dir() {
                directories.push(inside);
            }
        }
    }
    directories
}

/// Private stages (`.skillsbox-` directories and files) an interrupted
/// change left in the vault root, `dist/` or `.git/`, relative to the vault.
pub(crate) fn stages(vault: &Vault) -> Vec<String> {
    [
        vault.root.clone(),
        vault.root.join("dist"),
        vault.root.join(".git"),
    ]
    .iter()
    // Only a real directory is searched: a `dist` or `.git` that is a link
    // would list (and a repair would move) what lies outside the vault.
    .filter(|directory| std::fs::symlink_metadata(directory).is_ok_and(|m| m.is_dir()))
    .filter_map(|directory| std::fs::read_dir(directory).ok())
    .flatten()
    .filter_map(|entry| entry.ok().map(|entry| entry.path()))
    .filter(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(".skillsbox-") && name != ".skillsbox-build.toml")
    })
    .map(|path| {
        path.strip_prefix(&vault.root)
            .unwrap_or(&path)
            .display()
            .to_string()
    })
    .collect::<Vec<_>>()
}

/// The most paths one diagnosis names.
const SHOWN: usize = 10;

fn listed(paths: Vec<String>) -> String {
    let mut paths = paths
        .iter()
        .map(|path| crate::names::visible(path))
        .collect::<Vec<_>>();
    let more = paths.len().saturating_sub(SHOWN);
    paths.truncate(SHOWN);
    if more > 0 {
        paths.push(format!("and {more} more"));
    }
    paths.join(", ")
}

/// A change cut off between its first move and its cleanup (a killed
/// `remove`, a power loss) leaves at least one of: files that differ from the
/// last commit, private stages of `.skillsbox-` directories, or skills,
/// metadata and build output that no longer match. Doctor only reads and
/// names what it found, every name in one escape form; it prints no shell
/// command (owner decision 2026-11-17, sixth round): a printed line is read
/// again by the terminal and the shell before it reaches Git.
fn interrupted_change(vault: &Vault) -> DoctorCheck {
    let mut findings = Vec::new();
    let mut uncommitted = false;
    let mut recovery = None;
    match crate::gitlog::uncommitted(&vault.root) {
        Ok(None) => {}
        Ok(Some(found)) => {
            uncommitted = true;
            let paths = found.all();
            let mut directories = missing_empty_directories(vault, &found);
            findings.push(if paths.is_empty() {
                "tracked files differ from the last commit".to_owned()
            } else {
                format!("differs from the last commit: {}", listed(paths))
            });
            if !directories.is_empty() {
                findings.push(format!(
                    "empty directories the last commit's metadata records are missing: {}",
                    listed(std::mem::take(&mut directories))
                ));
            }
            recovery = Some(found);
        }
        Err(error) => findings.push(format!("Git state could not be read: {}", error.message)),
    }
    let stages = stages(vault);
    if !stages.is_empty() {
        findings.push(format!(
            "private stages of an interrupted change hold moved files: {}",
            listed(stages.clone())
        ));
    }
    let names = |directory: &str, files: bool| -> BTreeSet<String> {
        std::fs::read_dir(vault.root.join(directory))
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_type()
                    .is_ok_and(|kind| if files { kind.is_file() } else { kind.is_dir() })
            })
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| !name.starts_with('.'))
            .filter_map(|name| {
                if files {
                    name.strip_suffix(".toml").map(str::to_owned)
                } else {
                    Some(name)
                }
            })
            .collect()
    };
    let skills = names("skills", false);
    let meta = names("meta", true);
    let dist = names("dist", false);
    // A skill without generated output is a vault not built yet (a fresh
    // clone, for example), not an interruption: `build` covers it. Output
    // without its skill, or a skill without its metadata, is what a cut-off
    // removal or import leaves.
    let mut mismatched = Vec::new();
    for id in skills
        .union(&meta)
        .chain(dist.iter())
        .collect::<BTreeSet<_>>()
    {
        let missing = [
            (!skills.contains(id)).then_some("skills"),
            (!meta.contains(id)).then_some("meta"),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        if !missing.is_empty() {
            mismatched.push(format!("{id} (missing from {})", missing.join(", ")));
        }
    }
    if !mismatched.is_empty() {
        findings.push(format!(
            "skills, meta and dist do not match: {}",
            listed(mismatched)
        ));
    }
    DoctorCheck {
        name: "interrupted_change".into(),
        reason_code: "DOCTOR_INTERRUPTED_CHANGE".into(),
        passed: findings.is_empty(),
        message: if findings.is_empty() {
            "The vault matches its last commit and no change was left unfinished".into()
        } else {
            let mut message = format!(
                "A change may have been interrupted (or is still running): {}.",
                findings.join("; ")
            );
            if let Some(found) = recovery.as_ref() {
                if !found.restore.is_empty() || !found.unstage.is_empty() {
                    message.push_str(" If you did not make these changes yourself, kinana doctor --repair previews returning only these files to the last commit, not the whole vault, and backs up what it replaces before it does; then run kinana build.");
                }
                if !found.blocked.is_empty() {
                    message.push_str(&format!(
                        " Where the last commit has a file, these paths now hold a directory or another kind of node, or sit under one, and restoring them would delete it: {}. Move them out of the vault first, then run kinana doctor again.",
                        listed(found.blocked.clone())
                    ));
                }
                if !found.untracked.is_empty() || !found.unstage.is_empty() {
                    message.push_str(" Files the last commit does not have stay where they are; move them out of the vault yourself.");
                }
            } else if uncommitted {
                message.push_str(" Files the last commit does not have stay where they are; move them out of the vault yourself.");
            } else {
                message.push_str(" Run kinana build to regenerate dist.");
            }
            if !stages.is_empty() {
                message.push_str(" kinana doctor --repair moves the private stages into a backup folder once you confirm its preview.");
            }
            message
        },
    }
}
