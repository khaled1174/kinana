use kinana_core::{self as core, AgentInfo, Error, catalog::CatalogFilter, vault::Vault};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub struct Desktop {
    startup_error: Option<Error>,
    vault: Arc<Mutex<Vault>>,
    settings: Mutex<Option<Arc<SettingsJob>>>,
    scan: Mutex<Option<Arc<ScanJob>>>,
    import: Mutex<Option<Arc<ImportJob>>>,
    classification: Mutex<Option<Arc<ClassificationJob>>>,
    project: Mutex<Option<Arc<ProjectJob>>>,
    sources: Mutex<Option<Arc<SourceJob>>>,
}
impl Desktop {
    pub fn new(home: &Path) -> Self {
        // Existing state only: a fresh install writes nothing until the user
        // confirms a vault, and `init` creates the file then. A failure here
        // is reported by the first operation that needs the lease.
        if home.join(".kinana").is_dir() {
            let _ = core::state::ensure_lease_file(home);
        }
        Self {
            startup_error: core::state::index_path(home).err(),
            vault: Arc::new(Mutex::new(Vault::new(
                core::settings::preferred_vault(home)
                    .unwrap_or_else(|_| core::settings::default_vault(home)),
                core::state::index_path(home)
                    .unwrap_or_else(|_| home.join(".kinana").join(core::index::FILE_NAME)),
            ))),
            settings: Mutex::new(None),
            scan: Mutex::new(None),
            import: Mutex::new(None),
            classification: Mutex::new(None),
            project: Mutex::new(None),
            sources: Mutex::new(None),
        }
    }
    fn vault(&self) -> Result<Vault, Error> {
        if let Some(error) = &self.startup_error {
            return Err(error.clone());
        }
        let v = self.vault.lock().map_err(|_| busy())?;
        Ok(Vault::new(v.root.clone(), v.index_path.clone()))
    }
}
fn busy() -> Error {
    Error::new("BUSY", "Another operation is using the vault")
}
pub(crate) fn home() -> Result<PathBuf, Error> {
    // Test-only app data isolation; the provider CLI retains its normal HOME/auth.
    #[cfg(feature = "e2e")]
    if let Some(home) =
        std::env::var_os("KINANA_E2E_HOME").or_else(|| std::env::var_os("SKILLSBOX_E2E_HOME"))
    {
        return Ok(home.into());
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| Error::new("HOME_NOT_SET", "Home directory is unavailable"))
}
async fn work<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, Error> + Send + 'static,
) -> Result<T, Error> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| Error::new("DESKTOP_WORKER_FAILED", e.to_string()))?
}
#[derive(Serialize)]
pub struct Bootstrap {
    #[cfg(feature = "e2e")]
    test_pid: u32,
    version: &'static str,
    agents: Vec<AgentInfo>,
    home: PathBuf,
    vault: PathBuf,
    first_run: bool,
    startup_error: Option<Error>,
    /// The router names a CLI path that is no longer this app's: the app
    /// moved. The window offers the router preview; nothing is written here.
    router_moved: Option<RouterMoved>,
    /// Stored audits made by older rules, while the user has not answered the
    /// re-audit prompt on this computer (0.3). Computed by reading only.
    audit_rules: Option<AuditRules>,
}
#[derive(Serialize)]
pub struct AuditRules {
    older: usize,
    rules_version: u32,
}
#[derive(Serialize)]
pub struct RouterMoved {
    from: String,
    to: String,
}
fn inspect(home: &Path) -> Bootstrap {
    let preference = core::settings::preferred_vault(home);
    let root = preference
        .as_ref()
        .cloned()
        .unwrap_or_else(|_| core::settings::default_vault(home));
    let audit_rules = core::settings::audit_rules_prompt(home, &root)
        .ok()
        .flatten()
        .map(|older| AuditRules {
            older,
            rules_version: core::audit::RULES_VERSION,
        });
    Bootstrap {
        #[cfg(feature = "e2e")]
        test_pid: std::process::id(),
        home: home.into(),
        first_run: !root.join("box.toml").exists(),
        vault: root,
        startup_error: preference.err(),
        version: env!("CARGO_PKG_VERSION"),
        agents: core::agents::detect(home),
        router_moved: core::router::relocation(home).map(|(from, to)| RouterMoved { from, to }),
        audit_rules,
    }
}
/// The user answered the start prompt (opened the preview or chose "not
/// now"). Only this click writes; stored audits are not touched.
#[tauri::command]
pub async fn answer_audit_rules_prompt() -> Result<(), Error> {
    let home = home()?;
    work(move || core::settings::record_audit_rules_prompt(&home)).await
}
#[tauri::command]
pub fn bootstrap() -> Result<Bootstrap, Error> {
    Ok(inspect(&home()?))
}
#[tauri::command]
pub async fn catalog(
    state: tauri::State<'_, Desktop>,
    filter: CatalogFilter,
) -> Result<serde_json::Value, Error> {
    let v = state.vault()?;
    work(move || {
        // The first thing the window asks for, and never a preview: build
        // this version's index here after an upgrade from the 0.1.0 beta.
        // Best effort; a failure shows up as the index state of later commands.
        let _ = core::index::ensure_built(&v);
        core::diagnostics::catalog(&core::catalog::list(&v, &filter)?)
    })
    .await
}
#[tauri::command]
pub async fn connect_vault(
    state: tauri::State<'_, Desktop>,
    path: PathBuf,
) -> Result<serde_json::Value, Error> {
    if state.import.lock().map_err(|_| busy())?.is_some()
        || state.classification.lock().map_err(|_| busy())?.is_some()
    {
        return Err(busy());
    }
    if state.project.lock().map_err(|_| busy())?.is_some() {
        return Err(busy());
    }
    if state.sources.lock().map_err(|_| busy())?.is_some() {
        return Err(busy());
    }
    let v = Vault::new(path, core::state::index_path(&home()?)?);
    let _ = core::index::ensure_built(&v);
    let report = core::catalog::list(&v, &CatalogFilter::default())?;
    *state.vault.lock().map_err(|_| busy())? = v;
    core::diagnostics::catalog(&report)
}
#[tauri::command]
pub async fn skill_detail(
    state: tauri::State<'_, Desktop>,
    id: String,
) -> Result<serde_json::Value, Error> {
    let v = state.vault()?;
    work(move || core::diagnostics::detail(&core::catalog::detail(&v, &id)?)).await
}
#[tauri::command]
pub async fn pin_skill(
    state: tauri::State<'_, Desktop>,
    id: String,
    pinned: bool,
) -> Result<core::PinReport, Error> {
    let v = state.vault()?;
    work(move || core::edit::set_pinned(&v, &id, pinned, false)).await
}
#[tauri::command]
pub async fn category_skill(
    state: tauri::State<'_, Desktop>,
    id: String,
    category: String,
) -> Result<core::EditResult, Error> {
    let v = state.vault()?;
    work(move || core::edit::set_category(&v, &id, &category, false)).await
}
#[tauri::command]
pub async fn review_skill(
    state: tauri::State<'_, Desktop>,
    id: String,
    decision: String,
) -> Result<core::ReviewReport, Error> {
    let v = state.vault()?;
    let decision = match decision.as_str() {
        "accept" => core::ReviewDecision::Accept,
        "reject" => core::ReviewDecision::Reject,
        "original" => core::ReviewDecision::Original,
        _ => {
            return Err(Error::new(
                "INVALID_ARGUMENTS",
                "Choose accept, reject or original",
            ));
        }
    };
    work(move || core::review::review(&v, &id, decision, false)).await
}
/// Every pending description, read only, for the batch review list.
#[tauri::command]
pub async fn pending_descriptions(
    state: tauri::State<'_, Desktop>,
) -> Result<Vec<core::review::PendingDescription>, Error> {
    let v = state.vault()?;
    work(move || core::review::pending(&v)).await
}
/// Read-only preview of accepting several pending descriptions together.
#[tauri::command]
pub async fn preview_accept_descriptions(
    state: tauri::State<'_, Desktop>,
    ids: Vec<String>,
) -> Result<core::BatchReviewReport, Error> {
    let v = state.vault()?;
    work(move || core::review::accept_many(&v, &ids, None, true)).await
}
/// Accepts exactly the previewed batch in one commit; a changed state refuses
/// the token with `REVIEW_PREVIEW_CHANGED`.
#[tauri::command]
pub async fn accept_descriptions(
    state: tauri::State<'_, Desktop>,
    ids: Vec<String>,
    token: String,
) -> Result<core::BatchReviewReport, Error> {
    let v = state.vault()?;
    work(move || core::review::accept_many(&v, &ids, Some(&token), false)).await
}
#[tauri::command]
pub async fn undo_vault(state: tauri::State<'_, Desktop>) -> Result<core::UndoReport, Error> {
    let v = state.vault()?;
    work(move || v.undo(false)).await
}
/// Undoes the automatic description approval of a classification run, only
/// while its commit is still the latest vault change (0.3, r5): another
/// change after it is never reverted in its place.
#[tauri::command]
pub async fn undo_auto_approval(
    state: tauri::State<'_, Desktop>,
    commit: String,
) -> Result<core::UndoReport, Error> {
    let v = state.vault()?;
    work(move || {
        if v.undo(true)?.reverted_commit != commit {
            return Err(core::Error::decision(
                "AUTO_APPROVAL_NOT_LATEST",
                "The vault changed after the automatic approval; undo it from the vault history instead",
            ));
        }
        v.undo(false)
    })
    .await
}
/// Pi's providers and models from Pi itself (`pi --list-models`) behind the
/// classification checks; read only, no text sent (0.3, r5).
#[tauri::command]
pub async fn pi_models(
    state: tauri::State<'_, Desktop>,
) -> Result<core::agent_runner::pi::PiModelList, Error> {
    let v = state.vault()?;
    work(move || core::agent_runner::pi::list_models(&v)).await
}

