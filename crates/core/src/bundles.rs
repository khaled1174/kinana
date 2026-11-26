//! File-backed bundle definitions. Membership order is part of the saved data.
use crate::{
    Bundle, BundleDeleteReport, BundleSaveReport, Error, Result, gitlog, index, vault::Vault,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_STAGE: AtomicU64 = AtomicU64::new(0);

pub fn read(vault: &Vault, name: &str) -> Result<Bundle> {
    let directory = bundle_directory(vault)?;
    validate_name(name)?;
    let path = index::validate_path(&directory.join(format!("{name}.toml")))?;
    let saved = snapshot(&path)?.ok_or_else(|| missing(name))?;
    let bundle = parse(&saved.bytes)?;
    if bundle.name != name {
        return Err(invalid("Bundle name must match its filename"));
    }
    validate(vault, &bundle)?;
    Ok(bundle)
}

pub fn list(vault: &Vault) -> Result<Vec<Bundle>> {
    let directory = bundle_directory(vault)?;
    let mut names = BTreeSet::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_name() == ".DS_Store" {
            continue;
        }
        let path = index::validate_path(&entry.path())?;
        if !fs::symlink_metadata(&path)?.is_file() {
            return Err(invalid("Bundle entries must be regular TOML files"));
        }
        let filename = entry
            .file_name()
            .into_string()
            .map_err(|_| invalid("Bundle filenames must be valid Unicode"))?;
        let name = filename
            .strip_suffix(".toml")
            .ok_or_else(|| invalid("Bundle entries must have a .toml extension"))?;
        validate_name(name)?;
        names.insert(name.to_owned());
    }
    names.into_iter().map(|name| read(vault, &name)).collect()
}

/// Validate the whole definition using source files, without consulting SQLite.
pub fn validate(vault: &Vault, bundle: &Bundle) -> Result<()> {
    validate_name(&bundle.name)?;
    if bundle.title.trim().is_empty() {
        return Err(invalid("Bundle title must not be blank"));
    }
    let mut agents = BTreeSet::new();
    if bundle.agents.is_empty()
        || bundle.agents.iter().any(|agent| {
            !matches!(agent.as_str(), "claude" | "codex" | "cursor") || !agents.insert(agent)
        })
    {
        return Err(invalid(
            "Choose distinct bundle agents from claude, codex and cursor",
        ));
    }
    let known: BTreeSet<_> = vault
        .records()?
        .into_iter()
        .map(|record| record.id)
        .collect();
    let mut members = BTreeSet::new();
    let mut groups: BTreeMap<&str, usize> = BTreeMap::new();
    for member in &bundle.skills {
        crate::vault::validate_id(&member.id)?;
        if !members.insert(&member.id) {
            return Err(invalid(format!(
                "Skill appears more than once: {}",
                member.id
            )));
        }
        if !known.contains(&member.id) {
            return Err(Error::new(
                "SKILL_NOT_FOUND",
                format!("Unknown bundle skill: {}", member.id),
            ));
        }
        if !matches!(member.role.as_str(), "primary" | "secondary") {
            return Err(invalid("Bundle roles must be primary or secondary"));
        }
        if let Some(group) = &member.group {
            crate::vault::validate_id(group)
                .map_err(|_| invalid("Group names must be safe lowercase identifiers"))?;
            *groups.entry(group).or_default() += usize::from(member.role == "primary");
        }
    }
    for (group, primary) in groups {
        if primary != 1 {
            return Err(invalid(format!(
                "Group '{group}' must contain exactly one primary skill"
            )));
        }
    }
    Ok(())
}

pub fn create(vault: &Vault, input: &Path, dry_run: bool) -> Result<BundleSaveReport> {
    save(vault, None, input, dry_run)
}

pub fn edit(vault: &Vault, name: &str, input: &Path, dry_run: bool) -> Result<BundleSaveReport> {
    validate_name(name)?;
    save(vault, Some(name), input, dry_run)
}

