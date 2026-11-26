#[allow(dead_code)]
mod support;

use kinana_core::{
    Bundle, BundleSkill, ClassificationProposal, agents, edit, gitlog, vault::Vault,
};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use support::{Sandbox, copy_tree, snapshot};

type TreeSnapshot = Vec<(PathBuf, Vec<u8>)>;

fn output_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "Invalid JSON ({error}): stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn success(sandbox: &Sandbox, args: &[&str]) -> Value {
    let output = sandbox.run(args);
    assert!(
        output.status.success(),
        "{args:?}: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output_json(&output)
}

fn text_success(sandbox: &Sandbox, args: &[&str]) -> String {
    let output = sandbox.run(args);
    assert!(
        output.status.success(),
        "{args:?}: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn failure_without_writes(sandbox: &Sandbox, args: &[&str], status: i32, code: &str) {
    let before = snapshot(&sandbox.root);
    let head = gitlog::head(&sandbox.vault).unwrap();
    let output = sandbox.run(args);
    assert_eq!(
        output.status.code(),
        Some(status),
        "{args:?}: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output_json(&output)["error"]["code"], code, "{args:?}");
    assert_eq!(snapshot(&sandbox.root), before, "{args:?} mutated files");
    assert_eq!(gitlog::head(&sandbox.vault).unwrap(), head);
}

fn vault(sandbox: &Sandbox) -> Vault {
    Vault::new(
        sandbox.vault.clone(),
        sandbox.home.join(".kinana/index-v2.db"),
    )
}

struct Fixture {
    sandbox: Sandbox,
    sources: Vec<(PathBuf, TreeSnapshot)>,
    originals: TreeSnapshot,
    generated: TreeSnapshot,
}

impl Fixture {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        sandbox.initialize();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/scan/projects/project-a");
        let mut sources = Vec::new();
        for (id, relative) in [
            ("api-contract", ".agents/skills/api-contract"),
            ("frontend-review", ".claude/skills/frontend-review"),
        ] {
            let source = sandbox.root.join(format!("source-{id}"));
            // Copy actual, inert repository fixtures before importing them.
            copy_tree(&fixtures.join(relative), &source);
            sources.push(import_source(&sandbox, id, source));
        }
        let source = sandbox.root.join("source-api-alternate");
        fs::create_dir_all(source.join("assets/empty")).unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\r\nname: api-alternate\r\ndescription: Review alternative API contracts\r\n---\r\n\r\n# Inert review data\r\nUnicode مرحبا\r\nNo trailing newline",
        )
        .unwrap();
        fs::write(source.join("assets/payload.dat"), [0, 1, 255, 13, 10]).unwrap();
        sources.push(import_source(&sandbox, "api-alternate", source));
        let originals = snapshot(&sandbox.vault.join("skills"));
        let generated = snapshot(&sandbox.vault.join("dist"));
        Self {
            sandbox,
            sources,
            originals,
            generated,
        }
    }

    fn assert_originals(&self) {
        for (source, before) in &self.sources {
            assert_eq!(snapshot(source), *before);
        }
        assert_eq!(snapshot(&self.sandbox.vault.join("skills")), self.originals);
    }

    fn assert_bundle_only(&self) {
        self.assert_originals();
        assert_eq!(snapshot(&self.sandbox.vault.join("dist")), self.generated);
    }
}

fn import_source(sandbox: &Sandbox, id: &str, source: PathBuf) -> (PathBuf, TreeSnapshot) {
    let before = snapshot(&source);
    let report = success(sandbox, &["add", source.to_str().unwrap(), "--json"]);
    assert_eq!(report["imported"], json!([id]));
    assert_eq!(report["items"][0]["audit"]["level"], "clean");
    assert_eq!(snapshot(&source), before);
    assert_eq!(snapshot(&sandbox.vault.join("skills").join(id)), before);
    (source, before)
}

fn member(id: &str, role: &str, group: Option<&str>) -> BundleSkill {
    BundleSkill {
        id: id.into(),
        role: role.into(),
        group: group.map(str::to_owned),
    }
}

