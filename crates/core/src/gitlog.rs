use crate::{Error, Result, SourceInfo, UndoReport};
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output, Stdio};

// info/attributes has higher priority than imported .gitattributes. In
// particular, disabling a configured filter by name would miss new drivers
// introduced through a skill's own attribute rules.
const ATTRIBUTE_GUARD: &[u8] = b"# Managed by Skills Box: preserve imported files as exact bytes.\n* -text -filter -ident -working-tree-encoding -eol -merge -diff\n";
const MANAGED: [&str; 5] = ["box.toml", ".gitignore", "skills", "meta", "bundles"];
const INVENTORY: [&str; 4] = [
    ".DS_Store",
    "skills/.DS_Store",
    "meta/.DS_Store",
    "bundles/.DS_Store",
];

fn null_device() -> &'static str {
    if cfg!(windows) { "NUL" } else { "/dev/null" }
}

/// The Git for vault work, with its real error code (GIT_TOOLS_MISSING or
/// GIT_ERROR naming the searched locations) when there is none.
fn git_for(root: &Path) -> Result<PathBuf> {
    crate::executables::git(&[root.canonicalize()?])
}

/// Preflight a new file-backed operation against known original skill roots.
/// This checks the exact candidate used by vault Git operations, without
/// mutating PATH or invoking a process from an untrusted source.
pub(crate) fn reject_source_executable(root: &Path, source_roots: &[PathBuf]) -> Result<()> {
    let selected = git_for(root)?;
    for source in source_roots {
        if source
            .canonicalize()
            .is_ok_and(|path| selected.starts_with(path))
        {
            return Err(Error::new(
                "UNSAFE_GIT_EXECUTABLE",
                "Git on PATH belongs to an original skill source; choose Git outside skill sources",
            ));
        }
    }
    Ok(())
}

fn command_with(git: &Path, root: &Path) -> Command {
    // A relative PATH entry such as '.' could otherwise execute an imported
    // file named git merely because the caller's cwd is inside a skill. An
    // absolute PATH entry (or symlink) into the vault is equally untrusted:
    // `git` was looked up with those roots excluded.
    let mut command = Command::new(git);
    // Git accepts configuration, alternate object stores, indexes and worktree
    // paths from many environment variables. None are inherited by vault work.
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_SYSTEM", null_device())
        .env("GIT_CONFIG_GLOBAL", null_device())
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_PAGER", "cat")
        .env("GIT_EDITOR", "true")
        .stdin(Stdio::null());
    crate::executables::set_child_locale(&mut command);
    for setting in [
        format!("core.hooksPath={}", null_device()),
        format!("core.attributesFile={}", null_device()),
        format!("core.excludesFile={}", null_device()),
        "core.fsmonitor=false".into(),
        "core.autocrlf=false".into(),
        "core.safecrlf=false".into(),
        "core.ignoreStat=false".into(),
        "core.sparseCheckout=false".into(),
        "core.splitIndex=false".into(),
        "core.pager=cat".into(),
        "commit.gpgSign=false".into(),
        "tag.gpgSign=false".into(),
        "gc.auto=0".into(),
        "maintenance.auto=false".into(),
        "submodule.recurse=false".into(),
        "rerere.enabled=false".into(),
        "rerere.autoupdate=false".into(),
    ] {
        command.args(["-c", &setting]);
    }
    command.arg("-C").arg(root);
    command
}

fn checked(output: Output) -> Result<Output> {
    if output.status.success() {
        return Ok(output);
    }
    Err(Error::new(
        "GIT_ERROR",
        String::from_utf8_lossy(&output.stderr).trim().to_string(),
    ))
}

fn unsafe_repository(message: impl Into<String>) -> Error {
    Error::decision("GIT_UNSAFE_REPOSITORY", message)
}

fn dirty() -> Error {
    Error::decision(
        "GIT_DIRTY_VAULT",
        "Vault has staged, modified, or untracked files; commit or move them before continuing",
    )
}

fn is_inventory(path: &str) -> bool {
    INVENTORY.contains(&path)
}

fn is_managed(path: &str) -> bool {
    !is_inventory(path)
        && (matches!(path, "box.toml" | ".gitignore")
            || ["skills/", "meta/", "bundles/"]
                .iter()
                .any(|prefix| path.starts_with(prefix)))
}

/// Untracked files that leave the vault clean: generated `dist/`, and
/// Finder's `.DS_Store` by name at any depth (owner decision 2026-11-17).
/// Only the name counts; no other hidden file is treated this way. Every
/// operation commits exact paths, so such a file is never staged; a
/// `.DS_Store` that arrived with an imported skill is tracked payload and
/// stays under the usual rules.
fn untracked_allowed(path: &str) -> bool {
    path.starts_with("dist/") || is_inventory(path) || path.rsplit('/').next() == Some(".DS_Store")
}

