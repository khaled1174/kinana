//! Explicit, reviewed GitHub updates. No background checks or project writes.
use crate::activation_transaction::{
    self as tx, ActivationTransaction, Data, PlannedWrite, Snapshot,
};
use crate::{
    Audit, Error, Result, SkillMetadata, SourceInfo, audit, build, edit, frontmatter, gitlog, hash,
    import::ImportControl, index, layout, sources, vault::Vault,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Serialize)]
pub struct SourceStatus {
    pub id: String,
    pub status: String,
    pub reason_code: String,
    pub repository: Option<String>,
    pub reference: Option<String>,
    pub tracking_kind: Option<String>,
    pub old_commit: Option<String>,
    pub new_commit: Option<String>,
    pub content_changed: Option<bool>,
    pub error: Option<Error>,
}
#[derive(Debug, Serialize)]
pub struct OutdatedReport {
    pub items: Vec<SourceStatus>,
    pub groups_fetched: usize,
    pub complete: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileInfo {
    pub kind: String,
    pub mode: u32,
    pub bytes: u64,
    pub content_hash: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: PathBuf,
    pub change: String,
    pub old: Option<FileInfo>,
    pub new: Option<FileInfo>,
    pub unified_diff: Option<String>,
    pub truncated: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateWarning {
    pub reason_code: String,
    pub old_description: Option<String>,
    pub new_description: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateReport {
    pub id: String,
    pub repository: String,
    pub reference: String,
    pub tracking_kind: String,
    pub old_commit: String,
    pub new_commit: String,
    pub status: String,
    pub old_content_hash: String,
    pub new_content_hash: String,
    pub files: Vec<FileDiff>,
    pub old_audit: Audit,
    pub new_audit: Audit,
    pub warnings: Vec<UpdateWarning>,
    pub preview_token: String,
    pub dry_run: bool,
    pub requires_confirmation: bool,
    pub changed: bool,
    pub committed: bool,
    pub diff_byte_limit: usize,
    pub omitted_files: usize,
}
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateOptions {
    pub id: String,
    #[serde(default)]
    pub reference: Option<String>,
    #[serde(default)]
    pub confirm: Option<String>,
    #[serde(default)]
    pub allow_risky: bool,
    #[serde(default)]
    pub dry_run: bool,
}

fn valid_commit(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn normalized_source(
    source: &SourceInfo,
    explicit: Option<&str>,
) -> Result<(String, String, String)> {
    if source.kind != "github" {
        return Err(Error::decision(
            "UNSUPPORTED_SOURCE",
            "Only imported GitHub skills support source updates",
        ));
    }
    let url = source
        .url
        .clone()
        .ok_or_else(|| Error::new("INVALID_SOURCE_URL", "Missing GitHub repository"))?;
    let url = sources::canonical_github_repository(&url)?;
    let (reference, kind) = if let Some(reference) = explicit {
        if let Some(r) = reference.strip_prefix("tag:") {
            (r.into(), "tag".into())
        } else if let Some(r) = reference.strip_prefix("commit:") {
            (r.into(), "commit".into())
        } else if reference == "default" {
            ("HEAD".into(), "default".into())
        } else {
            (
                reference
                    .strip_prefix("branch:")
                    .unwrap_or(reference)
                    .into(),
                "branch".into(),
            )
        }
    } else {
        (
            source.tracking_ref.clone().ok_or_else(|| {
                Error::decision(
                    "SOURCE_TRACKING_REQUIRED",
                    "Choose --ref before updating an older source",
                )
            })?,
            source.tracking_kind.clone().ok_or_else(|| {
                Error::decision("SOURCE_TRACKING_REQUIRED", "Missing tracking kind")
            })?,
        )
    };
    Ok((url, reference, kind))
}
fn skill_path(checkout: &sources::PreparedSource, subpath: Option<&str>) -> Result<PathBuf> {
    let mut root = checkout.root.clone();
    if let Some(subpath) = subpath {
        if subpath.is_empty()
            || subpath.split('/').any(|p| {
                p.is_empty()
                    || p == "."
                    || p == ".."
                    || p.eq_ignore_ascii_case(".git")
                    || p.contains('\\')
            })
        {
            return Err(Error::new(
                "INVALID_SOURCE_SUBPATH",
                "Unsafe source skill subpath",
            ));
        }
        root.push(subpath);
    }
    let path = hash::checked_absolute_path(&root).map_err(|_| {
        Error::new(
            "SOURCE_PATH_MISSING",
            "Source skill path is missing or unsafe",
        )
    })?;
    if !fs::symlink_metadata(&path)?.is_dir() {
        return Err(Error::new(
            "SOURCE_PATH_MISSING",
            "Source skill path is not a directory",
        ));
    }
    Ok(path)
}
fn mode(permissions: &fs::Permissions) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.mode() & 0o7777
    }
    #[cfg(not(unix))]
    {
        if permissions.readonly() { 0o444 } else { 0o644 }
    }
}
fn info(entry: &build::PayloadEntry) -> FileInfo {
    FileInfo {
        kind: if entry.bytes.is_some() {
            "file"
        } else {
            "directory"
        }
        .into(),
        mode: mode(&entry.permissions),
        bytes: entry.bytes.as_ref().map_or(0, |b| b.len() as u64),
        content_hash: entry
            .bytes
            .as_ref()
            .map(|b| format!("blake3:{}", blake3::hash(b).to_hex())),
    }
}
fn changed_payload(old: &build::ActivationPayload, new: &build::ActivationPayload) -> bool {
    let nodes = |p: &build::ActivationPayload| {
        p.entries
            .iter()
            .map(|e| (e.relative.clone(), info(e)))
            .collect::<BTreeMap<_, _>>()
    };
    nodes(old) != nodes(new)
}

/// Fetch each canonical repository/ref exactly once; all skills share that checkout.
pub fn outdated(vault: &Vault, control: Option<&ImportControl>) -> Result<OutdatedReport> {
    vault.config()?;
    let records = vault.records()?;
    let mut report = OutdatedReport {
        items: vec![],
        groups_fetched: 0,
        complete: true,
    };
    let mut groups: BTreeMap<(String, String, String), Vec<usize>> = BTreeMap::new();
    for record in &records {
        let source = &record.source;
        let mut item = SourceStatus {
            id: record.id.clone(),
            status: String::new(),
            reason_code: String::new(),
            repository: source.url.clone(),
            reference: source.tracking_ref.clone(),
            tracking_kind: source.tracking_kind.clone(),
            old_commit: source.commit.clone(),
            new_commit: None,
            content_changed: None,
            error: None,
        };
        if source.kind == "github" && !source.commit.as_deref().is_some_and(valid_commit) {
            item.status = "error".into();
            item.reason_code = "INVALID_SOURCE_COMMIT".into();
            item.error = Some(Error::new(
                "INVALID_SOURCE_COMMIT",
                "Missing or invalid recorded source commit",
            ));
            report.complete = false;
            report.items.push(item);
            continue;
        }
        match normalized_source(source, None) {
            Ok(key) => {
                groups.entry(key).or_default().push(report.items.len());
            }
            Err(error) => {
                item.status = if error.code == "UNSUPPORTED_SOURCE" {
                    "unsupported_source"
                } else if error.code == "SOURCE_TRACKING_REQUIRED" {
                    "tracking_required"
                } else {
                    "error"
                }
                .into();
                item.reason_code = error.code.clone();
                if item.status == "error" {
                    report.complete = false;
                    item.error = Some(error);
                }
            }
        }
        report.items.push(item);
    }
    for ((url, reference, kind), members) in groups {
        if let Some(c) = control {
            c.stage("fetching")?;
        }
        report.groups_fetched += 1;
        let checkout = sources::checkout_github_ref(&url, &reference, &kind, None, control);
        for member in members {
            if let Some(c) = control {
                c.check()?;
            }
            let record = &records[member];
            let item = &mut report.items[member];
            let comparison = (|| -> Result<(String, bool)> {
                let prepared = checkout.as_ref().map_err(|error| {
                    let mut copy = Error::new(&error.code, &error.message);
                    copy.reason_code = error.reason_code.clone();
                    copy.needs_decision = error.needs_decision;
                    copy
                })?;
                let path = skill_path(prepared, record.source.subpath.as_deref())?;
                let new = build::source_payload(&path)?;
                frontmatter::read_skill(&path.join("SKILL.md"))?;
                let old = build::source_payload(&vault.root.join("skills").join(&record.id))?;
                if old.content_hash != record.content_hash {
                    return Err(Error::decision(
                        "SOURCE_HASH_MISMATCH",
                        "Local original differs from its recorded hash",
                    ));
                }
                Ok((
                    prepared.provenance.commit.clone().expect("resolved commit"),
                    changed_payload(&old, &new),
                ))
            })();
            match comparison {
                Ok((commit, changed)) => {
                    item.status = if changed {
                        "outdated"
                    } else if item.old_commit.as_ref() != Some(&commit) {
                        "source_advanced"
                    } else {
                        "unchanged"
                    }
                    .into();
                    item.reason_code = match item.status.as_str() {
                        "source_advanced" => "SOURCE_ADVANCED",
                        "outdated" => "SOURCE_OUTDATED",
                        _ => "SOURCE_UNCHANGED",
                    }
                    .into();
                    item.new_commit = Some(commit);
                    item.content_changed = Some(changed);
                }
                Err(error) => {
                    if error.code == "CANCELLED" {
                        return Err(error);
                    }
                    item.status = "error".into();
                    item.reason_code = error.reason_code.clone();
                    item.error = Some(error);
                    report.complete = false;
                }
            }
        }
    }
    Ok(report)
}

const DIFF_LIMIT: usize = 512 * 1024;
const FILE_LIMIT: usize = 64 * 1024;
fn diff_files(
    old: &build::ActivationPayload,
    new: &build::ActivationPayload,
) -> (Vec<FileDiff>, usize) {
    let old = old
        .entries
        .iter()
        .map(|e| (&e.relative, e))
        .collect::<BTreeMap<_, _>>();
    let new = new
        .entries
        .iter()
        .map(|e| (&e.relative, e))
        .collect::<BTreeMap<_, _>>();
    let keys = old
        .keys()
        .chain(new.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    let mut result = vec![];
    let mut remaining = DIFF_LIMIT;
    for path in keys {
        let a = old.get(path).copied();
        let b = new.get(path).copied();
        let ai = a.map(info);
        let bi = b.map(info);
        if ai == bi {
            continue;
        }
        let before = a.and_then(|e| e.bytes.as_deref()).unwrap_or_default();
        let after = b.and_then(|e| e.bytes.as_deref()).unwrap_or_default();
        let textual = !before.contains(&0)
            && !after.contains(&0)
            && std::str::from_utf8(before).is_ok()
            && std::str::from_utf8(after).is_ok();
        let mut truncated = false;
        let mut unified_diff = None;
        if textual && (before != after) {
            if before.len() + after.len() > FILE_LIMIT
                || before.len() + after.len() + 1024 > remaining
            {
                truncated = true;
            } else {
                let aa = String::from_utf8_lossy(before);
                let bb = String::from_utf8_lossy(after);
                let mut diff = format!(
                    "--- a/{}\n+++ b/{}\n@@ -{},{} +{},{} @@\n",
                    path.display(),
                    path.display(),
                    if aa.is_empty() { 0 } else { 1 },
                    aa.lines().count(),
                    if bb.is_empty() { 0 } else { 1 },
                    bb.lines().count()
                );
                for (prefix, text) in [('-', aa.as_ref()), ('+', bb.as_ref())] {
                    for line in text.split_inclusive('\n') {
                        diff.push(prefix);
                        diff.push_str(line);
                        if !line.ends_with('\n') {
                            diff.push_str("\n\\ No newline at end of file\n");
                        }
                    }
                }
                if diff.len() > remaining {
                    truncated = true;
                } else {
                    remaining -= diff.len();
                    unified_diff = Some(diff);
                }
            }
        }
        result.push(FileDiff {
            path: path.clone(),
            change: if a.is_none() {
                "added"
            } else if b.is_none() {
                "deleted"
            } else {
                "modified"
            }
            .into(),
            old: ai,
            new: bi,
            unified_diff,
            truncated,
        });
    }
    let omitted = result.iter().filter(|d| d.truncated).count();
    (result, omitted)
}

fn patched_meta(bytes: &[u8], updated: &SkillMetadata, source_changed: bool) -> Result<Vec<u8>> {
    let text =
        std::str::from_utf8(bytes).map_err(|e| Error::new("INVALID_METADATA", e.to_string()))?;
    let mut value: toml::Value = toml::from_str(text)?;
    let table = value
        .as_table_mut()
        .ok_or_else(|| Error::new("INVALID_METADATA", "Metadata must be a table"))?;
    for (key, val) in [
        (
            "content_hash",
            toml::Value::String(updated.content_hash.clone()),
        ),
        (
            "empty_directories",
            toml::Value::try_from(&updated.empty_directories)?,
        ),
    ] {
        table.insert(key.into(), val);
    }
    let source = table
        .entry("source")
        .or_insert_with(|| toml::Value::Table(Default::default()))
        .as_table_mut()
        .ok_or_else(|| Error::new("INVALID_METADATA", "source must be a table"))?;
    for (key, val) in [
        ("commit", &updated.source.commit),
        ("tracking_ref", &updated.source.tracking_ref),
        ("tracking_kind", &updated.source.tracking_kind),
    ] {
        if let Some(val) = val {
            source.insert(key.into(), toml::Value::String(val.clone()));
        }
    }
    let description = table
        .entry("description")
        .or_insert_with(|| toml::Value::Table(Default::default()))
        .as_table_mut()
        .ok_or_else(|| Error::new("INVALID_METADATA", "description must be a table"))?;
    description.insert(
        "original".into(),
        toml::Value::String(updated.description.original.clone()),
    );
    let audit = table
        .entry("audit")
        .or_insert_with(|| toml::Value::Table(Default::default()))
        .as_table_mut()
        .ok_or_else(|| Error::new("INVALID_METADATA", "audit must be a table"))?;
    let replacement = toml::Value::try_from(&updated.audit)?;
    for (key, val) in replacement.as_table().expect("audit table") {
        audit.insert(key.clone(), val.clone());
    }
    if source_changed && updated.proposal.is_some() {
        table.insert("proposal_source_changed".into(), toml::Value::Boolean(true));
    }
    Ok(toml::to_string_pretty(&value)?.into_bytes())
}

pub fn update(
    vault: &Vault,
    options: &UpdateOptions,
    control: Option<&ImportControl>,
) -> Result<UpdateReport> {
    let _state_guard = crate::state::write_guard(
        &vault.index_path,
        options.dry_run || options.confirm.is_none(),
    )?;
    let vault = Vault::new(
        index::validate_path(&vault.root)?,
        index::validate_path(&vault.index_path)?,
    );
    let vault = &vault;
    crate::vault::validate_id(&options.id)?;
    vault.config()?;
    let (old_meta, meta_bytes) = edit::read_snapshot(vault, &options.id)?;
    let (url, reference, kind) = normalized_source(&old_meta.source, options.reference.as_deref())?;
    let old_commit = old_meta
        .source
        .commit
        .clone()
        .filter(|s| valid_commit(s))
        .ok_or_else(|| Error::new("INVALID_SOURCE_COMMIT", "Missing recorded source commit"))?;
    let pinned = options
        .confirm
        .as_deref()
        .map(|token| token.split('.').collect::<Vec<_>>())
        .map(|pieces| {
            if pieces.len() == 3
                && pieces[0] == "u1"
                && valid_commit(pieces[1])
                && pieces[2].len() == 64
                && pieces[2].bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                Ok(pieces[1])
            } else {
                Err(Error::decision(
                    "SOURCE_PREVIEW_STALE",
                    "Invalid or stale source preview token",
                ))
            }
        })
        .transpose()?;
    if let Some(c) = control {
        c.stage("fetching")?;
    }
    let checkout = sources::checkout_github_ref(&url, &reference, &kind, pinned, control)?;
    let candidate = skill_path(&checkout, old_meta.source.subpath.as_deref())?;
    let original = vault.root.join("skills").join(&options.id);
    let old_payload = build::source_payload(&original)?;
    if old_payload.content_hash != old_meta.content_hash {
        return Err(Error::decision(
            "SOURCE_HASH_MISMATCH",
            "Local original differs from its recorded hash",
        ));
    }
    let payload = build::source_payload(&candidate)?;
    let old_manifest = frontmatter::read_skill(&original.join("SKILL.md"))?;
    let manifest = frontmatter::read_skill(&candidate.join("SKILL.md"))?;
    if old_manifest.name != manifest.name {
        return Err(Error::decision(
            "SOURCE_NAME_CHANGED",
            "Source changed the original skill name; update cannot rename activation targets",
        ));
    }
    let content_changed = changed_payload(&old_payload, &payload);
    let mut updated = old_meta.clone();
    if content_changed && updated.proposal.is_some() {
        updated.proposal_source_changed = Some(true);
    }
    updated.source.commit = checkout.provenance.commit.clone();
    updated.source.tracking_ref = Some(reference.clone());
    updated.source.tracking_kind = Some(kind.clone());
    updated.content_hash = payload.content_hash.clone();
    updated.empty_directories = layout::empty_directories(&candidate)?;
    updated.audit = audit::audit_skill(&candidate)?;
    updated.description.original = manifest.description.clone();
    let new_commit = updated.source.commit.clone().expect("resolved commit");
    let changed = content_changed
        || old_meta.source.commit != updated.source.commit
        || old_meta.source.tracking_ref != updated.source.tracking_ref
        || old_meta.source.tracking_kind != updated.source.tracking_kind;
    let (files, omitted_files) = diff_files(&old_payload, &payload);
    let mut warnings = vec![];
    if old_meta.description.original != manifest.description
        && old_meta.description.active == "improved"
    {
        warnings.push(UpdateWarning {
            reason_code: "SOURCE_DESCRIPTION_CHANGED_WITH_APPROVAL".into(),
            old_description: Some(old_meta.description.original.clone()),
            new_description: Some(manifest.description.clone()),
        });
    }
    if content_changed && old_meta.proposal.is_some() {
        warnings.push(UpdateWarning {
            reason_code: "SOURCE_CHANGED_WITH_PENDING_PROPOSAL".into(),
            old_description: None,
            new_description: None,
        });
    }
    let mut report = UpdateReport {
        id: options.id.clone(),
        repository: url,
        reference,
        tracking_kind: kind,
        old_commit,
        new_commit: new_commit.clone(),
        status: if content_changed {
            "outdated"
        } else if changed {
            "source_advanced"
        } else {
            "unchanged"
        }
        .into(),
        old_content_hash: old_meta.content_hash.clone(),
        new_content_hash: payload.content_hash.clone(),
        files,
        old_audit: old_meta.audit.clone(),
        new_audit: updated.audit.clone(),
        warnings,
        preview_token: String::new(),
        dry_run: options.dry_run,
        requires_confirmation: changed,
        changed,
        committed: false,
        diff_byte_limit: DIFF_LIMIT,
        omitted_files,
    };
    let dist = vault.root.join("dist");
    let meta_path = vault.root.join("meta").join(format!("{}.toml", options.id));
    let mut observations = vec![];
    for path in [
        original.clone(),
        meta_path.clone(),
        dist.join(&options.id),
        dist.join(".skillsbox-build.toml"),
        dist.join(".manual").join(&options.id),
        dist.join(".manual/.skillsbox-manual.toml"),
        vault.root.join("box.toml"),
        vault.index_path.clone(),
    ] {
        observations.push((path.clone(), Snapshot::read(&path)?));
    }
    let source_roots = vault
        .records()?
        .into_iter()
        .flat_map(|r| r.source.found_in)
        .collect::<Vec<_>>();
    gitlog::reject_source_executable(&vault.root, &source_roots)?;
    gitlog::ensure_clean(&vault.root)?;
    index::check(&vault.index_path)?;
    let head = gitlog::head(&vault.root)?;
    let mut digest = blake3::Hasher::new();
    // Runtime response flags do not participate in the review token.
    let canonical = (
        &report.id,
        &report.repository,
        &report.reference,
        &report.tracking_kind,
        &report.old_commit,
        &report.new_commit,
        &report.old_content_hash,
        &report.new_content_hash,
        &report.files,
        &report.new_audit,
        &report.warnings,
    );
    digest.update(
        &serde_json::to_vec(&canonical).map_err(|e| Error::new("JSON_ERROR", e.to_string()))?,
    );
    digest.update(head.as_bytes());
    for (path, snapshot) in &observations {
        digest.update(path.as_os_str().as_encoded_bytes());
        if let Some(snapshot) = snapshot {
            digest.update(snapshot.ownership().as_bytes());
            // Bind contents too: restoring size/mtime must not make a changed
            // metadata/config/cache file look like the reviewed state.
            if let Some(bytes) = snapshot.bytes() {
                digest.update(bytes);
            }
            if let Some(contents) = snapshot.digest() {
                digest.update(contents.as_bytes());
            }
        }
    }
    report.preview_token = format!("u1.{new_commit}.{}", digest.finalize().to_hex());
    if !changed {
        return Ok(report);
    }
    // Preflight generated outputs for preview too. Use an owned materialized copy
    // so a root-level GitHub skill never includes the private .git directory.
    let prepared = checkout
        .root
        .parent()
        .expect("owned scratch")
        .join("skill-payload");
    materialize(&prepared, &payload)?;
    let ids = vec![options.id.clone()];
    let cache = build::activation_cache_overrides(
        vault,
        &ids,
        &BTreeMap::from([(options.id.clone(), updated.clone())]),
        &BTreeMap::from([(options.id.clone(), prepared)]),
    )?;
    let mut writes = vec![];
    if content_changed {
        writes.push(write_tree(original.clone(), &dist, payload.clone())?);
    }
    let after_meta = patched_meta(&meta_bytes, &updated, content_changed)?;
    writes.push(PlannedWrite {
        target: meta_path.clone(),
        stage_parent: dist.clone(),
        before: Snapshot::read(&meta_path)?,
        data: Data::File(after_meta),
    });
    // Force mode changes as well as content changes into the generated payload.
    let generated = &cache.payloads[&options.id];
    if content_changed || cache.changed.contains_key(&options.id) {
        writes.push(write_tree(
            dist.join(&options.id),
            &dist,
            generated.clone(),
        )?);
    }
    if let Some(bytes) = cache.manifest_after {
        let path = dist.join(".skillsbox-build.toml");
        writes.push(PlannedWrite {
            before: Snapshot::read(&path)?,
            target: path,
            stage_parent: dist.clone(),
            data: Data::File(bytes),
        });
    }
    if content_changed || cache.changed.contains_key(&options.id) {
        refresh_manual(&dist, &options.id, generated, &mut writes)?;
    }
    let mut anchors = vec![];
    for path in [
        &vault.root,
        &vault.root.join("skills"),
        &vault.root.join("meta"),
        &dist,
        vault.index_path.parent().unwrap(),
    ] {
        anchors.push((path.to_path_buf(), fs::symlink_metadata(path)?));
    }
    tx::preflight(&writes, &observations, &anchors)?;
    if options.dry_run || options.confirm.is_none() {
        return Ok(report);
    }
    if options.confirm.as_deref() != Some(report.preview_token.as_str()) {
        return Err(Error::decision(
            "SOURCE_PREVIEW_STALE",
            "Source, metadata, audit or generated targets changed after preview",
        ));
    }
    if updated.audit.level == "risky" && !options.allow_risky {
        return Err(Error::decision(
            "SOURCE_RISK_APPROVAL_REQUIRED",
            "Review the new audit and explicitly pass --allow-risky",
        ));
    }
    // Provenance-only changes have no indexed fields. Preserve the existing
    // database bytes unless content, active description or audit level changes.
    let mut records = vault.records()?;
    let record = records
        .iter_mut()
        .find(|r| r.id == options.id)
        .ok_or_else(|| Error::new("SKILL_NOT_FOUND", "Unknown skill"))?;
    let description = if updated.description.active == "improved" {
        updated.description.improved.clone().unwrap_or_default()
    } else {
        updated.description.original.clone()
    };
    let index_changed = record.content_hash != updated.content_hash
        || record.audit_level != updated.audit.level
        || record.description != description;
    if index_changed {
        let staged_index = checkout.root.parent().unwrap().join("index.db");
        fs::copy(&vault.index_path, &staged_index)?;
        record.source = updated.source.clone();
        record.content_hash = updated.content_hash.clone();
        record.audit_level = updated.audit.level.clone();
        record.description = description;
        index::rebuild_for_vault(&staged_index, &vault.root, &records)?;
        let index_before = Snapshot::read(&vault.index_path)?;
        let data = Data::FileWithMode(
            fs::read(&staged_index)?,
            index_before
                .as_ref()
                .and_then(Snapshot::file_mode)
                .unwrap_or(0o600),
        );
        writes.push(PlannedWrite {
            target: vault.index_path.clone(),
            stage_parent: vault.index_path.parent().unwrap().into(),
            before: index_before,
            data,
        });
    }
    tx::preflight(&writes, &observations, &anchors)?;
    if let Some(c) = control {
        c.stage("committing")?;
    }
    // Published paths are verified against their prepared after-snapshots by
    // the transaction. Only untouched dependencies retain before-observations.
    observations.retain(|(path, _)| !writes.iter().any(|write| write.target == *path));
    let mut filesystem = ActivationTransaction::prepare(writes, observations, anchors)?;
    if let Err(error) = filesystem.apply().and_then(|_| filesystem.verify()) {
        return Err(filesystem.abort(error));
    }
    let mut paths = old_payload
        .entries
        .iter()
        .chain(payload.entries.iter())
        .filter(|e| e.bytes.is_some())
        .map(|e| Path::new("skills").join(&options.id).join(&e.relative))
        .collect::<BTreeSet<_>>();
    paths.insert(Path::new("meta").join(format!("{}.toml", options.id)));
    let paths = paths.into_iter().collect::<Vec<_>>();
    if let Err(error) = gitlog::commit_paths(
        &vault.root,
        &format!("Update source {} to {}", options.id, new_commit),
        &paths,
    ) {
        let error = filesystem.abort(error);
        let _ = gitlog::unstage_paths(&vault.root, &paths);
        return Err(error);
    }
    let cleanup = filesystem.finish();
    if !cleanup.is_empty() {
        report.warnings.push(UpdateWarning {
            reason_code: "SOURCE_CLEANUP_PRESERVED".into(),
            old_description: None,
            new_description: None,
        });
    }
    report.committed = true;
    report.requires_confirmation = false;
    Ok(report)
}
fn write_tree(
    target: PathBuf,
    parent: &Path,
    payload: build::ActivationPayload,
) -> Result<PlannedWrite> {
    Ok(PlannedWrite {
        before: Snapshot::read(&target)?,
        target,
        stage_parent: parent.into(),
        data: Data::Tree(payload),
    })
}
fn materialize(root: &Path, payload: &build::ActivationPayload) -> Result<()> {
    fs::create_dir(root)?;
    for entry in &payload.entries {
        let path = root.join(&entry.relative);
        if let Some(bytes) = &entry.bytes {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&path, bytes)?;
        } else {
            fs::create_dir_all(&path)?;
        }
        fs::set_permissions(path, entry.permissions.clone())?;
    }
    Ok(())
}
pub(crate) fn refresh_manual(
    dist: &Path,
    id: &str,
    payload: &build::ActivationPayload,
    writes: &mut Vec<PlannedWrite>,
) -> Result<()> {
    let path = dist.join(".manual/.skillsbox-manual.toml");
    let Some(before) = Snapshot::read(&path)? else {
        reject_unowned_manual(dist, id)?;
        return Ok(());
    };
    let bytes = before.bytes().ok_or_else(|| {
        Error::decision("BUILD_CONFLICT", "Manual ownership manifest must be a file")
    })?;
    let mut value: toml::Value = toml::from_str(
        std::str::from_utf8(bytes).map_err(|e| Error::new("INVALID_METADATA", e.to_string()))?,
    )?;
    if value.get("version").and_then(|v| v.as_integer()) != Some(1) {
        return Err(Error::decision(
            "BUILD_CONFLICT",
            "Unsupported manual manifest version",
        ));
    }
    let generated = value
        .get_mut("generated")
        .and_then(|v| v.as_table_mut())
        .ok_or_else(|| Error::decision("BUILD_CONFLICT", "Missing manual ownership table"))?;
    if let Some(recorded) = generated.get(id) {
        let target = dist.join(".manual").join(id);
        let target_before = Snapshot::read(&target)?;
        if target_before
            .as_ref()
            .is_some_and(|snapshot| snapshot.digest() != recorded.as_str())
        {
            return Err(Error::decision(
                "BUILD_CONFLICT",
                "Modified manual cache was preserved",
            ));
        }
        let manual = payload.manual()?;
        if target_before.is_some()
            && build::source_payload(&target)?.generation_hash() == manual.generation_hash()
        {
            return Ok(());
        }
        generated.insert(id.into(), toml::Value::String(manual.content_hash.clone()));
        writes.push(PlannedWrite {
            target,
            stage_parent: dist.join(".manual"),
            before: target_before,
            data: Data::Tree(manual),
        });
        writes.push(PlannedWrite {
            target: path,
            stage_parent: dist.into(),
            before: Some(before),
            data: Data::File(toml::to_string_pretty(&value)?.into_bytes()),
        });
    } else {
        reject_unowned_manual(dist, id)?;
    }
    Ok(())
}

fn reject_unowned_manual(dist: &Path, id: &str) -> Result<()> {
    match fs::symlink_metadata(dist.join(".manual").join(id)) {
        Ok(_) => Err(Error::decision(
            "BUILD_CONFLICT",
            "Unowned manual cache was preserved",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
