//! The one place Kinana looks up an external program (claude, codex, cursor,
//! git) and reads `PATH`.
//!
//! An app opened from Finder inherits launchd's `PATH`
//! (`/usr/bin:/bin:/usr/sbin:/sbin`), not the shell's, so the official
//! installers' `~/.local/bin` is invisible to it. After the absolute `PATH`
//! entries, a fixed list of install locations is searched (owner decision,
//! 2026-11-18): `~/.local/bin`, `/opt/homebrew/bin`, `/usr/local/bin`, and the
//! directories listed in `/etc/paths` and `/etc/paths.d/*` (read, never run).
//! A program found there is used only if the file it resolves to, every
//! directory above that file, and every directory above the location where it
//! was found, are owned by the current user or root, are never writable by
//! everyone, and are writable by a group only when that group is `admin`
//! (gid 80) and the current user belongs to it (owner decision, 2026-11-18:
//! Homebrew's `/opt/homebrew/bin` is `admin`-writable). Locations inside the
//! vault or a skill source are never used.
use crate::{Error, Result};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// What happened at one searched location, in search order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    pub directory: PathBuf,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Absent,
    /// Inside the vault or a skill source.
    Excluded,
    /// Owned by someone other than the user or root.
    Owner,
    /// Writable by a group other than `admin`, or by `admin` when the user
    /// is not a member.
    Writable,
    /// Writable by everyone.
    WorldWritable,
    /// An access list lets someone other than the owner write, append or
    /// delete entries (owner decision, 2026-11-18).
    Acl,
    /// The access list could not be read; refused.
    AclUnreadable,
    Found,
}

impl Outcome {
    fn text(self) -> &'static str {
        match self {
            Self::Absent => "not present",
            Self::Excluded => "inside the vault or a skill source",
            Self::Owner => "owned by another user",
            Self::Writable => "writable by another group",
            Self::WorldWritable => "writable by everyone",
            Self::Acl => "an access list lets another user write to it",
            Self::AclUnreadable => "its access list could not be read",
            Self::Found => "found",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Lookup {
    pub found: Option<PathBuf>,
    /// The searched directory the program was found in (as listed, before
    /// resolving links), `PATH` or fixed location.
    pub found_in: Option<PathBuf>,
    /// The `PATH` entries searched first (absolute only).
    pub path: Vec<PathBuf>,
    /// The fixed install locations searched after `PATH`, with their outcome.
    pub known: Vec<Checked>,
}

impl Lookup {
    /// "Searched PATH (/usr/bin:/bin) and ~/.local/bin (not present), …".
    /// Paths only; no environment values besides the directories themselves.
    pub fn searched(&self) -> String {
        let path = self
            .path
            .iter()
            .map(|directory| directory.display().to_string())
            .collect::<Vec<_>>()
            .join(":");
        let known = self
            .known
            .iter()
            .map(|checked| {
                format!(
                    "{} ({})",
                    checked.directory.display(),
                    checked.outcome.text()
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        if known.is_empty() {
            format!("Searched PATH ({path})")
        } else {
            format!("Searched PATH ({path}) and then {known}")
        }
    }

    /// Every searched directory, `PATH` first, for display.
    pub fn locations(&self) -> Vec<String> {
        self.path
            .iter()
            .map(|directory| directory.display().to_string())
            .chain(self.known.iter().map(|checked| {
                format!(
                    "{} ({})",
                    checked.directory.display(),
                    checked.outcome.text()
                )
            }))
            .collect()
    }
}

/// Where to look; [`SearchPlan::current`] in production, explicit in tests.
#[derive(Debug, Clone)]
pub struct SearchPlan {
    pub path: Vec<PathBuf>,
    pub known: Vec<PathBuf>,
    pub uid: u32,
    /// Whether the user belongs to `admin` (gid 80); see [`ADMIN_GID`].
    pub admin_member: AdminMembership,
}

/// Asked of the system at most once per lookup, and only when a node that is
/// group-writable by `admin` is met (owner decision, 2026-11-18): most
/// lookups never need it, and one lookup never asks twice.
#[derive(Debug, Clone, Default)]
pub struct AdminMembership(std::cell::OnceCell<bool>);

impl AdminMembership {
    /// A fixed answer, for tests.
    pub fn known(member: bool) -> Self {
        let cell = std::cell::OnceCell::new();
        let _ = cell.set(member);
        Self(cell)
    }
    /// Ask the system the first time it is needed.
    pub fn ask() -> Self {
        Self::default()
    }
    /// Whether the system was already asked (or the answer was fixed).
    pub fn answered(&self) -> bool {
        self.0.get().is_some()
    }
    fn get(&self, uid: u32) -> bool {
        *self.0.get_or_init(|| member_of_admin(uid))
    }
}

/// The system's answer for the current user.
pub fn current_user_is_admin() -> bool {
    member_of_admin(current_uid())
}

/// macOS's `admin` group. Group write is accepted for this group only.
pub const ADMIN_GID: u32 = 80;

impl SearchPlan {
    pub fn current() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let mut known: Vec<PathBuf> = home
            .filter(|home| home.is_absolute())
            .map(|home| home.join(".local/bin"))
            .into_iter()
            .collect();
        known.push("/opt/homebrew/bin".into());
        known.push("/usr/local/bin".into());
        known.extend(listed_directories(Path::new("/etc/paths")));
        let mut listed: Vec<PathBuf> = fs::read_dir("/etc/paths.d")
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .collect();
        listed.sort();
        for file in listed {
            known.extend(listed_directories(&file));
        }
        let mut unique = Vec::new();
        for directory in known {
            if !unique.contains(&directory) {
                unique.push(directory);
            }
        }
        Self {
            path: path_directories(),
            known: unique,
            uid: current_uid(),
            admin_member: AdminMembership::ask(),
        }
    }
}

/// The absolute entries of this process's `PATH`. A relative entry would be
/// resolved inside whatever directory a child happens to run in.
pub fn path_directories() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths)
                .filter(|path| path.is_absolute())
                .collect()
        })
        .unwrap_or_default()
}

