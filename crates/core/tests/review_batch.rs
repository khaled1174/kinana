//! Batch accept of pending descriptions (0.3, batch 1): one preview bound by a
//! token, one commit for the whole selection, and one `undo` that reverts it.
use kinana_core::{
    ClassificationProposal, ImportOptions, Result, build, edit, frontmatter, gitlog, import,
    review, vault::Vault,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
const IDS: [&str; 3] = ["batch-alpha", "batch-beta", "batch-gamma"];

struct Temp(PathBuf);
impl Temp {
    fn new() -> Result<Self> {
        let root = std::env::temp_dir().canonicalize()?.join(format!(
            "kinana-review-batch-{}-{}-{}",
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
        for id in IDS {
            let source = self.0.join("sources").join(id);
            fs::create_dir_all(&source)?;
            fs::write(
                source.join("SKILL.md"),
                format!("---\nname: {id}\ndescription: Original text of {id}.\n---\n\n# {id}\n"),
            )?;
            import::add(
                &vault,
                &source.to_string_lossy(),
                &self.0.join("state"),
                &ImportOptions::default(),
            )?;
        }
        build::build(&vault, &[], false)?;
        Ok(vault)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _cleanup = fs::remove_dir_all(&self.0);
    }
}

fn proposed(id: &str) -> String {
    format!("Use {id} before changing the API contract.")
}

fn save_proposals(vault: &Vault, ids: &[&str]) -> Result<()> {
    let mut updates = BTreeMap::new();
    for id in ids {
        let mut metadata = edit::read_metadata(vault, id)?;
        metadata.proposal = Some(ClassificationProposal {
            category: "backend-api".into(),
            stack: vec![],
            stage: vec![],
            tags: vec!["rest".into()],
            role: "Review API contracts".into(),
            when_to_use: "Before implementing endpoints".into(),
            when_not: "For deployment changes".into(),
            improved_description: proposed(id),
            overlaps_with: Vec::new(),
            confidence: 0.9,
            category_reason: String::new(),
        });
        updates.insert(id.to_string(), metadata);
    }
    edit::apply(vault, &updates, "Save pending batch fixtures", false)?;
    Ok(())
}

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| v.to_string()).collect()
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
            if metadata.is_dir() {
                if relative.starts_with(".git") {
                    continue;
                }
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

fn commit_count(vault: &Vault) -> Result<usize> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(&vault.root)
        .args(["rev-list", "--count", "HEAD"])
        .output()?;
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .unwrap_or(0))
}

fn dist_description(vault: &Vault, id: &str) -> Result<String> {
    Ok(frontmatter::read_skill(&vault.root.join("dist").join(id).join("SKILL.md"))?.description)
}

#[test]
fn preview_writes_nothing_then_the_confirmed_batch_is_one_commit_and_one_undo() -> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.vault()?;
    save_proposals(&vault, &IDS)?;
    let selected = ids(&["batch-gamma", "batch-alpha"]);
    let before = snapshot(&vault.root)?;
    let head = gitlog::head(&vault.root)?;
    let commits = commit_count(&vault)?;

    let preview = review::accept_many(&vault, &selected, None, false)?;
    assert!(preview.requires_confirmation);
    assert!(!preview.committed);
    assert_eq!(preview.decision, "accept");
    assert!(preview.preview_token.starts_with("r1."));
    assert_eq!(
        preview
            .items
            .iter()
            .map(|i| i.id.as_str())
            .collect::<Vec<_>>(),
        ["batch-gamma", "batch-alpha"]
    );
    assert_eq!(preview.items[0].proposed, proposed("batch-gamma"));
    assert_eq!(preview.items[0].current, "Original text of batch-gamma.");
    assert_eq!(preview.items[0].active, "original");
    assert_eq!(snapshot(&vault.root)?, before);
    assert_eq!(gitlog::head(&vault.root)?, head);

    // The token does not depend on the order of the selection.
    let reordered = review::accept_many(&vault, &ids(&["batch-alpha", "batch-gamma"]), None, true)?;
    assert_eq!(reordered.preview_token, preview.preview_token);

    let dry_run = review::accept_many(&vault, &selected, Some(&preview.preview_token), true)?;
    assert!(!dry_run.requires_confirmation);
    assert!(!dry_run.committed);
    assert_eq!(snapshot(&vault.root)?, before);

    let report = review::accept_many(&vault, &selected, Some(&preview.preview_token), false)?;
    assert!(report.committed);
    assert!(!report.requires_confirmation);
    assert!(report.items.iter().all(|item| item.active == "improved"));
    assert_eq!(commit_count(&vault)?, commits + 1);
    for id in ["batch-alpha", "batch-gamma"] {
        let metadata = edit::read_metadata(&vault, id)?;
        assert_eq!(metadata.description.active, "improved");
        assert_eq!(
            metadata.description.improved.as_deref(),
            Some(proposed(id).as_str())
        );
        assert!(metadata.proposal.is_none());
        assert!(metadata.description.approved_at.is_some());
        assert_eq!(dist_description(&vault, id)?, proposed(id));
    }
    // The skill left out of the selection keeps its pending proposal.
    let untouched = edit::read_metadata(&vault, "batch-beta")?;
    assert!(untouched.proposal.is_some());
    assert_eq!(untouched.description.active, "original");
    let message = std::process::Command::new("git")
        .arg("-C")
        .arg(&vault.root)
        .args(["log", "-1", "--format=%s"])
        .output()?;
    assert_eq!(
        String::from_utf8_lossy(&message.stdout).trim(),
        "Accept descriptions for 2 skills: batch-gamma, batch-alpha"
    );
    gitlog::ensure_clean(&vault.root)?;
    let accepted = gitlog::head(&vault.root)?;

    let undo = vault.undo(false)?;
    assert_eq!(undo.reverted_commit, accepted);
    assert_eq!(commit_count(&vault)?, commits + 2);
    assert_eq!(snapshot(&vault.root)?, before);
    Ok(())
}

