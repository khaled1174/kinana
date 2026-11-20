//! The PATH an agent CLI runs with, and the access-list rule (owner
//! decisions, 2026-11-18): the folder Claude was found in is never added;
//! a Node.js script gets a private 0700 folder holding one trusted `node`
//! link; a native CLI gets the system folders only; any file or parent with
//! an access list letting someone else write, append or delete entries is
//! refused.
#![cfg(target_os = "macos")]
use kinana_core::executables::{
    ADMIN_GID, AdminMembership, Launcher, Outcome, SYSTEM_PATH, SearchPlan, acl_text_grants_write,
    agent_path, find_in, launcher,
};
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, chown, symlink};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT: AtomicU32 = AtomicU32::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "kinana-agent-path-{}-{}",
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

fn executable(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Nothing from this machine: only the folders a test names.
fn plan(admin_member: bool) -> SearchPlan {
    SearchPlan {
        path: vec![],
        known: vec![],
        admin_member: AdminMembership::known(admin_member),
        ..SearchPlan::current()
    }
}

fn system() -> String {
    SYSTEM_PATH.join(":")
}

/// Homebrew's npm layout: prefix/bin/claude links to the package script,
/// prefix/bin/node links into Cellar; bin is admin-writable.
fn npm_prefix(temp: &Temp, cellar_mode: u32) -> PathBuf {
    let prefix = temp.0.join("homebrew");
    let script = prefix.join("lib/node_modules/@anthropic-ai/claude-code/cli.js");
    executable(&script, "#!/usr/bin/env node\nconsole.log('claude')\n");
    let node = prefix.join("Cellar/node/24.0.0/bin/node");
    executable(&node, "#!/bin/sh\n");
    fs::set_permissions(
        node.parent().unwrap(),
        fs::Permissions::from_mode(cellar_mode),
    )
    .unwrap();
    let bin = prefix.join("bin");
    fs::create_dir_all(&bin).unwrap();
    symlink(&script, bin.join("claude")).unwrap();
    symlink(&node, bin.join("node")).unwrap();
    if chown(&bin, None, Some(ADMIN_GID)).is_ok() {
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o775)).unwrap();
    }
    bin
}

#[test]
fn npm_install_with_homebrew_node_gets_a_private_node_and_system_folders() {
    let temp = Temp::new();
    let bin = npm_prefix(&temp, 0o755);
    let claude = fs::canonicalize(bin.join("claude")).unwrap();
    assert_eq!(launcher(&claude), Launcher::Env("node".into()));
    let path = agent_path(&plan(true), &claude, Some(&bin), &[]).unwrap();
    let private = path.private_directory().unwrap().to_path_buf();
    let entries: Vec<PathBuf> = std::env::split_paths(&path.path).collect();
    assert_eq!(entries[0], private);
    assert_eq!(
        std::env::join_paths(&entries[1..])
            .unwrap()
            .to_string_lossy(),
        system()
    );
    assert!(
        !entries.contains(&bin),
        "the folder Claude was found in is never added"
    );
    let metadata = fs::symlink_metadata(&private).unwrap();
    assert_eq!(metadata.mode() & 0o777, 0o700);
    let listing: Vec<_> = fs::read_dir(&private)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(listing, ["node"]);
    assert_eq!(
        fs::read_link(private.join("node")).unwrap(),
        fs::canonicalize(bin.join("node")).unwrap()
    );
    drop(path);
    assert!(
        !private.exists(),
        "the private folder is removed after the run"
    );
}

#[test]
fn native_claude_gets_the_system_folders_only() {
    let temp = Temp::new();
    let bin = temp.0.join("bin");
    fs::create_dir(&bin).unwrap();
    // Any Mach-O executable stands in for the native installer's binary.
    fs::copy("/usr/bin/true", bin.join("claude")).unwrap();
    executable(&bin.join("node"), "#!/bin/sh\n");
    let claude = bin.join("claude");
    assert_eq!(launcher(&claude), Launcher::Native);
    let path = agent_path(&plan(true), &claude, Some(&bin), &[]).unwrap();
    assert_eq!(path.path.to_string_lossy(), system());
    assert!(path.private_directory().is_none());
}

#[test]
fn an_untrusted_node_is_refused_with_its_reason() {
    let temp = Temp::new();
    let bin = npm_prefix(&temp, 0o777);
    let claude = fs::canonicalize(bin.join("claude")).unwrap();
    let refusal = agent_path(&plan(true), &claude, Some(&bin), &[]).unwrap_err();
    assert_eq!(refusal.code, "AGENT_NODE_UNTRUSTED");
    let expected = format!("{} (writable by everyone)", bin.display());
    assert!(
        refusal.locations.contains(&expected),
        "{:?}",
        refusal.locations
    );
    assert!(refusal.message.contains(&expected), "{}", refusal.message);
}

