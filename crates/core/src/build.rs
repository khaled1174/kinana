//! Build the ignored activation cache without changing source skill payloads.
use crate::{
    BuildItem, BuildReport, Error, Result, SkillMetadata, frontmatter, hash, index, vault::Vault,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use yaml_rust2::{Yaml, YamlEmitter, YamlLoader};

const MANIFEST: &str = ".skillsbox-build.toml";
static NEXT_STAGE: AtomicU64 = AtomicU64::new(0);

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Ownership {
    version: u32,
    generated: BTreeMap<String, String>,
}

#[derive(Clone)]
pub(crate) struct PayloadEntry {
    pub relative: PathBuf,
    pub bytes: Option<Vec<u8>>,
    pub permissions: fs::Permissions,
}

#[derive(Clone)]
pub(crate) struct ActivationPayload {
    pub entries: Vec<PayloadEntry>,
    pub content_hash: String,
}

pub(crate) struct ActivationCache {
    pub payloads: BTreeMap<String, ActivationPayload>,
    pub changed: BTreeMap<String, Option<String>>,
    pub manifest_before: Option<Vec<u8>>,
    pub manifest_after: Option<Vec<u8>>,
}

/// Reuse the normal build's source, approval and ownership checks, without writes.
pub(crate) fn activation_cache(vault: &Vault, ids: &[String]) -> Result<ActivationCache> {
    activation_cache_overrides(vault, ids, &BTreeMap::new(), &BTreeMap::new())
}

pub(crate) fn activation_cache_overrides(
    vault: &Vault,
    ids: &[String],
    overrides: &BTreeMap<String, SkillMetadata>,
    sources: &BTreeMap<String, PathBuf>,
) -> Result<ActivationCache> {
    let plan = match plan_sources(vault, ids, overrides, sources) {
        Err(error) if error.code == "SOURCE_HASH_MISMATCH" => {
            return Err(with_finder_hint(vault, ids, overrides, sources, error));
        }
        other => other?,
    };
    let mut payloads = BTreeMap::new();
    let mut changed = BTreeMap::new();
    for id in ids {
        let payload = if let Some(change) = plan.changes.iter().find(|change| change.id == *id) {
            changed.insert(id.clone(), change.before.clone());
            ActivationPayload {
                content_hash: payload_hash(&change.payload),
                entries: change.payload.clone(),
            }
        } else {
            let payload = source_payload(&plan.dist.join(id))?;
            if plan.generations.get(id) != Some(&payload.generation_hash()) {
                return Err(conflict(
                    "The reusable cache no longer matches the prepared source",
                ));
            }
            payload
        };
        payloads.insert(id.clone(), payload);
    }
    Ok(ActivationCache {
        payloads,
        changed,
        manifest_before: plan.manifest_before,
        manifest_after: plan.manifest_after,
    })
}

/// `use` does not delete Finder files itself; when untracked `.DS_Store`
/// files are the only difference between the skills it was asked for and
/// their recorded hashes, its refusal names them and suggests `kinana build`,
/// which removes them (owner decision 2026-11-17, sixth round). Any other
/// difference keeps the refusal as it was, without the suggestion. Read only.
fn with_finder_hint(
    vault: &Vault,
    ids: &[String],
    overrides: &BTreeMap<String, SkillMetadata>,
    sources: &BTreeMap<String, PathBuf>,
    mut error: Error,
) -> Error {
    let Ok(leftovers) = finder_leftovers(vault) else {
        return error;
    };
    let leftovers = leftovers
        .into_iter()
        .filter(|(id, _)| ids.contains(id))
        .collect::<FinderLeftovers>();
    if leftovers.is_empty() || plan_skipping(vault, ids, overrides, sources, &leftovers).is_err() {
        return error;
    }
    let files = leftover_names(&leftovers)
        .iter()
        .map(|name| crate::names::visible(name))
        .collect::<Vec<_>>();
    error.message.push_str(&format!(
        ". The only difference is Finder .DS_Store file(s) Git does not track: {}. Run kinana build {} to remove them, then try again",
        files.join(", "),
        // Only these skills: a plain `build` covers every skill and stops at
        // any other skill's real difference (review, sixth round, F2).
        leftovers.keys().cloned().collect::<Vec<_>>().join(" ")
    ));
    error
}

impl ActivationPayload {
    pub(crate) fn generation_hash(&self) -> String {
        let mut digest = blake3::Hasher::new();
        digest.update(self.content_hash.as_bytes());
        for entry in &self.entries {
            digest.update(entry.relative.as_os_str().as_encoded_bytes());
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                digest.update(&(entry.permissions.mode() & 0o7777).to_le_bytes());
            }
            #[cfg(not(unix))]
            {
                digest.update(&[entry.permissions.readonly() as u8]);
            }
        }
        format!("blake3:{}", digest.finalize().to_hex())
    }
    pub(crate) fn manual(&self) -> Result<Self> {
        let mut entries = self.entries.clone();
        let skill = entries
            .iter_mut()
            .find(|entry| entry.relative == Path::new("SKILL.md"))
            .and_then(|entry| entry.bytes.as_mut())
            .ok_or_else(|| {
                Error::new("INVALID_FRONTMATTER", "Missing SKILL.md in manual payload")
            })?;
        let text = std::str::from_utf8(skill)
            .map_err(|error| Error::new("INVALID_FRONTMATTER", error.to_string()))?;
        *skill = replace_frontmatter(text, "disable-model-invocation", Yaml::Boolean(true))?
            .into_bytes();
        Ok(Self {
            content_hash: payload_hash(&entries),
            entries,
        })
    }
}

