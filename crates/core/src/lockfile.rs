//! Project lock persistence only. Activation targets are never created or changed.
use crate::{
    Error, LockedSkill, LockfileWriteReport, ProjectLock, Result, agents, frontmatter, index,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const NAME: &str = ".kinana.lock";
pub const LEGACY_NAME: &str = ".skillsbox.lock";
static NEXT: AtomicU64 = AtomicU64::new(0);

pub fn path(project: &Path) -> Result<PathBuf> {
    let new = project.join(NAME);
    let old = project.join(LEGACY_NAME);
    let a = crate::state::exists(&new)?;
    let b = crate::state::exists(&old)?;
    if a && b {
        return Err(Error::decision(
            "MIGRATION_CONFLICT",
            "Both project lock names exist; neither is selected",
        ));
    }
    Ok(if b { old } else { new })
}
pub fn write_path(project: &Path) -> Result<PathBuf> {
    let selected = path(project)?;
    if selected.file_name().is_some_and(|name| name == LEGACY_NAME) {
        return Err(Error::decision(
            "MIGRATION_REQUIRED",
            "Legacy project lock is read-only; preview and confirm kinana migrate --project before modifying it",
        ));
    }
    Ok(selected)
}
const TOP: &[&str] = &["vault", "restoration", "bundle", "active"];
const MEMBER: &[&str] = &[
    "id",
    "agent",
    "mode",
    "name",
    "target_root",
    "generation_hash",
    "invocation",
    "content_hash",
    "ownership",
];
const RESTORE: &[&str] = &[
    "created_lock",
    "original_lock_mode",
    "original_empty_lock",
    "directories",
];
fn retain_known(value: &mut toml::Value, names: &[&str]) {
    if let Some(table) = value.as_table_mut() {
        table.retain(|key, _| names.contains(&key));
    }
}
fn prune_lock(value: &mut toml::Value) {
    retain_known(value, TOP);
    if let Some(items) = value.get_mut("active").and_then(toml::Value::as_array_mut) {
        for item in items {
            retain_known(item, MEMBER);
        }
    }
    if let Some(restoration) = value.get_mut("restoration") {
        retain_known(restoration, RESTORE);
        if let Some(items) = restoration
            .get_mut("directories")
            .and_then(toml::Value::as_array_mut)
        {
            for item in items {
                retain_known(item, &["path", "identity"]);
            }
        }
    }
}
fn merge_extensions(old: &toml::Value, new: &mut toml::Value, known: &[&str]) {
    if let (Some(old), Some(new)) = (old.as_table(), new.as_table_mut()) {
        for (key, value) in old {
            if !known.contains(&key.as_str()) {
                new.insert(key.clone(), value.clone());
            }
        }
    }
}
pub(crate) fn serialize_preserving(project: &Path, lock: &ProjectLock) -> Result<Vec<u8>> {
    let mut new = toml::Value::try_from(lock)?;
    if let Some(saved) = snapshot(&path(project)?)? {
        let old: toml::Value =
            toml::from_str(std::str::from_utf8(&saved.bytes).map_err(|e| invalid(e.to_string()))?)?;
        merge_extensions(&old, &mut new, TOP);
        if let (Some(old_items), Some(items)) = (
            old.get("active").and_then(toml::Value::as_array),
            new.get_mut("active").and_then(toml::Value::as_array_mut),
        ) {
            for item in items {
                if let Some(old) = old_items.iter().find(|old| {
                    old.get("id") == item.get("id") && old.get("agent") == item.get("agent")
                }) {
                    merge_extensions(old, item, MEMBER);
                }
            }
        }
        if let (Some(old), Some(new)) = (old.get("restoration"), new.get_mut("restoration")) {
            merge_extensions(old, new, RESTORE);
            if let (Some(old_items), Some(items)) = (
                old.get("directories").and_then(toml::Value::as_array),
                new.get_mut("directories")
                    .and_then(toml::Value::as_array_mut),
            ) {
                for item in items {
                    if let Some(old) = old_items
                        .iter()
                        .find(|old| old.get("path") == item.get("path"))
                    {
                        merge_extensions(old, item, &["path", "identity"]);
                    }
                }
            }
        }
    }
    Ok(toml::to_string_pretty(&new)?.into_bytes())
}

pub fn read(project: &Path) -> Result<Option<ProjectLock>> {
    let project = project_directory(project)?;
    let Some(saved) = snapshot(&path(&project)?)? else {
        return Ok(None);
    };
    let lock = parse(&saved.bytes)?;
    validate(&lock)?;
    Ok(Some(lock))
}

pub fn validate(lock: &ProjectLock) -> Result<()> {
    vault_path(&lock.vault)?;
    if let Some(bundle) = &lock.bundle {
        crate::vault::validate_id(bundle)
            .map_err(|_| invalid("Unsafe bundle name in project lock"))?;
    }
    let mut identities = BTreeSet::new();
    if let Some(restoration) = &lock.restoration {
        if restoration
            .original_lock_mode
            .is_some_and(|mode| mode > 0o7777)
        {
            return Err(invalid("Invalid original lock permissions"));
        }
        let mut paths = BTreeSet::new();
        for directory in &restoration.directories {
            if !allowed_owned_directory(&directory.path)
                || directory.identity.is_empty()
                || directory.identity.len() > 128
                || !directory
                    .identity
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte == b':')
                || !paths.insert(&directory.path)
            {
                return Err(invalid("Invalid owned project directory identity or path"));
            }
        }
        if let Some(original) = &restoration.original_empty_lock {
            let original: ProjectLock =
                parse(original.as_bytes()).map_err(|_| invalid("Invalid original empty lock"))?;
            if restoration.created_lock
                || original.vault != lock.vault
                || !original.active.is_empty()
                || original.restoration.is_some()
            {
                return Err(invalid(
                    "Original lock must be empty and belong to the same vault",
                ));
            }
            validate(&original)?;
        }
    }
    let mut targets: BTreeMap<(&str, &str), &LockedSkill> = BTreeMap::new();
    for skill in &lock.active {
        validate_skill(skill)?;
        if !identities.insert((&skill.id, &skill.agent)) {
            return Err(invalid("Project lock repeats a skill and agent"));
        }
        let key = (skill_prefix(skill)?, skill.name.as_str());
        if let Some(previous) = targets.insert(key, skill)
            && !(previous.agent != skill.agent
                && previous.id == skill.id
                && previous.mode == skill.mode
                && previous.invocation == skill.invocation
                && previous.content_hash == skill.content_hash
                && previous
                    .generation_hash
                    .as_ref()
                    .zip(skill.generation_hash.as_ref())
                    .is_none_or(|(left, right)| left == right)
                && previous.ownership == skill.ownership)
        {
            return Err(invalid("Conflicting shared activation target"));
        }
    }
    Ok(())
}

