//! Owner decision 2026-11-20, round 6: risky(old) ⊆ risky(new) ∪ approved
//! exceptions. The old level of a skill is computed here, at run time, from
//! the verbatim rules-version-2 tests in `audit_v2.rs` together with the
//! current audit's findings for the rules that did not change.
use super::audit_v2;
use kinana_core::Audit;
use std::{fs, path::Path};

/// What the old rules said about one skill.
pub struct Old {
    pub risky: bool,
    /// Strong secrets under the old rules once plain references are blanked:
    /// what the approved exception does not cover.
    pub strong_secrets: bool,
    pub strong_destructive: bool,
}

fn approved_nonsecret_default_in_delete(id: &str) -> bool {
    if id == "r8-destr-rm -rf ${HOME:-/}" {
        return true;
    }
    let Some(tail) = id
        .strip_prefix("destructive-closed-home-default-")
        .or_else(|| id.strip_prefix("destructive-closed-var-default-root-"))
    else {
        return false;
    };
    let Some((layout, context)) = tail.split_once('-') else {
        return false;
    };
    matches!(layout, "plain" | "spaces" | "tabs")
        && matches!(context, "fence" | "inline" | "prose" | "script" | "data")
}

fn texts(dir: &Path, out: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let kind = fs::symlink_metadata(&path).unwrap().file_type();
        if kind.is_dir() {
            texts(&path, out);
        } else if kind.is_file() {
            out.push(String::from_utf8_lossy(&fs::read(&path).unwrap()).into_owned());
        }
    }
}

/// The owner's rule for a name that holds a secret (2026-11-24, r7 item 4),
/// written here independently of `environment::secret_name`: boundaries
/// (`_`, `-`, digits, camelCase) become spaces, and a whole part is KEY,
/// TOKEN, SECRET, PASSWORD, PASSWD, AUTH, CREDENTIAL, PRIVATE, SESSION,
/// COOKIE or PAT; or the name contains SECRET, TOKEN, PASSWORD, PASSWD,
/// CREDENTIAL, APIKEY, PRIVATEKEY or ACCESSKEY. The code may be stricter
/// (a part ending in PAT, a mixed-case whole part), never laxer.
pub fn owner_secret_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    if [
        "SECRET",
        "TOKEN",
        "PASSWORD",
        "PASSWD",
        "CREDENTIAL",
        "APIKEY",
        "PRIVATEKEY",
        "ACCESSKEY",
    ]
    .iter()
    .any(|word| upper.contains(word))
    {
        return true;
    }
    let chars = name.chars().collect::<Vec<_>>();
    let mut spaced = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c == '_' || c == '-' || c.is_ascii_digit() {
            spaced.push(' ');
            continue;
        }
        if i > 0 {
            let before = chars[i - 1];
            let after = chars.get(i + 1).copied();
            if (before.is_ascii_lowercase() && c.is_ascii_uppercase())
                || (before.is_ascii_uppercase()
                    && c.is_ascii_uppercase()
                    && after.is_some_and(|a| a.is_ascii_lowercase()))
            {
                spaced.push(' ');
            }
        }
        spaced.push(c);
    }
    spaced.split_whitespace().any(|part| {
        [
            "KEY",
            "TOKEN",
            "SECRET",
            "PASSWORD",
            "PASSWD",
            "AUTH",
            "CREDENTIAL",
            "PRIVATE",
            "SESSION",
            "COOKIE",
            "PAT",
        ]
        .contains(&part.to_ascii_uppercase().as_str())
    })
}

/// The names after `$` or `${` in `text` as written, lowercased, that the
/// owner's rule calls secret. The old rules read lowercased text, where a
/// camelCase boundary (`$sessionId`) is gone; this keeps it.
fn secret_references(text: &str) -> std::collections::BTreeSet<String> {
    let b = text.as_bytes();
    let mut names = std::collections::BTreeSet::new();
    for (at, _) in text.match_indices('$') {
        let start = at + 1 + usize::from(b.get(at + 1) == Some(&b'{'));
        let len = b[start.min(b.len())..]
            .iter()
            .take_while(|c| c.is_ascii_alphanumeric() || **c == b'_')
            .count();
        let name = &text[start..start + len];
        if len > 0 && owner_secret_name(name) {
            names.insert(name.to_ascii_lowercase());
        }
    }
    names
}