struct Change {
    id: String,
    before: Option<String>,
    after: Option<String>,
    payload: Vec<PayloadEntry>,
    backed_up: bool,
    installed: bool,
    risky: bool,
}

struct Plan {
    dist: PathBuf,
    manifest_before: Option<Vec<u8>>,
    manifest_after: Option<Vec<u8>>,
    observed: BTreeMap<String, Option<String>>,
    generations: BTreeMap<String, String>,
    changes: Vec<Change>,
    report: BuildReport,
}

/// A prepared cache update. Until `finish`, dropping it restores its own changes.
/// Paths and staged bytes are owned, so callers can commit metadata independently.
pub struct BuildTransaction {
    vault: Vault,
    _state_guard: crate::state::WriteGuard,
    plan: Plan,
    stage: Option<PathBuf>,
    stage_identity: Option<fs::Metadata>,
    created_dist: Option<fs::Metadata>,
    manifest_backed_up: bool,
    manifest_installed: bool,
    applied: bool,
    finished: bool,
}

pub fn build(vault: &Vault, ids: &[String], dry_run: bool) -> Result<BuildReport> {
    let overrides = BTreeMap::new();
    if dry_run {
        return preview(vault, ids, &overrides);
    }
    let mut transaction = prepare(vault, ids, &overrides)?;
    transaction.apply()?;
    transaction.finish()
}

/// Untracked Finder files, by skill, that are that skill's only difference.
pub(crate) type FinderLeftovers = BTreeMap<String, BTreeSet<PathBuf>>;

/// `kinana build`. Untracked `.DS_Store` files whose removal makes a skill's
/// source match its recorded hash exactly are removed first and named in the
/// report (owner decision 2026-11-17); the hash definition is unchanged, and
/// any other difference still stops with `SOURCE_HASH_MISMATCH`. Nothing is
/// removed unless the whole build is valid without those files.
pub fn build_command(vault: &Vault, ids: &[String], dry_run: bool) -> Result<BuildReport> {
    let _state_guard = crate::state::write_guard(&vault.index_path, dry_run)?;
    let mut leftovers = finder_leftovers(vault)?;
    if !ids.is_empty() {
        leftovers.retain(|id, _| ids.contains(id));
    }
    let mut plan = plan_skipping(vault, ids, &BTreeMap::new(), &BTreeMap::new(), &leftovers)?;
    private_stages(vault, &plan)?;
    let names = leftover_names(&leftovers);
    let mut report = if dry_run {
        plan.report.dry_run = true;
        plan.report
    } else {
        remove_finder_leftovers(vault, &leftovers)?;
        build(vault, ids, false)?
    };
    report.finder_files_removed = names;
    Ok(report)
}

/// Finder `.DS_Store` files Git does not track whose absence makes their
/// skill's directory hash to the recorded `content_hash`. A tracked
/// `.DS_Store` is imported payload and is never listed.
pub(crate) fn finder_leftovers(vault: &Vault) -> Result<FinderLeftovers> {
    let root = index::validate_path(&vault.root)?;
    let mut candidates = FinderLeftovers::new();
    for path in crate::gitlog::untracked_finder_files(&root)? {
        let mut parts = path.splitn(3, '/');
        let (Some("skills"), Some(id), Some(rest)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if crate::vault::validate_id(id).is_err() {
            continue;
        }
        candidates
            .entry(id.to_owned())
            .or_default()
            .insert(PathBuf::from(rest));
    }
    let mut found = FinderLeftovers::new();
    for (id, files) in candidates {
        let source = index::validate_path(&root.join("skills").join(&id))?;
        let metadata_path = index::validate_path(&root.join("meta").join(format!("{id}.toml")))?;
        let Ok(text) = fs::read_to_string(&metadata_path) else {
            continue;
        };
        let Ok(metadata) = toml::from_str::<SkillMetadata>(&text) else {
            continue;
        };
        // A link or special file anywhere stops here and is reported by the
        // normal checks, which run next.
        if hash::hash_directory_without(&source, &files)
            .ok()
            .as_deref()
            == Some(metadata.content_hash.as_str())
        {
            found.insert(id, files);
        }
    }
    Ok(found)
}

pub(crate) fn leftover_names(leftovers: &FinderLeftovers) -> Vec<String> {
    leftovers
        .iter()
        .flat_map(|(id, files)| {
            files
                .iter()
                .map(move |file| format!("skills/{id}/{}", file.to_string_lossy()))
        })
        .collect()
}

pub(crate) fn remove_finder_leftovers(vault: &Vault, leftovers: &FinderLeftovers) -> Result<()> {
    for (id, files) in leftovers {
        for file in files {
            let path = index::validate_path(&vault.root.join("skills").join(id).join(file))?;
            if fs::symlink_metadata(&path)?.is_file() {
                fs::remove_file(&path)?;
            }
        }
    }
    Ok(())
}

/// The undo pre-check, with Finder leftovers left out of each skill's hash.
pub(crate) fn preview_skipping(vault: &Vault, leftovers: &FinderLeftovers) -> Result<()> {
    let plan = plan_skipping(vault, &[], &BTreeMap::new(), &BTreeMap::new(), leftovers)?;
    private_stages(vault, &plan)
}

fn private_stages(vault: &Vault, plan: &Plan) -> Result<()> {
    let home = crate::activation_risk::local_home();
    for change in plan.changes.iter().filter(|change| change.risky) {
        crate::activation_risk::require_private_stage(
            vault,
            &plan.dist.join(&change.id),
            home.as_deref(),
            None,
        )?;
    }
    Ok(())
}

/// Validate and describe the complete update without creating even staging files.
pub fn preview(
    vault: &Vault,
    ids: &[String],
    overrides: &BTreeMap<String, SkillMetadata>,
) -> Result<BuildReport> {
    let mut plan = plan(vault, ids, overrides)?;
    private_stages(vault, &plan)?;
    plan.report.dry_run = true;
    Ok(plan.report)
}

/// Validate the batch first, then privately stage only payloads that will change.
pub fn prepare(
    vault: &Vault,
    ids: &[String],
    overrides: &BTreeMap<String, SkillMetadata>,
) -> Result<BuildTransaction> {
    let plan = plan(vault, ids, overrides)?;
    private_stages(vault, &plan)?;
    let mut transaction = BuildTransaction {
        vault: vault.clone(),
        _state_guard: crate::state::write_guard(&vault.index_path, false)?,
        plan,
        stage: None,
        stage_identity: None,
        created_dist: None,
        manifest_backed_up: false,
        manifest_installed: false,
        applied: false,
        finished: false,
    };
    if transaction.plan.changes.is_empty() && transaction.plan.manifest_after.is_none() {
        return Ok(transaction);
    }
    let dist = index::validate_path(&transaction.plan.dist)?;
    match fs::symlink_metadata(&dist) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err(conflict("dist must be a directory")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            create_private_directory(&dist)?;
            transaction.created_dist = Some(fs::symlink_metadata(&dist)?);
        }
        Err(error) => return Err(error.into()),
    }
    let stage = create_stage(&dist)?;
    transaction.stage_identity = Some(fs::symlink_metadata(&stage)?);
    transaction.stage = Some(stage.clone());
    transaction.require_actual_private_stage()?;
    create_private_directory(&stage.join("new"))?;
    create_private_directory(&stage.join("old"))?;
    for change in &transaction.plan.changes {
        if change.after.is_none() {
            continue;
        }
        let target = stage.join("new").join(&change.id);
        create_private_directory(&target)?;
        write_payload(&target, &change.payload)?;
        if Some(hash::hash_directory(&target)?) != change.after {
            return Err(Error::new("FILE_CHANGED", "Staged build payload changed"));
        }
    }
    if let Some(bytes) = &transaction.plan.manifest_after {
        write_new(&stage.join("manifest-new"), bytes)?;
    }
    Ok(transaction)
}

