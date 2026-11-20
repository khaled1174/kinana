mod support;
use kinana_core::{Result, agents};
use std::{fs, path::PathBuf};
struct Home(PathBuf);
impl Home {
    fn new() -> Self {
        Self(support::temp_dir().unwrap())
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn names(agents: &[&str]) -> Vec<String> {
    agents.iter().map(|s| (*s).into()).collect()
}
#[test]
fn automatic_discovery_finds_known_global_roots_and_deduplicates_shared_paths() -> Result<()> {
    let home = Home::new();
    for name in [
        ".claude/skills",
        ".agents/skills",
        ".codex/skills",
        ".cursor/skills",
    ] {
        fs::create_dir_all(home.0.join(name))?;
    }
    let report =
        agents::skill_locations(&home.0, &names(&["claude", "codex", "cursor", "cursor"]))?;
    assert_eq!(report.roots.len(), 4);
    assert_eq!(report.locations.len(), 4);
    let shared = report
        .locations
        .iter()
        .find(|p| p.path == home.0.join(".agents/skills"))
        .unwrap();
    assert_eq!(shared.agents, ["codex", "cursor"]);
    assert_eq!(shared.status, "ready");
    assert!(
        report
            .roots
            .iter()
            .all(|p| p.starts_with(&home.0) && p.file_name().unwrap() == "skills")
    );
    assert!(!home.0.join(".kinana").exists());
    Ok(())
}
#[test]
fn selected_agent_limits_paths_and_legacy_codex_skills_are_found() -> Result<()> {
    let home = Home::new();
    fs::create_dir_all(home.0.join(".claude/skills"))?;
    fs::create_dir_all(home.0.join(".codex/skills"))?;
    let claude = agents::skill_locations(&home.0, &names(&["claude"]))?;
    assert_eq!(claude.roots, [home.0.join(".claude/skills")]);
    assert_eq!(claude.locations.len(), 1);
    let codex = agents::skill_locations(&home.0, &names(&["codex"]))?;
    assert_eq!(codex.roots, [home.0.join(".codex/skills")]);
    assert_eq!(codex.locations.len(), 2);
    Ok(())
}
#[test]
fn missing_locations_do_not_create_directories_or_search_the_whole_home() -> Result<()> {
    let home = Home::new();
    fs::create_dir(home.0.join("unrelated"))?;
    fs::write(
        home.0.join("unrelated/SKILL.md"),
        "Do not discover this unrelated file",
    )?;
    let report = agents::skill_locations(&home.0, &names(&["claude", "codex", "cursor"]))?;
    assert!(report.roots.is_empty());
    assert_eq!(report.locations.len(), 4);
    assert!(report.locations.iter().all(|p| p.status == "missing"));
    assert_eq!(fs::read_dir(&home.0)?.count(), 1);
    assert_eq!(
        agents::skill_locations(&home.0, &[]).unwrap_err().code,
        "NO_SCAN_AGENTS"
    );
    assert_eq!(
        agents::skill_locations(&home.0, &names(&["unknown"]))
            .unwrap_err()
            .code,
        "INVALID_AGENT"
    );
    Ok(())
}
#[cfg(unix)]
#[test]
fn symbolic_links_and_foreign_files_are_preserved_and_never_scanned() -> Result<()> {
    use std::os::unix::fs::symlink;
    let home = Home::new();
    let outside = Home::new();
    fs::write(outside.0.join("SKILL.md"), "Do not follow")?;
    fs::create_dir(home.0.join(".claude"))?;
    symlink(&outside.0, home.0.join(".claude/skills"))?;
    fs::create_dir(home.0.join(".codex"))?;
    fs::write(home.0.join(".codex/skills"), "Preserve this foreign file")?;
    let report = agents::skill_locations(&home.0, &names(&["claude", "codex"]))?;
    assert!(report.roots.is_empty());
    assert_eq!(
        report
            .locations
            .iter()
            .filter(|p| p.status == "blocked")
            .count(),
        2
    );
    assert!(
        report
            .locations
            .iter()
            .any(|p| p.reason_code == "SYMLINK_PATH")
    );
    assert!(home.0.join(".claude/skills").is_symlink());
    assert_eq!(
        fs::read_to_string(home.0.join(".codex/skills"))?,
        "Preserve this foreign file"
    );
    Ok(())
}
