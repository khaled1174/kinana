//! Classification dimensions (8.3): stack and stage lists, validation and manual edits.
mod support;

use kinana_core::{
    ClassificationProposal, ClassifyOptions, ImportOptions, ListFilter, Result, ReviewDecision,
    catalog, classify, edit, gitlog, import, review,
    settings::{self, Action},
    vault::Vault,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Result<Self> {
        Ok(Self(support::temp_dir()?))
    }

    fn vault(&self) -> Vault {
        Vault::new(self.0.join("vault"), self.0.join("cache/index.db"))
    }

    fn seed(&self, entries: &[(&str, &str)]) -> Result<Vault> {
        let vault = self.vault();
        vault.init(false)?;
        let collection = self.0.join("collection");
        fs::create_dir(&collection)?;
        for (name, description) in entries {
            let directory = collection.join(name);
            fs::create_dir(&directory)?;
            fs::write(
                directory.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: {description}\n---\nBody text.\n"),
            )?;
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
            if fs::symlink_metadata(&path)?.is_dir() {
                files.insert(relative, b"directory".to_vec());
                visit(root, &path, files)?;
            } else {
                files.insert(relative, fs::read(&path)?);
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files)?;
    Ok(files)
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn meta(vault: &Vault, id: &str) -> Result<String> {
    Ok(fs::read_to_string(
        vault.root.join("meta").join(format!("{id}.toml")),
    )?)
}

/// Remove the two list keys so box.toml looks like one written before 8.3.
fn make_box_toml_old(vault: &Vault) -> Result<Vec<u8>> {
    let path = vault.root.join("box.toml");
    let mut value: toml::Value = toml::from_str(&fs::read_to_string(&path)?)?;
    let table = value.as_table_mut().expect("box.toml table");
    assert!(table.remove("stacks").is_some(), "new vaults record stacks");
    assert!(table.remove("stages").is_some(), "new vaults record stages");
    fs::write(&path, toml::to_string_pretty(&value)?)?;
    gitlog::commit(&vault.root, "Fixture: box.toml from before dimensions")?;
    Ok(fs::read(path)?)
}

fn proposal(stack: &[&str], stage: &[&str]) -> ClassificationProposal {
    ClassificationProposal {
        category: "web-frontend".into(),
        stack: strings(stack),
        stage: strings(stage),
        tags: vec!["hooks".into()],
        role: "Review components".into(),
        when_to_use: "Before merging interface changes".into(),
        when_not: "For database migrations".into(),
        improved_description: "Review interface components before merging.".into(),
        overlaps_with: vec![],
        confidence: 0.9,
    }
}

const DEFAULT_STACKS: &[&str] = &[
    "react",
    "next",
    "expo",
    "react-native",
    "flutter",
    "swift",
    "rust",
    "python",
    "node",
    "docker",
];
const DEFAULT_STAGES: &[&str] = &["plan", "design", "build", "test", "review", "ship"];

#[test]
fn new_vaults_record_the_default_lists_and_an_old_box_toml_reads_them_without_a_write() -> Result<()>
{
    let temp = Temp::new()?;
    let vault = temp.seed(&[("alpha", "Review API contracts")])?;
    let config = vault.config()?;
    assert_eq!(config.stacks, strings(DEFAULT_STACKS));
    assert_eq!(config.stages, strings(DEFAULT_STAGES));
    let written: toml::Value = toml::from_str(&fs::read_to_string(vault.root.join("box.toml"))?)?;
    assert_eq!(written["stacks"].as_array().map(Vec::len), Some(10));
    assert_eq!(written["stages"].as_array().map(Vec::len), Some(6));

    let old = make_box_toml_old(&vault)?;
    let before = snapshot(&temp.0)?;
    let config = vault.config()?;
    assert_eq!(config.stacks, strings(DEFAULT_STACKS));
    assert_eq!(config.stages, strings(DEFAULT_STAGES));
    vault.records()?;
    vault.show("alpha")?;
    let listed = catalog::list(&vault, &Default::default())?;
    assert_eq!(listed.stacks, strings(DEFAULT_STACKS));
    assert_eq!(listed.stages, strings(DEFAULT_STAGES));
    assert_eq!(fs::read(vault.root.join("box.toml"))?, old);
    assert_eq!(snapshot(&temp.0)?, before, "reading wrote something");
    Ok(())
}

#[test]
fn old_metadata_reads_empty_dimensions_and_unrelated_rewrites_add_no_lines() -> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.seed(&[("alpha", "Review API contracts")])?;
    let original = meta(&vault, "alpha")?;
    assert!(!original.contains("stack") && !original.contains("stage"));

    let before = snapshot(&temp.0)?;
    let record = vault.records()?.remove(0);
    assert!(record.stack.is_empty() && record.stage.is_empty());
    let detail = vault.show("alpha")?;
    assert!(detail.metadata.classification.stack.is_empty());
    assert!(detail.metadata.classification.stage.is_empty());
    let json = serde_json::to_value(&detail.skill).expect("serialize skill");
    assert_eq!(json["stack"], serde_json::json!([]));
    assert_eq!(json["stage"], serde_json::json!([]));
    assert_eq!(snapshot(&temp.0)?, before, "reading wrote something");

    // Settle the key order of the value-based writer first, so the comparison
    // below is about content and not about which serializer wrote the file.
    edit::set_pinned(&vault, "alpha", true, false)?;
    edit::set_pinned(&vault, "alpha", false, false)?;
    let original = meta(&vault, "alpha")?;
    assert!(!original.contains("stack") && !original.contains("stage"));

    // A favorite rewrite changes only its own line, and reverting it is exact.
    assert!(edit::set_pinned(&vault, "alpha", true, false)?.committed);
    let pinned = meta(&vault, "alpha")?;
    assert!(!pinned.contains("stack") && !pinned.contains("stage"));
    let changed_lines = original
        .lines()
        .zip(pinned.lines())
        .filter(|(a, b)| a != b)
        .collect::<Vec<_>>();
    assert_eq!(changed_lines, [("pinned = false", "pinned = true")]);
    assert!(edit::set_pinned(&vault, "alpha", false, false)?.committed);
    assert_eq!(meta(&vault, "alpha")?, original);

    // The allowlisted classification writer must not add empty arrays either.
    assert!(edit::set_category(&vault, "alpha", "security", false)?.committed);
    let recategorized = meta(&vault, "alpha")?;
    assert!(!recategorized.contains("stack") && !recategorized.contains("stage"));
    Ok(())
}

#[test]
fn manual_dimensions_are_validated_stored_in_list_order_committed_and_undoable() -> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.seed(&[("alpha", "Review API contracts")])?;
    let original = meta(&vault, "alpha")?;
    let head = gitlog::head(&vault.root)?;

    let report = edit::set_dimensions(
        &vault,
        "alpha",
        Some(&strings(&["node", "react", "node"])),
        Some(&strings(&["review", "design"])),
        false,
    )?;
    assert_eq!(report.id, "alpha");
    assert_eq!(report.stack, ["react", "node"], "list order, no duplicates");
    assert_eq!(report.stage, ["design", "review"]);
    assert!(report.changed && report.committed && !report.dry_run);
    assert_ne!(gitlog::head(&vault.root)?, head);

    let stored = edit::read_metadata(&vault, "alpha")?;
    assert_eq!(stored.classification.stack, ["react", "node"]);
    assert_eq!(stored.classification.stage, ["design", "review"]);
    assert_eq!(stored.classification.classified_by, "pending");
    let record = vault.records()?.remove(0);
    assert_eq!(record.stack, ["react", "node"]);
    assert_eq!(record.stage, ["design", "review"]);

    vault.undo(false)?;
    assert_eq!(
        meta(&vault, "alpha")?,
        original,
        "undo restores the old bytes"
    );
    edit::set_dimensions(
        &vault,
        "alpha",
        Some(&strings(&["react", "node"])),
        Some(&strings(&["design", "review"])),
        false,
    )?;

    // An omitted dimension is left alone; an empty one is cleared and its line removed.
    let report = edit::set_dimensions(&vault, "alpha", Some(&[]), None, false)?;
    assert!(report.stack.is_empty());
    assert_eq!(report.stage, ["design", "review"]);
    let text = meta(&vault, "alpha")?;
    assert!(!text.contains("stack ="));
    assert!(text.contains("stage ="));

    // `undo` reverts the last change only (a second undo would redo it).
    vault.undo(false)?;
    assert_eq!(
        edit::read_metadata(&vault, "alpha")?.classification.stack,
        ["react", "node"]
    );
    Ok(())
}

#[test]
fn dimension_previews_and_unchanged_requests_write_nothing() -> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.seed(&[("alpha", "Review API contracts")])?;
    let before = snapshot(&temp.0)?;
    let preview = edit::set_dimensions(
        &vault,
        "alpha",
        Some(&strings(&["rust"])),
        Some(&strings(&["build"])),
        true,
    )?;
    assert!(preview.dry_run && preview.changed && !preview.committed);
    assert_eq!(preview.stack, ["rust"]);
    assert_eq!(preview.stage, ["build"]);
    assert_eq!(snapshot(&temp.0)?, before, "dry run wrote something");

    edit::set_dimensions(&vault, "alpha", Some(&strings(&["rust"])), None, false)?;
    let head = gitlog::head(&vault.root)?;
    let same = edit::set_dimensions(&vault, "alpha", Some(&strings(&["rust"])), None, false)?;
    assert!(!same.changed && !same.committed);
    assert_eq!(same.stack, ["rust"]);
    assert_eq!(gitlog::head(&vault.root)?, head);

    assert_eq!(
        edit::set_dimensions(&vault, "alpha", None, None, false)
            .unwrap_err()
            .code,
        "INVALID_ARGUMENTS"
    );
    Ok(())
}