impl BuildTransaction {
    fn require_actual_private_stage(&self) -> Result<()> {
        if let Some(stage) = &self.stage
            && self.plan.changes.iter().any(|change| change.risky)
        {
            crate::activation_risk::require_private_stage(
                &self.vault,
                stage,
                crate::activation_risk::local_home().as_deref(),
                None,
            )?;
        }
        Ok(())
    }

    pub fn report(&self) -> &BuildReport {
        &self.plan.report
    }

    /// Swap the validated batch into place. Any error restores completed swaps.
    pub fn apply(&mut self) -> Result<()> {
        if self.finished || self.applied {
            return Err(Error::new(
                "BUILD_TRANSACTION_STATE",
                "Build already applied or finished",
            ));
        }
        let result = self.apply_inner();
        if result.is_err() {
            self.rollback();
        }
        result
    }

    fn apply_inner(&mut self) -> Result<()> {
        private_stages(&self.vault, &self.plan)?;
        self.require_actual_private_stage()?;
        require_optional_directory(&self.plan.dist)?;
        if read_manifest_bytes(&self.plan.dist)? != self.plan.manifest_before {
            return Err(conflict(
                "The dist ownership manifest changed during the build",
            ));
        }
        // Validate every target before the first rename, including the unchanged
        // targets represented only in the ownership manifest.
        for (id, before) in &self.plan.observed {
            require_state(&self.plan.dist.join(id), before.as_deref())?;
        }
        let Some(stage) = self.stage.clone() else {
            self.applied = true;
            return Ok(());
        };
        self.check_stage()?;
        for change in &mut self.plan.changes {
            let target = index::validate_path(&self.plan.dist.join(&change.id))?;
            require_state(&target, change.before.as_deref())?;
            if change.before.is_some() {
                fs::rename(&target, stage.join("old").join(&change.id))?;
                change.backed_up = true;
            }
            if change.after.is_some() {
                fs::rename(stage.join("new").join(&change.id), &target)?;
                change.installed = true;
            }
        }
        if self.plan.manifest_after.is_some() {
            if read_manifest_bytes(&self.plan.dist)? != self.plan.manifest_before {
                return Err(conflict(
                    "The dist ownership manifest changed during the build",
                ));
            }
            let target = index::validate_path(&self.plan.dist.join(MANIFEST))?;
            if self.plan.manifest_before.is_some() {
                fs::rename(&target, stage.join("manifest-old"))?;
                self.manifest_backed_up = true;
            }
            fs::rename(stage.join("manifest-new"), &target)?;
            self.manifest_installed = true;
        }
        self.applied = true;
        Ok(())
    }

    /// Keep the applied cache and discard private backups. This never commits Git.
    pub fn finish(mut self) -> Result<BuildReport> {
        if !self.applied {
            return Err(Error::new(
                "BUILD_TRANSACTION_STATE",
                "Apply the build before finishing",
            ));
        }
        self.finished = true;
        self.cleanup_stage();
        Ok(self.plan.report.clone())
    }

    fn check_stage(&self) -> Result<()> {
        let Some(stage) = &self.stage else {
            return Ok(());
        };
        let path = index::validate_path(stage)?;
        let current = fs::symlink_metadata(path)?;
        if self
            .stage_identity
            .as_ref()
            .is_none_or(|original| !same_entry(original, &current))
        {
            return Err(conflict("Private build staging directory was replaced"));
        }
        Ok(())
    }