fn classify_options(
    id: Option<String>,
    all: Option<bool>,
    pending: Option<bool>,
    ids: Option<Vec<String>>,
    agent: String,
    dry_run: bool,
) -> core::ClassifyOptions {
    // After a stop the window continues with exactly the remaining skills,
    // never the whole vault again (review of 18ea598, finding 1).
    let pending = ids.is_none() && pending.unwrap_or(false);
    let (ids, all) = match ids {
        Some(ids) => (ids, false),
        None => (id.into_iter().collect(), all.unwrap_or(false)),
    };
    core::ClassifyOptions {
        ids,
        all,
        pending,
        agent,
        confirmed: true,
        dry_run,
        ..Default::default()
    }
}
/// The saved agent, or the user's explicit choice from the window after
/// Codex was refused or `auto` stopped (only Claude or keywords). The window
/// sends back the agent its preview used; a saved agent that changed since
/// is refused instead of running another behaviour (0.3).
fn chosen_agent(v: &Vault, choice: Option<String>) -> Result<String, Error> {
    let saved = v.config()?.classification_agent;
    match choice.as_deref() {
        None => Ok(saved),
        Some(agent @ ("claude" | "keywords")) => Ok(agent.to_owned()),
        Some(agent) if agent == saved => Ok(saved),
        Some(_) => Err(Error::decision(
            "CLASSIFY_AGENT_CHANGED",
            "The classification agent changed after the preview; review the new preview",
        )),
    }
}
#[tauri::command]
pub async fn preview_classify(
    state: tauri::State<'_, Desktop>,
    id: Option<String>,
    all: Option<bool>,
    pending: Option<bool>,
    ids: Option<Vec<String>>,
    agent: Option<String>,
) -> Result<serde_json::Value, Error> {
    let v = state.vault()?;
    let agent = chosen_agent(&v, agent)?;
    work(move || {
        let mut value = core::diagnostics::classification(&core::classify::classify(
            &v,
            &classify_options(id, all, pending, ids, agent.clone(), true),
        )?)?;
        if let Some(object) = value.as_object_mut() {
            object.insert("agent".into(), serde_json::Value::String(agent));
        }
        Ok(value)
    })
    .await
}
struct ClassificationJob {
    control: core::classify::ClassifyControl,
    state: Mutex<ClassificationState>,
}
#[derive(Default)]
struct ClassificationState {
    report: Option<core::ClassifyReport>,
    error: Option<Error>,
    done: bool,
}
#[derive(Serialize)]
pub struct ClassificationStatus {
    report: Option<serde_json::Value>,
    error: Option<serde_json::Value>,
    done: bool,
    committing: bool,
    progress: core::classify::ClassifyProgress,
}
// The window's named invoke arguments are the IPC contract.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn start_classify(
    state: tauri::State<'_, Desktop>,
    id: Option<String>,
    all: Option<bool>,
    pending: Option<bool>,
    estimate: core::TokenEstimate,
    confirmed: bool,
    ids: Option<Vec<String>>,
    agent: Option<String>,
    pi_confirmation: Option<String>,
) -> Result<(), Error> {
    let mut current = state.classification.lock().map_err(|_| busy())?;
    if current.is_some() {
        return Err(busy());
    }
    let v = state.vault()?;
    let agent = chosen_agent(&v, agent)?;
    let job = Arc::new(ClassificationJob {
        control: Default::default(),
        state: Mutex::new(Default::default()),
    });
    *current = Some(job.clone());
    tauri::async_runtime::spawn_blocking(move || {
        let mut options = classify_options(id, all, pending, ids, agent, false);
        options.confirmed = confirmed;
        // The Pi confirmation the user accepted in the window (0.3); core
        // refuses one for other settings or skills before any call.
        options.pi_confirmation = pi_confirmation;
        let result = core::classify::classify_confirmed(&v, &options, &estimate, &job.control);
        let mut s = job.state.lock().unwrap_or_else(|e| e.into_inner());
        match result {
            Ok(report) => s.report = Some(report),
            Err(error) => s.error = Some(error),
        }
        s.done = true;
    });
    Ok(())
}
#[tauri::command]
pub fn classify_status(state: tauri::State<'_, Desktop>) -> Result<ClassificationStatus, Error> {
    let job = state
        .classification
        .lock()
        .map_err(|_| busy())?
        .clone()
        .ok_or_else(|| Error::new("NO_CLASSIFICATION", "No classification is running"))?;
    let s = job.state.lock().map_err(|_| busy())?;
    Ok(ClassificationStatus {
        report: s
            .report
            .as_ref()
            .map(core::diagnostics::classification)
            .transpose()
            .map_err(|e| Error::new("SERIALIZE_ERROR", e.to_string()))?,
        error: s
            .error
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| Error::new("SERIALIZE_ERROR", e.to_string()))?,
        done: s.done,
        committing: job.control.is_committing(),
        progress: job.control.progress(),
    })
}
#[tauri::command]
pub fn cancel_classify(state: tauri::State<'_, Desktop>) -> Result<bool, Error> {
    let mut current = state.classification.lock().map_err(|_| busy())?;
    // A running job stops before its next skill and saves the completed ones;
    // it stays so classify_status can report how far it got. A finished job
    // is cleared.
    if let Some(job) = current.as_ref()
        && !job.state.lock().map_err(|_| busy())?.done
    {
        return Ok(job.control.cancel());
    }
    *current = None;
    Ok(true)
}