fn bundle(name: &str) -> Bundle {
    Bundle {
        name: name.into(),
        title: "Application reviews — مراجعة التطبيق".into(),
        agents: vec!["claude".into(), "codex".into()],
        skills: vec![
            member("api-contract", "primary", Some("api-review")),
            member("api-alternate", "secondary", Some("api-review")),
            member("frontend-review", "primary", None),
        ],
    }
}

fn write_bundle(sandbox: &Sandbox, label: &str, value: &Bundle) -> PathBuf {
    let path = sandbox.root.join(format!("{label}.toml"));
    fs::write(&path, toml::to_string_pretty(value).unwrap()).unwrap();
    path
}

fn git(sandbox: &Sandbox, args: &[&str]) -> String {
    let output = Command::new(agents::executable_in_path("git").unwrap())
        .arg("-C")
        .arg(&sandbox.vault)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_COUNT", "0")
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}");
    String::from_utf8(output.stdout).unwrap().trim().into()
}

fn commit_count(sandbox: &Sandbox) -> usize {
    git(sandbox, &["rev-list", "--count", "HEAD"])
        .parse()
        .unwrap()
}

fn assert_bundle_commit(sandbox: &Sandbox, parent: &str, subject: &str, name: &str) {
    assert_eq!(git(sandbox, &["rev-parse", "HEAD^"]), parent);
    assert_eq!(git(sandbox, &["log", "-1", "--format=%s"]), subject);
    assert_eq!(
        git(
            sandbox,
            &["diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"]
        ),
        format!("bundles/{name}.toml")
    );
    gitlog::ensure_clean(&sandbox.vault).unwrap();
}