pub fn validate_for_vault(project: &Path, expected_vault: &Path, lock: &ProjectLock) -> Result<()> {
    project_directory(project)?;
    validate(lock)?;
    if vault_path(expected_vault)? != vault_path(&lock.vault)? {
        return Err(Error::decision(
            "FOREIGN_PROJECT_LOCK",
            "Project lock belongs to a different vault",
        ));
    }
    for skill in &lock.active {
        target_path(project, skill)?;
    }
    Ok(())
}

/// Compute the fixed agent/name destination. Existing parent links are refused;
/// the final target itself is deliberately neither followed nor inspected.
pub fn target_path(project: &Path, skill: &LockedSkill) -> Result<PathBuf> {
    validate_skill(skill)?;
    let project = project_directory(project)?;
    let prefix = skill_prefix(skill)?;
    let parent = index::validate_path(&project.join(prefix))?;
    match fs::symlink_metadata(&parent) {
        Ok(metadata) if !metadata.is_dir() => {
            return Err(invalid("Agent skills parent must be a directory"));
        }
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
        _ => {}
    }
    Ok(parent.join(&skill.name))
}

/// Only fixed allowlisted prefixes, including the historical Cursor target.
pub(crate) fn skill_prefix(skill: &LockedSkill) -> Result<&'static str> {
    match (skill.agent.as_str(), skill.target_root.as_deref()) {
        ("cursor", None | Some(".cursor/skills")) => Ok(".cursor/skills"),
        ("cursor", Some(".agents/skills")) | ("codex", None | Some(".agents/skills")) => {
            Ok(".agents/skills")
        }
        ("claude", None | Some(".claude/skills")) => Ok(".claude/skills"),
        _ => Err(invalid(
            "Unsupported agent or unsafe activation target root",
        )),
    }
}

