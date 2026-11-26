#[path = "support/json_contract.rs"]
mod contract;
#[allow(dead_code)]
#[path = "support/router.rs"]
mod previous;
#[allow(dead_code)]
mod support;

use clap::CommandFactory;
use kinana_core::{ClassificationProposal, edit, vault::Vault};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};
use support::{Sandbox, copy_tree};

fn checked(s: &Sandbox, command: &str, input: &[&str], exit: i32) -> Value {
    let mut args = vec!["--json"];
    args.extend_from_slice(input);
    let output = s.run(&args);
    let context = format!(
        "{args:?}: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(exit), "{context}");
    let value: Value =
        serde_json::from_slice(&output.stdout).unwrap_or_else(|e| panic!("{e}: {context}"));
    let doc = contract::document();
    let shape = if command == "error" {
        &doc["x-error"]
    } else {
        &doc["x-commands"][command]
    };
    assert!(!shape.is_null(), "undocumented command {command}");
    contract::validate(&doc, shape, &value).unwrap_or_else(|e| panic!("{e}: {context}"));
    value
}

fn setup(s: &Sandbox) -> PathBuf {
    checked(s, "init", &["init"], 0);
    add(s, "api")
}

fn add(s: &Sandbox, id: &str) -> PathBuf {
    let source = s.root.join(format!("source-{id}"));
    fs::create_dir_all(source.join("assets/empty")).unwrap();
    fs::write(source.join("SKILL.md"), format!("---\nname: {id}\ndescription: Review REST API contracts\n---\n# Review\nCompare API contracts before implementing endpoints.\n")).unwrap();
    checked(s, "add", &["add", source.to_str().unwrap()], 0);
    source
}

fn vault(s: &Sandbox) -> Vault {
    Vault::new(s.vault.clone(), s.home.join(".kinana/index-v2.db"))
}

fn proposal(s: &Sandbox) {
    let v = vault(s);
    let mut m = edit::read_metadata(&v, "api").unwrap();
    m.source.url = Some("https://example.invalid/fixture".into());
    m.source.subpath = Some("skills/api".into());
    m.source.commit = Some("opaque-fixture-revision".into());
    m.classification.category = "backend-api".into();
    m.classification.classified_by = "claude".into();
    m.classification.classified_from = Some("full".into());
    m.classification.overlaps_with = vec!["alternate".into()];
    m.classification.tags = vec!["tagged".into()];
    m.classification.when_to_use = "Before implementing endpoints".into();
    m.classification.when_not = "For visual design".into();
    m.proposal = Some(ClassificationProposal {
        category: "backend-api".into(),
        stack: vec!["node".into()],
        stage: vec!["design".into(), "review".into()],
        tags: vec!["api".into()],
        role: "Review API contracts".into(),
        when_to_use: "Before implementing endpoints".into(),
        when_not: "For visual design".into(),
        improved_description: "Compare API contracts before implementing endpoints.".into(),
        overlaps_with: vec!["alternate".into()],
        confidence: 0.9,
        category_reason: String::new(),
    });
    edit::apply(
        &v,
        &BTreeMap::from([("api".into(), m)]),
        "Fixture: pending proposal",
        false,
    )
    .unwrap();
}

fn bundle_file(s: &Sandbox, title: &str) -> PathBuf {
    let path = s.root.join("bundle-input.toml");
    fs::write(&path, format!("name = \"api-kit\"\ntitle = \"{title}\"\nagents = [\"claude\", \"codex\"]\n[[skills]]\nid = \"api\"\nrole = \"primary\"\ngroup = \"contracts\"\n[[skills]]\nid = \"alternate\"\nrole = \"secondary\"\ngroup = \"contracts\"\n")).unwrap();
    path
}

fn bundle_setup(s: &Sandbox) -> PathBuf {
    setup(s);
    add(s, "alternate");
    bundle_file(s, "API kit")
}

fn project(s: &Sandbox) -> PathBuf {
    let p = s.root.join("project");
    fs::create_dir(&p).unwrap();
    p
}

fn activate(s: &Sandbox, p: &std::path::Path) {
    checked(
        s,
        "use",
        &[
            "use",
            "api",
            "--project",
            p.to_str().unwrap(),
            "--agent",
            "all",
        ],
        0,
    );
}

macro_rules! command_contracts {
    ($($name:ident: $command:literal => |$s:ident, $run:ident| $body:block),+ $(,)?) => {
        const COVERED_COMMANDS: &[&str] = &[$($command),+];
        $(#[test] fn $name() {
            let $s = Sandbox::new();
            let $run = |args: &[&str], exit| checked(&$s, $command, args, exit);
            $body
        })+
    };
}

