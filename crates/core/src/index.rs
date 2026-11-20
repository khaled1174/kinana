//! Rebuildable SQLite cache. Every row is scoped to its owning vault.
use crate::{Error, Result, SkillRecord};
use rusqlite::{Connection, OpenFlags, params};
use std::fs::{self, OpenOptions};
use std::path::{Component, Path, PathBuf};

const APPLICATION_ID: i64 = 0x5342_5831;
/// Schema 2 (8.3) adds `when_to_use` to `skills` and `skills_fts`.
const SCHEMA_VERSION: i64 = 2;
/// The schema of the 0.1.0 beta. That version may still be installed and
/// using its file, so a schema 1 file is read where it is found and never
/// written, upgraded or replaced.
const LEGACY_SCHEMA_VERSION: i64 = 1;
/// The schema 2 index in the application state directory. Schema 2 has its
/// own file so that the beta's `index.db` beside it stays exactly as it is.
pub const FILE_NAME: &str = "index-v2.db";
const LEGACY_FILE_NAME: &str = "index.db";

/// Refuse to write a file that belongs to the earlier version.
fn require_current(version: i64) -> Result<()> {
    if version == SCHEMA_VERSION {
        return Ok(());
    }
    Err(Error::decision(
        "INDEX_CONFLICT",
        "The index at this path belongs to an earlier Kinana version and is left unchanged; this version keeps its own index file",
    ))
}

