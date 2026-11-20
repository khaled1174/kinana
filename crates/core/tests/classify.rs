mod support;

use kinana_core::{
    ClassificationProposal, ClassifyOptions, ImportOptions, Result, classify, edit, gitlog, hash,
    import, vault::Vault,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Result<Self> {
        let path = support::temp_dir()?;
        Ok(Self(path))
    }

    fn seed(&self, entries: &[(&str, &str)]) -> Result<Vault> {
        let vault = Vault::new(self.0.join("vault"), self.0.join("cache/index.db"));
        vault.init(false)?;
        let collection = self.0.join("collection");
        fs::create_dir(&collection)?;
        for (name, description) in entries {
            let directory = collection.join(name);
            fs::create_dir(&directory)?;
            fs::create_dir_all(directory.join("assets/empty"))?;
            fs::write(
                directory.join("SKILL.md"),
                format!(
                    "---\nname: {name}\ndescription: {description}\n---\nBody mentions security, React, databases, and Docker.\n"
                ),
            )?;
            fs::write(directory.join("security-api.py"), "# Data only.\n")?;
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
        Ok(vault)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _cleanup = fs::remove_dir_all(&self.0);
    }
}

fn snapshot(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>> {
    fn visit(root: &Path, at: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) -> Result<()> {
        for entry in fs::read_dir(at)? {
            let path = entry?.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|error| std::io::Error::other(error.to_string()))?
                .to_path_buf();
            let info = fs::symlink_metadata(&path)?;
            if info.file_type().is_symlink() {
                files.insert(
                    relative,
                    fs::read_link(&path)?
                        .as_os_str()
                        .as_encoded_bytes()
                        .to_vec(),
                );
            } else if info.is_dir() {
                files.insert(relative, b"directory".to_vec());
                visit(root, &path, files)?;
            } else if info.is_file() {
                files.insert(relative, fs::read(&path)?);
            } else {
                files.insert(relative, b"special".to_vec());
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files)?;
    Ok(files)
}

fn keywords(ids: &[&str]) -> ClassifyOptions {
    ClassifyOptions {
        ids: ids.iter().map(|id| (*id).to_string()).collect(),
        agent: "keywords".into(),
        ..Default::default()
    }
}

fn categories(vault: &Vault, categories: &[&str]) -> Result<()> {
    let mut config = vault.config()?;
    config.categories = categories
        .iter()
        .map(|value| (*value).to_string())
        .collect();
    fs::write(
        vault.root.join("box.toml"),
        toml::to_string_pretty(&config)?,
    )?;
    gitlog::commit(&vault.root, "Configure taxonomy fixture")?;
    Ok(())
}

#[test]
fn offline_dictionary_covers_all_categories_and_preserves_every_source_byte() -> Result<()> {
    let temp = Temp::new()?;
    let fixtures = [
        ("fixture-00", "SwiftUI iOS screens.", "mobile"),
        ("fixture-01", "React HTML CSS.", "web-frontend"),
        ("fixture-02", "REST API endpoints.", "backend-api"),
        ("fixture-03", "PostgreSQL database SQL.", "data-db"),
        ("fixture-04", "Docker Kubernetes Terraform.", "devops"),
        ("fixture-05", "Pytest regression tests.", "testing-qa"),
        ("fixture-06", "PRD requirements roadmap.", "planning-specs"),
        ("fixture-07", "Figma wireframe UX.", "design-ui"),
        (
            "fixture-08",
            "Documentation README writing.",
            "docs-writing",
        ),
        ("fixture-09", "Video audio storyboard.", "content-media"),
        ("fixture-10", "MCP agents automation.", "automation-agents"),
        (
            "fixture-11",
            "Encryption vulnerabilities security.",
            "security",
        ),
        ("fixture-12", "Unrelated routines.", "other"),
    ];
    let entries: Vec<_> = fixtures
        .iter()
        .map(|(id, description, _)| (*id, *description))
        .collect();
    let vault = temp.seed(&entries)?;
    let source_before = snapshot(&vault.root.join("skills"))?;
    let report = classify::classify(
        &vault,
        &ClassifyOptions {
            all: true,
            confirmed: true,
            agent: "keywords".into(),
            ..Default::default()
        },
    )?;
    assert_eq!(report.selected, fixtures.len());
    assert_eq!(report.agent_calls, 0);
    assert!(report.committed);
    assert_eq!(report.build.items.len(), fixtures.len());
    for (id, original, category) in fixtures {
        let detail = vault.show(id)?;
        assert_eq!(detail.skill.category, category);
        assert_eq!(detail.metadata.classification.classified_by, "keywords");
        assert_eq!(detail.metadata.description.active, "original");
        assert_eq!(detail.metadata.description.original, original);
        assert!(detail.metadata.description.improved.is_none());
        assert!(detail.metadata.proposal.is_none());
        assert_eq!(
            detail.metadata.empty_directories,
            [PathBuf::from("assets/empty")]
        );
        assert_eq!(
            detail.metadata.content_hash,
            hash::hash_directory(&vault.root.join("skills").join(id))?
        );
        assert!(vault.root.join("dist").join(id).join("SKILL.md").is_file());
    }
    assert_eq!(snapshot(&vault.root.join("skills"))?, source_before);
    gitlog::ensure_clean(&vault.root)?;
    Ok(())
}

#[test]
fn dry_run_reports_counts_and_current_classification_with_no_writes_or_agent_attempts() -> Result<()>
{
    let temp = Temp::new()?;
    let vault = temp.seed(&[("one", "Swift iOS."), ("two", "REST API.")])?;
    let before = snapshot(&temp.0)?;
    let report = classify::classify(
        &vault,
        &ClassifyOptions {
            all: true,
            agent: "claude".into(),
            dry_run: true,
            ..Default::default()
        },
    )?;
    assert!(report.dry_run);
    assert_eq!(report.selected, 2);
    assert_eq!(report.estimated_agent_calls, 2);
    assert_eq!(report.agent_calls, 0);
    assert!(!report.committed);
    assert!(
        report
            .items
            .iter()
            .all(|item| item.attempts == 0 && !item.proposal_created)
    );
    assert!(
        report
            .items
            .iter()
            .all(|item| item.classified_by == "pending")
    );
    assert!(!report.warnings.is_empty());
    assert_eq!(snapshot(&temp.0)?, before);
    Ok(())
}

#[test]
fn confirmed_batch_reports_progress_and_keeps_preview_read_only() -> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.seed(&[("one", "Swift iOS."), ("two", "REST API.")])?;
    let options = ClassifyOptions {
        all: true,
        agent: "keywords".into(),
        confirmed: true,
        ..Default::default()
    };
    let before = snapshot(&temp.0)?;
    let preview = classify::classify(
        &vault,
        &ClassifyOptions {
            dry_run: true,
            ..options.clone()
        },
    )?;
    assert_eq!(preview.selected, 2);
    assert_eq!(snapshot(&temp.0)?, before);
    let control = classify::ClassifyControl::default();
    let report =
        classify::classify_confirmed(&vault, &options, &preview.estimated_tokens, &control)?;
    let progress = control.progress();
    assert_eq!(progress.total, 2);
    assert_eq!(progress.completed, 2);
    assert_eq!(progress.current_id, None);
    assert_eq!(report.items.len(), 2);
    assert!(report.committed);
    Ok(())
}

#[test]
fn cancelled_batch_does_not_classify_or_publish_any_skill() -> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.seed(&[("one", "Swift iOS."), ("two", "REST API.")])?;
    let options = ClassifyOptions {
        all: true,
        agent: "keywords".into(),
        confirmed: true,
        ..Default::default()
    };
    let preview = classify::classify(
        &vault,
        &ClassifyOptions {
            dry_run: true,
            ..options.clone()
        },
    )?;
    let before = snapshot(&temp.0)?;
    let control = classify::ClassifyControl::default();
    assert!(control.cancel());
    assert_eq!(
        classify::classify_confirmed(&vault, &options, &preview.estimated_tokens, &control)
            .unwrap_err()
            .code,
        "CANCELLED"
    );
    assert_eq!(control.progress().completed, 0);
    assert_eq!(snapshot(&temp.0)?, before);
    Ok(())
}

#[test]
fn whole_vault_confirmation_and_stale_estimates_are_rejected_before_changes() -> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.seed(&[("one", "Swift iOS."), ("two", "REST API.")])?;
    let mut options = ClassifyOptions {
        all: true,
        agent: "claude".into(),
        ..Default::default()
    };
    let preview = classify::classify(
        &vault,
        &ClassifyOptions {
            dry_run: true,
            ..options.clone()
        },
    )?;
    let before = snapshot(&temp.0)?;
    let control = classify::ClassifyControl::default();
    assert_eq!(
        classify::classify_confirmed(&vault, &options, &preview.estimated_tokens, &control)
            .unwrap_err()
            .code,
        "CLASSIFY_CONFIRMATION_REQUIRED"
    );
    options.confirmed = true;
    let mut stale = preview.estimated_tokens;
    stale.total_input_tokens += 1;
    assert_eq!(
        classify::classify_confirmed(&vault, &options, &stale, &control)
            .unwrap_err()
            .code,
        "CLASSIFY_ESTIMATE_CHANGED"
    );
    assert_eq!(control.progress().completed, 0);
    assert_eq!(snapshot(&temp.0)?, before);
    Ok(())
}

