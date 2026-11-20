//! Shared metadata changes with Git history and rollback of generated dist.
use crate::{
    BuildReport, EditResult, Error, Result, SkillMetadata, build,
    dimensions::{self, Dimension},
    frontmatter, gitlog, index,
    vault::Vault,
};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);

pub fn read_snapshot(vault: &Vault, id: &str) -> Result<(SkillMetadata, Vec<u8>)> {
    vault.config()?;
    crate::vault::validate_id(id)?;
    let path = index::validate_path(&vault.root.join("meta").join(format!("{id}.toml")))?;
    if !fs::symlink_metadata(&path)?.is_file() {
        return Err(Error::new(
            "INVALID_METADATA",
            "Expected regular metadata file",
        ));
    }
    let bytes = fs::read(path)?;
    let text =
        std::str::from_utf8(&bytes).map_err(|e| Error::new("INVALID_METADATA", e.to_string()))?;
    let meta: SkillMetadata = toml::from_str(text)?;
    if meta.id != id {
        return Err(Error::new(
            "METADATA_ID_MISMATCH",
            "Metadata ID does not match its filename",
        ));
    }
    Ok((meta, bytes))
}
pub fn read_metadata(vault: &Vault, id: &str) -> Result<SkillMetadata> {
    Ok(read_snapshot(vault, id)?.0)
}

fn check_git_trust(vault: &Vault) -> Result<()> {
    let roots = vault
        .records()?
        .into_iter()
        .flat_map(|r| r.source.found_in)
        .collect::<Vec<_>>();
    gitlog::reject_source_executable(&vault.root, &roots)
}

/// A favorite is metadata only: no dist staging/rebuild or index writes, even temporarily.
pub fn set_pinned(
    vault: &Vault,
    id: &str,
    pinned: bool,
    dry_run: bool,
) -> Result<crate::PinReport> {
    let _state_guard = crate::state::write_guard(&vault.index_path, dry_run)?;
    let vault = Vault::new(index::validate_path(&vault.root)?, vault.index_path.clone());
    vault.show(id)?;
    let (before, bytes) = read_snapshot(&vault, id)?;
    check_git_trust(&vault)?;
    gitlog::ensure_clean(&vault.root)?;
    let mut report = crate::PinReport {
        id: id.into(),
        pinned,
        dry_run,
        changed: before.pinned != pinned,
        committed: false,
    };
    if dry_run || !report.changed {
        return Ok(report);
    }
    let mut value: toml::Value = toml::from_str(
        std::str::from_utf8(&bytes).map_err(|e| Error::new("INVALID_METADATA", e.to_string()))?,
    )?;
    value
        .as_table_mut()
        .ok_or_else(|| Error::new("INVALID_METADATA", "Metadata must be a table"))?
        .insert("pinned".into(), toml::Value::Boolean(pinned));
    let after = toml::to_string_pretty(&value)?.into_bytes();
    // Git metadata is the only staging area used here; skills/ and dist/ are never written.
    let mut transaction = MetadataTransaction::new_in(&vault.root, &vault.root.join(".git"))?;
    transaction.publish(id, &bytes, &after)?;
    transaction.verify()?;
    transaction.commit_attempted = true;
    gitlog::commit_paths(
        &vault.root,
        &format!("Set favorite for {id}: {pinned}"),
        &transaction.paths,
    )?;
    transaction.finished = true;
    report.committed = true;
    Ok(report)
}

/// Explicit category selection does not approve or discard a pending description.
pub fn set_category(vault: &Vault, id: &str, category: &str, dry_run: bool) -> Result<EditResult> {
    if !vault.config()?.categories.iter().any(|c| c == category) {
        return Err(Error::new(
            "INVALID_CATEGORY",
            "Choose a category configured in box.toml",
        ));
    }
    let (mut metadata, bytes) = read_snapshot(vault, id)?;
    if metadata.classification.category == category {
        return Ok(EditResult {
            changed: vec![],
            committed: false,
            built: BuildReport {
                dry_run,
                ..Default::default()
            },
        });
    }
    metadata.classification.category = category.into();
    metadata.classification.classified_by = "manual".into();
    apply_checked(
        vault,
        &BTreeMap::from([(id.into(), metadata)]),
        &BTreeMap::from([(id.into(), bytes)]),
        &format!("Set category for {id}: {category}"),
        dry_run,
    )
}

