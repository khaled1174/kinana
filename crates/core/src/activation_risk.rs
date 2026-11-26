//! Audit evidence used to approve an exact project activation.
use crate::{
    Audit, AuditFinding, Error, Result, audit, build::ActivationPayload, edit, vault::Vault,
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone, Serialize)]
pub struct Risk {
    pub id: String,
    pub role: &'static str,
    pub audit_level: &'static str,
    pub rules_version: Option<u32>,
    pub details: Vec<AuditFinding>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Decision {
    requires_confirmation: bool,
    risky: Vec<Risk>,
}

pub(crate) fn decision(risks: &[Risk]) -> Error {
    let risky = risks
        .iter()
        .cloned()
        .map(|mut risk| {
            // The CLI and its router receive structured locations only.
            // Reconstruct labels because saved audit strings are untrusted.
            risk.details.retain_mut(|finding| {
                finding.excerpt = None;
                finding.forms.clear();
                let valid_path = !finding.path.as_os_str().is_empty()
                    && finding.path.components().all(|part| {
                        matches!(part, Component::Normal(name) if name.to_str().is_some_and(|value|
                            !value.chars().any(|c| c.is_control() || crate::names::hidden_or_reordering(c))))
                    });
                let standard = matches!(
                    finding.rule.as_str(),
                    "network" | "secrets" | "destructive" | "download_execution"
                        | "instruction_override" | "executable" | "symlink" | "hooks"
                        | "package_execution"
                ) && finding.reason_code == format!("AUDIT_{}_STRONG", finding.rule.to_ascii_uppercase());
                valid_path && (standard || (finding.rule == "destructive"
                    && finding.reason_code == "AUDIT_RECURSIVE_DELETE_STRONG"))
            });
            for finding in &mut risk.details {
                finding.strength = "strong".into();
                finding.level = "risky".into();
            }
            risk
        })
        .collect();
    let mut error = Error::decision(
        "RISKY_ACTIVATION_REQUIRES_CONFIRMATION",
        "Open Kinana to approve the risky activation. افتح كنانة للموافقة",
    );
    error.activation_preview = Some(Box::new(Decision {
        requires_confirmation: true,
        risky,
    }));
    error
}

fn verified(audit: &Audit) -> bool {
    audit.rules_version > 0 && matches!(audit.level.as_str(), "clean" | "review" | "risky")
}

pub(crate) fn for_payload(
    id: &str,
    role: &'static str,
    stored: &Audit,
    payload: &ActivationPayload,
) -> Option<Risk> {
    let current = audit::audit_payload(&payload.entries);
    if verified(stored) && stored.level != "risky" && current.level != "risky" {
        return None;
    }
    let rules_version = match (stored.level.as_str(), current.level.as_str()) {
        (_, "risky") => Some(current.rules_version),
        ("risky", _) if verified(stored) => Some(stored.rules_version),
        _ => None,
    };
    let mut details = if stored.level == "risky" && current.level != "risky" {
        // When audit evidence is current, derive its excerpts from the
        // prepared bytes rather than trusting text inserted in metadata.
        stored
            .details
            .iter()
            .filter(|d| d.level == "risky")
            .cloned()
            .map(|mut d| {
                d.excerpt = None;
                d.forms.clear();
                d
            })
            .collect::<Vec<_>>()
    } else {
        vec![]
    };
    for finding in current.details.into_iter().filter(|d| d.level == "risky") {
        details.retain(|old| {
            (old.rule.as_str(), &old.path, old.line)
                != (finding.rule.as_str(), &finding.path, finding.line)
        });
        details.push(finding);
    }
    Some(Risk {
        id: id.into(),
        role,
        audit_level: "risky",
        rules_version,
        details,
    })
}

/// Read-only catalog view. Failure to verify a skill is reported as risk.
pub(crate) fn level(vault: &Vault, id: &str) -> &'static str {
    let Ok(meta) = edit::read_metadata(vault, id) else {
        return "risky";
    };
    if !verified(&meta.audit) || meta.audit.level == "risky" {
        return "risky";
    }
    let Ok(current) = audit::audit_skill(&vault.root.join("skills").join(id)) else {
        return "risky";
    };
    if current.level == "risky" {
        return "risky";
    }
    if meta.description.active == "improved" {
        let Ok(payload) =
            crate::build::described_source_payload(&vault.root.join("skills").join(id), &meta)
        else {
            return "risky";
        };
        if audit::audit_payload(&payload.entries).level == "risky" {
            return "risky";
        }
    }
    if current.level == "review" || meta.audit.level == "review" {
        "review"
    } else {
        "clean"
    }
}