#[test]
fn a_node_script_without_any_node_is_refused() {
    let temp = Temp::new();
    let bin = temp.0.join("bin");
    executable(&bin.join("claude"), "#!/usr/bin/env node\n");
    let refusal = agent_path(&plan(true), &bin.join("claude"), Some(&bin), &[]).unwrap_err();
    assert_eq!(refusal.code, "AGENT_NODE_NOT_FOUND");
}

#[test]
fn an_untrusted_absolute_interpreter_is_refused() {
    let temp = Temp::new();
    let open = temp.0.join("open");
    executable(&open.join("python"), "#!/bin/sh\n");
    fs::set_permissions(&open, fs::Permissions::from_mode(0o777)).unwrap();
    let bin = temp.0.join("bin");
    executable(
        &bin.join("claude"),
        &format!("#!{}\n", open.join("python").display()),
    );
    let refusal = agent_path(&plan(true), &bin.join("claude"), Some(&bin), &[]).unwrap_err();
    assert_eq!(refusal.code, "AGENT_INTERPRETER_UNTRUSTED");
    // A system interpreter passes.
    executable(&bin.join("claude"), "#!/usr/bin/python3\n");
    let path = agent_path(&plan(true), &bin.join("claude"), Some(&bin), &[]).unwrap();
    assert_eq!(path.path.to_string_lossy(), system());
}

fn chmod_acl(rule: &str, path: &Path) {
    let status = std::process::Command::new("/bin/chmod")
        .arg("+a")
        .arg(rule)
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success(), "chmod +a {rule}");
}

fn lookup_outcome(bin: &Path) -> Outcome {
    let plan = SearchPlan {
        known: vec![bin.into()],
        ..plan(true)
    };
    let lookup = find_in(&plan, "claude", &[]);
    lookup.known[0].outcome
}

#[test]
fn an_access_list_letting_another_user_write_is_refused() {
    let user = std::env::var("USER").unwrap();
    for (target, rule, refused) in [
        ("folder", "staff allow add_file".to_string(), true),
        ("folder", "everyone allow delete_child".to_string(), true),
        ("folder", "admin allow add_subdirectory".to_string(), true),
        ("file", "everyone allow append".to_string(), true),
        ("file", "staff allow write".to_string(), true),
        (
            "folder",
            "staff allow add_file,file_inherit,directory_inherit".to_string(),
            true,
        ),
        ("folder", "staff allow writesecurity".to_string(), true),
        ("file", "everyone allow chown".to_string(), true),
        ("folder", "everyone deny delete".to_string(), false),
        ("folder", "staff allow list,search".to_string(), false),
        ("file", format!("user:{user} allow write"), false),
    ] {
        let temp = Temp::new();
        let bin = temp.0.join("bin");
        executable(&bin.join("claude"), "#!/bin/sh\n");
        let path = if target == "folder" {
            bin.clone()
        } else {
            bin.join("claude")
        };
        chmod_acl(&rule, &path);
        let outcome = lookup_outcome(&bin);
        assert_eq!(
            outcome == Outcome::Acl,
            refused,
            "{target} {rule}: {outcome:?}"
        );
        if !refused {
            assert_eq!(outcome, Outcome::Found, "{target} {rule}");
        }
    }
}

#[test]
fn an_access_list_on_a_parent_folder_is_refused() {
    let temp = Temp::new();
    let bin = temp.0.join("parent/bin");
    executable(&bin.join("claude"), "#!/bin/sh\n");
    chmod_acl(
        "staff allow add_subdirectory,delete_child",
        &temp.0.join("parent"),
    );
    assert_eq!(lookup_outcome(&bin), Outcome::Acl);
}