command_contracts! {
    migrate_json: "migrate" => |s, run| {
        run(&["migrate","--dry-run"],0);
        checked(&s,"init", &["init"],0);
        fs::rename(s.home.join(".kinana"),s.home.join(".skillsbox")).unwrap();
        let plan=run(&["migrate","--dry-run"],0);
        run(&["migrate"],2);
        run(&["migrate","--confirm",plan["preview_token"].as_str().unwrap()],0);
    },
    migrate_rollback_json: "migrate --rollback" => |s, run| {
        checked(&s,"init", &["init"],0);
        fs::rename(s.home.join(".kinana"),s.home.join(".skillsbox")).unwrap();
        let plan=checked(&s,"migrate", &["migrate","--dry-run"],0);
        checked(&s,"migrate", &["migrate","--confirm",plan["preview_token"].as_str().unwrap()],0);
        let plan=run(&["migrate","--rollback","--dry-run"],0);
        run(&["migrate","--rollback","--confirm",plan["preview_token"].as_str().unwrap()],0);
    },

    init_router_json: "init --install-router" => |s, run| {
        let preview=run(&["init","--install-router","--dry-run"],0);
        assert_eq!(preview["router"]["targets"].as_array().unwrap().len(),2);
        assert_eq!(preview["router"]["targets"][0]["action"],"install");
        run(&["init","--install-router"],0);
        assert_eq!(run(&["init","--install-router"],0)["router"]["targets"][1]["action"],"unchanged");
        // An owned, unmodified earlier version is reported as an upgrade with the same shape.
        #[cfg(unix)]
        {
            let s = Sandbox::new();
            previous::install_previous_router(&s.home, ".claude");
            let preview = checked(&s, "init --install-router", &["init","--install-router","--dry-run"], 0);
            assert_eq!(preview["router"]["targets"][0]["action"],"upgrade");
            assert_eq!(preview["router"]["targets"][1]["action"],"install");
            let done = checked(&s, "init --install-router", &["init","--install-router"], 0);
            assert_eq!(done["router"]["targets"][0]["action"],"upgrade");
            assert!(done["router"]["warnings"].as_array().unwrap().iter().all(Value::is_string));
        }
    },
    init_json: "init" => |s, run| {
        run(&["init", "--dry-run"], 0);
        run(&["init"], 0);
        run(&["init"], 0);
    },
    scan_json: "scan" => |s, run| {
        let source = setup(&s);
        let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/scan/projects");
        let report = run(&["scan", source.to_str().unwrap(), fixtures.to_str().unwrap()], 0);
        assert!(!report["entries"].as_array().unwrap().is_empty());
    },
    scan_import_json: "scan --import" => |s, run| {
        let source = setup(&s);
        run(&["scan", source.to_str().unwrap(), "--import", "--dry-run"], 0);
        run(&["scan", source.to_str().unwrap(), "--import"], 0);
    },
    add_json: "add" => |s, run| {
        let source = setup(&s);
        run(&["add", source.to_str().unwrap(), "--dry-run"], 0);
        run(&["add", source.to_str().unwrap()], 0);
        run(&["add", "https://github.com/example/skills/tree/main/api", "--dry-run"], 0);
    },
    list_json: "list" => |s, run| {
        checked(&s, "init", &["init"], 0);
        assert_eq!(run(&["list"], 0), json!([]));
        add(&s, "api");
        let report = run(&["list"], 0);
        assert_eq!(report.as_array().unwrap().len(), 1);
        run(&["list", "--category", "other", "--agent", "claude"], 0);
        // Dimensions are arrays in every skill, empty or not, and filter the list.
        assert_eq!(report[0]["stack"], json!([]));
        assert_eq!(report[0]["stage"], json!([]));
        checked(&s, "dimensions", &["dimensions", "api", "--stack", "node", "--stage", "review"], 0);
        let filtered = run(&["list", "--stack", "node", "--stage", "review", "--category", "other"], 0);
        assert_eq!(filtered[0]["stack"], json!(["node"]));
        assert_eq!(filtered[0]["stage"], json!(["review"]));
        assert_eq!(run(&["list", "--stack", "rust"], 0), json!([]));
        assert_eq!(checked(&s, "error", &["list", "--stack", "nodejs"], 1)["error"]["code"], "INVALID_STACK");
        assert_eq!(checked(&s, "error", &["list", "--stage", "deploy"], 1)["error"]["code"], "INVALID_STAGE");
    },
    list_brief_json: "list --brief" => |s, run| {
        assert!(include_str!("../../../docs/SPEC.md").contains("بثمانية مفاتيح ثابتة"));
        checked(&s, "init", &["init"], 0);
        assert_eq!(run(&["list", "--brief"], 0), json!({"skills": [], "bundles": [], "bundles_total": 0, "total": 0, "limit": 20, "page": 1, "pages": 1, "hint": ""}));
        let file = bundle_setup(&s);
        checked(&s, "bundle create", &["bundle", "create", "--file", file.to_str().unwrap()], 0);
        proposal(&s);
        checked(&s, "dimensions", &["dimensions", "api", "--stack", "node", "--stage", "review"], 0);
        let report = run(&["list", "--brief"], 0);
        assert_eq!(report["skills"].as_array().unwrap().len(), 2);
        assert_eq!(report["skills"][1], json!({
            "id": "api", "role": "", "when_to_use": "Before implementing endpoints",
            "when_not": "For visual design", "description": "Review REST API contracts",
            "category": "backend-api", "stack": ["node"], "stage": ["review"], "audit_level": "clean",
        }));
        assert_eq!(report["bundles"], json!([{"name": "api-kit", "members": 2, "title": "API kit", "risky_members": 0, "skills": [
            {"id": "api", "role": "primary", "audit_level": "clean"}, {"id": "alternate", "role": "secondary", "audit_level": "clean"},
        ]}]));
        // Filters keep the shape; bundles are not filtered.
        let filtered = run(&["list", "--brief", "--stack", "node", "--stage", "review", "--category", "backend-api", "--agent", "codex"], 0);
        assert_eq!(filtered["skills"].as_array().unwrap().len(), 1);
        assert_eq!(filtered["bundles"], report["bundles"]);
        let empty = run(&["list", "--brief", "--stack", "rust"], 0);
        assert_eq!(empty["skills"], json!([]));
        assert_eq!(empty["bundles"].as_array().unwrap().len(), 1);
        run(&["list", "--brief", "--query", "contracts", "--dry-run"], 0);
        let paged = run(&["list", "--brief", "--limit", "1", "--page", "2"], 0);
        assert_eq!(paged["skills"].as_array().unwrap().len(), 1);
        assert_eq!(json!([paged["total"], paged["limit"], paged["page"], paged["pages"]]), json!([2, 1, 2, 2]));
        assert!(paged["hint"].as_str().unwrap().starts_with("Showing 1 of 2 skills"));
        assert_eq!(checked(&s, "error", &["list", "--brief", "--stack", "nodejs"], 1)["error"]["code"], "INVALID_STACK");
        assert_eq!(checked(&s, "error", &["list", "--brief", "--stage", "deploy"], 1)["error"]["code"], "INVALID_STAGE");
    },
    dimensions_json: "dimensions" => |s, run| {
        setup(&s); proposal(&s);
        let preview = run(&["dimensions", "api", "--stack", "node", "--stack", "react", "--stage", "review", "--dry-run"], 0);
        assert_eq!(preview, json!({"id":"api","stack":["react","node"],"stage":["review"],"dry_run":true,"changed":true,"committed":false}));
        let report = run(&["dimensions", "api", "--stack", "node", "--stack", "react", "--stage", "review"], 0);
        assert_eq!(report, json!({"id":"api","stack":["react","node"],"stage":["review"],"dry_run":false,"changed":true,"committed":true}));
        assert_eq!(run(&["dimensions", "api", "--stage", "review"], 0)["committed"], false);
        // Empty dimensions keep the documented array types.
        let cleared = run(&["dimensions", "api", "--clear-stack", "--clear-stage"], 0);
        assert_eq!(cleared["stack"], json!([]));
        assert_eq!(cleared["stage"], json!([]));
        // The pending description proposal and its approval state are untouched.
        let detail = checked(&s, "show", &["show", "api"], 0);
        assert!(detail["metadata"]["proposal"].is_object());
        assert_eq!(detail["metadata"]["description"]["active"], "original");
        assert_eq!(detail["metadata"]["classification"]["classified_by"], "claude");
        assert_eq!(checked(&s, "error", &["dimensions", "api", "--stack", "nodejs"], 1)["error"]["code"], "INVALID_STACK");
        assert_eq!(checked(&s, "error", &["dimensions", "api", "--stage", "deploy"], 1)["error"]["code"], "INVALID_STAGE");
        assert_eq!(checked(&s, "error", &["dimensions", "api"], 1)["error"]["code"], "INVALID_ARGUMENTS");
    },
    category_json: "category" => |s, run| {
        setup(&s); proposal(&s);
        assert_eq!(run(&["category", "api", "security", "--dry-run"], 0)["committed"], false);
        assert_eq!(run(&["category", "api", "security"], 0)["committed"], true);
        assert_eq!(run(&["category", "api", "security"], 0)["committed"], false);
        let detail=checked(&s,"show", &["show","api"],0);
        assert!(detail["metadata"]["proposal"].is_object());
        assert_eq!(detail["metadata"]["description"]["active"],"original");
    },
    pin_json: "pin" => |s, run| {
        setup(&s);
        assert_eq!(run(&["pin", "api", "--dry-run"], 0)["committed"], false);
        assert_eq!(run(&["pin", "api"], 0)["pinned"], true);
        assert_eq!(run(&["pin", "api"], 0)["changed"], false);
        run(&["pin", "api", "--unset", "--dry-run"], 0);
        run(&["pin", "api", "--unset"], 0);
    },
    search_json: "search" => |s, run| {
        setup(&s);
        assert_eq!(run(&["search", "missing"], 0)["items"], json!([]));
        let found = run(&["search", "contracts"], 0);
        assert_eq!(found["items"].as_array().unwrap().len(), 1);
        assert_eq!(found["items"][0]["stack"], json!([]));
        assert!(found["items"][0]["when_to_use"].is_string());
        // A tag and a "when to use" word find the skill; a "when not" word does not.
        proposal(&s);
        assert_eq!(run(&["search", "tagged"], 0)["items"].as_array().unwrap().len(), 1);
        assert_eq!(run(&["search", "implementing"], 0)["items"].as_array().unwrap().len(), 1);
        assert_eq!(run(&["search", "visual"], 0)["items"], json!([]));
        run(&["search", "API", "--dry-run"], 0);
    },
    suggest_json: "suggest" => |s, run| {
        let file = bundle_setup(&s);
        checked(&s,"bundle create", &["bundle","create","--file",file.to_str().unwrap()],0);
        proposal(&s);
        let p=project(&s);
        let empty=run(&["suggest","--project",p.to_str().unwrap()],0);
        assert_eq!(empty["bundles"].as_array().unwrap().len(),1);
        fs::write(p.join("Cargo.toml"),"[workspace]\nmembers=[]\n").unwrap();
        fs::write(p.join("package.json"),"[").unwrap();
        let report=run(&["suggest","--project",p.to_str().unwrap(),"--dry-run"],0);
        assert!(!report["signals"].as_array().unwrap().is_empty());
        assert!(!report["skills"].as_array().unwrap().is_empty());
        assert!(!report["bundles"].as_array().unwrap().is_empty());
        assert!(!report["warnings"].as_array().unwrap().is_empty());
        // Additive 8.3 fields: detected stacks, their signals, and one reason per skill.
        assert_eq!(report["stacks"], json!(["rust"]));
        assert_eq!(report["stack_signals"].as_array().unwrap().len(), 1);
        assert_eq!(report["matches"].as_array().unwrap().len(), report["skills"].as_array().unwrap().len());
        checked(&s, "dimensions", &["dimensions", "alternate", "--stack", "rust"], 0);
        let ranked = run(&["suggest", "--project", p.to_str().unwrap()], 0);
        assert_eq!(ranked["skills"][0]["id"], "alternate");
        assert_eq!(ranked["matches"][0], json!({"id":"alternate","matched_stacks":["rust"],"matched_category":false}));
        assert_eq!(empty["stacks"], json!([]));
        assert_eq!(empty["matches"], json!([]));
    },
    show_json: "show" => |s, run| {
        let source = setup(&s);
        let initial = run(&["show", "api"], 0);
        assert!(initial["metadata"].get("proposal").is_none());
        add(&s, "alternate");
        proposal(&s);
        let pending = run(&["show", "api"], 0);
        assert!(pending["metadata"]["proposal"].is_object());
        checked(&s, "review", &["review", "api", "--accept"], 0);
        let approved = run(&["show", "api"], 0);
        assert!(approved["metadata"]["description"]["approved_at"].is_string());
        // The skill always carries both dimensions; the metadata mirror omits empty ones.
        assert_eq!(approved["skill"]["stack"], json!([]));
        assert!(approved["metadata"]["classification"].get("stack").is_none());
        checked(&s, "dimensions", &["dimensions", "api", "--stack", "node", "--stage", "design"], 0);
        let dimensioned = run(&["show", "api"], 0);
        assert_eq!(dimensioned["skill"]["stack"], json!(["node"]));
        assert_eq!(dimensioned["metadata"]["classification"]["stage"], json!(["design"]));
        let variant = s.root.join("variant");
        copy_tree(&source, &variant);
        let file = variant.join("SKILL.md");
        fs::write(&file, format!("{}\nVariant body\n", fs::read_to_string(&file).unwrap())).unwrap();
        checked(&s, "scan --import", &["scan", variant.to_str().unwrap(), "--import"], 0);
        assert!(!run(&["show", "api"], 0)["variants"].as_array().unwrap().is_empty());
        // Contract 11 (owner, 2026-11-24): a secrets finding carries the
        // names and forms on its lines, never a value.
        let env = s.root.join("source-env");
        fs::create_dir_all(env.join("scripts")).unwrap();
        fs::write(env.join("SKILL.md"), "---\nname: env\ndescription: Print the token\n---\n# Env\n").unwrap();
        fs::write(env.join("scripts/run.sh"), "echo \"$GITHUB_TOKEN\"\n").unwrap();
        checked(&s, "add", &["add", env.to_str().unwrap(), "--allow-risky"], 0);
        let shown = run(&["show", "env"], 0);
        let secrets = shown["metadata"]["audit"]["details"].as_array().unwrap().iter()
            .filter(|d| d["rule"] == "secrets" && d["path"] == "scripts/run.sh")
            .collect::<Vec<_>>();
        assert!(!secrets.is_empty(), "{shown}");
        assert!(secrets.iter().all(|d| d["forms"] == json!(["$GITHUB_TOKEN"])), "{shown}");
        // Absent, not empty, on a finding of another rule.
        assert!(shown["metadata"]["audit"]["details"].as_array().unwrap().iter()
            .filter(|d| d["rule"] != "secrets")
            .all(|d| d.get("forms").is_none()), "{shown}");
    },
    classify_json: "classify" => |s, run| {
        setup(&s);
        let preview = run(&["classify", "api", "--agent", "claude", "--dry-run"], 0);
        assert!(preview["estimated_tokens"]["calibration"].is_object());
        let keyword = run(&["classify", "api", "--agent", "keywords"], 0);
        assert!(keyword["estimated_tokens"]["calibration"].is_null());
        run(&["classify", "--all", "--agent", "keywords", "--yes", "--dry-run"], 0);
    },
    review_json: "review" => |s, run| {
        setup(&s);
        run(&["review", "api"], 0);
        for decision in ["--accept", "--reject", "--original"] {
            proposal(&s);
            run(&["review", "api", decision, "--dry-run"], 0);
            run(&["review", "api", decision], 0);
        }
    },
    build_json: "build" => |s, run| {
        setup(&s);
        run(&["build", "api", "--dry-run"], 0);
        let report = run(&["build"], 0);
        assert_eq!(report["items"].as_array().unwrap().len(), 1);
    },
    groups_json: "groups" => |s, run| {
        setup(&s);
        assert_eq!(run(&["groups"], 0)["candidates"], json!([]));
        add(&s, "alternate");
        proposal(&s);
        assert!(!run(&["groups"], 0)["candidates"].as_array().unwrap().is_empty());
    },
    bundle_create_json: "bundle create" => |s, run| {
        let file = bundle_setup(&s);
        run(&["bundle", "create", "--file", file.to_str().unwrap(), "--dry-run"], 0);
        run(&["bundle", "create", "--file", file.to_str().unwrap()], 0);
    },
    bundle_edit_json: "bundle edit" => |s, run| {
        let file = bundle_setup(&s);
        checked(&s, "bundle create", &["bundle", "create", "--file", file.to_str().unwrap()], 0);
        bundle_file(&s, "Updated API kit");
        run(&["bundle", "edit", "api-kit", "--file", file.to_str().unwrap(), "--dry-run"], 0);
        run(&["bundle", "edit", "api-kit", "--file", file.to_str().unwrap()], 0);
        run(&["bundle", "edit", "api-kit", "--file", file.to_str().unwrap()], 0);
    },
    bundle_delete_json: "bundle delete" => |s, run| {
        let file = bundle_setup(&s);
        checked(&s, "bundle create", &["bundle", "create", "--file", file.to_str().unwrap()], 0);
        let preview = run(&["bundle", "delete", "api-kit", "--dry-run"], 0);
        assert_eq!(preview, json!({"name":"api-kit","dry_run":true,"changed":true,"committed":false}));
        let report = run(&["bundle", "delete", "api-kit"], 0);
        assert_eq!(report, json!({"name":"api-kit","dry_run":false,"changed":true,"committed":true}));
        assert_eq!(checked(&s, "error", &["bundle", "delete", "api-kit"], 1)["error"]["code"], "BUNDLE_NOT_FOUND");
    },
    remove_json: "remove" => |s, run| {
        let file = bundle_setup(&s);
        checked(&s, "bundle create", &["bundle", "create", "--file", file.to_str().unwrap()], 0);
        let preview = run(&["remove", "alternate", "--dry-run"], 0);
        assert_eq!(preview["changed"], true);
        assert_eq!(preview["committed"], false);
        let report = run(&["remove", "alternate"], 0);
        assert_eq!(report["committed"], true);
        assert_eq!(report["bundles_changed"], json!(["api-kit"]));
        assert_eq!(report["removed_paths"], preview["removed_paths"]);
        assert!(report["removed_paths"].as_array().unwrap().iter().all(Value::is_string));
        assert_eq!(checked(&s, "error", &["remove", "alternate"], 1)["error"]["code"], "SKILL_NOT_FOUND");
        // Removing the only primary of a confirmed group needs a user decision.
        add(&s, "alternate");
        bundle_file(&s, "API kit");
        checked(&s, "bundle edit", &["bundle", "edit", "api-kit", "--file", file.to_str().unwrap()], 0);
        assert_eq!(checked(&s, "error", &["remove", "api"], 2)["error"]["code"], "BUNDLE_WOULD_BE_INVALID");
        checked(&s, "bundle delete", &["bundle", "delete", "api-kit"], 0);
        let p = project(&s);
        activate(&s, &p);
        assert_eq!(checked(&s, "error", &["remove", "api"], 2)["error"]["code"], "SKILL_IN_USE");
        // A bundle without members keeps the documented array types.
        let empty = run(&["remove", "alternate"], 0);
        assert_eq!(empty["bundles_changed"], json!([]));
    },
    bundle_list_json: "bundle list" => |s, run| {
        let file = bundle_setup(&s);
        assert_eq!(run(&["bundle", "list"], 0), json!([]));
        checked(&s, "bundle create", &["bundle", "create", "--file", file.to_str().unwrap()], 0);
        assert_eq!(run(&["bundle", "list"], 0).as_array().unwrap().len(), 1);
    },
    bundle_show_json: "bundle show" => |s, run| {
        let file = bundle_setup(&s);
        checked(&s, "bundle create", &["bundle", "create", "--file", file.to_str().unwrap()], 0);
        run(&["bundle", "show", "api-kit"], 0);
    },
    use_json: "use" => |s, run| {
        let file = bundle_setup(&s);
        let p = project(&s);
        run(&["use", "api", "--project", p.to_str().unwrap(), "--agent", "all", "--dry-run"], 0);
        checked(&s, "bundle create", &["bundle", "create", "--file", file.to_str().unwrap()], 0);
        let report = run(&["use", "api-kit", "--include-secondary", "alternate", "--project", p.to_str().unwrap(), "--agent", "claude"], 0);
        assert!(report["lock"]["restoration"]["directories"].as_array().unwrap().len() >= 2);
        run(&["use", "api", "--project", p.to_str().unwrap(), "--agent", "codex", "--mode", "copy"], 0);
    },
    status_json: "status" => |s, run| {
        setup(&s);
        run(&["status"], 0);
        let p = project(&s);
        run(&["status", "--project", p.to_str().unwrap()], 0);
        activate(&s, &p);
        assert_eq!(run(&["status"], 0)["projects"][0]["items"].as_array().unwrap().len(), 3);
    },
    unuse_json: "unuse" => |s, run| {
        setup(&s);
        let p = project(&s);
        activate(&s, &p);
        run(&["unuse", "api", "--project", p.to_str().unwrap(), "--dry-run"], 0);
        run(&["unuse", "api", "--project", p.to_str().unwrap(), "--agent", "claude"], 0);
        run(&["unuse", "api", "--project", p.to_str().unwrap()], 0);
    },
    adopt_json: "adopt" => |s, run| {
        let source = setup(&s);
        let p = project(&s);
        copy_tree(&source, &p.join(".claude/skills/api"));
        run(&["adopt", "--project", p.to_str().unwrap(), "--dry-run"], 0);
        let report = run(&["adopt", "--project", p.to_str().unwrap(), "--yes"], 0);
        assert!(report["activation"].is_object());
        fs::create_dir_all(p.join(".claude/skills/unknown")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&source, p.join(".claude/skills/foreign-link")).unwrap();
        let preview = run(&["adopt", "--project", p.to_str().unwrap(), "--dry-run"], 0);
        assert!(preview["activation"].is_null());
    },
    doctor_json: "doctor" => |s, run| {
        setup(&s);
        let p = project(&s);
        activate(&s, &p);
        run(&["doctor"], 0);
        fs::remove_file(s.vault.join("dist/api/SKILL.md")).unwrap();
        assert_eq!(run(&["doctor"], 1)["healthy"], false);
    },
    doctor_rebuild_json: "doctor --rebuild-index" => |s, run| {
        setup(&s);
        let p = project(&s);
        activate(&s, &p);
        fs::remove_file(s.home.join(".kinana/index-v2.db")).unwrap();
        run(&["doctor", "--rebuild-index", "--dry-run"], 0);
        let report = run(&["doctor", "--rebuild-index"], 0);
        assert_eq!(report["rebuild"]["observations"].as_array().unwrap().len(), 1);
    },
    doctor_repair_json: "doctor --repair" => |s, run| {
        setup(&s);
        let clean = run(&["doctor", "--repair"], 0);
        assert!(clean["backup_directory"].is_null());
        fs::write(s.vault.join("meta/api.toml"), "changed\n").unwrap();
        let preview = run(&["doctor", "--repair"], 0);
        let token = preview["preview_token"].as_str().unwrap().to_owned();
        let done = run(&["doctor", "--repair", "--confirm", &token], 0);
        assert!(done["after"].is_object());
    },
    outdated_json: "outdated" => |s, run| {
        setup(&s); let report=run(&["outdated"],0); assert_eq!(report["items"][0]["status"],"unsupported_source");
    },
    update_json: "update" => |s, _run| {
        setup(&s); checked(&s,"error",&["update","api"],2);
        // Successful preview/confirmation shapes use a real repository transport in source_updates.rs.
    },
    refresh_audit_json: "refresh-audit" => |s, run| {
        setup(&s);
        let current = run(&["refresh-audit"], 0);
        assert_eq!(current["older"], 0);
        assert_eq!(current["changed"], false);
        // An audit stored by older rules: preview, then confirm, then undo.
        let meta = s.vault.join("meta/api.toml");
        let mut value: toml::Value = toml::from_str(&fs::read_to_string(&meta).unwrap()).unwrap();
        value["audit"] = toml::from_str::<toml::Value>(
            "level='risky'\nfindings=['older reason']\nrules_version=2\n",
        )
        .unwrap();
        fs::write(&meta, toml::to_string(&value).unwrap()).unwrap();
        kinana_core::gitlog::commit_paths(&s.vault, "Older audit", &["meta/api.toml".into()]).unwrap();
        let before = fs::read(&meta).unwrap();
        let preview = run(&["refresh-audit"], 2);
        assert_eq!(preview["older"], 1);
        assert_eq!(preview["rules_version"], kinana_core::audit::RULES_VERSION);
        assert_eq!(preview["lowered"][0]["id"], "api");
        assert_eq!(preview["lowered"][0]["from"], "risky");
        assert_eq!(preview["requires_confirmation"], true);
        assert_eq!(fs::read(&meta).unwrap(), before, "a preview writes nothing");
        let token = preview["preview_token"].as_str().unwrap().to_owned();
        let dry = run(&["refresh-audit", "--confirm", &token, "--dry-run"], 0);
        assert_eq!(dry["committed"], false);
        assert_eq!(fs::read(&meta).unwrap(), before);
        checked(&s, "error", &["refresh-audit", "--confirm", "not-the-token"], 1);
        assert_eq!(fs::read(&meta).unwrap(), before);
        let done = run(&["refresh-audit", "--confirm", &token], 0);
        assert_eq!(done["committed"], true);
        let stored = kinana_core::edit::read_metadata(&vault(&s), "api").unwrap();
        assert_eq!(stored.audit.rules_version, kinana_core::audit::RULES_VERSION);
        checked(&s, "undo", &["undo"], 0);
        assert_eq!(fs::read(&meta).unwrap(), before, "one commit, reverted by undo");
    },
    undo_json: "undo" => |s, run| {
        setup(&s);
        run(&["undo", "--dry-run"], 0);
        run(&["undo"], 0);
    },
    help_command_json: "help" => |s, run| {
        run(&["help"], 0);
        run(&["help", "bundle", "create"], 0);
    },
    help_flag_json: "--help" => |s, run| {
        run(&["--help"], 0);
        run(&["use", "--help"], 0);
        let text = s.run(&["--help"]);
        assert!(String::from_utf8(text.stdout).unwrap().contains("Usage:"));
    },
    version_json: "--version" => |s, run| {
        run(&["--version"], 0);
        assert!(String::from_utf8(s.run(&["--version"]).stdout).unwrap().starts_with("kinana "));
    }
}

