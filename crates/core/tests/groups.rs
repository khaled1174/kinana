use kinana_core::{
    ClassificationProposal, ImportOptions, Result, edit, gitlog, groups, import, vault::Vault,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);
impl Temp {
    fn new() -> Result<Self> {
        let root = std::env::temp_dir().canonicalize()?.join(format!(
            "kinana-groups-{}-{}-{}",
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

    fn seed(&self, entries: &[(&str, &str, &str)]) -> Result<Vault> {
        let vault = Vault::new(self.0.join("vault"), self.0.join("cache/index.db"));
        vault.init(false)?;
        let collection = self.0.join("collection");
        fs::create_dir(&collection)?;
        for (id, _, _) in entries {
            let directory = collection.join(id);
            fs::create_dir(&directory)?;
            fs::write(
                directory.join("SKILL.md"),
                format!(
                    "---\nname: {id}\ndescription: Original fixture description.\n---\nSource body is data.\n"
                ),
            )?;
            fs::write(
                directory.join("never-run.sh"),
                format!(
                    "#!/bin/sh\nprintf executed > '{}'\n",
                    self.0
                        .join("GROUPS_SCRIPT_EXECUTED")
                        .to_string_lossy()
                        .replace('\'', "'\\''")
                ),
            )?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(
                    directory.join("never-run.sh"),
                    fs::Permissions::from_mode(0o700),
                )?;
            }
        }
        import::add(
            &vault,
            &collection.to_string_lossy(),
            &self.0.join("state"),
            &ImportOptions {
                all: true,
                ..Default::default()
            },
        )?;
        let mut updates = BTreeMap::new();
        for (id, category, role) in entries {
            let mut metadata = edit::read_metadata(&vault, id)?;
            metadata.classification.category = (*category).into();
            metadata.classification.role = (*role).into();
            metadata.classification.classified_by = "manual".into();
            updates.insert((*id).to_string(), metadata);
        }
        edit::apply(&vault, &updates, "Set group candidate fixtures", false)?;
        Ok(vault)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _cleanup = fs::remove_dir_all(&self.0);
    }
}

fn snapshot(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>> {
    fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) -> Result<()> {
        for entry in fs::read_dir(directory)? {
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

fn pending_proposal(overlaps: &[&str]) -> ClassificationProposal {
    ClassificationProposal {
        category: "backend-api".into(),
        stack: vec![],
        stage: vec![],
        tags: vec!["review".into()],
        role: "Legacy pending role".into(),
        when_to_use: "Before implementation".into(),
        when_not: "During deployment".into(),
        improved_description: "Pending review text.".into(),
        overlaps_with: overlaps.iter().map(|id| (*id).to_string()).collect(),
        confidence: 0.8,
        category_reason: String::new(),
    }
}

fn pairs(report: &kinana_core::GroupsReport) -> Vec<Vec<&str>> {
    report
        .candidates
        .iter()
        .map(|candidate| candidate.skill_ids.iter().map(String::as_str).collect())
        .collect()
}

#[test]
fn recorded_and_legacy_overlaps_are_sorted_deduplicated_pairs_with_safe_reference_warnings()
-> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.seed(&[
        ("alpha", "backend-api", "Review REST contracts"),
        ("beta", "web-frontend", "Render CSS layouts"),
        ("gamma", "backend-api", "Parse JSON schema"),
    ])?;
    let mut alpha = edit::read_metadata(&vault, "alpha")?;
    alpha.classification.overlaps_with = [
        "beta",
        "beta",
        "alpha",
        "missing-skill",
        "../outside",
        "/tmp/secret",
        "../outside",
        "",
    ]
    .iter()
    .map(|id| (*id).to_string())
    .collect();
    alpha.proposal = Some(pending_proposal(&["gamma", "missing-legacy", "gamma"]));
    let mut beta = edit::read_metadata(&vault, "beta")?;
    beta.classification.overlaps_with = vec!["alpha".into()];
    edit::apply(
        &vault,
        &BTreeMap::from([("alpha".into(), alpha), ("beta".into(), beta)]),
        "Record overlap fixtures",
        false,
    )?;
    fs::write(temp.0.join("outside"), b"Do not inspect as a skill.")?;
    let before = snapshot(&temp.0)?;
    let cache = fs::read(&vault.index_path)?;
    let first = groups::discover(&vault)?;
    let second = groups::discover(&vault)?;
    assert_eq!(
        pairs(&first),
        [vec!["alpha", "beta"], vec!["alpha", "gamma"]]
    );
    assert_eq!(first.candidates[0].category, None);
    assert_eq!(first.candidates[1].category.as_deref(), Some("backend-api"));
    assert_eq!(first.candidates[0].reasons.len(), 2);
    assert_eq!(first.candidates[1].reasons.len(), 1);
    assert!(
        first.candidates[0]
            .reasons
            .iter()
            .all(|reason| reason.contains("classification.overlaps_with"))
    );
    assert!(first.candidates[1].reasons[0].contains("proposal.overlaps_with"));
    assert_eq!(first.warnings.len(), 5);
    assert!(
        first
            .warnings
            .iter()
            .any(|warning| warning.contains("missing-skill"))
    );
    assert!(
        first
            .warnings
            .iter()
            .any(|warning| warning.contains("missing-legacy"))
    );
    assert!(
        first
            .warnings
            .iter()
            .any(|warning| warning.contains("../outside"))
    );
    assert!(
        first
            .warnings
            .iter()
            .any(|warning| warning.contains("/tmp/secret"))
    );
    assert!(first.warnings.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(
        serde_json::to_string(&first).unwrap(),
        serde_json::to_string(&second).unwrap()
    );
    assert_eq!(fs::read(&vault.index_path)?, cache);
    assert_eq!(snapshot(&temp.0)?, before);
    assert!(!vault.root.join("groups.toml").exists());
    Ok(())
}

#[test]
fn role_similarity_uses_same_category_unicode_tokens_and_conservative_jaccard_evidence()
-> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.seed(&[
        (
            "english-a",
            "backend-api",
            "Review REST API contracts before implementation",
        ),
        (
            "english-b",
            "backend-api",
            "Review REST API contracts before coding",
        ),
        ("exact-a", "devops", "Deploy Docker"),
        ("exact-b", "devops", "deploy DOCKER!"),
        ("unicode-a", "docs-writing", "مراجعة واجهات برمجية"),
        ("unicode-b", "docs-writing", "مراجعة، واجهات برمجية!"),
        ("threshold-a", "testing-qa", "alpha beta gamma delta"),
        ("threshold-b", "testing-qa", "alpha beta gamma epsilon"),
    ])?;
    let mut english = edit::read_metadata(&vault, "english-b")?;
    english.classification.overlaps_with = vec!["english-a".into(), "english-a".into()];
    edit::apply(
        &vault,
        &BTreeMap::from([("english-b".into(), english)]),
        "Record combined overlap evidence",
        false,
    )?;
    let report = groups::discover(&vault)?;
    assert_eq!(
        pairs(&report),
        [
            vec!["english-a", "english-b"],
            vec!["exact-a", "exact-b"],
            vec!["threshold-a", "threshold-b"],
            vec!["unicode-a", "unicode-b"]
        ]
    );
    assert_eq!(report.candidates[0].reasons.len(), 2);
    assert!(
        report
            .candidates
            .iter()
            .all(|candidate| candidate.category.is_some())
    );
    assert!(report.candidates.iter().all(|candidate| {
        candidate
            .reasons
            .iter()
            .any(|reason| reason.starts_with("role_similarity:"))
    }));
    assert!(
        report.candidates[2]
            .reasons
            .iter()
            .any(|reason| reason.contains("3/5"))
    );
    assert!(report.warnings.is_empty());
    Ok(())
}

#[test]
fn blank_generic_short_different_category_and_weakly_related_roles_create_no_guesses() -> Result<()>
{
    let temp = Temp::new()?;
    let vault = temp.seed(&[
        ("blank-a", "backend-api", ""),
        ("blank-b", "backend-api", "   \n\t"),
        ("generic-a", "docs-writing", "the and this skill helps"),
        ("generic-b", "docs-writing", "this skill helps and the"),
        ("single-a", "security", "Review"),
        ("single-b", "security", "review!"),
        ("cross-a", "devops", "Deploy Docker containers"),
        ("cross-b", "testing-qa", "Deploy Docker containers"),
        ("weak-a", "web-frontend", "alpha beta gamma"),
        ("weak-b", "web-frontend", "alpha beta epsilon zeta"),
        ("numeric-a", "mobile", "12 34"),
        ("numeric-b", "mobile", "12 34"),
    ])?;
    let before = snapshot(&temp.0)?;
    let report = groups::discover(&vault)?;
    assert!(report.candidates.is_empty());
    assert!(report.warnings.is_empty());
    assert_eq!(snapshot(&temp.0)?, before);
    Ok(())
}

#[test]
fn overlap_edges_are_candidates_without_automatic_transitive_groups_or_priorities() -> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.seed(&[
        ("alpha", "backend-api", "Review contracts"),
        ("beta", "backend-api", "Parse schemas"),
        ("gamma", "backend-api", "Measure performance"),
    ])?;
    let mut alpha = edit::read_metadata(&vault, "alpha")?;
    alpha.classification.overlaps_with = vec!["beta".into()];
    let mut beta = edit::read_metadata(&vault, "beta")?;
    beta.classification.overlaps_with = vec!["gamma".into()];
    edit::apply(
        &vault,
        &BTreeMap::from([("alpha".into(), alpha), ("beta".into(), beta)]),
        "Record overlap edges",
        false,
    )?;
    let before = snapshot(&temp.0)?;
    let report = groups::discover(&vault)?;
    assert_eq!(
        pairs(&report),
        [vec!["alpha", "beta"], vec!["beta", "gamma"]]
    );
    assert!(
        report
            .candidates
            .iter()
            .all(|candidate| candidate.skill_ids.len() == 2)
    );
    assert!(fs::read_dir(vault.root.join("bundles"))?.next().is_none());
    assert_eq!(snapshot(&temp.0)?, before);
    Ok(())
}

#[test]
fn discovery_uses_file_metadata_without_requiring_or_creating_a_cache_or_running_source_files()
-> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.seed(&[
        ("one", "devops", "Deploy Docker"),
        ("two", "devops", "Deploy Docker"),
    ])?;
    let missing_cache = temp.0.join("not-created/index.db");
    let without_cache = Vault::new(vault.root.clone(), missing_cache.clone());
    let before = snapshot(&temp.0)?;
    assert_eq!(groups::discover(&without_cache)?.candidates.len(), 1);
    assert!(!missing_cache.exists());
    assert!(!temp.0.join("not-created").exists());
    assert_eq!(snapshot(&temp.0)?, before);
    fs::write(&vault.index_path, b"stale cache data must remain unchanged")?;
    let before = snapshot(&temp.0)?;
    assert_eq!(groups::discover(&vault)?.candidates.len(), 1);
    assert_eq!(snapshot(&temp.0)?, before);
    assert!(!before.keys().any(|path| {
        path.file_name()
            .is_some_and(|name| name == "GROUPS_SCRIPT_EXECUTED")
    }));
    gitlog::ensure_clean(&vault.root)?;
    Ok(())
}
