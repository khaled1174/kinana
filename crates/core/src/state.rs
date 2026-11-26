//! Read old state without writes; all new writers share one process/OS lease.
use crate::{Error, Result, index};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
};

const LOCK_BYTES: &[u8] = b"Kinana state writer lease v1\n";
static LEASES: OnceLock<Mutex<BTreeMap<PathBuf, Weak<Lease>>>> = OnceLock::new();
struct Lease {
    file: File,
    migration: bool,
}
/// Release the lock explicitly before the file closes. A child process
/// spawned on another thread holds a copy of this open file until it starts
/// its program; closing our descriptor alone would leave the lock held by
/// that copy for a moment, and the next writer would see a false
/// `STATE_BUSY`. `flock(LOCK_UN)` releases it for every copy at once.
impl Drop for Lease {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}
pub struct WriteGuard {
    _lease: Option<Arc<Lease>>,
}
/// The last guard of a lease releases it while holding the lease map lock,
/// the same lock a new acquisition holds from lookup to `try_lock`. Without
/// it, another thread could find the lease gone and try the file lock
/// before this thread's unlock ran, and see a false `STATE_BUSY`.
impl Drop for WriteGuard {
    fn drop(&mut self) {
        let Some(lease) = self._lease.take() else {
            return;
        };
        let held = LEASES.get().map(|leases| leases.lock());
        drop(lease);
        drop(held);
    }
}

pub fn legacy(home: &Path) -> PathBuf {
    home.join(".skillsbox")
}
pub fn current(home: &Path) -> PathBuf {
    home.join(".kinana")
}
pub(crate) fn pending(home: &Path) -> PathBuf {
    home.join(".kinana-migrating.json")
}

