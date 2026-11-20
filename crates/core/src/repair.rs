//! `kinana doctor --repair`: return an interrupted change to the last commit
//! from inside Kinana, after a preview the user confirms (owner decisions
//! 2026-11-17, fourth and sixth rounds). Nothing is printed for a shell to
//! read again: paths reach Git byte for byte on stdin.
//!
//! Before anything in the vault changes, every tracked file about to be
//! replaced, Git's index and every private stage are copied to
//! `~/.kinana/repair-backups/<timestamp>/` and each copy is verified. Stages
//! are moved there, never deleted outright. Kinana never deletes that folder.
use crate::{
    DoctorRepairReport, Error, Result, doctor,
    gitlog::{self, Uncommitted},
    index, state,
    vault::Vault,
};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const BACKUPS: &str = "repair-backups";

struct Plan {
    restore: Vec<String>,
    unstage: Vec<String>,
    recreate_directories: Vec<String>,
    stages: Vec<String>,
    blocked: Vec<String>,
    untracked: Vec<String>,
    /// Everything the confirmation stands for: the last commit, Git's index,
    /// and the current state of every path the plan names or touches.
    digest: blake3::Hash,
}

impl Plan {
    fn is_empty(&self) -> bool {
        self.restore.is_empty()
            && self.unstage.is_empty()
            && self.recreate_directories.is_empty()
            && self.stages.is_empty()
    }
}

/// Preview without `confirm` (or with `--dry-run`); execute the previewed plan
/// with the token the preview returned.
pub fn repair(
    vault: &Vault,
    home: &Path,
    confirm: Option<&str>,
    dry_run: bool,
) -> Result<DoctorRepairReport> {
    let root = index::validate_path(&vault.root)?;
    let vault = Vault::new(root.clone(), vault.index_path.clone());
    let Some(token) = confirm.filter(|_| !dry_run) else {
        let plan = plan(&vault)?;
        let stamp = stamp()?;
        let mut report = report(&vault, &plan, true);
        if !plan.is_empty() {
            report.preview_token = token_for(&stamp, &plan);
            report.backup_directory = Some(state::current(home).join(BACKUPS).join(&stamp));
        }
        return Ok(report);
    };
    let stamp = parse_token(token)?;
    let _guard = state::write_guard(&vault.index_path, false)?;
    let plan = plan(&vault)?;
    if plan.is_empty() || token_for(&stamp, &plan) != token {
        return Err(Error::decision(
            "DOCTOR_REPAIR_STALE",
            "The vault changed since the repair preview; nothing was written. Run kinana doctor --repair again and review the new preview",
        ));
    }
    let mut report = report(&vault, &plan, false);
    let backup = backup(&vault, home, &stamp, &plan).map_err(|error| {
        Error::decision(
            "DOCTOR_REPAIR_BACKUP_FAILED",
            format!(
                "The backup before the repair failed, so the vault was not touched: {}",
                error.message
            ),
        )
    })?;
    report.backed_up = backup.files.clone();
    report.backup_directory = Some(backup.directory.clone());
    // From here on the vault changes: any failure names the backup.
    let failed = |error: Error| {
        Error::new(
            error.code.as_str(),
            format!(
                "The repair stopped part way: {}. Everything it replaces was copied to {} first. Run kinana doctor to see what is left",
                error.message,
                backup.directory.display()
            ),
        )
    };
    let changed = (|| -> Result<()> {
        gitlog::restore_paths(&root, &plan.restore, true)?;
        gitlog::restore_paths(&root, &plan.unstage, false)?;
        for directory in &plan.recreate_directories {
            // A link or file put in the way since the plan is left alone.
            if gitlog::blocked_on_disk(&root, directory) {
                report.warnings.push(format!(
                    "Not recreated, something else is in the way: {}",
                    crate::names::visible(directory)
                ));
                continue;
            }
            fs::create_dir_all(index::validate_path(&root.join(directory))?)?;
        }
        for stage in &plan.stages {
            // Checked again here: `dist` may have become a link since the
            // plan (review of 4687cc5). A link on the way is refused.
            let path = stage_path(&root, stage)?;
            if fs::symlink_metadata(&path)?.is_dir() {
                fs::remove_dir_all(&path)?;
            } else {
                fs::remove_file(&path)?;
            }
        }
        Ok(())
    })();
    changed.map_err(failed)?;
    if (!plan.restore.is_empty() || !plan.unstage.is_empty())
        && let Err(error) = index::rebuild(&vault, false)
    {
        report.warnings.push(format!(
            "The index was not rebuilt: {}. Run kinana doctor --rebuild-index",
            error.message
        ));
    }
    report.after = Some(doctor::check(&vault, home).map_err(failed)?);
    Ok(report)
}