pub fn write(
    project: &Path,
    expected_vault: &Path,
    lock: &ProjectLock,
    staging_agent: &str,
    dry_run: bool,
) -> Result<LockfileWriteReport> {
    let plan = plan(project, expected_vault, lock, staging_agent)?;
    if dry_run {
        let mut report = plan.report;
        report.dry_run = true;
        return Ok(report);
    }
    let mut prepared = PreparedLockfile::from_plan(plan)?;
    prepared.publish()?;
    prepared.finish()
}

/// Prepare only the fixed lock file; dropping an unfinished publication rolls it
/// back. Agent parent directories must already exist and are never created here.
pub fn prepare(
    project: &Path,
    expected_vault: &Path,
    lock: &ProjectLock,
    staging_agent: &str,
) -> Result<PreparedLockfile> {
    PreparedLockfile::from_plan(plan(project, expected_vault, lock, staging_agent)?)
}

struct Plan {
    report: LockfileWriteReport,
    before: Option<Snapshot>,
    staging_root: PathBuf,
    project_identity: fs::Metadata,
    staging_identity: fs::Metadata,
}

fn plan(
    project: &Path,
    expected_vault: &Path,
    lock: &ProjectLock,
    staging_agent: &str,
) -> Result<Plan> {
    let project = project_directory(project)?;
    validate_for_vault(&project, expected_vault, lock)?;
    let prefix = agents::project_skills(staging_agent)
        .ok_or_else(|| invalid("Unsupported staging agent"))?;
    let staging_root = index::validate_path(&project.join(prefix))?;
    let staging_identity = match fs::symlink_metadata(&staging_root) {
        Ok(metadata) if metadata.is_dir() => metadata,
        Ok(_) => return Err(invalid("Lock staging root must be a directory")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::new(
                "LOCK_STAGING_ROOT_MISSING",
                "Lock staging requires an existing agent skills directory; activation is not performed here",
            ));
        }
        Err(error) => return Err(error.into()),
    };
    let path = write_path(&project)?;
    let before = snapshot(&path)?;
    let changed = match &before {
        Some(saved) => {
            let existing = parse(&saved.bytes)?;
            validate_for_vault(&project, expected_vault, &existing)?;
            existing != *lock
        }
        None => true,
    };
    Ok(Plan {
        report: LockfileWriteReport {
            path,
            lock: lock.clone(),
            dry_run: false,
            changed,
            warnings: Vec::new(),
        },
        before,
        staging_root,
        project_identity: fs::symlink_metadata(project)?,
        staging_identity,
    })
}

pub struct PreparedLockfile {
    plan: Plan,
    after: Option<Snapshot>,
    stage: Option<PathBuf>,
    stage_identity: Option<fs::Metadata>,
    backed_up: bool,
    published: bool,
    applied: bool,
    finished: bool,
}

impl PreparedLockfile {
    fn from_plan(plan: Plan) -> Result<Self> {
        let mut prepared = Self {
            plan,
            after: None,
            stage: None,
            stage_identity: None,
            backed_up: false,
            published: false,
            applied: false,
            finished: false,
        };
        if !prepared.plan.report.changed {
            return Ok(prepared);
        }
        prepared.check_parents()?;
        let stage = loop {
            prepared.check_parents()?;
            let candidate = prepared.plan.staging_root.join(format!(
                ".skillsbox-lock-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            index::validate_path(&candidate)?;
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&candidate) {
                Ok(()) => break candidate,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        };
        prepared.stage_identity = Some(fs::symlink_metadata(&stage)?);
        prepared.stage = Some(stage.clone());
        let bytes = serialize_preserving(
            prepared.plan.report.path.parent().expect("project"),
            &prepared.plan.report.lock,
        )?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(stage.join("new.lock"))?;
        prepared.after = Some(Snapshot {
            bytes,
            identity: file.metadata()?,
        });
        if let Some(after) = &mut prepared.after {
            file.write_all(&after.bytes)?;
            if let Some(before) = &prepared.plan.before {
                file.set_permissions(before.identity.permissions())?;
            }
            after.identity = file.metadata()?;
        }
        Ok(prepared)
    }

    pub fn report(&self) -> &LockfileWriteReport {
        &self.plan.report
    }

    pub fn publish(&mut self) -> Result<()> {
        if self.applied || self.finished {
            return Err(invalid("Lock publication was already applied"));
        }
        self.check_parents()?;
        if !matches_snapshot(&self.plan.report.path, self.plan.before.as_ref())? {
            return Err(changed("Project lock changed during preparation"));
        }
        let Some(stage) = &self.stage else {
            self.applied = true;
            return Ok(());
        };
        self.check_stage()?;
        if let Some(before) = &self.plan.before {
            fs::rename(&self.plan.report.path, stage.join("old.lock"))?;
            self.backed_up = true;
            if !matches_snapshot(&stage.join("old.lock"), Some(before))? {
                self.plan.before = snapshot(&stage.join("old.lock"))?;
                return Err(changed(
                    "A concurrent lock replacement was moved; publication stopped",
                ));
            }
        }
        if !matches_snapshot(&stage.join("new.lock"), self.after.as_ref())? {
            return Err(changed("Staged project lock was modified"));
        }
        fs::hard_link(stage.join("new.lock"), &self.plan.report.path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                changed("A project lock appeared before publication")
            } else {
                error.into()
            }
        })?;
        self.published = true;
        if !matches_snapshot(&self.plan.report.path, self.after.as_ref())? {
            return Err(changed("Published project lock was replaced"));
        }
        self.applied = true;
        Ok(())
    }

