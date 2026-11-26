use kinana_core::{
    Classification, Description, SkillMetadata, SourceInfo, hash::hash_directory,
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
pub struct Sandbox {
    pub root: PathBuf,
    pub home: PathBuf,
    pub vault: PathBuf,
}
impl Sandbox {
    pub fn new() -> Self {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "kinana-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let home = root.join("home");
        fs::create_dir(&home).unwrap();
        let vault = root.join("vault");
        Self { root, home, vault }
    }
    pub fn run(&self, args: &[&str]) -> Output {
        // Since 0.3 the default `--agent auto` starts Claude: no test reaches
        // a real agent through PATH (fixed install directories excepted; see
        // scripts/test-without-agents.sh for a run that excludes them too).
        Command::new(env!("CARGO_BIN_EXE_kinana"))
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .arg("--vault")
            .arg(&self.vault)
            .args(args)
            .output()
            .unwrap()
    }
    pub fn initialize(&self) {
        let output = self.run(&["init", "--json"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
    pub fn seed(&self, id: &str, name: &str, category: &str) {
        let skill = self.vault.join("skills").join(id);
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), format!("---\nname: {name}\ndescription: Review API contracts\n---\n\n<script>untrusted()</script>\n")).unwrap();
        let meta = SkillMetadata {
            proposal_source_changed: None,
            classification_context: None,
            pinned: false,
            id: id.into(),
            content_hash: hash_directory(&skill).unwrap(),
            added_at: "2026-11-13T00:00:00Z".into(),
            source: SourceInfo::default(),
            classification: Classification {
                category: category.into(),
                ..Classification::default()
            },
            description: Description {
                original: "Review API contracts".into(),
                ..Description::default()
            },
            audit: kinana_core::audit::audit_skill(&skill).unwrap(),
            empty_directories: vec![],
            proposal: None,
        };
        fs::write(
            self.vault.join("meta").join(format!("{id}.toml")),
            toml::to_string(&meta).unwrap(),
        )
        .unwrap();
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub fn copy_tree(source: &Path, dest: &Path) {
    fs::create_dir_all(dest).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = dest.join(entry.file_name());
        let ty = entry.file_type().unwrap();
        if ty.is_dir() {
            copy_tree(&entry.path(), &target);
        } else if ty.is_file() {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}
pub fn snapshot(path: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(base: &Path, path: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let p = entry.path();
            if entry.file_type().unwrap().is_dir() {
                out.push((p.strip_prefix(base).unwrap().to_path_buf(), vec![]));
                walk(base, &p, out);
            } else if entry.file_type().unwrap().is_file() {
                out.push((
                    p.strip_prefix(base).unwrap().to_path_buf(),
                    fs::read(&p).unwrap(),
                ));
            }
        }
    }
    let mut result = vec![];
    walk(path, path, &mut result);
    result.sort();
    result
}

/// The statements `index::initialize` ran in the 0.1.0 beta (schema version 1).
#[allow(dead_code)]
pub const BETA_INDEX_SCHEMA: &str = "PRAGMA application_id = 1396856881;
     PRAGMA user_version = 1;
     CREATE TABLE skills (
         vault TEXT NOT NULL, id TEXT NOT NULL, name TEXT NOT NULL,
         description TEXT NOT NULL, category TEXT NOT NULL, audit_level TEXT NOT NULL,
         content_hash TEXT NOT NULL, source_url TEXT, added_at TEXT NOT NULL,
         role TEXT NOT NULL, tags TEXT NOT NULL, PRIMARY KEY(vault,id)
     );
     CREATE VIRTUAL TABLE skills_fts USING fts5(vault UNINDEXED,id UNINDEXED,name,description,role,tags);
     CREATE TABLE locations (vault TEXT NOT NULL,skill_id TEXT NOT NULL,path TEXT NOT NULL,
         content_hash TEXT NOT NULL,kind TEXT NOT NULL,PRIMARY KEY(vault,skill_id,path));
     CREATE TABLE variants (vault TEXT NOT NULL,skill_id TEXT NOT NULL,content_hash TEXT NOT NULL,
         path TEXT NOT NULL,PRIMARY KEY(vault,skill_id,path));
     CREATE TABLE groups (vault TEXT NOT NULL,group_name TEXT NOT NULL,skill_id TEXT NOT NULL,
         PRIMARY KEY(vault,group_name,skill_id));
     CREATE TABLE projects (vault TEXT NOT NULL,path TEXT NOT NULL,bundle TEXT,last_seen TEXT,
         PRIMARY KEY(vault,path));
     CREATE TABLE activations (vault TEXT NOT NULL,project_path TEXT NOT NULL,skill_id TEXT NOT NULL,
         agent TEXT NOT NULL,mode TEXT NOT NULL,PRIMARY KEY(vault,project_path,skill_id,agent));";

/// Leave a state directory as the 0.1.0 beta would have: `index.db` in
/// schema 1 with the rows this version's index holds, and no `index-v2.db`.
#[allow(dead_code)]
pub fn as_left_by_the_beta(state: &Path) {
    let current = state.join("index-v2.db");
    let connection = rusqlite::Connection::open(state.join("index.db")).unwrap();
    connection.execute_batch(BETA_INDEX_SCHEMA).unwrap();
    connection
        .execute("ATTACH DATABASE ?1 AS current", [current.to_str().unwrap()])
        .unwrap();
    connection
        .execute_batch(
            "INSERT INTO skills SELECT vault,id,name,description,category,audit_level,content_hash,source_url,added_at,role,tags FROM current.skills;
             INSERT INTO skills_fts (vault,id,name,description,role,tags) SELECT vault,id,name,description,role,tags FROM current.skills_fts;
             INSERT INTO locations SELECT * FROM current.locations;
             INSERT INTO variants SELECT * FROM current.variants;
             INSERT INTO groups SELECT * FROM current.groups;
             INSERT INTO projects SELECT * FROM current.projects;
             INSERT INTO activations SELECT * FROM current.activations;
             DETACH DATABASE current;",
        )
        .unwrap();
    drop(connection);
    fs::remove_file(current).unwrap();
}

/// 0.3 r5 (owner, 2026-11-24): turns off the automatic approval of clean
/// skills' descriptions, for tests of the manual review flow and of runs that
/// count their commits.
#[allow(dead_code)]
pub fn manual_review(vault: &Path) {
    let path = vault.join("box.toml");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("auto_approve_descriptions = false\n{text}")).unwrap();
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(vault)
        .args([
            "-c",
            "user.name=Kinana Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-qam",
            "Review descriptions by hand",
        ])
        .status()
        .unwrap();
    assert!(status.success());
}