/// A stage's path with no link on the way to it (the stage itself may be a
/// link, which is moved as one).
fn stage_path(root: &Path, stage: &str) -> Result<PathBuf> {
    let path = root.join(stage);
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(Error::new("INVALID_PATH", "A stage has no parent"));
    };
    Ok(index::validate_path(parent)?.join(name))
}

fn report(vault: &Vault, plan: &Plan, dry_run: bool) -> DoctorRepairReport {
    DoctorRepairReport {
        vault: vault.root.clone(),
        dry_run,
        preview_token: String::new(),
        backup_directory: None,
        restore: plan.restore.clone(),
        unstage: plan.unstage.clone(),
        recreate_directories: plan.recreate_directories.clone(),
        staging_removed: plan.stages.clone(),
        backed_up: Vec::new(),
        blocked: plan.blocked.clone(),
        untracked: plan.untracked.clone(),
        warnings: Vec::new(),
        after: None,
    }
}

fn plan(vault: &Vault) -> Result<Plan> {
    let root = &vault.root;
    let found = gitlog::uncommitted(root)?.unwrap_or(Uncommitted {
        restore: Vec::new(),
        unstage: Vec::new(),
        untracked: Vec::new(),
        blocked: Vec::new(),
    });
    let recreate_directories = if found.all().is_empty() {
        Vec::new()
    } else {
        doctor::missing_empty_directories(vault, &found)
    };
    let stages = doctor::stages(vault);
    // Files inside a stage move with it; they are not "left where they are".
    let mut found = found;
    found.untracked.retain(|path| {
        !stages
            .iter()
            .any(|stage| path == stage || path.starts_with(&format!("{stage}/")))
    });
    let mut digest = blake3::Hasher::new();
    digest.update(b"kinana doctor --repair v1\0");
    digest.update(gitlog::head_commit(root)?.as_bytes());
    digest.update(b"\0index\0");
    match fs::read(root.join(".git/index")) {
        Ok(bytes) => digest.update(blake3::hash(&bytes).as_bytes()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => digest.update(b"none"),
        Err(error) => return Err(error.into()),
    };
    for (label, paths) in [
        ("restore", &found.restore),
        ("unstage", &found.unstage),
        ("recreate", &recreate_directories),
        ("blocked", &found.blocked),
        ("untracked", &found.untracked),
    ] {
        digest.update(format!("\0{label}\0").as_bytes());
        for path in paths {
            digest.update(path.as_bytes());
            digest.update(b"\0");
            digest.update(describe(root, path)?.as_bytes());
            digest.update(b"\0");
        }
    }
    digest.update(b"\0stages\0");
    for stage in &stages {
        digest.update(stage.as_bytes());
        digest.update(b"\0");
        for (relative, state) in tree(&root.join(stage), true)? {
            digest.update(relative.as_bytes());
            digest.update(b"\0");
            digest.update(state.as_bytes());
            digest.update(b"\0");
        }
    }
    Ok(Plan {
        restore: found.restore,
        unstage: found.unstage,
        recreate_directories,
        stages,
        blocked: found.blocked,
        untracked: found.untracked,
        digest: digest.finalize(),
    })
}

/// What is at `relative` now, without following a link anywhere on the way.
fn describe(root: &Path, relative: &str) -> Result<String> {
    let mut path = root.to_path_buf();
    let parts = relative.split('/').collect::<Vec<_>>();
    for (n, part) in parts.iter().enumerate() {
        path.push(part);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                return Ok("missing".into());
            }
            Err(error) => return Err(error.into()),
        };
        if n + 1 < parts.len() {
            if !metadata.is_dir() {
                return Ok("blocked".into());
            }
            continue;
        }
        return state_of(&path, &metadata);
    }
    Ok("missing".into())
}

fn state_of(path: &Path, metadata: &fs::Metadata) -> Result<String> {
    state_with(path, metadata, true)
}