#[test]
fn every_cli_command_has_a_documented_and_exercised_contract() {
    fn leaves(command: &clap::Command, prefix: &str, out: &mut BTreeSet<String>) {
        for child in command.get_subcommands().filter(|c| c.get_name() != "help") {
            let name = format!("{prefix}{}", child.get_name());
            if child.has_subcommands() {
                leaves(child, &format!("{name} "), out);
            } else {
                out.insert(name);
            }
        }
    }
    let doc = contract::document();
    assert_eq!(doc["x-contract-version"], 8);
    // Pin the historical releases and validate the candidate version separately.
    assert_eq!(doc["x-released-in"], "0.2.0");
    assert_eq!(doc["x-current-release"]["contract"], 11);
    assert_eq!(doc["x-current-release"]["released-in"], "0.3.0");
    assert_eq!(doc["x-development-release"], env!("CARGO_PKG_VERSION"));
    let commands = doc["x-commands"].as_object().unwrap();
    let documented: BTreeSet<_> = commands.keys().map(String::as_str).collect();
    let exercised: BTreeSet<_> = COVERED_COMMANDS.iter().copied().collect();
    assert_eq!(exercised.len(), COVERED_COMMANDS.len());
    assert_eq!(documented, exercised);
    let mut actual = BTreeSet::new();
    leaves(&kinana::Args::command(), "", &mut actual);
    let ordinary = documented
        .iter()
        .filter(|name| !name.starts_with('-') && **name != "help" && !name.contains(" --"))
        .map(|name| (*name).to_owned())
        .collect();
    assert_eq!(actual, ordinary);
    for shape in commands
        .values()
        .chain(doc["$defs"].as_object().unwrap().values())
        .chain([&doc["x-error"]])
    {
        contract::check_schema(&doc, shape);
    }
}

