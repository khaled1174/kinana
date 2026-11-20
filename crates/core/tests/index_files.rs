//! Schema 2 of the index (8.3) lives in its own file, `index-v2.db`, built
//! from the vault's files. The `index.db` of the 0.1.0 beta beside it is
//! never written, upgraded or replaced, so that version keeps working.
mod support;

use kinana_core::{
    Audit, Classification, Description, Error, SkillMetadata, SourceInfo, hash, index, search,
    vault::Vault,
};
use rusqlite::{Connection, params};
use std::{
    fs,
    path::{Path, PathBuf},
};

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        Self {
            root: support::temp_dir().unwrap(),
        }
    }
    fn state(&self) -> PathBuf {
        self.root.join("state")
    }
    /// Where the 0.1.0 beta keeps its index.
    fn beta_index(&self) -> PathBuf {
        self.state().join("index.db")
    }
    /// A real vault as this version addresses it after an upgrade: its files
    /// are there and this version's own index file is not.
    fn vault(&self, name: &str) -> Vault {
        let vault = Vault::new(self.root.join(name), self.state().join(index::FILE_NAME));
        let had_index = vault.index_path.exists();
        vault.init(false).unwrap();
        if !had_index {
            fs::remove_file(&vault.index_path).unwrap();
        }
        vault
    }
    /// Write `path` as the beta would have: schema 1, with the rows of these vaults.
    fn beta_wrote(&self, path: &Path, vaults: &[&Vault]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let _ = fs::remove_file(path);
        let connection = Connection::open(path).unwrap();
        connection
            .execute_batch(support::BETA_INDEX_SCHEMA)
            .unwrap();
        for vault in vaults {
            let scope = vault.root.to_str().unwrap();
            for record in vault.records().unwrap() {
                let tags = record.tags.join(" ");
                connection.execute(
                    "INSERT INTO skills (vault,id,name,description,category,audit_level,content_hash,source_url,added_at,role,tags)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                    params![scope, record.id, record.name, record.description, record.category,
                        record.audit_level, record.content_hash, record.source.url, record.added_at,
                        record.role, tags],
                ).unwrap();
                connection.execute(
                    "INSERT INTO skills_fts (vault,id,name,description,role,tags) VALUES (?1,?2,?3,?4,?5,?6)",
                    params![scope, record.id, record.name, record.description, record.role, tags],
                ).unwrap();
            }
        }
        drop(connection);
        assert_eq!(version(path), 1);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn seed(vault: &Vault, id: &str, tags: &[&str], when_to_use: &str) {
    let path = vault.root.join("skills").join(id);
    fs::create_dir_all(&path).unwrap();
    fs::write(
        path.join("SKILL.md"),
        format!("---\nname: {id}\ndescription: Describes {id}\n---\nBody\n"),
    )
    .unwrap();
    let metadata = SkillMetadata {
        proposal_source_changed: None,
        pinned: false,
        id: id.into(),
        content_hash: hash::hash_directory(&path).unwrap(),
        added_at: "2026-11-14T00:00:00Z".into(),
        source: SourceInfo::default(),
        classification: Classification {
            role: format!("Role of {id}"),
            tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
            when_to_use: when_to_use.into(),
            when_not: "Never for forbidden topics".into(),
            ..Default::default()
        },
        description: Description {
            original: format!("Describes {id}"),
            ..Default::default()
        },
        audit: Audit::default(),
        empty_directories: vec![],
        proposal: None,
    };
    fs::write(
        vault.root.join("meta").join(format!("{id}.toml")),
        toml::to_string(&metadata).unwrap(),
    )
    .unwrap();
}

fn version(index: &Path) -> i64 {
    Connection::open(index)
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap()
}

fn ids(vault: &Vault, query: &str) -> Result<Vec<String>, Error> {
    Ok(search::search(vault, query)?
        .items
        .into_iter()
        .map(|skill| skill.id)
        .collect())
}

fn names(directory: &Path) -> Vec<String> {
    let mut names = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    names
}