#[test]
fn out_of_list_values_are_rejected_with_stable_codes_on_every_metadata_write_path() -> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.seed(&[("alpha", "Review API contracts")])?;
    let before = snapshot(&temp.0)?;

    // Manual edit, including previews.
    for dry_run in [true, false] {
        let error =
            edit::set_dimensions(&vault, "alpha", Some(&strings(&["reactjs"])), None, dry_run)
                .unwrap_err();
        assert_eq!(error.code, "INVALID_STACK");
        assert!(error.message.contains("reactjs"));
        assert!(!error.needs_decision);
        let error = edit::set_dimensions(
            &vault,
            "alpha",
            Some(&strings(&["react"])),
            Some(&strings(&["deploy"])),
            dry_run,
        )
        .unwrap_err();
        assert_eq!(error.code, "INVALID_STAGE");
        assert!(error.message.contains("deploy"));
    }

    // Any metadata update that changes a dimension.
    let mut update = edit::read_metadata(&vault, "alpha")?;
    update.classification.stack = strings(&["angular"]);
    let error = edit::apply(
        &vault,
        &BTreeMap::from([("alpha".into(), update)]),
        "Fixture",
        false,
    )
    .unwrap_err();
    assert_eq!(error.code, "INVALID_STACK");
    let mut update = edit::read_metadata(&vault, "alpha")?;
    update.classification.stage = strings(&["deploy"]);
    let error = edit::apply(
        &vault,
        &BTreeMap::from([("alpha".into(), update)]),
        "Fixture",
        false,
    )
    .unwrap_err();
    assert_eq!(error.code, "INVALID_STAGE");

    // A proposal being stored.
    for (bad, code) in [
        (proposal(&["angular"], &[]), "INVALID_STACK"),
        (proposal(&[], &["deploy"]), "INVALID_STAGE"),
    ] {
        let mut update = edit::read_metadata(&vault, "alpha")?;
        update.proposal = Some(bad);
        let error = edit::apply(
            &vault,
            &BTreeMap::from([("alpha".into(), update)]),
            "Fixture",
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, code);
    }
    assert_eq!(snapshot(&temp.0)?, before, "a rejected write changed files");

    // A stored proposal that no longer fits the lists cannot be accepted.
    for (key, value, code) in [
        ("stack", "angular", "INVALID_STACK"),
        ("stage", "deploy", "INVALID_STAGE"),
    ] {
        let mut update = edit::read_metadata(&vault, "alpha")?;
        update.proposal = Some(proposal(&["react"], &["review"]));
        edit::apply(
            &vault,
            &BTreeMap::from([("alpha".into(), update)]),
            "Fixture: proposal",
            false,
        )?;
        let path = vault.root.join("meta/alpha.toml");
        let mut value_toml: toml::Value = toml::from_str(&fs::read_to_string(&path)?)?;
        value_toml["proposal_dimensions"][key] =
            toml::Value::Array(vec![toml::Value::String(value.into())]);
        fs::write(&path, toml::to_string_pretty(&value_toml)?)?;
        gitlog::commit(&vault.root, "Fixture: hand-edited proposal")?;
        let stored = fs::read(&path)?;
        for dry_run in [true, false] {
            let error =
                review::review(&vault, "alpha", ReviewDecision::Accept, dry_run).unwrap_err();
            assert_eq!(error.code, code, "{key} dry_run={dry_run}");
        }
        assert_eq!(fs::read(&path)?, stored);
        review::review(&vault, "alpha", ReviewDecision::Reject, false)?;
    }
    Ok(())
}