#[test]
fn errors_have_code_reason_code_and_message_and_preserve_exit_semantics() {
    let s = Sandbox::new();
    checked(&s, "error", &["unknown-command"], 1);
    checked(&s, "error", &["use"], 1);
    setup(&s);
    checked(&s, "error", &["show", "absent"], 1);
    checked(
        &s,
        "error",
        &["classify", "--all", "--agent", "keywords"],
        2,
    );
    let p = project(&s);
    fs::create_dir_all(p.join(".claude/skills/api")).unwrap();
    checked(
        &s,
        "error",
        &[
            "use",
            "api",
            "--project",
            p.to_str().unwrap(),
            "--agent",
            "claude",
        ],
        2,
    );
}

#[test]
fn optional_restoration_and_rebuild_fields_are_checked_when_present() {
    let s = Sandbox::new();
    setup(&s);
    let p = project(&s);
    let empty = kinana_core::ProjectLock {
        vault: s.vault.clone(),
        restoration: None,
        bundle: None,
        active: vec![],
    };
    fs::write(p.join(".kinana.lock"), toml::to_string(&empty).unwrap()).unwrap();
    activate(&s, &p);
    let mut bytes = fs::read(s.home.join(".kinana/index-v2.db")).unwrap();
    bytes[100] = 0xff;
    fs::write(s.home.join(".kinana/index-v2.db"), bytes).unwrap();
    let rebuilt = checked(
        &s,
        "doctor --rebuild-index",
        &["doctor", "--rebuild-index"],
        0,
    );
    assert!(rebuilt["rebuild"]["backup"].is_string());
    let restoration = &rebuilt["rebuild"]["observations"][0]["lock"]["restoration"];
    assert!(restoration["original_empty_lock"].is_string());
    #[cfg(unix)]
    assert!(restoration["original_lock_mode"].is_u64());
    fs::remove_file(p.join(".kinana.lock")).unwrap();
    let broken = checked(
        &s,
        "doctor --rebuild-index",
        &["doctor", "--rebuild-index"],
        0,
    );
    assert!(broken["rebuild"]["observations"][0]["lock"].is_null());
}