    fn rollback(&mut self) {
        if self.check_stage().is_err() {
            return;
        }
        let Some(stage) = self.stage.clone() else {
            return;
        };
        let manifest = self.plan.dist.join(MANIFEST);
        if self.manifest_installed
            && read_manifest_bytes(&self.plan.dist).ok().flatten() == self.plan.manifest_after
            && fs::remove_file(&manifest).is_ok()
        {
            self.manifest_installed = false;
        }
        if self.manifest_backed_up
            && !self.manifest_installed
            && absent(&manifest).unwrap_or(false)
            && fs::rename(stage.join("manifest-old"), &manifest).is_ok()
        {
            self.manifest_backed_up = false;
        }
        for change in self.plan.changes.iter_mut().rev() {
            let target = self.plan.dist.join(&change.id);
            if change.installed
                && require_state(&target, change.after.as_deref()).is_ok()
                && remove_owned_tree(&target).is_ok()
            {
                change.installed = false;
            }
            if change.backed_up
                && !change.installed
                && absent(&target).unwrap_or(false)
                && fs::rename(stage.join("old").join(&change.id), &target).is_ok()
            {
                change.backed_up = false;
            }
        }
        self.applied = false;
    }

    fn cleanup_stage(&mut self) {
        // Keep backups if a concurrent modification prevented safe rollback.
        if !self.finished
            && (self.manifest_backed_up
                || self.manifest_installed
                || self
                    .plan
                    .changes
                    .iter()
                    .any(|change| change.backed_up || change.installed))
        {
            return;
        }
        if self.check_stage().is_ok()
            && let Some(stage) = self.stage.take()
        {
            let _cleanup = remove_owned_tree(&stage);
        }
        if let Some(original) = &self.created_dist
            && let Ok(current) = fs::symlink_metadata(&self.plan.dist)
            && same_entry(original, &current)
        {
            // Only an empty directory is removable; foreign entries survive.
            let _cleanup = fs::remove_dir(&self.plan.dist);
        }
    }
}

impl Drop for BuildTransaction {
    fn drop(&mut self) {
        if !self.finished {
            self.rollback();
        }
        self.cleanup_stage();
    }
}

fn plan(
    vault: &Vault,
    ids: &[String],
    overrides: &BTreeMap<String, SkillMetadata>,
) -> Result<Plan> {
    plan_sources(vault, ids, overrides, &BTreeMap::new())
}

fn plan_sources(
    vault: &Vault,
    ids: &[String],
    overrides: &BTreeMap<String, SkillMetadata>,
    sources: &BTreeMap<String, PathBuf>,
) -> Result<Plan> {
    plan_skipping(vault, ids, overrides, sources, &FinderLeftovers::new())
}

fn plan_skipping(
    vault: &Vault,
    ids: &[String],
    overrides: &BTreeMap<String, SkillMetadata>,
    sources: &BTreeMap<String, PathBuf>,
    leftovers: &FinderLeftovers,
) -> Result<Plan> {
    vault.config()?;
    let root = index::validate_path(&vault.root)?;
    let source_root = require_directory(&root.join("skills"))?;
    let meta_root = require_directory(&root.join("meta"))?;
    let dist = index::validate_path(&root.join("dist"))?;
    require_optional_directory(&dist)?;
    let manifest_before = read_manifest_bytes(&dist)?;
    let mut ownership: Ownership = match &manifest_before {
        Some(bytes) => toml::from_str(
            std::str::from_utf8(bytes)
                .map_err(|error| Error::new("INVALID_BUILD_MANIFEST", error.to_string()))?,
        )?,
        None => Ownership {
            version: 1,
            generated: BTreeMap::new(),
        },
    };
    if ownership.version != 1 {
        return Err(conflict("Unsupported dist ownership manifest version"));
    }
    for (id, digest) in &ownership.generated {
        crate::vault::validate_id(id)?;
        if !valid_hash(digest) {
            return Err(conflict(
                "Invalid generated hash in dist ownership manifest",
            ));
        }
    }
    let selected: BTreeSet<String> = if ids.is_empty() {
        inventory_ids(&source_root, &meta_root)?
    } else {
        ids.iter()
            .map(|id| {
                crate::vault::validate_id(id)?;
                Ok(id.clone())
            })
            .collect::<Result<_>>()?
    };
    let mut report = BuildReport::default();
    let mut changes = Vec::new();
    let mut observed = BTreeMap::new();
    let mut generations = BTreeMap::new();
    for id in &selected {
        let source = require_directory(sources.get(id).unwrap_or(&source_root.join(id)))?;
        let metadata_path = index::validate_path(&meta_root.join(format!("{id}.toml")))?;
        require_file(&metadata_path)?;
        let metadata: SkillMetadata = match overrides.get(id) {
            Some(metadata) => metadata.clone(),
            None => toml::from_str(&fs::read_to_string(&metadata_path)?)?,
        };
        if metadata.id != *id {
            return Err(Error::new(
                "METADATA_ID_MISMATCH",
                format!("Metadata ID does not match {id}"),
            ));
        }
        let mut payload = Vec::new();
        read_payload(&source, Path::new(""), &mut payload)?;
        payload.sort_by(|left, right| left.relative.cmp(&right.relative));
        let on_disk = match leftovers.get(id) {
            Some(leave_out) => {
                payload
                    .retain(|entry| entry.bytes.is_none() || !leave_out.contains(&entry.relative));
                hash::hash_directory_without(&source, leave_out)?
            }
            None => hash::hash_directory(&source)?,
        };
        if payload_hash(&payload) != metadata.content_hash || on_disk != metadata.content_hash {
            return Err(Error::new(
                "SOURCE_HASH_MISMATCH",
                format!("Original skill hash differs from metadata: {id}"),
            ));
        }
        apply_description(&mut payload, &metadata, id)?;
        let payload = ActivationPayload {
            content_hash: payload_hash(&payload),
            entries: payload,
        };
        let target = dist.join(id);
        let before = target_state(&target, ownership.generated.get(id).map(String::as_str))?;
        observed.insert(id.clone(), before.clone());
        let generation = payload.generation_hash();
        let changed = before.as_deref() != Some(payload.content_hash.as_str())
            || (before.is_some() && generation != source_payload(&target)?.generation_hash());
        let digest = payload.content_hash.clone();
        generations.insert(id.clone(), generation);
        if changed {
            let risky = crate::activation_risk::for_payload(id, "skill", &metadata.audit, &payload)
                .is_some();
            changes.push(Change {
                id: id.clone(),
                before,
                after: Some(digest.clone()),
                payload: payload.entries,
                backed_up: false,
                installed: false,
                risky,
            });
        }
        ownership.generated.insert(id.clone(), digest);
        report.items.push(BuildItem {
            id: id.clone(),
            path: target,
            description_source: metadata.description.active.clone(),
            changed,
        });
    }
    if ids.is_empty() {
        let removed: Vec<_> = ownership
            .generated
            .keys()
            .filter(|id| !selected.contains(*id))
            .cloned()
            .collect();
        for id in removed {
            let before = target_state(
                &dist.join(&id),
                ownership.generated.get(&id).map(String::as_str),
            )?;
            observed.insert(id.clone(), before.clone());
            if before.is_some() {
                changes.push(Change {
                    id: id.clone(),
                    before,
                    after: None,
                    payload: Vec::new(),
                    backed_up: false,
                    installed: false,
                    risky: false,
                });
            }
            ownership.generated.remove(&id);
            report.removed.push(id);
        }
    }
    let new_bytes = toml::to_string_pretty(&ownership)?.into_bytes();
    let manifest_after = if manifest_before.as_ref() == Some(&new_bytes)
        || (manifest_before.is_none() && ownership.generated.is_empty())
    {
        None
    } else {
        Some(new_bytes)
    };
    Ok(Plan {
        dist,
        manifest_before,
        manifest_after,
        observed,
        generations,
        changes,
        report,
    })
}

