//! The released 0.2 beta and 0.3 on one vault, through the real 0.2 binary.
//! Ignored by default because that binary is not part of this workspace:
//! build `kinana` from the 0.2 candidate commit (347c54d), point
//! `KINANA_BETA02_BIN` at it and run
//! `cargo test -p kinana --test beta02_coexistence -- --ignored`.
#[allow(dead_code)]
mod support;

use kinana_core::{
    ClassificationContext, ClassificationProposal, context_binding, edit,
    settings::{self, Action},
    taxonomy::Definitions,
    vault::Vault,
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use support::Sandbox;

fn run(binary: &Path, s: &Sandbox, args: &[&str], exit: i32) -> Value {
    let output = Command::new(binary)
        .env("HOME", &s.home)
        .env("USERPROFILE", &s.home)
        .arg("--vault")
        .arg(&s.vault)
        .args(args)
        .arg("--json")
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(exit),
        "{} {args:?}: {} {}",
        binary.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn source(s: &Sandbox, id: &str, description: &str) -> String {
    let source = s.root.join(format!("source-{id}"));
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        format!("---\nname: {id}\ndescription: {description}\n---\n# Body\nText.\n"),
    )
    .unwrap();
    source.to_str().unwrap().to_owned()
}

/// What a 0.3 agent classification writes: a proposal with its reason beside
/// `[proposal]`, and the context of the adopted classification.
fn classify_as_03(vault: &Vault, id: &str) {
    let mut metadata = edit::read_metadata(vault, id).unwrap();
    let proposal = ClassificationProposal {
        category: "design-ui".into(),
        stack: vec![],
        stage: vec!["review".into()],
        tags: vec!["layout".into()],
        role: "Reviews layouts".into(),
        when_to_use: "Before shipping a screen".into(),
        when_not: "For backend work".into(),
        improved_description: "Review screen layouts before shipping.".into(),
        overlaps_with: vec![],
        confidence: 0.8,
        category_reason: "The output is visual layout decisions.".into(),
    };
    metadata.classification.category = proposal.category.clone();
    metadata.classification.role = proposal.role.clone();
    metadata.classification.when_to_use = proposal.when_to_use.clone();
    metadata.classification.when_not = proposal.when_not.clone();
    metadata.classification.stage = proposal.stage.clone();
    metadata.classification.classified_by = "claude".into();
    metadata.classification.classified_from = Some("full".into());
    metadata.classification_context = Some(ClassificationContext {
        prompt_version: 2,
        category_reason: proposal.category_reason.clone(),
        binds: context_binding(&metadata.classification),
    });
    metadata.proposal = Some(proposal);
    edit::apply(
        vault,
        &BTreeMap::from([(id.to_string(), metadata)]),
        "0.3 agent classification",
        false,
    )
    .unwrap();
}

#[test]
#[ignore = "needs KINANA_BETA02_BIN, a kinana binary built from the 0.2 candidate"]
fn the_02_beta_reads_and_edits_a_vault_written_by_03() {
    let beta = PathBuf::from(
        std::env::var_os("KINANA_BETA02_BIN").expect("set KINANA_BETA02_BIN to the 0.2 binary"),
    );
    assert!(
        String::from_utf8_lossy(
            &Command::new(&beta)
                .arg("--version")
                .output()
                .unwrap()
                .stdout
        )
        .contains("0.2.0")
    );
    let new = Path::new(env!("CARGO_BIN_EXE_kinana"));
    let s = Sandbox::new();
    let index = s.home.join(".kinana/index-v2.db");
    run(new, &s, &["init"], 0);
    for (id, text) in [("alpha", "Review layouts"), ("beta", "Review forms")] {
        run(new, &s, &["add", &source(&s, id, text)], 0);
    }
    let vault = Vault::new(s.vault.clone(), index.clone());
    // 0.3 settings with changed definitions, written to box.toml.
    let config = vault.config().unwrap();
    let definitions = Definitions {
        categories: BTreeMap::from([("other".into(), "Use as the last resort.".into())]),
        rules: Some(vec!["design-ui vs other: prefer design-ui.".into()]),
        ..Definitions::default()
    };
    let action = Action::Save {
        classification_agent: config.classification_agent.clone(),
        categories: config.categories.clone(),
        stacks: None,
        stages: None,
        scan_paths: vec![],
        record_found_in: config.record_found_in,
        definitions: Some(definitions.clone()),
        pi: None,
        claude: None,
        // r5: a top-level key 0.2 does not know; it must still read the vault.
        auto_approve_descriptions: Some(false),
    };
    let plan = settings::preview(&vault, &s.home, &action).unwrap();
    settings::apply(&vault, &s.home, &action, &plan.token).unwrap();
    classify_as_03(&vault, "alpha");
    classify_as_03(&vault, "beta");
    run(new, &s, &["doctor", "--rebuild-index"], 0);

    // 0.2 reads everything 0.3 wrote.
    assert_eq!(run(&beta, &s, &["list"], 0).as_array().unwrap().len(), 2);
    let shown = run(&beta, &s, &["show", "alpha"], 0);
    assert_eq!(
        shown["metadata"]["proposal"]["improved_description"],
        "Review screen layouts before shipping."
    );
    assert!(run(&beta, &s, &["doctor"], 0)["healthy"].as_bool().unwrap());

    // 0.2 accepts a 0.3 proposal; the classification it adopted is unchanged,
    // so 0.3 keeps its reason.
    assert_eq!(
        run(&beta, &s, &["review", "alpha", "--accept"], 0)["committed"],
        true
    );
    let alpha = edit::read_metadata(&vault, "alpha").unwrap();
    assert!(alpha.proposal.is_none());
    assert_eq!(alpha.description.active, "improved");
    assert_eq!(
        alpha.classification_context.unwrap().category_reason,
        "The output is visual layout decisions."
    );

    // 0.2 reclassifies with keywords: 0.3 no longer shows the old reason.
    run(&beta, &s, &["classify", "beta", "--agent", "keywords"], 0);
    let beta_meta = edit::read_metadata(&vault, "beta").unwrap();
    assert_eq!(beta_meta.classification.classified_by, "keywords");
    assert!(beta_meta.classification_context.is_none());

    // 0.2 edits a category: the reason no longer applies.
    run(&beta, &s, &["category", "alpha", "web-frontend"], 0);
    assert!(
        edit::read_metadata(&vault, "alpha")
            .unwrap()
            .classification_context
            .is_none()
    );

    // The 0.3 definitions survived every 0.2 write, and 0.3 still works.
    assert_eq!(vault.config().unwrap().definitions, definitions);
    assert!(run(new, &s, &["doctor"], 0)["healthy"].as_bool().unwrap());
    assert_eq!(run(new, &s, &["list"], 0).as_array().unwrap().len(), 2);
}
