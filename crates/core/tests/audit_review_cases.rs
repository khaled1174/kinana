//! Every case named by the independent reviews of rounds 8 and 9 (H*, M*,
//! B01-B28, X01-X03 and the control, review, stricter and not-covered probes,
//! and the owner's r7 secret names),
//! with its level under rules version 2 (f40a88c). Owner decision 2026-11-20,
//! round 5: a case that was risky stays risky, except where an owner decision
//! named in the fixture keeps it below; and no recursive delete drops below
//! risky. Secret-like values are generated at run time from their shapes, and
//! PEM headers, key prefixes and credentials are joined from fixed pieces, so
//! the fixture holds no literal that looks like a real token or key.
mod support {
    pub mod audit_v2;
    pub mod old_vs_new;
}
use kinana_core::audit::audit_skill;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "kinana-audit-review-cases-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// xorshift64*: deterministic and dependency-free.
struct Random(u64);
impl Random {
    fn pick(&mut self, alphabet: &[u8]) -> char {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let n = self.0.wrapping_mul(0x2545_f491_4f6c_dd1d);
        char::from(alphabet[(n % alphabet.len() as u64) as usize])
    }
}

fn fixture() -> Value {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/audit-review/cases.json");
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

/// A value with the shape's prefix and the class of every character.
fn values(shapes: &Value) -> Vec<(String, String)> {
    let mut random = Random(0x4b49_4e41_4e41_0005);
    let mut list = shapes
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, shape)| {
            let mut value = shape["prefix"].as_str().unwrap().to_string();
            for class in shape["pattern"].as_str().unwrap().chars() {
                value.push(match class {
                    'U' => random.pick(b"ABCDEFGHIJKLMNOPQRSTUVWXYZ"),
                    'l' => random.pick(b"abcdefghijklmnopqrstuvwxyz"),
                    'd' => random.pick(b"0123456789"),
                    '7' => random.pick(b"234567"),
                    'h' => random.pick(b"0123456789abcdef"),
                    other => other,
                });
            }
            value.push_str(shape["suffix"].as_str().unwrap());
            (format!("{{{{{name}}}}}"), value)
        })
        .collect::<Vec<_>>();
    // `{{V10}}` before `{{V1}}`.
    list.sort_by_key(|(name, _)| std::cmp::Reverse(name.len()));
    // Publish review (2026-11-25): fixed pieces that would join a PEM header,
    // a key prefix or a curl credential into a literal in the fixture.
    list.extend(
        [
            ("{{BEGIN}}", "-----BEGIN"),
            ("{{begin}}", "-----begin"),
            ("{{sk}}", "sk-"),
            ("{{USER}}", "admin"),
        ]
        .map(|(name, value)| (name.to_string(), value.to_string())),
    );
    list
}

struct Outcome {
    id: String,
    /// Live old-versus-new failures (support/old_vs_new.rs).
    old_vs_new: Vec<String>,
    /// The old level computed live from the verbatim old rules.
    old_live_risky: bool,
    old: String,
    below_risky: Option<String>,
    recursive_delete: bool,
    level: String,
    strong_destructive: bool,
}

fn outcomes() -> Vec<Outcome> {
    let fixture = fixture();
    let values = values(&fixture["shapes"]);
    let scratch = Scratch::new();
    fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(index, case)| {
            let skill = scratch.0.join(format!("case-{index:04}"));
            for (path, body) in case["files"].as_object().unwrap() {
                let mut body = body.as_str().unwrap().to_string();
                for (name, value) in &values {
                    body = body.replace(name, value);
                }
                let file = skill.join(path);
                fs::create_dir_all(file.parent().unwrap()).unwrap();
                fs::write(file, body).unwrap();
            }
            let audit = audit_skill(&skill).unwrap();
            let id = case["id"].as_str().unwrap();
            Outcome {
                old_vs_new: support::old_vs_new::check(
                    id,
                    &skill,
                    &audit,
                    case["below_risky"].as_str(),
                ),
                old_live_risky: support::old_vs_new::old(&skill, &audit).risky,
                id: id.to_string(),
                old: case["old"].as_str().unwrap().to_string(),
                below_risky: case["below_risky"].as_str().map(str::to_string),
                recursive_delete: case["recursive_delete"].as_bool().unwrap(),
                strong_destructive: audit
                    .details
                    .iter()
                    .any(|d| d.rule == "destructive" && d.strength == "strong"),
                level: audit.level,
            }
        })
        .collect()
}

#[test]
fn every_case_risky_under_the_old_rules_stays_risky_unless_an_owner_decision_says_otherwise() {
    let outcomes = outcomes();
    assert_eq!(outcomes.len(), 1243);
    let old_risky = outcomes.iter().filter(|o| o.old == "risky").count();
    let mut failures = Vec::new();
    for o in outcomes.iter().filter(|o| o.old == "risky") {
        match &o.below_risky {
            None if o.level != "risky" => {
                failures.push(format!("{}: risky under v2, now {}", o.id, o.level));
            }
            // The named exceptions are pinned, so the list stays exact.
            Some(reason) if o.level == "risky" => {
                failures.push(format!("{}: listed below risky ({reason}) but risky", o.id));
            }
            _ => {}
        }
    }
    let exceptions = outcomes.iter().filter(|o| o.below_risky.is_some()).count();
    assert_eq!(exceptions, 82);
    assert!(
        failures.is_empty(),
        "{} of {old_risky} old-risky cases failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn no_recursive_delete_case_drops_below_risky() {
    let outcomes = outcomes();
    let deletes = outcomes
        .iter()
        .filter(|o| o.recursive_delete)
        .collect::<Vec<_>>();
    assert_eq!(deletes.len(), 640);
    let failures = deletes
        .iter()
        .filter(|o| o.level != "risky" || !o.strong_destructive)
        .map(|o| {
            format!(
                "{}: {} (strong destructive: {})",
                o.id, o.level, o.strong_destructive
            )
        })
        .collect::<Vec<_>>();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn the_old_rules_run_live_agree_with_the_recorded_old_levels() {
    // The recorded levels come from the old binary (f40a88c); the live ones
    // from the verbatim copy of its rules. They must agree on every case.
    let disagree = outcomes()
        .iter()
        .filter(|o| o.old_live_risky != (o.old == "risky"))
        .map(|o| {
            format!(
                "{}: recorded {}, live risky {}",
                o.id, o.old, o.old_live_risky
            )
        })
        .collect::<Vec<_>>();
    assert!(disagree.is_empty(), "{}", disagree.join("\n"));
}

#[test]
fn risky_under_the_old_rules_is_risky_now_unless_an_approved_exception_names_it() {
    let failures = outcomes()
        .into_iter()
        .flat_map(|o| o.old_vs_new)
        .collect::<Vec<_>>();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
