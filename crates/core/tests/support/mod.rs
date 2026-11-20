use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

pub fn temp_dir() -> io::Result<PathBuf> {
    // Resolve macOS's /var symlink, but keep our own name short enough for
    // nested Unix socket fixtures under the default system temp directory.
    let base = std::env::temp_dir().canonicalize()?;
    loop {
        let path = base.join(format!(
            "sb-{:x}-{:x}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            // A previous run may have left a directory with a reused PID.
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
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