#[test]
fn shape_changes_fail_while_paths_times_and_key_order_can_change() {
    let s = Sandbox::new();
    setup(&s);
    proposal(&s);
    let original = checked(&s, "show", &["show", "api"], 0);
    let doc = contract::document();
    let schema = &doc["x-commands"]["show"];
    let mut changed = original.clone();
    changed["metadata"]["added_at"] = json!("a different timestamp");
    changed["locations"] = json!(["/another/path"]);
    contract::validate(&doc, schema, &changed).unwrap();
    for (pointer, replacement) in [
        ("/metadata/proposal/confidence", json!("0.9")),
        ("/metadata/classification/tags", json!([42])),
        ("/metadata/proposal/stack", json!("node")),
        ("/metadata/proposal/stage", json!([7])),
        ("/skill/stack", Value::Null),
        ("/skill/stage", json!(["review", 1])),
        ("/metadata/classification/classified_from", Value::Null),
        ("/metadata/empty_directories", json!([false])),
        ("/metadata/description/active", json!(true)),
    ] {
        let mut changed = original.clone();
        *changed.pointer_mut(pointer).unwrap() = replacement;
        assert!(
            contract::validate(&doc, schema, &changed).is_err(),
            "accepted {pointer}"
        );
    }
    let mut missing = original.clone();
    missing["skill"].as_object_mut().unwrap().remove("id");
    assert!(
        contract::validate(&doc, schema, &missing)
            .unwrap_err()
            .contains("$.skill.id")
    );
    let mut extra = original;
    extra["metadata"]["proposal"]["future_field"] = json!("unexpected");
    assert!(
        contract::validate(&doc, schema, &extra)
            .unwrap_err()
            .contains("future_field")
    );
}