    pub fn finish(mut self) -> Result<LockfileWriteReport> {
        if !self.applied {
            return Err(invalid("Publish the prepared lock before finishing"));
        }
        self.check_parents()?;
        let expected = if self.plan.report.changed {
            self.after.as_ref()
        } else {
            self.plan.before.as_ref()
        };
        if !matches_snapshot(&self.plan.report.path, expected)? {
            return Err(changed("Project lock changed before finalization"));
        }
        self.finished = true;
        if let Some(warning) = self.cleanup() {
            self.plan.report.warnings.push(warning);
        }
        Ok(self.plan.report.clone())
    }

    fn check_parents(&self) -> Result<()> {
        let project = self
            .plan
            .report
            .path
            .parent()
            .ok_or_else(|| invalid("Project lock has no parent"))?;
        let project = project_directory(project)?;
        let stage_root = index::validate_path(&self.plan.staging_root)?;
        if !same_entry(&self.plan.project_identity, &fs::symlink_metadata(project)?)
            || !same_entry(
                &self.plan.staging_identity,
                &fs::symlink_metadata(stage_root)?,
            )
        {
            return Err(changed("Project or staging directory was replaced"));
        }
        Ok(())
    }

    fn check_stage(&self) -> Result<()> {
        self.check_parents()?;
        if let Some(stage) = &self.stage {
            let path = index::validate_path(stage)?;
            if self.stage_identity.as_ref().is_none_or(|expected| {
                !fs::symlink_metadata(path).is_ok_and(|actual| same_entry(expected, &actual))
            }) {
                return Err(changed("Private lock stage was replaced"));
            }
        }
        Ok(())
    }

    fn rollback(&mut self) {
        if self.check_stage().is_err() {
            return;
        }
        let Some(stage) = &self.stage else {
            return;
        };
        if self.published {
            if matches_snapshot(&self.plan.report.path, self.after.as_ref()).unwrap_or(false) {
                if fs::remove_file(&self.plan.report.path).is_ok() {
                    self.published = false;
                }
            } else {
                self.published = false;
            }
        }
        if self.backed_up
            && !self.published
            && matches_snapshot(&stage.join("old.lock"), self.plan.before.as_ref()).unwrap_or(false)
            && fs::hard_link(stage.join("old.lock"), &self.plan.report.path).is_ok()
            && fs::remove_file(stage.join("old.lock")).is_ok()
        {
            self.backed_up = false;
        }
    }

    fn cleanup(&self) -> Option<String> {
        let stage = self.stage.as_ref()?;
        if fs::symlink_metadata(stage)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        {
            return None;
        }
        let warning = || {
            format!(
                "Modified or unremovable lock staging was preserved at {}",
                stage.display()
            )
        };
        if self.check_stage().is_err() || (!self.finished && (self.published || self.backed_up)) {
            return Some(warning());
        }
        for (name, expected) in [
            ("new.lock", self.after.as_ref()),
            ("old.lock", self.plan.before.as_ref()),
        ] {
            let path = stage.join(name);
            if let Some(expected) = expected
                && index::validate_path(&path).is_ok()
                && fs::symlink_metadata(&path)
                    .is_ok_and(|metadata| same_entry(&metadata, &expected.identity))
                && (name != "old.lock" || matches_snapshot(&path, Some(expected)).unwrap_or(false))
            {
                let _cleanup = fs::remove_file(path);
            }
        }
        match fs::remove_dir(stage) {
            Ok(()) => None,
            Err(_) => Some(warning()),
        }
    }
}

