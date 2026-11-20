#[allow(dead_code)]
mod support;
use serde_json::Value;
use std::fs;
use support::{Sandbox, snapshot};
fn run(s: &Sandbox, args: &[&str]) -> Value {
    let o = s.run(&[args, &["--json"]].concat());
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    serde_json::from_slice(&o.stdout).unwrap()
}
#[test]
fn combined_catalog_filters_and_manual_category_are_shared_with_cli() {
    let s = Sandbox::new();
    s.initialize();
    for (id, description) in [
        ("api", "Review API contracts"),
        ("visual", "Design screen layouts"),
    ] {
        let p = s.root.join(id);
        fs::create_dir(&p).unwrap();
        fs::write(
            p.join("SKILL.md"),
            format!(
                "---\nname: {id}\ndescription: {description}\n---\n# Instructions\n{description}\n"
            ),
        )
        .unwrap();
        run(&s, &["add", p.to_str().unwrap()]);
    }
    run(&s, &["category", "api", "backend-api"]);
    run(&s, &["pin", "api"]);
    let before = snapshot(&s.root);
    let selected = run(
        &s,
        &[
            "list",
            "--query",
            "contracts",
            "--category",
            "backend-api",
            "--audit",
            "clean",
            "--agent",
            "claude",
            "--state",
            "original",
            "--pinned",
        ],
    );
    assert_eq!(selected.as_array().unwrap().len(), 1);
    assert_eq!(selected[0]["id"], "api");
    assert_eq!(snapshot(&s.root), before);
    assert_eq!(run(&s, &["list", "--state", "pending"])[0]["id"], "visual");
    let invalid = s.run(&["category", "api", "not-configured", "--json"]);
    assert_eq!(invalid.status.code(), Some(1));
    assert_eq!(snapshot(&s.root), before);
}