/// The prose of docs/cli-json.md for one type: `| `key` | `type` | ... |`
/// rows of the table that follows `anchor`, or `key: type` pairs written
/// inline on the line that starts with `anchor`.
fn documented_fields(anchor: &str, inline: bool) -> BTreeMap<String, String> {
    let text = include_str!("../../../docs/cli-json.md");
    let start = text
        .find(anchor)
        .unwrap_or_else(|| panic!("cli-json.md no longer has {anchor:?}"));
    let rest = &text[start..];
    let mut fields = BTreeMap::new();
    if inline {
        let line = rest.lines().next().unwrap();
        let definition = line.split("، و`").next().unwrap();
        for part in definition.split('`').skip(1).step_by(2) {
            if let Some((key, kind)) = part.split_once(": ") {
                fields.insert(key.to_owned(), kind.to_owned());
            }
        }
    } else {
        for line in rest
            .lines()
            .skip_while(|line| !line.starts_with("| المفتاح"))
            .skip(2)
            .take_while(|line| line.starts_with('|'))
        {
            let cells = line.split('|').map(str::trim).collect::<Vec<_>>();
            fields.insert(
                cells[1].trim_matches('`').to_owned(),
                cells[2].trim_matches('`').to_owned(),
            );
        }
    }
    assert!(!fields.is_empty(), "no fields documented after {anchor:?}");
    fields
}