/// Without `modes`, only kinds, link targets and bytes count: the backup
/// copies are private whatever the original's mode was.
fn state_with(path: &Path, metadata: &fs::Metadata, modes: bool) -> Result<String> {
    let kind = metadata.file_type();
    let mode = if modes {
        format!("{:o}", mode(metadata))
    } else {
        String::new()
    };
    Ok(if kind.is_symlink() {
        format!("link:{}", fs::read_link(path)?.to_string_lossy())
    } else if kind.is_dir() {
        format!("dir:{mode}")
    } else if kind.is_file() {
        format!("file:{mode}:{}", blake3::hash(&fs::read(path)?))
    } else {
        "other".into()
    })
}

fn mode(metadata: &fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o7777
    }
    #[cfg(not(unix))]
    {
        u32::from(metadata.permissions().readonly())
    }
}

/// Every entry under a stage (or the stage itself when it is a file), in a
/// stable order, never following a link.
fn tree(top: &Path, modes: bool) -> Result<Vec<(String, String)>> {
    let mut entries = Vec::new();
    for entry in walkdir::WalkDir::new(top)
        .follow_links(false)
        .follow_root_links(false)
        .sort_by_file_name()
    {
        let entry = entry.map_err(|error| Error::new("IO_ERROR", error.to_string()))?;
        let relative = entry
            .path()
            .strip_prefix(top)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .into_owned();
        let metadata = fs::symlink_metadata(entry.path())?;
        entries.push((relative, state_with(entry.path(), &metadata, modes)?));
    }
    Ok(entries)
}

fn stamp() -> Result<String> {
    Ok(crate::import::timestamp()?.replace(['-', ':'], ""))
}

fn token_for(stamp: &str, plan: &Plan) -> String {
    let mut digest = blake3::Hasher::new();
    digest.update(stamp.as_bytes());
    digest.update(b"\0");
    digest.update(plan.digest.as_bytes());
    format!("{stamp}.{}", digest.finalize().to_hex())
}

/// `YYYYMMDDTHHMMSSZ.<64 hex>`: the stamp names the backup directory, so it
/// is checked character by character before it is used in a path.
fn parse_token(token: &str) -> Result<String> {
    let invalid = || {
        Error::decision(
            "DOCTOR_REPAIR_STALE",
            "This is not a repair preview token; run kinana doctor --repair and use the token it prints",
        )
    };
    let (stamp, hash) = token.split_once('.').ok_or_else(invalid)?;
    let bytes = stamp.as_bytes();
    let stamp_ok = bytes.len() == 16
        && bytes[8] == b'T'
        && bytes[15] == b'Z'
        && bytes
            .iter()
            .enumerate()
            .all(|(n, byte)| n == 8 || n == 15 || byte.is_ascii_digit());
    let hash_ok = hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if !stamp_ok || !hash_ok {
        return Err(invalid());
    }
    Ok(stamp.to_owned())
}

struct Backup {
    directory: PathBuf,
    files: Vec<String>,
}