pub(crate) fn described_source_payload(
    path: &Path,
    meta: &SkillMetadata,
) -> Result<ActivationPayload> {
    let mut payload = source_payload(path)?;
    if payload.content_hash != meta.content_hash {
        return Err(Error::new(
            "SOURCE_HASH_MISMATCH",
            "Original skill hash differs from metadata",
        ));
    }
    apply_description(&mut payload.entries, meta, &meta.id)?;
    payload.content_hash = payload_hash(&payload.entries);
    Ok(payload)
}

fn apply_description(
    payload: &mut [PayloadEntry],
    metadata: &SkillMetadata,
    id: &str,
) -> Result<()> {
    let skill = payload
        .iter_mut()
        .find(|entry| entry.relative == Path::new("SKILL.md"))
        .and_then(|entry| entry.bytes.as_mut())
        .ok_or_else(|| {
            Error::new(
                "INVALID_FRONTMATTER",
                format!("Missing regular SKILL.md: {id}"),
            )
        })?;
    let text = std::str::from_utf8(skill)
        .map_err(|error| Error::new("INVALID_FRONTMATTER", error.to_string()))?;
    frontmatter::parse_skill(text)?;
    match metadata.description.active.as_str() {
        "original" => {}
        "improved" => {
            let description = metadata
                .description
                .improved
                .as_deref()
                .filter(|value| !value.trim().is_empty() && value.chars().count() <= 1024)
                .ok_or_else(|| {
                    Error::new(
                        "INVALID_METADATA",
                        format!("Approved description must contain 1–1024 characters: {id}"),
                    )
                })?;
            if metadata
                .description
                .approved_at
                .as_ref()
                .is_none_or(|value| value.trim().is_empty())
            {
                return Err(Error::new(
                    "INVALID_METADATA",
                    format!("Active improved description has no approval: {id}"),
                ));
            }
            *skill = replace_description(text, description)?.into_bytes();
        }
        _ => {
            return Err(Error::new(
                "INVALID_METADATA",
                format!("Unknown active description: {id}"),
            ));
        }
    }
    Ok(())
}

pub(crate) fn source_payload(path: &Path) -> Result<ActivationPayload> {
    let mut entries = Vec::new();
    read_payload(path, Path::new(""), &mut entries)?;
    entries.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(ActivationPayload {
        content_hash: payload_hash(&entries),
        entries,
    })
}

fn inventory_ids(skills: &Path, meta: &Path) -> Result<BTreeSet<String>> {
    let mut ids = BTreeSet::new();
    for entry in fs::read_dir(skills)? {
        let entry = entry?;
        if entry.file_name() == ".DS_Store" {
            continue;
        }
        require_directory(&entry.path())?;
        let id = entry
            .file_name()
            .into_string()
            .map_err(|_| Error::new("INVALID_SKILL_ID", "Skill IDs must be valid Unicode"))?;
        crate::vault::validate_id(&id)?;
        ids.insert(id);
    }
    for entry in fs::read_dir(meta)? {
        let entry = entry?;
        if entry.file_name() == ".DS_Store" {
            continue;
        }
        require_file(&entry.path())?;
        let filename = entry.file_name().into_string().map_err(|_| {
            Error::new(
                "INVALID_METADATA",
                "Metadata filenames must be valid Unicode",
            )
        })?;
        let id = filename
            .strip_suffix(".toml")
            .ok_or_else(|| Error::new("INVALID_METADATA", "Unexpected metadata filename"))?;
        crate::vault::validate_id(id)?;
        if !ids.contains(id) {
            return Err(Error::new(
                "ORPHAN_METADATA",
                format!("Metadata has no source skill: {id}"),
            ));
        }
    }
    Ok(ids)
}

