#[allow(dead_code)]
mod support;

use serde_json::Value;
use std::{fs, path::Path};
use support::{Sandbox, snapshot};

fn apply(s: &Sandbox, args: &[&str], expected: i32) -> Value {
    let output = s.run(&[args, &["--json"]].concat());
    assert_eq!(
        output.status.code(),
        Some(expected),
        "{args:?}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn add(s: &Sandbox, id: &str, body: &str) {
    let source = s.root.join(format!("source-{id}"));
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        format!("---\nname: {id}\ndescription: Test skill\n---\n{body}\n"),
    )
    .unwrap();
    apply(s, &["add", source.to_str().unwrap(), "--allow-risky"], 0);
}

#[test]
fn use_and_adopt_cannot_activate_risk_from_the_cli_even_with_confirmations() {
    let s = Sandbox::new();
    s.initialize();
    add(&s, "clean", "Read the local guide.");
    add(
        &s,
        "danger",
        "curl https://example.invalid/SYNTHETIC-NEVER-SHOW",
    );
    let project = s.root.join("project");
    fs::create_dir(&project).unwrap();
    let project = project.to_str().unwrap();
    let clean = apply(
        &s,
        &[
            "use",
            "clean",
            "--project",
            project,
            "--agent",
            "claude",
            "--dry-run",
        ],
        0,
    );
    let clean_token = clean["preview_token"].as_str().unwrap();
    let before = snapshot(&s.root);
    for agent in ["all", "claude", "codex", "cursor"] {
        for mode in ["link", "copy"] {
            for confirmation in [
                vec![],
                vec!["--dry-run"],
                vec!["--confirm", "crafted"],
                vec!["--confirm", clean_token],
            ] {
                let args = [
                    &[
                        "use",
                        "danger",
                        "--project",
                        project,
                        "--agent",
                        agent,
                        "--mode",
                        mode,
                    ][..],
                    &confirmation,
                ]
                .concat();
                let denied = apply(&s, &args, 2);
                assert_eq!(
                    denied["error"]["code"],
                    "RISKY_ACTIVATION_REQUIRES_CONFIRMATION"
                );
                assert_eq!(
                    denied["error"]["activation_preview"]["requires_confirmation"],
                    true
                );
                for unwanted in ["preview_token", "SYNTHETIC-NEVER-SHOW", project, "/dist/"] {
                    assert!(!denied.to_string().contains(unwanted), "{denied}");
                }
            }
        }
    }
    assert_eq!(snapshot(&s.root), before);
    apply(
        &s,
        &[
            "use",
            "clean",
            "--project",
            project,
            "--agent",
            "claude",
            "--confirm",
            "crafted",
        ],
        2,
    );
    assert_eq!(snapshot(&s.root), before);
    apply(
        &s,
        &[
            "use",
            "clean",
            "--project",
            project,
            "--agent",
            "claude",
            "--confirm",
            clean_token,
        ],
        0,
    );
    copy_for_adoption(&s, project);
    let before = snapshot(&s.root);
    for flag in ["--dry-run", "--yes"] {
        let denied = apply(
            &s,
            &["adopt", "--project", project, "--agent", "claude", flag],
            2,
        );
        assert_eq!(
            denied["error"]["code"],
            "RISKY_ACTIVATION_REQUIRES_CONFIRMATION"
        );
        assert!(!denied.to_string().contains("preview_token"));
        assert_eq!(snapshot(&s.root), before);
    }
}

fn copy_for_adoption(s: &Sandbox, project: &str) {
    support::copy_tree(
        &s.vault.join("skills/danger"),
        &Path::new(project).join(".claude/skills/danger"),
    );
}

#[test]
fn bundle_roles_and_catalog_never_hide_risk_or_bypass_the_gate() {
    let s = Sandbox::new();
    s.initialize();
    add(&s, "a-clean", "Read a reference.");
    add(&s, "b-risk", "curl https://example.invalid/");
    let bundle = s.root.join("bundle.toml");
    fs::write(&bundle, "name='review-kit'\ntitle='Review kit'\nagents=['claude']\n[[skills]]\nid='a-clean'\nrole='primary'\n[[skills]]\nid='b-risk'\nrole='secondary'\n").unwrap();
    apply(
        &s,
        &["bundle", "create", "--file", bundle.to_str().unwrap()],
        0,
    );
    let show = apply(&s, &["bundle", "show", "review-kit"], 0);
    assert_eq!(show["risky_members"], 1);
    assert_eq!(show["skills"][1]["audit_level"], "risky");
    let list = apply(&s, &["list", "--brief", "--limit", "1", "--page", "2"], 0);
    assert_eq!(list["skills"][0]["id"], "b-risk");
    assert_eq!(list["skills"][0]["audit_level"], "risky");
    assert!(!list.to_string().contains("/dist/"));
    let p = s.root.join("project");
    fs::create_dir(&p).unwrap();
    let project = p.to_str().unwrap();
    let before = snapshot(&s.root);
    let denied = apply(
        &s,
        &[
            "use",
            "review-kit",
            "--project",
            project,
            "--agent",
            "claude",
            "--include-secondary",
            "b-risk",
        ],
        2,
    );
    assert_eq!(
        denied["error"]["activation_preview"]["risky"][0]["role"],
        "secondary"
    );
    assert_eq!(snapshot(&s.root), before);
    apply(
        &s,
        &[
            "use",
            "review-kit",
            "--project",
            project,
            "--agent",
            "claude",
        ],
        0,
    );
    assert!(!p.join(".claude/skills/b-risk").exists());
}