#[test]
fn batches_require_confirmation_and_invalid_selection_never_partially_changes_metadata()
-> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.seed(&[("one", "Swift iOS."), ("two", "REST API.")])?;
    let before = snapshot(&temp.0)?;
    let error = classify::classify(&vault, &keywords(&["one", "two"])).unwrap_err();
    assert_eq!(error.code, "CLASSIFY_CONFIRMATION_REQUIRED");
    assert!(error.needs_decision);
    assert!(error.message.contains("2 skills"));
    let error = classify::classify(
        &vault,
        &ClassifyOptions {
            confirmed: true,
            agent: "claude".into(),
            ..keywords(&["one", "missing"])
        },
    )
    .unwrap_err();
    assert_eq!(error.code, "SKILL_NOT_FOUND");
    assert_eq!(
        classify::classify(&vault, &ClassifyOptions::default())
            .unwrap_err()
            .code,
        "INVALID_SELECTION"
    );
    assert_eq!(
        classify::classify(
            &vault,
            &ClassifyOptions {
                all: true,
                ..keywords(&["one"])
            }
        )
        .unwrap_err()
        .code,
        "INVALID_SELECTION"
    );
    assert_eq!(
        classify::classify(&vault, &keywords(&["one", "one"]))
            .unwrap_err()
            .code,
        "INVALID_SELECTION"
    );
    assert_eq!(snapshot(&temp.0)?, before);

    let single = Temp::new()?;
    let single_vault = single.seed(&[("only", "Swift iOS.")])?;
    let error = classify::classify(
        &single_vault,
        &ClassifyOptions {
            all: true,
            agent: "keywords".into(),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.code, "CLASSIFY_CONFIRMATION_REQUIRED");
    assert!(error.message.contains("1 skills"));
    Ok(())
}

