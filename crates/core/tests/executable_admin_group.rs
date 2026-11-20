//! Group write in the trusted install locations (owner decision,
//! 2026-11-18): the owner stays the user or root; group write is accepted only
//! for `admin` (gid 80) and only when the user is a member; write by everyone
//! is always refused. The rule covers the resolved file and every directory
//! above it and above the location where it was found.
#![cfg(unix)]
use kinana_core::executables::{
    ADMIN_GID, AdminMembership, Outcome, SearchPlan, current_user_is_admin, find_in, writable_by,
};
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, chown};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT: AtomicU32 = AtomicU32::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        // The per-user temp directory (0700, owned by the user, under
        // root-owned parents), not /tmp, which everyone can write to.
        let path = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "kinana-admin-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A skip is a failure when KINANA_REQUIRE_ADMIN_TESTS=1 (review 2026-11-18):
/// on an account outside admin these tests would otherwise pass untested.
fn skip(reason: &str) {
    assert!(
        std::env::var_os("KINANA_REQUIRE_ADMIN_TESTS").is_none(),
        "required admin test skipped: {reason}"
    );
    eprintln!("skipped: {reason}");
}

fn plan(bin: &Path, admin_member: bool) -> SearchPlan {
    SearchPlan {
        path: vec![],
        known: vec![bin.into()],
        admin_member: AdminMembership::known(admin_member),
        ..SearchPlan::current()
    }
}

/// A `bin` directory holding an executable `claude`, with the given group
/// and mode on the directory.
fn bin(temp: &Temp, gid: u32, mode: u32) -> Option<PathBuf> {
    let bin = temp.0.join("bin");
    fs::create_dir(&bin).unwrap();
    let program = bin.join("claude");
    fs::write(&program, "#!/bin/sh\n").unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    // Only a member of the group can give it a file; skip otherwise.
    chown(&bin, None, Some(gid)).ok()?;
    fs::set_permissions(&bin, fs::Permissions::from_mode(mode)).unwrap();
    Some(bin)
}

fn outcome(plan: &SearchPlan) -> Outcome {
    let lookup = find_in(plan, "claude", &[]);
    assert_eq!(lookup.known.len(), 1, "{lookup:?}");
    lookup.known[0].outcome
}

#[test]
fn the_rule_itself() {
    assert_eq!(writable_by(0o755, 20, false), Ok(()));
    assert_eq!(writable_by(0o775, ADMIN_GID, true), Ok(()));
    assert_eq!(writable_by(0o775, ADMIN_GID, false), Err(Outcome::Writable));
    assert_eq!(writable_by(0o775, 20, true), Err(Outcome::Writable));
    assert_eq!(
        writable_by(0o777, ADMIN_GID, true),
        Err(Outcome::WorldWritable)
    );
    assert_eq!(writable_by(0o757, 0, true), Err(Outcome::WorldWritable));
}

#[test]
fn admin_writable_bin_is_used_by_a_member() {
    let temp = Temp::new();
    let Some(bin) = bin(&temp, ADMIN_GID, 0o775) else {
        skip("this user cannot give a folder to admin");
        return;
    };
    let lookup = find_in(&plan(&bin, true), "claude", &[]);
    assert_eq!(lookup.found, Some(bin.join("claude")), "{lookup:?}");
}

#[test]
fn admin_writable_bin_is_refused_to_a_non_member() {
    let temp = Temp::new();
    let Some(bin) = bin(&temp, ADMIN_GID, 0o775) else {
        skip("this user cannot give a folder to admin");
        return;
    };
    assert_eq!(outcome(&plan(&bin, false)), Outcome::Writable);
}

#[test]
fn another_writable_group_is_refused() {
    let temp = Temp::new();
    // The user's primary group: a member, but not admin.
    let gid = fs::metadata(&temp.0).unwrap().gid();
    assert_ne!(gid, ADMIN_GID);
    let bin = bin(&temp, gid, 0o775).unwrap();
    assert_eq!(outcome(&plan(&bin, true)), Outcome::Writable);
}

#[test]
fn world_writable_is_refused_even_for_admin() {
    let temp = Temp::new();
    let bin = bin(&temp, ADMIN_GID, 0o777)
        .or_else(|| {
            let temp = Temp::new();
            let gid = fs::metadata(&temp.0).unwrap().gid();
            bin(&temp, gid, 0o777)
        })
        .unwrap();
    assert_eq!(outcome(&plan(&bin, true)), Outcome::WorldWritable);
}

#[test]
fn a_link_into_another_writable_group_is_refused() {
    // The rule applies to the resolved file's parents too: a link from a
    // clean bin into another group's writable folder is refused.
    let temp = Temp::new();
    let shared = temp.0.join("shared");
    fs::create_dir(&shared).unwrap();
    fs::write(shared.join("claude"), "#!/bin/sh\n").unwrap();
    fs::set_permissions(shared.join("claude"), fs::Permissions::from_mode(0o755)).unwrap();
    let gid = fs::metadata(&temp.0).unwrap().gid();
    fs::set_permissions(&shared, fs::Permissions::from_mode(0o775)).unwrap();
    assert_ne!(gid, ADMIN_GID);
    let bin = temp.0.join("bin");
    fs::create_dir(&bin).unwrap();
    std::os::unix::fs::symlink(shared.join("claude"), bin.join("claude")).unwrap();
    assert_eq!(outcome(&plan(&bin, true)), Outcome::Writable);
}