fn read_payload(root: &Path, relative: &Path, entries: &mut Vec<PayloadEntry>) -> Result<()> {
    let directory = require_directory(&root.join(relative))?;
    for child in fs::read_dir(directory)? {
        let child = child?;
        if relative.as_os_str().is_empty() && child.file_name() == ".git" {
            continue;
        }
        let relative = relative.join(child.file_name());
        let path = index::validate_path(&root.join(&relative))?;
        let metadata = fs::symlink_metadata(&path)?;
        let bytes = if metadata.is_file() {
            Some(fs::read(&path)?)
        } else if metadata.is_dir() {
            None
        } else {
            return Err(Error::new(
                "UNSUPPORTED_FILE",
                format!(
                    "Only files and directories can be built: {}",
                    path.display()
                ),
            ));
        };
        entries.push(PayloadEntry {
            relative: relative.clone(),
            bytes,
            permissions: metadata.permissions(),
        });
        if metadata.is_dir() {
            read_payload(root, &relative, entries)?;
        }
    }
    Ok(())
}

fn payload_hash(entries: &[PayloadEntry]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"skillsbox-directory-v1\0");
    for entry in entries {
        hasher.update(if entry.bytes.is_none() { b"d" } else { b"f" });
        let components: Vec<_> = entry.relative.components().collect();
        hasher.update(&(components.len() as u64).to_le_bytes());
        for component in components {
            let bytes = component.as_os_str().as_encoded_bytes();
            hasher.update(&(bytes.len() as u64).to_le_bytes());
            hasher.update(bytes);
        }
        if let Some(bytes) = &entry.bytes {
            hasher.update(&(bytes.len() as u64).to_le_bytes());
            hasher.update(bytes);
        }
    }
    format!("blake3:{}", hasher.finalize().to_hex())
}

fn write_payload(target: &Path, entries: &[PayloadEntry]) -> Result<()> {
    for entry in entries {
        let path = index::validate_path(&target.join(&entry.relative))?;
        match &entry.bytes {
            Some(bytes) => {
                write_new(&path, bytes)?;
                fs::set_permissions(&path, entry.permissions.clone())?;
            }
            None => create_private_directory(&path)?,
        }
    }
    // Set directory permissions last so a read-only source directory is copyable.
    for entry in entries.iter().rev().filter(|entry| entry.bytes.is_none()) {
        fs::set_permissions(target.join(&entry.relative), entry.permissions.clone())?;
    }
    Ok(())
}

/// Canonicalize only YAML. Delimiters, BOM, and all bytes of the body survive.
/// Whether `description` goes into the frontmatter of `text` and reads back
/// unchanged: the step a build applies to an approved description, checked
/// before an automatic approval is committed (0.3, r5).
pub(crate) fn description_fits(text: &str, description: &str) -> bool {
    replace_description(text, description)
        .and_then(|replaced| frontmatter::parse_skill(&replaced))
        .is_ok_and(|manifest| manifest.description == description)
}

fn replace_description(text: &str, description: &str) -> Result<String> {
    replace_frontmatter(text, "description", Yaml::String(description.into()))
}

fn replace_frontmatter(text: &str, key: &str, value: Yaml) -> Result<String> {
    let mut offset = 0;
    let mut opening_end = None;
    let mut closing_start = None;
    let mut crlf = false;
    for (number, line) in text.split_inclusive('\n').enumerate() {
        let content = line.trim_end_matches(['\r', '\n']);
        if number == 0 {
            if content
                .strip_prefix('\u{feff}')
                .unwrap_or(content)
                .trim_end()
                != "---"
            {
                return Err(Error::new(
                    "INVALID_FRONTMATTER",
                    "Missing opening YAML delimiter",
                ));
            }
            crlf = line.ends_with("\r\n");
            opening_end = Some(line.len());
        } else if content.trim_end() == "---" {
            closing_start = Some(offset);
            break;
        }
        offset += line.len();
    }
    let opening_end =
        opening_end.ok_or_else(|| Error::new("INVALID_FRONTMATTER", "Missing frontmatter"))?;
    let closing_start = closing_start
        .ok_or_else(|| Error::new("INVALID_FRONTMATTER", "Missing closing YAML delimiter"))?;
    let mut docs = YamlLoader::load_from_str(&text[opening_end..closing_start])
        .map_err(|error| Error::new("INVALID_FRONTMATTER", error.to_string()))?;
    if docs.len() != 1 {
        return Err(Error::new(
            "INVALID_FRONTMATTER",
            "Frontmatter must contain one mapping",
        ));
    }
    let mapping = match docs.first_mut() {
        Some(Yaml::Hash(mapping)) => mapping,
        _ => {
            return Err(Error::new(
                "INVALID_FRONTMATTER",
                "Frontmatter must be a mapping",
            ));
        }
    };
    mapping.insert(Yaml::String(key.into()), value);
    let mut emitted = String::new();
    YamlEmitter::new(&mut emitted)
        .dump(&docs[0])
        .map_err(|error| Error::new("INVALID_FRONTMATTER", error.to_string()))?;
    let yaml = emitted
        .strip_prefix("---\n")
        .unwrap_or(&emitted)
        .trim_end_matches('\n');
    let newline = if crlf { "\r\n" } else { "\n" };
    let yaml = if crlf {
        yaml.replace('\n', "\r\n")
    } else {
        yaml.to_string()
    };
    Ok(format!(
        "{}{yaml}{newline}{}",
        &text[..opening_end],
        &text[closing_start..]
    ))
}