/// Refuse existing symlinks in every component, including the final component.
/// Missing components are permitted so callers can safely preflight creation.
pub(crate) fn validate_path(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut checked = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::ParentDir => {
                return Err(Error::new(
                    "INVALID_PATH",
                    "Parent path components are not allowed",
                ));
            }
            Component::CurDir => continue,
            _ => checked.push(component.as_os_str()),
        }
        match fs::symlink_metadata(&checked) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(Error::new(
                    "SYMLINK_PATH",
                    format!("Refusing symbolic link: {}", checked.display()),
                ));
            }
            Ok(metadata) if checked != absolute && !metadata.is_dir() => {
                return Err(Error::new(
                    "INVALID_PATH",
                    format!("Not a directory: {}", checked.display()),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(checked)
}

fn reject_sidecars(path: &Path) -> Result<()> {
    // SQLite can open these during recovery or writes. Reject symlinks before
    // handing the database to SQLite as well as checking the database itself.
    for suffix in ["-journal", "-wal", "-shm"] {
        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push(suffix);
        let sidecar = validate_path(Path::new(&sidecar))?;
        match fs::symlink_metadata(&sidecar) {
            Ok(_) => {
                return Err(Error::new(
                    "INDEX_BUSY",
                    format!(
                        "SQLite sidecar requires a closed, clean index: {}",
                        sidecar.display()
                    ),
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn read_only(path: &Path) -> Result<Connection> {
    // immutable=1 prevents SQLite from creating journal/shared-memory files on
    // a read-only command (including init --dry-run). Our cache uses DELETE
    // journaling, so there are no uncheckpointed WAL records to overlook.
    let mut uri = String::from("file:");
    for byte in path.as_os_str().as_encoded_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~:".contains(byte) {
            uri.push(char::from(*byte));
        } else {
            use std::fmt::Write;
            write!(&mut uri, "%{byte:02X}")
                .map_err(|error| Error::new("INVALID_PATH", error.to_string()))?;
        }
    }
    uri.push_str("?immutable=1");
    Ok(Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?)
}

/// Complete column lists of schema 1. Schema 2 appends `when_to_use` to the
/// two skill tables and changes nothing else.
const LEGACY_TABLES: &[(&str, &[&str])] = &[
    (
        "skills",
        &[
            "vault",
            "id",
            "name",
            "description",
            "category",
            "audit_level",
            "content_hash",
            "source_url",
            "added_at",
            "role",
            "tags",
        ],
    ),
    (
        "skills_fts",
        &["vault", "id", "name", "description", "role", "tags"],
    ),
    (
        "locations",
        &["vault", "skill_id", "path", "content_hash", "kind"],
    ),
    ("variants", &["vault", "skill_id", "content_hash", "path"]),
    ("groups", &["vault", "group_name", "skill_id"]),
    ("projects", &["vault", "path", "bundle", "last_seen"]),
    (
        "activations",
        &["vault", "project_path", "skill_id", "agent", "mode"],
    ),
];
const WHEN_TO_USE_TABLES: [&str; 2] = ["skills", "skills_fts"];

/// Every table must have exactly the columns of the given schema version, in
/// order. A matching version number alone is not ownership of the layout.
fn check_layout(connection: &Connection, version: i64) -> Result<()> {
    let mut statement =
        connection.prepare("SELECT name FROM pragma_table_info(?1) ORDER BY cid")?;
    for (table, legacy) in LEGACY_TABLES {
        let mut expected = legacy.to_vec();
        if version == SCHEMA_VERSION && WHEN_TO_USE_TABLES.contains(table) {
            expected.push("when_to_use");
        }
        let actual = statement
            .query_map([table], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if actual != expected {
            return Err(Error::decision(
                "INDEX_CONFLICT",
                format!("Index table '{table}' does not have the schema version {version} layout"),
            ));
        }
    }
    Ok(())
}

/// Validate an owned database of a supported version and return that version.
fn check_schema(connection: &Connection) -> Result<i64> {
    let application_id: i64 = connection
        .pragma_query_value(None, "application_id", |row| row.get(0))
        .map_err(|error| {
            Error::decision(
                "INDEX_CONFLICT",
                format!("Index destination is not a readable Kinana database: {error}"),
            )
        })?;
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if application_id != APPLICATION_ID
        || !matches!(version, SCHEMA_VERSION | LEGACY_SCHEMA_VERSION)
    {
        return Err(Error::decision(
            "INDEX_CONFLICT",
            "The index is not a supported Kinana database",
        ));
    }
    let integrity: String =
        connection.pragma_query_value(None, "integrity_check", |row| row.get(0))?;
    if integrity != "ok" {
        return Err(Error::new("INDEX_CORRUPT", integrity));
    }
    // The complete column lists of this version, without attempting a
    // migration. A header that disagrees with the tables is not a database
    // this code wrote.
    check_layout(connection, version)?;
    Ok(version)
}

fn check_connection(connection: &Connection) -> Result<i64> {
    let version = check_schema(connection)?;
    let new_column = if version == SCHEMA_VERSION {
        " AND f.when_to_use=s.when_to_use"
    } else {
        ""
    };
    let inconsistent: bool = connection.query_row(
        &format!(
            "SELECT (SELECT count(*) FROM skills) != (SELECT count(*) FROM skills_fts)
         OR EXISTS (SELECT 1 FROM skills AS s WHERE
             (SELECT count(*) FROM skills_fts AS f
              WHERE f.vault=s.vault AND f.id=s.id AND f.name=s.name
              AND f.description=s.description AND f.role=s.role AND f.tags=s.tags{new_column}) != 1)"
        ),
        [],
        |row| row.get(0),
    )?;
    if inconsistent {
        return Err(Error::new(
            "INDEX_CORRUPT",
            "Skill rows and full-text search rows do not match",
        ));
    }
    Ok(version)
}

/// Check the destination before any vault initialization writes begin.
pub(crate) fn validate_target(path: &Path) -> Result<bool> {
    Ok(validated_version(path)?.is_some())
}

/// The schema version of a valid existing index; `None` when there is no file.
fn validated_version(path: &Path) -> Result<Option<i64>> {
    let path = validate_path(path)?;
    reject_sidecars(&path)?;
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() => Ok(Some(check_connection(&read_only(&path)?)?)),
        Ok(_) => Err(Error::decision(
            "INDEX_CONFLICT",
            format!("Index path is not a regular file: {}", path.display()),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Create an empty cache, or validate an existing cache without changing it.
pub fn initialize(path: &Path) -> Result<()> {
    let _state_guard = crate::state::write_guard(path, false)?;
    if validate_target(path)? {
        return Ok(());
    }
    let path = validate_path(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| Error::new("INVALID_PATH", "Index has no parent directory"))?;
    fs::create_dir_all(parent)?;
    validate_path(&path)?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    drop(file);
    let result = (|| -> Result<()> {
        reject_sidecars(&path)?;
        let connection = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        connection.execute_batch(
            "BEGIN IMMEDIATE;
             PRAGMA application_id = 1396856881;
             PRAGMA user_version = 2;
             CREATE TABLE skills (
                 vault TEXT NOT NULL, id TEXT NOT NULL, name TEXT NOT NULL,
                 description TEXT NOT NULL, category TEXT NOT NULL, audit_level TEXT NOT NULL,
                 content_hash TEXT NOT NULL, source_url TEXT, added_at TEXT NOT NULL,
                 role TEXT NOT NULL, tags TEXT NOT NULL,
                 when_to_use TEXT NOT NULL DEFAULT '', PRIMARY KEY(vault,id)
             );
             CREATE VIRTUAL TABLE skills_fts USING fts5(vault UNINDEXED,id UNINDEXED,name,description,role,tags,when_to_use);
             CREATE TABLE locations (vault TEXT NOT NULL,skill_id TEXT NOT NULL,path TEXT NOT NULL,
                 content_hash TEXT NOT NULL,kind TEXT NOT NULL,PRIMARY KEY(vault,skill_id,path));
             CREATE TABLE variants (vault TEXT NOT NULL,skill_id TEXT NOT NULL,content_hash TEXT NOT NULL,
                 path TEXT NOT NULL,PRIMARY KEY(vault,skill_id,path));
             CREATE TABLE groups (vault TEXT NOT NULL,group_name TEXT NOT NULL,skill_id TEXT NOT NULL,
                 PRIMARY KEY(vault,group_name,skill_id));
             CREATE TABLE projects (vault TEXT NOT NULL,path TEXT NOT NULL,bundle TEXT,last_seen TEXT,
                 PRIMARY KEY(vault,path));
             CREATE TABLE activations (vault TEXT NOT NULL,project_path TEXT NOT NULL,skill_id TEXT NOT NULL,
                 agent TEXT NOT NULL,mode TEXT NOT NULL,PRIMARY KEY(vault,project_path,skill_id,agent));
             COMMIT;",
        )?;
        check_connection(&connection).map(|_| ())
    })();
    if result.is_err() {
        // Only this call's exclusively-created file is eligible for cleanup.
        let _cleanup = fs::remove_file(&path);
    }
    result
}

/// Refresh one vault's skill cache atomically; preserve every other vault.
/// Scan results and project discoveries are deliberately outside this API.
pub fn rebuild_for_vault(path: &Path, vault_root: &Path, records: &[SkillRecord]) -> Result<()> {
    let _state_guard = crate::state::write_guard(path, false)?;
    let scope = validate_path(vault_root)?;
    // Files remain authoritative. Virtual scopes used by cache consumers may
    // have no bundles directory; an existing one must be valid and safe.
    let bundles = match fs::symlink_metadata(scope.join("bundles")) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.into()),
        Ok(_) => {
            crate::bundles::list(&crate::vault::Vault::new(scope.clone(), path.to_path_buf()))?
        }
    };
    let scope = scope.to_str().ok_or_else(|| {
        Error::new(
            "INVALID_PATH",
            "Vault path must be valid Unicode for the index",
        )
    })?;
    let path = validate_path(path)?;
    reject_sidecars(&path)?;
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() => {
            require_current(check_schema(&read_only(&path)?)?)?;
        }
        Ok(_) => {
            return Err(Error::decision(
                "INDEX_CONFLICT",
                "Index path is not a regular file",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => initialize(&path)?,
        Err(error) => return Err(error.into()),
    }
    let mut connection = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    let transaction =
        connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    write_vault_cache(&transaction, scope, records, &bundles)?;
    transaction.commit()?;
    Ok(())
}

fn write_vault_cache(
    transaction: &rusqlite::Transaction<'_>,
    scope: &str,
    records: &[SkillRecord],
    bundles: &[crate::Bundle],
) -> Result<()> {
    transaction.execute("DELETE FROM skills_fts WHERE vault=?1", [scope])?;
    transaction.execute("DELETE FROM skills WHERE vault=?1", [scope])?;
    transaction.execute("DELETE FROM locations WHERE vault=?1", [scope])?;
    transaction.execute("DELETE FROM variants WHERE vault=?1", [scope])?;
    transaction.execute("DELETE FROM groups WHERE vault=?1", [scope])?;
    for bundle in bundles {
        for member in &bundle.skills {
            if let Some(group) = &member.group {
                // Group confirmation belongs to a bundle. Safe names cannot
                // contain '/', so this namespace cannot merge two bundles.
                let name = format!("{}/{group}", bundle.name);
                transaction.execute(
                    "INSERT INTO groups (vault,group_name,skill_id) VALUES (?1,?2,?3)",
                    params![scope, name, member.id],
                )?;
            }
        }
    }
    for record in records {
        let tags = record.tags.join(" ");
        transaction.execute(
            "INSERT INTO skills (vault,id,name,description,category,audit_level,content_hash,source_url,added_at,role,tags,when_to_use)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![scope,record.id,record.name,record.description,record.category,record.audit_level,
                record.content_hash,record.source.url,record.added_at,record.role,tags,record.when_to_use],
        )?;
        // `when_not` is deliberately absent: a skill must not be found for a
        // request it says it does not fit.
        transaction.execute(
            "INSERT INTO skills_fts (vault,id,name,description,role,tags,when_to_use) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![scope,record.id,record.name,record.description,record.role,tags,record.when_to_use],
        )?;
    }
    for record in records {
        for location in &record.source.found_in {
            let observed_hash = match std::fs::symlink_metadata(location) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                    crate::hash::hash_payload_directory(location)
                        .unwrap_or_else(|_| record.content_hash.clone())
                }
                _ => record.content_hash.clone(),
            };
            let path = location
                .to_str()
                .ok_or_else(|| Error::new("INVALID_PATH", "Location must be valid Unicode"))?;
            transaction.execute("INSERT OR REPLACE INTO locations (vault,skill_id,path,content_hash,kind) VALUES (?1,?2,?3,?4,?5)",
                params![scope,record.id,path,observed_hash,record.source.kind])?;
            if observed_hash != record.content_hash {
                transaction.execute("INSERT OR REPLACE INTO variants (vault,skill_id,content_hash,path) VALUES (?1,?2,?3,?4)",
                    params![scope,record.id,observed_hash,path])?;
            }
        }
    }
    Ok(())
}

/// Whether `path` is a schema 1 index, judged by its header alone: SQLite is
/// not asked to open the file, so nothing can be created beside it.
fn is_legacy_index(path: &Path) -> Result<bool> {
    use std::io::Read;
    let path = validate_path(path)?;
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    }
    let mut header = Vec::with_capacity(100);
    fs::File::open(&path)?.take(100).read_to_end(&mut header)?;
    Ok(header.len() == 100
        && &header[..16] == b"SQLite format 3\0"
        && header[68..72] == (APPLICATION_ID as u32).to_be_bytes()
        && header[60..64] == (LEGACY_SCHEMA_VERSION as u32).to_be_bytes())
}

fn has_skill_rows(path: &Path, scope: &str) -> Result<bool> {
    Ok(read_only(path)?.query_row(
        "SELECT EXISTS (SELECT 1 FROM skills WHERE vault=?1)",
        [scope],
        |row| row.get(0),
    )?)
}

/// After an upgrade from the 0.1.0 beta the state directory holds only that
/// version's `index.db`. It is left exactly as it is, so the beta keeps
/// working, and this version's own file is built here from the vault's files
/// and its registered project locks, the way `doctor --rebuild-index` builds
/// it. A second vault that shared the old file gets its rows the same way the
/// first time it is used. Callers run this before a command that is not a
/// preview; it never writes inside a vault. Returns whether an index was built.
pub fn ensure_built(vault: &crate::vault::Vault) -> Result<bool> {
    let index = validate_path(&vault.index_path)?;
    if index.file_name().and_then(|name| name.to_str()) != Some(FILE_NAME) {
        return Ok(false);
    }
    let legacy = index.with_file_name(LEGACY_FILE_NAME);
    if !is_legacy_index(&legacy)? {
        return Ok(false);
    }
    let exists = match fs::symlink_metadata(&index) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    // Nothing to build from: `init` creates the vault and an empty index.
    if vault.config().is_err() {
        return Ok(false);
    }
    if exists {
        let scope = validate_path(&vault.root)?;
        let scope = scope
            .to_str()
            .ok_or_else(|| Error::new("INVALID_PATH", "Vault path must be Unicode"))?;
        if has_skill_rows(&index, scope)?
            || !has_skill_rows(&legacy, scope)?
            || vault.records()?.is_empty()
        {
            return Ok(false);
        }
    }
    Ok(recovery::rebuild(vault, false)?.rebuilt)
}

/// Verify the index without creating files or repairing data.
pub fn check(path: &Path) -> Result<()> {
    if validate_target(path)? {
        Ok(())
    } else {
        Err(Error::new(
            "INDEX_MISSING",
            format!("Index does not exist: {}", path.display()),
        ))
    }
}

/// One vault scope of a validated index, opened read-only for a search.
struct SearchCache {
    connection: Connection,
    scope: String,
    version: i64,
}

impl SearchCache {
    fn open(vault: &crate::vault::Vault) -> Result<Self> {
        let scope = validate_path(&vault.root)?;
        let scope = scope
            .to_str()
            .ok_or_else(|| Error::new("INVALID_PATH", "Vault path must be Unicode"))?
            .to_owned();
        let version = validated_version(&vault.index_path)
            .map_err(|mut error| {
                error
                    .message
                    .push_str("; run doctor --rebuild-index for this vault");
                error
            })?
            .ok_or_else(|| {
                Error::new(
                    "INDEX_MISSING",
                    "Search index is missing; run doctor --rebuild-index for this vault",
                )
            })?;
        Ok(Self {
            connection: read_only(&validate_path(&vault.index_path)?)?,
            scope,
            version,
        })
    }

    /// Compare the cache with file metadata. A difference is drift: reported,
    /// never repaired by a search. A schema 1 file has no `when_to_use`.
    fn require_current(&self, records: &[SkillRecord]) -> Result<()> {
        let when_to_use = if self.version == SCHEMA_VERSION {
            "when_to_use"
        } else {
            "''"
        };
        let mut statement = self.connection.prepare(&format!(
            "SELECT id,name,description,role,tags,{when_to_use} FROM skills WHERE vault=?1 ORDER BY id"
        ))?;
        let actual = statement
            .query_map([&self.scope], |row| {
                Ok((
                    [
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ],
                    row.get::<_, String>(5)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut expected = records
            .iter()
            .map(|record| {
                (
                    [
                        record.id.clone(),
                        record.name.clone(),
                        record.description.clone(),
                        record.role.clone(),
                        record.tags.join(" "),
                    ],
                    if self.version == SCHEMA_VERSION {
                        record.when_to_use.clone()
                    } else {
                        String::new()
                    },
                )
            })
            .collect::<Vec<_>>();
        expected.sort_by(|a, b| a.0[0].cmp(&b.0[0]));
        if actual == expected {
            Ok(())
        } else {
            Err(Error::new(
                "INDEX_STALE",
                "Search cache differs from file metadata; run doctor --rebuild-index for this vault",
            ))
        }
    }

    /// `terms` are quoted literals joined by AND; the column filter and the
    /// ranking weights follow the layout this file has.
    fn matching(&self, terms: &str, records: &[SkillRecord]) -> Result<Vec<SkillRecord>> {
        // `when_not` is not indexed: a skill must not surface for a request
        // it says it does not fit.
        let (columns, weights) = if self.version == SCHEMA_VERSION {
            ("name description role tags when_to_use", "0,0,8,4,2,2,1")
        } else {
            ("name description role tags", "0,0,8,4,2,2")
        };
        let mut statement = self.connection.prepare(&format!(
            "SELECT id FROM skills_fts WHERE skills_fts MATCH ?1 AND vault=?2
             ORDER BY bm25(skills_fts,{weights}),id LIMIT 101"
        ))?;
        let ids = statement
            .query_map(
                params![format!("{{{columns}}} : ({terms})"), self.scope],
                |row| row.get::<_, String>(0),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let by_id = records
            .iter()
            .map(|r| (r.id.as_str(), r))
            .collect::<std::collections::BTreeMap<_, _>>();
        ids.into_iter()
            .map(|id| {
                by_id
                    .get(id.as_str())
                    .map(|record| (*record).clone())
                    .ok_or_else(|| {
                        Error::new(
                            "INDEX_STALE",
                            "Search result is absent from file metadata; run doctor --rebuild-index",
                        )
                    })
            })
            .collect()
    }
}

/// Search one vault's scope. Reads only: neither the cache nor the vault is
/// written, and Git is not involved.
pub(crate) fn search(vault: &crate::vault::Vault, terms: &str) -> Result<Vec<SkillRecord>> {
    let records = vault.records()?;
    let cache = SearchCache::open(vault)?;
    cache.require_current(&records)?;
    cache.matching(terms, &records)
}

/// Activation never creates or repairs an index implicitly.
pub(crate) fn activation_connection(path: &Path, writable: bool) -> Result<Connection> {
    let Some(version) = validated_version(path)? else {
        return Err(Error::new(
            "INDEX_MISSING",
            "Initialize or rebuild the local index before activation",
        ));
    };
    let path = validate_path(path)?;
    if writable {
        require_current(version)?;
        Ok(Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE,
        )?)
    } else {
        read_only(&path)
    }
}

pub(crate) fn project_cache_matches(
    connection: &Connection,
    project: &Path,
    lock: &crate::ProjectLock,
) -> Result<bool> {
    use rusqlite::OptionalExtension;
    let vault = lock
        .vault
        .to_str()
        .ok_or_else(|| Error::new("INVALID_PATH", "Vault must be Unicode"))?;
    let project = project
        .to_str()
        .ok_or_else(|| Error::new("INVALID_PATH", "Project must be Unicode"))?;
    let bundle: Option<Option<String>> = connection
        .query_row(
            "SELECT bundle FROM projects WHERE vault=?1 AND path=?2",
            params![vault, project],
            |row| row.get(0),
        )
        .optional()?;
    if bundle != Some(lock.bundle.clone()) {
        return Ok(false);
    }
    let mut statement = connection.prepare("SELECT skill_id,agent,mode FROM activations WHERE vault=?1 AND project_path=?2 ORDER BY skill_id,agent,mode")?;
    let actual = statement
        .query_map(params![vault, project], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut expected = lock
        .active
        .iter()
        .map(|skill| (skill.id.clone(), skill.agent.clone(), skill.mode.clone()))
        .collect::<Vec<_>>();
    expected.sort();
    Ok(actual == expected)
}

pub(crate) fn write_project_cache(
    transaction: &rusqlite::Transaction<'_>,
    project: &Path,
    lock: &crate::ProjectLock,
) -> Result<()> {
    let vault = lock
        .vault
        .to_str()
        .ok_or_else(|| Error::new("INVALID_PATH", "Vault must be Unicode"))?;
    let project = project
        .to_str()
        .ok_or_else(|| Error::new("INVALID_PATH", "Project must be Unicode"))?;
    transaction.execute("INSERT INTO projects (vault,path,bundle) VALUES (?1,?2,?3) ON CONFLICT(vault,path) DO UPDATE SET bundle=excluded.bundle", params![vault, project, lock.bundle])?;
    transaction.execute(
        "DELETE FROM activations WHERE vault=?1 AND project_path=?2",
        params![vault, project],
    )?;
    for skill in &lock.active {
        transaction.execute("INSERT INTO activations (vault,project_path,skill_id,agent,mode) VALUES (?1,?2,?3,?4,?5)", params![vault, project, skill.id, skill.agent, skill.mode])?;
    }
    Ok(())
}

pub(crate) fn remove_project_cache(
    transaction: &rusqlite::Transaction<'_>,
    project: &Path,
    vault: &Path,
) -> Result<()> {
    let project = project
        .to_str()
        .ok_or_else(|| Error::new("INVALID_PATH", "Project must be Unicode"))?;
    let vault = vault
        .to_str()
        .ok_or_else(|| Error::new("INVALID_PATH", "Vault must be Unicode"))?;
    transaction.execute(
        "DELETE FROM activations WHERE vault=?1 AND project_path=?2",
        params![vault, project],
    )?;
    transaction.execute(
        "DELETE FROM projects WHERE vault=?1 AND path=?2",
        params![vault, project],
    )?;
    Ok(())
}

/// Optional location details from the disposable cache; source metadata remains authoritative.
pub fn detail_locations(
    path: &Path,
    vault_root: &Path,
    id: &str,
) -> Result<(Vec<PathBuf>, Vec<crate::ScanEntry>)> {
    check(path)?;
    let path = validate_path(path)?;
    let scope = validate_path(vault_root)?;
    let scope = scope
        .to_str()
        .ok_or_else(|| Error::new("INVALID_PATH", "Vault path must be valid Unicode"))?;
    let connection = read_only(&path)?;
    let mut statement = connection
        .prepare("SELECT path FROM locations WHERE vault=?1 AND skill_id=?2 ORDER BY path")?;
    let locations = statement
        .query_map(params![scope, id], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .map(PathBuf::from)
        .collect();
    let mut statement = connection.prepare(
        "SELECT path,content_hash FROM variants WHERE vault=?1 AND skill_id=?2 ORDER BY path",
    )?;
    let variants = statement
        .query_map(params![scope, id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .map(|(path, hash)| crate::ScanEntry {
            path: PathBuf::from(path),
            name: None,
            description: None,
            content_hash: Some(hash),
            status: crate::ScanStatus::Variant,
            skill_id: Some(id.into()),
            source: crate::SourceInfo::default(),
            error: None,
        })
        .collect();
    Ok((locations, variants))
}

mod recovery;
pub use recovery::rebuild;

pub(crate) fn registered_projects_match(
    vault: &crate::vault::Vault,
    observations: &[crate::RegistryObservation],
) -> Result<bool> {
    let connection = activation_connection(&vault.index_path, false)?;
    let scope = validate_path(&vault.root)?;
    let scope = scope
        .to_str()
        .ok_or_else(|| Error::new("INVALID_PATH", "Vault must be Unicode"))?;
    let expected = observations
        .iter()
        .filter(|observation| observation.status == "ok")
        .collect::<Vec<_>>();
    let count: i64 = connection.query_row(
        "SELECT count(*) FROM projects WHERE vault=?1",
        [scope],
        |row| row.get(0),
    )?;
    if count != expected.len() as i64 {
        return Ok(false);
    }
    let mut expected_activations = 0;
    for observation in expected {
        if let Some(lock) = &observation.lock {
            expected_activations += lock.active.len();
            if !project_cache_matches(&connection, &observation.project, lock)? {
                return Ok(false);
            }
        }
    }
    let count: i64 = connection.query_row(
        "SELECT count(*) FROM activations WHERE vault=?1",
        [scope],
        |row| row.get(0),
    )?;
    Ok(count == expected_activations as i64)
}
