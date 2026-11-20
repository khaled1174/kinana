use crate::dimensions::{self, Dimension};
use crate::{
    BoxConfig, Error, InitReport, ListFilter, Result, SkillDetail, SkillMetadata, SkillRecord,
};
use crate::{agents, frontmatter, gitlog, index};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct Vault {
    pub root: PathBuf,
    pub index_path: PathBuf,
}

impl Vault {
    pub fn new(root: PathBuf, index_path: PathBuf) -> Self {
        Self { root, index_path }
    }

    pub fn init(&self, dry_run: bool) -> Result<InitReport> {
        let root = index::validate_path(&self.root)?;
        let exists = match fs::symlink_metadata(&root) {
            Ok(metadata) if metadata.is_dir() => fs::read_dir(&root)?.next().transpose()?.is_some(),
            Ok(_) => {
                return Err(Error::decision(
                    "VAULT_CONFLICT",
                    "Vault path is not a directory",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.into()),
        };
        if exists {
            self.validate_structure()?;
        }
        let has_index = index::validate_target(&self.index_path)?;
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
        let detected = match home {
            Some(path) => agents::detect(Path::new(&path)),
            None => Vec::new(),
        };
        let mut planned_actions = Vec::new();
        if !exists {
            planned_actions.extend([
                format!("Create vault directories at {}", root.display()),
                "Write box.toml and .gitignore".into(),
                "Initialize git and commit the vault configuration".into(),
            ]);
        }
        if !has_index {
            planned_actions.push(format!(
                "Create SQLite index at {}",
                self.index_path.display()
            ));
        }
        if !dry_run {
            // The lease file exists, complete, before the first writer needs it.
            if let Some(home) = self
                .index_path
                .parent()
                .filter(|state| state.file_name().is_some_and(|name| name == ".kinana"))
                .and_then(Path::parent)
            {
                crate::state::ensure_lease_file(home)?;
            }
            let _state_guard = crate::state::write_guard(&self.index_path, false)?;
            if !exists {
                preflight_git()?;
                let config = BoxConfig {
                    detected_agents: detected
                        .iter()
                        .filter(|agent| agent.available)
                        .map(|agent| agent.name.clone())
                        .collect(),
                    ..BoxConfig::default()
                };
                let config = toml::to_string_pretty(&config)?;
                let mut transaction = InitTransaction::default();
                transaction.ensure_directory(&root)?;
                for name in ["skills", "meta", "bundles", "dist", ".git"] {
                    transaction.create_directory(&root.join(name))?;
                }
                transaction.write_file(&root.join("box.toml"), config.as_bytes())?;
                transaction.write_file(&root.join(".gitignore"), b"dist/\n.DS_Store\n")?;
                let initialized = gitlog::init(&root);
                transaction.capture_git_outputs(&root.join(".git"))?;
                initialized?;
                let committed = gitlog::commit(&root, "Initialize Kinana vault");
                transaction.capture_git_outputs(&root.join(".git"))?;
                committed?;
                transaction.finished = true;
            }
            if !has_index {
                index::initialize(&self.index_path)?;
            }
        }
        Ok(InitReport {
            vault: self.root.clone(),
            index: self.index_path.clone(),
            agents: detected,
            dry_run,
            created: !exists && !dry_run,
            planned_actions,
            router: None,
        })
    }

    pub fn undo(&self, dry_run: bool) -> Result<crate::UndoReport> {
        let _state_guard = crate::state::write_guard(&self.index_path, dry_run)?;
        self.config()?;
        gitlog::ensure_clean(&self.root)?;
        // Reverting the commit that introduced a skill deletes it. Like
        // `remove`, refuse while a registered project still has it active.
        for id in gitlog::skills_added_by_head(&self.root)? {
            let using = crate::remove::projects_using(self, &id)?;
            if !using.is_empty() {
                return Err(Error::decision(
                    "SKILL_IN_USE",
                    format!(
                        "Undoing the latest change would remove skill '{id}', which is active in registered project(s): {}. Run unuse for each project first; nothing was changed",
                        using
                            .iter()
                            .map(|project| project.display().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ));
            }
        }
        // Finder files that are a skill's only difference are removed first,
        // and only once the undo is known to be valid without them.
        let leftovers = crate::build::finder_leftovers(self)?;
        crate::build::preview_skipping(self, &leftovers)?;
        let manual_ids = crate::build::manual_cache_ids(self)?;
        if !dry_run && !leftovers.is_empty() {
            gitlog::undo(&self.root, true)?;
            crate::build::remove_finder_leftovers(self, &leftovers)?;
        }
        let mut report = gitlog::undo(&self.root, dry_run)?;
        report.finder_files_removed = crate::build::leftover_names(&leftovers);
        if !dry_run {
            crate::layout::restore(&self.root)?;
            crate::build::build(self, &[], false)
                .and_then(|_| crate::build::refresh_manual_cache(self, &manual_ids))
                .map_err(|error| {
                    Error::new(
                        "DIST_REFRESH_FAILED",
                        format!("Vault reverted, but dist refresh failed: {error}"),
                    )
                })?;
            index::rebuild_for_vault(&self.index_path, &self.root, &self.records()?).map_err(
                |error| {
                    Error::new(
                        "INDEX_REFRESH_FAILED",
                        format!("Vault reverted, but index refresh failed: {error}"),
                    )
                },
            )?;
        }
        Ok(report)
    }

    fn validate_structure(&self) -> Result<()> {
        self.config().map_err(|error| {
            Error::decision(
                "VAULT_CONFLICT",
                format!("Not an initialized Kinana vault: {error}"),
            )
        })?;
        for name in ["skills", "meta", "bundles", "dist", ".git"] {
            require_directory(&self.root.join(name))?;
        }
        require_file(&self.root.join(".gitignore"))?;
        Ok(())
    }

    pub fn config(&self) -> Result<BoxConfig> {
        let path = self.root.join("box.toml");
        require_file(&path)?;
        let config: BoxConfig = toml::from_str(&fs::read_to_string(path)?)?;
        if config.version != 1 {
            return Err(Error::new(
                "UNSUPPORTED_VAULT",
                format!("Unsupported vault version {}", config.version),
            ));
        }
        Ok(config)
    }

    /// Read the source files each time. The SQLite cache is never authoritative.
    pub fn records(&self) -> Result<Vec<SkillRecord>> {
        self.config()?;
        require_directory(&self.root.join("skills"))?;
        require_directory(&self.root.join("meta"))?;
        let mut ids = Vec::new();
        for entry in fs::read_dir(self.root.join("skills"))? {
            let entry = entry?;
            if entry.file_name() == ".DS_Store" {
                continue;
            }
            let path = entry.path();
            require_directory(&path)?;
            let id = entry
                .file_name()
                .into_string()
                .map_err(|_| Error::new("INVALID_SKILL_ID", "Skill IDs must be valid Unicode"))?;
            validate_id(&id)?;
            ids.push(id);
        }
        ids.sort();
        let mut records = Vec::with_capacity(ids.len());
        for id in &ids {
            records.push(self.read_detail(id)?.skill);
        }
        // Metadata without a matching source is also inconsistent, rather than
        // silently disappearing from a file-backed inventory.
        for entry in fs::read_dir(self.root.join("meta"))? {
            let entry = entry?;
            if entry.file_name() == ".DS_Store" {
                continue;
            }
            require_file(&entry.path())?;
            let filename = entry.file_name().into_string().map_err(|_| {
                Error::new("INVALID_SKILL_ID", "Metadata names must be valid Unicode")
            })?;
            let id = filename.strip_suffix(".toml").ok_or_else(|| {
                Error::new(
                    "INVALID_METADATA",
                    format!("Unexpected metadata file: {filename}"),
                )
            })?;
            validate_id(id)?;
            if ids
                .binary_search_by(|candidate| candidate.as_str().cmp(id))
                .is_err()
            {
                return Err(Error::new(
                    "ORPHAN_METADATA",
                    format!("Metadata has no source skill: {id}"),
                ));
            }
        }
        Ok(records)
    }

    pub fn list(&self, filter: &ListFilter) -> Result<Vec<SkillRecord>> {
        if let Some(agent) = &filter.agent
            && !matches!(agent.as_str(), "claude" | "codex" | "cursor")
        {
            return Err(Error::new(
                "INVALID_AGENT",
                format!("Unsupported agent: {agent}"),
            ));
        }
        // A misspelled dimension value is an error, not a silently empty list.
        if filter.stack.is_some() || filter.stage.is_some() {
            let config = self.config()?;
            for (dimension, value) in [
                (Dimension::Stack, &filter.stack),
                (Dimension::Stage, &filter.stage),
            ] {
                if let Some(value) = value {
                    dimensions::validate(
                        dimension,
                        std::slice::from_ref(value),
                        dimension.allowed(&config),
                    )?;
                }
            }
        }
        // All supported agents accept the common manifest; phase one has no activation data.
        let mut records: Vec<_> = self
            .records()?
            .into_iter()
            .filter(|record| {
                (!filter.pinned || record.pinned)
                    && filter
                        .category
                        .as_ref()
                        .is_none_or(|category| &record.category == category)
                    && filter
                        .stack
                        .as_ref()
                        .is_none_or(|stack| record.stack.contains(stack))
                    && filter
                        .stage
                        .as_ref()
                        .is_none_or(|stage| record.stage.contains(stage))
                    && filter
                        .audit
                        .as_ref()
                        .is_none_or(|audit| &record.audit_level == audit)
            })
            .collect();
        records.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(a.id.cmp(&b.id)));
        Ok(records)
    }

    pub fn show(&self, id: &str) -> Result<SkillDetail> {
        self.config()?;
        let mut detail = self.read_detail(id)?;
        if let Ok((locations, variants)) = index::detail_locations(&self.index_path, &self.root, id)
        {
            for path in locations {
                if !detail.locations.contains(&path) {
                    detail.locations.push(path);
                }
            }
            detail.locations.sort();
            detail.variants = variants;
        }
        Ok(detail)
    }

    fn read_detail(&self, id: &str) -> Result<SkillDetail> {
        validate_id(id)?;
        let directory = self.root.join("skills").join(id);
        require_directory(&directory)?;
        let skill_path = directory.join("SKILL.md");
        require_file(&skill_path)?;
        let metadata_path = self.root.join("meta").join(format!("{id}.toml"));
        require_file(&metadata_path)?;
        let metadata: SkillMetadata = toml::from_str(&fs::read_to_string(metadata_path)?)?;
        if metadata.id != id {
            return Err(Error::new(
                "METADATA_ID_MISMATCH",
                format!("Metadata ID {} does not match filename {id}", metadata.id),
            ));
        }
        let manifest = frontmatter::read_skill(&skill_path)?;
        let raw_skill_md = fs::read_to_string(&skill_path)?;
        let description = match metadata.description.active.as_str() {
            "original" => manifest.description,
            "improved"
                if metadata
                    .description
                    .approved_at
                    .as_ref()
                    .is_none_or(|approved| approved.trim().is_empty()) =>
            {
                return Err(Error::new(
                    "INVALID_METADATA",
                    format!("Active improved description has no approval: {id}"),
                ));
            }
            "improved" => metadata
                .description
                .improved
                .clone()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    Error::new(
                        "INVALID_METADATA",
                        format!("Active improved description is missing: {id}"),
                    )
                })?,
            _ => {
                return Err(Error::new(
                    "INVALID_METADATA",
                    format!("Unknown active description for {id}"),
                ));
            }
        };
        let skill = SkillRecord {
            pinned: metadata.pinned,
            id: id.into(),
            name: manifest.name,
            description,
            category: metadata.classification.category.clone(),
            stack: metadata.classification.stack.clone(),
            stage: metadata.classification.stage.clone(),
            audit_level: metadata.audit.level.clone(),
            content_hash: metadata.content_hash.clone(),
            source: metadata.source.clone(),
            added_at: metadata.added_at.clone(),
            role: metadata.classification.role.clone(),
            tags: metadata.classification.tags.clone(),
            when_to_use: metadata.classification.when_to_use.clone(),
        };
        Ok(SkillDetail {
            skill,
            locations: metadata.source.found_in.clone(),
            metadata,
            raw_skill_md,
            variants: Vec::new(),
        })
    }
}

pub(crate) fn validate_id(id: &str) -> Result<()> {
    let mut components = Path::new(id).components();
    if id.is_empty()
        || id.contains(['/', '\\'])
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
        || !id.chars().all(|character| {
            (character.is_alphanumeric() && !character.is_uppercase()) || character == '-'
        })
        || !id
            .chars()
            .next()
            .is_some_and(|character| character.is_alphanumeric() && !character.is_uppercase())
    {
        return Err(Error::new(
            "INVALID_SKILL_ID",
            format!("Unsafe skill ID: {id}"),
        ));
    }
    Ok(())
}

fn require_directory(path: &Path) -> Result<()> {
    let path = index::validate_path(path)?;
    if !fs::symlink_metadata(&path)?.is_dir() {
        return Err(Error::new(
            "INVALID_VAULT",
            format!("Expected directory: {}", path.display()),
        ));
    }
    Ok(())
}

fn require_file(path: &Path) -> Result<()> {
    let path = index::validate_path(path)?;
    if !fs::symlink_metadata(&path)?.is_file() {
        return Err(Error::new(
            "INVALID_VAULT",
            format!("Expected regular file: {}", path.display()),
        ));
    }
    Ok(())
}

fn preflight_git() -> Result<()> {
    // GIT_TOOLS_MISSING before the macOS stub can open an install prompt, or
    // GIT_ERROR naming every searched location.
    let executable = crate::executables::git(&[])?;
    let mut command = Command::new(executable);
    crate::executables::set_child_locale(&mut command);
    let output = command.arg("--version").output().map_err(|error| {
        Error::new(
            "GIT_ERROR",
            format!("Git is required to initialize a vault: {error}"),
        )
    })?;
    if !output.status.success() {
        return Err(Error::new(
            "GIT_ERROR",
            "Git is unavailable; no vault files were created",
        ));
    }
    Ok(())
}

#[derive(Default)]
struct InitTransaction {
    created: BTreeMap<PathBuf, OwnedEntry>,
    finished: bool,
}

struct OwnedEntry {
    metadata: fs::Metadata,
    bytes: Option<Vec<u8>>,
}

impl InitTransaction {
    fn ensure_directory(&mut self, path: &Path) -> Result<()> {
        index::validate_path(path)?;
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_dir() => Ok(()),
            Ok(_) => Err(Error::decision(
                "VAULT_CONFLICT",
                "Vault parent is not a directory",
            )),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let parent = path
                    .parent()
                    .ok_or_else(|| Error::new("INVALID_PATH", "Vault has no parent"))?;
                self.ensure_directory(parent)?;
                self.create_directory(path)
            }
            Err(error) => Err(error.into()),
        }
    }

    fn create_directory(&mut self, path: &Path) -> Result<()> {
        index::validate_path(path)?;
        fs::create_dir(path)?;
        self.created.insert(
            path.to_path_buf(),
            OwnedEntry {
                metadata: fs::symlink_metadata(path)?,
                bytes: None,
            },
        );
        Ok(())
    }

    fn write_file(&mut self, path: &Path, bytes: &[u8]) -> Result<()> {
        index::validate_path(path)?;
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        self.created.insert(
            path.to_path_buf(),
            OwnedEntry {
                metadata: file.metadata()?,
                bytes: Some(Vec::new()),
            },
        );
        let mut written = 0;
        while written < bytes.len() {
            let count = match file.write(&bytes[written..]) {
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            if count == 0 {
                return Err(std::io::Error::from(std::io::ErrorKind::WriteZero).into());
            }
            written += count;
            if let Some(entry) = self.created.get_mut(path) {
                entry.bytes = Some(bytes[..written].to_vec());
            }
        }
        Ok(())
    }

    fn capture_git_outputs(&mut self, git: &Path) -> Result<()> {
        index::validate_path(git)?;
        let head = git.join("HEAD");
        let branch = index::validate_path(&head)
            .ok()
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|head| head.trim().strip_prefix("ref: ").map(PathBuf::from))
            .filter(|path| {
                path.starts_with("refs/heads")
                    && path
                        .components()
                        .all(|part| matches!(part, Component::Normal(_)))
            });
        for entry in walkdir::WalkDir::new(git).min_depth(1).follow_links(false) {
            let entry = entry.map_err(|error| Error::new("IO_ERROR", error.to_string()))?;
            let path = entry.path();
            let relative = path
                .strip_prefix(git)
                .map_err(|error| Error::new("INVALID_PATH", error.to_string()))?;
            if entry.file_type().is_symlink()
                || !is_git_initialization_output(
                    relative,
                    entry.file_type().is_dir(),
                    branch.as_deref(),
                )
            {
                continue;
            }
            index::validate_path(path)?;
            let metadata = fs::symlink_metadata(path)?;
            let bytes = if metadata.is_file() {
                Some(fs::read(path)?)
            } else {
                None
            };
            self.created
                .insert(path.to_path_buf(), OwnedEntry { metadata, bytes });
        }
        Ok(())
    }
}

