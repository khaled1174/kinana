//! Owner decision 2026-11-20, round 6: every audit check is linear in the
//! line length. A crafted line of 1 MB for each token pattern, and for the
//! other strong and weak patterns, must be audited in under one second in a
//! release build; the test fails if any line takes longer.
//!
//! Run it with:
//! `scripts/test-without-agents.sh cargo test --release -p kinana-core --test audit_timing`
//! A debug build is several times slower, so the test is skipped there.
//! Each line is timed as the best of three runs, so a busy machine does not
//! fail a linear scan.
use kinana_core::audit::audit_skill;
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

const SIZE: usize = 1 << 20;

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// `unit` repeated to 1 MB.
fn repeat(unit: &str) -> String {
    unit.repeat(SIZE / unit.len() + 1)[..SIZE].to_string()
}

/// `prefix`, then `body` repeated to 1 MB.
fn long(prefix: &str, body: &str) -> String {
    format!("{prefix}{}", repeat(body))
}

/// Every prefix the matchers try at each position, and the body bytes that
/// keep a run going.
const TOKENS: &[(&str, &str)] = &[
    ("ghp_", "aB3"),
    ("gho_", "aB3"),
    ("ghu_", "aB3"),
    ("ghs_", "aB3"),
    ("ghr_", "aB3"),
    ("github_pat_", "aB3_"),
    ("glpat-", "aB3-"),
    ("AKIA", "AB27"),
    ("ASIA", "AB27"),
    ("ABIA", "AB27"),
    ("ACCA", "AB27"),
    ("sk-ant-api03-", "aB3-"),
    ("sk-ant-admin01-", "aB3-"),
    ("AIza", "aB3-"),
    ("sk_live_", "aB3"),
    ("sk_test_", "aB3"),
    ("sk_prod_", "aB3"),
    ("rk_live_", "aB3"),
    ("rk_test_", "aB3"),
    ("rk_prod_", "aB3"),
    ("npm_", "a3"),
    ("NPM_", "A3"),
    ("xoxb-", "1"),
    ("xoxp-", "1"),
    ("xoxe-", "1"),
    ("sk-", "aB3-"),
];

/// Publish review (2026-11-25): values a secret scanner takes for a key are
/// joined at run time, so this file holds no such literal.
fn openai_marker() -> String {
    ["T3Blbk", "FJ"].concat()
}

/// A PEM header line, `kind` ending in a space when it is not empty.
fn pem_begin(kind: &str) -> String {
    format!("-----BEGIN {kind}PRIVATE KEY-----")
}

fn lines() -> Vec<(String, String)> {
    let mut lines = Vec::new();
    let tokens = TOKENS
        .iter()
        .map(|(prefix, body)| (prefix.to_string(), *body));
    let openai = (format!("sk-{}", openai_marker()), "aB3-");
    for (prefix, body) in tokens.chain([openai]) {
        let prefix = prefix.as_str();
        lines.push((format!("{prefix} repeated"), repeat(prefix)));
        lines.push((format!("{prefix} then a long body"), long(prefix, body)));
        lines.push((
            format!("{prefix} with a body, repeated"),
            repeat(&format!("{prefix}{}-", body.repeat(12))),
        ));
        lines.push((format!("{prefix} spaced"), repeat(&format!("{prefix} "))));
    }
    for (name, unit) in [
        ("slack groups", format!("xoxp-{}", "1234567890-")),
        ("openai marker", format!("sk-{}-", openai_marker())),
        ("pem header", pem_begin("")),
        ("pem rsa header", format!("{}\\n", pem_begin("RSA "))),
        (
            "pem proc-type",
            format!("{}\\nProc-Type: 4,", pem_begin("RSA ")),
        ),
    ] {
        lines.push((format!("{name} repeated"), repeat(&unit)));
    }
    for (name, unit) in [
        ("pem begin", "-----begin "),
        ("assignment", "token="),
        ("assignment spaced", "api_token = "),
        ("typed assignment", "api_token: str = \"a1\" "),
        ("secret names", "secret_password_token_"),
        ("header", "authorization: "),
        ("token word", "token "),
        ("aws word", "aws_x "),
        ("dollar", "$"),
        ("dollar brace", "${"),
        ("template", "`${a}"),
        ("open braces", "{"),
        ("plain refs", "$HOME "),
        ("assignments for expansion", "a=rm; $a "),
        ("ssh", ".ssh"),
        ("aws path", "~/.aws/"),
        ("env", ".env "),
        ("process env", "process.env."),
        ("delete words", "rm -rf "),
        ("delete name only", "rm "),
        ("delete commands", "rm;"),
        ("delete with one flag", "rm -r "),
        ("php assignments", "$a = "),
        ("perl declarations", "my ($a, $b) = "),
        ("assignment targets", "$api_token .= "),
        ("ifs", "${IFS}"),
        ("json escape", "\\u0072"),
        ("remove-item", "Remove-Item -Recurse "),
        ("cmd", "rd /s "),
        ("backticks", "` "),
        ("caret", "^ "),
        ("git push", "git push "),
        ("downloader", "curl https://x "),
        ("pipe", "| "),
        ("quotes", "'\""),
    ] {
        lines.push((format!("{name} repeated"), repeat(unit)));
    }
    // Review r11: a short reference to a long value, repeated (expansion).
    for (name, value, reference) in [
        ("expansion of commas", "a,".repeat(32), "$a"),
        (
            "expansion of delete words",
            format!("{}r", "rm,".repeat(21)),
            "$a",
        ),
        ("expansion of one letter", "b".repeat(64), "$a"),
        ("braced expansion", "a,".repeat(32), "${a}"),
    ] {
        lines.push((name.into(), long(&format!("a=\"{value}\"; "), reference)));
    }
    lines.push(("quoted delete then operands".into(), long("r''m ", "a ")));
    lines.push(("token then long value".into(), long("token = ", "aB3")));
    lines.push(("assignment then long name".into(), long("", "api_token")));
    lines.push(("pem then long body".into(), long(&pem_begin(""), "aB3+")));
    lines
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "timed in a release build: cargo test --release --test audit_timing"
)]
fn a_crafted_one_megabyte_line_is_audited_in_under_a_second() {
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("kinana-audit-timing-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let scratch = Scratch(root);
    let mut slow = Vec::new();
    let mut slowest = (Duration::ZERO, String::new());
    for (index, (name, line)) in lines().into_iter().enumerate() {
        let skill = scratch.0.join(format!("case-{index:03}"));
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            "---\nname: timing\ndescription: Timing case.\n---\n",
        )
        .unwrap();
        fs::write(skill.join("line.txt"), &line).unwrap();
        let elapsed = (0..3)
            .map(|_| {
                let start = Instant::now();
                audit_skill(&skill).unwrap();
                start.elapsed()
            })
            .min()
            .unwrap();
        if std::env::var_os("AUDIT_TIMING_ALL").is_some() {
            eprintln!("{elapsed:>12.3?}  {name}");
        }
        if elapsed > slowest.0 {
            slowest = (elapsed, name.clone());
        }
        if elapsed >= Duration::from_secs(1) {
            slow.push(format!("{name}: {elapsed:?}"));
        }
    }
    eprintln!("slowest line: {} in {:?}", slowest.1, slowest.0);
    assert!(slow.is_empty(), "over one second:\n{}", slow.join("\n"));
}