/// The only Finder entry point. No frontend path, file/URL opening, plugin registration or opener capability.
#[tauri::command]
pub async fn reveal_skill_folder(
    state: tauri::State<'_, Desktop>,
    id: String,
) -> Result<(), Error> {
    let v = state.vault()?;
    work(move || {
        let directory = core::catalog::skill_directory(&v, &id)?;
        tauri_plugin_opener::reveal_item_in_dir(directory)
            .map_err(|e| Error::new("REVEAL_FAILED", e.to_string()))
    })
    .await
}
#[tauri::command]
pub async fn vault_bundles(state: tauri::State<'_, Desktop>) -> Result<Vec<core::Bundle>, Error> {
    let v = state.vault()?;
    work(move || core::bundles::list(&v)).await
}
#[tauri::command]
pub async fn vault_groups(state: tauri::State<'_, Desktop>) -> Result<serde_json::Value, Error> {
    let v = state.vault()?;
    work(move || core::diagnostics::groups(&core::groups::discover(&v)?)).await
}
#[tauri::command]
pub async fn save_bundle(
    state: tauri::State<'_, Desktop>,
    bundle: core::Bundle,
    editing: bool,
    dry_run: bool,
) -> Result<serde_json::Value, Error> {
    let v = state.vault()?;
    work(move || {
        let name = bundle.name.clone();
        core::diagnostics::bundle(&core::bundles::save_definition(
            &v,
            editing.then_some(name.as_str()),
            bundle,
            dry_run,
        )?)
    })
    .await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportSelection {
    pub reviewed: std::collections::BTreeMap<std::path::PathBuf, (String, String)>,
    pub select: Vec<String>,
    pub allow_risky: bool,
    pub suffix: Option<String>,
    /// Scan only: the chosen version for each name found with different contents.
    #[serde(default)]
    pub primary: Vec<String>,
}
impl ImportSelection {
    fn verify_preview(&self, report: Option<&core::ImportReport>) -> Result<(), Error> {
        let changed = || {
            Error::decision(
                "IMPORT_PREVIEW_CHANGED",
                "Review a new import preview before confirming.",
            )
        };
        let report = report.ok_or_else(changed)?;
        if self.select.is_empty() || self.select.len() != self.reviewed.len() {
            return Err(changed());
        }
        let shown: std::collections::BTreeMap<_, _> =
            report.items.iter().map(|i| (&i.path, i)).collect();
        for path in &self.select {
            let path = PathBuf::from(path);
            let item = shown.get(&path).ok_or_else(changed)?;
            let (hash, token) = self.reviewed.get(&path).ok_or_else(changed)?;
            if item.content_hash.as_ref() != Some(hash) || item.review_token.as_ref() != Some(token)
            {
                return Err(changed());
            }
        }
        Ok(())
    }

    fn options(&self, dry_run: bool) -> core::ImportOptions {
        core::ImportOptions {
            dry_run,
            reviewed: Some(self.reviewed.clone()),
            select: self.select.clone(),
            allow_risky: self.allow_risky,
            id_suffix: self.suffix.clone(),
            primary: self.primary.clone(),
            ..Default::default()
        }
    }
}
#[tauri::command]
pub async fn preview_import(
    state: tauri::State<'_, Desktop>,
    input: String,
) -> Result<serde_json::Value, Error> {
    let v = state.vault()?;
    let h = home()?;
    work(move || {
        core::diagnostics::import(&core::import::add(
            &v,
            &input,
            &core::state::directory(&h)?.join("imports"),
            &core::ImportOptions {
                dry_run: true,
                ..Default::default()
            },
        )?)
    })
    .await
}
struct ImportJob {
    control: core::import::ImportControl,
    state: Mutex<ImportState>,
    vault: Vault,
    input: String,
}
struct ImportState {
    source: Option<core::sources::PreparedSource>,
    report: Option<core::ImportReport>,
    error: Option<Error>,
    done: bool,
}
#[derive(Serialize)]
pub struct ImportStatus {
    prepared: bool,
    stage: String,
    done: bool,
    report: Option<serde_json::Value>,
    error: Option<serde_json::Value>,
}
#[tauri::command]
pub fn prepare_import(state: tauri::State<'_, Desktop>, input: String) -> Result<(), Error> {
    let mut current = state.import.lock().map_err(|_| busy())?;
    if current.is_some() {
        return Err(busy());
    }
    let job = Arc::new(ImportJob {
        control: Default::default(),
        state: Mutex::new(ImportState {
            source: None,
            report: None,
            error: None,
            done: false,
        }),
        vault: state.vault()?,
        input,
    });
    *current = Some(job.clone());
    let h = home()?;
    let import_directory = core::state::directory(&h)?.join("imports");
    tauri::async_runtime::spawn_blocking(move || {
        let result =
            core::import::prepare_source(&job.vault, &job.input, &import_directory, &job.control)
                .and_then(|source| {
                    let report = core::import::add_prepared(
                        &job.vault,
                        &job.input,
                        &source,
                        &core::ImportOptions {
                            dry_run: true,
                            ..Default::default()
                        },
                        Some(&job.control),
                    )?;
                    Ok((source, report))
                });
        let mut s = job.state.lock().unwrap_or_else(|e| e.into_inner());
        match result {
            Ok((source, report)) => {
                s.source = Some(source);
                s.report = Some(report);
            }
            Err(error) => s.error = Some(error),
        }
        s.done = true;
    });
    Ok(())
}
#[tauri::command]
pub fn import_status(state: tauri::State<'_, Desktop>) -> Result<ImportStatus, Error> {
    let job = state
        .import
        .lock()
        .map_err(|_| busy())?
        .clone()
        .ok_or_else(|| Error::new("NO_IMPORT", "No prepared import"))?;
    let s = job.state.lock().map_err(|_| busy())?;
    Ok(ImportStatus {
        prepared: s.source.is_some(),
        stage: job.control.progress(),
        done: s.done,
        report: s
            .report
            .as_ref()
            .map(core::diagnostics::import)
            .transpose()
            .map_err(|e| Error::new("SERIALIZE_ERROR", e.to_string()))?,
        error: s
            .error
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| Error::new("SERIALIZE_ERROR", e.to_string()))?,
    })
}
#[tauri::command]
pub fn cancel_import(state: tauri::State<'_, Desktop>) -> Result<bool, Error> {
    let mut current = state.import.lock().map_err(|_| busy())?;
    let Some(job) = current.as_ref() else {
        return Ok(true);
    };
    if job.state.lock().map_err(|_| busy())?.done {
        *current = None;
        return Ok(true);
    }
    if !job.control.cancel() {
        return Ok(false);
    }
    // Dropping the UI session also drops any prepared source after its worker finishes.
    *current = None;
    Ok(true)
}
#[tauri::command]
pub fn commit_import(
    state: tauri::State<'_, Desktop>,
    selection: ImportSelection,
) -> Result<(), Error> {
    let job = state
        .import
        .lock()
        .map_err(|_| busy())?
        .clone()
        .ok_or_else(|| Error::new("NO_IMPORT", "No prepared import"))?;
    {
        let mut s = job.state.lock().map_err(|_| busy())?;
        if !s.done || s.source.is_none() {
            return Err(busy());
        }
        selection.verify_preview(s.report.as_ref())?;
        s.done = false;
        s.error = None;
    }
    tauri::async_runtime::spawn_blocking(move || {
        let source = job
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .source
            .take();
        let result = source
            .as_ref()
            .ok_or_else(|| Error::new("NO_IMPORT", "Prepared source missing"))
            .and_then(|source| {
                core::import::add_prepared(
                    &job.vault,
                    &job.input,
                    source,
                    &selection.options(false),
                    Some(&job.control),
                )
            });
        let mut s = job.state.lock().unwrap_or_else(|e| e.into_inner());
        match result {
            Ok(report) => s.report = Some(report),
            Err(error) => {
                let _ = job.control.stage("inspecting");
                s.source = source;
                s.error = Some(error);
            }
        }
        s.done = true;
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_uses_core_agent_detection_without_creating_home() {
        let home =
            std::env::temp_dir().join(format!("kinana-desktop-readonly-{}", std::process::id()));
        assert!(!home.exists());
        let actual = inspect(&home);
        let expected = core::agents::detect(&home);
        assert_eq!(
            serde_json::to_value(actual.agents).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        assert_eq!(actual.version, env!("CARGO_PKG_VERSION"));
        assert!(actual.audit_rules.is_none());
        assert!(!home.exists(), "read-only bootstrap created files");
    }
}

#[tauri::command]
pub async fn projects(
    state: tauri::State<'_, Desktop>,
    selected: Vec<PathBuf>,
) -> Result<Vec<serde_json::Value>, Error> {
    let v = state.vault()?;
    work(move || {
        core::projects::list(&v, &selected)?
            .iter()
            .map(core::diagnostics::project)
            .collect()
    })
    .await
}
#[tauri::command]
pub async fn preview_project(
    state: tauri::State<'_, Desktop>,
    request: core::projects::Request,
) -> Result<serde_json::Value, Error> {
    let v = state.vault()?;
    let h = home()?;
    work(move || core::diagnostics::project_plan(&core::projects::preview(&v, &h, &request)?)).await
}
struct ProjectJob {
    control: core::projects::Control,
    result: Mutex<ProjectResult>,
}
#[derive(Default, Serialize)]
struct ProjectResult {
    done: bool,
    report: Option<core::projects::Preview>,
    error: Option<Error>,
}
#[derive(Serialize)]
pub struct ProjectProgress {
    done: bool,
    committing: bool,
    report: Option<serde_json::Value>,
    error: Option<serde_json::Value>,
}
#[tauri::command]
pub fn start_project(
    state: tauri::State<'_, Desktop>,
    request: core::projects::Request,
    token: String,
    confirmed: bool,
) -> Result<(), Error> {
    let mut current = state.project.lock().map_err(|_| busy())?;
    if current.is_some() {
        return Err(busy());
    }
    let v = state.vault()?;
    let h = home()?;
    let job = Arc::new(ProjectJob {
        control: Default::default(),
        result: Mutex::new(Default::default()),
    });
    *current = Some(job.clone());
    tauri::async_runtime::spawn_blocking(move || {
        let result = core::projects::apply(
            &v,
            &h,
            &request,
            if confirmed { &token } else { "" },
            &job.control,
        );
        let mut state = job.result.lock().unwrap_or_else(|e| e.into_inner());
        match result {
            Ok(report) => state.report = Some(report),
            Err(error) => state.error = Some(error),
        }
        state.done = true;
    });
    Ok(())
}
#[tauri::command]
pub fn project_status(state: tauri::State<'_, Desktop>) -> Result<ProjectProgress, Error> {
    let job = state
        .project
        .lock()
        .map_err(|_| busy())?
        .clone()
        .ok_or_else(|| Error::new("NO_PROJECT_JOB", "No project operation is running"))?;
    let result = job.result.lock().map_err(|_| busy())?;
    let serialize = |value| core::diagnostics::project_plan(value);
    Ok(ProjectProgress {
        done: result.done,
        committing: job.control.is_committing(),
        report: result.report.as_ref().map(&serialize).transpose()?,
        error: result
            .error
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| Error::new("JSON_ERROR", e.to_string()))?,
    })
}
#[tauri::command]
pub fn cancel_project(state: tauri::State<'_, Desktop>) -> Result<bool, Error> {
    let mut current = state.project.lock().map_err(|_| busy())?;
    if let Some(job) = current.as_ref()
        && !job.result.lock().map_err(|_| busy())?.done
    {
        return Ok(job.control.cancel());
    }
    *current = None;
    Ok(true)
}

#[tauri::command]
pub async fn desktop_settings(
    state: tauri::State<'_, Desktop>,
) -> Result<core::settings::Snapshot, Error> {
    let v = state.vault()?;
    let h = home()?;
    work(move || core::settings::read(&v, &h)).await
}
/// An unsaved edit of the categories and definitions in the settings.
#[derive(Deserialize)]
pub struct TaxonomyDraft {
    pub categories: Vec<String>,
    pub definitions: core::taxonomy::Definitions,
}
#[derive(Serialize)]
pub struct TaxonomyView {
    pub effective: core::taxonomy::Effective,
    /// The instructions the product sends with this draft.
    pub preview: Option<core::agent_runner::PromptPreview>,
    pub error: Option<Error>,
}
/// Read only: the definitions in effect and the exact instruction text for
/// the saved settings or an unsaved draft. Writes nothing.
#[tauri::command]
pub async fn classification_guide(
    state: tauri::State<'_, Desktop>,
    draft: Option<TaxonomyDraft>,
) -> Result<TaxonomyView, Error> {
    let v = state.vault()?;
    work(move || {
        let mut config = v.config()?;
        if let Some(draft) = draft {
            config.categories = draft.categories;
            config.definitions = draft.definitions;
        }
        let effective = core::taxonomy::effective(&config);
        // Definitions are checked even while template 1 does not send them:
        // the settings refuse to save what template 2 could not use.
        let checked = core::taxonomy::validate(&core::taxonomy::in_effect(&config));
        Ok(match core::agent_runner::prompt_preview(&config) {
            Ok(preview) => TaxonomyView {
                effective,
                preview: Some(preview),
                error: checked.err(),
            },
            Err(error) => TaxonomyView {
                effective,
                preview: None,
                error: Some(error),
            },
        })
    })
    .await
}
#[tauri::command]
pub fn vault_location_warnings(path: PathBuf) -> Result<Vec<String>, Error> {
    core::settings::vault_location_warning_codes(&home()?, &path)
}
#[tauri::command]
pub async fn agent_skill_locations(
    agents: Vec<String>,
) -> Result<core::agents::AgentSkillLocations, Error> {
    let h = home()?;
    work(move || core::agents::skill_locations(&h, &agents)).await
}
#[tauri::command]
pub async fn preview_settings(
    state: tauri::State<'_, Desktop>,
    action: core::settings::Action,
) -> Result<core::settings::Plan, Error> {
    let v = state.vault()?;
    let h = home()?;
    work(move || core::settings::preview(&v, &h, &action)).await
}
struct SettingsJob {
    control: core::settings::Control,
    result: Mutex<SettingsResult>,
}
#[derive(Default, Serialize)]
struct SettingsResult {
    done: bool,
    committed: bool,
    vault: Option<PathBuf>,
    error: Option<Error>,
}
#[derive(Serialize)]
pub struct SettingsStatus {
    done: bool,
    committed: bool,
    committing: bool,
    vault: Option<PathBuf>,
    error: Option<serde_json::Value>,
}
#[tauri::command]
pub fn start_settings(
    state: tauri::State<'_, Desktop>,
    action: core::settings::Action,
    token: String,
    confirmed: bool,
) -> Result<(), Error> {
    let mut current = state.settings.lock().map_err(|_| busy())?;
    if current.is_some() {
        return Err(busy());
    }
    let v = state.vault()?;
    let h = home()?;
    let destination = state.vault.clone();
    let job = Arc::new(SettingsJob {
        control: Default::default(),
        result: Mutex::new(Default::default()),
    });
    *current = Some(job.clone());
    tauri::async_runtime::spawn_blocking(move || {
        let result = core::settings::apply_controlled(
            &v,
            &h,
            &action,
            if confirmed { &token } else { "" },
            Some(&job.control),
        );
        let mut state = job.result.lock().unwrap_or_else(|e| e.into_inner());
        match result {
            Ok(applied) => {
                *destination.lock().unwrap_or_else(|e| e.into_inner()) =
                    Vault::new(applied.vault.clone(), v.index_path);
                state.vault = Some(applied.vault);
                state.committed = applied.committed;
            }
            Err(error) => state.error = Some(error),
        }
        state.done = true;
    });
    Ok(())
}
#[tauri::command]
pub fn settings_status(state: tauri::State<'_, Desktop>) -> Result<SettingsStatus, Error> {
    let job = state
        .settings
        .lock()
        .map_err(|_| busy())?
        .clone()
        .ok_or_else(|| Error::new("NO_SETTINGS_JOB", "No settings operation"))?;
    let result = job.result.lock().map_err(|_| busy())?;
    Ok(SettingsStatus {
        done: result.done,
        committed: result.committed,
        committing: job.control.is_committing(),
        vault: result.vault.clone(),
        error: result
            .error
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| Error::new("JSON_ERROR", e.to_string()))?,
    })
}
#[tauri::command]
pub fn cancel_settings(state: tauri::State<'_, Desktop>) -> Result<bool, Error> {
    let mut current = state.settings.lock().map_err(|_| busy())?;
    if let Some(job) = current.as_ref()
        && !job.result.lock().map_err(|_| busy())?.done
    {
        return Ok(job.control.cancel());
    }
    *current = None;
    Ok(true)
}
#[tauri::command]
pub async fn desktop_doctor(state: tauri::State<'_, Desktop>) -> Result<core::DoctorReport, Error> {
    let v = state.vault()?;
    let h = home()?;
    work(move || core::doctor::check(&v, &h)).await
}
struct ScanJob {
    control: core::import::ImportControl,
    result: Mutex<ScanResult>,
    vault: Vault,
    roots: Vec<PathBuf>,
}
#[derive(Default)]
struct ScanResult {
    done: bool,
    imported: bool,
    report: Option<core::ImportReport>,
    error: Option<Error>,
}
#[derive(Serialize)]
pub struct ScanStatus {
    done: bool,
    imported: bool,
    stage: String,
    /// Skills done and selected skills in a per-skill stage of an import.
    completed: usize,
    total: usize,
    report: Option<serde_json::Value>,
    error: Option<serde_json::Value>,
}
#[tauri::command]
pub fn start_scan(
    state: tauri::State<'_, Desktop>,
    roots: Vec<PathBuf>,
    reuse: Option<bool>,
) -> Result<(), Error> {
    let mut current = state.scan.lock().map_err(|_| busy())?;
    let v = state.vault()?;
    let roots = core::settings::validate_scan_paths(&v, &home()?, &roots)?;
    if let Some(job) = current.as_ref() {
        let mut result = job.result.lock().map_err(|_| busy())?;
        if !result.done {
            return Err(busy());
        }
        // A closed review keeps its finished scan: reopening the same folders
        // returns to the same list. Import re-reads every source anyway.
        if reuse.unwrap_or(false)
            && !result.imported
            && result.report.is_some()
            && job.roots == roots
            && job.vault.root == v.root
        {
            // The outcome of an earlier import attempt belongs to that attempt.
            result.error = None;
            return Ok(());
        }
    }
    let job = Arc::new(ScanJob {
        control: scan_control(),
        result: Mutex::new(Default::default()),
        vault: v,
        roots,
    });
    *current = Some(job.clone());
    tauri::async_runtime::spawn_blocking(move || {
        let result = core::import::import_scan_controlled(
            &job.vault,
            &job.roots,
            &core::ImportOptions {
                dry_run: true,
                ..Default::default()
            },
            Some(&job.control),
        );
        let mut state = job.result.lock().unwrap_or_else(|e| e.into_inner());
        match result {
            Ok(report) => state.report = Some(report),
            Err(error) => state.error = Some(error),
        }
        state.done = true;
    });
    Ok(())
}
#[tauri::command]
pub fn commit_scan(
    state: tauri::State<'_, Desktop>,
    selection: ImportSelection,
    confirmed: bool,
) -> Result<(), Error> {
    if !confirmed {
        return Err(Error::decision(
            "SCAN_CONFIRMATION_REQUIRED",
            "Preview scan results and confirm selected imports",
        ));
    }
    let job = state
        .scan
        .lock()
        .map_err(|_| busy())?
        .clone()
        .ok_or_else(|| Error::new("NO_SCAN", "No scan preview"))?;
    {
        let mut result = job.result.lock().map_err(|_| busy())?;
        if !result.done || result.report.is_none() || result.imported {
            return Err(busy());
        }
        selection.verify_preview(result.report.as_ref())?;
        result.done = false;
        result.error = None;
        // A cancelled or failed import leaves the preview for another try.
        job.control.reset();
    }
    tauri::async_runtime::spawn_blocking(move || {
        let result = core::import::import_scan_controlled(
            &job.vault,
            &job.roots,
            &selection.options(false),
            Some(&job.control),
        );
        let mut state = job.result.lock().unwrap_or_else(|e| e.into_inner());
        match result {
            Ok(report) => {
                state.report = Some(report);
                state.imported = true;
            }
            Err(error) => state.error = Some(error),
        }
        state.done = true;
    });
    Ok(())
}
#[tauri::command]
pub fn scan_status(state: tauri::State<'_, Desktop>) -> Result<ScanStatus, Error> {
    let job = state
        .scan
        .lock()
        .map_err(|_| busy())?
        .clone()
        .ok_or_else(|| Error::new("NO_SCAN", "No scan operation"))?;
    let result = job.result.lock().map_err(|_| busy())?;
    let (completed, total) = job.control.counts();
    Ok(ScanStatus {
        done: result.done,
        imported: result.imported,
        stage: job.control.progress(),
        completed,
        total,
        report: result
            .report
            .as_ref()
            .map(core::diagnostics::import)
            .transpose()?,
        error: result
            .error
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| Error::new("JSON_ERROR", e.to_string()))?,
    })
}
#[tauri::command]
/// Stops a running scan or import before publication. The finished scan
/// stays, so a cancelled import returns to the same list; false while an
/// import is already being saved.
pub fn cancel_scan(state: tauri::State<'_, Desktop>) -> Result<bool, Error> {
    let current = state.scan.lock().map_err(|_| busy())?;
    if let Some(job) = current.as_ref()
        && !job.result.lock().map_err(|_| busy())?.done
    {
        return Ok(job.control.cancel());
    }
    Ok(true)
}
fn scan_control() -> core::import::ImportControl {
    // Test-only pause per staged skill, to observe progress and cancellation.
    #[cfg(feature = "e2e")]
    if let Some(delay) = std::env::var("KINANA_E2E_IMPORT_ITEM_DELAY_MS")
        .ok()
        .and_then(|value| value.parse().ok())
    {
        return core::import::ImportControl::with_item_delay(std::time::Duration::from_millis(
            delay,
        ));
    }
    Default::default()
}