fn nul_paths(bytes: &[u8]) -> Result<Vec<&str>> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            std::str::from_utf8(path)
                .map_err(|_| unsafe_repository("Vault Git paths must be valid Unicode"))
        })
        .collect()
}

fn check_git_metadata(git: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(git)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(unsafe_repository(
            "Vault .git must be a local directory, not a symlink or worktree indirection",
        ));
    }
    for entry in walkdir::WalkDir::new(git).follow_links(false) {
        let entry = entry.map_err(|error| unsafe_repository(error.to_string()))?;
        if !entry.file_type().is_dir() && !entry.file_type().is_file() {
            return Err(unsafe_repository(format!(
                "Refusing non-regular Git metadata: {}",
                entry.path().display()
            )));
        }
    }
    Ok(())
}

struct TrackedFile {
    path: String,
    oid: String,
    executable: bool,
}

struct Repository {
    root: PathBuf,
    /// Looked up once per operation (owner decision, 2026-11-18).
    git: PathBuf,
    name: String,
    email: String,
}

impl Repository {
    fn open(root: &Path) -> Result<Self> {
        let git_executable = git_for(root)?;
        let root = crate::index::validate_path(root)?;
        if !fs::symlink_metadata(&root)?.is_dir() {
            return Err(unsafe_repository("Vault must be a directory"));
        }
        let git = root.join(".git");
        check_git_metadata(&git)?;
        for name in [
            "commondir",
            "config.worktree",
            "objects/info/alternates",
            "objects/info/http-alternates",
        ] {
            if git.join(name).exists() {
                return Err(unsafe_repository(format!(
                    "Vault Git metadata cannot redirect to external configuration or objects: {name}"
                )));
            }
        }
        let attributes = git.join("info/attributes");
        if attributes.exists() && fs::read(&attributes)? != ATTRIBUTE_GUARD {
            return Err(unsafe_repository(
                "Vault .git/info/attributes contains rules not owned by Kinana",
            ));
        }
        let config = checked(
            command_with(&git_executable, &root)
                .arg("config")
                .arg("--file")
                .arg(git.join("config"))
                .args(["--no-includes", "--null", "--list"])
                .output()?,
        )?;
        let mut name = "Kinana".to_string();
        let mut email = "kinana@localhost".to_string();
        for entry in config.stdout.split(|byte| *byte == 0) {
            let entry = std::str::from_utf8(entry)
                .map_err(|_| unsafe_repository("Vault Git configuration must be valid Unicode"))?;
            let (key, value) = entry.split_once('\n').unwrap_or((entry, ""));
            let key = key.to_ascii_lowercase();
            if key == "include.path"
                || key.starts_with("includeif.")
                || key == "core.worktree"
                || key == "extensions.worktreeconfig"
                || (key == "core.bare" && !matches!(value, "false" | "no" | "off" | "0"))
            {
                return Err(unsafe_repository(format!(
                    "Unsupported vault Git configuration: {key}"
                )));
            }
            if key == "user.name" && !value.trim().is_empty() {
                name = value.to_string();
            }
            if key == "user.email" && !value.trim().is_empty() {
                email = value.to_string();
            }
        }
        for name in [
            "MERGE_HEAD",
            "CHERRY_PICK_HEAD",
            "REVERT_HEAD",
            "rebase-apply",
            "rebase-merge",
            "sequencer",
            "index.lock",
            "HEAD.lock",
        ] {
            if git.join(name).exists() {
                return Err(Error::decision(
                    "GIT_BUSY",
                    format!("Finish the existing Git operation before continuing: {name}"),
                ));
            }
        }
        let repository = Self {
            root,
            git: git_executable,
            name,
            email,
        };
        checked(
            repository
                .command()
                .args(["rev-parse", "--is-inside-work-tree"])
                .output()?,
        )?;
        Ok(repository)
    }

    fn command(&self) -> Command {
        let mut command = command_with(&self.git, &self.root);
        // Pin both paths; a vault operation must not discover an enclosing repo.
        command
            .arg("--git-dir")
            .arg(self.root.join(".git"))
            .arg("--work-tree")
            .arg(&self.root);
        command
    }

    fn identity_command(&self) -> Command {
        let mut command = self.command();
        command
            .arg("-c")
            .arg(format!("user.name={}", self.name))
            .arg("-c")
            .arg(format!("user.email={}", self.email));
        command
    }

