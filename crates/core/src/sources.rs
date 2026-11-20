//! Prepare untrusted import sources without running any of their files.
use crate::{Error, Result, SourceInfo};
use std::collections::HashMap;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_SCRATCH: AtomicU64 = AtomicU64::new(0);

/// A local, read-only source, or an owned temporary checkout/extraction.
/// Keeping this value alive keeps its temporary source available to callers.
#[derive(Debug)]
pub struct PreparedSource {
    pub root: PathBuf,
    pub provenance: SourceInfo,
    pub label: String,
    _scratch: Option<Scratch>,
}

#[derive(Debug)]
struct Scratch {
    root: PathBuf,
    identity: Metadata,
}

impl Scratch {
    fn new(state_dir: &Path) -> Result<Self> {
        let state_dir = crate::index::validate_path(state_dir)?;
        fs::create_dir_all(&state_dir)?;
        crate::hash::checked_absolute_path(&state_dir)?;
        if !fs::symlink_metadata(&state_dir)?.is_dir() {
            return Err(Error::new(
                "INVALID_SOURCE",
                "Source scratch parent is not a directory",
            ));
        }
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| Error::new("IO_ERROR", error.to_string()))?
            .as_nanos();
        for _ in 0..128 {
            let sequence = NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed);
            let root = state_dir.join(format!(
                "source-{}-{timestamp}-{sequence}",
                std::process::id()
            ));
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&root) {
                Ok(()) => {
                    let identity = fs::symlink_metadata(&root)?;
                    return Ok(Self { root, identity });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(Error::new(
            "IO_ERROR",
            "Could not exclusively create source scratch directory",
        ))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Never follow a substituted parent or delete a replacement directory.
        if crate::hash::checked_absolute_path(&self.root).is_err() {
            return;
        }
        let Ok(current) = fs::symlink_metadata(&self.root) else {
            return;
        };
        if !current.is_dir() || !same_directory(&self.identity, &current) {
            return;
        }
        // remove_dir_all removes contained symlinks themselves, not their targets.
        let _cleanup = fs::remove_dir_all(&self.root);
    }
}

#[cfg(unix)]
fn same_directory(first: &Metadata, second: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    first.dev() == second.dev() && first.ino() == second.ino()
}

#[cfg(not(unix))]
fn same_directory(first: &Metadata, second: &Metadata) -> bool {
    match (first.created(), second.created()) {
        (Ok(first), Ok(second)) => first == second,
        _ => false,
    }
}

/// Identify URL-shaped input, including unsupported URL schemes and hosts.
/// Callers must still use `prepare` to validate a remote source.
pub fn is_remote(input: &str) -> bool {
    input.contains("://") || input.starts_with("git@")
}

/// Validate import syntax and local source/archive metadata without writes or
/// network access. ZIP payloads are not decompressed during this preflight.
pub fn validate_input(input: &str) -> Result<()> {
    if input.trim().is_empty() {
        return Err(Error::new(
            "INVALID_SOURCE",
            "Import source must not be empty",
        ));
    }
    if is_remote(input) {
        GithubSource::parse(input)?;
        return Ok(());
    }
    let path = crate::index::validate_path(Path::new(input))?;
    let metadata = crate::hash::checked_metadata(&path)?;
    if metadata.is_dir() {
        return Ok(());
    }
    if metadata.is_file() {
        open_archive(&path)?;
        return Ok(());
    }
    Err(Error::new(
        "INVALID_SOURCE",
        "Import source must be a local directory or ZIP archive",
    ))
}

pub fn prepare(input: &str, state_dir: &Path) -> Result<PreparedSource> {
    if input.trim().is_empty() {
        return Err(Error::new(
            "INVALID_SOURCE",
            "Import source must not be empty",
        ));
    }
    if is_remote(input) {
        return github(GithubSource::parse(input)?, state_dir, None);
    }
    let root = crate::index::validate_path(Path::new(input))?;
    let metadata = crate::hash::checked_metadata(&root)?;
    if metadata.is_file() {
        return archive_source(&root, state_dir);
    }
    if !metadata.is_dir() {
        return Err(Error::new(
            "INVALID_SOURCE",
            "Import source must be a local directory or a supported archive",
        ));
    }
    let mut provenance = local_git_hint(&root);
    provenance.kind = "local".into();
    provenance.found_in = vec![root.clone()];
    let label = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "local".into());
    Ok(PreparedSource {
        root,
        provenance,
        label,
        _scratch: None,
    })
}

pub fn prepare_controlled(
    input: &str,
    state_dir: &Path,
    control: &crate::import::ImportControl,
) -> Result<PreparedSource> {
    control.stage("preparing")?;
    validate_input(input)?;
    let source = if is_remote(input) {
        github(GithubSource::parse(input)?, state_dir, Some(control))?
    } else {
        prepare(input, state_dir)?
    };
    control.check()?;
    Ok(source)
}