#[test]
fn one_invalid_selection_stops_the_whole_batch_before_any_write() -> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.vault()?;
    save_proposals(&vault, &["batch-alpha", "batch-gamma"])?;
    let before = snapshot(&vault.root)?;
    let head = gitlog::head(&vault.root)?;

    let missing = review::accept_many(&vault, &ids(&["batch-alpha", "batch-beta"]), None, false)
        .expect_err("a skill without a proposal stops the batch");
    assert_eq!(missing.code, "NO_DESCRIPTION_PROPOSAL");
    assert!(missing.needs_decision);
    assert!(missing.message.contains("batch-beta"));

    let duplicate = review::accept_many(&vault, &ids(&["batch-alpha", "batch-alpha"]), None, false)
        .expect_err("duplicates are refused");
    assert_eq!(duplicate.code, "INVALID_ARGUMENTS");
    let empty = review::accept_many(&vault, &[], None, false).expect_err("empty");
    assert_eq!(empty.code, "INVALID_ARGUMENTS");
    let unknown = review::accept_many(&vault, &ids(&["batch-alpha", "../x"]), None, false)
        .expect_err("unsafe id");
    assert_ne!(unknown.code, "");

    // A proposal made for the previous source is reviewed on its own.
    let mut metadata = edit::read_metadata(&vault, "batch-gamma")?;
    metadata.proposal_source_changed = Some(true);
    edit::apply(
        &vault,
        &BTreeMap::from([("batch-gamma".to_string(), metadata)]),
        "Mark the source as changed",
        false,
    )?;
    let changed_before = snapshot(&vault.root)?;
    let changed = review::accept_many(&vault, &ids(&["batch-alpha", "batch-gamma"]), None, false)
        .expect_err("source changed");
    assert_eq!(changed.code, "PROPOSAL_SOURCE_CHANGED");
    assert_eq!(snapshot(&vault.root)?, changed_before);
    assert_ne!(gitlog::head(&vault.root)?, head);
    let _ = before;
    Ok(())
}

#[test]
fn a_stale_token_is_refused_without_writes() -> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.vault()?;
    save_proposals(&vault, &IDS)?;
    let selected = ids(&IDS);
    let preview = review::accept_many(&vault, &selected, None, false)?;

    // Another proposal arrives for one selected skill after the preview.
    let mut metadata = edit::read_metadata(&vault, "batch-beta")?;
    if let Some(proposal) = metadata.proposal.as_mut() {
        proposal.improved_description = "A different, newer proposal.".into();
    }
    edit::apply(
        &vault,
        &BTreeMap::from([("batch-beta".to_string(), metadata)]),
        "Replace one pending proposal",
        false,
    )?;
    let before = snapshot(&vault.root)?;
    let head = gitlog::head(&vault.root)?;
    let stale = review::accept_many(&vault, &selected, Some(&preview.preview_token), false)
        .expect_err("the preview no longer matches");
    assert_eq!(stale.code, "REVIEW_PREVIEW_CHANGED");
    assert!(stale.needs_decision);
    assert_eq!(snapshot(&vault.root)?, before);
    assert_eq!(gitlog::head(&vault.root)?, head);

    let wrong =
        review::accept_many(&vault, &selected, Some("r1.0000"), false).expect_err("wrong token");
    assert_eq!(wrong.code, "REVIEW_PREVIEW_CHANGED");
    assert_eq!(snapshot(&vault.root)?, before);
    Ok(())
}

#[test]
fn a_selection_above_the_limit_is_refused() -> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.vault()?;
    let many = (0..=review::BATCH_REVIEW_LIMIT)
        .map(|i| format!("skill-{i}"))
        .collect::<Vec<_>>();
    let error = review::accept_many(&vault, &many, None, false).expect_err("too many");
    assert_eq!(error.code, "INVALID_ARGUMENTS");
    Ok(())
}

#[test]
fn the_pending_list_is_read_only_and_marks_proposals_for_a_previous_source() -> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.vault()?;
    assert!(review::pending(&vault)?.is_empty());
    save_proposals(&vault, &["batch-alpha", "batch-gamma"])?;
    let mut metadata = edit::read_metadata(&vault, "batch-gamma")?;
    metadata.proposal_source_changed = Some(true);
    edit::apply(
        &vault,
        &BTreeMap::from([("batch-gamma".to_string(), metadata)]),
        "Mark the source as changed",
        false,
    )?;
    let before = snapshot(&vault.root)?;
    let head = gitlog::head(&vault.root)?;
    let pending = review::pending(&vault)?;
    assert_eq!(
        pending
            .iter()
            .map(|item| (item.id.as_str(), item.source_changed))
            .collect::<Vec<_>>(),
        [("batch-alpha", false), ("batch-gamma", true)]
    );
    assert_eq!(pending[0].current, "Original text of batch-alpha.");
    assert_eq!(pending[0].proposed, proposed("batch-alpha"));
    assert_eq!(snapshot(&vault.root)?, before);
    assert_eq!(gitlog::head(&vault.root)?, head);
    Ok(())
}