impl Drop for PreparedLockfile {
    fn drop(&mut self) {
        if !self.finished {
            self.rollback();
        }
        let _cleanup = self.cleanup();
    }
}

struct Snapshot {
    bytes: Vec<u8>,
    identity: fs::Metadata,
}

fn snapshot(path: &Path) -> Result<Option<Snapshot>> {
    let path = index::validate_path(path)?;
    let before = match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => return Err(invalid("Project lock must be a regular file")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let bytes = fs::read(&path)?;
    index::validate_path(&path)?;
    let after = fs::symlink_metadata(path)?;
    if !same_entry(&before, &after)
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return Err(changed("Project lock changed while being read"));
    }
    Ok(Some(Snapshot {
        bytes,
        identity: after,
    }))
}

fn matches_snapshot(path: &Path, expected: Option<&Snapshot>) -> Result<bool> {
    Ok(match (snapshot(path)?, expected) {
        (None, None) => true,
        (Some(actual), Some(expected)) => {
            same_entry(&actual.identity, &expected.identity) && actual.bytes == expected.bytes
        }
        _ => false,
    })
}

pub(crate) fn parse(bytes: &[u8]) -> Result<ProjectLock> {
    let text = std::str::from_utf8(bytes).map_err(|error| invalid(error.to_string()))?;
    let mut document: toml::Value =
        toml::from_str(text).map_err(|error| invalid(error.to_string()))?;
    prune_lock(&mut document);
    document
        .try_into()
        .map_err(|error: toml::de::Error| invalid(error.to_string()))
}

fn validate_skill(skill: &LockedSkill) -> Result<()> {
    crate::vault::validate_id(&skill.id)?;
    frontmatter::validate_name(&skill.name)?;
    if agents::project_skills(&skill.agent).is_none() {
        return Err(invalid(
            "Project lock agent must be claude, codex or cursor",
        ));
    }
    if !matches!(skill.mode.as_str(), "link" | "copy") {
        return Err(invalid("Project lock mode must be link or copy"));
    }
    if !matches!(skill.invocation.as_str(), "auto" | "manual") {
        return Err(invalid("Project lock invocation must be auto or manual"));
    }
    agents::validate_invocation(&skill.agent, &skill.invocation)?;
    if skill.generation_hash.as_ref().is_some_and(|value| {
        !value.strip_prefix("blake3:").is_some_and(|digest| {
            digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit())
        })
    }) {
        return Err(invalid("Invalid activation generation fingerprint"));
    }
    if let Some(ownership) = &skill.ownership
        && !ownership.strip_prefix("blake3:").is_some_and(|digest| {
            digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    {
        return Err(invalid("Invalid activation ownership fingerprint"));
    }
    if !skill
        .content_hash
        .strip_prefix("blake3:")
        .is_some_and(|digest| {
            digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    {
        return Err(invalid("Project lock requires a complete BLAKE3 hash"));
    }
    Ok(())
}

pub(crate) fn allowed_owned_directory(path: &str) -> bool {
    matches!(
        path,
        ".claude" | ".claude/skills" | ".agents" | ".agents/skills" | ".cursor" | ".cursor/skills"
    )
}

fn vault_path(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(invalid("Project lock vault must be an absolute path"));
    }
    let path = index::validate_path(path)?;
    match fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.is_dir() => {
            return Err(invalid("Project lock vault must be a directory"));
        }
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
        _ => {}
    }
    Ok(path)
}

fn project_directory(path: &Path) -> Result<PathBuf> {
    let path = index::validate_path(path)?;
    if !fs::symlink_metadata(&path)?.is_dir() {
        return Err(invalid("Project must be an existing real directory"));
    }
    Ok(path)
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new("INVALID_PROJECT_LOCK", message)
}
fn changed(message: impl Into<String>) -> Error {
    Error::decision("PROJECT_LOCK_CHANGED", message)
}

#[cfg(unix)]
fn same_entry(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev() && left.ino() == right.ino()
}
#[cfg(not(unix))]
fn same_entry(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.file_type() == right.file_type()
        && left
            .created()
            .ok()
            .zip(right.created().ok())
            .is_some_and(|(left, right)| left == right)
}
