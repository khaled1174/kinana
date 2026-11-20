//! `kinana list --brief` (8.4): the compact catalog an agent chooses from.
#[allow(dead_code)]
mod support;

use kinana_core::{
    Audit, Classification, Description, SkillMetadata, SourceInfo, hash::hash_directory,
};
use serde_json::{Value, json};
use std::fs;
use support::{Sandbox, snapshot};

fn output(s: &Sandbox, args: &[&str], exit: i32) -> (String, String) {
    let output = s.run(args);
    let (stdout, stderr) = (
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    );
    assert_eq!(
        output.status.code(),
        Some(exit),
        "{args:?}: {stdout} {stderr}"
    );
    (stdout, stderr)
}

fn brief(s: &Sandbox, filters: &[&str]) -> Value {
    let (stdout, _) = output(s, &[&["list", "--brief", "--json"], filters].concat(), 0);
    serde_json::from_str(&stdout).unwrap()
}

fn error_code(s: &Sandbox, filters: &[&str]) -> String {
    let (stdout, _) = output(s, &[&["list", "--brief", "--json"], filters].concat(), 1);
    let value: Value = serde_json::from_str(&stdout).unwrap();
    value["error"]["code"].as_str().unwrap().to_owned()
}

/// Write one stored skill and its metadata directly, as an earlier command would have.
fn seed(s: &Sandbox, id: &str, description: &str, classification: Classification) {
    let skill = s.vault.join("skills").join(id);
    fs::create_dir_all(&skill).unwrap();
    fs::write(
        skill.join("SKILL.md"),
        format!(
            "---\nname: {id}\ndescription: {}\n---\n# Body\nText.\n",
            serde_json::to_string(description).unwrap()
        ),
    )
    .unwrap();
    let metadata = SkillMetadata {
        proposal_source_changed: None,
        pinned: false,
        id: id.into(),
        content_hash: hash_directory(&skill).unwrap(),
        added_at: "2026-11-16T00:00:00Z".into(),
        source: SourceInfo::default(),
        classification,
        description: Description {
            original: description.into(),
            ..Description::default()
        },
        audit: Audit::default(),
        empty_directories: vec![],
        proposal: None,
    };
    fs::write(
        s.vault.join("meta").join(format!("{id}.toml")),
        toml::to_string(&metadata).unwrap(),
    )
    .unwrap();
}

fn classification(
    category: &str,
    stack: &[&str],
    stage: &[&str],
    role: &str,
    when_to_use: &str,
    when_not: &str,
) -> Classification {
    Classification {
        category: category.into(),
        stack: stack.iter().map(|value| (*value).to_owned()).collect(),
        stage: stage.iter().map(|value| (*value).to_owned()).collect(),
        role: role.into(),
        when_to_use: when_to_use.into(),
        when_not: when_not.into(),
        classified_by: "claude".into(),
        ..Classification::default()
    }
}

fn bundle(s: &Sandbox, name: &str, title: &str, members: &[(&str, &str)]) {
    let mut text = format!(
        "name = \"{name}\"\ntitle = {}\nagents = [\"claude\", \"codex\"]\n",
        toml::Value::String(title.to_owned())
    );
    if members.is_empty() {
        text.push_str("skills = []\n");
    }
    for (id, role) in members {
        text.push_str(&format!("[[skills]]\nid = \"{id}\"\nrole = \"{role}\"\n"));
    }
    fs::write(s.vault.join("bundles").join(format!("{name}.toml")), text).unwrap();
}

fn two_skills(s: &Sandbox) {
    s.initialize();
    seed(
        s,
        "api-review",
        "Review REST API contracts",
        classification(
            "backend-api",
            &["node"],
            &["design", "review"],
            "Reviews REST API designs",
            "Before writing new endpoints",
            "For visual design work",
        ),
    );
    seed(
        s,
        "form-kit",
        "Build accessible forms",
        classification(
            "web-frontend",
            &["react", "next"],
            &["build"],
            "Builds forms",
            "When adding a form",
            "For native mobile screens",
        ),
    );
    bundle(
        s,
        "web-kit",
        "Web kit",
        &[("form-kit", "primary"), ("api-review", "secondary")],
    );
}