impl Drop for InitTransaction {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        // Remove only recorded outputs that still have the same identity and
        // contents. remove_dir preserves all unrecorded or concurrent entries.
        for (path, owned) in self.created.iter().rev() {
            if index::validate_path(path).is_err() {
                continue;
            }
            let Ok(current) = fs::symlink_metadata(path) else {
                continue;
            };
            if !same_entry(&owned.metadata, &current) || current.file_type().is_symlink() {
                continue;
            }
            if let Some(expected) = &owned.bytes {
                if fs::read(path).is_ok_and(|actual| actual == *expected) {
                    let _cleanup = fs::remove_file(path);
                }
            } else {
                let _cleanup = fs::remove_dir(path);
            }
        }
    }
}

#[cfg(unix)]
fn same_entry(original: &fs::Metadata, current: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    original.dev() == current.dev() && original.ino() == current.ino()
}

#[cfg(not(unix))]
fn same_entry(original: &fs::Metadata, current: &fs::Metadata) -> bool {
    original.file_type() == current.file_type()
        && original
            .created()
            .ok()
            .zip(current.created().ok())
            .is_some_and(|(a, b)| a == b)
}

fn is_git_initialization_output(path: &Path, directory: bool, branch: Option<&Path>) -> bool {
    if directory
        && matches!(
            path.to_str(),
            Some(
                "objects"
                    | "objects/info"
                    | "objects/pack"
                    | "refs"
                    | "refs/heads"
                    | "refs/tags"
                    | "logs"
                    | "logs/refs"
                    | "logs/refs/heads"
                    | "info"
            )
        )
    {
        return true;
    }
    if !directory
        && matches!(
            path.to_str(),
            Some(
                "HEAD"
                    | "config"
                    | "description"
                    | "index"
                    | "COMMIT_EDITMSG"
                    | "logs/HEAD"
                    | "info/attributes"
            )
        )
    {
        return true;
    }
    if let Some(branch) = branch {
        let log = Path::new("logs").join(branch);
        if (!directory && (path == branch || path == log))
            || (directory && (branch.starts_with(path) || log.starts_with(path)))
        {
            return true;
        }
    }
    let parts: Vec<_> = path
        .components()
        .map(|part| part.as_os_str().to_str())
        .collect();
    let hex = |value: Option<&str>, lengths: &[usize]| {
        value.is_some_and(|value| {
            lengths.contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    };
    matches!(parts.as_slice(), [Some("objects"), prefix] if directory && hex(*prefix, &[2]))
        || matches!(parts.as_slice(), [Some("objects"), prefix, object]
            if !directory && hex(*prefix, &[2]) && hex(*object, &[38, 62]))
}
