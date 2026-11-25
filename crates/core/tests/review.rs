use kinana_core::{
    ClassificationProposal, ImportOptions, Result, ReviewDecision, build, edit, frontmatter,
    gitlog, import, review, vault::Vault,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
const ID: &str = "review-fixture";
const ORIGINAL: &str = "Original API description.";

struct Temp(PathBuf);
impl Temp {
    fn new() -> Result<Self> {
        let root = std::env::temp_dir().canonicalize()?.join(format!(
            "kinana-review-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| std::io::Error::other(error.to_string()))?
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&root)?;
        Ok(Self(root))
    }

    fn vault(&self) -> Result<Vault> {
        let vault = Vault::new(self.0.join("vault"), self.0.join("cache/index.db"));
        vault.init(false)?;
        let source = self.0.join("source");
        fs::create_dir(&source)?;
        fs::create_dir_all(source.join("assets/empty"))?;
        fs::write(
            source.join("SKILL.md"),
            format!(
                "---\r\nname: {ID}\r\ndescription: {ORIGINAL}\r\ncustom: [one, two]\r\n---\r\n\r\n# Exact source\r\nUnicode: مرحبا\r\nno trailing newline"
            ),
        )?;
        fs::write(source.join("assets/bytes.bin"), [0, 255, 13, 10, 42])?;
        import::add(
            &vault,
            &source.to_string_lossy(),
            &self.0.join("state"),
            &ImportOptions::default(),
        )?;
        build::build(&vault, &[], false)?;
        Ok(vault)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _cleanup = fs::remove_dir_all(&self.0);
    }
}

fn proposal(text: &str) -> ClassificationProposal {
    ClassificationProposal {
        category: "backend-api".into(),
        stack: vec![],
        stage: vec![],
        tags: vec!["rest".into(), "review".into()],
        role: "Review API contracts".into(),
        when_to_use: "Before implementing endpoints".into(),
        when_not: "For deployment changes".into(),
        improved_description: text.into(),
        overlaps_with: Vec::new(),
        confidence: 0.9,
        category_reason: String::new(),
    }
}

fn save_proposal(vault: &Vault, text: &str) -> Result<()> {
    let mut metadata = edit::read_metadata(vault, ID)?;
    metadata.proposal = Some(proposal(text));
    edit::apply(
        vault,
        &BTreeMap::from([(ID.to_string(), metadata)]),
        "Save a pending review fixture",
        false,
    )?;
    Ok(())
}

fn metadata_path(vault: &Vault) -> PathBuf {
    vault.root.join("meta").join(format!("{ID}.toml"))
}

fn dist_description(vault: &Vault) -> Result<String> {
    Ok(frontmatter::read_skill(&vault.root.join("dist").join(ID).join("SKILL.md"))?.description)
}

fn snapshot(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>> {
    fn visit(root: &Path, at: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) -> Result<()> {
        for entry in fs::read_dir(at)? {
            let path = entry?.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|error| std::io::Error::other(error.to_string()))?
                .to_path_buf();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                files.insert(
                    relative,
                    fs::read_link(&path)?
                        .as_os_str()
                        .as_encoded_bytes()
                        .to_vec(),
                );
            } else if metadata.is_dir() {
                files.insert(relative, b"directory".to_vec());
                visit(root, &path, files)?;
            } else {
                files.insert(relative, fs::read(path)?);
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files)?;
    Ok(files)
}

fn without_git(files: BTreeMap<PathBuf, Vec<u8>>) -> BTreeMap<PathBuf, Vec<u8>> {
    files
        .into_iter()
        .filter(|(path, _)| !path.starts_with("vault/.git"))
        .collect()
}

fn metadata_value(vault: &Vault) -> Result<toml::Value> {
    Ok(toml::from_str(&fs::read_to_string(metadata_path(vault))?)?)
}

#[test]
fn preview_and_accept_dry_run_are_read_only_and_accept_activates_only_the_reviewed_text()
-> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.vault()?;
    let text = "Before implementing REST endpoints, review the API contract.";
    save_proposal(&vault, text)?;
    let source_before = snapshot(&vault.root.join("skills"))?;
    let before = snapshot(&temp.0)?;
    let head = gitlog::head(&vault.root)?;
    let preview = review::review(&vault, ID, ReviewDecision::Preview, false)?;
    assert_eq!(preview.id, ID);
    assert_eq!(preview.original, ORIGINAL);
    assert_eq!(preview.current, ORIGINAL);
    assert_eq!(preview.proposed.as_deref(), Some(text));
    assert_eq!(preview.active, "original");
    assert_eq!(preview.decision, "preview");
    assert!(!preview.committed);
    let dry_run = review::review(&vault, ID, ReviewDecision::Accept, true)?;
    assert!(dry_run.dry_run);
    assert_eq!(dry_run.active, "improved");
    assert!(!dry_run.committed);
    assert_eq!(dry_run.build.items[0].description_source, "improved");
    assert_eq!(snapshot(&temp.0)?, before);
    assert_eq!(gitlog::head(&vault.root)?, head);

    let classification_before = metadata_value(&vault)?["classification"].clone();
    let report = review::review(&vault, ID, ReviewDecision::Accept, false)?;
    assert!(report.committed);
    assert_eq!(report.active, "improved");
    assert_eq!(report.current, ORIGINAL);
    assert_eq!(report.proposed.as_deref(), Some(text));
    let metadata = edit::read_metadata(&vault, ID)?;
    assert_eq!(metadata.description.original, ORIGINAL);
    assert_eq!(metadata.description.improved.as_deref(), Some(text));
    assert_eq!(metadata.description.active, "improved");
    let approved_at = metadata
        .description
        .approved_at
        .expect("accept records an approval");
    assert!(approved_at.contains('T') && approved_at.ends_with('Z'));
    assert!(metadata.proposal.is_none());
    assert_eq!(
        metadata_value(&vault)?["classification"],
        classification_before
    );
    assert_eq!(vault.show(ID)?.skill.description, text);
    assert_eq!(dist_description(&vault)?, text);
    assert_eq!(snapshot(&vault.root.join("skills"))?, source_before);
    assert_ne!(gitlog::head(&vault.root)?, head);
    gitlog::ensure_clean(&vault.root)?;
    Ok(())
}

#[test]
fn new_pending_text_and_rejection_preserve_the_previous_approval_then_original_restores_source()
-> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.vault()?;
    let old_text = "Before changing contracts, inspect the API design.";
    let new_text = "Before adding endpoints, validate the revised REST contract.";
    save_proposal(&vault, old_text)?;
    review::review(&vault, ID, ReviewDecision::Accept, false)?;
    let approved = edit::read_metadata(&vault, ID)?.description;
    let source_before = snapshot(&vault.root.join("skills"))?;
    let dist_before = snapshot(&vault.root.join("dist"))?;
    save_proposal(&vault, new_text)?;
    let pending = edit::read_metadata(&vault, ID)?;
    assert_eq!(pending.description.improved, approved.improved);
    assert_eq!(pending.description.approved_at, approved.approved_at);
    assert_eq!(pending.description.active, "improved");
    assert_eq!(
        pending
            .proposal
            .as_ref()
            .map(|value| value.improved_description.as_str()),
        Some(new_text)
    );
    assert_eq!(snapshot(&vault.root.join("dist"))?, dist_before);
    let preview = review::review(&vault, ID, ReviewDecision::Preview, false)?;
    assert_eq!(preview.current, old_text);
    assert_eq!(preview.proposed.as_deref(), Some(new_text));

    let before = snapshot(&temp.0)?;
    let dry_run = review::review(&vault, ID, ReviewDecision::Reject, true)?;
    assert_eq!(dry_run.active, "improved");
    assert!(!dry_run.committed);
    assert_eq!(snapshot(&temp.0)?, before);
    let rejected = review::review(&vault, ID, ReviewDecision::Reject, false)?;
    assert!(rejected.committed);
    let metadata = edit::read_metadata(&vault, ID)?;
    assert!(metadata.proposal.is_none());
    assert_eq!(metadata.description.improved, approved.improved);
    assert_eq!(metadata.description.approved_at, approved.approved_at);
    assert_eq!(metadata.description.active, "improved");
    assert_eq!(dist_description(&vault)?, old_text);

    save_proposal(&vault, new_text)?;
    let before = snapshot(&temp.0)?;
    let original_preview = review::review(&vault, ID, ReviewDecision::Original, true)?;
    assert_eq!(original_preview.active, "original");
    assert_eq!(snapshot(&temp.0)?, before);
    let original = review::review(&vault, ID, ReviewDecision::Original, false)?;
    assert!(original.committed);
    assert_eq!(original.active, "original");
    let metadata = edit::read_metadata(&vault, ID)?;
    assert_eq!(metadata.description.improved, approved.improved);
    assert_eq!(metadata.description.approved_at, approved.approved_at);
    assert!(metadata.proposal.is_some());
    assert_eq!(dist_description(&vault)?, ORIGINAL);
    assert_eq!(
        snapshot(&vault.root.join("dist").join(ID))?,
        snapshot(&vault.root.join("skills").join(ID))?
    );
    review::review(&vault, ID, ReviewDecision::Accept, false)?;
    assert_eq!(dist_description(&vault)?, new_text);
    assert_eq!(snapshot(&vault.root.join("skills"))?, source_before);
    gitlog::ensure_clean(&vault.root)?;
    Ok(())
}

#[test]
fn accepting_or_rejecting_without_a_pending_proposal_requires_a_decision_without_writes()
-> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.vault()?;
    let before = snapshot(&temp.0)?;
    for decision in [ReviewDecision::Accept, ReviewDecision::Reject] {
        for dry_run in [false, true] {
            let error = review::review(&vault, ID, decision, dry_run).unwrap_err();
            assert_eq!(error.code, "NO_DESCRIPTION_PROPOSAL");
            assert!(error.needs_decision);
        }
    }
    assert_eq!(snapshot(&temp.0)?, before);
    Ok(())
}