#[test]
fn accepting_a_proposal_keeps_its_dimensions_in_the_classification() -> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.seed(&[("alpha", "Review API contracts")])?;
    let mut update = edit::read_metadata(&vault, "alpha")?;
    update.classification.category = "web-frontend".into();
    update.classification.stack = strings(&["react", "next"]);
    update.classification.stage = strings(&["review"]);
    update.classification.classified_by = "claude".into();
    update.proposal = Some(proposal(&["react", "next"], &["review"]));
    edit::apply(
        &vault,
        &BTreeMap::from([("alpha".into(), update)]),
        "Fixture: agent result",
        false,
    )?;
    let stored = edit::read_metadata(&vault, "alpha")?;
    let pending = stored.proposal.expect("stored proposal");
    assert_eq!(pending.stack, ["react", "next"]);
    assert_eq!(pending.stage, ["review"]);

    let report = review::review(&vault, "alpha", ReviewDecision::Accept, false)?;
    assert!(report.committed);
    let accepted = edit::read_metadata(&vault, "alpha")?;
    assert!(accepted.proposal.is_none());
    assert_eq!(accepted.description.active, "improved");
    assert_eq!(accepted.classification.stack, ["react", "next"]);
    assert_eq!(accepted.classification.stage, ["review"]);
    let record = vault.records()?.remove(0);
    assert_eq!(record.stack, ["react", "next"]);
    assert_eq!(record.stage, ["review"]);
    Ok(())
}