/// The prose type of one schema property: `string`, `integer`, `X[]`.
fn schema_type(property: &Value) -> String {
    if let Some(reference) = property["$ref"].as_str() {
        return reference.strip_prefix("#/$defs/").unwrap().to_owned();
    }
    match property["type"].as_str().unwrap() {
        "array" => format!("{}[]", schema_type(&property["items"])),
        other => other.to_owned(),
    }
}

/// The JSON type of one real value, spelt like the prose.
fn value_type(value: &Value, item: &str) -> String {
    match value {
        Value::String(_) => "string".into(),
        Value::Bool(_) => "boolean".into(),
        Value::Number(number) if number.is_u64() || number.is_i64() => "integer".into(),
        Value::Array(items) => {
            assert!(!items.is_empty(), "an empty array proves nothing: {item}");
            format!("{item}[]")
        }
        Value::Object(_) => item.into(),
        other => panic!("unexpected {other}"),
    }
}

/// Contract version 8, field by field: for every type the version 7 and 8
/// sections describe in prose (the brief catalog and its parts), the prose,
/// the machine-readable schema and the JSON a real run prints name the same
/// keys with the same types, every array is checked with items in it, and
/// every error code and doctor check those sections name is produced with
/// its documented exit code. The per-command tests above already refuse an
/// undocumented field and a missing required one; this one also refuses a
/// documented field that no real output carries.
#[test]
fn version_8_prose_schema_and_real_output_agree_field_by_field() {
    let doc = contract::document();
    assert_eq!(doc["x-contract-version"], 8);
    // Pin the historical releases and validate the candidate version separately.
    assert_eq!(doc["x-released-in"], "0.2.0");
    assert_eq!(doc["x-current-release"]["contract"], 11);
    assert_eq!(doc["x-current-release"]["released-in"], "0.3.0");
    assert_eq!(doc["x-development-release"], env!("CARGO_PKG_VERSION"));
    let types = [
        (
            "BriefCatalog",
            documented_fields("يعيد `BriefCatalog`، كائنًا في الجذر", false),
        ),
        (
            "BriefSkill",
            documented_fields("`BriefSkill`، وكل مفاتيحه حاضرة دائمًا", false),
        ),
        (
            "BriefBundle",
            documented_fields("`BriefBundle`: `name: string`", true),
        ),
        (
            "BriefBundleSkill",
            documented_fields("`BriefBundleSkill`: `id: string`", true),
        ),
    ];
    assert_eq!(doc["x-development-contract"], 12);
    let mut types = types;
    for (name, fields) in &mut types {
        if *name == "BriefSkill" || *name == "BriefBundleSkill" {
            fields.insert("audit_level".into(), "string".into());
        } else if *name == "BriefBundle" {
            fields.insert("risky_members".into(), "integer".into());
        }
    }
    // Prose against schema: the same keys, each with the same type, all required.
    for (name, prose) in &types {
        let schema = &doc["$defs"][*name];
        let properties = schema["properties"].as_object().unwrap();
        let from_schema = properties
            .iter()
            .map(|(key, property)| (key.clone(), schema_type(property)))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(&from_schema, prose, "{name}: prose and schema differ");
        let required = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|key| key.as_str().unwrap().to_owned())
            .collect::<BTreeSet<_>>();
        // Every key is always present, except the two a bundle shown by
        // name and member count only leaves out (documented in the prose).
        let optional: &[&str] = if *name == "BriefBundle" {
            &["title", "skills"]
        } else {
            &[]
        };
        assert_eq!(
            required,
            prose
                .keys()
                .filter(|key| !optional.contains(&key.as_str()))
                .cloned()
                .collect::<BTreeSet<_>>(),
            "{name}: the prose says which keys are always present"
        );
    }

    // Real output with every array non-empty: two pages, a bundle with
    // members, a skill with stack and stage values.
    let s = Sandbox::new();
    let file = bundle_setup(&s);
    checked(
        &s,
        "bundle create",
        &["bundle", "create", "--file", file.to_str().unwrap()],
        0,
    );
    proposal(&s);
    checked(
        &s,
        "dimensions",
        &["dimensions", "api", "--stack", "node", "--stage", "review"],
        0,
    );
    checked(
        &s,
        "dimensions",
        &[
            "dimensions",
            "alternate",
            "--stack",
            "react",
            "--stage",
            "build",
        ],
        0,
    );
    let first = checked(&s, "list --brief", &["list", "--brief", "--limit", "1"], 0);
    let second = checked(
        &s,
        "list --brief",
        &["list", "--brief", "--limit", "1", "--page", "2"],
        0,
    );
    assert!(!first["hint"].as_str().unwrap().is_empty());
    assert_eq!(second["bundles"], json!([]));
    let item_type =
        |prose: &BTreeMap<String, String>, key: &str| prose[key].trim_end_matches("[]").to_owned();
    let compare = |name: &str, prose: &BTreeMap<String, String>, value: &Value| {
        let object = value.as_object().unwrap();
        let keys = object.keys().cloned().collect::<BTreeSet<_>>();
        assert_eq!(
            keys,
            prose.keys().cloned().collect::<BTreeSet<_>>(),
            "{name}: real keys differ from the documented ones"
        );
        for (key, field) in object {
            assert_eq!(
                value_type(field, &item_type(prose, key)),
                prose[key],
                "{name}.{key}: real type differs from the documented one"
            );
        }
    };
    let prose = |name: &str| &types.iter().find(|(n, _)| *n == name).unwrap().1;
    compare("BriefCatalog", prose("BriefCatalog"), &first);
    compare("BriefSkill", prose("BriefSkill"), &first["skills"][0]);
    compare("BriefSkill", prose("BriefSkill"), &second["skills"][0]);
    compare("BriefBundle", prose("BriefBundle"), &first["bundles"][0]);
    compare(
        "BriefBundleSkill",
        prose("BriefBundleSkill"),
        &first["bundles"][0]["skills"][0],
    );

    // The index file version 8 documents for `Doctor` and `Init`.
    let doctor = checked(&s, "doctor", &["doctor"], 0);
    assert!(
        doctor["index"]
            .as_str()
            .unwrap()
            .ends_with(".kinana/index-v2.db")
    );
    // The doctor check version 8 adds, by its documented name and reason code.
    assert!(doctor["checks"].as_array().unwrap().iter().any(|check| {
        check["name"] == "interrupted_change" && check["reason_code"] == "DOCTOR_INTERRUPTED_CHANGE"
    }));
    // `metadata.proposal` keeps its stack and stage in JSON (version 8 moved
    // them to `[proposal_dimensions]` in the meta file only).
    let shown = checked(&s, "show", &["show", "api"], 0);
    assert_eq!(shown["metadata"]["proposal"]["stack"], json!(["node"]));
    assert_eq!(
        shown["metadata"]["proposal"]["stage"],
        json!(["design", "review"])
    );

    // Every error code the version 8 section and the remove table name, with
    // its documented exit code.
    let text = include_str!("../../../docs/cli-json.md");
    let row = |code: &str| {
        text.lines()
            .find(|line| line.starts_with('|') && line.contains(&format!("| `{code}` |")))
            .unwrap_or_else(|| panic!("{code} has no row in an error table"))
            .rsplit('|')
            .nth(1)
            .unwrap()
            .trim()
            .parse::<i32>()
            .unwrap()
    };
    // The brief catalog is documented in the version 7 section, which version 8 extends.
    let sections = text
        .split("## الفهرس المختصر ونص الموجّه — عقد إصدار7")
        .nth(1)
        .unwrap();
    for code in [
        "SKILL_HAS_UNRECOVERABLE_CONTENT",
        "SKILL_IN_USE",
        "INVALID_ARGUMENTS",
        "INDEX_MISSING",
    ] {
        assert!(
            sections.contains(&format!("`{code}`")),
            "{code} is not in the version 7 or 8 sections"
        );
    }
    fs::create_dir(s.vault.join("skills/alternate/.git")).unwrap();
    fs::write(
        s.vault.join("skills/alternate/.git/HEAD"),
        "ref: refs/heads/main\n",
    )
    .unwrap();
    let error = checked(
        &s,
        "error",
        &["remove", "alternate"],
        row("SKILL_HAS_UNRECOVERABLE_CONTENT"),
    );
    assert_eq!(error["error"]["code"], "SKILL_HAS_UNRECOVERABLE_CONTENT");
    fs::remove_dir_all(s.vault.join("skills/alternate/.git")).unwrap();
    let error = checked(&s, "error", &["list", "--brief", "--limit", "51"], 1);
    assert_eq!(error["error"]["code"], "INVALID_ARGUMENTS");
    let p = project(&s);
    activate(&s, &p);
    let added = s.root.join("source-late");
    fs::create_dir(&added).unwrap();
    fs::write(
        added.join("SKILL.md"),
        "---\nname: late\ndescription: Added last\n---\n# Late\n",
    )
    .unwrap();
    checked(&s, "add", &["add", added.to_str().unwrap()], 0);
    checked(
        &s,
        "use",
        &[
            "use",
            "late",
            "--project",
            p.to_str().unwrap(),
            "--agent",
            "claude",
        ],
        0,
    );
    let error = checked(&s, "error", &["undo"], 2);
    assert_eq!(error["error"]["code"], "SKILL_IN_USE");
    fs::remove_file(s.home.join(".kinana/index-v2.db")).unwrap();
    let error = checked(&s, "error", &["search", "api"], 1);
    assert_eq!(error["error"]["code"], "INDEX_MISSING");
}