struct ControlledChild {
    child: std::process::Child,
    finished: bool,
}
impl ControlledChild {
    fn stop(&mut self) {
        #[cfg(unix)]
        {
            let _ = Command::new("/bin/kill")
                .args(["-KILL", "--", &format!("-{}", self.child.id())])
                .env_clear()
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Drop for ControlledChild {
    fn drop(&mut self) {
        if !self.finished {
            self.stop();
        }
    }
}
fn controlled_output(
    command: &mut Command,
    control: Option<&crate::import::ImportControl>,
) -> Result<Output> {
    let Some(control) = control else {
        return Ok(command.output()?);
    };
    control.check()?;
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut owned = ControlledChild {
        child: command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?,
        finished: false,
    };
    let stdout = owned
        .child
        .stdout
        .take()
        .ok_or_else(|| Error::new("IO_ERROR", "Missing source stdout"))?;
    let stderr = owned
        .child
        .stderr
        .take()
        .ok_or_else(|| Error::new("IO_ERROR", "Missing source stderr"))?;
    let (tx, rx) = std::sync::mpsc::channel();
    let mut readers = Vec::new();
    for (index, mut pipe) in [
        (0, Box::new(stdout) as Box<dyn Read + Send>),
        (1, Box::new(stderr) as Box<dyn Read + Send>),
    ] {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            let mut v = Vec::new();
            let r = pipe
                .by_ref()
                .take(4 * 1024 * 1024 + 1)
                .read_to_end(&mut v)
                .map(|_| v);
            let _ = tx.send((index, r));
        }));
    }
    drop(tx);
    let mut output: [Option<Vec<u8>>; 2] = [None, None];
    let mut status = None;
    loop {
        control.check()?;
        if status.is_none() {
            status = owned.child.try_wait()?;
        }
        if status.is_some() && output.iter().all(Option::is_some) {
            break;
        }
        match rx.recv_timeout(std::time::Duration::from_millis(25)) {
            Ok((index, result)) => {
                let bytes = result?;
                if bytes.len() > 4 * 1024 * 1024 {
                    return Err(Error::new(
                        "SOURCE_OUTPUT_LIMIT",
                        "Git source output exceeded its bounded limit",
                    ));
                }
                output[index] = Some(bytes);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
                if output.iter().all(Option::is_some) =>
            {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(_) => return Err(Error::new("IO_ERROR", "Source output worker disconnected")),
        }
    }
    for reader in readers {
        reader
            .join()
            .map_err(|_| Error::new("IO_ERROR", "Source output worker failed"))?;
    }
    owned.finished = true;
    control.check()?;
    Ok(Output {
        status: status.ok_or_else(|| Error::new("IO_ERROR", "Source exit status missing"))?,
        stdout: output[0].take().unwrap_or_default(),
        stderr: output[1].take().unwrap_or_default(),
    })
}

#[cfg(all(test, unix))]
mod controlled_tests {
    use super::*;
    #[test]
    fn cancellation_terminates_owned_git_process_group_and_pipe_holders() {
        let control = std::sync::Arc::new(crate::import::ImportControl::default());
        let c = control.clone();
        let cancel = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            c.cancel();
        });
        let start = std::time::Instant::now();
        let result = controlled_output(
            Command::new("/bin/sh").args(["-c", "/bin/sleep 30 & wait"]),
            Some(&control),
        );
        cancel.join().unwrap();
        assert_eq!(result.unwrap_err().code, "CANCELLED");
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
    }
}

const MAX_ARCHIVE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_MEMBER_BYTES: u64 = 32 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ARCHIVE_MEMBERS: usize = 10_000;
const MAX_CENTRAL_BYTES: u64 = 16 * 1024 * 1024;
const MAX_MEMBER_PATH_BYTES: usize = 1024;
const MAX_MEMBER_DEPTH: usize = 64;

struct ArchiveMember {
    index: usize,
    relative: PathBuf,
    is_directory: bool,
    size: u64,
    mode: Option<u32>,
}

fn archive_error(message: impl Into<String>) -> Error {
    Error::new("INVALID_ARCHIVE", message)
}

fn zip_error(error: zip::result::ZipError) -> Error {
    archive_error(error.to_string())
}

fn le16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn le32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