/// The approved secret exception, written independently of the rules and
/// narrower than them: every `$NAME` and `${NAME}` (ASCII names) becomes a
/// space, except a variable being written (assigned, indexed or declared)
/// and, owner decision (r3, recorded 2026-11-23; rule r7), a name that holds
/// a secret: `secret` says which (the text here is lowercased).
/// The third approved form, `${expr}` in a JavaScript template literal, is
/// not read here: a case that
/// needs it is named as an exception in its fixture.
fn blank_plain_references(text: &str, secret: &dyn Fn(&str) -> bool) -> String {
    let b = text.as_bytes();
    let name = |at: usize| {
        if !b
            .get(at)
            .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
        {
            return 0;
        }
        b[at..]
            .iter()
            .take_while(|c| c.is_ascii_alphanumeric() || **c == b'_')
            .count()
    };
    // Written, not referenced: assigned (`=`, `.=`, `+=`, `||=`, ...),
    // indexed (`$a[`, `$h{`), or declared with `my`/`our`/`local`.
    let assigned = |start: usize, end: usize| {
        if matches!(b.get(end), Some(b'[' | b'{')) {
            return true;
        }
        let before = String::from_utf8_lossy(&b[..start]);
        let before = before.trim_end();
        if ["my", "our", "local"].iter().any(|w| {
            before.ends_with(w)
                && !before[..before.len() - w.len()]
                    .ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
        }) {
            return true;
        }
        let rest = &b[end..];
        let at = rest.iter().take_while(|c| c.is_ascii_whitespace()).count();
        let rest = &rest[at..];
        let op = rest
            .iter()
            .take_while(|c| b".+-*/%|&^x?<>".contains(c))
            .count();
        rest.get(op) == Some(&b'=')
            && !matches!(rest.get(op + 1), Some(b'=' | b'~'))
            && !matches!(&rest[..op], [b'<'] | [b'>'])
    };
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'$' {
            let plain = name(i + 1);
            if plain > 0
                && !assigned(i, i + 1 + plain)
                && !secret(&String::from_utf8_lossy(&b[i + 1..i + 1 + plain]))
            {
                out.push(b' ');
                i += 1 + plain;
                continue;
            }
            if b.get(i + 1) == Some(&b'{') {
                let braced = name(i + 2);
                if braced > 0
                    && b.get(i + 2 + braced) == Some(&b'}')
                    && !secret(&String::from_utf8_lossy(&b[i + 2..i + 2 + braced]))
                {
                    out.push(b' ');
                    i += 3 + braced;
                    continue;
                }
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn old_secrets_beyond_references(text: &str) -> bool {
    // Blanked in the text as the old rules read it, so `$\` + newline + `HOME`
    // and `$` + zero-width space + `HOME` are the plain `$home` they matched.
    let written = secret_references(text);
    let secret = |name: &str| owner_secret_name(name) || written.contains(name);
    audit_v2::strong_secrets(&blank_plain_references(&audit_v2::normalize(text), &secret))
}

/// The old audit of `skill`, given the current audit of the same skill.
pub fn old(skill: &Path, current: &Audit) -> Old {
    let mut files = Vec::new();
    texts(skill, &mut files);
    // Every line, and the whole file as the fallback check reads it.
    let any = |test: fn(&str) -> bool| {
        files
            .iter()
            .any(|text| test(text) || text.lines().any(test))
    };
    let old_secrets = any(audit_v2::strong_secrets);
    let strong_secrets = any(old_secrets_beyond_references);
    let strong_destructive = any(audit_v2::strong_destructive);
    // Rules that are the same code in both versions, and file-level findings
    // (a sensitive path is recorded under `secrets` without a line).
    let unchanged = current.details.iter().any(|d| {
        d.strength == "strong"
            && (!matches!(d.rule.as_str(), "secrets" | "destructive") || d.line.is_none())
    });
    Old {
        risky: unchanged || old_secrets || strong_destructive,
        strong_secrets,
        strong_destructive,
    }
}

/// Every way `current` falls short of the old rules for one case. A case
/// named as an approved exception must be risky under the old rules and below
/// risky now, so the list of exceptions stays exact.
pub fn check(id: &str, skill: &Path, current: &Audit, exception: Option<&str>) -> Vec<String> {
    let old = old(skill, current);
    let strong = |rule: &str| {
        current
            .details
            .iter()
            .any(|d| d.rule == rule && d.strength == "strong")
    };
    let mut failures = Vec::new();
    match exception {
        Some(reason) => {
            if !old.risky || current.level == "risky" {
                failures.push(format!(
                    "{id}: named exception ({reason}) but old risky {} and now {}",
                    old.risky, current.level
                ));
            }
        }
        None => {
            if old.risky && current.level != "risky" {
                failures.push(format!(
                    "{id}: risky under the old rules, now {}",
                    current.level
                ));
            }
            if old.strong_secrets && !strong("secrets") && !approved_nonsecret_default_in_delete(id)
            {
                failures.push(format!("{id}: strong secrets under the old rules, not now"));
            }
            if old.strong_destructive && !strong("destructive") {
                failures.push(format!(
                    "{id}: strong destructive under the old rules, not now"
                ));
            }
        }
    }
    failures
}