/// Replace the named dimensions of one skill; `None` leaves a dimension alone.
pub fn set_dimensions(
    vault: &Vault,
    id: &str,
    stack: Option<&[String]>,
    stage: Option<&[String]>,
    dry_run: bool,
) -> Result<crate::DimensionsReport> {
    if stack.is_none() && stage.is_none() {
        return Err(Error::new(
            "INVALID_ARGUMENTS",
            "Name at least one dimension to set or clear",
        ));
    }
    let config = vault.config()?;
    let (mut metadata, bytes) = read_snapshot(vault, id)?;
    let before = (
        metadata.classification.stack.clone(),
        metadata.classification.stage.clone(),
    );
    // Who classified the skill is unchanged: this edits two fields, and a
    // pending skill must still be offered to `classify --pending`.
    for (dimension, requested, current) in [
        (Dimension::Stack, stack, &mut metadata.classification.stack),
        (Dimension::Stage, stage, &mut metadata.classification.stage),
    ] {
        if let Some(values) = requested {
            let allowed = dimension.allowed(&config);
            dimensions::validate(dimension, values, allowed)?;
            *current = dimensions::ordered(values, allowed);
        }
    }
    let (stack, stage) = (
        metadata.classification.stack.clone(),
        metadata.classification.stage.clone(),
    );
    let mut report = crate::DimensionsReport {
        id: id.into(),
        stack,
        stage,
        dry_run,
        changed: false,
        committed: false,
    };
    if (&report.stack, &report.stage) == (&before.0, &before.1) {
        return Ok(report);
    }
    let result = apply_checked(
        vault,
        &BTreeMap::from([(id.into(), metadata)]),
        &BTreeMap::from([(id.into(), bytes)]),
        &format!(
            "Set dimensions for {id}: stack=[{}] stage=[{}]",
            report.stack.join(", "),
            report.stage.join(", ")
        ),
        dry_run,
    )?;
    report.changed = !result.changed.is_empty();
    report.committed = result.committed;
    Ok(report)
}

pub fn apply(
    vault: &Vault,
    updates: &BTreeMap<String, SkillMetadata>,
    message: &str,
    dry_run: bool,
) -> Result<EditResult> {
    let mut expected = BTreeMap::new();
    for id in updates.keys() {
        expected.insert(id.clone(), read_snapshot(vault, id)?.1);
    }
    apply_checked(vault, updates, &expected, message, dry_run)
}

pub fn apply_checked(
    vault: &Vault,
    updates: &BTreeMap<String, SkillMetadata>,
    expected: &BTreeMap<String, Vec<u8>>,
    message: &str,
    dry_run: bool,
) -> Result<EditResult> {
    let _state_guard = crate::state::write_guard(&vault.index_path, dry_run)?;
    let vault = Vault::new(
        index::validate_path(&vault.root)?,
        index::validate_path(&vault.index_path)?,
    );
    if updates.is_empty() {
        return Ok(EditResult {
            changed: vec![],
            built: BuildReport {
                dry_run,
                ..BuildReport::default()
            },
            committed: false,
        });
    }
    check_git_trust(&vault)?;
    gitlog::ensure_clean(&vault.root)?;
    index::check(&vault.index_path)?;
    let config = vault.config()?;
    let mut writes = BTreeMap::new();
    for (id, update) in updates {
        let (before, bytes) = read_snapshot(&vault, id)?;
        if expected.get(id) != Some(&bytes) {
            return Err(Error::decision(
                "METADATA_CHANGED",
                format!("Metadata changed while preparing '{id}'; review the new state and retry"),
            ));
        }
        if update.id != *id || update.content_hash != before.content_hash {
            return Err(Error::new(
                "INVALID_METADATA_UPDATE",
                "Metadata edits cannot replace the skill ID or source hash",
            ));
        }
        if update.classification.category != before.classification.category
            && !config.categories.contains(&update.classification.category)
        {
            return Err(Error::decision(
                "CATEGORY_CHANGED",
                "Classification is outside the current category list; retry with the updated configuration",
            ));
        }
        // Like the category, a dimension is checked when it changes; an
        // unrelated rewrite of hand-edited metadata is not blocked by it.
        for (dimension, new, old) in [
            (
                Dimension::Stack,
                &update.classification.stack,
                &before.classification.stack,
            ),
            (
                Dimension::Stage,
                &update.classification.stage,
                &before.classification.stage,
            ),
        ] {
            if new != old {
                dimensions::validate(dimension, new, dimension.allowed(&config))?;
            }
        }
        if let Some(proposal) = &update.proposal {
            crate::agent_runner::validate_proposal(proposal, &(&config).into())?;
        }
        let original =
            frontmatter::read_skill(&vault.root.join("skills").join(id).join("SKILL.md"))?;
        if update.description.original != before.description.original
            && update.description.original != original.description
        {
            return Err(Error::new(
                "INVALID_METADATA_UPDATE",
                "Original description must match SKILL.md",
            ));
        }
        if update.description.active == "improved" {
            if update
                .description
                .approved_at
                .as_ref()
                .is_none_or(|value| value.trim().is_empty())
                || update
                    .description
                    .improved
                    .as_ref()
                    .is_none_or(|value| value.trim().is_empty() || value.chars().count() > 1024)
            {
                return Err(Error::new(
                    "UNAPPROVED_DESCRIPTION",
                    "An improved description needs explicit approval and at most 1024 characters",
                ));
            }
        } else if update.description.active != "original" {
            return Err(Error::new(
                "INVALID_METADATA_UPDATE",
                "Unknown active description",
            ));
        }
        let serialized = patched(&bytes, update)?;
        if serialized != bytes {
            writes.insert(id.clone(), (bytes, serialized));
        }
    }
    let ids = updates.keys().cloned().collect::<Vec<_>>();
    let changed = writes.keys().cloned().collect::<Vec<_>>();
    if dry_run {
        return Ok(EditResult {
            changed,
            built: build::preview(&vault, &ids, updates)?,
            committed: false,
        });
    }
    let mut generated = build::prepare(&vault, &ids, updates)?;
    // Check snapshots again after the potentially slow dist staging step.
    for (id, bytes) in expected {
        if read_snapshot(&vault, id)?.1 != *bytes {
            return Err(Error::decision(
                "METADATA_CHANGED",
                format!("Metadata changed for '{id}'; no changes committed"),
            ));
        }
    }
    let mut metadata = MetadataTransaction::new(&vault.root)?;
    for (id, (before, after)) in &writes {
        metadata.publish(id, before, after)?;
    }
    generated.apply()?;
    metadata.verify()?;
    if !changed.is_empty() {
        metadata.commit_attempted = true;
        gitlog::commit_paths(&vault.root, message, &metadata.paths)?;
    }
    metadata.finished = true;
    let mut built = generated.finish()?;
    match vault
        .records()
        .and_then(|records| index::rebuild_for_vault(&vault.index_path, &vault.root, &records))
    {
        Ok(()) => {}
        Err(error) => built
            .warnings
            .push(format!("Metadata saved; index refresh failed: {error}")),
    }
    Ok(EditResult {
        committed: !changed.is_empty(),
        changed,
        built,
    })
}