/// Every key of a stored `[proposal]` table that the 0.1.0 beta knows. It
/// rejects any other key there and can then read nothing in the vault.
const BETA_PROPOSAL_KEYS: &[&str] = &[
    "category",
    "confidence",
    "improved_description",
    "overlaps_with",
    "role",
    "tags",
    "when_not",
    "when_to_use",
];

#[test]
fn a_waiting_proposal_keeps_its_dimensions_beside_the_table_the_beta_reads() -> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.seed(&[("alpha", "Review API contracts")])?;
    let path = vault.root.join("meta/alpha.toml");
    let mut update = edit::read_metadata(&vault, "alpha")?;
    update.proposal = Some(proposal(&["react", "next"], &["review"]));
    edit::apply(
        &vault,
        &BTreeMap::from([("alpha".into(), update)]),
        "Fixture: agent result",
        false,
    )?;

    let stored: toml::Value = toml::from_str(&fs::read_to_string(&path)?)?;
    let mut keys = stored["proposal"]
        .as_table()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(keys, BETA_PROPOSAL_KEYS);
    assert_eq!(
        stored["proposal_dimensions"],
        toml::toml! { stack = ["react", "next"] stage = ["review"] }.into()
    );
    let pending = edit::read_metadata(&vault, "alpha")?.proposal;
    assert_eq!(pending, Some(proposal(&["react", "next"], &["review"])));

    // The beta removes `[proposal]` when it accepts or rejects and keeps the
    // table it does not know. That leftover is not a proposal, and the next
    // write of the file by this version drops it.
    let mut leftover = stored.clone();
    leftover.as_table_mut().unwrap().remove("proposal");
    fs::write(&path, toml::to_string_pretty(&leftover)?)?;
    gitlog::commit(&vault.root, "Fixture: the beta decided the proposal")?;
    assert!(edit::read_metadata(&vault, "alpha")?.proposal.is_none());
    edit::set_category(&vault, "alpha", "security", false)?;
    assert!(!fs::read_to_string(&path)?.contains("proposal_dimensions"));
    Ok(())
}