/// One absolute directory per line; anything else is ignored. A missing or
/// unreadable file lists nothing. Only regular files owned by root are read.
fn listed_directories(file: &Path) -> Vec<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let Ok(metadata) = fs::symlink_metadata(file) else {
            return vec![];
        };
        if !metadata.is_file() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
            return vec![];
        }
    }
    let Ok(text) = fs::read_to_string(file) else {
        return vec![];
    };
    text.lines()
        .map(str::trim)
        .filter(|line| line.starts_with('/'))
        .map(PathBuf::from)
        .collect()
}

#[cfg(unix)]
fn current_uid() -> u32 {
    unsafe extern "C" {
        fn getuid() -> u32;
    }
    // SAFETY: getuid takes no arguments, cannot fail and touches no memory.
    unsafe { getuid() }
}
#[cfg(not(unix))]
fn current_uid() -> u32 {
    0
}

/// Membership as the system resolves it (Directory Services, nested groups
/// and the primary group included), not the process's own group list, which
/// the kernel may truncate. Any failure answers "not a member".
#[cfg(target_os = "macos")]
fn member_of_admin(uid: u32) -> bool {
    unsafe extern "C" {
        fn mbr_uid_to_uuid(id: u32, uu: *mut u8) -> i32;
        fn mbr_gid_to_uuid(id: u32, uu: *mut u8) -> i32;
        fn mbr_check_membership(user: *const u8, group: *const u8, ismember: *mut i32) -> i32;
    }
    let mut user = [0u8; 16];
    let mut group = [0u8; 16];
    let mut member: i32 = 0;
    // SAFETY: each uuid_t is a 16-byte buffer owned here; the functions write
    // at most 16 bytes into it, or one int into `member`, and keep no pointer.
    unsafe {
        mbr_uid_to_uuid(uid, user.as_mut_ptr()) == 0
            && mbr_gid_to_uuid(ADMIN_GID, group.as_mut_ptr()) == 0
            && mbr_check_membership(user.as_ptr(), group.as_ptr(), &mut member) == 0
            && member == 1
    }
}
#[cfg(not(target_os = "macos"))]
fn member_of_admin(_: u32) -> bool {
    false
}

