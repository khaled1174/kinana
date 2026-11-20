mod support;
use kinana_core::{
    ActivateOptions, Bundle, BundleSkill, ImportOptions, ProjectRegistration, ProjectRegistry,
    activate, bundles, doctor, import, index, project_registry, vault::Vault,
};
use rusqlite::{Connection, types::Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        Self(support::temp_dir().unwrap())
    }
    fn vault(&self, name: &str) -> Vault {
        let v = Vault::new(self.0.join(name), self.0.join("state/index.db"));
        v.init(false).unwrap();
        v
    }
    fn seed(&self, v: &Vault, name: &str) {
        let source = self.0.join(format!(
            "source-{}-{name}",
            v.root.file_name().unwrap().to_string_lossy()
        ));
        fs::create_dir(&source).unwrap();
        fs::write(source.join("SKILL.md"),format!("---\nname: {name}\ndescription: Review React accessibility\n---\nReview components before UI changes.\n")).unwrap();
        import::add(
            v,
            source.to_str().unwrap(),
            &self.0.join("import-tmp"),
            &ImportOptions::default(),
        )
        .unwrap();
        let input = self.0.join(format!(
            "bundle-{}.toml",
            v.root.file_name().unwrap().to_string_lossy()
        ));
        let bundle = Bundle {
            name: "suite".into(),
            title: "Suite".into(),
            agents: vec!["claude".into()],
            skills: vec![BundleSkill {
                id: name.into(),
                role: "primary".into(),
                group: Some("review".into()),
            }],
        };
        fs::write(&input, toml::to_string(&bundle).unwrap()).unwrap();
        bundles::create(v, &input, false).unwrap();
    }
    fn project(&self, v: &Vault, name: &str, id: &str) -> PathBuf {
        let p = self.0.join(name);
        fs::create_dir(&p).unwrap();
        fs::write(p.join("README.md"), "keep\n").unwrap();
        activate::activate(
            v,
            &ActivateOptions {
                project: p.clone(),
                home: self.0.clone(),
                selections: vec![id.into()],
                agent: "claude".into(),
                mode: "link".into(),
                include_secondary: vec![],
                dry_run: false,
            },
        )
        .unwrap();
        p
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, path: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            let m = fs::symlink_metadata(&path).unwrap();
            let rel = path.strip_prefix(root).unwrap().into();
            if m.file_type().is_symlink() {
                out.insert(
                    rel,
                    fs::read_link(path)
                        .unwrap()
                        .as_os_str()
                        .as_encoded_bytes()
                        .to_vec(),
                );
            } else if m.is_dir() {
                out.insert(rel, b"dir".to_vec());
                walk(root, &path, out);
            } else {
                out.insert(rel, fs::read(path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}
fn rows(database: &Path, scope: &Path) -> BTreeMap<String, Vec<Vec<Value>>> {
    let c = Connection::open(database).unwrap();
    let mut tables = BTreeMap::new();
    for table in [
        "skills",
        "skills_fts",
        "locations",
        "variants",
        "groups",
        "projects",
        "activations",
    ] {
        let count = c
            .prepare(&format!("SELECT * FROM {table} LIMIT 0"))
            .unwrap()
            .column_count();
        let order = (1..=count)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let mut statement = c
            .prepare(&format!(
                "SELECT * FROM {table} WHERE vault=?1 ORDER BY {order}"
            ))
            .unwrap();
        let values = statement
            .query_map([scope.to_str().unwrap()], |row| {
                (0..count)
                    .map(|i| row.get::<_, Value>(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        tables.insert(table.into(), values);
    }
    tables
}

#[test]
fn healthy_rebuild_restores_selected_rows_and_preserves_every_other_vault_column() {
    let t = Temp::new();
    let a = t.vault("a");
    let b = t.vault("b");
    t.seed(&a, "item");
    t.seed(&b, "item");
    let p = t.project(&a, "project-a", "item");
    t.project(&b, "project-b", "item");
    let other = rows(&a.index_path, &b.root);
    let c = Connection::open(&a.index_path).unwrap();
    for table in ["skills", "skills_fts"] {
        c.execute(
            &format!("UPDATE {table} SET name='stale' WHERE vault=?1"),
            [a.root.to_str().unwrap()],
        )
        .unwrap();
    }
    c.execute(
        "INSERT INTO projects(vault,path,bundle) VALUES (?1,'/do-not-discover-from-cache','ghost')",
        [a.root.to_str().unwrap()],
    )
    .unwrap();
    drop(c);
    let before = tree(&t.0);
    let preview = index::rebuild(&a, true).unwrap();
    assert_eq!(preview.projects, 1);
    assert!(preview.other_vaults_preserved);
    assert_eq!(tree(&t.0), before);
    let vault_before = tree(&a.root);
    let project_before = tree(&p);
    let registry = fs::read(project_registry::path(&a).unwrap()).unwrap();
    let report = index::rebuild(&a, false).unwrap();
    assert_eq!(report.mode, "healthy");
    assert!(report.rebuilt);
    assert_eq!(
        (
            report.skills,
            report.groups,
            report.projects,
            report.activations
        ),
        (1, 1, 1, 1)
    );
    assert_eq!(rows(&a.index_path, &b.root), other);
    assert_eq!(
        rows(&a.index_path, &a.root)["skills"][0][2],
        Value::Text("item".into())
    );
    assert_eq!(rows(&a.index_path, &a.root)["projects"].len(), 1);
    assert_eq!(tree(&a.root), vault_before);
    assert_eq!(tree(&p), project_before);
    assert_eq!(
        fs::read(project_registry::path(&a).unwrap()).unwrap(),
        registry
    );
}

#[test]
fn missing_index_restores_groups_projects_and_activations_only_for_selected_vault() {
    let t = Temp::new();
    let a = t.vault("a");
    let b = t.vault("b");
    t.seed(&a, "item");
    t.seed(&b, "item");
    let p = t.project(&a, "project-a", "item");
    t.project(&b, "project-b", "item");
    fs::remove_file(&a.index_path).unwrap();
    let before = tree(&p);
    let result = index::rebuild(&a, false).unwrap();
    assert!(result.other_vaults_recovery_required);
    assert!(!result.other_vaults_preserved);
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("other vault"))
    );
    assert_eq!(
        (result.groups, result.projects, result.activations),
        (1, 1, 1)
    );
    assert!(rows(&a.index_path, &b.root).values().all(Vec::is_empty));
    assert_eq!(tree(&p), before);
}

#[test]
fn damaged_owned_database_is_backed_up_and_other_vaults_are_not_claimed_as_recovered() {
    let t = Temp::new();
    let a = t.vault("a");
    t.seed(&a, "item");
    t.project(&a, "project", "item");
    let mut bytes = fs::read(&a.index_path).unwrap();
    bytes[100] = 0xff;
    fs::write(&a.index_path, &bytes).unwrap();
    assert!(index::check(&a.index_path).is_err());
    let before = tree(&t.0);
    assert_eq!(index::rebuild(&a, true).unwrap().mode, "owned_corrupt");
    assert_eq!(tree(&t.0), before);
    let report = index::rebuild(&a, false).unwrap();
    assert!(report.other_vaults_recovery_required);
    assert_eq!(fs::read(report.backup.unwrap()).unwrap(), bytes);
    index::check(&a.index_path).unwrap();
    assert_eq!(rows(&a.index_path, &a.root)["activations"].len(), 1);
}

#[test]
fn foreign_unknown_and_future_version_indices_are_preserved_without_staging() {
    for variant in ["foreign-sqlite", "unknown-bytes", "future-version"] {
        let t = Temp::new();
        let v = t.vault("vault");
        if variant == "unknown-bytes" {
            fs::write(&v.index_path, b"unknown user data").unwrap();
        } else {
            let c = Connection::open(&v.index_path).unwrap();
            if variant == "foreign-sqlite" {
                c.pragma_update(None, "application_id", 42).unwrap();
            } else {
                c.pragma_update(None, "user_version", 99).unwrap();
            }
        }
        let before = tree(&t.0);
        for dry in [true, false] {
            assert_eq!(index::rebuild(&v, dry).unwrap_err().code, "INDEX_CONFLICT");
            assert_eq!(tree(&t.0), before);
        }
    }
}

#[cfg(unix)]
#[test]
fn sidecars_and_symlinked_index_or_registry_are_refused_without_following_them() {
    for variant in ["journal", "wal", "shm", "index-link", "registry-link"] {
        let t = Temp::new();
        let v = t.vault("vault");
        let foreign = t.0.join("foreign");
        fs::write(&foreign, "keep").unwrap();
        match variant {
            "index-link" => {
                fs::remove_file(&v.index_path).unwrap();
                std::os::unix::fs::symlink(&foreign, &v.index_path).unwrap();
            }
            "registry-link" => {
                std::os::unix::fs::symlink(&foreign, project_registry::path(&v).unwrap()).unwrap();
            }
            suffix => {
                fs::write(
                    v.index_path.with_file_name(format!("index.db-{suffix}")),
                    "foreign sidecar",
                )
                .unwrap();
            }
        }
        let before = tree(&t.0);
        assert!(index::rebuild(&v, false).is_err());
        assert_eq!(tree(&t.0), before);
    }
}

#[cfg(unix)]
#[test]
fn registered_bad_projects_are_reported_and_never_followed_repaired_or_deleted() {
    let t = Temp::new();
    let v = t.vault("vault");
    let other = t.vault("other");
    t.seed(&v, "item");
    let good = t.project(&v, "good", "item");
    let original_lock = fs::read(good.join(".kinana.lock")).unwrap();
    let invalid = t.0.join("invalid");
    fs::create_dir(&invalid).unwrap();
    fs::write(invalid.join(".kinana.lock"), "not toml [").unwrap();
    let foreign = t.0.join("foreign-project");
    fs::create_dir(&foreign).unwrap();
    let mut lock: kinana_core::ProjectLock =
        toml::from_str(std::str::from_utf8(&original_lock).unwrap()).unwrap();
    lock.vault = other.root.clone();
    fs::write(
        foreign.join(".kinana.lock"),
        toml::to_string(&lock).unwrap(),
    )
    .unwrap();
    let linked = t.0.join("linked");
    std::os::unix::fs::symlink(&good, &linked).unwrap();
    let inactive = t.0.join("inactive");
    fs::create_dir(&inactive).unwrap();
    let missing = t.0.join("missing");
    let paths = [
        good.clone(),
        invalid.clone(),
        foreign.clone(),
        linked.clone(),
        inactive.clone(),
        missing,
    ];
    let registry = ProjectRegistry {
        version: 1,
        projects: paths
            .into_iter()
            .map(|path| ProjectRegistration {
                path,
                vault: v.root.clone(),
            })
            .collect(),
    };
    fs::write(
        project_registry::path(&v).unwrap(),
        toml::to_string(&registry).unwrap(),
    )
    .unwrap();
    let before = [&good, &invalid, &foreign, &inactive]
        .into_iter()
        .map(|p| (p.clone(), tree(p)))
        .collect::<Vec<_>>();
    let report = index::rebuild(&v, false).unwrap();
    assert_eq!((report.projects, report.activations), (1, 1));
    let states = report
        .observations
        .iter()
        .map(|o| o.status.as_str())
        .collect::<Vec<_>>();
    for state in [
        "ok",
        "invalid_lock",
        "foreign_lock",
        "invalid_path",
        "missing_lock",
        "missing_project",
    ] {
        assert!(states.contains(&state), "{states:?}");
    }
    for (p, before) in before {
        assert_eq!(tree(&p), before);
    }
    assert!(linked.is_symlink());
}

#[test]
fn doctor_diagnoses_broken_targets_and_cache_mismatch_without_writes() {
    let t = Temp::new();
    let v = t.vault("vault");
    t.seed(&v, "item");
    let p = t.project(&v, "project", "item");
    fs::remove_file(v.root.join("dist/item/SKILL.md")).unwrap();
    let before = tree(&t.0);
    let report = doctor::check(&v, &t.0).unwrap();
    assert!(!report.healthy);
    assert!(
        report
            .checks
            .iter()
            .any(|check| check.name == "activation_targets" && !check.passed)
    );
    assert_eq!(tree(&t.0), before);
    let result = index::rebuild(&v, false).unwrap();
    assert_eq!(result.activations, 1);
    assert!(p.join(".claude/skills/item").is_symlink());
    assert!(!v.root.join("dist/item/SKILL.md").exists());
}

#[test]
fn invalid_registry_or_metadata_stops_before_any_index_write() {
    for variant in ["registry", "metadata"] {
        let t = Temp::new();
        let v = t.vault("vault");
        t.seed(&v, "item");
        if variant == "registry" {
            fs::write(project_registry::path(&v).unwrap(), "version=99").unwrap();
        } else {
            fs::write(v.root.join("meta/item.toml"), "not toml [").unwrap();
        }
        let before = tree(&t.0);
        assert!(index::rebuild(&v, false).is_err());
        assert_eq!(tree(&t.0), before);
    }
}

#[test]
fn staging_sql_failure_does_not_modify_the_live_index_or_authoritative_files() {
    let t = Temp::new();
    let v = t.vault("vault");
    t.seed(&v, "item");
    let p = t.project(&v, "project", "item");
    let c = Connection::open(&v.index_path).unwrap();
    c.execute_batch("CREATE TRIGGER refuse BEFORE DELETE ON projects BEGIN SELECT RAISE(FAIL,'injected rebuild failure'); END;").unwrap();
    drop(c);
    let database = fs::read(&v.index_path).unwrap();
    let source = tree(&v.root);
    let project = tree(&p);
    let error = index::rebuild(&v, false).unwrap_err();
    assert!(error.message.contains("preserved at"));
    assert_eq!(fs::read(&v.index_path).unwrap(), database);
    assert_eq!(tree(&v.root), source);
    assert_eq!(tree(&p), project);
}

#[test]
fn foreign_scope_changes_from_a_database_trigger_abort_the_rebuild() {
    let t = Temp::new();
    let a = t.vault("a");
    let b = t.vault("b");
    t.seed(&a, "item");
    t.seed(&b, "item");
    t.project(&a, "project-a", "item");
    t.project(&b, "project-b", "item");
    let c = Connection::open(&a.index_path).unwrap();
    c.execute_batch("CREATE TRIGGER tamper_other AFTER DELETE ON projects BEGIN UPDATE projects SET bundle='tampered'; END;").unwrap();
    drop(c);
    let before = fs::read(&a.index_path).unwrap();
    let other = rows(&a.index_path, &b.root);
    let error = index::rebuild(&a, false).unwrap_err();
    assert_eq!(error.code, "INDEX_CONFLICT");
    assert!(error.message.contains("another vault scope"));
    assert_eq!(fs::read(&a.index_path).unwrap(), before);
    assert_eq!(rows(&a.index_path, &b.root), other);
}