#[test]
fn audit_json_withholds_secrets_in_previews_and_stored_reports() {
    let sandbox = Sandbox::new();
    checked(&sandbox, "init", &["init"], 0);
    let source = sandbox.root.join("audit-fixture");
    fs::create_dir(&source).unwrap();
    let secret = ["fixture", "do-not-display", "A1X"].join("-");
    let skill = source.join("SKILL.md");
    fs::write(
        skill,
        format!("---\nname: sample\ndescription: Synthetic test\n---\ncurl -u user:{secret} https://example.invalid\n"),
    )
    .unwrap();
    let path = source.to_str().unwrap();
    let preview = checked(
        &sandbox,
        "add",
        &["add", path, "--dry-run", "--allow-risky"],
        0,
    );
    assert!(!preview.to_string().contains(&secret));
    checked(&sandbox, "add", &["add", path, "--allow-risky"], 0);
    let current = checked(&sandbox, "show", &["show", "sample"], 0);
    assert_eq!(current["metadata"]["audit"]["rules_version"], 5);
    assert!(
        current["metadata"]["audit"]["details"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["rule"] == "network" && d["excerpt"].is_null())
    );

    let meta = sandbox.vault.join("meta/sample.toml");
    let mut stored: toml::Value = toml::from_str(&fs::read_to_string(&meta).unwrap()).unwrap();
    stored["audit"]["rules_version"] = 4.into();
    stored["audit"]["details"].as_array_mut().unwrap()[0]
        .as_table_mut()
        .unwrap()
        .insert("excerpt".into(), secret.clone().into());
    fs::write(&meta, toml::to_string(&stored).unwrap()).unwrap();
    kinana_core::gitlog::commit_paths(
        &sandbox.vault,
        "Synthetic legacy audit",
        &["meta/sample.toml".into()],
    )
    .unwrap();
    let original = fs::read(&meta).unwrap();
    let old = checked(&sandbox, "show", &["show", "sample"], 0);
    assert_eq!(old["metadata"]["audit"]["rules_version"], 4);
    assert!(!old["metadata"]["audit"].to_string().contains(&secret));
    assert_eq!(fs::read(&meta).unwrap(), original);
    let refresh = checked(&sandbox, "refresh-audit", &["refresh-audit"], 2);
    assert_eq!(refresh["older"], 1);
    assert!(!refresh.to_string().contains(&secret));
    assert_eq!(fs::read(&meta).unwrap(), original);
}