/// Bound metadata allocations and count every central record before the ZIP
/// library places entries in its map (where duplicate names would be hidden).
fn preflight_directory(file: &mut File) -> Result<(usize, u64)> {
    let length = file.metadata()?.len();
    if !(22..=MAX_ARCHIVE_BYTES).contains(&length) {
        return Err(archive_error(
            "ZIP archive must be between 22 bytes and 128 MiB",
        ));
    }
    let tail_length = length.min(65_557) as usize;
    file.seek(SeekFrom::End(-(tail_length as i64)))?;
    let mut tail = vec![0; tail_length];
    file.read_exact(&mut tail)?;
    let end = (0..=tail_length - 22)
        .rev()
        .find(|offset| {
            tail[*offset..].starts_with(b"PK\x05\x06")
                && *offset + 22 + usize::from(le16(&tail, *offset + 20)) == tail_length
        })
        .ok_or_else(|| archive_error("ZIP end-of-directory record is missing"))?;
    let end_offset = length - tail_length as u64 + end as u64;
    let end = &tail[end..];
    let count = usize::from(le16(end, 10));
    let central_size = u64::from(le32(end, 12));
    let central_start = u64::from(le32(end, 16));
    if le16(end, 4) != 0 || le16(end, 6) != 0 || le16(end, 8) != le16(end, 10) {
        return Err(archive_error("Multi-disk ZIP archives are unsupported"));
    }
    if count == usize::from(u16::MAX)
        || central_size == u64::from(u32::MAX)
        || central_start == u64::from(u32::MAX)
    {
        return Err(archive_error("ZIP64 archives are unsupported"));
    }
    if count > MAX_ARCHIVE_MEMBERS || central_size > MAX_CENTRAL_BYTES {
        return Err(archive_error(
            "ZIP archive exceeds the entry or metadata limit",
        ));
    }
    if central_start.checked_add(central_size) != Some(end_offset) {
        return Err(archive_error(
            "ZIP central-directory bounds are inconsistent; ZIP64 and appended records are unsupported",
        ));
    }
    let mut cursor = central_start;
    for _ in 0..count {
        if cursor.checked_add(46).is_none_or(|end| end > end_offset) {
            return Err(archive_error("Truncated ZIP central-directory header"));
        }
        file.seek(SeekFrom::Start(cursor))?;
        let mut header = [0; 46];
        file.read_exact(&mut header)?;
        if !header.starts_with(b"PK\x01\x02") {
            return Err(archive_error("Invalid ZIP central-directory header"));
        }
        if le16(&header, 8) & 0x2041 != 0 {
            return Err(archive_error("Encrypted ZIP members are unsupported"));
        }
        if !matches!(le16(&header, 10), 0 | 8) {
            return Err(archive_error(
                "Only stored and deflated ZIP members are supported",
            ));
        }
        // Honor link/special-file type bits even for an unusual creator OS,
        // rather than relying only on the ZIP reader's Unix-mode mapping.
        let kind = (le32(&header, 38) >> 16) & 0o170000;
        if !matches!(kind, 0 | 0o040000 | 0o100000) {
            return Err(archive_error(
                "ZIP symlinks and special files are forbidden",
            ));
        }
        if le32(&header, 20) == u32::MAX
            || le32(&header, 24) == u32::MAX
            || le32(&header, 42) == u32::MAX
            || le16(&header, 34) != 0
        {
            return Err(archive_error(
                "ZIP64 and multi-disk members are unsupported",
            ));
        }
        let variable_size = u64::from(le16(&header, 28))
            + u64::from(le16(&header, 30))
            + u64::from(le16(&header, 32));
        cursor = cursor
            .checked_add(46 + variable_size)
            .filter(|cursor| *cursor <= end_offset)
            .ok_or_else(|| archive_error("ZIP member metadata escapes the central directory"))?;
    }
    if cursor != end_offset {
        return Err(archive_error(
            "ZIP entry count does not cover its central directory",
        ));
    }
    Ok((count, central_start))
}

fn member_path(name: &str, is_directory: bool) -> Result<PathBuf> {
    if name.is_empty()
        || name.len() > MAX_MEMBER_PATH_BYTES
        || name.starts_with('/')
        || name.contains(['\\', ':'])
        || name.chars().any(char::is_control)
    {
        return Err(archive_error(format!("Unsafe ZIP member path: {name:?}")));
    }
    let name = if is_directory {
        name.strip_suffix('/')
            .ok_or_else(|| archive_error("ZIP directory name must end in /"))?
    } else {
        name
    };
    let parts = name.split('/').collect::<Vec<_>>();
    if parts.len() > MAX_MEMBER_DEPTH
        || parts.iter().any(|part| {
            part.is_empty()
                || *part == "."
                || *part == ".."
                || part.ends_with(['.', ' '])
                || windows_reserved_name(part)
        })
    {
        return Err(archive_error(format!("Unsafe ZIP member path: {name:?}")));
    }
    Ok(parts.iter().collect())
}

fn windows_reserved_name(part: &str) -> bool {
    let base = part.split('.').next().unwrap_or(part).to_ascii_uppercase();
    matches!(
        base.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" | "CLOCK$"
    ) || base
        .strip_prefix("COM")
        .or_else(|| base.strip_prefix("LPT"))
        .is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
}