#[test]
fn review_preserves_unknown_fields_and_source_audit_hash_and_empty_directories() -> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.vault()?;
    save_proposal(
        &vault,
        "Before implementation, review the REST API contract.",
    )?;
    let typed_before = edit::read_metadata(&vault, ID)?;
    let mut value = metadata_value(&vault)?;
    value.as_table_mut().unwrap().insert(
        "custom_root".into(),
        toml::Value::String("retain root data".into()),
    );
    for table in ["source", "classification", "description", "audit"] {
        value[table].as_table_mut().unwrap().insert(
            "custom_nested".into(),
            toml::Value::Array(vec![
                toml::Value::String(table.into()),
                toml::Value::Integer(7),
            ]),
        );
    }
    value.as_table_mut().unwrap().insert(
        "invocation".into(),
        toml::Value::Table(toml::map::Map::from_iter([(
            "mode".into(),
            toml::Value::String("manual".into()),
        )])),
    );
    fs::write(metadata_path(&vault), toml::to_string_pretty(&value)?)?;
    gitlog::commit(&vault.root, "Record unknown metadata fixture")?;
    review::review(&vault, ID, ReviewDecision::Accept, false)?;
    review::review(&vault, ID, ReviewDecision::Original, false)?;
    let after = metadata_value(&vault)?;
    assert_eq!(after["custom_root"], value["custom_root"]);
    assert_eq!(after["invocation"], value["invocation"]);
    for table in ["source", "classification", "description", "audit"] {
        assert_eq!(after[table]["custom_nested"], value[table]["custom_nested"]);
    }
    let typed_after = edit::read_metadata(&vault, ID)?;
    assert_eq!(
        toml::to_string(&typed_after.source)?,
        toml::to_string(&typed_before.source)?
    );
    assert_eq!(
        toml::to_string(&typed_after.audit)?,
        toml::to_string(&typed_before.audit)?
    );
    assert_eq!(typed_after.content_hash, typed_before.content_hash);
    assert_eq!(typed_after.added_at, typed_before.added_at);
    assert_eq!(
        typed_after.empty_directories,
        typed_before.empty_directories
    );
    gitlog::ensure_clean(&vault.root)?;
    Ok(())
}

