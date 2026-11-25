mod support;
use kinana_core::{
    Audit, Bundle, BundleSkill, Classification, Description, SkillMetadata, SourceInfo, hash,
    index, search, suggest, vault::Vault,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

struct Fixture {
    root: PathBuf,
    vault: Vault,
}
impl Fixture {
    fn new() -> Self {
        let root = support::temp_dir().unwrap();
        let vault = Vault::new(root.join("vault"), root.join("state/index.db"));
        vault.init(false).unwrap();
        Self { root, vault }
    }
    fn seed(&self, id: &str, category: &str, description: &str, role: &str) {
        let path = self.vault.root.join("skills").join(id);
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join("SKILL.md"),
            format!("---\nname: {id}\ndescription: Original description\n---\nOriginal body\n"),
        )
        .unwrap();
        let metadata = SkillMetadata {
            proposal_source_changed: None,
            classification_context: None,
            pinned: false,
            id: id.into(),
            content_hash: hash::hash_directory(&path).unwrap(),
            added_at: "2026-11-14T00:00:00Z".into(),
            source: SourceInfo::default(),
            classification: Classification {
                category: category.into(),
                role: role.into(),
                tags: vec!["tag-only-secret".into()],
                ..Default::default()
            },
            description: Description {
                original: "Original description".into(),
                improved: Some(description.into()),
                active: "improved".into(),
                approved_at: Some("2026-11-14T00:00:00Z".into()),
            },
            audit: Audit::default(),
            empty_directories: vec![],
            proposal: None,
        };
        fs::write(
            self.vault.root.join("meta").join(format!("{id}.toml")),
            toml::to_string(&metadata).unwrap(),
        )
        .unwrap();
        self.rebuild();
    }
    /// Rewrite the classification text of a seeded skill, then refresh the cache.
    fn classify(&self, id: &str, tags: &[&str], when_to_use: &str, when_not: &str) {
        let path = self.vault.root.join("meta").join(format!("{id}.toml"));
        let mut metadata: SkillMetadata =
            toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        metadata.classification.tags = tags.iter().map(|tag| (*tag).to_string()).collect();
        metadata.classification.when_to_use = when_to_use.into();
        metadata.classification.when_not = when_not.into();
        fs::write(&path, toml::to_string(&metadata).unwrap()).unwrap();
        self.rebuild();
    }
    /// Give a seeded skill stack values, as a classifier or a manual edit would.
    fn stack(&self, id: &str, stack: &[&str]) {
        let path = self.vault.root.join("meta").join(format!("{id}.toml"));
        let mut metadata: SkillMetadata =
            toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        metadata.classification.stack = stack.iter().map(|value| (*value).to_string()).collect();
        fs::write(&path, toml::to_string(&metadata).unwrap()).unwrap();
    }
    fn rebuild(&self) {
        index::rebuild_for_vault(
            &self.vault.index_path,
            &self.vault.root,
            &self.vault.records().unwrap(),
        )
        .unwrap();
    }
    fn project(&self) -> PathBuf {
        let p = self.root.join("project");
        fs::create_dir(&p).unwrap();
        p
    }
    fn bundle(&self, name: &str, entries: &[(&str, &str)]) {
        let b = Bundle {
            name: name.into(),
            title: name.into(),
            agents: vec!["claude".into(), "codex".into()],
            skills: entries
                .iter()
                .map(|(id, role)| BundleSkill {
                    id: (*id).into(),
                    role: (*role).into(),
                    group: None,
                })
                .collect(),
        };
        fs::write(
            self.vault.root.join("bundles").join(format!("{name}.toml")),
            toml::to_string(&b).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn search_warns_only_when_results_exceed_the_cap_and_keeps_stable_order() {
    let f = Fixture::new();
    for number in 0..100 {
        f.seed(
            &format!("result-{number:03}"),
            "other",
            "Shared searchable phrase",
            "",
        );
    }
    let exactly = search::search(&f.vault, "searchable").unwrap();
    assert_eq!(exactly.items.len(), 100);
    assert!(exactly.warnings.is_empty());
    f.seed("result-100", "other", "Shared searchable phrase", "");
    let before = tree(&f.root);
    let capped = search::search(&f.vault, "searchable").unwrap();
    assert_eq!(capped.items.len(), 100);
    assert_eq!(capped.warnings.len(), 1);
    assert_eq!(capped.items.first().unwrap().id, "result-000");
    assert_eq!(capped.items.last().unwrap().id, "result-099");
    assert_eq!(tree(&f.root), before);
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, path: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for e in fs::read_dir(path).unwrap() {
            let p = e.unwrap().path();
            let m = fs::symlink_metadata(&p).unwrap();
            let bytes = if m.file_type().is_symlink() {
                fs::read_link(&p)
                    .unwrap()
                    .as_os_str()
                    .as_encoded_bytes()
                    .to_vec()
            } else if m.is_dir() {
                vec![]
            } else {
                fs::read(&p).unwrap()
            };
            out.insert(p.strip_prefix(root).unwrap().to_path_buf(), bytes);
            if m.is_dir() {
                visit(root, &p, out);
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}
fn ids(f: &Fixture, q: &str) -> Vec<String> {
    search::search(&f.vault, q)
        .unwrap()
        .items
        .into_iter()
        .map(|s| s.id)
        .collect()
}

#[test]
fn search_matches_names_approved_descriptions_and_roles_with_literal_cross_field_terms() {
    let f = Fixture::new();
    f.seed(
        "zeta",
        "backend-api",
        "Review invoices",
        "GraphQL contracts",
    );
    f.seed(
        "alpha",
        "web-frontend",
        "مراجعة الواجهات",
        "Accessibility review",
    );
    let before = tree(&f.root);
    assert_eq!(ids(&f, "zeta"), ["zeta"]);
    assert_eq!(ids(&f, "invoices"), ["zeta"]);
    assert_eq!(ids(&f, "GraphQL"), ["zeta"]);
    assert_eq!(ids(&f, "zeta GraphQL"), ["zeta"]);
    assert_eq!(ids(&f, "مراجعة"), ["alpha"]);
    assert!(ids(&f, "Original").is_empty());
    // Tags are searched since 8.3; both fixtures carry this one.
    assert_eq!(ids(&f, "tag-only-secret"), ["alpha", "zeta"]);
    assert!(ids(&f, "missing").is_empty());
    assert_eq!(tree(&f.root), before);
}

#[test]
fn search_covers_tags_and_when_to_use_but_never_when_not() {
    let f = Fixture::new();
    f.seed("payments", "backend-api", "Take card payments", "Billing");
    f.classify(
        "payments",
        &["stripe", "checkout-flow"],
        "Before wiring a subscription provider",
        "Never for cryptocurrency wallets",
    );
    f.seed("notes", "other", "Summarize meetings", "Notes");
    f.classify("notes", &["minutes"], "After a long call", "");
    let before = tree(&f.root);
    // A tag returns its skill, including a hyphenated tag as one phrase.
    assert_eq!(ids(&f, "stripe"), ["payments"]);
    assert_eq!(ids(&f, "checkout-flow"), ["payments"]);
    assert_eq!(ids(&f, "minutes"), ["notes"]);
    // So does a word that appears only in "when to use".
    assert_eq!(ids(&f, "subscription"), ["payments"]);
    assert_eq!(ids(&f, "call"), ["notes"]);
    // Terms are still combined with AND across fields.
    assert_eq!(ids(&f, "stripe subscription card"), ["payments"]);
    assert!(ids(&f, "stripe minutes").is_empty());
    // A skill must not surface for a request it says it does not fit.
    assert!(ids(&f, "cryptocurrency").is_empty());
    assert!(ids(&f, "wallets").is_empty());
    assert!(ids(&f, "stripe wallets").is_empty());
    assert_eq!(tree(&f.root), before);
    // The "when not" text is not stored in the index at all.
    let index = fs::read(&f.vault.index_path).unwrap();
    assert!(index.windows(6).any(|window| window == b"stripe"));
    assert!(!index.windows(14).any(|window| window == b"cryptocurrency"));
}

#[test]
fn search_ranks_a_name_above_a_tag_and_a_tag_above_when_to_use() {
    let f = Fixture::new();
    for (id, tags, when_to_use) in [
        ("when-match", &["alpha"][..], "Use for orbit planning"),
        ("tag-match", &["orbit"][..], "Use for route planning"),
        ("orbit", &["alpha"][..], "Use for route planning"),
    ] {
        f.seed(id, "other", "Shared description text", "Shared role");
        f.classify(id, tags, when_to_use, "");
    }
    assert_eq!(ids(&f, "orbit"), ["orbit", "tag-match", "when-match"]);
}

#[test]
fn a_changed_tag_or_when_to_use_makes_the_search_cache_stale() {
    let f = Fixture::new();
    f.seed("review", "backend-api", "Review invoices", "Contracts");
    f.classify("review", &["ledger"], "Before closing the month", "");
    assert_eq!(ids(&f, "month"), ["review"]);
    let path = f.vault.root.join("meta/review.toml");
    for (old, new) in [("month", "quarter"), ("ledger", "journal")] {
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains(old));
        fs::write(&path, text.replace(old, new)).unwrap();
        let before = tree(&f.root);
        let error = search::search(&f.vault, new).unwrap_err();
        assert_eq!(error.code, "INDEX_STALE");
        assert!(error.message.contains("doctor --rebuild-index"));
        assert_eq!(tree(&f.root), before);
        f.rebuild();
        assert_eq!(ids(&f, new), ["review"]);
        assert!(ids(&f, old).is_empty());
    }
}

#[test]
fn search_is_scoped_even_with_identical_ids_in_another_vault() {
    let f = Fixture::new();
    f.seed("shared", "backend-api", "First orchard", "Review contracts");
    let other = Fixture::new();
    other.seed("shared", "backend-api", "Foreign orchard", "Private role");
    index::rebuild_for_vault(
        &f.vault.index_path,
        &other.vault.root,
        &other.vault.records().unwrap(),
    )
    .unwrap();
    assert!(ids(&f, "Foreign").is_empty());
    assert_eq!(ids(&f, "orchard"), ["shared"]);
}

#[test]
fn user_search_text_cannot_inject_fts_operators_or_sql() {
    let f = Fixture::new();
    f.seed("review", "backend-api", "Review invoices", "Contracts");
    for q in [
        "review OR absent",
        "review\" OR \"absent",
        "name:absent",
        "review'); DROP TABLE skills; --",
    ] {
        assert!(ids(&f, q).is_empty(), "{q}");
    }
    for q in ["", " \t ", "***", "bad\0query"] {
        assert_eq!(
            search::search(&f.vault, q).unwrap_err().code,
            "INVALID_SEARCH_QUERY"
        );
    }
    assert_eq!(
        search::search(&f.vault, &"x".repeat(4097))
            .unwrap_err()
            .code,
        "INVALID_SEARCH_QUERY"
    );
    assert_eq!(
        search::search(&f.vault, &vec!["x"; 33].join(" "))
            .unwrap_err()
            .code,
        "INVALID_SEARCH_QUERY"
    );
    assert_eq!(ids(&f, "invoices"), ["review"]);
}

#[test]
fn stale_missing_and_foreign_search_indexes_fail_without_repair_or_writes() {
    let f = Fixture::new();
    f.seed("review", "backend-api", "Before", "Contracts");
    let path = f.vault.root.join("meta/review.toml");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.replace("Before", "After")).unwrap();
    let before = tree(&f.root);
    let err = search::search(&f.vault, "After").unwrap_err();
    assert_eq!(err.code, "INDEX_STALE");
    assert!(err.message.contains("doctor --rebuild-index"));
    assert_eq!(tree(&f.root), before);
    fs::remove_file(&f.vault.index_path).unwrap();
    let before = tree(&f.root);
    assert_eq!(
        search::search(&f.vault, "After").unwrap_err().code,
        "INDEX_MISSING"
    );
    assert_eq!(tree(&f.root), before);
    fs::write(&f.vault.index_path, b"foreign bytes").unwrap();
    let before = tree(&f.root);
    let err = search::search(&f.vault, "After").unwrap_err();
    assert!(err.needs_decision);
    assert_eq!(tree(&f.root), before);
}

#[test]
fn bundle_ranking_counts_distinct_primary_categories_and_excludes_secondary_only_matches() {
    let f = Fixture::new();
    f.seed("web", "web-frontend", "Web", "React");
    f.seed("web-two", "web-frontend", "Web", "React");
    f.seed("mobile", "mobile", "Mobile", "Expo");
    f.seed("ops", "devops", "Ops", "Docker");
    f.bundle("full", &[("web", "primary"), ("mobile", "primary")]);
    f.bundle(
        "duplicate-category",
        &[("web", "primary"), ("web-two", "primary")],
    );
    f.bundle("a-single", &[("web", "primary")]);
    f.bundle("secondary", &[("ops", "primary"), ("mobile", "secondary")]);
    let p = f.project();
    fs::write(
        p.join("package.json"),
        r#"{"dependencies":{"expo":"1","react":"19","next":"15"}}"#,
    )
    .unwrap();
    let before = tree(&f.root);
    let result = suggest::suggest(&f.vault, &p).unwrap();
    assert_eq!(result.categories, ["mobile", "web-frontend"]);
    assert_eq!(
        result
            .bundles
            .iter()
            .map(|b| b.bundle.name.as_str())
            .collect::<Vec<_>>(),
        ["full", "a-single", "duplicate-category"]
    );
    assert_eq!(result.bundles[0].match_count, 2);
    assert_eq!(result.bundles[2].match_count, 1);
    assert_eq!(tree(&f.root), before);
}

#[test]
fn mobile_dependencies_do_not_accidentally_add_web_and_empty_projects_show_all_bundles() {
    let f = Fixture::new();
    f.seed("mobile", "mobile", "Mobile", "Expo");
    f.bundle("z", &[("mobile", "primary")]);
    f.bundle("a", &[("mobile", "primary")]);
    let p = f.project();
    let empty = suggest::suggest(&f.vault, &p).unwrap();
    assert!(empty.categories.is_empty());
    assert!(empty.skills.is_empty());
    assert_eq!(
        empty
            .bundles
            .iter()
            .map(|b| b.bundle.name.as_str())
            .collect::<Vec<_>>(),
        ["a", "z"]
    );
    fs::write(
        p.join("package.json"),
        r#"{"dependencies":{"expo":"1","react":"19"}}"#,
    )
    .unwrap();
    assert_eq!(
        suggest::suggest(&f.vault, &p).unwrap().categories,
        ["mobile"]
    );
}

#[test]
fn manifests_and_native_directories_produce_explained_signals_without_running_scripts_or_needing_index()
 {
    let f = Fixture::new();
    f.seed("ops", "devops", "Ops", "Docker");
    let p = f.project();
    fs::write(p.join("Cargo.toml"), "[workspace]\nmembers=[]\n").unwrap();
    fs::write(p.join("pyproject.toml"), "[project]\nname='test'\n").unwrap();
    fs::write(
        p.join("pubspec.yaml"),
        "name: fixture\ndependencies:\n  flutter:\n    sdk: flutter\n",
    )
    .unwrap();
    fs::write(p.join("Dockerfile"), "RUN touch NEVER_EXECUTED\n").unwrap();
    fs::create_dir(p.join("ios")).unwrap();
    fs::create_dir(p.join("android")).unwrap();
    fs::remove_file(&f.vault.index_path).unwrap();
    let before = tree(&f.root);
    let result = suggest::suggest(&f.vault, &p).unwrap();
    assert_eq!(result.categories, ["backend-api", "devops", "mobile"]);
    assert_eq!(result.signals.len(), 6);
    assert_eq!(result.skills[0].id, "ops");
    assert_eq!(tree(&f.root), before);
}

#[test]
fn a_react_project_ranks_react_stack_skills_above_other_web_frontend_skills() {
    let f = Fixture::new();
    // Ids are chosen so that plain id order would put the generic skills first.
    for (id, category, stack) in [
        ("aaa-web-generic", "web-frontend", &[][..]),
        ("bbb-web-vue-only", "web-frontend", &["flutter"][..]),
        ("react-hooks", "web-frontend", &["react"][..]),
        ("react-next-routing", "web-frontend", &["react", "next"][..]),
        ("react-testing", "testing-qa", &["react"][..]),
        ("zzz-unrelated", "data-db", &["python"][..]),
    ] {
        f.seed(id, category, "Description", "Role");
        f.stack(id, stack);
    }
    let p = f.project();
    fs::write(
        p.join("package.json"),
        r#"{"dependencies":{"react":"19","react-dom":"19"}}"#,
    )
    .unwrap();
    let before = tree(&f.root);
    let result = suggest::suggest(&f.vault, &p).unwrap();
    assert_eq!(result.categories, ["web-frontend"]);
    assert_eq!(result.stacks, ["react", "node"], "configured-list order");
    let order = result
        .skills
        .iter()
        .map(|s| s.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        order,
        [
            // React stack and the detected domain.
            "react-hooks",
            "react-next-routing",
            // React stack from another domain still precedes domain-only matches.
            "react-testing",
            // Domain only: other web-frontend skills, whatever their own stack.
            "aaa-web-generic",
            "bbb-web-vue-only",
        ]
    );
    // Why each skill is listed, in the same order as `skills`.
    let matches = result
        .matches
        .iter()
        .map(|m| (m.id.as_str(), m.matched_stacks.clone(), m.matched_category))
        .collect::<Vec<_>>();
    assert_eq!(
        matches,
        [
            ("react-hooks", vec!["react".to_string()], true),
            ("react-next-routing", vec!["react".to_string()], true),
            ("react-testing", vec!["react".to_string()], false),
            ("aaa-web-generic", vec![], true),
            ("bbb-web-vue-only", vec![], true),
        ]
    );
    assert_eq!(result.skills[0].stack, ["react"]);
    assert_eq!(tree(&f.root), before);
    // More matching stacks rank higher once Next.js is a dependency too.
    fs::write(
        p.join("package.json"),
        r#"{"dependencies":{"react":"19","next":"15"}}"#,
    )
    .unwrap();
    let result = suggest::suggest(&f.vault, &p).unwrap();
    assert_eq!(result.stacks, ["react", "next", "node"]);
    assert_eq!(result.skills[0].id, "react-next-routing");
    assert_eq!(result.matches[0].matched_stacks, ["react", "next"]);
    assert_eq!(result.skills[1].id, "react-hooks");
}

#[test]
fn manifest_signals_map_to_configured_stack_values_only() {
    let f = Fixture::new();
    let p = f.project();
    let stacks = |project: &Path| {
        let result = suggest::suggest(&f.vault, project).unwrap();
        // Every detected stack is explained by at least one signal.
        for stack in &result.stacks {
            assert!(
                result.stack_signals.iter().any(|s| &s.stack == stack
                    && !s.reason.is_empty()
                    && s.path.starts_with(project)),
                "{stack} has no signal"
            );
        }
        assert!(
            result.stack_signals.len() >= result.stacks.len(),
            "{:?}",
            result.stack_signals
        );
        (result.stacks, result.categories, result.warnings)
    };
    assert_eq!(stacks(&p).0, Vec::<String>::new());

    fs::write(
        p.join("package.json"),
        r#"{"devDependencies":{"expo":"1","react":"19","react-native":"0.8"}}"#,
    )
    .unwrap();
    let (found, categories, _) = stacks(&p);
    assert_eq!(found, ["react", "expo", "react-native", "node"]);
    // The domain signals keep their old meaning: Expo alone is not web-frontend.
    assert_eq!(categories, ["mobile"]);

    fs::write(p.join("package.json"), r#"{"name":"tool"}"#).unwrap();
    assert_eq!(stacks(&p).0, ["node"]);
    fs::write(p.join("package.json"), "[").unwrap();
    assert_eq!(stacks(&p).0, Vec::<String>::new());
    fs::remove_file(p.join("package.json")).unwrap();

    fs::write(p.join("Cargo.toml"), "[package]\nname='x'\n").unwrap();
    fs::write(p.join("pyproject.toml"), "[project]\nname='x'\n").unwrap();
    fs::write(
        p.join("pubspec.yaml"),
        "name: x\ndependencies:\n  flutter:\n    sdk: flutter\n",
    )
    .unwrap();
    fs::write(p.join("Dockerfile"), "RUN touch NEVER_EXECUTED\n").unwrap();
    let (found, _, warnings) = stacks(&p);
    assert_eq!(found, ["flutter", "rust", "python", "docker"]);
    assert!(warnings.is_empty(), "{warnings:?}");

    // Swift and Xcode markers are names in the project root; nothing is opened.
    for marker in ["Package.swift", "App.xcodeproj", "App.xcworkspace"] {
        let swift = f.root.join(format!("swift-{marker}"));
        fs::create_dir(&swift).unwrap();
        if marker.ends_with(".swift") {
            fs::write(swift.join(marker), "// swift-tools-version:6.0\n").unwrap();
        } else {
            fs::create_dir(swift.join(marker)).unwrap();
        }
        let before = tree(&f.root);
        assert_eq!(stacks(&swift).0, ["swift"], "{marker}");
        assert_eq!(tree(&f.root), before);
    }
    // A file named like an Xcode project is not one.
    let fake = f.root.join("not-swift");
    fs::create_dir(&fake).unwrap();
    fs::write(fake.join("App.xcodeproj"), "").unwrap();
    fs::create_dir(fake.join("Package.swift")).unwrap();
    assert_eq!(stacks(&fake).0, Vec::<String>::new());

    // Only values in the configured list are reported.
    let path = f.vault.root.join("box.toml");
    let mut config = f.vault.config().unwrap();
    config.stacks = vec!["rust".into(), "kotlin".into()];
    fs::write(&path, toml::to_string_pretty(&config).unwrap()).unwrap();
    assert_eq!(stacks(&p).0, ["rust"]);
}

#[cfg(unix)]
#[test]
fn a_symlinked_xcode_marker_is_reported_and_not_followed() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let p = f.project();
    let real = f.root.join("Real.xcodeproj");
    fs::create_dir(&real).unwrap();
    symlink(&real, p.join("App.xcodeproj")).unwrap();
    let result = suggest::suggest(&f.vault, &p).unwrap();
    assert!(result.stacks.is_empty());
    assert_eq!(result.warnings.len(), 1, "{:?}", result.warnings);
    assert!(result.warnings[0].contains("App.xcodeproj"));
}

#[test]
fn malformed_large_and_nonregular_manifests_are_skipped_with_diagnostics() {
    let f = Fixture::new();
    let p = f.project();
    fs::write(p.join("package.json"), "[]").unwrap();
    fs::write(p.join("Cargo.toml"), "[broken").unwrap();
    fs::write(p.join("pubspec.yaml"), "x: [").unwrap();
    fs::write(p.join("pyproject.toml"), vec![b'x'; 1024 * 1024 + 1]).unwrap();
    fs::create_dir(p.join("Dockerfile")).unwrap();
    let before = tree(&f.root);
    let result = suggest::suggest(&f.vault, &p).unwrap();
    assert!(result.categories.is_empty());
    assert_eq!(result.warnings.len(), 5);
    assert_eq!(tree(&f.root), before);
}

#[cfg(unix)]
#[test]
fn manifest_and_directory_symlinks_are_not_followed_and_symlink_projects_are_rejected() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let p = f.project();
    let external = f.root.join("external.json");
    fs::write(&external, r#"{"dependencies":{"react":"19"}}"#).unwrap();
    symlink(&external, p.join("package.json")).unwrap();
    symlink(&f.root, p.join("ios")).unwrap();
    let before = tree(&f.root);
    let result = suggest::suggest(&f.vault, &p).unwrap();
    assert!(result.categories.is_empty());
    assert_eq!(result.warnings.len(), 2);
    assert_eq!(tree(&f.root), before);
    let alias = f.root.join("alias");
    symlink(&p, &alias).unwrap();
    assert_eq!(
        suggest::suggest(&f.vault, &alias).unwrap_err().code,
        "SYMLINK_PATH"
    );
}
