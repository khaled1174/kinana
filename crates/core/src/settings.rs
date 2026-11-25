//! Local settings and first-run operations. The desktop is only a caller.
use crate::{
    BoxConfig, Error, Result, agents, audit,
    dimensions::{self, Dimension},
    doctor, gitlog, index, router,
    vault::Vault,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preferences {
    pub version: u32,
    pub vault: PathBuf,
}
pub fn preferences_path(home: &Path) -> PathBuf {
    home.join(".kinana/desktop.toml")
}
/// New accounts only; existing selections and previous default vaults stay put.
pub fn default_vault(home: &Path) -> PathBuf {
    home.join("KinanaVault")
}
pub fn preferred_vault(home: &Path) -> Result<PathBuf> {
    let path = index::validate_path(&crate::state::directory(home)?.join("desktop.toml"))?;
    match fs::read_to_string(path) {
        Ok(text) => {
            let p: Preferences = toml::from_str(&text)?;
            if p.version != 1 {
                return Err(Error::new(
                    "INVALID_DESKTOP_PREFERENCES",
                    "Unsupported desktop settings",
                ));
            }
            index::validate_path(&p.vault)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let legacy = home.join("SkillsBox");
            if legacy.join("box.toml").is_file() {
                Ok(legacy)
            } else {
                let previous = home.join("Documents/Kinana");
                if previous.join("box.toml").is_file() {
                    index::validate_path(&previous)
                } else {
                    Ok(default_vault(home))
                }
            }
        }
        Err(e) => Err(e.into()),
    }
}
/// Advisory only: inspect path components without reading protected folders,
/// following links, creating directories, or changing the user's selection.
/// Mutation previews retain their independent path and ownership validation.
pub fn vault_location_warning_codes(home: &Path, path: &Path) -> Result<Vec<String>> {
    let expanded = if path == Path::new("~") {
        home.to_path_buf()
    } else if let Ok(part) = path.strip_prefix("~") {
        home.join(part)
    } else if path.is_absolute() {
        path.to_path_buf()
    } else {
        // The mutation refuses it with VAULT_PATH_RELATIVE; no advice here.
        return Ok(vec![]);
    };
    let mut normalized = PathBuf::new();
    for component in expanded.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(Error::new(
                    "INVALID_PATH",
                    "Parent path components are not allowed",
                ));
            }
            _ => normalized.push(component),
        }
    }
    let under = |root: PathBuf| {
        let mut parts = normalized.components();
        root.components().all(|expected| {
            parts.next().is_some_and(|actual| {
                #[cfg(target_os = "macos")]
                {
                    actual
                        .as_os_str()
                        .eq_ignore_ascii_case(expected.as_os_str())
                }
                #[cfg(not(target_os = "macos"))]
                {
                    actual == expected
                }
            })
        })
    };
    let mut codes = vec![];
    for (folder, code) in [
        ("Documents", "VAULT_LOCATION_DOCUMENTS"),
        ("Desktop", "VAULT_LOCATION_DESKTOP"),
    ] {
        if under(home.join(folder)) {
            codes.push(code.into());
        }
    }
    if [
        "Library/Mobile Documents",
        "Library/CloudStorage/iCloud Drive",
        "iCloud Drive",
    ]
    .iter()
    .any(|root| under(home.join(root)))
    {
        codes.push("VAULT_LOCATION_ICLOUD".into());
    }
    Ok(codes)
}
#[derive(Serialize)]
pub struct Snapshot {
    pub migration_required: bool,
    pub vault_location_codes: Vec<String>,
    pub vault: PathBuf,
    pub index: PathBuf,
    pub home: PathBuf,
    pub preferences: PathBuf,
    pub first_run: bool,
    pub path_conflict: bool,
    pub config: Option<BoxConfig>,
    pub error: Option<Error>,
    pub agents: Vec<crate::AgentInfo>,
    pub remote: Option<String>,
    pub backups: Vec<PathBuf>,
}
pub fn read(vault: &Vault, home: &Path) -> Result<Snapshot> {
    let (config, mut error) = match vault.config() {
        Ok(config) => (Some(config), None),
        Err(error) => (None, Some(error)),
    };
    let first_run = !vault.root.join("box.toml").try_exists()?;
    let path_conflict = config.is_none()
        && vault.root.is_dir()
        && fs::read_dir(&vault.root)?.next().transpose()?.is_some();
    if path_conflict {
        error = Some(Error::new(
            "VAULT_CONFLICT",
            "The selected location contains a non-vault tree; preserved",
        ));
    }
    let remote = if config.is_some() {
        gitlog::configured_remote(&vault.root)?
    } else {
        None
    };
    let mut backups = vec![];
    if let Some(parent) = vault.index_path.parent()
        && parent.is_dir()
    {
        for entry in fs::read_dir(index::validate_path(parent)?)? {
            let path = entry?.path();
            if path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(".skillsbox-index-rebuild-"))
                && index::validate_path(&path).is_ok()
                && fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir())
            {
                let backup = path.join("original.db");
                if index::validate_path(&backup).is_ok()
                    && fs::symlink_metadata(&backup).is_ok_and(|m| m.is_file())
                {
                    backups.push(backup);
                }
            }
        }
    }
    backups.sort();
    Ok(Snapshot {
        migration_required: crate::state::directory(home)? == crate::state::legacy(home),
        vault_location_codes: vault_location_warning_codes(home, &vault.root)?,
        vault: vault.root.clone(),
        index: vault.index_path.clone(),
        home: home.into(),
        preferences: crate::state::directory(home)?.join("desktop.toml"),
        first_run,
        path_conflict,
        config,
        error,
        agents: agents::detect(home),
        remote,
        backups,
    })
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
// One settings request at a time, read from JSON; size is not a concern.
#[allow(clippy::large_enum_variant)]
pub enum Action {
    Initialize {
        path: PathBuf,
    },
    SelectVault {
        path: PathBuf,
    },
    Save {
        classification_agent: String,
        categories: Vec<String>,
        /// Absent means "leave the list as it is", which is what callers
        /// written before 8.3 send.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stacks: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stages: Option<Vec<String>>,
        scan_paths: Vec<PathBuf>,
        record_found_in: bool,
        /// 0.3: the whole set of user definitions after the edit; absent
        /// keeps the current ones. Values removed from a list lose theirs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        definitions: Option<crate::taxonomy::Definitions>,
        /// 0.3: Pi settings after the edit; absent keeps the current ones.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pi: Option<crate::PiSettings>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        claude: Option<crate::ClaudeSettings>,
        /// 0.3 (r5, r6): approve proposed descriptions of skills that are not risky
        /// automatically after a classification; absent keeps the current.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auto_approve_descriptions: Option<bool>,
    },
    RebuildIndex,
    InstallRouter,
    RefreshAudit,
    Remote {
        url: String,
    },
}
#[derive(Debug, Serialize)]
pub struct Change {
    pub path: PathBuf,
    pub reason_code: String,
}
#[derive(Debug, Serialize)]
pub struct Plan {
    pub action: Action,
    pub token: String,
    pub changes: Vec<Change>,
    pub diagnostic_codes: Vec<String>,
    pub changed: bool,
    /// For `refresh_audit`: how every skill's level would change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit: Option<audit::RulesUpgrade>,
}
/// A vault path from the desktop must be absolute or start with `~`: an app
/// opened from Finder runs in `/`, so a relative path would land there
/// (owner decision, 2026-11-18).
fn resolve_vault(home: &Path, path: &Path) -> Result<PathBuf> {
    let text = path.to_str().unwrap_or_default();
    if !path.is_absolute() && text != "~" && !text.starts_with("~/") {
        return Err(Error::new(
            "VAULT_PATH_RELATIVE",
            "Choose a full vault path (starting with / or ~/); a relative path is not accepted.",
        ));
    }
    resolve(home, path)
}
fn resolve(home: &Path, path: &Path) -> Result<PathBuf> {
    let text = path
        .to_str()
        .ok_or_else(|| Error::new("INVALID_PATH", "Path must be Unicode"))?;
    let expanded = if text == "~" {
        home.into()
    } else if let Some(part) = text.strip_prefix("~/") {
        home.join(part)
    } else {
        path.into()
    };
    index::validate_path(&expanded)
}
fn candidate(vault: &Vault, home: &Path, action: &Action) -> Result<BoxConfig> {
    let Action::Save {
        classification_agent,
        categories,
        stacks,
        stages,
        scan_paths,
        record_found_in,
        definitions,
        pi,
        claude,
        auto_approve_descriptions,
    } = action
    else {
        unreachable!()
    };
    // `codex` from a 0.2 box.toml is kept while unchanged; choosing it anew
    // is refused with the reason (owner decision, 2026-11-18).
    let pi = match pi {
        Some(pi) => {
            crate::agent_runner::pi::validate_settings(pi)?;
            pi.clone()
        }
        None => vault.config()?.classification_pi,
    };
    match classification_agent.as_str() {
        "claude" | "keywords" | "auto" => {}
        "pi" if pi.enabled => {}
        "pi" => {
            return Err(Error::new(
                "AGENT_PI_DISABLED",
                "Enable Pi (experimental) before choosing it as the classification agent",
            ));
        }
        "codex" if vault.config()?.classification_agent == "codex" => {}
        "codex" => return Err(crate::agent_runner::codex_unsupported()),
        _ => return Err(Error::new("INVALID_AGENT", "Unknown classification agent")),
    }
    if categories.is_empty()
        || categories
            .iter()
            .any(|s| s.trim() != s || s.is_empty() || s.chars().any(char::is_control))
        || categories.iter().collect::<BTreeSet<_>>().len() != categories.len()
    {
        return Err(Error::new(
            "INVALID_CATEGORIES",
            "Categories must be unique non-empty names",
        ));
    }
    let requested_lists = [(Dimension::Stack, stacks), (Dimension::Stage, stages)];
    for (dimension, list) in &requested_lists {
        if let Some(list) = list {
            dimensions::validate_list(*dimension, list)?;
        }
    }
    // value -> skills that still hold it, for each dimension being edited.
    let mut held: [BTreeMap<String, BTreeSet<String>>; 2] = Default::default();
    for record in vault.records()? {
        let metadata = crate::edit::read_metadata(vault, &record.id)?;
        if !categories.contains(&record.category)
            || metadata
                .proposal
                .as_ref()
                .is_some_and(|p| !categories.contains(&p.category))
        {
            return Err(Error::new(
                "CATEGORY_IN_USE",
                "A stored skill or proposal still uses a removed category",
            ));
        }
        for ((dimension, list), held) in requested_lists.iter().zip(held.iter_mut()) {
            let Some(list) = list else { continue };
            let (current, proposed) = match dimension {
                Dimension::Stack => (
                    &metadata.classification.stack,
                    metadata.proposal.as_ref().map(|p| &p.stack),
                ),
                Dimension::Stage => (
                    &metadata.classification.stage,
                    metadata.proposal.as_ref().map(|p| &p.stage),
                ),
            };
            for value in current.iter().chain(proposed.into_iter().flatten()) {
                if !list.contains(value) {
                    held.entry(value.clone())
                        .or_default()
                        .insert(record.id.clone());
                }
            }
        }
    }
    // Never drop a value from skills implicitly: name them and let the user
    // edit those skills (`kinana dimensions`) before shortening the list.
    for ((dimension, _), held) in requested_lists.iter().zip(&held) {
        if !held.is_empty() {
            let details = held
                .iter()
                .map(|(value, ids)| {
                    format!(
                        "'{value}' ({})",
                        ids.iter().cloned().collect::<Vec<_>>().join(", ")
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            return Err(Error::new(
                dimension.in_use_code(),
                format!(
                    "Skills or stored proposals still hold {} values missing from the new list: {details}. Change those skills first.",
                    dimension.key()
                ),
            ));
        }
    }
    let scan_paths = scan_paths
        .iter()
        .map(|p| resolve(home, p))
        .collect::<Result<Vec<_>>>()?;
    if scan_paths
        .iter()
        .any(|p| p == home || p.starts_with(&vault.root) || !p.is_dir())
    {
        return Err(Error::new(
            "INVALID_SCAN_ROOT",
            "Choose existing explicit roots outside the vault, not the home directory itself",
        ));
    }
    let mut config = vault.config()?;
    config.classification_agent = classification_agent.clone();
    config.categories = categories.clone();
    if let Some(stacks) = stacks {
        config.stacks = stacks.clone();
    }
    if let Some(stages) = stages {
        config.stages = stages.clone();
    }
    config.scan_paths = scan_paths;
    config.record_found_in = *record_found_in;
    // A definition for a value outside its new list is refused when sent; a
    // stored one for a removed value goes with the value in the same save.
    // A sent definition for a value this same save removes goes with it; a
    // key that is in neither the old nor the new list is refused.
    if let Some(definitions) = definitions {
        let previous = vault.config()?;
        let mut requested = config.clone();
        requested.definitions = definitions.clone();
        for (kind, sent, old, new) in [
            (
                "category",
                &definitions.categories,
                &previous.categories,
                &config.categories,
            ),
            (
                "stack",
                &definitions.stacks,
                &previous.stacks,
                &config.stacks,
            ),
            (
                "stage",
                &definitions.stages,
                &previous.stages,
                &config.stages,
            ),
        ] {
            if let Some(key) = sent.keys().find(|k| !old.contains(k) && !new.contains(k)) {
                return Err(Error::new(
                    "UNKNOWN_DEFINITION_KEY",
                    format!("A {kind} definition names '{key}', which is not in the {kind} list"),
                ));
            }
        }
        crate::taxonomy::validate(&crate::taxonomy::in_effect(&requested))?;
    }
    let definitions = definitions
        .clone()
        .unwrap_or_else(|| config.definitions.clone());
    config.definitions = crate::taxonomy::normalized(&config, definitions);
    config.classification_pi = pi;
    if let Some(claude) = claude {
        crate::agent_runner::validate_claude_settings(claude)?;
        config.classification_claude = claude.clone();
    }
    if let Some(auto) = auto_approve_descriptions {
        config.auto_approve_descriptions = *auto;
    }
    crate::taxonomy::validate(&config)?;
    Ok(config)
}
fn preferences_bytes(home: &Path, root: &Path) -> Result<Vec<u8>> {
    let path = crate::state::directory(home)?.join("desktop.toml");
    let mut document: toml::Value = if path.exists() {
        toml::from_str(&fs::read_to_string(index::validate_path(&path)?)?)?
    } else {
        toml::Value::Table(Default::default())
    };
    let table = document
        .as_table_mut()
        .ok_or_else(|| Error::new("INVALID_DESKTOP_PREFERENCES", "Expected a settings table"))?;
    table.insert("version".into(), toml::Value::Integer(1));
    table.insert(
        "vault".into(),
        toml::Value::String(
            root.to_str()
                .ok_or_else(|| Error::new("INVALID_PATH", "Vault path must be Unicode"))?
                .into(),
        ),
    );
    Ok(toml::to_string_pretty(&document)?.into_bytes())
}
fn config_bytes(vault: &Vault, config: &BoxConfig) -> Result<Vec<u8>> {
    let mut document: toml::Value =
        toml::from_str(&fs::read_to_string(vault.root.join("box.toml"))?)?;
    let table = document
        .as_table_mut()
        .ok_or_else(|| Error::new("INVALID_CONFIG", "Expected a settings table"))?;
    let current = vault.config()?;
    let updated = toml::Value::try_from(config)?;
    for key in [
        "classification_agent",
        "categories",
        "scan_paths",
        "record_found_in",
    ] {
        table.insert(key.into(), updated.get(key).unwrap().clone());
    }
    // A box.toml from before 8.3 has neither list and reads the defaults.
    // Write a list only when it is already recorded or actually changes, so
    // an unrelated settings save adds no lines to an old vault.
    for (key, changed) in [
        ("stacks", current.stacks != config.stacks),
        ("stages", current.stages != config.stages),
    ] {
        if changed || table.contains_key(key) {
            table.insert(key.into(), updated.get(key).unwrap().clone());
        }
    }
    // Only what differs from the defaults is written; nothing for a vault
    // that keeps them (0.3).
    for key in [
        "classification_pi",
        "classification_claude",
        "auto_approve_descriptions",
    ] {
        if let Some(value) = updated.get(key) {
            table.insert(key.into(), value.clone());
        } else {
            table.remove(key);
        }
    }
    match updated.get("definitions") {
        Some(definitions) => {
            table.insert("definitions".into(), definitions.clone());
        }
        None => {
            table.remove("definitions");
        }
    }
    Ok(toml::to_string_pretty(&document)?.into_bytes())
}
fn validate_remote(url: &str) -> Result<()> {
    if url.is_empty()
        || url.trim() != url
        || url.chars().any(char::is_control)
        || url.starts_with('-')
        || url.contains("ext::")
        || url.contains("file://")
        || url.starts_with("http://")
    {
        return Err(Error::new(
            "INVALID_REMOTE",
            "Choose an HTTPS, SSH, or absolute local Git backup location",
        ));
    }
    if !(url.starts_with("https://")
        || url.starts_with("ssh://")
        || (url.contains('@') && url.contains(':') && !url.contains("://"))
        || Path::new(url).is_absolute())
    {
        return Err(Error::new("INVALID_REMOTE", "Unsupported remote protocol"));
    }
    if (url.starts_with("https://") && url[8..].split('/').next().is_some_and(|s| s.contains('@')))
        || url.contains('?')
    {
        return Err(Error::new(
            "REMOTE_CREDENTIALS_REJECTED",
            "Do not store credentials in remote URLs",
        ));
    }
    Ok(())
}
pub fn preview(vault: &Vault, home: &Path, action: &Action) -> Result<Plan> {
    let mut plan = Plan {
        action: action.clone(),
        token: String::new(),
        changes: vec![],
        diagnostic_codes: vec![],
        changed: true,
        audit: None,
    };
    let target = match action {
        Action::Initialize { path } | Action::SelectVault { path } => {
            Vault::new(resolve_vault(home, path)?, vault.index_path.clone())
        }
        _ => vault.clone(),
    };
    let change = |path: PathBuf, reason: &str| Change {
        path,
        reason_code: reason.into(),
    };
    if matches!(
        action,
        Action::Initialize { .. } | Action::SelectVault { .. }
    ) {
        plan.diagnostic_codes
            .extend(vault_location_warning_codes(home, &target.root)?);
    }
    match action {
        Action::Initialize { .. } => {
            target.init(true)?;
            plan.changes
                .push(change(target.root.clone(), "INITIALIZE_VAULT"));
            plan.changes
                .push(change(target.index_path.clone(), "INITIALIZE_INDEX"));
            plan.changes
                .push(change(preferences_path(home), "SELECT_DEFAULT_VAULT"));
        }
        Action::SelectVault { .. } => {
            crate::catalog::list(&target, &Default::default())?;
            plan.changes
                .push(change(preferences_path(home), "SELECT_DEFAULT_VAULT"));
        }
        Action::Save { .. } => {
            candidate(vault, home, action)?;
            gitlog::ensure_clean(&vault.root)?;
            plan.changes
                .push(change(vault.root.join("box.toml"), "SAVE_VAULT_SETTINGS"));
        }
        Action::RebuildIndex => {
            doctor::run(vault, home, true, true)?;
            plan.changes
                .push(change(vault.index_path.clone(), "REBUILD_INDEX"));
        }
        Action::InstallRouter => {
            let report = router::initialize(vault, home, true)?;
            if let Some(router) = report.router {
                let mut previous = None;
                for target in router.targets {
                    // An owned, unmodified earlier version is replaced, or the
                    // app moved and the path the router names changes; say so.
                    let reason = match target.action.as_str() {
                        "upgrade" => "UPGRADE_ROUTER",
                        "relocate" => "RELOCATE_ROUTER",
                        _ => "INSTALL_ROUTER",
                    };
                    if target.previous_cli_path.is_some() {
                        previous = target.previous_cli_path.clone();
                    }
                    plan.changes.push(change(target.path, reason));
                }
                // Both ends of a move, for the preview (owner decision: alert,
                // preview and confirmation when the app moved).
                if let Some(previous) = previous {
                    plan.changes
                        .push(change(PathBuf::from(previous), "ROUTER_PREVIOUS_CLI_PATH"));
                }
                if router.changed {
                    plan.changes
                        .push(change(PathBuf::from(&router.cli_path), "ROUTER_CLI_PATH"));
                }
            }
        }
        Action::RefreshAudit => {
            plan.audit = Some(audit::rules_upgrade(vault)?);
            for r in vault.records()? {
                plan.changes.push(change(
                    vault.root.join("meta").join(format!("{}.toml", r.id)),
                    "REFRESH_AUDIT",
                ));
            }
        }
        Action::Remote { url } => {
            validate_remote(url)?;
            gitlog::ensure_clean(&vault.root)?;
            plan.changes
                .push(change(vault.root.join(".git/config"), "CONFIGURE_REMOTE"));
            plan.diagnostic_codes
                .push("REMOTE_LOCAL_CONFIG_ONLY".into());
            if vault
                .records()?
                .iter()
                .any(|r| !r.source.found_in.is_empty())
            {
                plan.diagnostic_codes
                    .push("REMOTE_HISTORY_CONTAINS_LOCAL_PATHS".into());
            }
        }
    }
    plan.changed = match action {
        Action::Save { .. } => {
            let current = vault.config()?;
            let next = candidate(vault, home, action)?;
            current.classification_agent != next.classification_agent
                || current.categories != next.categories
                || current.stacks != next.stacks
                || current.stages != next.stages
                || current.scan_paths != next.scan_paths
                || current.record_found_in != next.record_found_in
                || current.definitions != next.definitions
                || current.classification_pi != next.classification_pi
                || current.classification_claude != next.classification_claude
                || current.auto_approve_descriptions != next.auto_approve_descriptions
        }
        Action::SelectVault { .. } => {
            fs::read(preferences_path(home)).ok().as_deref()
                != Some(preferences_bytes(home, &target.root)?.as_slice())
        }
        Action::Initialize { .. } => {
            !target.root.join("box.toml").exists()
                || !target.index_path.exists()
                || fs::read(preferences_path(home)).ok().as_deref()
                    != Some(preferences_bytes(home, &target.root)?.as_slice())
        }
        Action::InstallRouter => router::initialize(vault, home, true)?
            .router
            .is_some_and(|r| r.changed),
        Action::RefreshAudit => !audit::refresh(vault, &[], true)?.changed.is_empty(),
        Action::Remote { url } => gitlog::configured_remote(&vault.root)?.as_deref() != Some(url),
        Action::RebuildIndex => true,
    };
    // Validate an existing preferences file before any initializer or local write.
    if matches!(
        action,
        Action::Initialize { .. } | Action::SelectVault { .. }
    ) {
        preferred_vault(home)?;
    }
    let mut hash = blake3::Hasher::new();
    hash.update(&serde_json::to_vec(action).map_err(|e| Error::new("JSON_ERROR", e.to_string()))?);
    for path in [
        target.root.join("box.toml"),
        target.root.join(".git/config"),
        preferences_path(home),
        target.index_path.clone(),
    ] {
        let path = index::validate_path(&path)?;
        hash.update(path.as_os_str().as_encoded_bytes());
        match fs::read(&path) {
            Ok(bytes) => {
                hash.update(&bytes);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    if matches!(action, Action::RefreshAudit) {
        for r in vault.records()? {
            hash.update(
                fs::read(vault.root.join("meta").join(format!("{}.toml", r.id)))?.as_slice(),
            );
            hash.update(
                crate::hash::hash_directory(&vault.root.join("skills").join(r.id))?.as_bytes(),
            );
        }
    }
    if matches!(action, Action::InstallRouter) {
        // The confirmation covers what was previewed for each location: a
        // plan shown as an install is never applied as an upgrade, or the
        // other way round.
        for change in &plan.changes {
            hash.update(change.path.as_os_str().as_encoded_bytes());
            hash.update(change.reason_code.as_bytes());
        }
        hash.update(&[u8::from(plan.changed)]);
    }
    plan.token = hash.finalize().to_hex().to_string();
    Ok(plan)
}
/// One owned settings file. Roll back only our bytes/identity; preserve foreign replacements.
struct SettingsWrite {
    path: PathBuf,
    before: Option<Vec<u8>>,
    after: Vec<u8>,
    identity: fs::Metadata,
    finished: bool,
}
fn same(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    {
        a.len() == b.len() && a.modified().ok() == b.modified().ok()
    }
}
impl SettingsWrite {
    fn publish(path: PathBuf, after: Vec<u8>) -> Result<Self> {
        let path = index::validate_path(&path)?;
        let parent = path
            .parent()
            .ok_or_else(|| Error::new("INVALID_PATH", "Settings need a parent"))?;
        let before = match fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        fs::create_dir_all(parent)?;
        index::validate_path(parent)?;
        let temporary = parent.join(format!(
            ".skillsbox-settings-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&after)?;
        if let Ok(meta) = fs::symlink_metadata(&path) {
            file.set_permissions(meta.permissions())?;
        }
        file.sync_all()?;
        drop(file);
        let result = (|| -> Result<()> {
            if before
                .as_ref()
                .is_some_and(|bytes| fs::read(&path).ok().as_ref() != Some(bytes))
            {
                return Err(Error::new(
                    "SETTINGS_CHANGED",
                    "Settings changed during publication",
                ));
            }
            if before.is_some() {
                fs::rename(&temporary, &path)?;
            } else {
                crate::safe_rename::rename_no_replace(&temporary, &path)?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            let _ = fs::remove_file(&temporary);
            return Err(e);
        }
        let identity = fs::symlink_metadata(&path)?;
        Ok(Self {
            path,
            before,
            after,
            identity,
            finished: false,
        })
    }
    fn owned(&self) -> bool {
        index::validate_path(&self.path).is_ok()
            && fs::symlink_metadata(&self.path).is_ok_and(|m| same(&self.identity, &m))
            && fs::read(&self.path).is_ok_and(|b| b == self.after)
    }
}
impl Drop for SettingsWrite {
    fn drop(&mut self) {
        if !self.finished && self.owned() {
            if let Some(bytes) = &self.before {
                if let Some(parent) = self.path.parent() {
                    let temporary = parent.join(format!(
                        ".skillsbox-settings-restore-{}-{}",
                        std::process::id(),
                        NEXT.fetch_add(1, Ordering::Relaxed)
                    ));
                    let restore = (|| -> Result<()> {
                        let mut file = OpenOptions::new()
                            .create_new(true)
                            .write(true)
                            .open(&temporary)?;
                        file.write_all(bytes)?;
                        file.set_permissions(self.identity.permissions())?;
                        drop(file);
                        if self.owned() {
                            fs::rename(&temporary, &self.path)?;
                        }
                        Ok(())
                    })();
                    let _ = restore;
                    let _ = fs::remove_file(temporary);
                }
            } else {
                let _ = fs::remove_file(&self.path);
            }
        }
    }
}
pub fn apply(vault: &Vault, home: &Path, action: &Action, token: &str) -> Result<PathBuf> {
    apply_controlled(vault, home, action, token, None).map(|r| r.vault)
}
pub fn apply_controlled(
    vault: &Vault,
    home: &Path,
    action: &Action,
    token: &str,
    control: Option<&Control>,
) -> Result<Applied> {
    let _state_guard = crate::state::write_guard(&vault.index_path, false)?;
    if token.is_empty() {
        return Err(Error::decision(
            "SETTINGS_CONFIRMATION_REQUIRED",
            "Preview and confirm settings before writing",
        ));
    }
    let plan = preview(vault, home, action)?;
    if plan.token != token {
        return Err(Error::decision(
            "SETTINGS_PREVIEW_CHANGED",
            "Settings changed; review a new preview",
        ));
    }
    if let Some(control) = control {
        control.begin()?;
    }
    let target = match action {
        Action::Initialize { path } | Action::SelectVault { path } => {
            Vault::new(resolve_vault(home, path)?, vault.index_path.clone())
        }
        _ => vault.clone(),
    };
    if !plan.changed {
        return Ok(Applied {
            vault: target.root,
            committed: false,
            changed: false,
        });
    }
    let head_before = gitlog::head(&target.root).ok();
    match action {
        Action::Initialize { .. } | Action::SelectVault { .. } => {
            if matches!(action, Action::Initialize { .. }) {
                target.init(false)?;
            }
            let mut write = SettingsWrite::publish(
                preferences_path(home),
                preferences_bytes(home, &target.root)?,
            )?;
            write.finished = true;
        }
        Action::Save { .. } => {
            let config = candidate(vault, home, action)?;
            let mut write =
                SettingsWrite::publish(vault.root.join("box.toml"), config_bytes(vault, &config)?)?;
            let result =
                gitlog::commit_paths(&vault.root, "Update vault settings", &["box.toml".into()]);
            if result.is_err() {
                let _ = gitlog::unstage_paths(&vault.root, &["box.toml".into()]);
            }
            result?;
            if !write.owned() {
                return Err(Error::new(
                    "SETTINGS_CHANGED",
                    "Settings changed during commit",
                ));
            }
            write.finished = true;
        }
        Action::RebuildIndex => {
            doctor::run(vault, home, true, false)?;
        }
        Action::InstallRouter => {
            router::initialize(vault, home, false)?;
        }
        Action::RefreshAudit => {
            audit::refresh(vault, &[], false)?;
        }
        Action::Remote { url } => gitlog::configure_remote(&vault.root, url)?,
    }
    let committed = matches!(action, Action::Save { .. } | Action::RefreshAudit)
        && gitlog::head(&target.root).ok() != head_before;
    Ok(Applied {
        vault: target.root,
        committed,
        changed: true,
    })
}

/// `desktop.toml` key: the rules version whose re-audit prompt the user has
/// already answered on this computer.
const AUDIT_RULES_PROMPTED: &str = "audit_rules_prompted";

/// Number of stored audits made by older rules, when the app should offer the
/// re-audit preview at start. Reads only; first open writes nothing. Legacy
/// state (`~/.skillsbox`) is read-only until `migrate`, so nothing is offered
/// before the migration: the answer could not be recorded (review H1).
pub fn audit_rules_prompt(home: &Path, vault_root: &Path) -> Result<Option<usize>> {
    if !vault_root.join("box.toml").is_file() {
        return Ok(None);
    }
    if crate::state::directory(home)? != crate::state::current(home) {
        return Ok(None);
    }
    let older = audit::older_rules(vault_root)?;
    if older == 0 {
        return Ok(None);
    }
    let path = index::validate_path(&crate::state::directory(home)?.join("desktop.toml"))?;
    let prompted = match fs::read_to_string(&path) {
        Ok(text) => toml::from_str::<toml::Value>(&text)?
            .get(AUDIT_RULES_PROMPTED)
            .and_then(toml::Value::as_integer)
            .unwrap_or(0),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
        Err(e) => return Err(e.into()),
    };
    Ok((prompted < i64::from(audit::RULES_VERSION)).then_some(older))
}

/// Record that the user answered the prompt (opened the preview or chose
/// "not now"). Only an explicit click calls this; stored audits are untouched.
/// The write goes through the state guard: on legacy state it is refused with
/// `MIGRATION_REQUIRED` and `~/.kinana` is never created next to it (H1).
pub fn record_audit_rules_prompt(home: &Path) -> Result<()> {
    let _state_guard = crate::state::write_guard(&crate::state::index_path(home)?, false)?;
    let root = preferred_vault(home)?;
    let mut document: toml::Value = toml::from_str(
        std::str::from_utf8(&preferences_bytes(home, &root)?)
            .map_err(|e| Error::new("INVALID_DESKTOP_PREFERENCES", e.to_string()))?,
    )?;
    document
        .as_table_mut()
        .ok_or_else(|| Error::new("INVALID_DESKTOP_PREFERENCES", "Expected a settings table"))?
        .insert(
            AUDIT_RULES_PROMPTED.into(),
            toml::Value::Integer(i64::from(audit::RULES_VERSION)),
        );
    let mut write = SettingsWrite::publish(
        preferences_path(home),
        toml::to_string_pretty(&document)?.into_bytes(),
    )?;
    write.finished = true;
    Ok(())
}

pub fn validate_scan_paths(vault: &Vault, home: &Path, paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    if paths.is_empty() {
        return Err(Error::new("NO_SCAN_PATHS", "Choose explicit scan roots"));
    }
    paths
        .iter()
        .map(|p| {
            let p = resolve(home, p)?;
            if p == home || p.starts_with(&vault.root) || !p.is_dir() {
                return Err(Error::new(
                    "INVALID_SCAN_ROOT",
                    "Choose existing roots outside the vault, not the whole home",
                ));
            }
            Ok(p)
        })
        .collect()
}
#[derive(Default)]
pub struct Control(std::sync::atomic::AtomicU8);
impl Control {
    pub fn cancel(&self) -> bool {
        self.0
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
            || self.0.load(Ordering::SeqCst) == 1
    }
    pub fn is_committing(&self) -> bool {
        self.0.load(Ordering::SeqCst) == 2
    }
    fn begin(&self) -> Result<()> {
        self.0
            .compare_exchange(0, 2, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| ())
            .map_err(|_| Error::new("CANCELLED", "Settings cancelled before publication"))
    }
}

#[derive(Debug, Serialize)]
pub struct Applied {
    pub vault: PathBuf,
    pub committed: bool,
    pub changed: bool,
}