#[test]
fn a_stored_proposal_from_before_dimensions_still_reads_and_is_rewritten_without_new_lines()
-> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.seed(&[("alpha", "Review API contracts")])?;
    let path = vault.root.join("meta/alpha.toml");
    let text = format!(
        "{}\n[proposal]\ncategory = \"web-frontend\"\ntags = [\"hooks\"]\nrole = \"Review components\"\nwhen_to_use = \"Before merging\"\nwhen_not = \"For migrations\"\nimproved_description = \"Review interface components.\"\noverlaps_with = []\nconfidence = 0.9\n",
        fs::read_to_string(&path)?
    );
    fs::write(&path, text)?;
    gitlog::commit(&vault.root, "Fixture: proposal stored before dimensions")?;
    let stored = edit::read_metadata(&vault, "alpha")?;
    let pending = stored.proposal.expect("old proposal reads");
    assert!(pending.stack.is_empty() && pending.stage.is_empty());
    edit::set_category(&vault, "alpha", "security", false)?;
    let rewritten = fs::read_to_string(&path)?;
    assert!(rewritten.contains("[proposal]"));
    assert!(!rewritten.contains("stack") && !rewritten.contains("stage"));
    Ok(())
}

fn save(vault: &Vault, stacks: Option<&[&str]>, stages: Option<&[&str]>) -> Result<Action> {
    let config = vault.config()?;
    Ok(Action::Save {
        classification_agent: config.classification_agent,
        categories: config.categories,
        stacks: stacks.map(strings),
        stages: stages.map(strings),
        scan_paths: config.scan_paths,
        record_found_in: config.record_found_in,
    })
}