struct ArchiveNode {
    spelling: String,
    is_directory: bool,
    explicit: bool,
}

fn record_member(nodes: &mut HashMap<String, ArchiveNode>, member: &ArchiveMember) -> Result<()> {
    let mut spelling = String::new();
    let parts = member.relative.iter().collect::<Vec<_>>();
    for (index, part) in parts.iter().enumerate() {
        if !spelling.is_empty() {
            spelling.push('/');
        }
        spelling.push_str(&part.to_string_lossy());
        let explicit = index + 1 == parts.len();
        let is_directory = !explicit || member.is_directory;
        let key = spelling.to_lowercase();
        if let Some(previous) = nodes.get_mut(&key) {
            if previous.spelling != spelling
                || !previous.is_directory
                || !is_directory
                || (explicit && previous.explicit)
            {
                return Err(archive_error(format!(
                    "Duplicate or colliding ZIP path: {spelling}"
                )));
            }
            previous.explicit |= explicit;
        } else {
            nodes.insert(
                key,
                ArchiveNode {
                    spelling: spelling.clone(),
                    is_directory,
                    explicit,
                },
            );
        }
    }
    Ok(())
}

fn open_archive(path: &Path) -> Result<(zip::ZipArchive<File>, Vec<ArchiveMember>)> {
    let before = crate::hash::checked_metadata(path)?;
    if !before.is_file() {
        return Err(archive_error("ZIP source must be a regular file"));
    }
    let mut file = File::open(path)?;
    if !same_directory(&before, &file.metadata()?) {
        return Err(archive_error("ZIP source changed while opening it"));
    }
    let (count, central_start) = preflight_directory(&mut file)?;
    let mut archive = zip::ZipArchive::new(file).map_err(zip_error)?;
    if archive.len() != count || archive.central_directory_start() != central_start {
        return Err(archive_error(
            "ZIP archive contains duplicate names or inconsistent central metadata",
        ));
    }
    let mut members = Vec::with_capacity(count);
    let mut nodes = HashMap::new();
    let mut total_size = 0u64;
    let mut total_compressed = 0u64;
    for index in 0..count {
        let file = archive.by_index_raw(index).map_err(zip_error)?;
        if file.encrypted() {
            return Err(archive_error("Encrypted ZIP members are unsupported"));
        }
        if !matches!(
            file.compression(),
            zip::CompressionMethod::Stored | zip::CompressionMethod::Deflated
        ) {
            return Err(archive_error(
                "Only stored and deflated ZIP members are supported",
            ));
        }
        let name = std::str::from_utf8(file.name_raw())
            .map_err(|_| archive_error("ZIP member names must be valid UTF-8"))?;
        if name != file.name() {
            return Err(archive_error(
                "ZIP member has inconsistent or unsupported filename encodings",
            ));
        }
        let is_directory = file.is_dir();
        let relative = member_path(name, is_directory)?;
        let mode = file.unix_mode();
        let kind = mode.unwrap_or(0) & 0o170000;
        if (kind != 0 && kind != if is_directory { 0o040000 } else { 0o100000 })
            || (is_directory && file.size() != 0)
        {
            return Err(archive_error(format!(
                "ZIP symlinks and special files are forbidden: {name}"
            )));
        }
        if file.size() > MAX_MEMBER_BYTES || file.compressed_size() > MAX_ARCHIVE_BYTES {
            return Err(archive_error(
                "ZIP member exceeds the 32 MiB uncompressed limit",
            ));
        }
        total_size = total_size
            .checked_add(file.size())
            .filter(|size| *size <= MAX_EXTRACTED_BYTES)
            .ok_or_else(|| archive_error("ZIP archive exceeds the 256 MiB uncompressed limit"))?;
        total_compressed = total_compressed
            .checked_add(file.compressed_size())
            .filter(|size| *size <= MAX_ARCHIVE_BYTES)
            .ok_or_else(|| {
                archive_error("ZIP archive exceeds the 128 MiB compressed-work limit")
            })?;
        if file
            .data_start()
            .checked_add(file.compressed_size())
            .is_none_or(|end| end > central_start)
        {
            return Err(archive_error("ZIP member data escapes the archive payload"));
        }
        let member = ArchiveMember {
            index,
            relative,
            is_directory,
            size: file.size(),
            mode,
        };
        record_member(&mut nodes, &member)?;
        members.push(member);
    }
    // Run only after every range has passed checked addition and archive bounds,
    // so malformed large sizes cannot overflow the ZIP reader's range check.
    if archive.has_overlapping_files().map_err(zip_error)? {
        return Err(archive_error(
            "ZIP members overlap or reuse compressed payload data",
        ));
    }
    Ok((archive, members))
}