// Thin asynchronous boundary; source tracking, review tokens and publication
// remain entirely in kinana-core.
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "lowercase", deny_unknown_fields)]
pub enum SourceRequest {
    Check,
    Update {
        options: core::source_updates::UpdateOptions,
    },
}
struct SourceJob {
    id: String,
    control: core::import::ImportControl,
    result: Mutex<SourceResult>,
}
#[derive(Default)]
struct SourceResult {
    report: Option<serde_json::Value>,
    error: Option<Error>,
    done: bool,
}
#[derive(Serialize)]
pub struct SourceProgress {
    done: bool,
    committing: bool,
    stage: String,
    report: Option<serde_json::Value>,
    error: Option<serde_json::Value>,
}
fn source_slot_available(previous: Option<&SourceJob>) -> Result<bool, Error> {
    match previous {
        Some(job) => Ok(job.result.lock().map_err(|_| busy())?.done),
        None => Ok(true),
    }
}

#[tauri::command]
pub fn start_sources(
    state: tauri::State<'_, Desktop>,
    request: SourceRequest,
    id: String,
) -> Result<(), Error> {
    let mut current = state.sources.lock().map_err(|_| busy())?;
    if !source_slot_available(current.as_deref())? {
        return Err(busy());
    }
    let vault = state.vault()?;
    let job = Arc::new(SourceJob {
        id,
        control: Default::default(),
        result: Mutex::new(Default::default()),
    });
    *current = Some(job.clone());
    tauri::async_runtime::spawn_blocking(move || {
        let result = match request {
            SourceRequest::Check => core::source_updates::outdated(&vault, Some(&job.control))
                .and_then(|r| {
                    serde_json::to_value(r).map_err(|e| Error::new("JSON_ERROR", e.to_string()))
                }),
            SourceRequest::Update { options } => {
                core::source_updates::update(&vault, &options, Some(&job.control)).and_then(|r| {
                    serde_json::to_value(r).map_err(|e| Error::new("JSON_ERROR", e.to_string()))
                })
            }
        };
        let mut state = job.result.lock().unwrap_or_else(|e| e.into_inner());
        match result {
            Ok(report) => state.report = Some(report),
            Err(error) => state.error = Some(error),
        }
        state.done = true;
    });
    Ok(())
}
#[tauri::command]
pub fn sources_status(
    state: tauri::State<'_, Desktop>,
    id: String,
) -> Result<SourceProgress, Error> {
    let job = state
        .sources
        .lock()
        .map_err(|_| busy())?
        .clone()
        .filter(|job| job.id == id)
        .ok_or_else(|| Error::new("NO_SOURCE_JOB", "No source operation is running"))?;
    let result = job.result.lock().map_err(|_| busy())?;
    let stage = job.control.progress();
    Ok(SourceProgress {
        done: result.done,
        committing: stage == "committing" && !result.done,
        stage,
        report: result.report.clone(),
        error: result
            .error
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| Error::new("JSON_ERROR", e.to_string()))?,
    })
}
#[tauri::command]
pub fn cancel_sources(state: tauri::State<'_, Desktop>, id: String) -> Result<bool, Error> {
    let mut current = state.sources.lock().map_err(|_| busy())?;
    if current.as_ref().is_none_or(|job| job.id != id) {
        return Ok(true);
    }
    if let Some(job) = current.as_ref()
        && !job.result.lock().map_err(|_| busy())?.done
        && !job.control.cancel()
    {
        return Ok(false);
    }
    *current = None;
    Ok(true)
}