pub(crate) fn exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}
fn real_directory(path: &Path) -> Result<bool> {
    index::validate_path(path)?;
    if !exists(path)? {
        return Ok(false);
    }
    if !fs::symlink_metadata(path)?.is_dir() {
        return Err(Error::decision(
            "STATE_CONFLICT",
            "Application state must be a real directory; existing node preserved",
        ));
    }
    Ok(true)
}
pub fn directory(home: &Path) -> Result<PathBuf> {
    let home = index::validate_path(home)?;
    if exists(&pending(&home))? {
        return Err(Error::decision(
            "MIGRATION_INCOMPLETE",
            "A migration or rollback needs explicit recovery before other operations",
        ));
    }
    let old = legacy(&home);
    let new = current(&home);
    let has_old = real_directory(&old)?;
    let has_new = real_directory(&new)?;
    if has_new {
        crate::migration::validate_active_state(&home, has_old)?;
        Ok(new)
    } else if has_old {
        Ok(old)
    } else {
        Ok(new)
    }
}
pub fn index_path(home: &Path) -> Result<PathBuf> {
    let directory = directory(home)?;
    // Legacy state is read-only until `kinana migrate` and holds only the
    // earlier version's index, which is read where it is.
    let name = if directory
        .file_name()
        .is_some_and(|name| name == ".skillsbox")
    {
        "index.db"
    } else {
        index::FILE_NAME
    };
    Ok(directory.join(name))
}
pub fn require_writable(index_path: &Path) -> Result<()> {
    let Some(parent) = index_path.parent() else {
        return Ok(());
    };
    let Some(name) = parent.file_name().and_then(|s| s.to_str()) else {
        return Ok(());
    };
    if name == ".skillsbox" {
        return Err(Error::decision(
            "MIGRATION_REQUIRED",
            "Legacy state is read-only. Preview and confirm kinana migrate before modifying it",
        ));
    }
    if name == ".kinana" {
        let home = parent
            .parent()
            .ok_or_else(|| Error::new("INVALID_PATH", "State has no home"))?;
        if directory(home)? != parent {
            return Err(Error::decision(
                "STATE_CONFLICT",
                "Selected state does not match the active application state",
            ));
        }
    }
    Ok(())
}
pub fn write_guard(index_path: &Path, dry_run: bool) -> Result<WriteGuard> {
    if dry_run {
        return Ok(WriteGuard { _lease: None });
    }
    require_writable(index_path)?;
    let Some(parent) = index_path
        .parent()
        .filter(|p| p.file_name().is_some_and(|n| n == ".kinana"))
    else {
        return Ok(WriteGuard { _lease: None });
    };
    let guard = lease(
        parent
            .parent()
            .ok_or_else(|| Error::new("INVALID_PATH", "State has no home"))?,
        false,
    )?;
    // A rollback may have finished after the preflight and before the lease.
    require_writable(index_path)?;
    Ok(guard)
}
pub(crate) fn migration_guard(home: &Path) -> Result<WriteGuard> {
    lease(home, true)
}
pub(crate) fn preflight_lease(home: &Path) -> Result<()> {
    let path = index::validate_path(&home.join(".kinana-write.lock"))?;
    if !exists(&path)? {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_file() || metadata.len() != LOCK_BYTES.len() as u64 {
        return Err(Error::decision(
            "STATE_CONFLICT",
            "Foreign state lease preserved",
        ));
    }
    let mut file = File::open(&path)?;
    if !same_file(&metadata, &file.metadata()?) {
        return Err(Error::decision(
            "STATE_CONFLICT",
            "State lease was replaced",
        ));
    }
    let mut data = vec![];
    file.read_to_end(&mut data)?;
    if data != LOCK_BYTES {
        return Err(Error::decision(
            "STATE_CONFLICT",
            "Foreign state lease preserved",
        ));
    }
    Ok(())
}
/// Creates the writer lease file ahead of any writer, so two writers never
/// both find it missing and race to create it (owner decision 2026-11-17).
/// `init` and the app's start on existing state call this. The file appears
/// complete or not at all: its bytes are written to a private file under
/// `~/.kinana/` and hard-linked into place. The lock itself (`flock` on the
/// same file, same bytes) is unchanged. An existing file is validated and
/// never rewritten; legacy state, read-only until `migrate`, gets nothing.
pub fn ensure_lease_file(home: &Path) -> Result<()> {
    let home = index::validate_path(home)?;
    let state = current(&home);
    if directory(&home)? != state {
        return Ok(());
    }
    let path = index::validate_path(&home.join(".kinana-write.lock"))?;
    if exists(&path)? {
        return preflight_lease(&home);
    }
    if !real_directory(&state)? {
        fs::create_dir_all(&state)?;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let staged = index::validate_path(&state.join(format!(
        ".kinana-write.lock.{}.{nanos}.tmp",
        std::process::id()
    )))?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| -> Result<()> {
        let mut file = options.open(&staged)?;
        file.write_all(LOCK_BYTES)?;
        file.sync_all()?;
        match fs::hard_link(&staged, &path) {
            Ok(()) => Ok(()),
            // Another process created it first: validate theirs.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => preflight_lease(&home),
            Err(e) => Err(e.into()),
        }
    })();
    let _ = fs::remove_file(&staged);
    result
}

fn lease(home: &Path, exclusive_in_process: bool) -> Result<WriteGuard> {
    let mut leases = LEASES
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| Error::new("STATE_BUSY", "State lease is unavailable"))?;
    // Checked under the map lock: another thread of this process may be
    // creating the file and writing its bytes right now.
    preflight_lease(home)?;
    let path = index::validate_path(&home.join(".kinana-write.lock"))?;
    if let Some(lease) = leases.get(&path).and_then(Weak::upgrade) {
        if exclusive_in_process || lease.migration {
            return Err(Error::decision(
                "STATE_BUSY",
                "Another state operation is running",
            ));
        }
        if !same_file(&lease.file.metadata()?, &fs::symlink_metadata(&path)?) {
            return Err(Error::decision(
                "STATE_CONFLICT",
                "State lease was replaced",
            ));
        }
        return Ok(WriteGuard {
            _lease: Some(lease),
        });
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = match options.open(&path) {
        Ok(mut file) => {
            file.write_all(LOCK_BYTES)?;
            file.sync_all()?;
            file
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            if !fs::symlink_metadata(&path)?.is_file() {
                return Err(Error::decision(
                    "STATE_CONFLICT",
                    "Foreign state lease preserved",
                ));
            }
            OpenOptions::new().read(true).write(true).open(&path)?
        }
        Err(e) => return Err(e.into()),
    };
    file.try_lock()
        .map_err(|_| Error::decision("STATE_BUSY", "Another Kinana process is modifying state"))?;
    // From here on every exit, including the refusals below, unlocks.
    let mut lease = Lease {
        file,
        migration: exclusive_in_process,
    };
    let mut bytes = Vec::new();
    lease.file.read_to_end(&mut bytes)?;
    // A newly created file's cursor is at EOF; read it again for the same validation.
    if bytes.is_empty() {
        use std::io::{Seek, SeekFrom};
        lease.file.seek(SeekFrom::Start(0))?;
        lease.file.read_to_end(&mut bytes)?;
    }
    if bytes != LOCK_BYTES || !same_file(&lease.file.metadata()?, &fs::symlink_metadata(&path)?) {
        return Err(Error::decision(
            "STATE_CONFLICT",
            "Foreign or replaced state lease preserved",
        ));
    }
    let lease = Arc::new(lease);
    leases.insert(path, Arc::downgrade(&lease));
    Ok(WriteGuard {
        _lease: Some(lease),
    })
}
fn same_file(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    {
        a.is_file() == b.is_file() && a.len() == b.len() && a.created().ok() == b.created().ok()
    }
}