fn archive_directory(
    root: &Path,
    relative: &Path,
    directories: &mut HashMap<PathBuf, Metadata>,
) -> Result<()> {
    let mut logical = PathBuf::new();
    let expected = directories
        .get(&logical)
        .ok_or_else(|| archive_error("ZIP extraction root has no ownership identity"))?;
    check_archive_directory(root, expected)?;
    for component in relative.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err(archive_error(
                "ZIP directory has a non-local path component",
            ));
        }
        logical.push(component.as_os_str());
        let absolute = root.join(&logical);
        if let Some(expected) = directories.get(&logical) {
            check_archive_directory(&absolute, expected)?;
            continue;
        }
        // Do not reuse a directory discovered only through the filesystem. On
        // APFS, distinct Unicode spellings can name the same existing directory.
        // Only exact logical paths previously created by this extraction may be
        // reused, with their identity checked above.
        crate::index::validate_path(&absolute)?;
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&absolute) {
            Ok(()) => {
                let identity = crate::hash::checked_metadata(&absolute)?;
                if !identity.is_dir() {
                    return Err(Error::new(
                        "ZIP_PATH_COLLISION",
                        "ZIP directory changed while creating it",
                    ));
                }
                directories.insert(logical.clone(), identity);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(Error::new(
                    "ZIP_PATH_COLLISION",
                    format!(
                        "ZIP directory aliases an existing path: {}",
                        absolute.display()
                    ),
                ));
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn check_archive_directory(path: &Path, expected: &Metadata) -> Result<()> {
    crate::hash::checked_absolute_path(path)?;
    let current = crate::hash::checked_metadata(path)?;
    if !current.is_dir() || !same_directory(expected, &current) {
        return Err(Error::new(
            "ZIP_PATH_COLLISION",
            format!("ZIP directory ownership changed: {}", path.display()),
        ));
    }
    Ok(())
}

fn archive_source(path: &Path, state_dir: &Path) -> Result<PreparedSource> {
    // Complete member preflight before creating any temporary directory.
    let (mut archive, members) = open_archive(path)?;
    let scratch = Scratch::new(state_dir)?;
    let root = scratch.root.join("contents");
    fs::create_dir(&root)?;
    let mut directories = HashMap::new();
    directories.insert(PathBuf::new(), crate::hash::checked_metadata(&root)?);
    for member in members {
        let destination = root.join(&member.relative);
        crate::index::validate_path(&destination)?;
        if member.is_directory {
            archive_directory(&root, &member.relative, &mut directories)?;
            continue;
        }
        let parent = member
            .relative
            .parent()
            .ok_or_else(|| archive_error("ZIP member has no parent directory"))?;
        archive_directory(&root, parent, &mut directories)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut output = options.open(&destination).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                Error::new(
                    "ZIP_PATH_COLLISION",
                    format!(
                        "ZIP file aliases an existing path: {}",
                        destination.display()
                    ),
                )
            } else {
                error.into()
            }
        })?;
        let file = archive.by_index(member.index).map_err(zip_error)?;
        let mut bounded = file.take(member.size + 1);
        let copied = std::io::copy(&mut bounded, &mut output)
            .map_err(|error| archive_error(format!("Could not read ZIP payload: {error}")))?;
        if copied != member.size {
            return Err(archive_error(
                "ZIP member's actual size differs from its declared size",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Preserve executable intent for auditing, while retaining owner
            // access and stripping setuid/setgid and group/other write access.
            let permissions = 0o600 | (member.mode.unwrap_or(0) & 0o111);
            output.set_permissions(fs::Permissions::from_mode(permissions))?;
        }
    }
    let label = path
        .file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive".into());
    Ok(PreparedSource {
        root,
        provenance: SourceInfo {
            kind: "local".into(),
            found_in: vec![path.to_path_buf()],
            ..SourceInfo::default()
        },
        label,
        _scratch: Some(scratch),
    })
}

#[derive(Debug)]
struct GithubSource {
    url: String,
    label: String,
    branch: Option<String>,
    subpath: Option<String>,
}

impl GithubSource {
    fn parse(input: &str) -> Result<Self> {
        let invalid = || {
            Error::new(
                "INVALID_SOURCE_URL",
                "Use https://github.com/owner/repo or /tree/<single-segment-ref>/<subpath>; credentials, query strings, fragments, encoded paths and traversal are not supported",
            )
        };
        let path = input
            .strip_prefix("https://github.com/")
            .ok_or_else(invalid)?;
        if path
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
            || path.contains(['?', '#', '@', '%', '\\', ':'])
        {
            return Err(invalid());
        }
        let path = path.strip_suffix('/').unwrap_or(path);
        let pieces = path.split('/').collect::<Vec<_>>();
        if pieces.len() < 2
            || pieces
                .iter()
                .any(|piece| piece.is_empty() || *piece == "." || *piece == "..")
        {
            return Err(invalid());
        }
        let owner = pieces[0];
        let repo = pieces[1].strip_suffix(".git").unwrap_or(pieces[1]);
        if !valid_name(owner, false) || !valid_name(repo, true) {
            return Err(invalid());
        }
        let (branch, subpath) = if pieces.len() == 2 {
            (None, None)
        } else {
            if pieces.len() < 4 || pieces[2] != "tree" || !valid_ref(pieces[3]) {
                return Err(invalid());
            }
            for part in &pieces[4..] {
                if !valid_name(part, true) || part.eq_ignore_ascii_case(".git") {
                    return Err(invalid());
                }
            }
            let subpath = (pieces.len() > 4).then(|| pieces[4..].join("/"));
            (Some(pieces[3].to_string()), subpath)
        };
        Ok(Self {
            url: format!("https://github.com/{owner}/{repo}"),
            label: owner.to_string(),
            branch,
            subpath,
        })
    }
}

fn valid_name(name: &str, allow_dot: bool) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.starts_with('-')
        && name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || byte == b'-'
                || byte == b'_'
                || (allow_dot && byte == b'.')
        })
}