#[cfg(test)]
mod batch_classification_tests {
    use super::*;

    #[test]
    fn classification_status_includes_batch_progress() {
        let status = ClassificationStatus {
            report: None,
            error: None,
            done: false,
            committing: false,
            progress: Default::default(),
        };
        let value = serde_json::to_value(status).unwrap();
        assert_eq!(value["progress"]["completed"], 0);
        assert_eq!(value["progress"]["total"], 0);
        assert!(value["progress"]["current_id"].is_null());
    }
}

#[cfg(test)]
mod source_job_tests {
    use super::*;
    #[test]
    fn abandoned_completed_source_check_allows_a_new_job_but_active_check_stays_busy() {
        let job = SourceJob {
            id: "first".into(),
            result: Mutex::new(Default::default()),
            control: Default::default(),
        };
        assert!(!source_slot_available(Some(&job)).unwrap());
        job.result.lock().unwrap().done = true;
        assert!(source_slot_available(Some(&job)).unwrap());
    }
}

#[cfg(test)]
mod classify_follow_up_tests {
    use super::classify_options;

    /// Review of 18ea598, finding 1: after a stop the window continues with
    /// the remaining skills only, never the whole vault.
    #[test]
    fn remaining_ids_replace_a_whole_vault_selection() {
        let options = classify_options(
            None,
            Some(true),
            Some(true),
            Some(vec!["e".into(), "f".into()]),
            "keywords".into(),
            false,
        );
        assert!(!options.all);
        assert!(!options.pending);
        assert_eq!(options.ids, ["e", "f"]);
        let first = classify_options(Some("a".into()), None, None, None, "auto".into(), true);
        assert_eq!(first.ids, ["a"]);
        assert!(first.dry_run);
        let pending = classify_options(None, None, Some(true), None, "claude".into(), true);
        assert!(pending.pending);
        assert!(!pending.all);
        assert!(pending.ids.is_empty());
    }
}
