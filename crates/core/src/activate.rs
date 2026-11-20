//! Plan the complete activation before staging any cache, project or state data.
use crate::activation_transaction::{
    self as transaction, ActivationTransaction, Data, PlannedWrite, Snapshot,
};
use crate::{
    ActivateOptions, ActivationItem, ActivationReport, Error, LockedSkill, ProjectLock,
    ProjectRegistration, Result, agents, build, bundles, edit, frontmatter, index, lockfile,
    project_registry, vault::Vault,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManualOwnership {
    version: u32,
    generated: BTreeMap<String, String>,
}

struct Plan {
    report: ActivationReport,
    writes: Vec<PlannedWrite>,
    observations: Vec<(PathBuf, Option<Snapshot>)>,
    anchors: Vec<(PathBuf, fs::Metadata)>,
}

pub fn activate(vault: &Vault, options: &ActivateOptions) -> Result<ActivationReport> {
    execute(
        vault,
        plan(vault, options, &BTreeMap::new(), None)?,
        options.dry_run,
    )
}

pub(crate) fn adopt_validated(
    vault: &Vault,
    options: &ActivateOptions,
    targets: &BTreeMap<PathBuf, Snapshot>,
    pairs: &BTreeSet<(String, String)>,
) -> Result<ActivationReport> {
    execute(
        vault,
        plan(vault, options, targets, Some(pairs))?,
        options.dry_run,
    )
}

fn execute(vault: &Vault, mut plan: Plan, dry_run: bool) -> Result<ActivationReport> {
    transaction::preflight(&plan.writes, &plan.observations, &plan.anchors)?;
    if dry_run || !plan.report.changed {
        return Ok(plan.report);
    }
    let _state_guard = crate::state::write_guard(&vault.index_path, false)?;
    // Hold the cache transaction until every filesystem publication is verified.
    let mut connection = plan
        .report
        .index_changed
        .then(|| index::activation_connection(&vault.index_path, true))
        .transpose()?;
    let database = connection
        .as_mut()
        .map(|connection| {
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        })
        .transpose()?;
    if let Some(database) = &database {
        index::write_project_cache(database, &plan.report.project, &plan.report.lock)?;
    }
    let mut filesystem =
        ActivationTransaction::prepare(plan.writes, plan.observations, plan.anchors)?;
    if let Some(bytes) = filesystem.staged_file(&lockfile::write_path(&plan.report.project)?) {
        plan.report.lock = lockfile::parse(bytes)?;
    }
    if let Err(error) = filesystem.apply() {
        return Err(filesystem.abort(error));
    }
    if let Err(error) = filesystem.verify() {
        return Err(filesystem.abort(error));
    }
    if let Some(database) = database
        && let Err(error) = database.commit()
    {
        return Err(filesystem.abort(error.into()));
    }
    plan.report.warnings.extend(filesystem.finish());
    Ok(plan.report)
}

fn plan(
    vault: &Vault,
    options: &ActivateOptions,
    adopting: &BTreeMap<PathBuf, Snapshot>,
    pairs: Option<&BTreeSet<(String, String)>>,
) -> Result<Plan> {
    if options.selections.is_empty()
        || !matches!(options.mode.as_str(), "link" | "copy")
        || !matches!(
            options.agent.as_str(),
            "claude" | "codex" | "cursor" | "all"
        )
    {
        return Err(Error::new(
            "INVALID_ARGUMENTS",
            "Choose skills or one bundle, agent claude/codex/cursor/all and mode link/copy",
        ));
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    return Err(Error::new(
        "UNSUPPORTED_PLATFORM",
        "Exclusive activation publication is unavailable on this platform",
    ));
    let root = index::validate_path(&vault.root)?;
    let project = index::validate_path(&options.project)?;
    let home = index::validate_path(&options.home)?;
    if !fs::symlink_metadata(&project)?.is_dir() {
        return Err(Error::new(
            "INVALID_PROJECT",
            "Project must be an existing real directory",
        ));
    }
    if project.starts_with(&root)
        || project == home
        || project.starts_with(home.join(".claude/skills"))
        || project.starts_with(home.join(".agents/skills"))
        || project.starts_with(home.join(".cursor/skills"))
    {
        return Err(conflict(
            "Choose a project outside the vault and global skill directories",
        ));
    }
    vault.config()?;
    let records = vault.records()?;
    for source in records.iter().flat_map(|record| &record.source.found_in) {
        if project.starts_with(source) {
            return Err(conflict(
                "Activation must not modify an imported source tree",
            ));
        }
    }
    let lock_path = lockfile::write_path(&project)?;
    let lock_before = Snapshot::read(&lock_path)?;
    let old_lock = lockfile::read(&project)?;
    if let Some(lock) = &old_lock {
        lockfile::validate_for_vault(&project, &root, lock)?;
    }
    if !transaction::matches(&lock_path, lock_before.as_ref())? {
        return Err(conflict("Project lock changed during preflight"));
    }
    let mut lock = old_lock.clone().unwrap_or_else(|| ProjectLock {
        restoration: None,
        vault: root.clone(),
        bundle: None,
        active: Vec::new(),
    });
    if lock.restoration.is_none() {
        lock.restoration = Some(crate::ProjectRestoration {
            created_lock: old_lock.is_none(),
            original_lock_mode: lock_before.as_ref().and_then(Snapshot::file_mode),
            original_empty_lock: if old_lock.as_ref().is_some_and(|lock| lock.active.is_empty()) {
                lock_before
                    .as_ref()
                    .and_then(Snapshot::bytes)
                    .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            } else {
                None
            },
            directories: Vec::new(),
        });
    }
    let known = records
        .iter()
        .map(|record| record.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut selected = Vec::new();
    let mut bundle_agents = None;
    let mut secondary = BTreeSet::new();
    for selection in &options.selections {
        crate::vault::validate_id(selection)?;
    }
    if options.selections.len() == 1 {
        let name = &options.selections[0];
        let bundle_path = root.join("bundles").join(format!("{name}.toml"));
        let has_bundle = exists(&bundle_path)?;
        if has_bundle && known.contains(name.as_str()) {
            return Err(Error::decision(
                "AMBIGUOUS_SELECTION",
                "Selection names both a skill and a bundle",
            ));
        }
        if has_bundle {
            let bundle = bundles::read(vault, name)?;
            let wanted = options
                .include_secondary
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            if wanted.len() != options.include_secondary.len() {
                return Err(Error::new("INVALID_ARGUMENTS", "Repeated secondary skill"));
            }
            for id in &wanted {
                if !bundle
                    .skills
                    .iter()
                    .any(|member| member.id == *id && member.role == "secondary")
                {
                    return Err(Error::new(
                        "INVALID_ARGUMENTS",
                        "--include-secondary must name a secondary member of the selected bundle",
                    ));
                }
            }
            for member in &bundle.skills {
                if member.role == "primary" || wanted.contains(member.id.as_str()) {
                    selected.push(member.id.clone());
                    if member.role == "secondary" {
                        secondary.insert(member.id.clone());
                    }
                }
            }
            lock.bundle = Some(bundle.name);
            bundle_agents = Some(bundle.agents);
        } else {
            selected.push(name.clone());
        }
    } else {
        selected = options.selections.clone();
    }
    if bundle_agents.is_none() && !options.include_secondary.is_empty() {
        return Err(Error::new(
            "INVALID_ARGUMENTS",
            "--include-secondary requires one selected bundle",
        ));
    }
    if selected.is_empty() {
        return Err(Error::new(
            "INVALID_ARGUMENTS",
            "Selection has no active members",
        ));
    }
    let mut distinct = BTreeSet::new();
    for id in &selected {
        if !distinct.insert(id) {
            return Err(Error::new("INVALID_ARGUMENTS", "Repeated skill selection"));
        }
        if !known.contains(id.as_str()) {
            return Err(Error::new(
                "SKILL_NOT_FOUND",
                format!("Unknown skill: {id}"),
            ));
        }
    }
    let agents = match (&bundle_agents, options.agent.as_str()) {
        (Some(agents), "all") => agents.clone(),
        (Some(agents), agent) if agents.iter().any(|allowed| allowed == agent) => {
            vec![agent.into()]
        }
        (Some(_), _) => {
            return Err(Error::decision(
                "BUNDLE_AGENT_CONFLICT",
                "Requested agent is not enabled in the bundle",
            ));
        }
        (None, "all") => agents::SUPPORTED.iter().map(|a| (*a).into()).collect(),
        (None, agent) => vec![agent.into()],
    };
    let cache = build::activation_cache(vault, &selected)?;
    let mut writes = Vec::new();
    let mut observations = Vec::new();
    for id in &selected {
        observations.push((
            root.join("skills").join(id),
            Snapshot::read(&root.join("skills").join(id))?,
        ));
        let metadata = root.join("meta").join(format!("{id}.toml"));
        observations.push((metadata.clone(), Snapshot::read(&metadata)?));
    }
    let dist = root.join("dist");
    for (id, before_hash) in &cache.changed {
        let target = dist.join(id);
        let before = owned_tree(&target, before_hash.as_deref())?;
        writes.push(PlannedWrite {
            target,
            stage_parent: dist.clone(),
            before,
            data: Data::Tree(cache.payloads[id].clone()),
        });
    }
    let manifest_path = dist.join(".skillsbox-build.toml");
    let auto_manifest_before = file_snapshot(&manifest_path, cache.manifest_before.as_deref())?;
    if let Some(bytes) = cache.manifest_after {
        writes.push(PlannedWrite {
            target: manifest_path,
            stage_parent: dist.clone(),
            before: auto_manifest_before,
            data: Data::File(bytes),
        });
    } else {
        observations.push((manifest_path, auto_manifest_before));
    }
    let manual_root = dist.join(".manual");
    index::validate_path(&manual_root)?;
    let manual_manifest_path = manual_root.join(".skillsbox-manual.toml");
    let manual_before = Snapshot::read(&manual_manifest_path)?;
    let mut manual_manifest = match &manual_before {
        Some(snapshot) => toml::from_str::<ManualOwnership>(
            std::str::from_utf8(
                snapshot
                    .bytes()
                    .ok_or_else(|| conflict("Manual manifest must be a regular file"))?,
            )
            .map_err(|error| conflict(error.to_string()))?,
        )
        .map_err(|error| conflict(error.to_string()))?,
        None => ManualOwnership {
            version: 1,
            ..Default::default()
        },
    };
    if manual_manifest.version != 1 {
        return Err(conflict("Unsupported manual cache manifest"));
    }
    for (id, digest) in &manual_manifest.generated {
        crate::vault::validate_id(id)?;
        if !digest.strip_prefix("blake3:").is_some_and(|hash| {
            hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
            return Err(conflict("Invalid manual cache ownership hash"));
        }
    }
    let original_manual = manual_manifest.generated.clone();
    let mut manual_done = BTreeSet::new();
    let mut items = Vec::new();
    let mut selected_names = BTreeSet::new();
    let mut suggestions = Vec::new();
    let mut warnings = Vec::new();
    for id in &selected {
        let metadata_path = root.join("meta").join(format!("{id}.toml"));
        let (_, bytes) = edit::read_snapshot(vault, id)?;
        let metadata: toml::Value = toml::from_str(
            std::str::from_utf8(&bytes).map_err(|error| conflict(error.to_string()))?,
        )?;
        let configured = match metadata.get("invocation") {
            None => "auto",
            Some(value) => match value
                .as_table()
                .ok_or_else(|| Error::new("INVALID_METADATA", "invocation must be a table"))?
                .get("mode")
            {
                None => "auto",
                Some(mode) => mode
                    .as_str()
                    .filter(|mode| matches!(*mode, "auto" | "manual"))
                    .ok_or_else(|| {
                        Error::new(
                            "INVALID_METADATA",
                            format!(
                                "{}: invocation.mode must be auto or manual",
                                metadata_path.display()
                            ),
                        )
                    })?,
            },
        };
        let invocation = if secondary.contains(id) || configured == "manual" {
            "manual"
        } else {
            "auto"
        };
        let base = &cache.payloads[id];
        let payload = if invocation == "manual" {
            base.manual()?
        } else {
            base.clone()
        };
        let source = if invocation == "manual" {
            manual_root.join(id)
        } else {
            dist.join(id)
        };
        if invocation == "manual" && manual_done.insert(id.clone()) {
            let before = owned_tree(
                &source,
                manual_manifest.generated.get(id).map(String::as_str),
            )?;
            if before.as_ref().and_then(Snapshot::digest) != Some(payload.content_hash.as_str()) {
                writes.push(PlannedWrite {
                    target: source.clone(),
                    stage_parent: dist.clone(),
                    before,
                    data: Data::Tree(payload.clone()),
                });
            } else {
                observations.push((source.clone(), before));
            }
            manual_manifest
                .generated
                .insert(id.clone(), payload.content_hash.clone());
        } else if invocation == "auto" && !cache.changed.contains_key(id) {
            observations.push((source.clone(), Snapshot::read(&source)?));
        }
        let name = frontmatter::read_skill(&root.join("skills").join(id).join("SKILL.md"))?.name;
        if name.chars().any(crate::names::hidden_or_reordering) {
            return Err(Error::decision(
                "UNSAFE_ACTIVATION_NAME",
                format!(
                    "Unsafe activation folder name: {}",
                    crate::names::visible(&name)
                ),
            ));
        }
        for agent in &agents {
            if pairs.is_some_and(|pairs| !pairs.contains(&(id.clone(), agent.clone()))) {
                continue;
            }
            if !selected_names.insert((agent.clone(), name.clone())) {
                return Err(Error::decision(
                    "ACTIVATION_NAME_CONFLICT",
                    "Selected skills have the same original activation name",
                ));
            }
            agents::validate_invocation(agent, invocation)?;
            let prefix = agents::project_skills(agent).ok_or_else(|| conflict("Unknown agent"))?;
            let parent = index::validate_path(&project.join(prefix))?;
            if agent == "codex" || agent == "cursor" {
                let global = home.join(".agents/skills").join(&name);
                index::validate_path(global.parent().expect("global parent"))?;
                if exists(&global)? {
                    return Err(Error::decision(
                        "CODEX_GLOBAL_NAME_CONFLICT",
                        format!("A global Codex skill already uses the name {name}"),
                    ));
                }
                observations.push((global, None));
            }
            if agent == "claude"
                && fs::symlink_metadata(&parent)
                    .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
                && !warnings
                    .iter()
                    .any(|message: &String| message.contains("/reload-skills"))
            {
                warnings.push("Creating .claude/skills for the first time: run /reload-skills in an already-open Claude Code session.".into());
            }
            let target = parent.join(&name);
            if let Some(legacy) = lock
                .active
                .iter()
                .find(|skill| skill.id == *id && skill.agent == *agent)
                && lockfile::target_path(&project, legacy)? != target
            {
                return Err(Error::decision(
                    "CURSOR_LEGACY_TARGET",
                    "Explicitly unuse the historical Cursor target before activating the shared target",
                ));
            }
            let shared = lock.active.iter().find(|skill| {
                skill.agent != *agent
                    && lockfile::target_path(&project, skill).ok().as_ref() == Some(&target)
            });
            if let Some(shared) = shared
                && (shared.id != *id
                    || shared.mode != options.mode
                    || shared.invocation != invocation
                    || ((shared.content_hash != payload.content_hash
                        || shared
                            .generation_hash
                            .as_ref()
                            .is_some_and(|hash| hash != &payload.generation_hash()))
                        && !agents.contains(&shared.agent)))
            {
                return Err(Error::decision(
                    "SHARED_ACTIVATION_CONFLICT",
                    "Codex and Cursor share this target; their skill, mode, invocation and generation must match",
                ));
            }
            let previous = lock
                .active
                .iter()
                .find(|skill| skill.id == *id && skill.agent == *agent)
                .or(shared)
                .cloned();
            if previous.is_none() && exists(&target)? && !adopting.contains_key(&target) {
                return Err(Error::decision(
                    "ACTIVATION_TARGET_EXISTS",
                    format!("Unowned target already exists: {}", target.display()),
                ));
            }
            let before = if let Some(expected) = adopting.get(&target) {
                if !transaction::matches(&target, Some(expected))? {
                    return Err(conflict("Adoption candidate changed after preview"));
                }
                Some(expected.clone())
            } else if previous.is_some() {
                Snapshot::read(&target)?
            } else {
                None
            };
            if let Some(previous) = &previous {
                if previous.name != name {
                    return Err(conflict(
                        "An active skill changed its original name; explicitly unuse it before renaming",
                    ));
                }
                if let Some(snapshot) = &before
                    && !adopting.contains_key(&target)
                {
                    if previous
                        .ownership
                        .as_ref()
                        .is_some_and(|ownership| *ownership != snapshot.ownership())
                    {
                        return Err(conflict(
                            "Activation target identity or contents changed; it was preserved",
                        ));
                    }
                    let old_source = if previous.invocation == "manual" {
                        manual_root.join(id)
                    } else {
                        dist.join(id)
                    };
                    let owned = if previous.mode == "link" {
                        snapshot.link() == Some(old_source.as_path())
                    } else {
                        snapshot.digest() == Some(previous.content_hash.as_str())
                    };
                    if !owned {
                        return Err(conflict(format!(
                            "Activation target was modified or replaced: {}",
                            target.display()
                        )));
                    }
                }
            } else if before.is_some() && !adopting.contains_key(&target) {
                return Err(Error::decision(
                    "ACTIVATION_TARGET_EXISTS",
                    format!("Unowned target already exists: {}", target.display()),
                ));
            }
            let mut desired = LockedSkill {
                ownership: None,
                id: id.clone(),
                agent: agent.clone(),
                mode: options.mode.clone(),
                name: name.clone(),
                target_root: (agent == "cursor").then(|| ".agents/skills".into()),
                generation_hash: Some(payload.generation_hash()),
                invocation: invocation.into(),
                content_hash: payload.content_hash.clone(),
            };
            let unchanged = if options.mode == "link" {
                before.as_ref().and_then(Snapshot::link) == Some(source.as_path())
            } else {
                before.as_ref().and_then(Snapshot::digest) == Some(payload.content_hash.as_str())
                    && build::source_payload(&target)
                        .is_ok_and(|copy| copy.generation_hash() == payload.generation_hash())
            };
            if unchanged {
                desired.ownership = before.as_ref().map(Snapshot::ownership);
            }
            let already_planned = writes.iter().any(|write| write.target == target);
            if unchanged {
                observations.push((target.clone(), before));
            } else if !already_planned {
                writes.push(PlannedWrite {
                    target: target.clone(),
                    stage_parent: parent.clone(),
                    before,
                    data: if options.mode == "link" {
                        Data::Link(source.clone())
                    } else {
                        Data::Tree(payload.clone())
                    },
                });
            }
            lock.active
                .retain(|skill| !(skill.id == *id && skill.agent == *agent));
            lock.active.push(desired);
            items.push(ActivationItem {
                id: id.clone(),
                agent: agent.clone(),
                mode: options.mode.clone(),
                invocation: invocation.into(),
                name: name.clone(),
                source: source.clone(),
                target: target.clone(),
                content_hash: payload.content_hash.clone(),
                changed: !unchanged,
            });
            if options.mode == "link" {
                suggestions.push(format!("/{}/{}", prefix, escape_gitignore(&name)));
            }
        }
    }
    if manual_manifest.generated != original_manual {
        writes.push(PlannedWrite {
            target: manual_manifest_path,
            stage_parent: dist.clone(),
            before: manual_before,
            data: Data::File(toml::to_string_pretty(&manual_manifest)?.into_bytes()),
        });
    } else {
        observations.push((manual_manifest_path, manual_before));
    }
    lock.active
        .sort_by(|left, right| (&left.id, &left.agent).cmp(&(&right.id, &right.agent)));
    lockfile::validate_for_vault(&project, &root, &lock)?;
    let stage_parent =
        project.join(agents::project_skills(&items[0].agent).expect("selected agent"));
    let missing_parent = items
        .iter()
        .any(|item| item.target.parent().is_some_and(|parent| !parent.is_dir()));
    if old_lock.as_ref() != Some(&lock) || missing_parent {
        writes.push(PlannedWrite {
            target: lock_path,
            stage_parent,
            before: lock_before,
            data: Data::ProjectLock(lock.clone()),
        });
    } else {
        observations.push((lock_path, lock_before));
    }
    let registry_path = project_registry::path(vault)?;
    let registry_before = Snapshot::read(&registry_path)?;
    let mut registry = project_registry::read(vault)?;
    if !transaction::matches(&registry_path, registry_before.as_ref())? {
        return Err(conflict("Project registry changed during preflight"));
    }
    match registry.projects.iter().find(|entry| entry.path == project) {
        Some(entry) if entry.vault != root => {
            return Err(Error::decision(
                "PROJECT_VAULT_CONFLICT",
                "Project registry belongs to another vault",
            ));
        }
        Some(_) => observations.push((registry_path, registry_before)),
        None => {
            registry.projects.push(ProjectRegistration {
                path: project.clone(),
                vault: root.clone(),
            });
            writes.push(PlannedWrite {
                stage_parent: registry_path.parent().expect("registry parent").into(),
                target: registry_path,
                before: registry_before,
                data: Data::File(toml::to_string_pretty(&registry)?.into_bytes()),
            });
        }
    }
    let connection = index::activation_connection(&vault.index_path, false)?;
    let index_changed = !index::project_cache_matches(&connection, &project, &lock)?;
    let mut anchors = BTreeMap::new();
    for path in [&project, &root, &dist, &vault.index_path] {
        if let Ok(metadata) = fs::symlink_metadata(path) {
            anchors.insert(path.clone(), metadata);
        }
    }
    let mut targets = BTreeSet::new();
    for write in &writes {
        if !targets.insert(&write.target) {
            return Err(conflict("Activation batch repeats a destination"));
        }
        for parent in [
            write.target.parent().expect("target parent"),
            write.stage_parent.as_path(),
        ] {
            for ancestor in parent.ancestors() {
                index::validate_path(ancestor)?;
                if let Ok(metadata) = fs::symlink_metadata(ancestor) {
                    if !metadata.is_dir() {
                        return Err(conflict("An activation parent is not a directory"));
                    }
                    anchors.insert(ancestor.to_path_buf(), metadata);
                }
            }
        }
    }
    suggestions.sort();
    suggestions.dedup();
    let changed = !writes.is_empty() || index_changed;
    Ok(Plan {
        report: ActivationReport {
            project,
            dry_run: options.dry_run,
            changed,
            index_changed,
            items,
            lock,
            gitignore_suggestions: suggestions,
            warnings,
        },
        writes,
        observations,
        anchors: anchors.into_iter().collect(),
    })
}

fn owned_tree(path: &Path, expected: Option<&str>) -> Result<Option<Snapshot>> {
    if expected.is_none() && exists(path)? {
        return Err(conflict(format!(
            "Unowned generated cache: {}",
            path.display()
        )));
    }
    let snapshot = Snapshot::read(path)?;
    match (&snapshot, expected) {
        (None, _) => Ok(None),
        (Some(snapshot), Some(expected)) if snapshot.digest() == Some(expected) => {
            Ok(Some(snapshot.clone()))
        }
        _ => Err(conflict(format!(
            "Unowned or modified generated cache: {}",
            path.display()
        ))),
    }
}

fn file_snapshot(path: &Path, expected: Option<&[u8]>) -> Result<Option<Snapshot>> {
    let snapshot = Snapshot::read(path)?;
    let same = match (&snapshot, expected) {
        (None, None) => true,
        (Some(snapshot), Some(bytes)) => snapshot.bytes() == Some(bytes),
        _ => false,
    };
    if !same {
        return Err(conflict(
            "Generated cache manifest changed during preflight",
        ));
    }
    Ok(snapshot)
}

fn escape_gitignore(name: &str) -> String {
    let mut escaped = String::new();
    for character in name.chars() {
        if matches!(character, ' ' | '\\' | '[' | ']' | '*' | '?' | '!' | '#') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}
fn conflict(message: impl Into<String>) -> Error {
    transaction::changed(message)
}

fn exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}