fn valid_ref(reference: &str) -> bool {
    valid_name(reference, true)
        && !reference.starts_with('.')
        && !reference.ends_with('.')
        && !reference.ends_with(".lock")
        && !reference.contains("..")
}

fn git_executable(source_root: Option<&Path>) -> Result<PathBuf> {
    let excluded: Vec<PathBuf> = source_root.map(Path::to_path_buf).into_iter().collect();
    crate::executables::git(&excluded).map_err(|error| {
        if error.code == "GIT_TOOLS_MISSING" {
            error
        } else {
            Error::new(
                "SOURCE_GIT_ERROR",
                format!(
                    "Git is not available outside the import source. {}",
                    error.message
                ),
            )
        }
    })
}

fn git_command(executable: &Path, directory: &Path) -> Command {
    let mut command = Command::new(executable);
    // Clear every injected Git scope/config/transport setting, including numbered
    // GIT_CONFIG_KEY_n / VALUE_n and GIT_CONFIG_PARAMETERS variables.
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    // Git's transport helpers also inherit PATH; never search the checkout via
    // `.` or another relative entry, even when launching an absolute git binary.
    let directories = crate::executables::path_directories()
        .into_iter()
        .filter(|path| {
            path.canonicalize()
                .is_ok_and(|path| !path.starts_with(directory))
        });
    match std::env::join_paths(directories) {
        Ok(path) => {
            command.env("PATH", path);
        }
        Err(_) => {
            command.env("PATH", "/usr/bin:/bin");
        }
    }
    crate::executables::set_child_locale(&mut command);
    command
        .current_dir(directory)
        .stdin(Stdio::null())
        .env_remove("SSH_ASKPASS")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "/bin/false")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_CEILING_DIRECTORIES", directory)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.attributesFile=/dev/null",
            "-c",
            "credential.helper=",
            "-c",
            "protocol.allow=never",
            "-c",
            "protocol.https.allow=always",
            "-c",
            "protocol.ext.allow=never",
            "-c",
            "protocol.file.allow=never",
            "-c",
            "http.followRedirects=false",
            "-c",
            "filter.lfs.smudge=",
            "-c",
            "filter.lfs.process=",
            "-c",
            "filter.lfs.required=false",
        ]);
    command
}