/// Copies everything the repair replaces or moves, and verifies each copy,
/// before the vault is touched. Directories are private (0700), files 0600.
fn backup(vault: &Vault, home: &Path, stamp: &str, plan: &Plan) -> Result<Backup> {
    let state_directory = state::directory(home)?;
    if state_directory != state::current(&index::validate_path(home)?) {
        return Err(Error::new(
            "MIGRATION_REQUIRED",
            "Repair backups are kept in ~/.kinana; run kinana migrate first",
        ));
    }
    // The user confirmed this repair; its backup may be the first state
    // Kinana keeps.
    private_directory(&state_directory, true)?;
    let base = state_directory.join(BACKUPS);
    private_directory(&base, true)?;
    let directory = base.join(stamp);
    private_directory(&directory, false).map_err(|error| {
        Error::new(
            error.code.as_str(),
            format!(
                "{}. Run kinana doctor --repair again for a new preview",
                error.message
            ),
        )
    })?;
    let root = &vault.root;
    let mut files = Vec::new();
    let mut manifest = String::from("# Kinana doctor --repair backup\n");
    for path in &plan.restore {
        let source = root.join(path);
        let state = describe(root, path)?;
        if !state.starts_with("file:") {
            continue;
        }
        let target = directory.join("vault").join(path);
        copy_file(&source, &target, &directory)?;
        if describe(root, path)? != state {
            return Err(Error::new(
                "IO_ERROR",
                format!(
                    "{} changed while it was copied",
                    crate::names::visible(path)
                ),
            ));
        }
        manifest.push_str(&format!("vault/{path}\t{state}\n"));
        files.push(path.clone());
    }
    // Content only Git's index holds (staged, then changed or removed on
    // disk) is lost from the index by the restore: copy it too.
    for path in plan.restore.iter().chain(&plan.unstage) {
        let Some(bytes) = gitlog::index_file(root, path)? else {
            continue;
        };
        if gitlog::head_file(root, path)?.as_deref() == Some(bytes.as_slice()) {
            continue;
        }
        let target = directory.join("index").join(path);
        private_parents(&target, &directory)?;
        let mut file = new_private_file(&target)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        manifest.push_str(&format!("index/{path}\t{}\n", blake3::hash(&bytes)));
    }
    let git_index = root.join(".git/index");
    if fs::symlink_metadata(&git_index).is_ok_and(|metadata| metadata.is_file()) {
        copy_file(&git_index, &directory.join("git-index"), &directory)?;
        manifest.push_str("git-index\n");
    }
    for stage in &plan.stages {
        let source = stage_path(root, stage)?;
        let target = directory.join("stages").join(stage);
        copy_tree(&source, &target, &directory)?;
        if tree(&source, false)? != tree(&target, false)? {
            return Err(Error::new(
                "IO_ERROR",
                format!(
                    "The copy of {} does not match it",
                    crate::names::visible(stage)
                ),
            ));
        }
        for (relative, state) in tree(&source, true)? {
            manifest.push_str(&format!("stages/{stage}/{relative}\t{state}\n"));
        }
    }
    let mut file = new_private_file(&directory.join("MANIFEST.txt"))?;
    file.write_all(manifest.as_bytes())?;
    file.sync_all()?;
    Ok(Backup { directory, files })
}

/// A real directory with mode 0700: created when missing; when `existing_ok`
/// an existing real directory is used, never a link.
fn private_directory(path: &Path, existing_ok: bool) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && existing_ok => {
            if fs::symlink_metadata(path)?.is_dir() {
                Ok(())
            } else {
                Err(Error::new(
                    "STATE_CONFLICT",
                    format!("{} is not a real directory", path.display()),
                ))
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Err(Error::new(
            "IO_ERROR",
            format!("{} already exists", path.display()),
        )),
        Err(error) => Err(error.into()),
    }
}

/// Creates the parents of `target` below `inside`, each private.
fn private_parents(target: &Path, inside: &Path) -> Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| Error::new("INVALID_PATH", "Backup path has no parent"))?;
    if parent == inside || parent.is_dir() {
        return Ok(());
    }
    private_parents(parent, inside)?;
    private_directory(parent, true)
}

fn new_private_file(path: &Path) -> Result<fs::File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

fn copy_file(source: &Path, target: &Path, inside: &Path) -> Result<()> {
    private_parents(target, inside)?;
    let bytes = fs::read(source)?;
    let mut file = new_private_file(target)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    if blake3::hash(&fs::read(target)?) != blake3::hash(&bytes) {
        return Err(Error::new(
            "IO_ERROR",
            format!("The copy of {} does not match it", source.display()),
        ));
    }
    Ok(())
}

/// Copies a stage as it is: files with their bytes, directories, and links as
/// links (never followed), all private; the original modes are in the
/// manifest. Anything else stops the backup.
fn copy_tree(source: &Path, target: &Path, inside: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    let kind = metadata.file_type();
    if kind.is_symlink() {
        private_parents(target, inside)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(fs::read_link(source)?, target)?;
        return Ok(());
    }
    if kind.is_file() {
        return copy_file(source, target, inside);
    }
    if !kind.is_dir() {
        return Err(Error::new(
            "IO_ERROR",
            format!(
                "{} is neither a file, a directory nor a link",
                source.display()
            ),
        ));
    }
    private_parents(target, inside)?;
    private_directory(target, false)?;
    let mut names = fs::read_dir(source)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<Vec<_>>>()?;
    names.sort();
    for name in names {
        copy_tree(&source.join(&name), &target.join(&name), inside)?;
    }
    Ok(())
}