fn save(
    vault: &Vault,
    editing: Option<&str>,
    input: &Path,
    dry_run: bool,
) -> Result<BundleSaveReport> {
    bundle_directory(vault)?;
    let input_path = index::validate_path(input)?;
    let input_snapshot = snapshot(&input_path)?.ok_or_else(|| {
        Error::new(
            "BUNDLE_INPUT_NOT_FOUND",
            format!("Bundle input is missing: {}", input_path.display()),
        )
    })?;
    let bundle = parse(&input_snapshot.bytes)?;
    save_definition_checked(vault, editing, bundle, dry_run, || {
        if !matches_snapshot(&input_path, Some(&input_snapshot))? {
            return Err(changed_error("Bundle input changed during validation"));
        }
        Ok(())
    })
}

pub fn save_definition(
    vault: &Vault,
    editing: Option<&str>,
    bundle: Bundle,
    dry_run: bool,
) -> Result<BundleSaveReport> {
    save_definition_checked(vault, editing, bundle, dry_run, || Ok(()))
}
fn save_definition_checked(
    vault: &Vault,
    editing: Option<&str>,
    bundle: Bundle,
    dry_run: bool,
    check_input: impl FnOnce() -> Result<()>,
) -> Result<BundleSaveReport> {
    let _state_guard = crate::state::write_guard(&vault.index_path, dry_run)?;
    let directory = bundle_directory(vault)?;

    validate(vault, &bundle)?;
    if editing.is_some_and(|name| name != bundle.name) {
        return Err(invalid("Editing a bundle cannot change its name"));
    }
    let target = index::validate_path(&directory.join(format!("{}.toml", bundle.name)))?;
    let before = snapshot(&target)?;
    let changed = match (&before, editing) {
        (Some(_), None) => {
            return Err(Error::decision(
                "BUNDLE_EXISTS",
                format!("Bundle already exists: {}", bundle.name),
            ));
        }
        (None, Some(name)) => return Err(missing(name)),
        (None, None) => true,
        (Some(saved), Some(_)) => {
            let existing = parse(&saved.bytes)?;
            if existing.name != bundle.name {
                return Err(invalid("Existing bundle name does not match its filename"));
            }
            existing != bundle
        }
    };
    // Complete preflight before creating a stage or touching either Git's index
    // or the bundle file. Input and target snapshots are checked again below.
    let source_roots = known_source_roots(vault)?;
    gitlog::reject_source_executable(&vault.root, &source_roots)?;
    gitlog::ensure_clean(&vault.root)?;
    index::check(&vault.index_path)?;
    let operation = if editing.is_some() { "edit" } else { "create" };
    let mut report = BundleSaveReport {
        dry_run,
        operation: operation.into(),
        bundle,
        path: target.clone(),
        changed,
        committed: false,
        warnings: Vec::new(),
    };
    if dry_run || !changed {
        return Ok(report);
    }
    check_input()?;
    let bytes = toml::to_string_pretty(&report.bundle)?.into_bytes();
    let relative = PathBuf::from("bundles").join(format!("{}.toml", report.bundle.name));
    let mut transaction = BundleSave::new(vault, target, relative, before, bytes, source_roots)?;
    transaction.publish()?;
    transaction.verify_published()?;
    gitlog::reject_source_executable(&vault.root, &transaction.source_roots)?;
    transaction.commit_attempted = true;
    let message = format!(
        "{} bundle {}",
        if editing.is_some() { "Edit" } else { "Create" },
        report.bundle.name
    );
    if let Err(mut error) = gitlog::commit_paths(
        &vault.root,
        &message,
        std::slice::from_ref(&transaction.relative),
    ) {
        if !transaction.rollback() {
            error.message.push_str(&format!(
                "; bundle rollback could not finish safely; inspect retained staging at {} and the Git index",
                transaction.stage.display()
            ));
        }
        return Err(error);
    }
    transaction.finished = true;
    report.committed = true;
    if let Some(warning) = transaction.cleanup() {
        report.warnings.push(warning);
    }
    if let Err(error) = vault
        .records()
        .and_then(|records| index::rebuild_for_vault(&vault.index_path, &vault.root, &records))
    {
        report
            .warnings
            .push(format!("Bundle saved; index refresh failed: {error}"));
    }
    Ok(report)
}