fn successful(output: Output, operation: &str) -> Result<Output> {
    if output.status.success() {
        return Ok(output);
    }
    Err(Error::new(
        "SOURCE_GIT_ERROR",
        format!(
            "{operation}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    ))
}

fn github(
    source: GithubSource,
    state_dir: &Path,
    control: Option<&crate::import::ImportControl>,
) -> Result<PreparedSource> {
    if source.branch.as_deref().is_some_and(valid_commit) {
        let mut prepared = checkout_github_ref(
            &source.url,
            source.branch.as_deref().unwrap(),
            "commit",
            None,
            control,
        )?;
        if let Some(subpath) = &source.subpath {
            prepared.root = crate::hash::checked_absolute_path(&prepared.root.join(subpath))?;
        }
        prepared.provenance.subpath = source.subpath;
        return Ok(prepared);
    }
    let executable = git_executable(None)?;
    let scratch = Scratch::new(state_dir)?;
    let checkout = scratch.root.join("checkout");
    let mut clone = git_command(&executable, &scratch.root);
    clone.args([
        "clone",
        "--quiet",
        "--depth",
        "1",
        "--no-checkout",
        "--no-recurse-submodules",
        "--template=",
    ]);
    if let Some(branch) = &source.branch {
        clone.args(["--branch", branch]);
    }
    successful(
        controlled_output(
            clone
                .arg("--")
                .arg(format!("{}.git", source.url))
                .arg(&checkout),
            control,
        )?,
        "Could not clone GitHub source (refs containing slashes are unsupported)",
    )?;
    crate::hash::checked_absolute_path(&checkout)?;
    let output = successful(
        controlled_output(
            git_command(&executable, &checkout).args(["rev-parse", "--verify", "HEAD"]),
            control,
        )?,
        "Could not read source commit",
    )?;
    let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !valid_commit(&commit) {
        return Err(Error::new(
            "SOURCE_GIT_ERROR",
            "Git returned an invalid source commit",
        ));
    }
    let tracking_kind = if source.branch.is_none() {
        "default"
    } else if git_command(&executable, &checkout)
        .args(["symbolic-ref", "-q", "HEAD"])
        .output()?
        .status
        .success()
    {
        "branch"
    } else {
        "tag"
    };
    // clone does not copy the source repository's config. With an empty template,
    // no system/global configuration and no injected GIT_* settings, source
    // attributes cannot name a configured executable filter. Hooks are disabled.
    successful(
        controlled_output(
            git_command(&executable, &checkout)
                .args(["checkout", "--quiet", "--force", "--detach", &commit, "--"]),
            control,
        )?,
        "Could not prepare source files",
    )?;
    let root = match &source.subpath {
        Some(subpath) => checkout.join(subpath),
        None => checkout,
    };
    let root = crate::hash::checked_absolute_path(&root)?;
    if !fs::symlink_metadata(&root)?.is_dir() {
        return Err(Error::new(
            "INVALID_SOURCE",
            "GitHub source subpath must be a directory",
        ));
    }
    Ok(PreparedSource {
        root,
        provenance: SourceInfo {
            kind: "github".into(),
            tracking_ref: Some(source.branch.clone().unwrap_or_else(|| "HEAD".into())),
            tracking_kind: Some(tracking_kind.into()),
            url: Some(source.url),
            subpath: source.subpath,
            commit: Some(commit),
            found_in: vec![],
        },
        label: source.label,
        _scratch: Some(scratch),
    })
}

/// One network fetch for one repository/ref group. Fetch a pinned preview commit
/// on confirmation; never resolve a moving branch instead of the reviewed tree.
pub(crate) fn canonical_github_repository(url: &str) -> Result<String> {
    let source = GithubSource::parse(url)?;
    if source.branch.is_some() || source.subpath.is_some() {
        return Err(Error::new(
            "INVALID_SOURCE_URL",
            "Expected a canonical GitHub repository URL",
        ));
    }
    Ok(source.url.to_ascii_lowercase())
}

pub(crate) fn checkout_github_ref(
    url: &str,
    reference: &str,
    kind: &str,
    pinned: Option<&str>,
    control: Option<&crate::import::ImportControl>,
) -> Result<PreparedSource> {
    let source = GithubSource::parse(url)?;
    if source.branch.is_some() || source.subpath.is_some() {
        return Err(Error::new(
            "INVALID_SOURCE_URL",
            "Expected a canonical GitHub repository URL",
        ));
    }
    let valid = !reference.is_empty()
        && !reference.starts_with(['-', '/', '.'])
        && !reference.ends_with(['/', '.'])
        && !reference.contains("..")
        && !reference.ends_with(".lock")
        && !reference.contains("@{")
        && reference
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_./".contains(&c));
    if !valid
        || !matches!(kind, "default" | "branch" | "tag" | "commit")
        || (kind == "commit" && !valid_commit(reference))
        || pinned.is_some_and(|commit| !valid_commit(commit))
    {
        return Err(Error::new(
            "INVALID_SOURCE_REF",
            "Invalid source tracking reference or commit",
        ));
    }
    let executable = git_executable(None)?;
    let scratch = Scratch::new(&std::env::temp_dir().canonicalize()?)?;
    let checkout = scratch.root.join("checkout");
    successful(
        controlled_output(
            git_command(&executable, &scratch.root)
                .args(["init", "--quiet", "--template="])
                .arg(&checkout),
            control,
        )?,
        "Could not initialize source scratch",
    )?;
    let requested = pinned.map(str::to_string).unwrap_or_else(|| match kind {
        "default" => "HEAD".into(),
        "branch" => format!("refs/heads/{reference}"),
        "tag" => format!("refs/tags/{reference}"),
        _ => reference.into(),
    });
    successful(
        controlled_output(
            git_command(&executable, &checkout)
                .args([
                    "fetch",
                    "--quiet",
                    "--depth=1",
                    "--no-tags",
                    "--no-recurse-submodules",
                    "--",
                ])
                .arg(format!("{}.git", source.url))
                .arg(&requested),
            control,
        )?,
        "Could not fetch source reference",
    )?;
    let output = successful(
        controlled_output(
            git_command(&executable, &checkout).args([
                "rev-parse",
                "--verify",
                "FETCH_HEAD^{commit}",
            ]),
            control,
        )?,
        "Could not resolve source commit",
    )?;
    let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !valid_commit(&commit) || pinned.is_some_and(|pinned| !pinned.eq_ignore_ascii_case(&commit))
    {
        return Err(Error::new(
            "SOURCE_COMMIT_MISMATCH",
            "Fetched commit differs from the pinned preview",
        ));
    }
    successful(
        controlled_output(
            git_command(&executable, &checkout)
                .args(["checkout", "--quiet", "--force", "--detach", &commit, "--"]),
            control,
        )?,
        "Could not materialize source commit",
    )?;
    Ok(PreparedSource {
        root: checkout,
        label: source.label,
        provenance: SourceInfo {
            kind: "github".into(),
            url: Some(source.url),
            commit: Some(commit),
            tracking_ref: Some(reference.into()),
            tracking_kind: Some(kind.into()),
            ..Default::default()
        },
        _scratch: Some(scratch),
    })
}

fn valid_commit(commit: &str) -> bool {
    matches!(commit.len(), 40 | 64) && commit.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn local_git_hint(root: &Path) -> SourceInfo {
    let mut source = SourceInfo::default();
    for ancestor in root.ancestors() {
        let git = ancestor.join(".git");
        let Ok(metadata) = fs::symlink_metadata(&git) else {
            continue;
        };
        // Worktree indirections and links are not followed just for a hint.
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return source;
        }
        // A commondir file redirects reference/config reads to another location.
        if fs::symlink_metadata(git.join("commondir")).is_ok() {
            return source;
        }
        let Ok(executable) = git_executable(Some(ancestor)) else {
            return source;
        };
        for name in ["config", "HEAD", "refs", "packed-refs"] {
            let path = git.join(name);
            if fs::symlink_metadata(&path).is_err() {
                continue;
            }
            for entry in walkdir::WalkDir::new(path).follow_links(false) {
                let Ok(entry) = entry else {
                    return source;
                };
                if entry.file_type().is_symlink()
                    || (!entry.file_type().is_file() && !entry.file_type().is_dir())
                {
                    return source;
                }
            }
        }
        let Some(trusted_directory) = executable.parent() else {
            return source;
        };
        // Treat the config as a data file, without activating the source repo.
        let mut command = git_command(&executable, trusted_directory);
        let Ok(output) = command
            .args(["config", "--file"])
            .arg(git.join("config"))
            .args([
                "--no-includes",
                "--null",
                "--get-regexp",
                r"^remote\..*\.url$",
            ])
            .output()
        else {
            return source;
        };
        if !output.status.success() {
            return source;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let mut remotes = text
            .split('\0')
            .filter_map(|entry| entry.split_once('\n'))
            .filter(|(_, url)| !url.trim().is_empty())
            .collect::<Vec<_>>();
        remotes.sort_by_key(|(name, _)| (*name != "remote.origin.url", *name));
        let Some((_, url)) = remotes.first() else {
            return source;
        };
        source.url = crate::gitlog::public_remote(url);
        source.commit = local_head(&git);
        if let Ok(subpath) = root.strip_prefix(ancestor)
            && !subpath.as_os_str().is_empty()
        {
            source.subpath = Some(subpath.to_string_lossy().replace('\\', "/"));
        }
        return source;
    }
    source
}

fn local_head(git: &Path) -> Option<String> {
    let mut path = git.join("HEAD");
    for _ in 0..8 {
        crate::hash::checked_absolute_path(&path).ok()?;
        if !crate::hash::checked_metadata(&path).ok()?.is_file() {
            return None;
        }
        let value = fs::read_to_string(&path).ok()?;
        let value = value.trim();
        if valid_commit(value) {
            return Some(value.to_string());
        }
        let reference = value.strip_prefix("ref: ")?;
        // Ref names must remain beneath the checked metadata directory on every
        // platform. Symbolic ref loops are bounded by the iteration limit.
        if !reference.starts_with("refs/")
            || reference.contains(['\\', ':'])
            || reference
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return None;
        }
        path = git.join(reference);
        match fs::symlink_metadata(&path) {
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let packed = git.join("packed-refs");
                crate::hash::checked_absolute_path(&packed).ok()?;
                if !crate::hash::checked_metadata(&packed).ok()?.is_file() {
                    return None;
                }
                for line in fs::read_to_string(packed).ok()?.lines() {
                    if let Some((commit, name)) = line.split_once(' ')
                        && name == reference
                        && valid_commit(commit)
                    {
                        return Some(commit.to_string());
                    }
                }
                return None;
            }
            Err(_) => return None,
        }
    }
    None
}
