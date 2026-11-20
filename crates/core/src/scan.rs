//! Read-only discovery restricted to explicitly supplied roots.

use crate::hash::{checked_absolute_path, checked_metadata, hash_directory};
use crate::{Error, Result, ScanEntry, ScanReport, ScanStatus, SkillRecord, SourceInfo};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const AGENT_DIRECTORIES: &[&str] = &[".claude", ".agents", ".codex", ".cursor"];

struct Discovery<'a> {
    check: &'a dyn Fn() -> Result<()>,
    candidates: BTreeSet<PathBuf>,
    visited: BTreeSet<PathBuf>,
    warnings: BTreeSet<String>,
}

/// Discover skills without writing an index, changing a project, or executing a skill.
///
/// Valid skills are compared against vault records first, then earlier paths in
/// this report. All paths and classifications are deterministic across root order.
/// Invalid roots become warnings when another root is usable; with no usable root
/// the call fails. No global or home directories are added implicitly.
pub fn scan(paths: &[PathBuf], known: &[SkillRecord]) -> Result<ScanReport> {
    scan_with_check(paths, known, &|| Ok(()))
}
pub fn scan_with_check(
    paths: &[PathBuf],
    known: &[SkillRecord],
    check: &dyn Fn() -> Result<()>,
) -> Result<ScanReport> {
    check()?;
    if paths.is_empty() {
        return Err(Error::new("NO_SCAN_PATHS", "Supply at least one scan root"));
    }
    let mut discovery = Discovery {
        check,
        candidates: Default::default(),
        visited: Default::default(),
        warnings: Default::default(),
    };
    let mut roots = BTreeSet::new();
    for supplied in paths {
        check()?;
        match checked_absolute_path(supplied).and_then(|path| {
            if checked_metadata(&path)?.is_dir() {
                Ok(path)
            } else {
                Err(Error::new(
                    "NOT_A_DIRECTORY",
                    format!("Scan root is not a directory: {}", path.display()),
                ))
            }
        }) {
            Ok(path) => {
                roots.insert(path);
            }
            Err(error) => {
                discovery
                    .warnings
                    .insert(format!("Skipped scan root {}: {error}", supplied.display()));
            }
        }
    }
    if roots.is_empty() {
        return Err(Error::new(
            "INVALID_SCAN_ROOT",
            discovery
                .warnings
                .into_iter()
                .collect::<Vec<_>>()
                .join("; "),
        ));
    }
    let excluded_roots = roots.iter().cloned().collect::<Vec<_>>();
    for root in roots {
        check()?;
        if discovery.has_manifest(&root) {
            discovery.candidates.insert(root);
        } else if is_skills_directory(&root) {
            discovery.visit_skills(&root);
        } else if is_agent_directory(&root) {
            discovery.visit_agent(&root);
        } else if !ignored_name(&root) {
            discovery.visit_projects(&root);
        }
    }

    let mut by_hash = BTreeMap::new();
    let mut by_name = BTreeMap::new();
    let mut known = known.iter().collect::<Vec<_>>();
    known.sort_by(|left, right| left.id.cmp(&right.id));
    for record in known {
        by_hash
            .entry(record.content_hash.clone())
            .or_insert_with(|| Some(record.id.clone()));
        by_name
            .entry(record.name.clone())
            .or_insert_with(|| Some(record.id.clone()));
    }

    let mut report = ScanReport::default();
    for path in &discovery.candidates {
        check()?;
        let mut entry = ScanEntry {
            path: path.clone(),
            name: None,
            description: None,
            content_hash: None,
            status: ScanStatus::Broken,
            skill_id: None,
            source: SourceInfo::default(),
            error: None,
        };
        // Hashing validates the whole tree before the parser opens SKILL.md.
        // In particular a symbolic link named SKILL.md is never parsed.
        let inspected = hash_directory(path).and_then(|hash| {
            entry.content_hash = Some(hash.clone());
            checked_absolute_path(&path.join("SKILL.md"))?;
            crate::frontmatter::read_skill(&path.join("SKILL.md")).map(|manifest| (hash, manifest))
        });
        match inspected {
            Ok((hash, manifest)) => {
                entry.name = Some(manifest.name.clone());
                entry.description = Some(manifest.description);
                entry.source = crate::gitlog::remote_hint_excluding(path, &excluded_roots);
                entry.source.found_in = vec![path.clone()];
                if let Some(id) = by_hash.get(&hash) {
                    entry.status = ScanStatus::Duplicate;
                    entry.skill_id = id.clone();
                } else if let Some(id) = by_name.get(&manifest.name) {
                    entry.status = ScanStatus::Variant;
                    entry.skill_id = id.clone();
                } else {
                    entry.status = ScanStatus::New;
                }
                by_hash
                    .entry(hash)
                    .or_insert_with(|| entry.skill_id.clone());
                by_name
                    .entry(manifest.name)
                    .or_insert_with(|| entry.skill_id.clone());
            }
            Err(error) => {
                if error.code == "SYMLINK_REJECTED" {
                    discovery.warnings.insert(error.to_string());
                }
                entry.source.found_in = vec![path.clone()];
                entry.error = Some(error.to_string());
            }
        }
        match entry.status {
            ScanStatus::New => report.counts.new += 1,
            ScanStatus::Duplicate => report.counts.duplicate += 1,
            ScanStatus::Variant => report.counts.variant += 1,
            ScanStatus::Broken => report.counts.broken += 1,
        }
        report.entries.push(entry);
    }
    check()?;
    report.warnings = discovery.warnings.into_iter().collect();
    Ok(report)
}