/// This Mac's Homebrew prefix, when it has the standard layout: owned by the
/// user, group admin, group-writable `bin`.
#[test]
fn homebrew_bin_with_admin_group_is_accepted() {
    let bin = Path::new("/opt/homebrew/bin");
    let Ok(metadata) = fs::metadata(bin) else {
        skip("no /opt/homebrew/bin");
        return;
    };
    let current = SearchPlan::current();
    if metadata.gid() != ADMIN_GID || metadata.mode() & 0o020 == 0 || !current_user_is_admin() {
        skip("/opt/homebrew/bin is not admin-writable or the user is not in admin");
        return;
    }
    let plan = SearchPlan {
        path: vec![],
        known: vec![bin.into()],
        ..current
    };
    let lookup = find_in(&plan, "brew", &[]);
    assert_eq!(lookup.known[0].outcome, Outcome::Found, "{lookup:?}");
    // The same folder for a user outside admin.
    let refused = SearchPlan {
        admin_member: AdminMembership::known(false),
        ..plan
    };
    assert_eq!(
        find_in(&refused, "brew", &[]).known[0].outcome,
        Outcome::Writable
    );
}

/// Homebrew's real layout: bin/x is a link into an admin-writable folder
/// (Cellar/Caskroom). The target's own chain decides, for members only.
#[test]
fn a_link_into_an_admin_writable_folder_follows_membership() {
    let temp = Temp::new();
    let cellar = temp.0.join("cellar");
    fs::create_dir(&cellar).unwrap();
    fs::write(cellar.join("claude"), "#!/bin/sh\n").unwrap();
    fs::set_permissions(cellar.join("claude"), fs::Permissions::from_mode(0o755)).unwrap();
    if chown(&cellar, None, Some(ADMIN_GID)).is_err() {
        skip("this user cannot give a folder to admin");
        return;
    }
    fs::set_permissions(&cellar, fs::Permissions::from_mode(0o775)).unwrap();
    let bin = temp.0.join("bin");
    fs::create_dir(&bin).unwrap();
    std::os::unix::fs::symlink(cellar.join("claude"), bin.join("claude")).unwrap();
    let lookup = find_in(&plan(&bin, true), "claude", &[]);
    assert_eq!(lookup.found, Some(cellar.join("claude")), "{lookup:?}");
    assert_eq!(outcome(&plan(&bin, false)), Outcome::Writable);
}

/// The membership answer is asked only when an admin-writable node is met,
/// and at most once per lookup plan (owner decision, 2026-11-18).
#[test]
fn membership_is_asked_only_when_needed_and_once() {
    let temp = Temp::new();
    let gid = fs::metadata(&temp.0).unwrap().gid();
    let staff = bin(&temp, gid, 0o755).unwrap();
    let lazy = SearchPlan {
        path: vec![],
        known: vec![staff.clone()],
        admin_member: AdminMembership::ask(),
        ..SearchPlan::current()
    };
    assert_eq!(
        find_in(&lazy, "claude", &[]).found,
        Some(staff.join("claude"))
    );
    assert!(
        !lazy.admin_member.answered(),
        "no admin-writable node, no question"
    );
    let admin = Temp::new();
    let Some(admin_bin) = bin(&admin, ADMIN_GID, 0o775) else {
        skip("this user cannot give a folder to admin");
        return;
    };
    let lazy = SearchPlan {
        known: vec![admin_bin.clone(), staff],
        ..lazy
    };
    find_in(&lazy, "claude", &[]);
    assert!(
        lazy.admin_member.answered(),
        "asked at the admin-writable node"
    );
}

/// Owner decision 2026-11-18: the first trusted real Git wins over the
/// Command Line Tools stub; the stub alone is GIT_TOOLS_MISSING.
#[test]
fn a_trusted_real_git_wins_over_the_tools_stub() {
    use kinana_core::executables::git_choosing;
    let temp = Temp::new();
    let stub_bin = temp.0.join("stub");
    let real_bin = temp.0.join("real");
    for directory in [&stub_bin, &real_bin] {
        fs::create_dir(directory).unwrap();
        fs::write(directory.join("git"), "#!/bin/sh\n").unwrap();
        fs::set_permissions(directory.join("git"), fs::Permissions::from_mode(0o755)).unwrap();
    }
    let is_stub = |git: &Path| git.starts_with(&stub_bin);
    let plan = |known: Vec<PathBuf>| SearchPlan {
        path: vec![stub_bin.clone()],
        known,
        admin_member: AdminMembership::known(true),
        ..SearchPlan::current()
    };
    assert_eq!(
        git_choosing(&plan(vec![real_bin.clone()]), &[], is_stub).unwrap(),
        real_bin.join("git")
    );
    let missing = git_choosing(&plan(vec![]), &[], is_stub).unwrap_err();
    assert_eq!(missing.code, "GIT_TOOLS_MISSING");
    // An untrusted real Git is not used instead of the stub.
    fs::set_permissions(&real_bin, fs::Permissions::from_mode(0o777)).unwrap();
    let untrusted = git_choosing(&plan(vec![real_bin.clone()]), &[], is_stub).unwrap_err();
    assert_eq!(untrusted.code, "GIT_TOOLS_MISSING");
    // Without the stub, the first Git in order is used as before.
    assert_eq!(
        git_choosing(&plan(vec![real_bin.clone()]), &[], |_| false).unwrap(),
        stub_bin.join("git")
    );
}