/// Delete one bundle file. Its skills and every project activation are left
/// alone. The file is not parsed: deleting is how an unreadable bundle is
/// repaired. A link in place of the file is refused, never followed or removed.
pub fn delete(vault: &Vault, name: &str, dry_run: bool) -> Result<BundleDeleteReport> {
    use crate::activation_transaction::{self as tx, Data, PlannedWrite};
    let _state_guard = crate::state::write_guard(&vault.index_path, dry_run)?;
    // The name is data. It must be one safe component before it joins any path.
    validate_name(name)?;
    let vault = Vault::new(
        index::validate_path(&vault.root)?,
        index::validate_path(&vault.index_path)?,
    );
    let directory = bundle_directory(&vault)?;
    let target = index::validate_path(&directory.join(format!("{name}.toml")))?;
    let before = tx::Snapshot::read(&target)?.ok_or_else(|| missing(name))?;
    if before.bytes().is_none() {
        return Err(invalid(format!(
            "Bundle path must be a regular file: {}",
            target.display()
        )));
    }
    let dist = require_directory(&vault.root.join("dist"))?;
    let source_roots = known_source_roots(&vault)?;
    gitlog::reject_source_executable(&vault.root, &source_roots)?;
    gitlog::ensure_clean(&vault.root)?;
    index::check(&vault.index_path)?;
    let mut report = BundleDeleteReport {
        name: name.into(),
        dry_run,
        changed: true,
        committed: false,
    };
    let writes = vec![PlannedWrite {
        target,
        stage_parent: dist.clone(),
        before: Some(before),
        data: Data::Remove,
    }];
    let anchors = crate::remove::anchors(&[&vault.root, &directory, &dist])?;
    tx::preflight(&writes, &[], &anchors)?;
    if dry_run {
        return Ok(report);
    }
    crate::remove::publish(
        &vault,
        writes,
        anchors,
        &[PathBuf::from("bundles").join(format!("{name}.toml"))],
        &format!("Delete bundle {name}"),
        &format!("Bundle '{name}' was deleted"),
    )?;
    report.committed = true;
    Ok(report)
}

struct Snapshot {
    bytes: Vec<u8>,
    identity: fs::Metadata,
}