#[cfg(all(test, target_os = "macos"))]
mod membership_tests {
    use super::*;
    /// The real answer from Directory Services, not a value set by hand.
    #[test]
    fn only_real_admin_members_are_members() {
        assert!(member_of_admin(0), "root is in admin");
        assert!(!member_of_admin(70), "_www is not in admin");
        assert!(!member_of_admin(4_294_967_294), "nobody is not in admin");
        assert!(!member_of_admin(54_321), "an unknown uid is not in admin");
        let output = Command::new("/usr/bin/id").arg("-G").output().unwrap();
        let groups = String::from_utf8(output.stdout).unwrap();
        let listed = groups.split_whitespace().any(|gid| gid == "80");
        assert_eq!(member_of_admin(current_uid()), listed);
    }
}

/// Finds `name` with the current process environment.
pub fn find(name: &str, excluded: &[PathBuf]) -> Lookup {
    find_in(&SearchPlan::current(), name, excluded)
}

pub fn find_in(plan: &SearchPlan, name: &str, excluded: &[PathBuf]) -> Lookup {
    let file_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    let excluded: Vec<PathBuf> = excluded
        .iter()
        .flat_map(|root| [Some(root.clone()), root.canonicalize().ok()])
        .flatten()
        .collect();
    let mut lookup = Lookup {
        path: plan.path.clone(),
        ..Default::default()
    };
    for directory in &plan.path {
        if let Some(found) = path_candidate(&directory.join(&file_name), &excluded) {
            lookup.found = Some(found);
            lookup.found_in = Some(directory.clone());
            return lookup;
        }
    }
    for directory in &plan.known {
        if plan.path.contains(directory) {
            continue;
        }
        let outcome = known_candidate(directory, &file_name, &excluded, plan);
        lookup.known.push(Checked {
            directory: directory.clone(),
            outcome: outcome
                .as_ref()
                .map_or_else(|outcome| *outcome, |_| Outcome::Found),
        });
        if let Ok(found) = outcome {
            lookup.found = Some(found);
            lookup.found_in = Some(directory.clone());
            return lookup;
        }
    }
    lookup
}

/// Like [`find_in`], but every candidate, `PATH` entries included, must pass
/// the fixed-location trust rule, and `accept` may pass over a candidate
/// (it is then reported as [`Outcome::Absent`] and the search goes on).
fn find_trusted(
    plan: &SearchPlan,
    name: &str,
    excluded: &[PathBuf],
    first: &[PathBuf],
    accept: impl Fn(&Path) -> bool,
) -> Lookup {
    let excluded: Vec<PathBuf> = excluded
        .iter()
        .flat_map(|root| [Some(root.clone()), root.canonicalize().ok()])
        .flatten()
        .collect();
    let mut lookup = Lookup::default();
    let mut seen: Vec<PathBuf> = Vec::new();
    for directory in first.iter().chain(&plan.path).chain(&plan.known) {
        if seen.contains(directory) {
            continue;
        }
        seen.push(directory.clone());
        let outcome = known_candidate(directory, name, &excluded, plan).and_then(|found| {
            if accept(&found) {
                Ok(found)
            } else {
                Err(Outcome::Absent)
            }
        });
        lookup.known.push(Checked {
            directory: directory.clone(),
            outcome: outcome
                .as_ref()
                .map_or_else(|outcome| *outcome, |_| Outcome::Found),
        });
        if let Ok(found) = outcome {
            lookup.found = Some(found);
            lookup.found_in = Some(directory.clone());
            return lookup;
        }
    }
    lookup
}

fn inside(path: &Path, excluded: &[PathBuf]) -> bool {
    excluded.iter().any(|root| path.starts_with(root))
}

fn executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// A `PATH` candidate: the user chose this directory in their own `PATH`, so
/// only the vault/source exclusion applies, as before 2026-11-18.
fn path_candidate(candidate: &Path, excluded: &[PathBuf]) -> Option<PathBuf> {
    let canonical = candidate.canonicalize().ok()?;
    if inside(candidate, excluded) || inside(&canonical, excluded) || !executable_file(&canonical) {
        return None;
    }
    Some(canonical)
}