#[test]
fn stale_metadata_snapshot_cannot_overwrite_a_new_committed_review_or_classification() -> Result<()>
{
    let temp = Temp::new()?;
    let vault = temp.vault()?;
    save_proposal(&vault, "Before implementation, review the API contract.")?;
    let (mut stale, bytes) = edit::read_snapshot(&vault, ID)?;
    review::review(&vault, ID, ReviewDecision::Accept, false)?;
    let concurrent = edit::read_metadata(&vault, ID)?;
    stale.classification.role = "Stale classification result".into();
    let before = snapshot(&temp.0)?;
    let head = gitlog::head(&vault.root)?;
    for dry_run in [false, true] {
        let error = edit::apply_checked(
            &vault,
            &BTreeMap::from([(ID.to_string(), stale.clone())]),
            &BTreeMap::from([(ID.to_string(), bytes.clone())]),
            "Attempt stale classification",
            dry_run,
        )
        .unwrap_err();
        assert_eq!(error.code, "METADATA_CHANGED");
        assert!(error.needs_decision);
    }
    assert_eq!(snapshot(&temp.0)?, before);
    assert_eq!(gitlog::head(&vault.root)?, head);
    let after = edit::read_metadata(&vault, ID)?;
    assert_eq!(after.description.active, "improved");
    assert_eq!(after.description.improved, concurrent.description.improved);
    assert!(after.proposal.is_none());
    gitlog::ensure_clean(&vault.root)?;
    Ok(())
}