/// Every file under a directory (Git internals included) with its bytes.
fn tree(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(path: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if fs::symlink_metadata(&path).unwrap().is_dir() {
                out.push((path.clone(), vec![]));
                walk(&path, out);
            } else {
                out.push((path.clone(), fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = vec![];
    walk(root, &mut out);
    out.sort();
    out
}

#[test]
fn this_versions_index_is_built_from_the_vault_beside_an_untouched_beta_index() {
    let f = Fixture::new();
    let (first, second) = (f.vault("first"), f.vault("second"));
    seed(&first, "alpha", &["ledger"], "Before closing the month");
    seed(&second, "beta", &["rocket"], "When planning a launch");
    f.beta_wrote(&f.beta_index(), &[&first, &second]);
    // The beta's cache then drifted from the files in both directions. Built
    // from the vault means: no row the files lack, no miss of one they have.
    let scope = first.root.to_str().unwrap();
    let connection = Connection::open(f.beta_index()).unwrap();
    connection.execute("INSERT INTO skills VALUES (?1,'ghost','Ghost','Haunts','other','clean','blake3:x',NULL,'2026-01-01','Ghost role','phantom')", [scope]).unwrap();
    connection
        .execute(
            "INSERT INTO skills_fts VALUES (?1,'ghost','Ghost','Haunts','Ghost role','phantom')",
            [scope],
        )
        .unwrap();
    drop(connection);
    seed(&first, "gamma", &["harbor"], "While docking");
    let beta = fs::read(f.beta_index()).unwrap();
    let vaults = (tree(&first.root), tree(&second.root));
    assert!(!first.index_path.exists());

    assert!(index::ensure_built(&first).unwrap());
    assert_eq!(version(&first.index_path), 2);
    // "When to use" text, which schema 1 never held, and the newer skill.
    assert_eq!(ids(&first, "closing").unwrap(), ["alpha"]);
    assert_eq!(ids(&first, "harbor").unwrap(), ["gamma"]);
    assert!(ids(&first, "phantom").unwrap().is_empty());
    assert_eq!(fs::read(f.beta_index()).unwrap(), beta);

    // Another vault that shared the beta's file gets its rows on first use.
    assert_eq!(ids(&second, "launch").unwrap_err().code, "INDEX_STALE");
    assert!(index::ensure_built(&second).unwrap());
    assert_eq!(ids(&second, "launch").unwrap(), ["beta"]);
    assert_eq!(ids(&first, "closing").unwrap(), ["alpha"]);
    assert_eq!(fs::read(f.beta_index()).unwrap(), beta);

    // Settled: nothing more is built, and only the two index files exist.
    let built = fs::read(&first.index_path).unwrap();
    assert!(!index::ensure_built(&first).unwrap());
    assert!(!index::ensure_built(&second).unwrap());
    assert_eq!(fs::read(&first.index_path).unwrap(), built);
    assert_eq!(names(&f.state()), ["index-v2.db", "index.db"]);
    assert_eq!((tree(&first.root), tree(&second.root)), vaults);
}

#[test]
fn nothing_is_built_where_there_is_no_beta_index_to_upgrade_from() {
    // No `index.db` beside the missing index: an ordinary missing index.
    let f = Fixture::new();
    let vault = f.vault("vault");
    seed(&vault, "alpha", &["ledger"], "Before closing the month");
    assert!(!index::ensure_built(&vault).unwrap());
    assert!(!vault.index_path.exists());

    // Files named `index.db` that are not the beta's index.
    let foreign: &[(&str, &[&str])] = &[
        (
            "another application's database",
            &["PRAGMA application_id = 42"],
        ),
        ("a later schema version", &["PRAGMA user_version = 3"]),
        (
            "this version's schema under the old name",
            &["PRAGMA user_version = 2"],
        ),
    ];
    for (name, statements) in foreign {
        f.beta_wrote(&f.beta_index(), &[&vault]);
        let connection = Connection::open(f.beta_index()).unwrap();
        for statement in *statements {
            connection.execute_batch(statement).unwrap();
        }
        drop(connection);
        let bytes = fs::read(f.beta_index()).unwrap();
        assert!(!index::ensure_built(&vault).unwrap(), "{name}");
        assert!(!vault.index_path.exists(), "{name}");
        assert_eq!(fs::read(f.beta_index()).unwrap(), bytes, "{name}");
    }
    fs::write(f.beta_index(), b"not a database").unwrap();
    assert!(!index::ensure_built(&vault).unwrap());
    assert!(!vault.index_path.exists());

    // A real beta index, but no vault to build from; `init` handles that.
    f.beta_wrote(&f.beta_index(), &[&vault]);
    let absent = Vault::new(f.root.join("absent"), vault.index_path.clone());
    assert!(!index::ensure_built(&absent).unwrap());
    assert!(!absent.root.exists() && !vault.index_path.exists());

    // An index path that is not the state directory's own file name.
    let custom = Vault::new(vault.root.clone(), f.state().join("custom.db"));
    assert!(!index::ensure_built(&custom).unwrap());
    assert_eq!(names(&f.state()), ["index.db"]);

    // A vault the beta never indexed, once this version's file exists.
    assert!(index::ensure_built(&vault).unwrap());
    let other = f.vault("other");
    seed(&other, "delta", &["river"], "At the crossing");
    let built = fs::read(&vault.index_path).unwrap();
    assert!(!index::ensure_built(&other).unwrap());
    assert_eq!(fs::read(&vault.index_path).unwrap(), built);
}

#[test]
fn a_beta_index_found_at_the_index_path_is_read_and_never_written() {
    // `~/.skillsbox` stays read-only until `kinana migrate` and holds only a
    // schema 1 index. It is searched where it is, with the columns it has.
    let f = Fixture::new();
    let home = f.root.join("home");
    let legacy_index = home.join(".skillsbox/index.db");
    let vault = Vault::new(f.root.join("vault"), f.state().join("setup.db"));
    vault.init(false).unwrap();
    seed(&vault, "alpha", &["ledger"], "Before closing the month");
    let legacy = Vault::new(vault.root.clone(), legacy_index.clone());
    f.beta_wrote(&legacy_index, &[&vault]);
    let bytes = fs::read(&legacy_index).unwrap();
    let before = tree(&home);

    assert_eq!(ids(&legacy, "ledger").unwrap(), ["alpha"]);
    // The old layout has no "when to use" column to search.
    assert!(ids(&legacy, "month").unwrap().is_empty());
    index::check(&legacy_index).unwrap();
    assert!(!index::ensure_built(&legacy).unwrap());
    assert_eq!(
        tree(&home),
        before,
        "no lease, sidecar or state file appeared"
    );

    // The same file outside read-only state: every write path refuses it.
    let elsewhere = Vault::new(vault.root.clone(), f.state().join("index.db"));
    fs::copy(&legacy_index, &elsewhere.index_path).unwrap();
    let records = vault.records().unwrap();
    for error in [
        index::rebuild_for_vault(&elsewhere.index_path, &elsewhere.root, &records).unwrap_err(),
        index::rebuild(&elsewhere, false).unwrap_err(),
        index::rebuild(&elsewhere, true).unwrap_err(),
    ] {
        assert_eq!(error.code, "INDEX_CONFLICT", "{error}");
        assert!(error.needs_decision);
    }
    // Damage does not make it this version's file to replace either.
    let connection = Connection::open(&elsewhere.index_path).unwrap();
    connection.execute_batch("DROP TABLE variants").unwrap();
    drop(connection);
    let damaged = fs::read(&elsewhere.index_path).unwrap();
    assert_eq!(
        index::rebuild(&elsewhere, false).unwrap_err().code,
        "INDEX_CONFLICT"
    );
    assert_eq!(fs::read(&elsewhere.index_path).unwrap(), damaged);
    assert_eq!(fs::read(&legacy_index).unwrap(), bytes);
    assert_eq!(names(&f.state()), ["index.db", "setup.db"]);
}

#[test]
fn foreign_and_inexact_databases_at_the_index_path_are_never_adopted() {
    // (what, statements applied to a valid index of this version, whether
    // the header still says it is this version's file)
    let cases: &[(&str, &[&str], bool)] = &[
        (
            "another application's database",
            &["PRAGMA application_id = 42"],
            false,
        ),
        (
            "an unset application id",
            &["PRAGMA application_id = 0"],
            false,
        ),
        (
            "a future schema version",
            &["PRAGMA user_version = 3"],
            false,
        ),
        ("version zero", &["PRAGMA user_version = 0"], false),
        (
            "schema 1 claimed over this version's columns",
            &["PRAGMA user_version = 1"],
            false,
        ),
        (
            "a foreign extra column",
            &["ALTER TABLE projects ADD COLUMN owner TEXT"],
            true,
        ),
        ("a missing table", &["DROP TABLE variants"], true),
    ];
    for (name, statements, owned_header) in cases {
        let f = Fixture::new();
        let vault = Vault::new(f.root.join("vault"), f.state().join(index::FILE_NAME));
        vault.init(false).unwrap();
        seed(&vault, "alpha", &["ledger"], "Before closing the month");
        let records = vault.records().unwrap();
        index::rebuild_for_vault(&vault.index_path, &vault.root, &records).unwrap();
        let connection = Connection::open(&vault.index_path).unwrap();
        for statement in *statements {
            connection.execute_batch(statement).unwrap();
        }
        drop(connection);
        let bytes = fs::read(&vault.index_path).unwrap();
        for error in [
            index::check(&vault.index_path).unwrap_err(),
            index::rebuild_for_vault(&vault.index_path, &vault.root, &records).unwrap_err(),
            ids(&vault, "ledger").unwrap_err(),
        ] {
            assert_eq!(error.code, "INDEX_CONFLICT", "{name}: {error}");
            assert!(error.needs_decision, "{name}");
        }
        assert!(!index::ensure_built(&vault).unwrap(), "{name}");
        assert_eq!(fs::read(&vault.index_path).unwrap(), bytes, "{name}");
        assert_eq!(names(&f.state()), ["index-v2.db"], "{name}");
        // `doctor --rebuild-index` refuses a file without this version's
        // header. With the header but not the layout it is damage: the cache
        // is rebuilt from files and the original is kept byte for byte.
        match index::rebuild(&vault, false) {
            Err(error) => {
                assert!(!owned_header, "{name}: {error}");
                assert_eq!(error.code, "INDEX_CONFLICT", "{name}: {error}");
                assert_eq!(fs::read(&vault.index_path).unwrap(), bytes, "{name}");
                assert_eq!(names(&f.state()), ["index-v2.db"], "{name}");
            }
            Ok(report) => {
                assert!(
                    owned_header,
                    "{name}: a file without an owned header was adopted"
                );
                assert_eq!(report.mode, "owned_corrupt", "{name}");
                let backup = report.backup.expect("the original is preserved");
                assert_eq!(fs::read(backup).unwrap(), bytes, "{name}");
                assert_eq!(version(&vault.index_path), 2);
                assert_eq!(ids(&vault, "month").unwrap(), ["alpha"]);
            }
        }
    }
}