fn known_candidate(
    directory: &Path,
    file_name: &str,
    excluded: &[PathBuf],
    plan: &SearchPlan,
) -> std::result::Result<PathBuf, Outcome> {
    let candidate = directory.join(file_name);
    let canonical = candidate.canonicalize().map_err(|_| Outcome::Absent)?;
    if !executable_file(&canonical) {
        return Err(Outcome::Absent);
    }
    if inside(&candidate, excluded) || inside(&canonical, excluded) {
        return Err(Outcome::Excluded);
    }
    let directory = directory.canonicalize().map_err(|_| Outcome::Absent)?;
    if inside(&directory, excluded) {
        return Err(Outcome::Excluded);
    }
    secure_chain(&canonical, plan)?;
    secure_chain(&directory, plan)?;
    Ok(canonical)
}

/// `path` and every directory above it: owned by the user or root, never
/// writable by everyone, and writable by a group only per [`writable_by`].
#[cfg(unix)]
fn secure_chain(path: &Path, plan: &SearchPlan) -> std::result::Result<(), Outcome> {
    use std::os::unix::fs::MetadataExt;
    for ancestor in path.ancestors() {
        let metadata = fs::metadata(ancestor).map_err(|_| Outcome::Absent)?;
        if metadata.uid() != plan.uid && metadata.uid() != 0 {
            return Err(Outcome::Owner);
        }
        let (mode, gid) = (metadata.mode(), metadata.gid());
        // Only an admin-writable node needs the membership answer.
        let member = mode & 0o020 != 0 && gid == ADMIN_GID && plan.admin_member.get(plan.uid);
        writable_by(mode, gid, member)?;
        if acl_grants_write(ancestor, metadata.uid())? {
            return Err(Outcome::Acl);
        }
    }
    Ok(())
}

/// Whether `path`'s extended access list (macOS) has an allow entry giving
/// anyone but the owner `write` (`add_file`), `append` (`add_subdirectory`)
/// or `delete_child`. Group entries never count as the owner. An access list
/// that cannot be read refuses the path. Deny entries cannot grant anything.
#[cfg(target_os = "macos")]
fn acl_grants_write(path: &Path, owner: u32) -> std::result::Result<bool, Outcome> {
    use std::os::unix::ffi::OsStrExt;
    const ACL_TYPE_EXTENDED: i32 = 0x100;
    unsafe extern "C" {
        fn acl_get_link_np(path: *const std::ffi::c_char, kind: i32) -> *mut std::ffi::c_void;
        fn acl_to_text(acl: *mut std::ffi::c_void, length: *mut isize) -> *mut std::ffi::c_char;
        fn acl_free(object: *mut std::ffi::c_void) -> i32;
    }
    let Ok(name) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return Err(Outcome::Acl);
    };
    // SAFETY: `name` is a valid NUL-terminated string for the call; the
    // returned list and text are owned here and freed with acl_free once;
    // the text is copied before it is freed.
    let text = unsafe {
        let acl = acl_get_link_np(name.as_ptr(), ACL_TYPE_EXTENDED);
        if acl.is_null() {
            return match std::io::Error::last_os_error().raw_os_error() {
                Some(2) => Ok(false), // ENOENT: no extended access list
                _ => Err(Outcome::AclUnreadable),
            };
        }
        let text = acl_to_text(acl, std::ptr::null_mut());
        let copied = (!text.is_null()).then(|| {
            std::ffi::CStr::from_ptr(text)
                .to_string_lossy()
                .into_owned()
        });
        if !text.is_null() {
            acl_free(text.cast());
        }
        acl_free(acl);
        copied
    };
    let Some(text) = text else {
        return Err(Outcome::AclUnreadable);
    };
    Ok(acl_text_grants_write(&text, owner))
}
#[cfg(all(unix, not(target_os = "macos")))]
fn acl_grants_write(_: &Path, _: u32) -> std::result::Result<bool, Outcome> {
    Ok(false)
}