fn snapshot(path: &Path) -> Result<Option<Snapshot>> {
    let path = index::validate_path(path)?;
    let before = match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => {
            return Err(invalid(format!(
                "Bundle path must be a regular file: {}",
                path.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let bytes = fs::read(&path)?;
    let checked = index::validate_path(&path)?;
    let after = fs::symlink_metadata(checked)?;
    if !same_entry(&before, &after)
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return Err(changed_error("Bundle file changed while reading it"));
    }
    Ok(Some(Snapshot {
        bytes,
        identity: after,
    }))
}

fn matches_snapshot(path: &Path, expected: Option<&Snapshot>) -> Result<bool> {
    Ok(match (snapshot(path)?, expected) {
        (None, None) => true,
        (Some(current), Some(expected)) => {
            same_entry(&current.identity, &expected.identity) && current.bytes == expected.bytes
        }
        _ => false,
    })
}

fn parse(bytes: &[u8]) -> Result<Bundle> {
    let text = std::str::from_utf8(bytes).map_err(|error| invalid(error.to_string()))?;
    Ok(toml::from_str(text)?)
}

#[derive(Debug, serde::Serialize)]
pub struct MemberView {
    #[serde(flatten)]
    pub member: crate::BundleSkill,
    pub audit_level: &'static str,
}

#[derive(Debug, serde::Serialize)]
pub struct BundleView {
    pub name: String,
    pub title: String,
    pub agents: Vec<String>,
    pub skills: Vec<MemberView>,
    pub risky_members: usize,
}

pub fn views(vault: &Vault, bundles: Vec<Bundle>) -> Vec<BundleView> {
    let levels = crate::activation_risk::levels(
        vault,
        bundles
            .iter()
            .flat_map(|bundle| bundle.skills.iter().map(|member| member.id.clone())),
    );
    bundles
        .into_iter()
        .map(|bundle| BundleView {
            name: bundle.name,
            title: bundle.title,
            agents: bundle.agents,
            risky_members: bundle
                .skills
                .iter()
                .filter(|member| levels[&member.id] == "risky")
                .count(),
            skills: bundle
                .skills
                .into_iter()
                .map(|member| MemberView {
                    audit_level: levels[&member.id],
                    member,
                })
                .collect(),
        })
        .collect()
}

fn bundle_directory(vault: &Vault) -> Result<PathBuf> {
    vault.config()?;
    require_directory(&vault.root.join("bundles"))
}

fn known_source_roots(vault: &Vault) -> Result<Vec<PathBuf>> {
    Ok(vault
        .records()?
        .into_iter()
        .flat_map(|record| record.source.found_in)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect())
}

fn require_directory(path: &Path) -> Result<PathBuf> {
    let path = index::validate_path(path)?;
    if !fs::symlink_metadata(&path)?.is_dir() {
        return Err(invalid(format!("Expected a directory: {}", path.display())));
    }
    Ok(path)
}

fn validate_name(name: &str) -> Result<()> {
    crate::vault::validate_id(name)
        .map_err(|_| Error::new("INVALID_BUNDLE_NAME", format!("Unsafe bundle name: {name}")))
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new("INVALID_BUNDLE", message)
}
fn changed_error(message: impl Into<String>) -> Error {
    Error::decision("BUNDLE_CHANGED", message)
}
fn missing(name: &str) -> Error {
    Error::new("BUNDLE_NOT_FOUND", format!("Bundle does not exist: {name}"))
}

/// One bundle file only: its old inode stays in the ignored stage until commit.
/// A no-clobber hard link publishes the new file without replacing foreign data.
struct BundleSave {
    root: PathBuf,
    target: PathBuf,
    relative: PathBuf,
    source_roots: Vec<PathBuf>,
    before: Option<Snapshot>,
    after: Snapshot,
    stage: PathBuf,
    stage_identity: fs::Metadata,
    backed_up: bool,
    published: bool,
    finished: bool,
    commit_attempted: bool,
}

impl BundleSave {
    fn new(
        vault: &Vault,
        target: PathBuf,
        relative: PathBuf,
        before: Option<Snapshot>,
        bytes: Vec<u8>,
        source_roots: Vec<PathBuf>,
    ) -> Result<Self> {
        let dist = require_directory(&vault.root.join("dist"))?;
        let stage = loop {
            let candidate = dist.join(format!(
                ".skillsbox-bundle-{}-{}",
                std::process::id(),
                NEXT_STAGE.fetch_add(1, Ordering::Relaxed)
            ));
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
        let stage_identity = fs::symlink_metadata(&stage)?;
        // Construct the guard before writing, so partial staging also cleans up.
        let mut transaction = Self {
            root: index::validate_path(&vault.root)?,
            target,
            relative,
            source_roots,
            before,
            after: Snapshot {
                bytes,
                identity: stage_identity.clone(),
            },
            stage,
            stage_identity,
            backed_up: false,
            published: false,
            finished: false,
            commit_attempted: false,
        };
        let temporary = transaction.stage.join("new.toml");
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        transaction.after.identity = file.metadata()?;
        file.write_all(&transaction.after.bytes)?;
        if let Some(before) = &transaction.before {
            file.set_permissions(before.identity.permissions())?;
        }
        transaction.after.identity = file.metadata()?;
        Ok(transaction)
    }

    fn verify_stage(&self) -> Result<()> {
        let path = index::validate_path(&self.stage)?;
        if !same_entry(&self.stage_identity, &fs::symlink_metadata(path)?) {
            return Err(changed_error("Private bundle stage was replaced"));
        }
        Ok(())
    }

    fn publish(&mut self) -> Result<()> {
        self.verify_stage()?;
        if !matches_snapshot(&self.target, self.before.as_ref())? {
            return Err(changed_error("Bundle target changed during validation"));
        }
        if let Some(before) = &self.before {
            fs::rename(&self.target, self.stage.join("old.toml"))?;
            self.backed_up = true;
            if !matches_snapshot(&self.stage.join("old.toml"), Some(before))? {
                // A regular foreign replacement raced with the rename. Capture
                // that exact moved inode so rollback can put it back safely.
                self.before = snapshot(&self.stage.join("old.toml"))?;
                return Err(changed_error(
                    "Bundle target changed before its backup was made",
                ));
            }
        }
        if !matches_snapshot(&self.stage.join("new.toml"), Some(&self.after))? {
            return Err(changed_error("Staged bundle changed before publication"));
        }
        fs::hard_link(self.stage.join("new.toml"), &self.target).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                changed_error("A foreign bundle appeared before publication")
            } else {
                error.into()
            }
        })?;
        self.published = true;
        self.verify_published()
    }

    fn verify_published(&self) -> Result<()> {
        if !matches_snapshot(&self.target, Some(&self.after))? {
            return Err(changed_error("Published bundle was modified concurrently"));
        }
        Ok(())
    }

    fn rollback(&mut self) -> bool {
        if self.verify_stage().is_err() {
            return false;
        }
        if self.published {
            if self.verify_published().is_ok() {
                if fs::remove_file(&self.target).is_ok() {
                    self.published = false;
                }
            } else {
                // The current target belongs to the concurrent writer. Never
                // remove it, even when it contains the same bytes in a new inode.
                self.published = false;
            }
        }
        if self.backed_up && !self.published {
            let backup = self.stage.join("old.toml");
            // Restore only the captured backup; a concurrent stage replacement
            // survives. hard_link also refuses a concurrent destination.
            if matches_snapshot(&backup, self.before.as_ref()).unwrap_or(false)
                && fs::hard_link(&backup, &self.target).is_ok()
                && fs::remove_file(&backup).is_ok()
            {
                self.backed_up = false;
            }
        }
        if self.commit_attempted {
            if gitlog::reject_source_executable(&self.root, &self.source_roots).is_err()
                || gitlog::unstage_paths(&self.root, std::slice::from_ref(&self.relative)).is_err()
            {
                return false;
            }
            self.commit_attempted = false;
        }
        !self.published && !self.backed_up
    }

    fn cleanup(&self) -> Option<String> {
        if let Err(error) = fs::symlink_metadata(&self.stage)
            && error.kind() == std::io::ErrorKind::NotFound
        {
            return None;
        }
        if self.verify_stage().is_err()
            || (!self.finished && (self.published || self.backed_up || self.commit_attempted))
        {
            return Some(self.retained_warning());
        }
        for (name, expected) in [
            ("new.toml", Some(&self.after)),
            ("old.toml", self.before.as_ref()),
        ] {
            let path = self.stage.join(name);
            if let Some(expected) = expected
                && index::validate_path(&path).is_ok()
                && fs::symlink_metadata(&path).is_ok_and(|metadata| {
                    metadata.is_file() && same_entry(&metadata, &expected.identity)
                })
                && (name != "old.toml" || matches_snapshot(&path, Some(expected)).unwrap_or(false))
            {
                let _cleanup = fs::remove_file(path);
            }
        }
        match fs::remove_dir(&self.stage) {
            Ok(()) => None,
            Err(_) => Some(self.retained_warning()),
        }
    }

    fn retained_warning(&self) -> String {
        format!(
            "Bundle staging contains modified or unremovable data and was preserved at {}",
            self.stage.display()
        )
    }
}

impl Drop for BundleSave {
    fn drop(&mut self) {
        if !self.finished {
            self.rollback();
        }
        let _cleanup = self.cleanup();
    }
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
