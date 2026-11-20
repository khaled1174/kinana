use kinana_core::{ScanStatus, SkillRecord, SourceInfo, hash, scan};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TREE: AtomicU64 = AtomicU64::new(0);

struct Tree(PathBuf);

impl Tree {
    fn new() -> Self {
        // macOS commonly exposes the system temp directory through /var, a
        // symlink. Tests use its physical location so the scanner can retain
        // strict rejection of links in explicitly supplied path components.
        let path = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "kinana-scan-{}-{}",
            std::process::id(),
            NEXT_TREE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn skill(&self, relative: &str, name: &str, description: &str) -> PathBuf {
        let path = self.0.join(relative);
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\nInstructions.\n"),
        )
        .unwrap();
        path
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn record(name: &str, content_hash: String) -> SkillRecord {
    SkillRecord {
        stack: vec![],
        stage: vec![],
        when_to_use: String::new(),
        pinned: false,
        id: format!("vault-{name}"),
        name: name.into(),
        description: "Known skill".into(),
        category: "other".into(),
        audit_level: "unscanned".into(),
        content_hash,
        source: SourceInfo::default(),
        added_at: "2026-11-13T00:00:00Z".into(),
        role: String::new(),
        tags: Vec::new(),
    }
}

fn snapshot(path: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut entries = fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    entries.sort();
    let mut result = Vec::new();
    for entry in entries {
        if entry.is_dir() {
            result.extend(snapshot(&entry));
        } else {
            result.push((entry.clone(), fs::read(entry).unwrap()));
        }
    }
    result
}

#[test]
fn scan_counts_all_four_states_and_leaves_source_files_unchanged() {
    let tree = Tree::new();
    tree.skill("a/.claude/skills/review", "review", "Original review.");
    tree.skill("b/.agents/skills/review", "review", "Original review.");
    tree.skill("c/.codex/skills/review", "review", "Changed review.");
    tree.skill("c/.codex/skills/build", "build", "Build a project.");
    let broken = tree.0.join("c/.codex/skills/broken");
    fs::create_dir_all(&broken).unwrap();
    fs::write(broken.join("SKILL.md"), "---\nname: broken\n---\n").unwrap();
    let invalid = tree.0.join("c/.codex/skills/invalid");
    fs::create_dir_all(&invalid).unwrap();
    fs::write(invalid.join("SKILL.md"), "---\nname: [\n---\n").unwrap();
    let before = snapshot(&tree.0);

    let report = scan::scan(std::slice::from_ref(&tree.0), &[]).unwrap();

    assert_eq!(report.counts.new, 2);
    assert_eq!(report.counts.duplicate, 1);
    assert_eq!(report.counts.variant, 1);
    assert_eq!(report.counts.broken, 2);
    assert_eq!(report.entries.len(), 6);
    assert_eq!(before, snapshot(&tree.0));
}

#[test]
fn known_hash_takes_precedence_over_name_and_known_name_marks_variant() {
    let tree = Tree::new();
    let duplicate = tree.skill("project/.claude/skills/duplicate", "copy", "Same bytes.");
    tree.skill("project/.claude/skills/variant", "review", "Changed bytes.");
    let known = vec![
        record("different-name", hash::hash_directory(&duplicate).unwrap()),
        record("review", "blake3:not-the-same".into()),
    ];

    let report = scan::scan(std::slice::from_ref(&tree.0), &known).unwrap();

    assert_eq!(report.entries[0].status, ScanStatus::Duplicate);
    assert_eq!(
        report.entries[0].skill_id.as_deref(),
        Some("vault-different-name")
    );
    assert_eq!(report.entries[1].status, ScanStatus::Variant);
    assert_eq!(report.entries[1].skill_id.as_deref(), Some("vault-review"));
}

#[test]
fn discovery_stays_inside_agent_skill_directories_and_explicit_roots() {
    let tree = Tree::new();
    let included = tree.skill("project/.claude/skills/included", "included", "Included.");
    for directory in [
        "loose",
        "project/node_modules/p/.agents/skills/ignored",
        "project/target/p/.agents/skills/ignored",
        "project/dist/p/.agents/skills/ignored",
        "project/Library/p/.agents/skills/ignored",
        "project/.private/p/.agents/skills/ignored",
        "project/plugins/p/.agents/skills/ignored",
        "project/.claude/plugins/p/skills/ignored",
    ] {
        tree.skill(directory, "ignored", "Ignored.");
    }

    let roots = vec![tree.0.clone(), tree.0.join("project"), included.clone()];
    let report = scan::scan(&roots, &[]).unwrap();
    assert_eq!(report.entries.len(), 1);
    assert_eq!(report.entries[0].path, included);

    let direct = scan::scan(&[tree.0.join("loose")], &[]).unwrap();
    assert_eq!(direct.entries.len(), 1);
    let container = scan::scan(&[tree.0.join("project/.claude/skills")], &[]).unwrap();
    assert_eq!(container.entries.len(), 1);
}

#[test]
fn root_order_does_not_change_classification() {
    let tree = Tree::new();
    let a = tree.skill("a/.agents/skills/review", "review", "Same.");
    let b = tree.skill("b/.codex/skills/review", "review", "Same.");
    let forward = scan::scan(&[a.clone(), b.clone()], &[]).unwrap();
    let backward = scan::scan(&[b, a], &[]).unwrap();
    let states = |report: &kinana_core::ScanReport| {
        report
            .entries
            .iter()
            .map(|entry| (entry.path.clone(), entry.status))
            .collect::<Vec<_>>()
    };
    assert_eq!(states(&forward), states(&backward));
    assert_eq!(forward.entries[0].status, ScanStatus::New);
    assert_eq!(forward.entries[1].status, ScanStatus::Duplicate);
}

#[test]
fn invalid_roots_are_errors_and_an_empty_scan_does_not_default_to_home() {
    let tree = Tree::new();
    assert!(scan::scan(&[], &[]).is_err());
    assert!(scan::scan(&[tree.0.join("missing")], &[]).is_err());
    let file = tree.0.join("file");
    fs::write(&file, "not a directory").unwrap();
    assert!(scan::scan(&[file], &[]).is_err());
}

#[test]
fn hash_is_independent_of_creation_order_and_root_name() {
    let tree = Tree::new();
    let first = tree.0.join("first");
    let second = tree.0.join("second");
    fs::create_dir_all(first.join("references")).unwrap();
    fs::create_dir_all(second.join("references")).unwrap();
    fs::write(first.join("a"), "one").unwrap();
    fs::write(first.join("references/b"), "two").unwrap();
    fs::write(second.join("references/b"), "two").unwrap();
    fs::write(second.join("a"), "one").unwrap();

    let digest = hash::hash_directory(&first).unwrap();
    assert!(digest.starts_with("blake3:"));
    assert_eq!(digest.len(), "blake3:".len() + 64);
    assert_eq!(digest, hash::hash_directory(&second).unwrap());
    fs::rename(second.join("a"), second.join("renamed")).unwrap();
    assert_ne!(digest, hash::hash_directory(&second).unwrap());
}

#[test]
fn hash_framing_distinguishes_ambiguous_names_contents_and_empty_directories() {
    let tree = Tree::new();
    let first = tree.0.join("first");
    let second = tree.0.join("second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    fs::write(first.join("a"), "bc").unwrap();
    fs::write(second.join("ab"), "c").unwrap();
    assert_ne!(
        hash::hash_directory(&first).unwrap(),
        hash::hash_directory(&second).unwrap()
    );
    fs::remove_file(second.join("ab")).unwrap();
    fs::write(second.join("a"), "bc").unwrap();
    assert_eq!(
        hash::hash_directory(&first).unwrap(),
        hash::hash_directory(&second).unwrap()
    );
    fs::create_dir(second.join("empty")).unwrap();
    assert_ne!(
        hash::hash_directory(&first).unwrap(),
        hash::hash_directory(&second).unwrap()
    );
}

#[cfg(unix)]
#[test]
fn links_are_rejected_without_following_root_components_or_skill_contents() {
    use std::os::unix::fs::symlink;

    let tree = Tree::new();
    let outside = Tree::new();
    let external_skill = outside.skill("secret", "secret", "Outside root.");
    let regular = tree.skill("project/.agents/skills/regular", "regular", "Regular.");
    let linked = tree.0.join("project/.agents/skills/linked");
    symlink(&external_skill, &linked).unwrap();
    symlink(&outside.0, tree.0.join("shortcut")).unwrap();
    symlink(outside.0.join("not-present"), tree.0.join("dangling")).unwrap();
    symlink(
        external_skill.join("SKILL.md"),
        regular.join("reference.md"),
    )
    .unwrap();

    let report = scan::scan(std::slice::from_ref(&tree.0), &[]).unwrap();
    assert_eq!(report.entries.len(), 1);
    assert_eq!(report.entries[0].status, ScanStatus::Broken);
    assert!(
        report.entries[0]
            .error
            .as_deref()
            .unwrap()
            .contains("symbolic link")
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("linked"))
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("dangling"))
    );
    assert!(
        hash::hash_directory(&regular)
            .unwrap_err()
            .message
            .contains("reference.md")
    );
    assert!(hash::hash_directory(&linked).is_err());
    assert!(scan::scan(&[tree.0.join("shortcut/secret")], &[]).is_err());
    assert!(hash::hash_directory(&tree.0.join("shortcut/secret")).is_err());
}

#[cfg(unix)]
#[test]
fn a_linked_manifest_is_reported_as_broken() {
    use std::os::unix::fs::symlink;

    let tree = Tree::new();
    let outside = Tree::new();
    let external_skill = outside.skill("secret", "secret", "Outside root.");
    let directory = tree.0.join("project/.claude/skills/linked-manifest");
    fs::create_dir_all(&directory).unwrap();
    symlink(external_skill.join("SKILL.md"), directory.join("SKILL.md")).unwrap();
    let report = scan::scan(std::slice::from_ref(&tree.0), &[]).unwrap();
    assert_eq!(report.entries.len(), 1);
    assert_eq!(report.entries[0].status, ScanStatus::Broken);
    assert!(report.entries[0].name.is_none());
}

#[cfg(unix)]
#[test]
fn non_utf8_names_are_hashed_losslessly_and_hidden_directories_are_ignored() {
    use std::ffi::OsString;
    use std::io::{self, Write};
    use std::os::unix::ffi::OsStringExt;

    fn fixture_created(result: io::Result<()>) -> bool {
        // Darwin reports EILSEQ for these names; ErrorKind leaves it uncategorized.
        const DARWIN_EILSEQ: i32 = 92;
        match result {
            Ok(()) => true,
            Err(error)
                if error.kind() == io::ErrorKind::InvalidInput
                    || (cfg!(target_os = "macos")
                        && error.raw_os_error() == Some(DARWIN_EILSEQ)) =>
            {
                // Bypass the successful-test output capture so the reason is
                // also visible with plain `cargo test --locked`.
                writeln!(
                    io::stderr().lock(),
                    "SKIP non_utf8_names_are_hashed_losslessly_and_hidden_directories_are_ignored: \
                     filesystem rejects non-UTF-8 filenames ({error}); \
                     assertions remain enabled on filesystems that support them."
                )
                .unwrap();
                false
            }
            Err(error) => panic!("could not create non-UTF-8 fixture: {error}"),
        }
    }

    let tree = Tree::new();
    let first = tree.0.join("first");
    let second = tree.0.join("second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    if !fixture_created(fs::write(
        first.join(OsString::from_vec(vec![0x80])),
        "content",
    )) || !fixture_created(fs::write(
        second.join(OsString::from_vec(vec![0x81])),
        "content",
    )) {
        return;
    }
    assert_ne!(
        hash::hash_directory(&first).unwrap(),
        hash::hash_directory(&second).unwrap()
    );

    let hidden = tree.0.join(OsString::from_vec(b".hidden-\x80".to_vec()));
    let skill = hidden.join(".agents/skills/ignored");
    if !fixture_created(fs::create_dir_all(&skill)) {
        return;
    }
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: ignored\ndescription: Hidden.\n---\n",
    )
    .unwrap();
    let report = scan::scan(std::slice::from_ref(&tree.0), &[]).unwrap();
    assert!(report.entries.is_empty());
}