/// `acl_to_text` lines: `kind:uuid:name:id:rule[,flags]:permissions`, where
/// the rule field may carry inheritance flags (`allow,file_inherit,...`).
/// The name may itself hold a colon, so the fields are read from both ends.
/// Fail closed: an allow entry for anyone but the owner refuses unless every
/// permission it grants is in a known read-only set (this also refuses
/// `writesecurity` and `chown`, which let the holder grant itself write, and
/// any permission name not recognised). An unreadable entry refuses.
pub fn acl_text_grants_write(text: &str, owner: u32) -> bool {
    const READ_ONLY: [&str; 8] = [
        "read",
        "execute",
        "readattr",
        "readextattr",
        "readsecurity",
        "list",
        "search",
        "synchronize",
    ];
    text.lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#') && !line.starts_with("!#"))
        .any(|line| {
            let mut back = line.rsplitn(4, ':');
            let (Some(permissions), Some(rule), Some(id), Some(front)) =
                (back.next(), back.next(), back.next(), back.next())
            else {
                return true;
            };
            let kind = front.split(':').next().unwrap_or("");
            match rule.split(',').next().map(str::trim) {
                Some("deny") => false,
                Some("allow") => {
                    let owner_entry = kind == "user" && id.parse::<u32>() == Ok(owner);
                    let read_only = permissions
                        .split(',')
                        .map(str::trim)
                        .all(|permission| READ_ONLY.contains(&permission));
                    !owner_entry && !read_only
                }
                _ => true,
            }
        })
}
#[cfg(not(unix))]
fn secure_chain(_: &Path, _: &SearchPlan) -> std::result::Result<(), Outcome> {
    Ok(())
}

/// The write rule for one file or directory (owner decision, 2026-11-18).
pub fn writable_by(mode: u32, gid: u32, admin_member: bool) -> std::result::Result<(), Outcome> {
    if mode & 0o002 != 0 {
        return Err(Outcome::WorldWritable);
    }
    if mode & 0o020 != 0 && !(gid == ADMIN_GID && admin_member) {
        return Err(Outcome::Writable);
    }
    Ok(())
}

/// The system directories every agent child may use.
pub const SYSTEM_PATH: [&str; 4] = ["/usr/bin", "/bin", "/usr/sbin", "/sbin"];

/// The `PATH` an agent CLI runs with (owner decision, 2026-11-18): the
/// system directories, plus, only when the CLI is a script run through
/// `/usr/bin/env node`, a private 0700 directory holding one link: a `node`
/// that passed the fixed-location trust rule. A native (Mach-O) CLI gets the
/// system directories only. The folder the CLI was found in is never added.
#[derive(Debug)]
pub struct AgentPath {
    pub path: OsString,
    private: Option<PrivateBin>,
}

impl AgentPath {
    /// The private directory, when one was made.
    pub fn private_directory(&self) -> Option<&Path> {
        self.private.as_ref().map(|bin| bin.0.as_path())
    }
}

/// Why an agent was not started; `locations` are shown to the user.
#[derive(Debug)]
pub struct Refusal {
    pub code: &'static str,
    pub message: String,
    pub locations: Vec<String>,
}