pub(crate) fn levels(
    vault: &Vault,
    ids: impl IntoIterator<Item = String>,
) -> BTreeMap<String, &'static str> {
    let mut levels = BTreeMap::new();
    for id in ids {
        levels
            .entry(id.clone())
            .or_insert_with(|| level(vault, &id));
    }
    levels
}

const AGENT_ROOTS: &[&str] = &[
    ".agents/skills",
    ".claude/skills",
    ".codex/skills",
    ".cursor/skills",
    ".pi/skills",
];

fn unsafe_staging() -> Error {
    Error::decision(
        "RISKY_STAGE_EXPOSED",
        "Risky skills can only be built outside agent skill folders and existing activation links",
    )
}

fn contains(candidate: &Path, target: &Path) -> bool {
    // Default macOS filesystems are case-insensitive. Conservatively apply
    // the same comparison on other filesystems before publishing risk.
    let candidate = candidate
        .components()
        .map(|p| p.as_os_str().to_string_lossy().to_lowercase())
        .collect::<Vec<_>>();
    let target = target
        .components()
        .map(|p| p.as_os_str().to_string_lossy().to_lowercase())
        .collect::<Vec<_>>();
    target.starts_with(&candidate) || candidate.starts_with(&target)
}

pub(crate) fn require_private_stage(
    vault: &Vault,
    target: &Path,
    home: Option<&Path>,
    project: Option<&Path>,
) -> Result<()> {
    let target = crate::index::validate_path(target)?;
    if target
        .components()
        .collect::<Vec<_>>()
        .windows(2)
        .any(|parts| {
            let host = parts[0].as_os_str().to_string_lossy().to_lowercase();
            parts[1].as_os_str().eq_ignore_ascii_case("skills")
                && matches!(
                    host.as_str(),
                    ".agents" | ".claude" | ".codex" | ".cursor" | ".pi" | "codex"
                )
        })
    {
        return Err(unsafe_staging());
    }
    let mut roots = vec![PathBuf::from("/etc/codex/skills")];
    if let Some(home) = home {
        roots.extend(AGENT_ROOTS.iter().map(|name| home.join(name)));
    }
    for path in crate::project_registry::read(vault)?
        .projects
        .into_iter()
        .map(|p| p.path)
        .chain(project.map(Path::to_path_buf))
    {
        roots.extend(AGENT_ROOTS.iter().map(|name| path.join(name)));
    }
    roots.sort();
    roots.dedup();
    for root in roots {
        let resolved = canonical_with_missing(&root).ok_or_else(unsafe_staging)?;
        if contains(&root, &target) || contains(&resolved, &target) {
            return Err(unsafe_staging());
        }
        if !crate::state::exists(&resolved)? {
            continue;
        }
        // Follow directory aliases to discover nested links. Files are examined
        // as directory entries only. An alias whose target cannot be checked
        // prevents preparation.
        for entry in walkdir::WalkDir::new(&resolved).follow_links(true) {
            match entry {
                Ok(entry) if entry.path_is_symlink() => check_stage_alias(entry.path(), &target)?,
                Ok(_) => {}
                Err(error)
                    if error
                        .io_error()
                        .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
                {
                    check_stage_alias(error.path().ok_or_else(unsafe_staging)?, &target)?;
                }
                Err(_) => return Err(unsafe_staging()),
            }
        }
    }
    Ok(())
}

fn check_stage_alias(path: &Path, target: &Path) -> Result<()> {
    let destination = fs::read_link(path).map_err(|_| unsafe_staging())?;
    let absolute = path.parent().ok_or_else(unsafe_staging)?.join(destination);
    let resolved = canonical_with_missing(&absolute).ok_or_else(unsafe_staging)?;
    if contains(&resolved, target) {
        return Err(unsafe_staging());
    }
    Ok(())
}

