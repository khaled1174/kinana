//! Explicit, fingerprint-bound state/lock migration. Originals and journals survive.
use crate::{Error, Result, index, lockfile, safe_rename, state};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const RECORD: &str = ".kinana-state-v1.json";
const JOURNALS: &str = ".kinana-migrations";
const HEAD: &str = ".kinana-migration-head";
const OWNER: &[u8] = b"Kinana migration recovery v1\n";
static NEXT_OPERATION: AtomicU64 = AtomicU64::new(0);
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Node {
    kind: String,
    identity: String,
    mode: u32,
    hash: Option<String>,
}
type Tree = BTreeMap<PathBuf, Node>;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LockChange {
    pub project: PathBuf,
    pub from: PathBuf,
    pub to: PathBuf,
    pub state: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RouterChange {
    pub agent: String,
    pub from: PathBuf,
    pub to: PathBuf,
    pub state: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StateChange {
    pub from: PathBuf,
    pub to: Option<PathBuf>,
    pub kind: String,
    pub operation: String,
    pub mode: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub version: u32,
    pub action: String,
    pub source_state: PathBuf,
    pub target_state: PathBuf,
    pub journal: Option<PathBuf>,
    pub preview_token: String,
    pub changed: bool,
    pub dry_run: bool,
    pub projects: Vec<LockChange>,
    pub routers: Vec<RouterChange>,
    pub state_files: Vec<StateChange>,
    pub control_paths: Vec<PathBuf>,
    pub warnings: Vec<String>,
    pub reason_codes: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StateRecord {
    version: u32,
    id: String,
    source: Option<Tree>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedLock {
    change: LockChange,
    original: Node,
    parent: Node,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedRouter {
    change: RouterChange,
    old: Tree,
    new_before: Option<Tree>,
    prepared: Option<Tree>,
    parent: Node,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    version: u32,
    home: PathBuf,
    home_identity: Node,
    id: String,
    source: Option<Tree>,
    new_state: Option<Node>,
    locks: Vec<SavedLock>,
    routers: Vec<SavedRouter>,
    pending: Node,
    previous_head: Option<Vec<u8>>,
}
struct Forward {
    report: Report,
    source: Option<Tree>,
    locks: Vec<SavedLock>,
    routers: Vec<SavedRouter>,
    copy_state: bool,
}
fn decision(code: &str, message: impl Into<String>) -> Error {
    Error::decision(code, message)
}
fn changed() -> Error {
    decision(
        "MIGRATION_CHANGED",
        "Migration inputs or owned targets changed; review a new preview. All conflicting data is preserved",
    )
}
fn open_regular(path: &Path) -> Result<File> {
    index::validate_path(path)?;
    let before = fs::symlink_metadata(path)?;
    if !before.is_file() {
        return Err(decision(
            "MIGRATION_CONFLICT",
            "Expected a real file; existing node preserved",
        ));
    }
    let file = File::open(path)?;
    let opened = file.metadata()?;
    let after = fs::symlink_metadata(path)?;
    #[cfg(unix)]
    let same = {
        use std::os::unix::fs::MetadataExt;
        before.dev() == opened.dev()
            && before.ino() == opened.ino()
            && after.dev() == opened.dev()
            && after.ino() == opened.ino()
    };
    #[cfg(not(unix))]
    let same = before.created().ok() == opened.created().ok()
        && after.created().ok() == opened.created().ok();
    if !same || !after.is_file() {
        return Err(changed());
    }
    index::validate_path(path)?;
    Ok(file)
}
fn bytes(path: &Path) -> Result<Vec<u8>> {
    let mut file = open_regular(path)?;
    if file.metadata()?.len() > 8 * 1024 * 1024 {
        return Err(decision(
            "MIGRATION_RECORD_TOO_LARGE",
            "Migration control record exceeds the 8 MiB limit",
        ));
    }
    let mut result = vec![];
    file.read_to_end(&mut result)?;
    Ok(result)
}
fn copy_file(source: &Path, target: &Path) -> Result<()> {
    let mut source_file = open_regular(source)?;
    index::validate_path(target)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut target_file = options.open(target)?;
    std::io::copy(&mut source_file, &mut target_file)?;
    target_file.set_permissions(source_file.metadata()?.permissions())?;
    target_file.sync_all()?;
    Ok(())
}
pub(crate) fn node(path: &Path) -> Result<Option<Node>> {
    index::validate_path(path)?;
    let m = match fs::symlink_metadata(path) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if !m.is_file() && !m.is_dir() {
        return Err(decision(
            "MIGRATION_UNSUPPORTED_NODE",
            "Symlinks and special files in migration state or lock paths are preserved and require explicit resolution",
        ));
    }
    #[cfg(unix)]
    let (identity, mode) = {
        use std::os::unix::fs::MetadataExt;
        (format!("{}:{}", m.dev(), m.ino()), m.mode() & 0o7777)
    };
    #[cfg(not(unix))]
    let (identity, mode) = (
        format!("{:?}:{}", m.created().ok(), m.len()),
        if m.permissions().readonly() {
            0o444
        } else {
            0o644
        },
    );
    let hash = if m.is_file() {
        let mut file = open_regular(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let opened = file.metadata()?;
            if opened.ino() != m.ino() || opened.dev() != m.dev() {
                return Err(changed());
            }
        }
        let mut hasher = blake3::Hasher::new();
        hasher.update_reader(&mut file)?;
        Some(hasher.finalize().to_hex().to_string())
    } else {
        None
    };
    Ok(Some(Node {
        kind: if m.is_dir() { "directory" } else { "file" }.into(),
        identity,
        mode,
        hash,
    }))
}
fn tree(root: &Path) -> Result<Option<Tree>> {
    let Some(root_node) = node(root)? else {
        return Ok(None);
    };
    if root_node.kind != "directory" {
        return Err(decision(
            "STATE_CONFLICT",
            "State path must be a real directory",
        ));
    }
    fn visit(root: &Path, path: &Path, out: &mut Tree) -> Result<()> {
        out.insert(
            path.strip_prefix(root).map_err(|_| changed())?.into(),
            node(path)?.ok_or_else(changed)?,
        );
        for entry in fs::read_dir(path)? {
            let p = entry?.path();
            let n = node(&p)?.ok_or_else(changed)?;
            if n.kind == "directory" {
                visit(root, &p, out)?;
            } else {
                out.insert(p.strip_prefix(root).map_err(|_| changed())?.into(), n);
            }
        }
        Ok(())
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result)?;
    Ok(Some(result))
}
fn parse<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let b = bytes(path)?;
    if b.len() > 8 * 1024 * 1024 {
        return Err(decision(
            "MIGRATION_CONFLICT",
            "Recovery record exceeds its size limit",
        ));
    }
    serde_json::from_slice(&b).map_err(|e| decision("MIGRATION_CONFLICT", e.to_string()))
}
fn write_new(path: &Path, data: &[u8]) -> Result<()> {
    index::validate_path(path)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(data)?;
    file.sync_all()?;
    Ok(())
}
fn save_new(path: &Path, value: &impl Serialize) -> Result<()> {
    let encoded =
        serde_json::to_vec_pretty(value).map_err(|e| Error::new("JSON_ERROR", e.to_string()))?;
    if encoded.len() > 8 * 1024 * 1024 {
        return Err(decision(
            "MIGRATION_RECORD_TOO_LARGE",
            "Migration recovery record exceeds 8 MiB; no public paths were changed",
        ));
    }
    write_new(path, &encoded)
}
fn validate_id(id: &str) -> Result<()> {
    if !id.starts_with("migration-")
        || id.len() > 100
        || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(decision(
            "MIGRATION_CONFLICT",
            "Invalid recovery identifier",
        ));
    }
    Ok(())
}
fn journal(home: &Path, id: &str) -> Result<PathBuf> {
    validate_id(id)?;
    let root = home.join(JOURNALS);
    if bytes(&root.join("owner"))? != OWNER {
        return Err(decision(
            "MIGRATION_CONFLICT",
            "Foreign recovery directory preserved",
        ));
    }
    index::validate_path(&root.join(id))
}
fn record(home: &Path, id: &str) -> Result<Saved> {
    let saved: Saved = parse(&journal(home, id)?.join("prepared.json"))?;
    if saved.version != 1 || saved.home != home || saved.id != id {
        return Err(decision(
            "MIGRATION_CONFLICT",
            "Recovery record belongs to another home or version",
        ));
    }
    for item in &saved.locks {
        index::validate_path(&item.change.project)?;
        if item.change.from != item.change.project.join(lockfile::LEGACY_NAME)
            || item.change.to != item.change.project.join(lockfile::NAME)
        {
            return Err(decision(
                "MIGRATION_CONFLICT",
                "Recovery lock paths are not fixed project lock names",
            ));
        }
    }
    for item in &saved.routers {
        let base = match item.change.agent.as_str() {
            "claude" => ".claude",
            "codex" => ".agents",
            _ => {
                return Err(decision(
                    "MIGRATION_CONFLICT",
                    "Invalid router agent in recovery record",
                ));
            }
        };
        if item.change.from != home.join(base).join("skills/skillsbox")
            || item.change.to != home.join(base).join("skills/kinana")
        {
            return Err(decision(
                "MIGRATION_CONFLICT",
                "Recovery router paths are outside the two fixed locations",
            ));
        }
    }
    Ok(saved)
}
pub(crate) fn validate_active_state(home: &Path, has_old: bool) -> Result<()> {
    let path = state::current(home).join(RECORD);
    if !state::exists(&path)? {
        if has_old {
            return Err(decision(
                "STATE_CONFLICT",
                "Both old and new state exist without a verified migration; neither was selected or merged",
            ));
        }
        return Ok(());
    }
    let active: StateRecord = parse(&path)?;
    if active.version != 1 {
        return Err(decision(
            "STATE_CONFLICT",
            "Unsupported state migration record",
        ));
    }
    let saved = record(home, &active.id)?;
    if saved.new_state.as_ref() != node(&state::current(home))?.as_ref() {
        return Err(decision(
            "STATE_CONFLICT",
            "The active state directory was replaced or changed ownership",
        ));
    }
    if saved.source != active.source || tree(&state::legacy(home))? != active.source {
        return Err(decision(
            "MIGRATION_SOURCE_CHANGED",
            "Legacy state changed after migration; no automatic merge is allowed",
        ));
    }
    if !state::exists(&journal(home, &active.id)?.join("completed.json"))? {
        return Err(decision(
            "MIGRATION_INCOMPLETE",
            "Migration needs explicit rollback",
        ));
    }
    Ok(())
}
fn project_paths(home: &Path, explicit: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut paths = explicit
        .iter()
        .map(|p| index::validate_path(p))
        .collect::<Result<BTreeSet<_>>>()?;
    let directory = state::directory(home)?;
    let registry = directory.join("projects.toml");
    if state::exists(&registry)? {
        let document: toml::Value = toml::from_str(
            std::str::from_utf8(&bytes(&registry)?)
                .map_err(|e| decision("MIGRATION_CONFLICT", e.to_string()))?,
        )?;
        if let Some(items) = document.get("projects").and_then(toml::Value::as_array) {
            for item in items {
                let path = item
                    .get("path")
                    .and_then(toml::Value::as_str)
                    .ok_or_else(|| {
                        decision("MIGRATION_CONFLICT", "Registry project path is invalid")
                    })?;
                let path = index::validate_path(Path::new(path))?;
                if path.is_dir() {
                    paths.insert(path);
                }
            }
        }
    }
    Ok(paths.into_iter().collect())
}
fn forward(home: &Path, explicit: &[PathBuf]) -> Result<Forward> {
    let home = index::validate_path(home)?;
    state::preflight_lease(&home)?;
    let recovery = home.join(JOURNALS);
    if state::exists(&recovery)?
        && (node(&recovery)?.is_none_or(|node| node.kind != "directory")
            || !state::exists(&recovery.join("owner"))?
            || bytes(&recovery.join("owner"))? != OWNER)
    {
        return Err(decision(
            "MIGRATION_CONFLICT",
            "Foreign recovery directory preserved before any write",
        ));
    }
    let active = state::directory(&home)?;
    let old = state::legacy(&home);
    let new = state::current(&home);
    let source = tree(&old)?;
    let copy_state = active == old;
    if copy_state && state::exists(&old.join("index.db"))? {
        // Ordinary cache validation rejects WAL. Migration must retain it:
        // inspect the immutable ownership header, then validate the staged backup.
        let mut header = vec![];
        open_regular(&old.join("index.db"))?
            .take(100)
            .read_to_end(&mut header)?;
        if header.len() < 100
            || &header[..16] != b"SQLite format 3\0"
            || u32::from_be_bytes(header[68..72].try_into().map_err(|_| changed())?) != 0x5342_5831
        {
            return Err(decision(
                "MIGRATION_DATABASE_INVALID",
                "Legacy index has no valid Kinana/Skills Box ownership header",
            ));
        }
    }
    if copy_state
        && source
            .as_ref()
            .is_some_and(|t| t.contains_key(Path::new(RECORD)))
    {
        return Err(decision(
            "MIGRATION_CONFLICT",
            "Legacy state contains a reserved migration record; preserved",
        ));
    }
    let mut locks = vec![];
    let mut projects = vec![];
    for project in project_paths(&home, explicit)? {
        if project == home || project.starts_with(&old) || project.starts_with(&new) {
            return Err(decision(
                "MIGRATION_CONFLICT",
                "Choose a project outside application state and the home root",
            ));
        }
        let from = project.join(lockfile::LEGACY_NAME);
        let to = project.join(lockfile::NAME);
        let a = node(&from)?;
        let b = node(&to)?;
        if a.is_some() && b.is_some() {
            return Err(decision(
                "MIGRATION_CONFLICT",
                "Both project lock names exist; neither is overwritten",
            ));
        }
        let change = LockChange {
            project: project.clone(),
            from: from.clone(),
            to,
            state: if a.is_some() {
                "rename"
            } else if b.is_some() {
                "current"
            } else {
                "no_lock"
            }
            .into(),
        };
        if let Some(original) = a {
            let content = bytes(&from)?;
            let parsed = lockfile::parse(&content)?;
            lockfile::validate(&parsed)?;
            match_node(&from, &original)?;
            locks.push(SavedLock {
                change: change.clone(),
                original,
                parent: node(&project)?.ok_or_else(changed)?,
            });
        }
        projects.push(change);
    }
    let mut routers = vec![];
    for (agent, from, to) in crate::router::legacy_targets(&home)? {
        routers.push(SavedRouter {
            parent: node(from.parent().ok_or_else(changed)?)?.ok_or_else(changed)?,
            old: tree(&from)?.ok_or_else(changed)?,
            new_before: tree(&to)?,
            prepared: None,
            change: RouterChange {
                agent,
                from,
                to,
                state: "replace_owned".into(),
            },
        });
    }
    let mut report = Report {
        version: 1,
        action: "migrate".into(),
        source_state: old.clone(),
        target_state: new.clone(),
        journal: Some(home.join(JOURNALS)),
        preview_token: String::new(),
        changed: copy_state || !locks.is_empty() || !routers.is_empty(),
        dry_run: true,
        projects,
        routers: routers.iter().map(|item| item.change.clone()).collect(),
        state_files: if copy_state {
            source
                .as_ref()
                .into_iter()
                .flat_map(|tree| tree.iter())
                .map(|(path, node)| {
                    let aux = matches!(path.to_str(), Some("index.db-wal" | "index.db-shm"));
                    StateChange {
                        from: old.join(path),
                        to: if aux { None } else { Some(new.join(path)) },
                        kind: node.kind.clone(),
                        operation: if aux {
                            "retain_legacy_and_recovery"
                        } else if path == Path::new("index.db") {
                            "sqlite_backup"
                        } else {
                            "copy"
                        }
                        .into(),
                        mode: node.mode,
                    }
                })
                .collect()
        } else {
            vec![]
        },
        control_paths: control_paths(&home),
        warnings: vec![],
        reason_codes: vec![],
    };
    let head = owned_head(&home)?;
    let token = serde_json::to_vec(&(
        &report,
        &source,
        &locks,
        &routers,
        head,
        node(&home)?,
        node(&state::current(&home))?,
    ))
    .map_err(|e| Error::new("JSON_ERROR", e.to_string()))?;
    report.preview_token = blake3::hash(&token).to_hex().to_string();
    Ok(Forward {
        report,
        source,
        locks,
        routers,
        copy_state,
    })
}
pub fn preview(home: &Path, projects: &[PathBuf], rollback: bool) -> Result<Report> {
    if rollback {
        Ok(rollback_plan(home)?.0)
    } else {
        Ok(forward(home, projects)?.report)
    }
}
fn copy_tree(source: &Path, target: &Path, skip_db: bool) -> Result<()> {
    let expected = node(source)?.ok_or_else(changed)?;
    if expected.kind != "directory" {
        return Err(changed());
    }
    fs::create_dir(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_text = name.to_string_lossy();
        if skip_db
            && matches!(
                name_text.as_ref(),
                "index.db" | "index.db-wal" | "index.db-shm"
            )
        {
            continue;
        }
        let from = entry.path();
        let to = target.join(name);
        let n = node(&from)?.ok_or_else(changed)?;
        if n.kind == "directory" {
            copy_tree(&from, &to, false)?;
        } else {
            copy_file(&from, &to)?;
        }
    }
    fs::set_permissions(target, fs::metadata(source)?.permissions())?;
    Ok(())
}
fn prepare_state(home: &Path, id: &str, source: &Option<Tree>, work: &Path) -> Result<Node> {
    let old = state::legacy(home);
    let copy = work.join("source-state");
    let new = work.join("new-state");
    copy_tree(&old, &copy, false)?;
    copy_tree(&old, &new, true)?;
    // Stage privately; a read-only legacy root is copied with its final mode afterwards.
    let final_mode = fs::metadata(&new)?.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&new, fs::Permissions::from_mode(final_mode.mode() | 0o700))?;
    }
    if copy.join("index.db").is_file() {
        let db = rusqlite::Connection::open_with_flags(
            copy.join("index.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let application_id: i64 = db.query_row("PRAGMA application_id", [], |row| row.get(0))?;
        if application_id != 0x5342_5831 {
            return Err(decision(
                "MIGRATION_DATABASE_INVALID",
                "The legacy index is not an owned Skills Box/Kinana index; preserved",
            ));
        }
        db.backup("main", new.join("index.db"), None)?;
        let verified = rusqlite::Connection::open(new.join("index.db"))?;
        let integrity: String =
            verified.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        if integrity != "ok" {
            return Err(decision("MIGRATION_DATABASE_INVALID", integrity));
        }
        drop(verified);
        fs::set_permissions(
            new.join("index.db"),
            fs::metadata(old.join("index.db"))?.permissions(),
        )?;
    }
    save_new(
        &new.join(RECORD),
        &StateRecord {
            version: 1,
            id: id.into(),
            source: source.clone(),
        },
    )?;
    fs::set_permissions(&new, final_mode)?;
    if tree(&old)? != *source {
        return Err(changed());
    }
    node(&new)?.ok_or_else(changed)
}
fn pending_id(home: &Path) -> Result<Option<String>> {
    if state::exists(&state::pending(home))? {
        let id: String = parse(&state::pending(home))?;
        validate_id(&id)?;
        Ok(Some(id))
    } else {
        Ok(None)
    }
}
fn head_id(home: &Path) -> Result<Option<String>> {
    if state::exists(&home.join(HEAD))? {
        let id: String = serde_json::from_slice(&bytes(&home.join(HEAD))?)
            .map_err(|e| decision("MIGRATION_CONFLICT", e.to_string()))?;
        validate_id(&id)?;
        Ok(Some(id))
    } else {
        Ok(None)
    }
}
fn owned_head(home: &Path) -> Result<Option<Vec<u8>>> {
    let Some(id) = head_id(home)? else {
        return Ok(None);
    };
    let saved = record(home, &id)?;
    match_node(&home.join(HEAD), &saved.pending)?;
    if !state::exists(&journal(home, &id)?.join("completed.json"))?
        || state::exists(&journal(home, &id)?.join("rolled-back.json"))?
    {
        return Err(decision(
            "MIGRATION_CONFLICT",
            "Migration head is not a completed owned operation",
        ));
    }
    Ok(Some(bytes(&home.join(HEAD))?))
}
fn match_node(path: &Path, expected: &Node) -> Result<()> {
    if node(path)?.as_ref() != Some(expected) {
        return Err(changed());
    }
    Ok(())
}
fn remove_owned(path: &Path, expected: &Node) -> Result<()> {
    match_node(path, expected)?;
    fs::remove_file(path)?;
    Ok(())
}
fn rollback_plan(home: &Path) -> Result<(Report, Saved)> {
    let home = index::validate_path(home)?;
    let id = pending_id(&home)?
        .or(head_id(&home)?)
        .ok_or_else(|| decision("NO_MIGRATION", "No migration is available to roll back"))?;
    let saved = record(&home, &id)?;
    if state::exists(&state::pending(&home))? {
        match_node(&state::pending(&home), &saved.pending)?;
    } else {
        match_node(&home.join(HEAD), &saved.pending)?;
    }
    match_node(&home, &saved.home_identity)?;
    let work = journal(&home, &id)?;
    if tree(&state::legacy(&home))? != saved.source {
        return Err(decision(
            "MIGRATION_SOURCE_CHANGED",
            "Legacy state changed; both states are retained for manual reconciliation",
        ));
    }
    let mut observations = vec![];
    for item in &saved.locks {
        match_node(&item.change.project, &item.parent)?;
        let a = node(&item.change.from)?;
        let b = node(&item.change.to)?;
        if !((a.as_ref() == Some(&item.original) && b.is_none())
            || (a.is_none() && b.as_ref() == Some(&item.original)))
        {
            return Err(changed());
        }
        observations.push((a, b));
    }
    let mut router_observations = vec![];
    for (n, item) in saved.routers.iter().enumerate() {
        match_node(item.change.from.parent().ok_or_else(changed)?, &item.parent)?;
        let old = tree(&item.change.from)?;
        let new = tree(&item.change.to)?;
        let backup = tree(&work.join(format!("router-old-{n}")))?;
        let original_present = old.as_ref() == Some(&item.old) && backup.is_none();
        let original_backed_up = old.is_none() && backup.as_ref() == Some(&item.old);
        let new_unchanged = new == item.new_before;
        let new_published =
            item.new_before.is_none() && item.prepared.is_some() && new == item.prepared;
        if !(original_present && new_unchanged
            || original_backed_up && (new_unchanged || new_published))
        {
            return Err(changed());
        }
        if let Some(archived) = tree(&work.join(format!("router-rolled-back-{n}")))?
            && (new.is_some() || Some(archived) != item.prepared)
        {
            return Err(changed());
        }
        router_observations.push((old, new, backup));
    }
    let state_now = node(&state::current(&home))?;
    if let Some(expected) = &saved.new_state {
        if state_now.is_some() && state_now.as_ref() != Some(expected) {
            return Err(changed());
        }
        if let Some(archived) = node(&work.join("rolled-back-state"))?
            && (state_now.is_some() || &archived != expected)
        {
            return Err(decision(
                "MIGRATION_CONFLICT",
                "A retained rollback state already exists",
            ));
        }
    }
    let mut report = Report {
        version: 1,
        action: "rollback".into(),
        source_state: state::current(&home),
        target_state: state::legacy(&home),
        journal: Some(work.clone()),
        preview_token: String::new(),
        changed: true,
        dry_run: true,
        projects: saved
            .locks
            .iter()
            .map(|item| LockChange {
                project: item.change.project.clone(),
                from: item.change.to.clone(),
                to: item.change.from.clone(),
                state: "restore".into(),
            })
            .collect(),
        routers: saved
            .routers
            .iter()
            .map(|item| RouterChange {
                agent: item.change.agent.clone(),
                from: item.change.to.clone(),
                to: item.change.from.clone(),
                state: "restore".into(),
            })
            .collect(),
        state_files: if saved.new_state.is_some() {
            tree(&state::current(&home))?
                .into_iter()
                .flat_map(|tree| tree.into_iter())
                .map(|(path, node)| StateChange {
                    from: state::current(&home).join(&path),
                    to: Some(work.join("rolled-back-state").join(&path)),
                    kind: node.kind,
                    operation: "retain".into(),
                    mode: node.mode,
                })
                .collect()
        } else {
            vec![]
        },
        control_paths: control_paths(&home),
        warnings: vec![
            "New application state is retained in the recovery journal; it is never discarded"
                .into(),
        ],
        reason_codes: vec!["MIGRATION_STATE_RETAINED".into()],
    };
    report.preview_token = blake3::hash(
        &serde_json::to_vec(&(
            &report,
            observations,
            router_observations,
            state_now,
            tree(&state::current(&home))?,
            pending_id(&home)?,
            head_id(&home)?,
        ))
        .map_err(|e| Error::new("JSON_ERROR", e.to_string()))?,
    )
    .to_hex()
    .to_string();
    Ok((report, saved))
}
pub fn apply(home: &Path, projects: &[PathBuf], rollback: bool, token: &str) -> Result<Report> {
    let home = index::validate_path(home)?;
    apply_with_hook(&home, projects, rollback, token, &mut |_| Ok(()))
}
fn apply_with_hook(
    home: &Path,
    projects: &[PathBuf],
    rollback: bool,
    token: &str,
    hook: &mut impl FnMut(usize) -> Result<()>,
) -> Result<Report> {
    let pre = preview(home, projects, rollback)?;
    if pre.preview_token != token {
        return Err(decision(
            "MIGRATION_PREVIEW_CHANGED",
            "Migration state changed or confirmation is invalid; review a new preview",
        ));
    }
    if !pre.changed {
        let mut report = pre;
        report.dry_run = false;
        return Ok(report);
    }
    let _guard = state::migration_guard(home)?;
    if preview(home, projects, rollback)?.preview_token != token {
        return Err(changed());
    }
    if rollback {
        let (mut report, saved) = rollback_plan(home)?;
        restore(&saved)?;
        report.dry_run = false;
        return Ok(report);
    }
    let plan = forward(home, projects)?;
    let parent = index::validate_path(&home.join(JOURNALS))?;
    if !state::exists(&parent)? {
        fs::create_dir(&parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&parent, fs::Permissions::from_mode(0o700))?;
        }
        write_new(&parent.join("owner"), OWNER)?;
    }
    if bytes(&parent.join("owner"))? != OWNER {
        return Err(decision(
            "MIGRATION_CONFLICT",
            "Foreign recovery directory preserved",
        ));
    }
    let id = format!(
        "migration-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| Error::new("CLOCK_ERROR", e.to_string()))?
            .as_nanos(),
        NEXT_OPERATION.fetch_add(1, Ordering::Relaxed)
    );
    let work = parent.join(&id);
    fs::create_dir(&work)?;
    let mut routers = plan.routers;
    for (n, item) in routers.iter_mut().enumerate() {
        if item.new_before.is_none() {
            let stage = work.join(format!("router-new-{n}"));
            crate::router::prepare_migration_router(&stage)?;
            item.prepared = tree(&stage)?;
        }
    }
    let new_state = if plan.copy_state {
        Some(
            prepare_state(home, &id, &plan.source, &work).map_err(|mut error| {
                error.message.push_str(&format!(
                    "; private recovery data retained at {}",
                    work.display()
                ));
                error
            })?,
        )
    } else {
        None
    };
    save_new(&work.join("pending.json"), &id)?;
    let pending = node(&work.join("pending.json"))?.ok_or_else(changed)?;
    let saved = Saved {
        version: 1,
        home: home.into(),
        home_identity: node(home)?.ok_or_else(changed)?,
        id,
        source: plan.source,
        new_state,
        locks: plan.locks,
        routers,
        pending,
        previous_head: owned_head(home)?,
    };
    for (n, item) in saved.locks.iter().enumerate() {
        let backup = work.join(format!("lock-{n}.original"));
        match_node(&item.change.from, &item.original)?;
        copy_file(&item.change.from, &backup)?;
    }
    save_new(&work.join("prepared.json"), &saved)?;
    // All preparation is private and all source/target observations are checked again.
    if forward(home, projects)?.report.preview_token != token {
        return Err(changed());
    }
    fs::hard_link(work.join("pending.json"), state::pending(home))?;
    let result = (|| {
        match_node(home, &saved.home_identity)?;
        for (n, item) in saved.locks.iter().enumerate() {
            match_node(&item.change.project, &item.parent)?;
            match_node(&item.change.from, &item.original)?;
            safe_rename::rename_no_replace(&item.change.from, &item.change.to)?;
            match_node(&item.change.to, &item.original)?;
            hook(n + 1)?;
        }
        for (n, item) in saved.routers.iter().enumerate() {
            match_node(item.change.from.parent().ok_or_else(changed)?, &item.parent)?;
            if tree(&item.change.from)?.as_ref() != Some(&item.old)
                || tree(&item.change.to)? != item.new_before
            {
                return Err(changed());
            }
            safe_rename::rename_no_replace(
                &item.change.from,
                &work.join(format!("router-old-{n}")),
            )?;
            if item.new_before.is_none() {
                let stage = work.join(format!("router-new-{n}"));
                if tree(&stage)? != item.prepared {
                    return Err(changed());
                }
                safe_rename::rename_no_replace(&stage, &item.change.to)?;
                if tree(&item.change.to)? != item.prepared {
                    return Err(changed());
                }
            }
            hook(saved.locks.len() + n + 1)?;
        }
        if let Some(expected) = &saved.new_state {
            match_node(&work.join("new-state"), expected)?;
            if tree(&state::legacy(home))? != saved.source {
                return Err(changed());
            }
            safe_rename::rename_no_replace(&work.join("new-state"), &state::current(home))?;
            match_node(&state::current(home), expected)?;
            hook(saved.locks.len() + saved.routers.len() + 1)?;
        }
        let head = home.join(HEAD);
        if let Some(expected) = &saved.previous_head {
            if bytes(&head)? != *expected {
                return Err(changed());
            }
            safe_rename::rename_no_replace(&head, &work.join("previous-head"))?;
        } else if state::exists(&head)? {
            return Err(changed());
        }
        fs::hard_link(work.join("pending.json"), &head)?;
        save_new(&work.join("completed.json"), &true)?;
        remove_owned(&state::pending(home), &saved.pending)?;
        Ok(())
    })();
    if let Err(mut error) = result {
        if let Err(recovery) = restore(&saved) {
            error.message.push_str(&format!(
                "; recovery retained at {}: {}",
                work.display(),
                recovery
            ));
        }
        return Err(error);
    }
    let mut report = plan.report;
    report.dry_run = false;
    report.journal = Some(work);
    Ok(report)
}
fn restore(saved: &Saved) -> Result<()> {
    let home = &saved.home;
    let work = journal(home, &saved.id)?;
    // Whole rollback preflight precedes the first rename, including partial operations.
    rollback_plan(home)?;
    for (n, item) in saved.routers.iter().enumerate().rev() {
        if item.new_before.is_none() && tree(&item.change.to)?.is_some() {
            if tree(&item.change.to)? != item.prepared {
                return Err(changed());
            }
            safe_rename::rename_no_replace(
                &item.change.to,
                &work.join(format!("router-rolled-back-{n}")),
            )?;
        }
        let original = work.join(format!("router-old-{n}"));
        if state::exists(&original)? {
            if tree(&original)?.as_ref() != Some(&item.old) {
                return Err(changed());
            }
            safe_rename::rename_no_replace(&original, &item.change.from)?;
        }
    }
    for item in saved.locks.iter().rev() {
        if state::exists(&item.change.to)? {
            match_node(&item.change.to, &item.original)?;
            safe_rename::rename_no_replace(&item.change.to, &item.change.from)?;
            match_node(&item.change.from, &item.original)?;
        }
    }
    if let Some(expected) = &saved.new_state
        && state::exists(&state::current(home))?
    {
        match_node(&state::current(home), expected)?;
        safe_rename::rename_no_replace(&state::current(home), &work.join("rolled-back-state"))?;
    }
    let head = home.join(HEAD);
    if state::exists(&head)?
        && bytes(&head)?
            == serde_json::to_vec_pretty(&saved.id)
                .map_err(|e| Error::new("JSON_ERROR", e.to_string()))?
    {
        match_node(&head, &saved.pending)?;
        fs::remove_file(&head)?;
    }
    if let Some(previous) = &saved.previous_head {
        if !state::exists(&head)? {
            let backup = work.join("previous-head");
            if bytes(&backup)? != *previous {
                return Err(changed());
            }
            safe_rename::rename_no_replace(&backup, &head)?;
        } else if bytes(&head)? != *previous {
            return Err(changed());
        }
    }
    if state::exists(&state::pending(home))? {
        remove_owned(&state::pending(home), &saved.pending)?;
    }
    if !state::exists(&work.join("rolled-back.json"))? {
        save_new(&work.join("rolled-back.json"), &true)?;
    }
    Ok(())
}

fn control_paths(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".kinana-write.lock"),
        home.join(JOURNALS),
        state::pending(home),
        home.join(HEAD),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        root: PathBuf,
        home: PathBuf,
        project: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "kinana-migration-test-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT_OPERATION.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            let root = fs::canonicalize(root).unwrap();
            let home = root.join("home");
            fs::create_dir(&home).unwrap();
            fs::create_dir(state::legacy(&home)).unwrap();
            fs::write(state::legacy(&home).join("future"), b"unchanged state").unwrap();
            let project = root.join("مشروع");
            fs::create_dir(&project).unwrap();
            let vault = root.join("vault");
            fs::create_dir(&vault).unwrap();
            let value = format!(
                "vault = {:?}\nactive = []\nfuture = 'preserved'\n",
                vault.to_str().unwrap()
            );
            fs::write(project.join(lockfile::LEGACY_NAME), value).unwrap();
            Self {
                root,
                home,
                project,
            }
        }
        fn plan(&self) -> Report {
            preview(&self.home, std::slice::from_ref(&self.project), false).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    #[test]
    fn every_publication_failure_restores_original_lock_identity_and_keeps_recovery() {
        for step in 1..=2 {
            let f = Fixture::new();
            let before = tree(&f.project).unwrap();
            let old = tree(&state::legacy(&f.home)).unwrap();
            let plan = f.plan();
            let error = apply_with_hook(
                &f.home,
                std::slice::from_ref(&f.project),
                false,
                &plan.preview_token,
                &mut |at| {
                    if at == step {
                        Err(Error::new("INJECTED_FAILURE", "owned publication failure"))
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap_err();
            assert_eq!(error.code, "INJECTED_FAILURE");
            assert_eq!(tree(&f.project).unwrap(), before);
            assert_eq!(tree(&state::legacy(&f.home)).unwrap(), old);
            assert!(!state::current(&f.home).exists());
            assert!(!state::pending(&f.home).exists());
            assert!(f.home.join(JOURNALS).is_dir());
        }
    }
    #[test]
    fn a_foreign_identical_byte_replacement_survives_failed_migration() {
        let f = Fixture::new();
        let plan = f.plan();
        let mut foreign = None;
        let error = apply_with_hook(
            &f.home,
            std::slice::from_ref(&f.project),
            false,
            &plan.preview_token,
            &mut |at| {
                if at == 1 {
                    let path = f.project.join(lockfile::NAME);
                    let content = bytes(&path)?;
                    fs::rename(&path, f.root.join("owned-lock-retained"))?;
                    write_new(&path, &content)?;
                    foreign = node(&path)?;
                    return Err(Error::new("INJECTED_FAILURE", "replacement"));
                }
                Ok(())
            },
        )
        .unwrap_err();
        assert!(error.message.contains("recovery retained"));
        assert_eq!(node(&f.project.join(lockfile::NAME)).unwrap(), foreign);
        assert!(state::pending(&f.home).exists());
        assert!(f.root.join("owned-lock-retained").is_file());
    }
    #[test]
    fn interruption_requires_explicit_recovery_and_newer_state_is_retained_on_rollback() {
        let f = Fixture::new();
        let plan = f.plan();
        let report = apply(
            &f.home,
            std::slice::from_ref(&f.project),
            false,
            &plan.preview_token,
        )
        .unwrap();
        fs::write(
            state::current(&f.home).join("new-owner-data"),
            "must survive rollback",
        )
        .unwrap();
        let rollback = preview(&f.home, &[], true).unwrap();
        let work = report.journal.unwrap();
        apply(&f.home, &[], true, &rollback.preview_token).unwrap();
        assert_eq!(
            fs::read_to_string(work.join("rolled-back-state/new-owner-data")).unwrap(),
            "must survive rollback"
        );
        assert!(f.project.join(lockfile::LEGACY_NAME).is_file());
    }
    #[test]
    fn stale_confirmation_and_legacy_source_divergence_are_rejected_without_writes() {
        let f = Fixture::new();
        let plan = f.plan();
        fs::write(
            state::legacy(&f.home).join("future"),
            "changed before confirmation",
        )
        .unwrap();
        let before = tree(&f.root).unwrap();
        assert_eq!(
            apply(
                &f.home,
                std::slice::from_ref(&f.project),
                false,
                &plan.preview_token
            )
            .unwrap_err()
            .code,
            "MIGRATION_PREVIEW_CHANGED"
        );
        assert_eq!(tree(&f.root).unwrap(), before);
        let plan = f.plan();
        apply(
            &f.home,
            std::slice::from_ref(&f.project),
            false,
            &plan.preview_token,
        )
        .unwrap();
        fs::write(
            state::legacy(&f.home).join("future"),
            "old binary wrote after migration",
        )
        .unwrap();
        let before = tree(&f.root).unwrap();
        assert_eq!(
            state::directory(&f.home).unwrap_err().code,
            "MIGRATION_SOURCE_CHANGED"
        );
        assert_eq!(
            preview(&f.home, &[], true).unwrap_err().code,
            "MIGRATION_SOURCE_CHANGED"
        );
        assert_eq!(tree(&f.root).unwrap(), before);
    }
    #[cfg(unix)]
    #[test]
    fn symlinked_state_or_lock_destinations_are_rejected_before_any_write() {
        use std::os::unix::fs::symlink;
        let f = Fixture::new();
        let outside = f.root.join("outside");
        fs::create_dir(&outside).unwrap();
        symlink(&outside, state::current(&f.home)).unwrap();
        assert!(preview(&f.home, std::slice::from_ref(&f.project), false).is_err());
        assert!(fs::read_dir(outside).unwrap().next().is_none());
        fs::remove_file(state::current(&f.home)).unwrap();
        symlink(f.root.join("absent"), f.project.join(lockfile::NAME)).unwrap();
        assert!(preview(&f.home, std::slice::from_ref(&f.project), false).is_err());
        assert!(!f.root.join("absent").exists());
    }

    #[test]
    fn foreign_recovery_controls_and_replaced_active_state_are_never_adopted() {
        for path in [JOURNALS, HEAD, ".kinana-write.lock"] {
            let f = Fixture::new();
            fs::write(f.home.join(path), "foreign control bytes").unwrap();
            let before = tree(&f.root).unwrap();
            assert!(preview(&f.home, std::slice::from_ref(&f.project), false).is_err());
            assert_eq!(tree(&f.root).unwrap(), before);
        }
        let f = Fixture::new();
        let plan = f.plan();
        apply(
            &f.home,
            std::slice::from_ref(&f.project),
            false,
            &plan.preview_token,
        )
        .unwrap();
        fs::rename(state::current(&f.home), f.home.join("retained-owned-state")).unwrap();
        fs::create_dir(state::current(&f.home)).unwrap();
        copy_file(
            &f.home.join("retained-owned-state").join(RECORD),
            &state::current(&f.home).join(RECORD),
        )
        .unwrap();
        let before = tree(&f.root).unwrap();
        assert_eq!(
            state::directory(&f.home).unwrap_err().code,
            "STATE_CONFLICT"
        );
        assert!(preview(&f.home, &[], true).is_err());
        assert_eq!(tree(&f.root).unwrap(), before);
    }
    #[test]
    fn the_migration_lease_blocks_other_core_writers_and_never_creates_read_state() {
        let f = Fixture::new();
        assert_eq!(state::directory(&f.home).unwrap(), state::legacy(&f.home));
        assert!(!state::current(&f.home).exists());
        let _guard = state::migration_guard(&f.home).unwrap();
        assert!(state::migration_guard(&f.home).is_err());
        assert_eq!(
            state::write_guard(&state::legacy(&f.home).join("index.db"), false)
                .err()
                .unwrap()
                .code,
            "MIGRATION_REQUIRED"
        );
    }
    #[test]
    fn an_interrupted_publication_can_be_recovered_by_explicit_rollback() {
        let f = Fixture::new();
        let before = tree(&f.project).unwrap();
        let plan = f.plan();
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = apply_with_hook(
                &f.home,
                std::slice::from_ref(&f.project),
                false,
                &plan.preview_token,
                &mut |at| {
                    if at == 1 {
                        panic!("simulated interruption after a published lock")
                    };
                    Ok(())
                },
            );
        }));
        assert!(panic.is_err());
        assert_eq!(
            state::directory(&f.home).unwrap_err().code,
            "MIGRATION_INCOMPLETE"
        );
        let plan = preview(&f.home, &[], true).unwrap();
        apply(&f.home, &[], true, &plan.preview_token).unwrap();
        assert_eq!(tree(&f.project).unwrap(), before);
        assert!(!state::pending(&f.home).exists());
    }
}