impl Discovery<'_> {
    fn visit_projects(&mut self, directory: &Path) {
        if !self.visited.insert(directory.to_path_buf()) {
            return;
        }
        for child in self.children(directory) {
            if !self.is_directory(&child) {
                continue;
            }
            if is_agent_directory(&child) {
                self.visit_agent(&child);
            } else if !ignored_name(&child) {
                self.visit_projects(&child);
            }
        }
    }

    fn visit_agent(&mut self, directory: &Path) {
        if let Err(error) = checked_absolute_path(directory) {
            self.warnings.insert(error.to_string());
            return;
        }
        let skills = directory.join("skills");
        match fs::symlink_metadata(&skills) {
            Ok(metadata) if metadata.file_type().is_symlink() => self.warn_link(&skills),
            Ok(metadata) if metadata.is_dir() => self.visit_skills(&skills),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => self.warn_io(&skills, error),
        }
    }

    fn visit_skills(&mut self, directory: &Path) {
        if !self.visited.insert(directory.to_path_buf()) {
            return;
        }
        for child in self.children(directory) {
            if self.is_directory(&child) && !ignored_name(&child) && self.has_manifest(&child) {
                self.candidates.insert(child);
            }
        }
    }

    fn has_manifest(&mut self, directory: &Path) -> bool {
        if let Err(error) = checked_absolute_path(directory) {
            self.warnings.insert(error.to_string());
            return false;
        }
        let manifest = directory.join("SKILL.md");
        match fs::symlink_metadata(&manifest) {
            // Keep linked manifests as candidates so the rejected skill is
            // visible as Broken instead of silently disappearing.
            Ok(metadata) => metadata.is_file() || metadata.file_type().is_symlink(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                self.warn_io(&manifest, error);
                false
            }
        }
    }

    fn children(&mut self, directory: &Path) -> Vec<PathBuf> {
        if (self.check)().is_err() {
            return vec![];
        }
        // An explicitly supplied root and every directory reached beneath it
        // are checked again before read_dir, including their components.
        if let Err(error) = checked_absolute_path(directory) {
            self.warnings.insert(error.to_string());
            return Vec::new();
        }
        let children = match fs::read_dir(directory) {
            Ok(children) => children,
            Err(error) => {
                self.warn_io(directory, error);
                return Vec::new();
            }
        };
        let mut paths = Vec::new();
        for child in children {
            if (self.check)().is_err() {
                break;
            }
            match child {
                Ok(child) => paths.push(child.path()),
                Err(error) => self.warn_io(directory, error),
            }
        }
        paths.sort();
        paths
    }

    fn is_directory(&mut self, path: &Path) -> bool {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                self.warn_link(path);
                false
            }
            Ok(metadata) => metadata.is_dir(),
            Err(error) => {
                self.warn_io(path, error);
                false
            }
        }
    }

    fn warn_link(&mut self, path: &Path) {
        self.warnings
            .insert(format!("Skipped symbolic link: {}", path.display()));
    }

    fn warn_io(&mut self, path: &Path, error: std::io::Error) {
        self.warnings
            .insert(format!("Could not inspect {}: {error}", path.display()));
    }
}

fn is_agent_directory(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| AGENT_DIRECTORIES.contains(&name))
}

fn is_skills_directory(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == "skills")
        && path.parent().is_some_and(is_agent_directory)
}

fn ignored_name(path: &Path) -> bool {
    path.file_name().is_some_and(|name| {
        name.as_encoded_bytes().starts_with(b".")
            || matches!(
                name.to_str(),
                Some("node_modules" | "target" | "dist" | "Library" | "plugins")
            )
    })
}