#[test]
fn the_settings_path_edits_both_lists_and_leaves_an_old_box_toml_alone_otherwise() -> Result<()> {
    let temp = Temp::new()?;
    let home = temp.0.join("home");
    fs::create_dir(&home)?;
    let vault = temp.seed(&[("alpha", "Review API contracts")])?;
    let old = make_box_toml_old(&vault)?;

    // An unrelated settings request (the shape the desktop sends today) is a no-op.
    let untouched = save(&vault, None, None)?;
    let plan = settings::preview(&vault, &home, &untouched)?;
    assert!(!plan.changed);
    settings::apply(&vault, &home, &untouched, &plan.token)?;
    assert_eq!(fs::read(vault.root.join("box.toml"))?, old);
    let encoded = serde_json::to_value(&untouched).expect("serialize action");
    assert!(encoded.get("stacks").is_none() && encoded.get("stages").is_none());
    let decoded: Action = serde_json::from_value(encoded).expect("the old action shape parses");
    assert!(matches!(
        decoded,
        Action::Save {
            stacks: None,
            stages: None,
            ..
        }
    ));

    let action = save(
        &vault,
        Some(&["react", "vue"]),
        Some(&["plan", "build", "ship"]),
    )?;
    let plan = settings::preview(&vault, &home, &action)?;
    assert!(plan.changed);
    assert_eq!(fs::read(vault.root.join("box.toml"))?, old, "preview wrote");
    let head = gitlog::head(&vault.root)?;
    let applied = settings::apply_controlled(&vault, &home, &action, &plan.token, None)?;
    assert!(applied.committed && applied.changed);
    assert_ne!(gitlog::head(&vault.root)?, head);
    let config = vault.config()?;
    assert_eq!(config.stacks, ["react", "vue"]);
    assert_eq!(config.stages, ["plan", "build", "ship"]);
    // A value the user added is usable at once, and the list edit is undoable.
    let preview = edit::set_dimensions(&vault, "alpha", Some(&strings(&["vue"])), None, true)?;
    assert_eq!(preview.stack, ["vue"]);
    vault.undo(false)?;
    assert_eq!(fs::read(vault.root.join("box.toml"))?, old);

    // Entries follow the category name rules; an empty list is allowed.
    for (stacks, stages, code) in [
        (Some(&["react", "react"][..]), None, "INVALID_STACKS"),
        (Some(&[" react"][..]), None, "INVALID_STACKS"),
        (Some(&[""][..]), None, "INVALID_STACKS"),
        (None, Some(&["plan", "plan"][..]), "INVALID_STAGES"),
        (None, Some(&["plan\n"][..]), "INVALID_STAGES"),
    ] {
        let error = settings::preview(&vault, &home, &save(&vault, stacks, stages)?).unwrap_err();
        assert_eq!(error.code, code);
    }
    let action = save(&vault, Some(&[]), Some(&[]))?;
    let plan = settings::preview(&vault, &home, &action)?;
    settings::apply(&vault, &home, &action, &plan.token)?;
    assert!(vault.config()?.stacks.is_empty() && vault.config()?.stages.is_empty());
    Ok(())
}

#[test]
fn removing_a_list_value_that_a_skill_or_proposal_still_holds_is_refused_naming_the_skills()
-> Result<()> {
    let temp = Temp::new()?;
    let home = temp.0.join("home");
    fs::create_dir(&home)?;
    let vault = temp.seed(&[
        ("alpha", "Review API contracts"),
        ("beta", "Write documentation"),
        ("gamma", "Plan releases"),
    ])?;
    edit::set_dimensions(
        &vault,
        "alpha",
        Some(&strings(&["react"])),
        Some(&strings(&["review"])),
        false,
    )?;
    let mut update = edit::read_metadata(&vault, "beta")?;
    update.proposal = Some(proposal(&["react"], &["review"]));
    edit::apply(
        &vault,
        &BTreeMap::from([("beta".into(), update)]),
        "Fixture: proposal",
        false,
    )?;
    let before = snapshot(&temp.0)?;

    let without_react = DEFAULT_STACKS
        .iter()
        .copied()
        .filter(|value| *value != "react")
        .collect::<Vec<_>>();
    let error =
        settings::preview(&vault, &home, &save(&vault, Some(&without_react), None)?).unwrap_err();
    assert_eq!(error.code, "STACK_IN_USE");
    assert!(!error.needs_decision);
    assert!(error.message.contains("react"));
    assert!(error.message.contains("alpha") && error.message.contains("beta"));
    assert!(!error.message.contains("gamma"));

    let without_review = DEFAULT_STAGES
        .iter()
        .copied()
        .filter(|value| *value != "review")
        .collect::<Vec<_>>();
    let action = save(&vault, None, Some(&without_review))?;
    let error = settings::preview(&vault, &home, &action).unwrap_err();
    assert_eq!(error.code, "STAGE_IN_USE");
    assert!(error.message.contains("alpha") && error.message.contains("beta"));
    assert_eq!(
        settings::apply(&vault, &home, &action, "stale-token")
            .unwrap_err()
            .code,
        "STAGE_IN_USE"
    );
    assert_eq!(
        snapshot(&temp.0)?,
        before,
        "a refused settings change wrote"
    );

    // Unused values can go, and reordering keeps every held value valid.
    let without_docker = DEFAULT_STACKS
        .iter()
        .copied()
        .filter(|value| *value != "docker")
        .rev()
        .collect::<Vec<_>>();
    let action = save(&vault, Some(&without_docker), None)?;
    let plan = settings::preview(&vault, &home, &action)?;
    settings::apply(&vault, &home, &action, &plan.token)?;
    assert!(!vault.config()?.stacks.contains(&"docker".to_string()));
    Ok(())
}

