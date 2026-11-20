use crate::{AgentInfo, Error, Result};
use std::path::{Path, PathBuf};

pub const SUPPORTED: &[&str] = &["claude", "codex", "cursor"];

#[derive(Debug, serde::Serialize)]
pub struct AgentSkillLocation {
    pub path: PathBuf,
    pub agents: Vec<String>,
    pub status: String,
    pub reason_code: String,
}
#[derive(Debug, serde::Serialize)]
pub struct AgentSkillLocations {
    pub roots: Vec<PathBuf>,
    pub locations: Vec<AgentSkillLocation>,
}
/// Discover only the selected agents' known user skill directories. Do not
/// create state, start an agent, search the whole home, or follow links.
pub fn skill_locations(home: &Path, agents: &[String]) -> Result<AgentSkillLocations> {
    use std::collections::{BTreeMap, BTreeSet};
    if agents.is_empty() {
        return Err(Error::new("NO_SCAN_AGENTS", "Select at least one agent"));
    }
    if agents
        .iter()
        .any(|agent| !SUPPORTED.contains(&agent.as_str()))
    {
        return Err(Error::new(
            "INVALID_AGENT",
            "Unsupported skill source agent",
        ));
    }
    let mut paths: BTreeMap<PathBuf, BTreeSet<String>> = BTreeMap::new();
    for agent in agents {
        let directories: &[&str] = match agent.as_str() {
            "claude" => &[".claude/skills"],
            "codex" => &[".agents/skills", ".codex/skills"],
            "cursor" => &[
                ".agents/skills",
                ".cursor/skills",
                ".claude/skills",
                ".codex/skills",
            ],
            _ => unreachable!(),
        };
        for directory in directories {
            paths
                .entry(home.join(directory))
                .or_default()
                .insert(agent.clone());
        }
    }
    let mut report = AgentSkillLocations {
        roots: vec![],
        locations: vec![],
    };
    for (path, agents) in paths {
        let inspected =
            crate::index::validate_path(&path).and_then(|checked| match std::fs::symlink_metadata(
                &checked,
            ) {
                Ok(metadata) if metadata.is_dir() => Ok(true),
                Ok(_) => Err(Error::new(
                    "SKILLS_LOCATION_NOT_DIRECTORY",
                    "Skill location is not a directory",
                )),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(error) => Err(error.into()),
            });
        let (status, reason_code) = match inspected {
            Ok(true) => {
                report.roots.push(path.clone());
                ("ready", "SKILLS_LOCATION_READY".into())
            }
            Ok(false) => ("missing", "SKILLS_LOCATION_MISSING".into()),
            Err(error) => ("blocked", error.reason_code),
        };
        report.locations.push(AgentSkillLocation {
            path,
            agents: agents.into_iter().collect(),
            status: status.into(),
            reason_code,
        });
    }
    Ok(report)
}

pub fn project_skills(agent: &str) -> Option<&'static str> {
    match agent {
        "claude" => Some(".claude/skills"),
        "codex" | "cursor" => Some(".agents/skills"),
        _ => None,
    }
}

/// Validate agent-specific invocation capabilities without configuring an agent.
pub(crate) fn validate_invocation(agent: &str, invocation: &str) -> Result<()> {
    if agent == "codex" && invocation == "manual" {
        return Err(Error::decision(
            "CODEX_MANUAL_UNVERIFIED",
            "Manual Codex activation requires verification of its invocation controls",
        ));
    }
    if agent == "cursor" && invocation == "manual" {
        return Err(Error::decision(
            "CURSOR_MANUAL_UNVERIFIED",
            "Manual Cursor activation requires a verified invocation-controls experiment",
        ));
    }
    Ok(())
}

pub fn detect(home: &Path) -> Vec<AgentInfo> {
    SUPPORTED
        .iter()
        .map(|&name| AgentInfo {
            name: name.into(),
            // Cursor support is for the editor. Its standalone cursor-agent CLI
            // and a generic `agent` command do not establish an editor install.
            available: if name == "cursor" {
                executable_in_path("cursor").is_some() || cursor_app_available(home)
            } else {
                executable_in_path(name).is_some()
            },
            project_skills: project_skills(name).expect("known agent").into(),
            global_skills: home.join(project_skills(name).expect("known agent")),
        })
        .collect()
}

fn cursor_app_available(home: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        cursor_app_at(Path::new("/Applications")) || cursor_app_at(&home.join("Applications"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = home;
        false
    }
}
#[cfg(target_os = "macos")]
fn cursor_app_at(applications: &Path) -> bool {
    let bundle = applications.join("Cursor.app/Contents");
    // Read metadata only; discovery must never start the editor or its installer.
    is_executable(&bundle.join("MacOS/Cursor"))
        && is_executable(&bundle.join("Resources/app/bin/cursor"))
}

/// The program `name` from `PATH` or a trusted install location (see
/// [`crate::executables`]); an app opened from Finder has no shell `PATH`.
pub fn executable_in_path(name: &str) -> Option<PathBuf> {
    crate::executables::find(name, &[]).found
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    #[test]
    fn macos_cursor_bundle_requires_both_executables_and_is_not_started() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!(
            "sb-cursor-app-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        assert!(!cursor_app_at(&root));
        let bundle = root.join("Cursor.app/Contents");
        std::fs::create_dir_all(bundle.join("MacOS")).unwrap();
        std::fs::create_dir_all(bundle.join("Resources/app/bin")).unwrap();
        for path in [
            bundle.join("MacOS/Cursor"),
            bundle.join("Resources/app/bin/cursor"),
        ] {
            // Invalid executable data: the detector only inspects file metadata.
            std::fs::write(&path, b"not executable program contents").unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert!(cursor_app_at(&root));
        std::fs::set_permissions(
            bundle.join("MacOS/Cursor"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert!(!cursor_app_at(&root));
        std::fs::remove_dir_all(root).unwrap();
    }
}