fn group_rows(sandbox: &Sandbox) -> Vec<(String, String)> {
    let connection = Connection::open_with_flags(
        sandbox.home.join(".kinana/index-v2.db"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let mut query = connection
        .prepare(
            "SELECT group_name,skill_id FROM groups WHERE vault=?1 ORDER BY group_name,skill_id",
        )
        .unwrap();
    query
        .query_map([sandbox.vault.to_str().unwrap()], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

fn expected_groups(names: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut rows: Vec<_> = names
        .iter()
        .flat_map(|(name, group)| {
            ["api-contract", "api-alternate"]
                .into_iter()
                .map(move |id| (format!("{name}/{group}"), id.to_owned()))
        })
        .collect();
    rows.sort();
    rows
}

#[test]
fn bundle_commands_preserve_order_priorities_and_exact_history_through_noop_and_undo() {
    let fixture = Fixture::new();
    let sandbox = &fixture.sandbox;
    assert_eq!(commit_count(sandbox), 4, "init plus three real imports");
    assert!(group_rows(sandbox).is_empty());
    let original = bundle("web-stack");
    let file = write_bundle(sandbox, "create-input", &original);
    let before = snapshot(&sandbox.root);
    let dry = success(
        sandbox,
        &[
            "--json",
            "bundle",
            "create",
            "--file",
            file.to_str().unwrap(),
            "--dry-run",
        ],
    );
    assert_eq!(dry["operation"], "create");
    assert_eq!(dry["dry_run"], true);
    assert_eq!(dry["changed"], true);
    assert_eq!(dry["committed"], false);
    assert_eq!(dry["bundle"], serde_json::to_value(&original).unwrap());
    assert_eq!(snapshot(&sandbox.root), before);
    let text = text_success(
        sandbox,
        &[
            "bundle",
            "create",
            "--file",
            file.to_str().unwrap(),
            "--dry-run",
        ],
    );
    assert!(text.contains("Would create: web-stack"));
    assert_eq!(snapshot(&sandbox.root), before);

    let parent = gitlog::head(&sandbox.vault).unwrap();
    let created = success(
        sandbox,
        &[
            "bundle",
            "create",
            "--file",
            file.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(created["committed"], true);
    assert_eq!(created["changed"], true);
    assert_eq!(created["dry_run"], false);
    assert_eq!(created["bundle"], serde_json::to_value(&original).unwrap());
    assert_eq!(
        created["path"],
        json!(sandbox.vault.join("bundles/web-stack.toml"))
    );
    assert_bundle_commit(sandbox, &parent, "Create bundle web-stack", "web-stack");
    assert_eq!(commit_count(sandbox), 5);
    let original_bytes = fs::read(sandbox.vault.join("bundles/web-stack.toml")).unwrap();
    assert_eq!(
        group_rows(sandbox),
        expected_groups(&[("web-stack", "api-review")])
    );

    // Priorities belong to a bundle: the same pair may choose the other primary.
    let mut alternate = bundle("alternate-stack");
    alternate.skills[0].role = "secondary".into();
    alternate.skills[1].role = "primary".into();
    let alternate_file = write_bundle(sandbox, "alternate-input", &alternate);
    let parent = gitlog::head(&sandbox.vault).unwrap();
    let text = text_success(
        sandbox,
        &[
            "bundle",
            "create",
            "--file",
            alternate_file.to_str().unwrap(),
        ],
    );
    assert!(text.contains("Bundle create: alternate-stack"));
    assert_bundle_commit(
        sandbox,
        &parent,
        "Create bundle alternate-stack",
        "alternate-stack",
    );
    assert_eq!(commit_count(sandbox), 6);
    let expected = expected_groups(&[
        ("web-stack", "api-review"),
        ("alternate-stack", "api-review"),
    ]);
    assert_eq!(group_rows(sandbox), expected);

    let before = snapshot(&sandbox.root);
    let listed = success(sandbox, &["bundle", "list", "--json", "--dry-run"]);
    let names: Vec<_> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["alternate-stack", "web-stack"]);
    let shown = success(sandbox, &["bundle", "show", "web-stack", "--json"]);
    let clean_view = |bundle: &Bundle| {
        let mut view = serde_json::to_value(bundle).unwrap();
        view["risky_members"] = 0.into();
        for member in view["skills"].as_array_mut().unwrap() {
            member["audit_level"] = "clean".into();
        }
        view
    };
    assert_eq!(shown, clean_view(&original));
    let text = text_success(sandbox, &["bundle", "list"]);
    assert!(text.contains("web-stack\t"));
    assert!(text.contains("alternate-stack\t"));
    let text = text_success(sandbox, &["bundle", "show", "web-stack"]);
    assert!(text.contains("api-contract\tprimary\tapi-review"));
    assert!(text.contains("api-alternate\tsecondary\tapi-review"));
    assert!(text.contains("Agents: claude, codex"));
    assert_eq!(snapshot(&sandbox.root), before);

    let mut changed = original.clone();
    changed.title = "Updated application reviews".into();
    changed.skills = vec![
        member("frontend-review", "primary", None),
        member("api-alternate", "primary", Some("api-contracts")),
        member("api-contract", "secondary", Some("api-contracts")),
    ];
    let edit_file = write_bundle(sandbox, "edit-input", &changed);
    let before = snapshot(&sandbox.root);
    let preview = success(
        sandbox,
        &[
            "bundle",
            "edit",
            "web-stack",
            "--file",
            edit_file.to_str().unwrap(),
            "--json",
            "--dry-run",
        ],
    );
    assert_eq!(preview["operation"], "edit");
    assert_eq!(preview["bundle"], serde_json::to_value(&changed).unwrap());
    assert_eq!(preview["changed"], true);
    assert_eq!(preview["committed"], false);
    let text = text_success(
        sandbox,
        &[
            "bundle",
            "edit",
            "web-stack",
            "--file",
            edit_file.to_str().unwrap(),
            "--dry-run",
        ],
    );
    assert!(text.contains("Would edit: web-stack"));
    assert_eq!(snapshot(&sandbox.root), before);
    let parent = gitlog::head(&sandbox.vault).unwrap();
    let edited = success(
        sandbox,
        &[
            "bundle",
            "edit",
            "web-stack",
            "--file",
            edit_file.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(edited["committed"], true);
    assert_eq!(
        edited["bundle"]["skills"],
        serde_json::to_value(&changed.skills).unwrap()
    );
    assert_bundle_commit(sandbox, &parent, "Edit bundle web-stack", "web-stack");
    assert_eq!(commit_count(sandbox), 7);
    assert_eq!(
        group_rows(sandbox),
        expected_groups(&[
            ("web-stack", "api-contracts"),
            ("alternate-stack", "api-review")
        ])
    );

    let stored = fs::read(sandbox.vault.join("bundles/web-stack.toml")).unwrap();
    fs::write(
        &edit_file,
        format!(
            "# Equivalent data, different formatting\n{}",
            toml::to_string(&changed).unwrap()
        ),
    )
    .unwrap();
    let before = snapshot(&sandbox.root);
    let no_op = success(
        sandbox,
        &[
            "bundle",
            "edit",
            "web-stack",
            "--file",
            edit_file.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(no_op["changed"], false);
    assert_eq!(no_op["committed"], false);
    let text = text_success(
        sandbox,
        &[
            "bundle",
            "edit",
            "web-stack",
            "--file",
            edit_file.to_str().unwrap(),
        ],
    );
    assert!(text.contains("Changed: false  Committed: false"));
    assert_eq!(snapshot(&sandbox.root), before);
    assert_eq!(
        fs::read(sandbox.vault.join("bundles/web-stack.toml")).unwrap(),
        stored
    );
    assert_eq!(commit_count(sandbox), 7);

    let edit_head = gitlog::head(&sandbox.vault).unwrap();
    let before = snapshot(&sandbox.root);
    let preview = success(sandbox, &["undo", "--dry-run", "--json"]);
    assert_eq!(preview["reverted_commit"], edit_head);
    assert_eq!(snapshot(&sandbox.root), before);
    let undone = success(sandbox, &["undo", "--json"]);
    assert_eq!(undone["reverted_commit"], edit_head);
    assert_eq!(git(sandbox, &["rev-parse", "HEAD^"]), edit_head);
    assert_eq!(commit_count(sandbox), 8);
    assert_eq!(
        fs::read(sandbox.vault.join("bundles/web-stack.toml")).unwrap(),
        original_bytes
    );
    assert_eq!(
        success(sandbox, &["bundle", "show", "web-stack", "--json"]),
        clean_view(&original)
    );
    assert_eq!(group_rows(sandbox), expected);
    assert_eq!(
        success(sandbox, &["bundle", "show", "alternate-stack", "--json"]),
        clean_view(&alternate)
    );
    fixture.assert_bundle_only();
    gitlog::ensure_clean(&sandbox.vault).unwrap();
}

#[test]
fn invalid_bundle_definitions_return_typed_errors_without_writes() {
    let fixture = Fixture::new();
    let sandbox = &fixture.sandbox;
    let mut cases = Vec::new();
    for name in [
        "",
        "../escape",
        "/outside",
        "two/parts",
        "..\\escape",
        "Uppercase",
    ] {
        let mut value = bundle(name);
        value.title = "Invalid name".into();
        cases.push((value, "INVALID_BUNDLE_NAME"));
    }
    let mut value = bundle("empty-agents");
    value.agents.clear();
    cases.push((value, "INVALID_BUNDLE"));
    let mut value = bundle("unknown-agent");
    value.agents = vec!["unknown".into()];
    cases.push((value, "INVALID_BUNDLE"));
    let mut value = bundle("duplicate-agent");
    value.agents = vec!["claude".into(), "claude".into()];
    cases.push((value, "INVALID_BUNDLE"));
    let mut value = bundle("duplicate-member");
    value.skills.push(value.skills[0].clone());
    cases.push((value, "INVALID_BUNDLE"));
    let mut value = bundle("missing-member");
    value.skills[0].id = "not-imported".into();
    cases.push((value, "SKILL_NOT_FOUND"));
    let mut value = bundle("unsafe-member");
    value.skills[0].id = "../api-contract".into();
    cases.push((value, "INVALID_SKILL_ID"));
    let mut value = bundle("invalid-role");
    value.skills[0].role = "recommended".into();
    cases.push((value, "INVALID_BUNDLE"));
    let mut value = bundle("no-primary");
    value.skills[0].role = "secondary".into();
    cases.push((value, "INVALID_BUNDLE"));
    let mut value = bundle("two-primaries");
    value.skills[1].role = "primary".into();
    cases.push((value, "INVALID_BUNDLE"));
    for group in ["", "../group", "two/parts"] {
        let mut value = bundle("unsafe-group");
        value.skills[0].group = Some(group.into());
        cases.push((value, "INVALID_BUNDLE"));
    }
    for (index, (value, code)) in cases.into_iter().enumerate() {
        let file = write_bundle(sandbox, &format!("invalid-{index}"), &value);
        failure_without_writes(
            sandbox,
            &[
                "bundle",
                "create",
                "--file",
                file.to_str().unwrap(),
                "--json",
            ],
            1,
            code,
        );
        failure_without_writes(
            sandbox,
            &[
                "bundle",
                "create",
                "--file",
                file.to_str().unwrap(),
                "--json",
                "--dry-run",
            ],
            1,
            code,
        );
    }
    let valid = toml::to_string(&bundle("strict-fields")).unwrap();
    for (index, invalid) in [
        "name = [\n".to_owned(),
        format!("unexpected = true\n{valid}"),
        format!("{valid}\nunexpected = true\n"),
        valid.replace("role = \"primary\"", "role = 7"),
    ]
    .into_iter()
    .enumerate()
    {
        let file = sandbox.root.join(format!("invalid-toml-{index}.toml"));
        fs::write(&file, invalid).unwrap();
        failure_without_writes(
            sandbox,
            &[
                "bundle",
                "create",
                "--file",
                file.to_str().unwrap(),
                "--json",
            ],
            1,
            "INVALID_TOML",
        );
    }
    failure_without_writes(
        sandbox,
        &["bundle", "show", "missing", "--json"],
        1,
        "BUNDLE_NOT_FOUND",
    );
    failure_without_writes(
        sandbox,
        &["bundle", "show", "../escape", "--json"],
        1,
        "INVALID_BUNDLE_NAME",
    );
    failure_without_writes(
        sandbox,
        &["bundle", "create", "--json"],
        1,
        "INVALID_ARGUMENTS",
    );
    let missing_file = sandbox.root.join("missing-input.toml");
    failure_without_writes(
        sandbox,
        &[
            "bundle",
            "create",
            "--file",
            missing_file.to_str().unwrap(),
            "--json",
        ],
        1,
        "BUNDLE_INPUT_NOT_FOUND",
    );
    let missing_bundle = write_bundle(sandbox, "missing-bundle-input", &bundle("missing"));
    failure_without_writes(
        sandbox,
        &[
            "bundle",
            "edit",
            "missing",
            "--file",
            missing_bundle.to_str().unwrap(),
            "--json",
        ],
        1,
        "BUNDLE_NOT_FOUND",
    );
    failure_without_writes(
        sandbox,
        &[
            "bundle",
            "edit",
            "renamed",
            "--file",
            missing_bundle.to_str().unwrap(),
            "--json",
        ],
        1,
        "INVALID_BUNDLE",
    );
    assert!(group_rows(sandbox).is_empty());
    fixture.assert_bundle_only();
}

#[test]
fn existing_bundle_and_foreign_toml_are_never_overwritten_by_create() {
    let fixture = Fixture::new();
    let sandbox = &fixture.sandbox;
    let mut value = bundle("existing");
    let file = write_bundle(sandbox, "existing-input", &value);
    success(
        sandbox,
        &[
            "bundle",
            "create",
            "--file",
            file.to_str().unwrap(),
            "--json",
        ],
    );
    let stored = fs::read(sandbox.vault.join("bundles/existing.toml")).unwrap();
    value.title = "Do not replace the existing bundle".into();
    write_bundle(sandbox, "existing-input", &value);
    for dry in [false, true] {
        let mut args = vec![
            "bundle",
            "create",
            "--file",
            file.to_str().unwrap(),
            "--json",
        ];
        if dry {
            args.push("--dry-run");
        }
        failure_without_writes(sandbox, &args, 2, "BUNDLE_EXISTS");
    }
    assert_eq!(
        fs::read(sandbox.vault.join("bundles/existing.toml")).unwrap(),
        stored
    );

    let foreign = sandbox.vault.join("bundles/foreign.toml");
    fs::write(&foreign, b"Foreign application data\0not TOML\n").unwrap();
    gitlog::commit_paths(
        &sandbox.vault,
        "Fixture: preserve foreign data",
        &[PathBuf::from("bundles/foreign.toml")],
    )
    .unwrap();
    let file = write_bundle(sandbox, "foreign-input", &bundle("foreign"));
    failure_without_writes(
        sandbox,
        &[
            "bundle",
            "create",
            "--file",
            file.to_str().unwrap(),
            "--json",
        ],
        2,
        "BUNDLE_EXISTS",
    );
    failure_without_writes(
        sandbox,
        &[
            "bundle",
            "edit",
            "foreign",
            "--file",
            file.to_str().unwrap(),
            "--json",
        ],
        1,
        "INVALID_TOML",
    );
    failure_without_writes(
        sandbox,
        &["bundle", "show", "foreign", "--json"],
        1,
        "INVALID_TOML",
    );
    failure_without_writes(sandbox, &["bundle", "list", "--json"], 1, "INVALID_TOML");
    assert_eq!(
        fs::read(foreign).unwrap(),
        b"Foreign application data\0not TOML\n"
    );
    fixture.assert_bundle_only();
}

#[cfg(unix)]
#[test]
fn bundle_input_target_and_parent_symlinks_are_refused_without_touching_foreign_data() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let sandbox = &fixture.sandbox;
    let file = write_bundle(sandbox, "safe-input", &bundle("linked"));
    let input_link = sandbox.root.join("input-link.toml");
    symlink(&file, &input_link).unwrap();
    failure_without_writes(
        sandbox,
        &[
            "bundle",
            "create",
            "--file",
            input_link.to_str().unwrap(),
            "--json",
        ],
        1,
        "SYMLINK_PATH",
    );
    assert_eq!(fs::read_link(&input_link).unwrap(), file);

    let outside = sandbox.root.join("foreign-bundle.toml");
    fs::write(&outside, "foreign sentinel\n").unwrap();
    let target = sandbox.vault.join("bundles/linked.toml");
    symlink(&outside, &target).unwrap();
    failure_without_writes(
        sandbox,
        &[
            "bundle",
            "create",
            "--file",
            file.to_str().unwrap(),
            "--json",
        ],
        1,
        "SYMLINK_PATH",
    );
    failure_without_writes(
        sandbox,
        &["bundle", "show", "linked", "--json"],
        1,
        "SYMLINK_PATH",
    );
    assert_eq!(fs::read_link(&target).unwrap(), outside);
    assert_eq!(fs::read(&outside).unwrap(), b"foreign sentinel\n");
    fs::remove_file(&target).unwrap();

    let directory = sandbox.vault.join("bundles");
    let original = sandbox.root.join("original-bundles");
    fs::rename(&directory, &original).unwrap();
    let outside_directory = sandbox.root.join("foreign-bundles");
    fs::create_dir(&outside_directory).unwrap();
    fs::write(outside_directory.join("sentinel"), "preserve").unwrap();
    symlink(&outside_directory, &directory).unwrap();
    failure_without_writes(
        sandbox,
        &[
            "bundle",
            "create",
            "--file",
            file.to_str().unwrap(),
            "--json",
            "--dry-run",
        ],
        1,
        "SYMLINK_PATH",
    );
    failure_without_writes(sandbox, &["bundle", "list", "--json"], 1, "SYMLINK_PATH");
    assert_eq!(fs::read_link(&directory).unwrap(), outside_directory);
    assert_eq!(
        fs::read(outside_directory.join("sentinel")).unwrap(),
        b"preserve"
    );
    assert!(!outside_directory.join("linked.toml").exists());
    fs::remove_file(&directory).unwrap();
    fs::rename(original, directory).unwrap();
    fixture.assert_bundle_only();
}

fn candidate<'a>(report: &'a Value, left: &str, right: &str) -> Option<&'a Value> {
    report["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| {
            let ids = item["skill_ids"].as_array().unwrap();
            ids.len() == 2 && ids.contains(&json!(left)) && ids.contains(&json!(right))
        })
}

#[test]
fn groups_are_readonly_proposals_and_both_review_decisions_preserve_overlaps() {
    let fixture = Fixture::new();
    let sandbox = &fixture.sandbox;
    let vault = vault(sandbox);
    let before = snapshot(&sandbox.root);
    assert!(
        success(sandbox, &["groups", "--json"])["candidates"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(text_success(sandbox, &["groups"]).contains("No same-function candidates."));
    assert_eq!(snapshot(&sandbox.root), before);
    let mut updates = BTreeMap::new();
    for id in ["api-contract", "api-alternate", "frontend-review"] {
        let mut metadata = edit::read_metadata(&vault, id).unwrap();
        metadata.classification.category = if id == "frontend-review" {
            "web-frontend"
        } else {
            "backend-api"
        }
        .into();
        metadata.classification.role = "Review REST API contracts before implementation".into();
        metadata.classification.classified_by = "manual".into();
        if id == "api-contract" {
            metadata.classification.overlaps_with = vec!["frontend-review".into()];
        }
        updates.insert(id.into(), metadata);
    }
    assert!(
        edit::apply(
            &vault,
            &updates,
            "Fixture: candidate roles and overlaps",
            false
        )
        .unwrap()
        .committed
    );
    let before = snapshot(&sandbox.root);
    let report = success(sandbox, &["groups", "--json"]);
    let similar = candidate(&report, "api-contract", "api-alternate")
        .expect("same-category roles must propose a pair");
    assert_eq!(similar["category"], "backend-api");
    assert!(
        similar["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason.as_str().unwrap().contains("role"))
    );
    let explicit = candidate(&report, "api-contract", "frontend-review")
        .expect("persisted overlap must propose a pair");
    assert!(
        explicit["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason.as_str().unwrap().contains("overlap"))
    );
    assert!(
        candidate(&report, "api-alternate", "frontend-review").is_none(),
        "similar roles in different categories must not create a candidate"
    );
    let text = text_success(sandbox, &["groups"]);
    assert!(text.contains("api-contract"));
    assert!(text.contains("api-alternate"));
    assert!(text.contains("frontend-review"));
    assert_eq!(success(sandbox, &["--dry-run", "groups", "--json"]), report);
    assert_eq!(snapshot(&sandbox.root), before);
    assert!(group_rows(sandbox).is_empty());
    assert!(
        success(sandbox, &["bundle", "list", "--json"])
            .as_array()
            .unwrap()
            .is_empty()
    );

    for (id, decision, overlap) in [
        ("api-contract", "--reject", "api-alternate"),
        ("api-alternate", "--accept", "frontend-review"),
    ] {
        let mut metadata = edit::read_metadata(&vault, id).unwrap();
        let persistent = metadata.classification.overlaps_with.clone();
        metadata.proposal = Some(ClassificationProposal {
            category: "backend-api".into(),
            stack: vec![],
            stage: vec![],
            tags: vec!["review".into()],
            role: "Review REST API contracts before implementation".into(),
            when_to_use: "Before endpoint implementation".into(),
            when_not: "For visual design changes".into(),
            improved_description: "Before implementing endpoints, review REST API contracts."
                .into(),
            overlaps_with: vec![overlap.into()],
            confidence: 0.9,
            category_reason: String::new(),
        });
        assert!(
            edit::apply(
                &vault,
                &BTreeMap::from([(id.into(), metadata)]),
                "Fixture: legacy pending overlap proposal",
                false
            )
            .unwrap()
            .committed
        );
        let reviewed = success(sandbox, &["review", id, decision, "--json"]);
        assert_eq!(reviewed["committed"], true);
        let metadata = edit::read_metadata(&vault, id).unwrap();
        assert!(metadata.proposal.is_none());
        assert!(
            metadata
                .classification
                .overlaps_with
                .contains(&overlap.to_owned())
        );
        for previous in persistent {
            assert!(metadata.classification.overlaps_with.contains(&previous));
        }
        assert_eq!(
            metadata.description.active,
            if decision == "--accept" {
                "improved"
            } else {
                "original"
            }
        );
        let shown = success(sandbox, &["show", id, "--json"]);
        assert!(
            shown["metadata"]["classification"]["overlaps_with"]
                .as_array()
                .unwrap()
                .contains(&json!(overlap))
        );
    }
    let before = snapshot(&sandbox.root);
    let after = success(sandbox, &["groups", "--json"]);
    assert!(candidate(&after, "api-alternate", "frontend-review").is_some());
    assert_eq!(snapshot(&sandbox.root), before);
    assert!(
        group_rows(sandbox).is_empty(),
        "proposals must not become confirmed index groups"
    );
    assert!(
        fs::read_dir(sandbox.vault.join("bundles"))
            .unwrap()
            .next()
            .is_none()
    );
    fixture.assert_originals();
    gitlog::ensure_clean(&sandbox.vault).unwrap();
}