#[test]
fn pending_selects_only_pending_classification_and_an_empty_selection_is_a_noop() -> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.seed(&[
        ("done", "REST API."),
        ("next", "React HTML."),
        ("waiting", "Swift iOS."),
    ])?;
    classify::classify(&vault, &keywords(&["done"]))?;
    let options = ClassifyOptions {
        pending: true,
        agent: "keywords".into(),
        ..Default::default()
    };
    let preview = classify::classify(
        &vault,
        &ClassifyOptions {
            dry_run: true,
            ..options.clone()
        },
    )?;
    assert_eq!(preview.selected, 2);
    assert_eq!(
        preview
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["next", "waiting"]
    );
    assert_eq!(
        classify::classify(&vault, &options).unwrap_err().code,
        "CLASSIFY_CONFIRMATION_REQUIRED"
    );
    let report = classify::classify(
        &vault,
        &ClassifyOptions {
            confirmed: true,
            ..options.clone()
        },
    )?;
    assert_eq!(report.selected, 2);
    let before = snapshot(&temp.0)?;
    let empty = classify::classify(&vault, &options)?;
    assert_eq!(empty.selected, 0);
    assert!(empty.items.is_empty());
    assert!(!empty.committed);
    assert_eq!(snapshot(&temp.0)?, before);
    Ok(())
}

#[test]
fn unavailable_safe_auto_and_codex_adapters_fall_back_with_zero_model_calls() -> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.seed(&[("fixture", "React HTML CSS.")])?;
    for agent in ["auto", "codex"] {
        let report = classify::classify(
            &vault,
            &ClassifyOptions {
                agent: agent.into(),
                ..keywords(&["fixture"])
            },
        )?;
        assert_eq!(report.agent_calls, 0);
        assert_eq!(report.estimated_agent_calls, 0);
        assert_eq!(report.items[0].category, "web-frontend");
        assert_eq!(report.items[0].classified_by, "keywords");
        assert_eq!(report.items[0].attempts, 0);
        assert!(!report.items[0].warnings.is_empty());
        assert!(!report.items[0].proposal_created);
    }
    Ok(())
}