#[test]
fn list_filters_by_stack_and_stage_combine_with_category_and_reject_unknown_values() -> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.seed(&[
        ("alpha", "First"),
        ("beta", "Second"),
        ("gamma", "Third"),
        ("delta", "Fourth"),
    ])?;
    for (id, category, stack, stage) in [
        (
            "alpha",
            "web-frontend",
            &["react", "next"][..],
            &["build"][..],
        ),
        ("beta", "web-frontend", &["react"][..], &["review"][..]),
        (
            "gamma",
            "testing-qa",
            &["react"][..],
            &["test", "review"][..],
        ),
    ] {
        edit::set_category(&vault, id, category, false)?;
        edit::set_dimensions(
            &vault,
            id,
            Some(&strings(stack)),
            Some(&strings(stage)),
            false,
        )?;
    }
    let ids = |filter: ListFilter| -> Result<Vec<String>> {
        Ok(vault.list(&filter)?.into_iter().map(|r| r.id).collect())
    };
    let stack = |value: &str| Some(value.to_string());
    assert_eq!(
        ids(ListFilter {
            stack: stack("react"),
            ..Default::default()
        })?,
        ["alpha", "beta", "gamma"]
    );
    assert_eq!(
        ids(ListFilter {
            stack: stack("next"),
            ..Default::default()
        })?,
        ["alpha"]
    );
    assert_eq!(
        ids(ListFilter {
            stage: stack("review"),
            ..Default::default()
        })?,
        ["beta", "gamma"]
    );
    assert_eq!(
        ids(ListFilter {
            stack: stack("react"),
            stage: stack("review"),
            category: Some("web-frontend".into()),
            ..Default::default()
        })?,
        ["beta"]
    );
    assert!(
        ids(ListFilter {
            stack: stack("rust"),
            ..Default::default()
        })?
        .is_empty(),
        "a configured value nobody holds is an empty result, not an error"
    );
    assert_eq!(
        vault
            .list(&ListFilter {
                stack: stack("reactjs"),
                ..Default::default()
            })
            .unwrap_err()
            .code,
        "INVALID_STACK"
    );
    assert_eq!(
        vault
            .list(&ListFilter {
                stage: stack("deploy"),
                ..Default::default()
            })
            .unwrap_err()
            .code,
        "INVALID_STAGE"
    );

    // The desktop catalog uses the same filters and carries the dimensions.
    let report = catalog::list(
        &vault,
        &catalog::CatalogFilter {
            stack: stack("react"),
            stage: stack("review"),
            ..Default::default()
        },
    )?;
    assert_eq!(report.total, 4);
    assert_eq!(
        report
            .items
            .iter()
            .map(|item| item.skill.id.as_str())
            .collect::<Vec<_>>(),
        ["beta", "gamma"]
    );
    assert_eq!(report.items[1].skill.stage, ["test", "review"]);
    assert_eq!(
        catalog::list(
            &vault,
            &catalog::CatalogFilter {
                stage: stack("deploy"),
                ..Default::default()
            },
        )
        .unwrap_err()
        .code,
        "INVALID_STAGE"
    );
    Ok(())
}

fn keywords(ids: &[&str]) -> ClassifyOptions {
    ClassifyOptions {
        ids: strings(ids),
        agent: "keywords".into(),
        confirmed: true,
        ..Default::default()
    }
}