#[test]
fn brief_json_has_the_fixed_shape_filters_like_list_and_writes_nothing() {
    let s = Sandbox::new();
    two_skills(&s);
    let before = snapshot(&s.root);
    assert_eq!(
        brief(&s, &[]),
        json!({
            "skills": [
                {"id": "api-review", "role": "Reviews REST API designs",
                 "when_to_use": "Before writing new endpoints", "when_not": "For visual design work",
                 "description": "Review REST API contracts", "category": "backend-api",
                 "stack": ["node"], "stage": ["design", "review"]},
                {"id": "form-kit", "role": "Builds forms",
                 "when_to_use": "When adding a form", "when_not": "For native mobile screens",
                 "description": "Build accessible forms", "category": "web-frontend",
                 "stack": ["react", "next"], "stage": ["build"]},
            ],
            "bundles": [
                {"name": "web-kit", "members": 2, "title": "Web kit", "skills": [
                    {"id": "form-kit", "role": "primary"},
                    {"id": "api-review", "role": "secondary"},
                ]},
            ],
            "bundles_total": 1,
            "total": 2,
            "limit": 20,
            "page": 1,
            "pages": 1,
            "hint": "",
        })
    );
    // The same filters as `list`, all required together; bundles stay whole.
    let react = brief(&s, &["--stack", "react"]);
    assert_eq!(react["skills"].as_array().unwrap().len(), 1);
    assert_eq!(react["skills"][0]["id"], "form-kit");
    assert_eq!(react["bundles"][0]["skills"].as_array().unwrap().len(), 2);
    assert_eq!(
        brief(&s, &["--stage", "review"])["skills"][0]["id"],
        "api-review"
    );
    assert_eq!(
        brief(
            &s,
            &[
                "--category",
                "backend-api",
                "--stack",
                "node",
                "--stage",
                "design",
                "--agent",
                "claude"
            ]
        )["skills"][0]["id"],
        "api-review"
    );
    assert_eq!(
        brief(&s, &["--category", "backend-api", "--stack", "react"])["skills"],
        json!([])
    );
    assert_eq!(brief(&s, &["--stack", "rust"])["skills"], json!([]));
    assert_eq!(error_code(&s, &["--stack", "reactjs"]), "INVALID_STACK");
    assert_eq!(error_code(&s, &["--stage", "deploy"]), "INVALID_STAGE");
    assert_eq!(snapshot(&s.root), before);
}