#[test]
fn custom_taxonomy_never_emits_disallowed_categories_or_guesses_a_fallback() -> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.seed(&[
        ("fixture", "React HTML CSS."),
        ("custom", "Acme domain activities."),
    ])?;
    categories(&vault, &["security", "acme-domain"])?;
    let before = snapshot(&temp.0)?;
    let error = classify::classify(&vault, &keywords(&["fixture"])).unwrap_err();
    assert_eq!(error.code, "CATEGORY_FALLBACK_REQUIRED");
    assert!(error.needs_decision);
    assert_eq!(snapshot(&temp.0)?, before);
    assert_eq!(
        classify::classify(&vault, &keywords(&["custom"]))?.items[0].category,
        "acme-domain"
    );
    categories(&vault, &["security", "other"])?;
    assert_eq!(
        classify::classify(&vault, &keywords(&["fixture"]))?.items[0].category,
        "other"
    );
    Ok(())
}

#[test]
fn keyword_reclassification_preserves_approved_description_and_unknown_metadata() -> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.seed(&[("fixture", "Documentation README writing.")])?;
    let mut metadata = edit::read_metadata(&vault, "fixture")?;
    metadata.description.active = "improved".into();
    metadata.description.improved = Some("Security encryption vulnerabilities.".into());
    metadata.description.approved_at = Some("2026-11-13T09:05:00Z".into());
    metadata.proposal = Some(ClassificationProposal {
        category: "content-media".into(),
        stack: vec![],
        stage: vec![],
        tags: vec!["video".into()],
        role: "Video role".into(),
        when_to_use: "Before recording".into(),
        when_not: "During editing".into(),
        improved_description: "Unreviewed video text.".into(),
        overlaps_with: vec![],
        confidence: 0.8,
    });
    let original_metadata = metadata.clone();
    let path = vault.root.join("meta/fixture.toml");
    fs::write(
        &path,
        format!(
            "{}\n[invocation]\nmode='manual'\n[custom]\nkeep='unknown value'\n",
            toml::to_string_pretty(&metadata)?
        ),
    )?;
    gitlog::commit(&vault.root, "Approved description fixture")?;
    let source_before = snapshot(&vault.root.join("skills"))?;
    let report = classify::classify(&vault, &keywords(&["fixture"]))?;
    assert_eq!(report.items[0].category, "docs-writing");
    let after = edit::read_metadata(&vault, "fixture")?;
    assert_eq!(after.description.active, "improved");
    assert_eq!(
        after.description.improved,
        original_metadata.description.improved
    );
    assert_eq!(
        after.description.approved_at,
        original_metadata.description.approved_at
    );
    assert!(after.proposal.is_none());
    assert_eq!(
        toml::to_string(&after.source)?,
        toml::to_string(&original_metadata.source)?
    );
    assert_eq!(
        toml::to_string(&after.audit)?,
        toml::to_string(&original_metadata.audit)?
    );
    assert_eq!(after.content_hash, original_metadata.content_hash);
    assert_eq!(after.empty_directories, original_metadata.empty_directories);
    let value: toml::Value = toml::from_str(&fs::read_to_string(path)?)?;
    assert_eq!(value["invocation"]["mode"].as_str(), Some("manual"));
    assert_eq!(value["custom"]["keep"].as_str(), Some("unknown value"));
    assert_eq!(
        vault.show("fixture")?.skill.description,
        "Security encryption vulnerabilities."
    );
    assert!(
        fs::read_to_string(vault.root.join("dist/fixture/SKILL.md"))?
            .contains("Security encryption vulnerabilities.")
    );
    assert_eq!(snapshot(&vault.root.join("skills"))?, source_before);
    Ok(())
}

#[cfg(unix)]
#[test]
fn entire_selected_batch_rejects_links_and_special_files_before_any_provider_call() -> Result<()> {
    use std::os::unix::fs::symlink;
    use std::os::unix::net::UnixListener;
    let temp = Temp::new()?;
    let vault = temp.seed(&[("a-good", "React HTML."), ("z-bad", "Swift iOS.")])?;
    let link = vault.root.join("skills/z-bad/linked");
    symlink(&temp.0, &link)?;
    let before = snapshot(&temp.0)?;
    let options = ClassifyOptions {
        agent: "claude".into(),
        confirmed: true,
        ..keywords(&["a-good", "z-bad"])
    };
    assert_eq!(
        classify::classify(&vault, &options).unwrap_err().code,
        "SYMLINK_REJECTED"
    );
    assert_eq!(snapshot(&temp.0)?, before);
    fs::remove_file(link)?;
    let socket_path = vault.root.join("skills/z-bad/socket");
    let listener = UnixListener::bind(&socket_path)?;
    let before = snapshot(&temp.0)?;
    assert_eq!(
        classify::classify(&vault, &options).unwrap_err().code,
        "UNSUPPORTED_FILE"
    );
    assert_eq!(snapshot(&temp.0)?, before);
    drop(listener);
    Ok(())
}