fn target_state(path: &Path, owned: Option<&str>) -> Result<Option<String>> {
    if absent(path)? {
        return Ok(None);
    }
    let Some(expected) = owned else {
        return Err(conflict(format!(
            "Refusing an unowned dist target: {}",
            path.display()
        )));
    };
    let actual = hash::hash_directory(path)?;
    if actual != expected {
        return Err(conflict(format!(
            "Generated dist target was modified: {}",
            path.display()
        )));
    }
    Ok(Some(actual))
}

fn require_state(path: &Path, expected: Option<&str>) -> Result<()> {
    if target_state(path, expected)?.as_deref() != expected {
        return Err(conflict(format!(
            "Dist target changed during build: {}",
            path.display()
        )));
    }
    Ok(())
}

fn read_manifest_bytes(dist: &Path) -> Result<Option<Vec<u8>>> {
    let target = index::validate_path(&dist.join(MANIFEST))?;
    if absent(&target)? {
        return Ok(None);
    }
    require_file(&target)?;
    Ok(Some(fs::read(target)?))
}

fn absent(path: &Path) -> Result<bool> {
    let path = index::validate_path(path)?;
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error.into()),
    }
}

fn require_directory(path: &Path) -> Result<PathBuf> {
    let path = index::validate_path(path)?;
    if !fs::symlink_metadata(&path)?.is_dir() {
        return Err(conflict(format!(
            "Expected a directory: {}",
            path.display()
        )));
    }
    Ok(path)
}

fn require_optional_directory(path: &Path) -> Result<()> {
    if !absent(path)? {
        require_directory(path)?;
    }
    Ok(())
}

fn require_file(path: &Path) -> Result<()> {
    let path = index::validate_path(path)?;
    if !fs::symlink_metadata(&path)?.is_file() {
        return Err(conflict(format!(
            "Expected a regular file: {}",
            path.display()
        )));
    }
    Ok(())
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let path = index::validate_path(path)?;
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    Ok(())
}

fn create_private_directory(path: &Path) -> Result<()> {
    index::validate_path(path)?;
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    Ok(())
}

/// Source permissions may make a copied directory read-only. Only call this for
/// a private stage or an installed target whose generated hash was just checked.
fn remove_owned_tree(path: &Path) -> Result<()> {
    let path = index::validate_path(path)?;
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_dir() {
        return Err(conflict("Owned build cleanup expected a directory"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            &path,
            fs::Permissions::from_mode(metadata.permissions().mode() | 0o700),
        )?;
    }
    #[cfg(not(unix))]
    {
        let mut permissions = metadata.permissions();
        permissions.set_readonly(false);
        fs::set_permissions(&path, permissions)?;
    }
    for child in fs::read_dir(&path)? {
        let child = child?;
        let child_path = index::validate_path(&child.path())?;
        let metadata = fs::symlink_metadata(&child_path)?;
        if metadata.is_dir() {
            remove_owned_tree(&child_path)?;
        } else if metadata.is_file() {
            #[cfg(not(unix))]
            {
                let mut permissions = metadata.permissions();
                permissions.set_readonly(false);
                fs::set_permissions(&child_path, permissions)?;
            }
            fs::remove_file(child_path)?;
        } else {
            return Err(conflict("Unexpected entry in owned build cleanup"));
        }
    }
    fs::remove_dir(path)?;
    Ok(())
}

fn create_stage(dist: &Path) -> Result<PathBuf> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| Error::new("IO_ERROR", error.to_string()))?
        .as_nanos();
    for _ in 0..64 {
        let path = dist.join(format!(
            ".skillsbox-stage-{}-{stamp}-{}",
            std::process::id(),
            NEXT_STAGE.fetch_add(1, Ordering::Relaxed)
        ));
        match create_private_directory(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.code == "IO_ERROR" && fs::symlink_metadata(&path).is_ok() => {
                continue;
            }
            Err(error) => return Err(error),
        }
    }
    Err(conflict(
        "Unable to allocate a private build staging directory",
    ))
}