#[test]
fn brief_follows_the_order_of_list() {
    let s = Sandbox::new();
    s.initialize();
    for id in ["alpha", "beta", "gamma"] {
        let source = s.root.join(format!("source-{id}"));
        fs::create_dir(&source).unwrap();
        fs::write(
            source.join("SKILL.md"),
            format!("---\nname: {id}\ndescription: Describes {id}\n---\n# Body\n"),
        )
        .unwrap();
        output(&s, &["add", source.to_str().unwrap(), "--json"], 0);
    }
    output(&s, &["pin", "gamma", "--json"], 0);
    let (listed, _) = output(&s, &["list", "--json"], 0);
    let listed: Value = serde_json::from_str(&listed).unwrap();
    let order = |value: &Value| -> Vec<String> {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|skill| skill["id"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(order(&listed), ["gamma", "alpha", "beta"]);
    assert_eq!(order(&brief(&s, &[])["skills"]), order(&listed));
    // `--query` goes through the same literal search and keeps the list order.
    assert_eq!(
        order(&brief(&s, &["--query", "Describes"])["skills"]),
        order(&listed)
    );
    assert_eq!(order(&brief(&s, &["--query", "beta"])["skills"]), ["beta"]);
    assert_eq!(order(&brief(&s, &["--pinned"])["skills"]), ["gamma"]);
}

#[test]
fn keyword_classified_skills_have_the_description_as_role_and_no_when_text() {
    let s = Sandbox::new();
    s.initialize();
    let long = format!("Review React forms. {}", "Detail. ".repeat(60));
    let long = long.trim_end();
    for (id, description) in [("forms", long), ("waiting", "Never classified at all")] {
        let source = s.root.join(format!("source-{id}"));
        fs::create_dir(&source).unwrap();
        fs::write(
            source.join("SKILL.md"),
            format!("---\nname: {id}\ndescription: {description}\n---\n# Body\n"),
        )
        .unwrap();
        output(&s, &["add", source.to_str().unwrap(), "--json"], 0);
    }
    output(
        &s,
        &["classify", "forms", "--agent", "keywords", "--json"],
        0,
    );
    let report = brief(&s, &[]);
    let forms = &report["skills"][0];
    assert_eq!(forms["id"], "forms");
    // The keyword fallback copies the whole original description into `role`
    // and has no evidence for either "when" field. The brief catalog carries
    // that role cut like the description (review finding M3).
    assert_eq!(forms["role"], forms["description"]);
    assert_eq!(forms["when_to_use"], "");
    assert_eq!(forms["when_not"], "");
    assert_eq!(forms["stack"], json!(["react"]));
    let description = forms["description"].as_str().unwrap();
    assert_eq!(description.chars().count(), 200);
    assert!(description.ends_with('…'));
    assert!(long.starts_with(description.trim_end_matches('…')));
    // A skill that was never classified has neither a role nor "when" text.
    let waiting = &report["skills"][1];
    assert_eq!(waiting["id"], "waiting");
    assert_eq!(waiting["role"], "");
    assert_eq!(waiting["when_to_use"], "");
    assert_eq!(waiting["description"], "Never classified at all");
}

#[test]
fn brief_needs_no_index_and_creates_none() {
    let s = Sandbox::new();
    two_skills(&s);
    let expected = brief(&s, &[]);
    fs::remove_file(s.home.join(".kinana/index-v2.db")).unwrap();
    let before = snapshot(&s.root);
    assert_eq!(brief(&s, &[]), expected);
    assert_eq!(
        brief(&s, &["--stack", "react"])["skills"][0]["id"],
        "form-kit"
    );
    assert_eq!(snapshot(&s.root), before);
    assert!(!s.home.join(".kinana/index-v2.db").exists());
}

#[test]
fn brief_text_is_one_line_per_skill_and_author_text_cannot_break_it() {
    let s = Sandbox::new();
    two_skills(&s);
    let (text, _) = output(&s, &["list", "--brief"], 0);
    assert_eq!(
        text,
        "Skills: 2 of 2 (page 1 of 1)\n\
         api-review\tbackend-api\tstack=node\tstage=design,review\trole=Reviews REST API designs\tuse=Before writing new endpoints\tnot=For visual design work\tdesc=Review REST API contracts\n\
         form-kit\tweb-frontend\tstack=react,next\tstage=build\trole=Builds forms\tuse=When adding a form\tnot=For native mobile screens\tdesc=Build accessible forms\n\
         Bundles: 1 of 1\n\
         web-kit\tWeb kit\tform-kit=primary, api-review=secondary\n"
    );

    // Newlines, tabs, escape sequences and Unicode line or direction controls
    // inside author-written fields stay inside their own column.
    let hostile = "first\nsecond\r\nthird\tcolumn\u{1b}[2Jcleared\u{2028}line\u{2029}para\u{0085}next\u{202e}reversed\u{2066}isolated";
    seed(
        &s,
        "hostile",
        "Plain description",
        classification("other", &[], &[], hostile, hostile, hostile),
    );
    bundle(&s, "odd", "Title\twith\u{1b}[31m controls", &[]);
    let (text, _) = output(&s, &["list", "--brief"], 0);
    let lines = text.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 7, "{text:?}");
    assert_eq!(text.matches('\n').count(), 7);
    assert_eq!(lines[0], "Skills: 3 of 3 (page 1 of 1)");
    assert_eq!(lines[4], "Bundles: 2 of 2");
    for line in &lines[1..4] {
        assert_eq!(line.split('\t').count(), 8, "{line:?}");
    }
    let clean = "first second  third columncleared line para next reversed isolated";
    assert_eq!(
        lines[3],
        format!(
            "hostile\tother\tstack=\tstage=\trole={clean}\tuse={clean}\tnot={clean}\tdesc=Plain description"
        )
    );
    assert_eq!(lines[5], "odd\tTitle with controls\t");
    assert!(
        !text.chars().any(|c| (c.is_control() && c != '\n' && c != '\t')
            || matches!(c, '\u{2028}' | '\u{2029}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')),
        "{text:?}"
    );
    // JSON carries the same text without its control characters (owner
    // decision 2026-11-17); Unicode line and direction marks are not control
    // characters and stay there, only the text output replaces them.
    assert_eq!(
        brief(&s, &["--category", "other"])["skills"][0]["role"],
        "first second  third columncleared\u{2028}line\u{2029}para next\u{202e}reversed\u{2066}isolated"
    );
    assert_eq!(brief(&s, &[])["bundles"][0]["title"], "Title with controls");
}

/// The IDs of a JSON array of skills, in its order.
fn order(skills: &Value) -> Vec<String> {
    skills
        .as_array()
        .unwrap()
        .iter()
        .map(|skill| skill["id"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn brief_returns_one_page_by_default_and_never_the_whole_of_a_large_catalog() {
    let s = Sandbox::new();
    s.initialize();
    for n in 0..45usize {
        let stack: &[&str] = if n % 9 == 0 { &["react"] } else { &[] };
        seed(
            &s,
            &format!("skill-{n:02}"),
            "Described",
            classification("other", stack, &["build"], "Role", "When", "Not"),
        );
    }
    let (listed, _) = output(&s, &["list", "--json"], 0);
    let listed: Value = serde_json::from_str(&listed).unwrap();
    let listed = order(&listed);
    assert_eq!(listed.len(), 45);
    let before = snapshot(&s.root);

    let first = brief(&s, &[]);
    assert_eq!(order(&first["skills"]), listed[..20]);
    assert_eq!(
        json!([
            first["total"],
            first["limit"],
            first["page"],
            first["pages"]
        ]),
        json!([45, 20, 1, 3])
    );
    assert_eq!(
        first["hint"],
        "Showing 20 of 45 skills (page 1 of 3). Narrow the catalog with --stack, --stage or --category, or continue with --page 2."
    );
    // The pages together are the whole list, each skill once and in order.
    let mut paged = order(&first["skills"]);
    for page in ["2", "3"] {
        paged.extend(order(&brief(&s, &["--page", page])["skills"]));
    }
    assert_eq!(paged, listed);
    let last = brief(&s, &["--page", "3"]);
    assert_eq!(last["skills"].as_array().unwrap().len(), 5);
    assert_eq!(
        last["hint"],
        "Showing 5 of 45 skills (page 3 of 3). Narrow the catalog with --stack, --stage or --category."
    );
    let beyond = brief(&s, &["--page", "4"]);
    assert_eq!(beyond["skills"], json!([]));
    assert_eq!(
        beyond["hint"],
        "Page 4 does not exist; this catalog has 3 page(s)."
    );

    // A filter that fits one page needs no hint; the total is what it matched.
    let react = brief(&s, &["--stack", "react"]);
    assert_eq!(
        json!([react["total"], react["pages"], react["hint"]]),
        json!([5, 1, ""])
    );
    // A larger page on request, up to the limit of 50; never everything at once.
    let wide = brief(&s, &["--limit", "50"]);
    assert_eq!(wide["skills"].as_array().unwrap().len(), 45);
    assert_eq!(json!([wide["pages"], wide["hint"]]), json!([1, ""]));
    assert_eq!(
        brief(&s, &["--limit", "7", "--page", "7"])["skills"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    for invalid in [
        &["--limit", "51"][..],
        &["--limit", "0"],
        &["--page", "0"],
        &["--limit", "all"],
    ] {
        assert_eq!(error_code(&s, invalid), "INVALID_ARGUMENTS", "{invalid:?}");
    }
    // Paging belongs to the brief catalog only.
    let (stdout, _) = output(&s, &["list", "--limit", "5", "--json"], 1);
    assert!(stdout.contains("INVALID_ARGUMENTS"), "{stdout}");

    let (text, _) = output(&s, &["list", "--brief"], 0);
    let lines = text.lines().collect::<Vec<_>>();
    assert_eq!(lines[0], "Skills: 20 of 45 (page 1 of 3)");
    assert_eq!(lines.len(), 1 + 20 + 1 + 1);
    assert!(
        lines[22].starts_with("Hint: Showing 20 of 45 skills"),
        "{}",
        lines[22]
    );
    assert_eq!(snapshot(&s.root), before);
}

/// Deterministic filler of an exact character count from ordinary words.
fn sentence(seed: usize, characters: usize) -> String {
    const WORDS: &[&str] = &[
        "review",
        "component",
        "before",
        "implementing",
        "accessibility",
        "contracts",
        "when",
        "the",
        "project",
        "needs",
        "keyboard",
        "navigation",
        "database",
        "migrations",
        "and",
        "release",
        "checklist",
        "for",
        "screens",
        "deployment",
        "pipeline",
        "tests",
        "coverage",
        "design",
        "tokens",
        "payment",
        "flow",
        "error",
        "states",
        "documentation",
    ];
    let mut text = String::new();
    let mut n = seed;
    while text.len() < characters {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(WORDS[n % WORDS.len()]);
        n = n.wrapping_mul(31).wrapping_add(7);
    }
    text.truncate(characters);
    text
}

/// Size evidence for SPEC §4 (8.4): a vault of 200 agent-classified skills
/// with field lengths in the range of the classifications kept under
/// docs/evidence (role 30–250, when-to-use 150–420, when-not 90–470
/// characters, descriptions longer than the 200 character cut).
/// Run with `--nocapture` to print the numbers.
#[test]
fn brief_size_for_a_vault_of_200_classified_skills() {
    const CATEGORIES: &[&str] = &[
        "mobile",
        "web-frontend",
        "backend-api",
        "data-db",
        "devops",
        "testing-qa",
        "planning-specs",
        "design-ui",
        "docs-writing",
        "content-media",
    ];
    const STACKS: &[&str] = &[
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
    const STAGES: &[&str] = &["plan", "design", "build", "test", "review", "ship"];
    let s = Sandbox::new();
    s.initialize();
    let mut ids = vec![];
    for n in 0..200usize {
        let id = format!(
            "{}-{}-{n:03}",
            sentence(n, 8).replace(' ', "-"),
            CATEGORIES[n % 10]
        );
        // Half of the skills are tied to one technology, a tenth to two.
        let stack: Vec<&str> = match n % 10 {
            0 => vec![STACKS[n / 10 % 10], STACKS[(n / 10 + 3) % 10]],
            1..=4 => vec![STACKS[(n / 10 + n) % 10]],
            _ => vec![],
        };
        let stage: Vec<&str> = if n % 3 == 0 {
            vec![STAGES[n % 6], STAGES[(n + 2) % 6]]
        } else {
            vec![STAGES[n % 6]]
        };
        seed(
            &s,
            &id,
            &sentence(n + 1, 260 + (n * 37) % 500),
            classification(
                CATEGORIES[n % 10],
                &stack,
                &stage,
                &sentence(n + 2, 30 + (n * 13) % 120),
                &sentence(n + 3, 150 + (n * 17) % 180),
                &sentence(n + 4, 90 + (n * 19) % 200),
            ),
        );
        ids.push(id);
    }
    for b in 0..8usize {
        let members = (0..10usize)
            .map(|m| {
                (
                    ids[b * 20 + m].as_str(),
                    if m < 7 { "primary" } else { "secondary" },
                )
            })
            .collect::<Vec<_>>();
        bundle(
            &s,
            &format!("kit-{b}"),
            &format!("Starter kit number {b}"),
            &members,
        );
    }
    let measure = |filters: &[&str]| -> (usize, usize, usize, usize) {
        let (stdout, _) = output(&s, &[&["list", "--brief", "--json"], filters].concat(), 0);
        let value: Value = serde_json::from_str(&stdout).unwrap();
        let bundles = serde_json::to_string(&value["bundles"]).unwrap().len();
        let bytes = stdout.trim_end().len();
        (
            value["skills"].as_array().unwrap().len(),
            bytes,
            bytes.div_ceil(4),
            bundles,
        )
    };
    // What one call returns now, and what the whole catalog costs when every
    // page is fetched: the size a single unpaged response used to have.
    let page = measure(&[]);
    let largest = measure(&["--limit", "50"]);
    let all = (1..=4).fold((0, 0, 0, 0), |sum, n| {
        let part = measure(&["--limit", "50", "--page", &n.to_string()]);
        (sum.0 + part.0, sum.1 + part.1, sum.2 + part.2, part.3)
    });
    let react = measure(&["--stack", "react"]);
    let build = measure(&["--stage", "build"]);
    let narrow = measure(&["--stack", "react", "--stage", "build"]);
    for (name, (skills, bytes, tokens, bundles)) in [
        ("unfiltered, default page", page),
        ("unfiltered, --limit 50", largest),
        ("all four pages of --limit 50 together", all),
        ("--stack react", react),
        ("--stage build", build),
        ("--stack react --stage build", narrow),
    ] {
        println!(
            "list --brief --json {name}: {skills} skills, {bytes} bytes, ~{tokens} tokens (ceil(bytes/4)); bundles part {bundles} bytes"
        );
    }
    assert_eq!((page.0, largest.0, all.0), (20, 50, 200));
    assert!(react.0 > 0 && react.0 <= 20, "{react:?}");
    // Guards against the shape growing: under 1 KiB a skill on these lengths,
    // so one default call stays under about 6,000 tokens and the largest
    // page a caller can ask for under about 13,000.
    assert!(all.1 < 200 * 1024, "{all:?}");
    assert!(page.2 < 6_000, "{page:?}");
    assert!(largest.2 < 13_000, "{largest:?}");
    assert!(narrow.1 <= react.1);
}

/// Review finding M3: `role`, `when_to_use` and `when_not` come from
/// unreviewed classifier output or hand-edited meta files, and a bundle title
/// from a hand-edited bundle file. However long they are, each reaches the
/// brief catalog cut to at most 200 characters, and a page whose text is
/// still over its budget is cut further, so a page stays under about 6,000
/// tokens by default and 15,000 for the largest, in any script.
#[test]
fn a_very_long_field_cannot_push_a_page_past_its_limit() {
    let s = Sandbox::new();
    s.initialize();
    // About 30,000 characters each, in one-, two- and four-byte text.
    let huge = |n: usize| -> String {
        match n % 3 {
            0 => "Use this skill for everything. ".repeat(1_000),
            1 => "استخدم هذه المهارة لكل شيء دائمً ".repeat(1_000),
            _ => "🧰🔧📦".repeat(10_000),
        }
    };
    let mut ids = vec![];
    for n in 0..60usize {
        let id = format!("long-field-{n:02}");
        seed(
            &s,
            &id,
            &huge(n),
            classification("other", &[], &[], &huge(n), &huge(n + 1), &huge(n)),
        );
        ids.push(id);
    }
    bundle(
        &s,
        "huge-kit",
        &huge(0),
        &[(ids[0].as_str(), "primary"), (ids[1].as_str(), "secondary")],
    );
    for (filters, skills, ceiling) in [(&[][..], 20, 6_000), (&["--limit", "50"][..], 50, 15_000)] {
        let (stdout, _) = output(&s, &[&["list", "--brief", "--json"], filters].concat(), 0);
        let value: Value = serde_json::from_str(&stdout).unwrap();
        let page = value["skills"].as_array().unwrap();
        assert_eq!(page.len(), skills);
        for skill in page {
            for field in ["role", "when_to_use", "when_not", "description"] {
                let text = skill[field].as_str().unwrap();
                assert!(text.chars().count() <= 200, "{field}: {text}");
                assert!(text.ends_with('…'), "{field}: {text}");
            }
        }
        assert!(
            value["hint"].as_str().unwrap().contains("kinana show <id>"),
            "{}",
            value["hint"]
        );
        // The 30,000-character title is cut, or the page shows the bundle
        // by name and member count only.
        match value["bundles"][0]["title"].as_str() {
            Some(title) => assert_eq!(title.chars().count(), 200),
            None => assert_eq!(value["bundles"][0]["members"], 2),
        }
        let tokens = stdout.trim_end().len().div_ceil(4);
        println!(
            "long fields, {filters:?}: {} bytes, ~{tokens} tokens; hint: {}",
            stdout.trim_end().len(),
            value["hint"]
        );
        assert!(tokens < ceiling, "{filters:?}: ~{tokens} tokens");
    }
}

/// Second review, finding 4: `stack`, `stage` and `category` come from the
/// meta file too, which can be edited by hand. Values outside the configured
/// lists are left out and the category is cut, so they cannot defeat the
/// page budget either.
#[test]
fn hand_edited_dimensions_and_category_cannot_defeat_the_page_budget() {
    let s = Sandbox::new();
    s.initialize();
    let huge = "A".repeat(20_000);
    for n in 0..20usize {
        seed(
            &s,
            &format!("edited-{n:02}"),
            "Plain description",
            classification(
                &"c".repeat(30_000),
                &[huge.as_str(), "react"],
                &[huge.as_str(), "build"],
                "Role",
                "When",
                "Not",
            ),
        );
    }
    let (stdout, _) = output(&s, &["list", "--brief", "--json"], 0);
    let value: Value = serde_json::from_slice(stdout.as_bytes()).unwrap();
    for skill in value["skills"].as_array().unwrap() {
        assert_eq!(skill["stack"], json!(["react"]));
        assert_eq!(skill["stage"], json!(["build"]));
        assert!(skill["category"].as_str().unwrap().chars().count() <= 200);
    }
    let tokens = stdout.trim_end().len().div_ceil(4);
    assert!(tokens < 6_000, "~{tokens} tokens");
}

/// Bytes of the page's `skills` and `bundles` together, as the budget counts them.
fn page_bytes(value: &Value) -> usize {
    serde_json::to_vec(&value["skills"]).unwrap().len()
        + serde_json::to_vec(&value["bundles"]).unwrap().len()
}

/// Owner decision 2026-11-17: the bundles of the first page count toward the
/// page budget. Over it, bundles are shown by name and member count only,
/// with a hint naming `kinana bundle show <name>`; when even that does not
/// fit, the page shows the bundles that do and says how many there are.
#[test]
fn many_bundles_with_long_names_and_members_keep_the_page_within_its_budget() {
    let s = Sandbox::new();
    s.initialize();
    let mut ids = vec![];
    for n in 0..60usize {
        // Long IDs in two-byte letters: every member entry is long too.
        let id = format!("{}-{n:02}", "مهارة-طويلة-الاسم".repeat(6));
        seed(
            &s,
            &id,
            &"Describe the skill at length. ".repeat(20),
            classification(
                "other",
                &[],
                &[],
                &"Role text. ".repeat(30),
                &"When to use it. ".repeat(20),
                &"When not. ".repeat(30),
            ),
        );
        ids.push(id);
    }
    let members = ids
        .iter()
        .enumerate()
        .map(|(n, id)| {
            (
                id.as_str(),
                if n % 2 == 0 { "primary" } else { "secondary" },
            )
        })
        .collect::<Vec<_>>();
    let bundle_names = (0..300usize)
        .map(|n| format!("{}-{n:03}", "حزمة-باسم-طويل".repeat(7)))
        .collect::<Vec<_>>();
    for name in &bundle_names {
        bundle(&s, name, &"A very long bundle title. ".repeat(40), &members);
    }

    for (filters, limit, ceiling) in [(&[][..], 20, 6_000), (&["--limit", "50"][..], 50, 15_000)] {
        let (stdout, _) = output(&s, &[&["list", "--brief", "--json"], filters].concat(), 0);
        let value: Value = serde_json::from_str(&stdout).unwrap();
        let bytes = page_bytes(&value);
        assert!(
            bytes <= limit * 1_100,
            "{filters:?}: {bytes} bytes over {}",
            limit * 1_100
        );
        let tokens = stdout.trim_end().len().div_ceil(4);
        println!(
            "300 long bundles, {filters:?}: page {bytes} bytes, reply ~{tokens} tokens; {} of {} bundles; hint: {}",
            value["bundles"].as_array().unwrap().len(),
            value["bundles_total"],
            value["hint"]
        );
        assert!(tokens < ceiling, "{filters:?}: ~{tokens} tokens");
        assert_eq!(value["skills"].as_array().unwrap().len(), limit);
        assert_eq!(value["bundles_total"], 300);
        let shown = value["bundles"].as_array().unwrap();
        assert!(shown.len() < 300);
        for (bundle, name) in shown.iter().zip(&bundle_names) {
            // Name and member count only, in name order, never cut.
            assert_eq!(bundle["name"], name.as_str());
            assert_eq!(bundle["members"], 60);
            assert!(bundle.get("title").is_none(), "{bundle}");
            assert!(bundle.get("skills").is_none(), "{bundle}");
        }
        let hint = value["hint"].as_str().unwrap();
        assert!(hint.contains("kinana bundle show <name>"), "{hint}");
        assert!(
            hint.contains(&format!("{} of 300 bundles are shown", shown.len())),
            "{hint}"
        );
        assert!(hint.contains("kinana bundle list"), "{hint}");
    }

    // Later pages carry no bundles but still say how many there are.
    let value = brief(&s, &["--page", "2"]);
    assert_eq!(value["bundles"], json!([]));
    assert_eq!(value["bundles_total"], 300);
    assert!(!value["hint"].as_str().unwrap().contains("bundle"));
}

/// A few bundles that fit leave the page as before: full bundles, no hint
/// about them; when they alone push it over, they shrink to name and count
/// before any skill text is cut.
#[test]
fn bundles_shrink_to_name_and_count_before_skill_text_is_cut() {
    let s = Sandbox::new();
    s.initialize();
    let mut ids = vec![];
    for n in 0..20usize {
        let id = format!("skill-{n:02}");
        seed(
            &s,
            &id,
            "Short description",
            classification("other", &[], &[], "Role", "When", "Not"),
        );
        ids.push(id);
    }
    let members = ids
        .iter()
        .map(|id| (id.as_str(), "primary"))
        .collect::<Vec<_>>();
    bundle(&s, "small-kit", "Small kit", &members[..2]);
    let value = brief(&s, &[]);
    assert_eq!(value["bundles"][0]["title"], "Small kit");
    assert_eq!(value["bundles"][0]["members"], 2);
    assert_eq!(value["bundles"][0]["skills"].as_array().unwrap().len(), 2);
    assert_eq!(value["hint"], "");

    for n in 0..40usize {
        bundle(&s, &format!("kit-{n:02}"), &"Title ".repeat(30), &members);
    }
    let value = brief(&s, &[]);
    assert!(page_bytes(&value) <= 20 * 1_100);
    assert_eq!(value["bundles"].as_array().unwrap().len(), 41);
    assert!(value["bundles"][0].get("skills").is_none());
    let hint = value["hint"].as_str().unwrap();
    assert!(hint.contains("kinana bundle show <name>"), "{hint}");
    assert!(!hint.contains("was cut"), "{hint}");
    assert_eq!(value["skills"][0]["role"], "Role");
}

/// Owner decision 2026-11-17: control characters, terminal escape sequences
/// included, are removed from every text field of the brief catalog.
#[test]
fn control_characters_and_terminal_escapes_are_removed_from_every_field() {
    let s = Sandbox::new();
    s.initialize();
    let dirty = "a\u{1b}[31mRED\u{1b}[0m b\u{1b}]0;title\u{7} c\u{1b}]8;;http://x\u{1b}\\link d\u{7}\u{0}\u{8}\u{7f}\u{9b}2Je\nf\tg\rh";
    seed(
        &s,
        "dirty",
        dirty,
        classification(dirty, &[], &[], dirty, dirty, dirty),
    );
    bundle(&s, "dirty-kit", dirty, &[("dirty", "primary")]);
    let (stdout, _) = output(&s, &["list", "--brief", "--json"], 0);
    let value: Value = serde_json::from_str(&stdout).unwrap();
    let mut texts = vec![];
    fn strings(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::String(text) => out.push(text.clone()),
            Value::Array(items) => items.iter().for_each(|item| strings(item, out)),
            Value::Object(map) => map.values().for_each(|item| strings(item, out)),
            _ => {}
        }
    }
    strings(&value, &mut texts);
    for text in &texts {
        assert!(
            !text.chars().any(char::is_control),
            "control character left in {text:?}"
        );
    }
    let skill = &value["skills"][0];
    for field in ["role", "when_to_use", "when_not", "description", "category"] {
        assert_eq!(
            skill[field], "aRED b clink d2Je f g h",
            "{field}: {}",
            skill[field]
        );
    }
    assert_eq!(value["bundles"][0]["title"], "aRED b clink d2Je f g h");
    // Escaped in the raw reply too: nothing for a terminal to act on.
    assert!(!stdout.contains("\\u001b"), "{stdout}");
}

/// Third review, finding 4: a hand-edited meta file that repeats a stack or
/// stage value thousands of times cannot grow the page; each configured
/// value appears once, in the order of `box.toml`.
#[test]
fn repeated_dimension_values_appear_once_and_keep_the_page_within_budget() {
    let s = Sandbox::new();
    s.initialize();
    let many = vec!["react"; 2_000];
    let stages = vec!["build"; 2_000];
    for n in 0..6usize {
        let mut stack = many.clone();
        stack.push("node");
        seed(
            &s,
            &format!("repeated-{n}"),
            "Plain description",
            classification("other", &stack, &stages, "Role", "When", "Not"),
        );
    }
    let value = brief(&s, &["--limit", "6"]);
    for skill in value["skills"].as_array().unwrap() {
        assert_eq!(skill["stage"], json!(["build"]));
        let stack = skill["stack"].as_array().unwrap();
        assert_eq!(stack.len(), 2, "{stack:?}");
    }
    assert!(
        page_bytes(&value) <= 6 * 1_100,
        "{} bytes",
        page_bytes(&value)
    );
}

/// A hand-edited `box.toml` that repeats a list entry still shows each value
/// once (third independent review, finding 5).
#[test]
fn a_list_entry_repeated_in_box_toml_appears_once() {
    let s = Sandbox::new();
    s.initialize();
    let config_path = s.vault.join("box.toml");
    let mut config: toml::Value =
        toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    let table = config.as_table_mut().unwrap();
    table.insert(
        "stacks".into(),
        toml::Value::Array(
            vec!["react".into(); 500]
                .into_iter()
                .chain(["node".into()])
                .collect(),
        ),
    );
    table.insert(
        "stages".into(),
        toml::Value::Array(vec!["build".into(); 500]),
    );
    fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();
    seed(
        &s,
        "repeated-config",
        "Plain description",
        classification(
            "other",
            &["react", "node"],
            &["build"],
            "Role",
            "When",
            "Not",
        ),
    );
    let value = brief(&s, &[]);
    let skill = &value["skills"][0];
    assert_eq!(skill["stack"], json!(["react", "node"]));
    assert_eq!(skill["stage"], json!(["build"]));
}