    fn head(&self) -> Result<String> {
        let output = checked(
            self.command()
                .args(["rev-parse", "--verify", "HEAD^{commit}"])
                .output()?,
        )?;
        let head = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !matches!(head.len(), 40 | 64) || !head.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(unsafe_repository("Vault HEAD is not a commit object"));
        }
        Ok(head)
    }

    fn install_attribute_guard(&self) -> Result<()> {
        let info = self.root.join(".git/info");
        crate::index::validate_path(&info)?;
        if !info.exists() {
            fs::create_dir(&info)?;
        }
        let path = info.join("attributes");
        crate::index::validate_path(&path)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() && fs::read(&path)? == ATTRIBUTE_GUARD => Ok(()),
            Ok(_) => Err(unsafe_repository(
                "Vault .git/info/attributes contains rules not owned by Kinana",
            )),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)?;
                if let Err(error) = file.write_all(ATTRIBUTE_GUARD) {
                    let _ = fs::remove_file(&path);
                    return Err(error.into());
                }
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }

    fn staged_paths(&self) -> Result<Vec<String>> {
        let output = checked(
            self.command()
                .args([
                    "diff",
                    "--cached",
                    "--name-only",
                    "--no-renames",
                    "--no-ext-diff",
                    "--no-textconv",
                    "-z",
                    "--",
                ])
                .output()?,
        )?;
        Ok(nul_paths(&output.stdout)?
            .into_iter()
            .map(str::to_string)
            .collect())
    }

    fn tracked(&self) -> Result<Vec<TrackedFile>> {
        let flags = checked(self.command().args(["ls-files", "-v", "-z"]).output()?)?;
        for entry in flags.stdout.split(|byte| *byte == 0) {
            if entry
                .first()
                .is_some_and(|flag| flag.is_ascii_lowercase() || *flag == b'S')
            {
                return Err(unsafe_repository(
                    "Vault index cannot contain assume-unchanged or skip-worktree entries",
                ));
            }
        }
        let output = checked(
            self.command()
                .args(["ls-files", "--stage", "-z"])
                .output()?,
        )?;
        let mut files = Vec::new();
        for entry in nul_paths(&output.stdout)? {
            let (fields, path) = entry
                .split_once('\t')
                .ok_or_else(|| unsafe_repository("Invalid vault index entry"))?;
            let fields: Vec<_> = fields.split(' ').collect();
            if fields.len() != 3 || fields[2] != "0" {
                return Err(dirty());
            }
            if !matches!(fields[0], "100644" | "100755") {
                return Err(unsafe_repository(
                    "Vault index cannot contain symlinks or submodules",
                ));
            }
            files.push(TrackedFile {
                path: path.to_string(),
                oid: fields[1].to_string(),
                executable: fields[0] == "100755",
            });
        }
        Ok(files)
    }

    fn files_unchanged(&self, files: &[&TrackedFile]) -> Result<()> {
        for files in files.chunks(64) {
            for file in files {
                let path = crate::index::validate_path(&self.root.join(&file.path))?;
                let metadata = match fs::symlink_metadata(path) {
                    Ok(metadata) => metadata,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        return Err(dirty());
                    }
                    Err(error) => return Err(error.into()),
                };
                if !metadata.is_file() {
                    return Err(dirty());
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if (metadata.permissions().mode() & 0o111 != 0) != file.executable {
                        return Err(dirty());
                    }
                }
            }
            // --no-filters is essential here: even dry-run must never execute
            // a clean filter, and cleanliness means actual bytes, not transformed data.
            let output = checked(
                self.command()
                    .args(["hash-object", "--no-filters", "--"])
                    .args(files.iter().map(|file| &file.path))
                    .output()?,
            )?;
            let hashes: Vec<_> = std::str::from_utf8(&output.stdout)
                .map_err(|_| unsafe_repository("Git returned invalid object hashes"))?
                .lines()
                .collect();
            if hashes.len() != files.len()
                || hashes
                    .iter()
                    .zip(files.iter())
                    .any(|(hash, file)| *hash != file.oid)
            {
                return Err(dirty());
            }
        }
        Ok(())
    }

    fn untracked(&self) -> Result<Vec<String>> {
        // Inspect ignored files too. Only dist and the documented Finder
        // inventory files may be untracked in a clean vault.
        let output = checked(
            self.command()
                .args(["ls-files", "--others", "-z"])
                .output()?,
        )?;
        Ok(nul_paths(&output.stdout)?
            .into_iter()
            .map(str::to_string)
            .collect())
    }

    fn ensure_clean(&self) -> Result<()> {
        self.head()?;
        if !self.staged_paths()?.is_empty() {
            return Err(dirty());
        }
        let files = self.tracked()?;
        self.files_unchanged(&files.iter().collect::<Vec<_>>())?;
        if self
            .untracked()?
            .iter()
            .any(|path| !untracked_allowed(path))
        {
            return Err(dirty());
        }
        Ok(())
    }

    fn ensure_no_foreign_changes(&self) -> Result<Vec<TrackedFile>> {
        if self.staged_paths()?.iter().any(|path| !is_managed(path)) {
            return Err(dirty());
        }
        let files = self.tracked()?;
        self.files_unchanged(
            &files
                .iter()
                .filter(|file| !is_managed(&file.path))
                .collect::<Vec<_>>(),
        )?;
        if self
            .untracked()?
            .iter()
            .any(|path| !is_managed(path) && !untracked_allowed(path))
        {
            return Err(dirty());
        }
        Ok(files)
    }

    fn managed_pathspecs(&self, files: &[TrackedFile]) -> Vec<String> {
        let mut paths: Vec<_> = MANAGED
            .iter()
            .filter(|path| {
                self.root.join(path).exists()
                    || files.iter().any(|file| {
                        file.path == **path || file.path.starts_with(&format!("{path}/"))
                    })
            })
            .map(|path| (*path).to_string())
            .collect();
        for path in &INVENTORY[1..] {
            paths.push(format!(":(top,exclude,literal){path}"));
        }
        paths
    }

    fn exact_file_paths(&self, paths: &[PathBuf]) -> Result<std::collections::BTreeSet<String>> {
        let invalid = || {
            Error::new(
                "INVALID_GIT_PATH",
                "Expected an exact normalized managed relative file path",
            )
        };
        let mut normalized = std::collections::BTreeSet::new();
        for path in paths {
            let text = path.to_str().ok_or_else(invalid)?;
            if text.is_empty()
                || text.contains('\0')
                || path
                    .components()
                    .any(|part| !matches!(part, Component::Normal(_)))
                || text
                    .split(|ch| ch == '/' || (cfg!(windows) && ch == '\\'))
                    .any(|part| matches!(part, "" | "." | ".."))
            {
                return Err(invalid());
            }
            let relative = path
                .components()
                .map(|part| part.as_os_str().to_str().ok_or_else(invalid))
                .collect::<Result<Vec<_>>>()?
                .join("/");
            if !is_managed(&relative) {
                return Err(invalid());
            }
            let absolute = crate::index::validate_path(&self.root.join(path))?;
            match fs::symlink_metadata(&absolute) {
                Ok(metadata) if metadata.is_file() => {}
                // A source update may turn a tracked file into a directory.
                // Keep its old exact path in the staging allowlist so Git may
                // remove that index entry while adding the listed new files.
                Ok(metadata)
                    if metadata.is_dir()
                        && self.tracked()?.iter().any(|file| file.path == relative) => {}
                Ok(_) => return Err(invalid()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            normalized.insert(relative);
        }
        // A missing directory could still expand to every tracked descendant.
        // Check both index and HEAD because a rollback may have removed files
        // from either the worktree or the index already.
        for args in [
            vec!["ls-files", "-z"],
            vec!["ls-tree", "-r", "--name-only", "--full-tree", "-z", "HEAD"],
        ] {
            let output = checked(self.command().args(args).output()?)?;
            for known in nul_paths(&output.stdout)? {
                if known
                    .match_indices('/')
                    .any(|(index, _)| normalized.contains(&known[..index]))
                {
                    return Err(invalid());
                }
            }
        }
        Ok(normalized)
    }

    fn prune_removed_skills(&self, changed: &[&str]) -> Result<()> {
        let remaining = self.tracked()?;
        let ids: std::collections::BTreeSet<_> = changed
            .iter()
            .filter_map(|path| path.strip_prefix("skills/"))
            .filter_map(|path| path.split_once('/').map(|(id, _)| id))
            .collect();
        for id in ids {
            let prefix = format!("skills/{id}/");
            if remaining.iter().any(|file| file.path.starts_with(&prefix)) {
                continue;
            }
            let root = crate::index::validate_path(&self.root.join("skills").join(id))?;
            if !root.exists() {
                continue;
            }
            // Imported empty directories are part of the source hash but not
            // Git's tree. Remove only empty directories in a skill whose last
            // tracked file was removed by this revert. Never remove a file,
            // follow a symlink, or prune an unrelated skill directory.
            for entry in walkdir::WalkDir::new(&root)
                .follow_links(false)
                .contents_first(true)
            {
                let entry = entry.map_err(|error| Error::new("IO_ERROR", error.to_string()))?;
                if !entry.file_type().is_dir() {
                    continue;
                }
                match fs::remove_dir(entry.path()) {
                    Ok(()) => {}
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::DirectoryNotEmpty | std::io::ErrorKind::NotFound
                        ) => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        Ok(())
    }
}

/// Validate the local repository without running hooks, filters, or changing files.
pub fn verify_repository(root: &Path) -> Result<()> {
    Repository::open(root).map(|_| ())
}

pub fn head(root: &Path) -> Result<String> {
    Repository::open(root)?.head()
}

/// Require no staged changes, tracked byte changes, or unexpected untracked files.
pub fn ensure_clean(root: &Path) -> Result<()> {
    Repository::open(root)?.ensure_clean()
}

pub fn init(root: &Path) -> Result<()> {
    let git = git_for(root)?;
    let root = crate::index::validate_path(root)?;
    match fs::symlink_metadata(root.join(".git")) {
        Ok(_) => check_git_metadata(&root.join(".git"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    checked(
        command_with(&git, &root)
            .args(["init", "--quiet", "--template="])
            .output()?,
    )?;
    Ok(())
}

pub fn commit(root: &Path, message: &str) -> Result<()> {
    let repository = Repository::open(root)?;
    let files = repository.ensure_no_foreign_changes()?;
    repository.install_attribute_guard()?;
    checked(
        repository
            .command()
            .args(["add", "--all", "--force", "--"])
            .args(repository.managed_pathspecs(&files))
            .output()?,
    )?;
    let paths = repository.staged_paths()?;
    if paths.iter().any(|path| !is_managed(path)) {
        return Err(dirty());
    }
    if paths.is_empty() {
        return Ok(());
    }
    checked(
        repository
            .identity_command()
            .args(["commit", "--quiet", "--no-verify", "-m", message])
            .output()?,
    )?;
    Ok(())
}

/// Commit only the listed managed files; unrelated changes are never staged.
pub fn commit_paths(root: &Path, message: &str, paths: &[PathBuf]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let repository = Repository::open(root)?;
    let paths = repository.exact_file_paths(paths)?;
    if repository
        .staged_paths()?
        .iter()
        .any(|path| !paths.contains(path))
    {
        return Err(dirty());
    }
    let tracked = repository.tracked()?;
    repository.files_unchanged(
        &tracked
            .iter()
            .filter(|file| !paths.contains(&file.path))
            .collect::<Vec<_>>(),
    )?;
    if repository
        .untracked()?
        .iter()
        .any(|path| !paths.contains(path) && !untracked_allowed(path))
    {
        return Err(dirty());
    }
    repository.install_attribute_guard()?;
    let pathspecs: Vec<_> = paths
        .iter()
        .filter(|path| {
            repository.root.join(path).exists() || tracked.iter().any(|file| &file.path == *path)
        })
        .map(|path| format!(":(top,literal){path}"))
        .collect();
    for paths in pathspecs.chunks(64) {
        checked(
            repository
                .command()
                .args(["add", "--all", "--force", "--"])
                .args(paths)
                .output()?,
        )?;
    }
    let staged = repository.staged_paths()?;
    if staged.iter().any(|path| !paths.contains(path)) {
        return Err(dirty());
    }
    if !staged.is_empty() {
        checked(
            repository
                .identity_command()
                .args(["commit", "--quiet", "--no-verify", "-m", message])
                .output()?,
        )?;
    }
    Ok(())
}

/// Restore only the listed managed index files, preserving every other entry.
pub fn unstage_paths(root: &Path, paths: &[PathBuf]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let repository = Repository::open(root)?;
    let paths = repository.exact_file_paths(paths)?;
    repository.install_attribute_guard()?;
    for paths in paths.into_iter().collect::<Vec<_>>().chunks(64) {
        checked(
            repository
                .command()
                .args(["reset", "--no-refresh", "--quiet", "HEAD", "--"])
                .args(paths.iter().map(|path| format!(":(top,literal){path}")))
                .output()?,
        )?;
    }
    Ok(())
}

/// Restore only managed index entries after a failed import. Never changes files.
pub fn unstage_managed(root: &Path) -> Result<()> {
    let repository = Repository::open(root)?;
    repository.head()?;
    // Even a path-scoped index reset can consult conversion filters. This is a
    // real mutation, so install the same owned guard used by commit and revert.
    repository.install_attribute_guard()?;
    checked(
        repository
            .command()
            .args(["reset", "--no-refresh", "--quiet", "HEAD", "--"])
            .args(MANAGED)
            .args(
                INVENTORY[1..]
                    .iter()
                    .map(|path| format!(":(top,exclude,literal){path}")),
            )
            .output()?,
    )?;
    Ok(())
}

/// Revert the latest vault commit, retaining the complete Git history.
pub fn undo(root: &Path, dry_run: bool) -> Result<UndoReport> {
    let repository = Repository::open(root)?;
    repository.ensure_clean()?;
    let reverted_commit = repository.head()?;
    let parents = checked(
        repository
            .command()
            .args(["rev-list", "--parents", "-n", "1", &reverted_commit])
            .output()?,
    )?;
    let parent_count = String::from_utf8_lossy(&parents.stdout)
        .split_whitespace()
        .count()
        .saturating_sub(1);
    if parent_count == 0 {
        return Err(Error::decision(
            "UNDO_INITIAL_COMMIT",
            "The initial vault commit cannot be undone; choose a later vault change",
        ));
    }
    if parent_count != 1 {
        return Err(Error::decision(
            "UNDO_MERGE_COMMIT",
            "Undoing a merge requires choosing a parent; resolve it with Git before continuing",
        ));
    }
    let changed = checked(
        repository
            .command()
            .args([
                "diff-tree",
                "--no-commit-id",
                "--name-only",
                "--no-renames",
                "-r",
                "-z",
                &reverted_commit,
                "--",
            ])
            .output()?,
    )?;
    let changed_paths = nul_paths(&changed.stdout)?;
    if changed_paths.iter().any(|path| !is_managed(path)) {
        return Err(Error::decision(
            "UNDO_UNMANAGED_COMMIT",
            "The latest commit changes files outside Kinana's managed paths",
        ));
    }
    let resulting_commit = if dry_run {
        None
    } else {
        repository.install_attribute_guard()?;
        checked(
            repository
                .identity_command()
                .args(["revert", "--no-edit", &reverted_commit])
                .output()?,
        )?;
        repository.prune_removed_skills(&changed_paths)?;
        // Git does not track empty directories and removes these when the last
        // imported source/metadata file is reverted. They remain part of the
        // initialized vault structure.
        for name in ["skills", "meta", "bundles"] {
            let path = crate::index::validate_path(&repository.root.join(name))?;
            if !path.exists() {
                fs::create_dir(path)?;
            }
        }
        Some(repository.head()?)
    };
    Ok(UndoReport {
        vault: root.to_path_buf(),
        reverted_commit,
        resulting_commit,
        dry_run,
        finder_files_removed: Vec::new(),
    })
}

/// Read provenance hints without executing skill content or making a network call.
/// IDs of the skills the latest commit introduced: reverting it deletes them.
/// A skill exists through its `SKILL.md` and its metadata file.
/// What differs from the last commit, for diagnosis only. `None` when the
/// vault is clean.
pub(crate) struct Uncommitted {
    /// Paths the last commit has whose index entry or file differs from it
    /// (staged, missing, replaced, or changed in place): restoring each from
    /// the last commit puts it back.
    pub restore: Vec<String>,
    /// Staged paths the last commit does not have: unstaging leaves the file.
    pub unstage: Vec<String>,
    /// Untracked files other than `dist/` and Finder's `.DS_Store`.
    pub untracked: Vec<String>,
    /// Paths the last commit has that cannot be restored without deleting
    /// something on disk: the path is now a directory, a link or another
    /// kind of node, or one of its parents is no longer a real directory.
    /// They are named, never restored by a printed command.
    pub blocked: Vec<String>,
}

/// Whether restoring `relative` as a file would remove what is on disk now:
/// the path is something other than a regular file, or a parent of it exists
/// and is not a real directory (a file, or a link that must not be followed).
pub(crate) fn blocked_on_disk(root: &Path, relative: &str) -> bool {
    let mut path = root.to_path_buf();
    let parts = relative.split('/').collect::<Vec<_>>();
    for (n, part) in parts.iter().enumerate() {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if n + 1 == parts.len() => return !metadata.is_file(),
            Ok(metadata) if !metadata.is_dir() => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
    false
}

impl Uncommitted {
    pub fn all(&self) -> Vec<String> {
        let mut all = self
            .restore
            .iter()
            .chain(&self.unstage)
            .chain(&self.untracked)
            .chain(&self.blocked)
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        all.sort();
        all
    }
}

pub(crate) fn uncommitted(root: &Path) -> Result<Option<Uncommitted>> {
    let repository = Repository::open(root)?;
    if repository.ensure_clean().is_ok() {
        return Ok(None);
    }
    let committed = checked(
        repository
            .command()
            .args(["ls-tree", "-r", "-z", "--name-only", "HEAD"])
            .output()?,
    )?;
    let committed = nul_paths(&committed.stdout)?
        .into_iter()
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    let mut differing = repository
        .staged_paths()?
        .into_iter()
        .collect::<BTreeSet<_>>();
    let tracked = repository.tracked()?;
    for files in tracked.chunks(64) {
        let mut present = Vec::new();
        for file in files {
            // A link, or a file where a directory was, anywhere on the way:
            // named, never followed.
            if blocked_on_disk(&repository.root, &file.path) {
                differing.insert(file.path.clone());
                continue;
            }
            let path = crate::index::validate_path(&repository.root.join(&file.path))?;
            match fs::symlink_metadata(path) {
                Ok(metadata) if metadata.is_file() => {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        if (metadata.permissions().mode() & 0o111 != 0) != file.executable {
                            differing.insert(file.path.clone());
                            continue;
                        }
                    }
                    present.push(file);
                }
                Ok(_) => {
                    differing.insert(file.path.clone());
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                    ) =>
                {
                    differing.insert(file.path.clone());
                }
                Err(error) => return Err(error.into()),
            }
        }
        if present.is_empty() {
            continue;
        }
        // --no-filters: actual bytes, and no clean filter ever runs.
        let output = checked(
            repository
                .command()
                .args(["hash-object", "--no-filters", "--"])
                .args(present.iter().map(|file| &file.path))
                .output()?,
        )?;
        let hashes = std::str::from_utf8(&output.stdout)
            .map_err(|_| unsafe_repository("Git returned invalid object hashes"))?
            .lines()
            .collect::<Vec<_>>();
        if hashes.len() != present.len() {
            return Err(unsafe_repository("Git returned invalid object hashes"));
        }
        for (hash, file) in hashes.into_iter().zip(present) {
            if hash != file.oid {
                differing.insert(file.path.clone());
            }
        }
    }
    let (in_head, unstage) = differing
        .into_iter()
        .partition::<Vec<_>, _>(|path| committed.contains(path));
    let (blocked, restore) = in_head
        .into_iter()
        .partition::<Vec<_>, _>(|path| blocked_on_disk(&repository.root, path));
    let untracked = repository
        .untracked()?
        .into_iter()
        .filter(|path| !untracked_allowed(path))
        .collect();
    Ok(Some(Uncommitted {
        restore,
        unstage,
        untracked,
        blocked,
    }))
}

/// The commit `HEAD` names, for `doctor --repair`'s preview token.
pub(crate) fn head_commit(root: &Path) -> Result<String> {
    Repository::open(root)?.head()
}

/// `git restore` for exactly `paths`, read by Git from stdin, NUL-separated
/// and literal: no shell, no pattern, no conversion of any name. With
/// `worktree` the files return to the last commit on disk too; without it
/// only the index entry does (the file on disk stays).
pub(crate) fn restore_paths(root: &Path, paths: &[String], worktree: bool) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let repository = Repository::open(root)?;
    // As for every other Git write: no attribute from the vault may convert
    // the bytes Git writes back (review of 4de5068).
    repository.install_attribute_guard()?;
    let mut command = repository.command();
    command.arg("--literal-pathspecs").arg("restore");
    if worktree {
        command.args(["--source=HEAD", "--staged", "--worktree"]);
    } else {
        command.arg("--staged");
    }
    command
        .args(["--pathspec-from-file=-", "--pathspec-file-nul"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let mut input = Vec::new();
    for path in paths {
        input.extend_from_slice(path.as_bytes());
        input.push(0);
    }
    child
        .stdin
        .take()
        .ok_or_else(|| Error::new("GIT_ERROR", "Git did not accept the path list"))?
        .write_all(&input)?;
    checked(child.wait_with_output()?).map(|_| ())
}

/// The bytes Git's index holds for `path` (stage 0), or `None` when it has no
/// entry. Raw object bytes: no filter or conversion runs.
pub(crate) fn index_file(root: &Path, path: &str) -> Result<Option<Vec<u8>>> {
    let repository = Repository::open(root)?;
    let output = repository
        .command()
        .args(["cat-file", "blob", &format!(":0:{path}")])
        .output()?;
    Ok(output.status.success().then_some(output.stdout))
}

/// The bytes of `path` in the last commit, or `None` when it has none.
/// Reads the object only: no filter, conversion or checkout runs.
pub(crate) fn head_file(root: &Path, path: &str) -> Result<Option<Vec<u8>>> {
    let repository = Repository::open(root)?;
    let output = repository
        .command()
        .args(["cat-file", "blob", &format!("HEAD:{path}")])
        .output()?;
    Ok(output.status.success().then_some(output.stdout))
}

/// Paths Git tracks under `prefix`, relative to the vault root. Read from the
/// index; callers have already required it to match `HEAD` and the files.
pub(crate) fn tracked_under(root: &Path, prefix: &str) -> Result<BTreeSet<String>> {
    let repository = Repository::open(root)?;
    let output = checked(
        repository
            .command()
            .args(["ls-files", "-z", "--", prefix])
            .output()?,
    )?;
    Ok(nul_paths(&output.stdout)?
        .into_iter()
        .map(str::to_string)
        .collect())
}

/// Finder's inventory files: untracked by design and never user content.
pub(crate) fn is_inventory_name(name: &std::ffi::OsStr) -> bool {
    name == ".DS_Store"
}

/// Untracked `.DS_Store` files under `skills/`, ignored ones included, as
/// vault-relative paths.
pub(crate) fn untracked_finder_files(root: &Path) -> Result<Vec<String>> {
    let repository = Repository::open(root)?;
    let output = checked(
        repository
            .command()
            .args(["ls-files", "--others", "-z", "--", "skills"])
            .output()?,
    )?;
    Ok(nul_paths(&output.stdout)?
        .into_iter()
        .filter(|path| path.rsplit('/').next() == Some(".DS_Store"))
        .map(str::to_owned)
        .collect())
}

pub(crate) fn skills_added_by_head(root: &Path) -> Result<Vec<String>> {
    let repository = Repository::open(root)?;
    let added = checked(
        repository
            .command()
            .args([
                "diff-tree",
                "--no-commit-id",
                "--name-only",
                "--no-renames",
                "--diff-filter=A",
                "-r",
                "-z",
                "HEAD",
                "--",
                "skills",
                "meta",
            ])
            .output()?,
    )?;
    let mut ids = nul_paths(&added.stdout)?
        .into_iter()
        .filter_map(|path| {
            path.strip_prefix("meta/")
                .and_then(|name| name.strip_suffix(".toml"))
                .or_else(|| {
                    path.strip_prefix("skills/")
                        .and_then(|rest| rest.strip_suffix("/SKILL.md"))
                })
                .filter(|id| !id.contains('/'))
                .map(str::to_owned)
        })
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

fn safe_git_metadata(path: &Path) -> Option<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    for parent in absolute.ancestors() {
        let git = parent.join(".git");
        let Ok(metadata) = fs::symlink_metadata(&git) else {
            continue;
        };
        // A worktree indirection or symlink is not followed merely to obtain a hint.
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return None;
        }
        for name in ["config", "HEAD", "packed-refs", "refs"] {
            let target = git.join(name);
            if !target.exists() {
                continue;
            }
            for entry in walkdir::WalkDir::new(&target).follow_links(false) {
                let Ok(entry) = entry else {
                    return None;
                };
                if entry.file_type().is_symlink() {
                    return None;
                }
            }
        }
        return Some(parent.to_path_buf());
    }
    None
}

/// Persist only hierarchical network origins. Local paths and ambiguous Git
/// transport syntax have no reliably separable public portion.
pub(crate) fn public_remote(raw: &str) -> Option<String> {
    let mut url = url::Url::parse(raw.trim()).ok()?;
    if !matches!(url.scheme(), "https" | "http" | "git" | "ssh") || url.host().is_none() {
        return None;
    }
    url.set_password(None).ok()?;
    url.set_username("").ok()?;
    url.set_query(None);
    url.set_fragment(None);
    Some(url.into())
}

pub fn remote_hint(path: &Path) -> SourceInfo {
    remote_hint_excluding(path, &[])
}

pub(crate) fn remote_hint_excluding(path: &Path, excluded_roots: &[PathBuf]) -> SourceInfo {
    let mut source = SourceInfo::default();
    let Some(repository) = safe_git_metadata(path) else {
        return source;
    };
    let excluded_roots: Vec<_> = std::iter::once(repository)
        .chain(excluded_roots.iter().cloned())
        .collect();
    // The command runs inside the selected skill, but no executable from its
    // containing repository or another selected scan root may be launched.
    let Ok(git) = crate::executables::git(&excluded_roots) else {
        return source;
    };
    let provenance_command = || command_with(&git, path);
    let Ok(output) = provenance_command()
        .args([
            "config",
            "--local",
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
    source.kind = "scan".into();
    source.url = public_remote(url);
    if let Ok(output) = provenance_command().args(["rev-parse", "HEAD"]).output()
        && output.status.success()
    {
        let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !commit.is_empty() {
            source.commit = Some(commit);
        }
    }
    if let Ok(output) = provenance_command()
        .args(["rev-parse", "--show-toplevel"])
        .output()
        && output.status.success()
    {
        let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if let Ok(subpath) = path.strip_prefix(root) {
            source.subpath = Some(subpath.to_string_lossy().replace('\\', "/"));
        }
    }
    source
}

/// Local configuration only: never fetch, push or start a remote helper.
pub fn configured_remote(root: &Path) -> Result<Option<String>> {
    // The Git that opened and checked the repository runs the command.
    let repository = Repository::open(root)?;
    let output = repository
        .command()
        .args(["config", "--local", "--get", "remote.origin.url"])
        .output()?;
    if output.status.code() == Some(1) {
        return Ok(None);
    }
    let output = checked(output)?;
    Ok(Some(String::from_utf8_lossy(&output.stdout).trim().into()))
}
pub(crate) fn configure_remote(root: &Path, url: &str) -> Result<()> {
    let repository = Repository::open(root)?;
    repository.ensure_clean()?;
    checked(
        repository
            .command()
            .args([
                "config",
                "--local",
                "--replace-all",
                "remote.origin.url",
                url,
            ])
            .output()?,
    )?;
    Ok(())
}