#[test]
fn access_list_text_rule() {
    let owner = 501;
    let deny = "!#acl 1\ngroup:ABCDEFAB-CDEF-ABCD-EFAB-CDEF0000000C:everyone:12:deny:delete\n";
    assert!(!acl_text_grants_write(deny, owner));
    let own = "!#acl 1\nuser:88DF566D-A38D-498B-8DFA-666E8E997CCD:me:501:allow:write,append\n";
    assert!(!acl_text_grants_write(own, owner));
    let other = "!#acl 1\nuser:88DF566D-A38D-498B-8DFA-666E8E997CCD:you:502:allow:append\n";
    assert!(acl_text_grants_write(other, owner));
    let group = "!#acl 1\ngroup:ABCDEFAB-CDEF-ABCD-EFAB-CDEF00000014:staff:20:allow:delete_child\n";
    assert!(acl_text_grants_write(group, owner));
    let colon = "!#acl 1\nuser:88DF566D-A38D-498B-8DFA-666E8E997CCD:odd:name:502:allow:write\n";
    assert!(acl_text_grants_write(colon, owner));
    // Inheritance flags ride on the rule field (security review, 2026-11-18).
    let inherit = "!#acl 1\ngroup:ABCDEFAB-CDEF-ABCD-EFAB-CDEF00000014:staff:20:allow,file_inherit,directory_inherit:write\n";
    assert!(acl_text_grants_write(inherit, owner));
    let inherited = "!#acl 1\ngroup:ABCDEFAB-CDEF-ABCD-EFAB-CDEF00000014:staff:20:allow,inherited,file_inherit:delete_child\n";
    assert!(acl_text_grants_write(inherited, owner));
    let deny_inherit =
        "!#acl 1\ngroup:ABCDEFAB-CDEF-ABCD-EFAB-CDEF0000000C:everyone:12:deny,file_inherit:write\n";
    assert!(!acl_text_grants_write(deny_inherit, owner));
    // Permissions that lead to write, and unknown names, refuse.
    for permission in [
        "writesecurity",
        "chown",
        "writeattr",
        "writeextattr",
        "delete",
        "something_new",
    ] {
        let line = format!(
            "!#acl 1\ngroup:ABCDEFAB-CDEF-ABCD-EFAB-CDEF00000014:staff:20:allow:readattr,{permission}\n"
        );
        assert!(acl_text_grants_write(&line, owner), "{permission}");
    }
    let read_only = "!#acl 1\ngroup:ABCDEFAB-CDEF-ABCD-EFAB-CDEF00000014:staff:20:allow:read,list,search,readattr,readextattr,readsecurity\n";
    assert!(!acl_text_grants_write(read_only, owner));
    assert!(
        acl_text_grants_write("!#acl 1\ngarbage\n", owner),
        "unreadable entries refuse"
    );
}

/// Independent review 2026-11-18: only `env [-S] <bare name>` is accepted.
/// Any other `env` form could run a program that was never checked.
#[test]
fn only_plain_env_lines_are_accepted() {
    let temp = Temp::new();
    let bin = temp.0.join("bin");
    for (line, accepted) in [
        ("#!/usr/bin/env node", true),
        ("#!/usr/bin/env -S node --no-warnings", true),
        ("#!/usr/bin/env python3", true),
        ("#!/usr/bin/env /tmp/evil/node", false),
        ("#!/usr/bin/env -P /tmp/evil node", false),
        ("#!/usr/bin/env PATH=/tmp/evil node", false),
        ("#!/usr/bin/env -u FOO node", false),
        ("#!/usr/bin/env -i node", false),
        ("#!/usr/bin/env", false),
    ] {
        executable(&bin.join("claude"), &format!("{line}\n"));
        match launcher(&bin.join("claude")) {
            Launcher::Env(_) => assert!(accepted, "{line}"),
            Launcher::Refused(_) => {
                assert!(!accepted, "{line}");
                let refusal =
                    agent_path(&plan(true), &bin.join("claude"), Some(&bin), &[]).unwrap_err();
                assert_eq!(refusal.code, "AGENT_INTERPRETER_UNTRUSTED", "{line}");
            }
            other => panic!("{line}: {other:?}"),
        }
    }
}

/// The usual way an access list reaches a file: inherited from its folder.
#[test]
fn a_file_with_an_inherited_access_list_is_refused() {
    let temp = Temp::new();
    let bin = temp.0.join("bin");
    fs::create_dir(&bin).unwrap();
    chmod_acl(
        "everyone allow add_file,delete_child,write,file_inherit,directory_inherit",
        &bin,
    );
    executable(&bin.join("claude"), "#!/bin/sh\n");
    let listing = std::process::Command::new("/bin/ls")
        .args(["-le"])
        .arg(bin.join("claude"))
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&listing.stdout).contains("inherited"),
        "the file inherited the entry"
    );
    // Remove the folder's own list: only the file's inherited entry remains.
    let status = std::process::Command::new("/bin/chmod")
        .arg("-N")
        .arg(&bin)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(lookup_outcome(&bin), Outcome::Acl);
}