// Update only the fields this feature owns; preserve unknown/future metadata,
// including invocation settings and source extensions, at every nesting level.
fn patched(bytes: &[u8], update: &SkillMetadata) -> Result<Vec<u8>> {
    let text =
        std::str::from_utf8(bytes).map_err(|e| Error::new("INVALID_METADATA", e.to_string()))?;
    let mut value: toml::Value = toml::from_str(text)?;
    let root = value
        .as_table_mut()
        .ok_or_else(|| Error::new("INVALID_METADATA", "Metadata must be a TOML table"))?;
    merge_owned(
        root,
        "classification",
        toml::Value::try_from(&update.classification)?,
        &[
            "category",
            "stack",
            "stage",
            "tags",
            "role",
            "when_to_use",
            "when_not",
            "classified_by",
            "classified_from",
            "overlaps_with",
        ],
    )?;
    merge_owned(
        root,
        "description",
        toml::Value::try_from(&update.description)?,
        &["original", "improved", "active", "approved_at"],
    )?;
    merge_owned(
        root,
        "audit",
        toml::Value::try_from(&update.audit)?,
        &["level", "findings", "rules_version", "details"],
    )?;
    match update.proposal_source_changed {
        Some(changed) => {
            root.insert(
                "proposal_source_changed".into(),
                toml::Value::Boolean(changed),
            );
        }
        None => {
            root.remove("proposal_source_changed");
        }
    }
    // `stack` and `stage` of a waiting proposal are stored beside the
    // `[proposal]` table (see `StoredSkillMetadata`).
    root.remove("proposal_dimensions");
    match &update.proposal {
        Some(proposal) => {
            let mut stored = toml::Value::try_from(proposal)?;
            let mut dimensions = toml::map::Map::new();
            if let Some(table) = stored.as_table_mut() {
                for key in ["stack", "stage"] {
                    if let Some(values) = table.remove(key) {
                        dimensions.insert(key.into(), values);
                    }
                }
            }
            root.insert("proposal".into(), stored);
            if !dimensions.is_empty() {
                root.insert("proposal_dimensions".into(), toml::Value::Table(dimensions));
            }
        }
        None => {
            root.remove("proposal");
        }
    }
    Ok(toml::to_string_pretty(&value)?.into_bytes())
}
fn merge_owned(
    root: &mut toml::map::Map<String, toml::Value>,
    name: &str,
    updated: toml::Value,
    keys: &[&str],
) -> Result<()> {
    let old = root
        .entry(name)
        .or_insert_with(|| toml::Value::Table(Default::default()));
    let old = old
        .as_table_mut()
        .ok_or_else(|| Error::new("INVALID_METADATA", format!("'{name}' must be a table")))?;
    let updates = updated
        .as_table()
        .ok_or_else(|| Error::new("INVALID_METADATA_UPDATE", "Expected a table"))?;
    for key in keys {
        match updates.get(*key) {
            Some(value) => {
                old.insert((*key).into(), value.clone());
            }
            None => {
                old.remove(*key);
            }
        }
    }
    Ok(())
}