#[test]
fn keyword_classification_adds_only_explicitly_named_stacks_and_never_a_stage() -> Result<()> {
    let temp = Temp::new()?;
    let cases: &[(&str, &str, &[&str])] = &[
        (
            "native-app",
            "Build screens with React Native.",
            &["react-native"],
        ),
        (
            "native-and-web",
            "Share code between React Native and a React web app.",
            &["react", "react-native"],
        ),
        (
            "hyphen-native",
            "Tips for react-native navigation.",
            &["react-native"],
        ),
        ("next-dot", "Route handlers in Next.js apps.", &["next"]),
        ("next-joined", "Deploy a nextjs site.", &["next"]),
        ("next-word", "Decide the next step of a plan.", &[]),
        ("node-dot", "Write a Node.js command line tool.", &["node"]),
        ("node-joined", "Profile nodejs services.", &["node"]),
        ("node-word", "Walk every node of a syntax tree.", &[]),
        ("reactive", "Reactive spreadsheets and rusty pythons.", &[]),
        (
            "many",
            "Dockerfile linting for Rust, Python, Flutter, Expo and SwiftUI projects.",
            &["expo", "flutter", "swift", "rust", "python", "docker"],
        ),
        (
            "react-plain",
            "Review React components and hooks.",
            &["react"],
        ),
        ("plain", "Summarize meeting notes.", &[]),
    ];
    let vault = temp.seed(
        &cases
            .iter()
            .map(|(name, description, _)| (*name, *description))
            .collect::<Vec<_>>(),
    )?;
    let ids = cases.iter().map(|(name, _, _)| *name).collect::<Vec<_>>();
    classify::classify(&vault, &keywords(&ids))?;
    for (name, description, expected) in cases {
        let stored = edit::read_metadata(&vault, name)?.classification;
        assert_eq!(stored.classified_by, "keywords");
        assert_eq!(stored.stack, strings(expected), "{name}: {description}");
        assert!(
            stored.stage.is_empty(),
            "{name}: keywords never set a stage"
        );
    }
    // A name counts as well as the description.
    let named = Temp::new()?;
    let vault = named.seed(&[("flutter-widgets", "Compose reusable widgets.")])?;
    classify::classify(&vault, &keywords(&["flutter-widgets"]))?;
    assert_eq!(
        edit::read_metadata(&vault, "flutter-widgets")?
            .classification
            .stack,
        ["flutter"]
    );
    Ok(())
}

#[test]
fn keyword_classification_keeps_existing_dimensions_and_honours_the_configured_list() -> Result<()>
{
    let temp = Temp::new()?;
    let home = temp.0.join("home");
    fs::create_dir(&home)?;
    let vault = temp.seed(&[
        ("alpha", "Review React components."),
        ("beta", "Build Vue and Docker setups."),
    ])?;
    edit::set_dimensions(
        &vault,
        "alpha",
        Some(&strings(&["next"])),
        Some(&strings(&["review"])),
        false,
    )?;
    // docker leaves the list and vue joins it: only configured values are detected.
    let action = save(&vault, Some(&["react", "next", "vue"]), None)?;
    let plan = settings::preview(&vault, &home, &action)?;
    settings::apply(&vault, &home, &action, &plan.token)?;

    classify::classify(&vault, &keywords(&["alpha", "beta"]))?;
    let alpha = edit::read_metadata(&vault, "alpha")?.classification;
    assert_eq!(alpha.stack, ["react", "next"], "detected values are added");
    assert_eq!(alpha.stage, ["review"], "a manual stage survives keywords");
    let beta = edit::read_metadata(&vault, "beta")?.classification;
    assert_eq!(beta.stack, ["vue"]);
    assert!(beta.stage.is_empty());

    // A preview shows nothing new and writes nothing.
    let before = snapshot(&temp.0)?;
    classify::classify(
        &vault,
        &ClassifyOptions {
            dry_run: true,
            ..keywords(&["alpha", "beta"])
        },
    )?;
    assert_eq!(snapshot(&temp.0)?, before);
    Ok(())
}