fn valid_hash(value: &str) -> bool {
    value
        .strip_prefix("blake3:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn conflict(message: impl Into<String>) -> Error {
    Error::decision("BUILD_CONFLICT", message)
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
            .is_some_and(|(left, right)| left == right)
}

pub(crate) struct CacheRemoval {
    pub writes: Vec<crate::activation_transaction::PlannedWrite>,
    /// Vault-relative generated copies that the plan deletes.
    pub removed: Vec<String>,
}

/// Plan deletion of one skill's generated copies (automatic and manual) and of
/// their ownership records. Only a copy whose hash matches its record is
/// planned; a modified or unowned copy is refused. Nothing is written here.
pub(crate) fn plan_removal(vault: &Vault, id: &str) -> Result<CacheRemoval> {
    use crate::activation_transaction::{Data, PlannedWrite, Snapshot};
    crate::vault::validate_id(id)?;
    let dist = index::validate_path(&vault.root.join("dist"))?;
    let mut plan = CacheRemoval {
        writes: Vec::new(),
        removed: Vec::new(),
    };
    if absent(&dist)? {
        return Ok(plan);
    }
    require_directory(&dist)?;
    for (directory, manifest, label) in [
        (dist.clone(), MANIFEST, format!("dist/{id}")),
        (
            dist.join(".manual"),
            ".skillsbox-manual.toml",
            format!("dist/.manual/{id}"),
        ),
    ] {
        if absent(&directory)? {
            continue;
        }
        require_directory(&directory)?;
        let manifest_path = index::validate_path(&directory.join(manifest))?;
        let manifest_before = Snapshot::read(&manifest_path)?;
        let mut ownership = match &manifest_before {
            Some(snapshot) => {
                let bytes = snapshot
                    .bytes()
                    .ok_or_else(|| conflict("Generated cache manifest must be a regular file"))?;
                toml::from_str::<Ownership>(
                    std::str::from_utf8(bytes)
                        .map_err(|error| Error::new("INVALID_BUILD_MANIFEST", error.to_string()))?,
                )?
            }
            None => Ownership {
                version: 1,
                generated: BTreeMap::new(),
            },
        };
        if ownership.version != 1 {
            return Err(conflict("Unsupported generated cache manifest version"));
        }
        let recorded = ownership.generated.remove(id);
        let target = directory.join(id);
        if target_state(&target, recorded.as_deref())?.is_some() {
            let before = Snapshot::read(&target)?;
            if before.as_ref().and_then(Snapshot::digest) != recorded.as_deref() {
                return Err(conflict(format!(
                    "Generated copy changed during planning: {}",
                    target.display()
                )));
            }
            plan.writes.push(PlannedWrite {
                target,
                stage_parent: dist.clone(),
                before,
                data: Data::Remove,
            });
            plan.removed.push(label);
        }
        if recorded.is_some() {
            let mode = manifest_before
                .as_ref()
                .and_then(Snapshot::file_mode)
                .unwrap_or(0o600);
            plan.writes.push(PlannedWrite {
                target: manifest_path,
                stage_parent: dist.clone(),
                before: manifest_before,
                data: Data::FileWithMode(toml::to_string_pretty(&ownership)?.into_bytes(), mode),
            });
        }
    }
    Ok(plan)
}

/// Validate every recorded manual cache before Git undo writes anything. The
/// cache manifest remains an ownership record, never a caller-supplied path.
pub(crate) fn manual_cache_ids(vault: &Vault) -> Result<Vec<String>> {
    let root = index::validate_path(&vault.root.join("dist/.manual"))?;
    let path = root.join(".skillsbox-manual.toml");
    let Some(snapshot) = crate::activation_transaction::Snapshot::read(&path)? else {
        if root.is_dir() && fs::read_dir(&root)?.next().is_some() {
            return Err(conflict(
                "Manual cache has no ownership manifest; preserved",
            ));
        }
        return Ok(vec![]);
    };
    let bytes = snapshot
        .bytes()
        .ok_or_else(|| conflict("Manual manifest must be a file"))?;
    let ownership: Ownership =
        toml::from_str(std::str::from_utf8(bytes).map_err(|e| conflict(e.to_string()))?)?;
    if ownership.version != 1 {
        return Err(conflict("Unsupported manual manifest version"));
    }
    for (id, digest) in &ownership.generated {
        crate::vault::validate_id(id)?;
        if !valid_hash(digest) {
            return Err(conflict("Invalid manual cache ownership hash"));
        }
        target_state(&root.join(id), Some(digest))?;
    }
    Ok(ownership.generated.into_keys().collect())
}

/// Rebuild only changed, already-owned manual copies after a successful undo.
/// Project nodes and their recorded generations remain untouched.
pub(crate) fn refresh_manual_cache(vault: &Vault, ids: &[String]) -> Result<()> {
    use crate::activation_transaction::{self as tx, ActivationTransaction, Snapshot};
    let known = vault
        .records()?
        .into_iter()
        .map(|r| r.id)
        .collect::<BTreeSet<_>>();
    let dist = vault.root.join("dist");
    for id in ids.iter().filter(|id| known.contains(*id)) {
        let cache = activation_cache(vault, std::slice::from_ref(id))?;
        let mut writes = vec![];
        crate::source_updates::refresh_manual(&dist, id, &cache.payloads[id], &mut writes)?;
        if writes.is_empty() {
            continue;
        }
        let guarded = crate::activation_risk::guard_cache_writes(
            vault,
            &writes,
            crate::activation_risk::level(vault, id) == "risky",
        )?;
        let mut observations = vec![];
        for path in [
            vault.root.join("skills").join(id),
            vault.root.join("meta").join(format!("{id}.toml")),
            dist.join(id),
        ] {
            observations.push((path.clone(), Snapshot::read(&path)?));
        }
        let anchors = [vault.root.clone(), dist.clone(), dist.join(".manual")]
            .into_iter()
            .map(|p| fs::symlink_metadata(&p).map(|m| (p, m)))
            .collect::<std::io::Result<Vec<_>>>()?;
        tx::preflight(&writes, &observations, &anchors)?;
        let check_stage = |target: &Path, stage: &Path| {
            crate::activation_risk::check_cache_stage(vault, &guarded, target, stage)
        };
        let mut transaction =
            ActivationTransaction::prepare_guarded(writes, observations, anchors, &check_stage)?;
        if let Err(error) = crate::activation_risk::recheck_cache_writes(vault, &guarded)
            .and_then(|()| transaction.verify_staging(&check_stage))
            .and_then(|()| transaction.apply())
            .and_then(|_| transaction.verify())
        {
            return Err(transaction.abort(error));
        }
        let warnings = transaction.finish();
        if !warnings.is_empty() {
            return Err(conflict(warnings.join("; ")));
        }
    }
    Ok(())
}