struct Saved {
    path: PathBuf,
    before: Vec<u8>,
    after: Vec<u8>,
    identity: fs::Metadata,
    permissions: fs::Permissions,
}
struct MetadataTransaction {
    root: PathBuf,
    stage: PathBuf,
    identity: fs::Metadata,
    saved: Vec<Saved>,
    paths: Vec<PathBuf>,
    finished: bool,
    commit_attempted: bool,
}
impl MetadataTransaction {
    fn new(root: &Path) -> Result<Self> {
        Self::new_in(root, &root.join("dist"))
    }
    fn new_in(root: &Path, parent: &Path) -> Result<Self> {
        let parent = index::validate_path(parent)?;
        let stage = loop {
            let path = parent.join(format!(
                ".skillsbox-meta-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => break path,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        };
        Ok(Self {
            root: root.to_path_buf(),
            identity: fs::symlink_metadata(&stage)?,
            stage,
            saved: vec![],
            paths: vec![],
            finished: false,
            commit_attempted: false,
        })
    }
    fn publish(&mut self, id: &str, before: &[u8], after: &[u8]) -> Result<()> {
        let relative = PathBuf::from("meta").join(format!("{id}.toml"));
        let target = index::validate_path(&self.root.join(&relative))?;
        if fs::read(&target)? != before {
            return Err(Error::decision(
                "METADATA_CHANGED",
                format!("Metadata changed for '{id}'"),
            ));
        }
        let permissions = fs::symlink_metadata(&target)?.permissions();
        let temporary = self.stage.join(format!("{id}.toml"));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(after)?;
        file.set_permissions(permissions.clone())?;
        drop(file);
        fs::rename(&temporary, &target)?;
        self.saved.push(Saved {
            identity: fs::symlink_metadata(&target)?,
            path: target,
            before: before.to_vec(),
            after: after.to_vec(),
            permissions,
        });
        self.paths.push(relative);
        Ok(())
    }
    fn verify(&self) -> Result<()> {
        for saved in &self.saved {
            if !owned(saved) {
                return Err(Error::decision(
                    "METADATA_CHANGED",
                    "Metadata was modified concurrently; refusing to commit",
                ));
            }
        }
        Ok(())
    }
}
impl Drop for MetadataTransaction {
    fn drop(&mut self) {
        if !self.finished {
            for saved in self.saved.iter().rev() {
                if owned(saved) {
                    // The installed metadata can be read-only. Restore through
                    // an atomic rename in its writable parent, not fs::write.
                    let temporary = self
                        .stage
                        .join(format!("restore-{}", NEXT.fetch_add(1, Ordering::Relaxed)));
                    let restore = || -> Result<()> {
                        index::validate_path(&self.stage)?;
                        if !same(&self.identity, &fs::symlink_metadata(&self.stage)?) {
                            return Ok(());
                        }
                        let mut file = OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&temporary)?;
                        file.write_all(&saved.before)?;
                        file.set_permissions(saved.permissions.clone())?;
                        drop(file);
                        if owned(saved) {
                            fs::rename(&temporary, &saved.path)?;
                        }
                        Ok(())
                    };
                    let _ = restore();
                }
            }
            if self.commit_attempted {
                let _ = gitlog::unstage_paths(&self.root, &self.paths);
            }
        }
        if index::validate_path(&self.stage).is_ok()
            && fs::symlink_metadata(&self.stage).is_ok_and(|current| same(&self.identity, &current))
        {
            let _ = fs::remove_dir_all(&self.stage);
        }
    }
}
fn owned(saved: &Saved) -> bool {
    index::validate_path(&saved.path).is_ok()
        && fs::symlink_metadata(&saved.path).is_ok_and(|current| same(&saved.identity, &current))
        && fs::read(&saved.path).is_ok_and(|bytes| bytes == saved.after)
}
#[cfg(unix)]
fn same(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev() && a.ino() == b.ino()
}
#[cfg(not(unix))]
fn same(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    a.file_type() == b.file_type()
        && a.created()
            .ok()
            .zip(b.created().ok())
            .is_some_and(|(a, b)| a == b)
}
