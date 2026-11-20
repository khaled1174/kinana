mod support;
use kinana_core::{
    ImportOptions, catalog,
    import::{self, ImportControl},
    sources,
    vault::Vault,
};
use std::{fs, path::PathBuf};
struct Fixture {
    root: PathBuf,
    vault: Vault,
    source: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = support::temp_dir().unwrap();
        let vault = Vault::new(root.join("vault"), root.join("index.db"));
        vault.init(false).unwrap();
        let source = root.join("source");
        fs::create_dir(&source).unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\nname: reveal-safe\ndescription: Inspect data only\n---\n# Inspect\nData only.\n",
        )
        .unwrap();
        import::add(
            &vault,
            source.to_str().unwrap(),
            &root.join("scratch"),
            &ImportOptions::default(),
        )
        .unwrap();
        Self {
            root,
            vault,
            source,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn reveal_resolves_only_stored_real_directories_and_never_child_files() {
    let f = Fixture::new();
    let folder = f.vault.root.join("skills/reveal-safe");
    assert_eq!(
        catalog::skill_directory(&f.vault, "reveal-safe").unwrap(),
        folder
    );
    for id in [
        "../source",
        "/tmp/skill.command",
        "reveal-safe/SKILL.md",
        "https://example.invalid",
        "unknown",
    ] {
        assert!(catalog::skill_directory(&f.vault, id).is_err(), "{id}");
    }
    #[cfg(unix)]
    {
        let outside = f.root.join("outside");
        fs::rename(&folder, &outside).unwrap();
        std::os::unix::fs::symlink(&outside, &folder).unwrap();
        assert!(catalog::skill_directory(&f.vault, "reveal-safe").is_err());
        assert!(outside.join("SKILL.md").exists());
    }
}
#[test]
fn cancelled_preparation_and_import_never_publish_or_create_scratch() {
    let f = Fixture::new();
    let before = fs::read(f.vault.root.join("meta/reveal-safe.toml")).unwrap();
    let control = ImportControl::default();
    assert!(control.cancel());
    assert_eq!(
        sources::prepare_controlled(
            f.source.to_str().unwrap(),
            &f.root.join("new-scratch"),
            &control
        )
        .unwrap_err()
        .code,
        "CANCELLED"
    );
    assert!(!f.root.join("new-scratch").exists());
    let prepared = sources::prepare(f.source.to_str().unwrap(), &f.root.join("scratch")).unwrap();
    assert_eq!(
        import::add_prepared(
            &f.vault,
            f.source.to_str().unwrap(),
            &prepared,
            &ImportOptions::default(),
            Some(&control)
        )
        .unwrap_err()
        .code,
        "CANCELLED"
    );
    assert_eq!(
        fs::read(f.vault.root.join("meta/reveal-safe.toml")).unwrap(),
        before
    );
    let committing = ImportControl::default();
    committing.stage("committing").unwrap();
    assert!(!committing.cancel());
    assert!(committing.check().is_ok());
}
#[test]
fn preview_parts_preserve_every_byte_of_the_read_skill() {
    let f = Fixture::new();
    let report = catalog::detail(&f.vault, "reveal-safe").unwrap();
    assert_eq!(
        format!("{}{}", report.frontmatter, report.markdown_body),
        report.detail.raw_skill_md
    );
}

#[test]
fn prepare_rejects_invalid_vault_before_any_scratch_write() {
    let f = Fixture::new();
    let missing = Vault::new(
        f.root.join("missing-vault"),
        f.root.join("missing-index.db"),
    );
    assert!(
        import::prepare_source(
            &missing,
            f.source.to_str().unwrap(),
            &f.root.join("never-created"),
            &ImportControl::default()
        )
        .is_err()
    );
    assert!(!f.root.join("never-created").exists());
    assert!(!missing.root.exists());
}

#[test]
fn cancelled_classification_never_saves_keyword_fallback_or_changes_dist() {
    let f = Fixture::new();
    let before = fs::read(f.vault.root.join("meta/reveal-safe.toml")).unwrap();
    let source = fs::read(f.vault.root.join("skills/reveal-safe/SKILL.md")).unwrap();
    let control = kinana_core::classify::ClassifyControl::default();
    assert!(control.cancel());
    let result = kinana_core::classify::classify_with_control(
        &f.vault,
        &kinana_core::ClassifyOptions {
            ids: vec!["reveal-safe".into()],
            agent: "claude".into(),
            ..Default::default()
        },
        &control,
    );
    assert_eq!(result.err().unwrap().code, "CANCELLED");
    assert_eq!(
        fs::read(f.vault.root.join("meta/reveal-safe.toml")).unwrap(),
        before
    );
    assert_eq!(
        fs::read(f.vault.root.join("skills/reveal-safe/SKILL.md")).unwrap(),
        source
    );
    let item = catalog::list(&f.vault, &Default::default())
        .unwrap()
        .items
        .remove(0);
    assert_eq!(item.classified_by, "pending");
}

#[test]
fn agent_confirmation_and_estimate_are_rechecked_in_core_before_any_write() {
    let f = Fixture::new();
    let before = fs::read(f.vault.root.join("meta/reveal-safe.toml")).unwrap();
    let mut options = kinana_core::ClassifyOptions {
        ids: vec!["reveal-safe".into()],
        agent: "claude".into(),
        dry_run: true,
        ..Default::default()
    };
    let mut estimate = kinana_core::classify::classify(&f.vault, &options)
        .unwrap()
        .estimated_tokens;
    options.dry_run = false;
    let c = kinana_core::classify::ClassifyControl::default();
    assert_eq!(
        kinana_core::classify::classify_confirmed(&f.vault, &options, &estimate, &c)
            .err()
            .unwrap()
            .code,
        "CLASSIFY_CONFIRMATION_REQUIRED"
    );
    options.confirmed = true;
    estimate.total_input_tokens += 1;
    assert_eq!(
        kinana_core::classify::classify_confirmed(&f.vault, &options, &estimate, &c)
            .err()
            .unwrap()
            .code,
        "CLASSIFY_ESTIMATE_CHANGED"
    );
    assert_eq!(
        fs::read(f.vault.root.join("meta/reveal-safe.toml")).unwrap(),
        before
    );
    assert!(!f.vault.index_path.parent().unwrap().join("tmp").exists());
}