#[cfg(unix)]
fn executable_git() -> Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let search = std::env::var_os("PATH").ok_or_else(|| std::io::Error::other("PATH missing"))?;
    for directory in std::env::split_paths(&search).filter(|path| path.is_absolute()) {
        let candidate = directory.join("git");
        if candidate.is_file() && fs::metadata(&candidate)?.permissions().mode() & 0o111 != 0 {
            return Ok(candidate.canonicalize()?);
        }
    }
    Err(std::io::Error::other("git missing").into())
}

#[cfg(unix)]
#[test]
fn failed_git_commit_restores_metadata_dist_cache_source_and_index_staging() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;
    let temp = Temp::new()?;
    let vault = temp.vault()?;
    save_proposal(&vault, "Before implementation, review the API contract.")?;
    fs::set_permissions(metadata_path(&vault), fs::Permissions::from_mode(0o444))?;
    let binary = temp.0.join("trusted-test-bin");
    fs::create_dir(&binary)?;
    fs::write(
        binary.join("git"),
        r#"#!/bin/sh
set -eu
for argument do
    if test "$argument" = commit; then
        printf 'injected review commit failure\n' >&2
        exit 89
    fi
done
exec "$REVIEW_TEST_REAL_GIT" "$@"
"#,
    )?;
    fs::set_permissions(binary.join("git"), fs::Permissions::from_mode(0o700))?;
    let search = std::env::var_os("PATH").ok_or_else(|| std::io::Error::other("PATH missing"))?;
    let directories = std::iter::once(binary).chain(std::env::split_paths(&search));
    let before = without_git(snapshot(&temp.0)?);
    let head = gitlog::head(&vault.root)?;
    let child = Command::new(std::env::current_exe()?)
        .args(["--exact", "review_commit_failure_child", "--nocapture"])
        .env("REVIEW_TEST_ROOT", &temp.0)
        .env("REVIEW_TEST_REAL_GIT", executable_git()?)
        .env(
            "PATH",
            std::env::join_paths(directories)
                .map_err(|error| std::io::Error::other(error.to_string()))?,
        )
        .output()?;
    assert!(
        child.status.success(),
        "child failed:\n{}\n{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
    assert_eq!(without_git(snapshot(&temp.0)?), before);
    assert_eq!(gitlog::head(&vault.root)?, head);
    assert_eq!(
        edit::read_metadata(&vault, ID)?.description.active,
        "original"
    );
    assert!(edit::read_metadata(&vault, ID)?.proposal.is_some());
    assert_eq!(dist_description(&vault)?, ORIGINAL);
    assert_eq!(
        fs::metadata(metadata_path(&vault))?.permissions().mode() & 0o777,
        0o444
    );
    gitlog::ensure_clean(&vault.root)?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn review_commit_failure_child() -> Result<()> {
    let Some(root) = std::env::var_os("REVIEW_TEST_ROOT") else {
        return Ok(());
    };
    let root = PathBuf::from(root);
    let vault = Vault::new(root.join("vault"), root.join("cache/index.db"));
    let error = review::review(&vault, ID, ReviewDecision::Accept, false).unwrap_err();
    assert_eq!(error.code, "GIT_ERROR");
    assert!(error.message.contains("injected review commit failure"));
    Ok(())
}