#[derive(Debug)]
struct PrivateBin(PathBuf);
impl Drop for PrivateBin {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// How the kernel will start `executable`.
#[derive(Debug, PartialEq, Eq)]
pub enum Launcher {
    Native,
    /// `#!/usr/bin/env <program>`.
    Env(String),
    /// `#!<absolute interpreter>`.
    Interpreter(PathBuf),
    /// A `/usr/bin/env` line other than `env [-S] <bare name>`.
    Refused(String),
    Unknown,
}

pub fn launcher(executable: &Path) -> Launcher {
    use std::io::Read;
    let mut head = [0u8; 512];
    let Ok(read) = fs::File::open(executable).and_then(|mut file| file.read(&mut head)) else {
        return Launcher::Unknown;
    };
    let head = &head[..read];
    const MAGIC: [[u8; 4]; 8] = [
        [0xcf, 0xfa, 0xed, 0xfe],
        [0xce, 0xfa, 0xed, 0xfe],
        [0xfe, 0xed, 0xfa, 0xcf],
        [0xfe, 0xed, 0xfa, 0xce],
        [0xca, 0xfe, 0xba, 0xbe],
        [0xbe, 0xba, 0xfe, 0xca],
        [0xca, 0xfe, 0xba, 0xbf],
        [0xbf, 0xba, 0xfe, 0xca],
    ];
    if head.len() >= 4 && MAGIC.iter().any(|magic| head[..4] == magic[..]) {
        return Launcher::Native;
    }
    let Some(line) = head.strip_prefix(b"#!") else {
        return Launcher::Unknown;
    };
    let line = line.split(|byte| *byte == b'\n').next().unwrap_or_default();
    let line = String::from_utf8_lossy(line);
    let mut words = line.split_whitespace();
    let Some(interpreter) = words.next() else {
        return Launcher::Unknown;
    };
    if interpreter == "/usr/bin/env" {
        // Only `env [-S] <bare name>` (review 2026-11-18): any other option,
        // a NAME=VALUE assignment or a program with a path could run a
        // program that was never checked.
        let mut program = words.next();
        if program == Some("-S") {
            program = words.next();
        }
        return match program {
            Some(name) if !name.contains(['/', '=']) && !name.starts_with('-') => {
                Launcher::Env(name.to_owned())
            }
            _ => Launcher::Refused(line.trim().to_owned()),
        };
    }
    Launcher::Interpreter(interpreter.into())
}

pub fn agent_path(
    plan: &SearchPlan,
    executable: &Path,
    found_in: Option<&Path>,
    excluded: &[PathBuf],
) -> std::result::Result<AgentPath, Refusal> {
    let system = || std::env::join_paths(SYSTEM_PATH).expect("fixed system PATH");
    match launcher(executable) {
        Launcher::Native | Launcher::Unknown => Ok(AgentPath {
            path: system(),
            private: None,
        }),
        Launcher::Interpreter(interpreter) => {
            let directory = interpreter.parent().unwrap_or(Path::new("/"));
            let name = interpreter
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let lookup = find_trusted(
                &SearchPlan {
                    path: vec![],
                    known: vec![],
                    ..plan.clone()
                },
                &name,
                excluded,
                &[directory.into()],
                |_| true,
            );
            match lookup.found {
                Some(_) => Ok(AgentPath {
                    path: system(),
                    private: None,
                }),
                None => Err(Refusal {
                    code: "AGENT_INTERPRETER_UNTRUSTED",
                    message: format!(
                        "The Claude script names the interpreter {}, which does not pass the trust rule ({}). Claude was not started. Using keywords.",
                        interpreter.display(),
                        lookup.locations().join(", ")
                    ),
                    locations: lookup.locations(),
                }),
            }
        }
        Launcher::Env(program) if program == "node" => {
            let first: Vec<PathBuf> = found_in.map(Path::to_path_buf).into_iter().collect();
            let lookup = find_trusted(plan, "node", excluded, &first, |_| true);
            let Some(node) = lookup.found.clone() else {
                let untrusted = lookup
                    .known
                    .iter()
                    .any(|checked| !matches!(checked.outcome, Outcome::Absent | Outcome::Found));
                return Err(Refusal {
                    code: if untrusted {
                        "AGENT_NODE_UNTRUSTED"
                    } else {
                        "AGENT_NODE_NOT_FOUND"
                    },
                    message: format!(
                        "Claude is a Node.js script, and no node passing the trust rule was found. Searched {}. Claude was not started. Using keywords.",
                        lookup.locations().join(", ")
                    ),
                    locations: lookup.locations(),
                });
            };
            let private = private_bin(&node).map_err(|_| Refusal {
                code: "AGENT_WORKDIR_UNAVAILABLE",
                message: "A private directory for node could not be created; using keywords."
                    .into(),
                locations: vec![],
            })?;
            let mut entries = vec![private.0.clone()];
            entries.extend(SYSTEM_PATH.iter().map(PathBuf::from));
            Ok(AgentPath {
                path: std::env::join_paths(entries).expect("absolute PATH entries"),
                private: Some(private),
            })
        }
        // Any other `env` program resolves in the system directories only.
        Launcher::Refused(line) => Err(Refusal {
            code: "AGENT_INTERPRETER_UNTRUSTED",
            message: format!(
                "The Claude script starts with `#!{line}`, which could run a program that was not checked. Claude was not started. Using keywords."
            ),
            locations: vec![],
        }),
        Launcher::Env(_) => Ok(AgentPath {
            path: system(),
            private: None,
        }),
    }
}

fn private_bin(node: &Path) -> std::io::Result<PrivateBin> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let base = fs::canonicalize(std::env::temp_dir())?;
    let directory = base.join(format!(
        "kinana-agent-bin-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |time| time.as_nanos()),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    // Fails if anything already exists at this name: nothing is followed.
    builder.create(&directory)?;
    let private = PrivateBin(directory);
    #[cfg(unix)]
    std::os::unix::fs::symlink(node, private.0.join("node"))?;
    Ok(private)
}

/// Child processes speak one locale whether the app was opened from Finder
/// (no `LANG`) or from a terminal (any `LANG`/`LC_*`): owner decision.
pub fn set_child_locale(command: &mut Command) {
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("LC_") {
            command.env_remove(key);
        }
    }
    command.env("LANG", "en_US.UTF-8");
}

/// The Git used for vault and source work, outside `excluded`. On macOS,
/// `/usr/bin/git` is a stub until the Command Line Tools (or Xcode) are
/// installed: running it opens an install prompt and fails. That case is
/// reported as `GIT_TOOLS_MISSING` before Git is ever run.
pub fn git(excluded: &[PathBuf]) -> Result<PathBuf> {
    git_in(&SearchPlan::current(), excluded)
}

/// The first Git in search order that is not the Command Line Tools stub
/// (owner decision, 2026-11-18): a trusted Git later in the order wins over
/// `/usr/bin/git` when the tools are missing. `GIT_TOOLS_MISSING` only when
/// the stub was the only Git found.
pub fn git_in(plan: &SearchPlan, excluded: &[PathBuf]) -> Result<PathBuf> {
    git_choosing(plan, excluded, |git| {
        cfg!(target_os = "macos") && git == Path::new("/usr/bin/git") && !developer_tools_present()
    })
}

/// [`git_in`] with the stub test given, so the choice can be tested on a
/// machine that has the Command Line Tools.
pub fn git_choosing(
    plan: &SearchPlan,
    excluded: &[PathBuf],
    stub: impl Fn(&Path) -> bool,
) -> Result<PathBuf> {
    let lookup = find_in(plan, "git", excluded);
    let Some(git) = lookup.found.clone() else {
        return Err(Error::new(
            "GIT_ERROR",
            format!(
                "Git was not found outside the vault and skill sources. {}",
                lookup.searched()
            ),
        ));
    };
    if !stub(&git) {
        return Ok(git);
    }
    // Every other location, PATH included, under the fixed-location rule.
    let other = find_trusted(plan, "git", excluded, &[], |found| !stub(found));
    other.found.ok_or_else(|| {
        Error::new(
            "GIT_TOOLS_MISSING",
            "Git needs Apple's Command Line Tools. Install them with `xcode-select --install`, then retry.",
        )
    })
}

/// `xcode-select -p` is a real binary (not a stub) and prints the active
/// developer directory, or fails without prompting when there is none.
fn developer_tools_present() -> bool {
    // Only a positive answer is kept: tools installed while the app runs are
    // seen at the next Git call, and Git calls do not each start xcode-select.
    static PRESENT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    if PRESENT.get().is_some() {
        return true;
    }
    let present = query_developer_tools();
    if present {
        let _ = PRESENT.set(());
    }
    present
}

fn query_developer_tools() -> bool {
    let mut command = Command::new("/usr/bin/xcode-select");
    command
        .arg("-p")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let Ok(output) = command.output() else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let Ok(directory) = String::from_utf8(output.stdout) else {
        return false;
    };
    Path::new(directory.trim()).join("usr/bin/git").is_file()
}