pub(crate) fn guard_cache_writes(
    vault: &Vault,
    writes: &[crate::activation_transaction::PlannedWrite],
    stored_risky: bool,
) -> Result<Vec<PathBuf>> {
    let mut guarded = Vec::new();
    let dist = vault.root.join("dist");
    for write in writes {
        if write.target.starts_with(&dist)
            && let crate::activation_transaction::Data::Tree(payload) = &write.data
            && (stored_risky || audit::audit_payload(&payload.entries).level == "risky")
        {
            guarded.push(write.target.clone());
        }
    }
    recheck_cache_writes(vault, &guarded)?;
    Ok(guarded)
}

pub(crate) fn recheck_cache_writes(vault: &Vault, guarded: &[PathBuf]) -> Result<()> {
    let home = local_home();
    for path in guarded {
        require_private_stage(vault, path, home.as_deref(), None)?;
    }
    Ok(())
}

pub(crate) fn check_cache_stage(
    vault: &Vault,
    guarded: &[PathBuf],
    target: &Path,
    stage: &Path,
) -> Result<()> {
    if guarded.iter().any(|path| path == target) {
        require_private_stage(vault, stage, local_home().as_deref(), None)?;
    }
    Ok(())
}

pub(crate) fn local_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn canonical_with_missing(path: &Path) -> Option<PathBuf> {
    let mut parent = path;
    let mut suffix = Vec::new();
    loop {
        match parent.canonicalize() {
            Ok(mut resolved) => {
                resolved.extend(suffix.into_iter().rev());
                return Some(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match fs::symlink_metadata(parent) {
                    Ok(_) => return None,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return None,
                }
                suffix.push(parent.file_name()?);
                parent = parent.parent()?;
            }
            Err(_) => return None,
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        os::unix::fs::symlink,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn risky_staging_rejects_nested_roots_and_direct_or_parent_aliases() {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "kinana-risk-stage-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let home = root.join("home");
        let project = root.join("project");
        let state = root.join("state/index.db");
        let vault = Vault::new(root.join("vault"), state.clone());
        let real_stage = vault.root.join("dist/danger");
        for name in AGENT_ROOTS {
            let nested = Vault::new(project.join(name).join("sub/vault"), state.clone());
            let stage = nested.root.join("dist/danger");
            assert_eq!(
                require_private_stage(&nested, &stage, Some(&home), Some(&project))
                    .unwrap_err()
                    .code,
                "RISKY_STAGE_EXPOSED"
            );
            assert!(!stage.exists());
        }
        fs::create_dir_all(home.join(".agents/skills/nested")).unwrap();
        fs::create_dir_all(&vault.root).unwrap();
        assert!(require_private_stage(&vault, &real_stage, Some(&home), Some(&project)).is_ok());
        let alias = home.join(".agents/skills/nested/alias");
        symlink(&real_stage, &alias).unwrap();
        assert_eq!(
            require_private_stage(&vault, &real_stage, Some(&home), Some(&project))
                .unwrap_err()
                .code,
            "RISKY_STAGE_EXPOSED"
        );
        fs::remove_file(alias).unwrap();
        let external = root.join("external");
        fs::create_dir(&external).unwrap();
        symlink(&real_stage, external.join("downstream")).unwrap();
        symlink(&external, home.join(".agents/skills/nested/forward")).unwrap();
        assert!(require_private_stage(&vault, &real_stage, Some(&home), Some(&project)).is_err());
        fs::remove_file(external.join("downstream")).unwrap();
        symlink(vault.root.join("dist"), external.join("missing-parent")).unwrap();
        symlink(
            external.join("missing-parent/danger"),
            external.join("through-parent"),
        )
        .unwrap();
        assert!(require_private_stage(&vault, &real_stage, Some(&home), Some(&project)).is_err());
        fs::remove_dir_all(external).unwrap();
        fs::remove_file(home.join(".agents/skills/nested/forward")).unwrap();
        fs::create_dir_all(vault.root.join("skills")).unwrap();
        symlink(&vault.root, home.join(".claude")).unwrap();
        let aliased_stage = vault.root.join("skills/dist/id");
        assert_eq!(
            require_private_stage(&vault, &aliased_stage, Some(&home), Some(&project))
                .unwrap_err()
                .code,
            "RISKY_STAGE_EXPOSED"
        );
        assert!(!real_stage.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
