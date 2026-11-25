//! Strong rules for secrets and destructive commands (rules version 3).
//!
//! Owner decision 2026-11-20, round 6: the new rules are the old rules
//! (rules version 2, f40a88c) in full, plus new patterns, minus two approved
//! exceptions named here and nowhere else:
//! - a match of the old secret test whose matched part is a plain variable
//!   reference (`$NAME`, `${NAME}`, or `${expr}` inside a JavaScript template
//!   literal) is dropped; every other clause of the old test runs unchanged;
//! - `chmod 777` is review, not strong.
//!
//! The new secret patterns are known token shapes, a PEM private key with its
//! body, a high-entropy literal assigned to a secret-like name, and named
//! credential files. A recursive forced delete is strong whatever its target;
//! the old token reading runs next to readings of every spelling a shell would
//! run, and the result is their union. Every check is linear in the line
//! length: each matcher looks at a bounded window from each position.
//!
//! The token shapes and entropy thresholds follow the rules of gitleaks
//! v8.30.1 (`config/gitleaks.toml` at commit
//! 83d9cd684c87d95d656c1458ef04895a7f1cbd8e), Copyright (c) 2019 Zachary Rice,
//! MIT License; see THIRD_PARTY_NOTICES.md. Only the shapes are reused; the
//! matchers below are written here without a regular-expression dependency.
use super::*;

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}
fn is_word_dash(b: u8) -> bool {
    is_word(b) || b == b'-'
}
fn is_alnum(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}
fn is_aws_body(b: u8) -> bool {
    b.is_ascii_uppercase() || (b'2'..=b'7').contains(&b)
}
fn is_lower_alnum(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit()
}
fn is_digit(b: u8) -> bool {
    b.is_ascii_digit()
}
fn is_value(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'_' | b'.' | b'~' | b'-')
}

/// Shannon entropy in bits per byte.
fn entropy(bytes: &[u8]) -> f64 {
    if bytes.is_empty() {
        return 0.0;
    }
    let mut counts = [0usize; 256];
    for &b in bytes {
        counts[usize::from(b)] += 1;
    }
    let n = bytes.len() as f64;
    counts
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / n;
            -p * p.log2()
        })
        .sum()
}

/// Length of the run of `ok` bytes at `start`, counting at most `cap`, so a
/// matcher tried at every position stays linear in the line length.
fn run(bytes: &[u8], start: usize, ok: fn(u8) -> bool, cap: usize) -> usize {
    bytes.get(start..).map_or(0, |rest| {
        rest.iter().take(cap).take_while(|&&b| ok(b)).count()
    })
}

fn starts_with_at(bytes: &[u8], at: usize, prefix: &str) -> bool {
    bytes
        .get(at..at + prefix.len())
        .is_some_and(|slice| slice == prefix.as_bytes())
}

struct Format {
    prefix: &'static str,
    body: fn(u8) -> bool,
    min: usize,
    max: usize,
    /// The body must be followed by a non-word byte or the end.
    end_boundary: bool,
    suffix: &'static str,
    entropy: f64,
}

const fn format(
    prefix: &'static str,
    body: fn(u8) -> bool,
    (min, max): (usize, usize),
    end_boundary: bool,
    entropy: f64,
) -> Format {
    Format {
        prefix,
        body,
        min,
        max,
        end_boundary,
        suffix: "",
        entropy,
    }
}

/// gitleaks rule ids in brackets.
const FORMATS: &[Format] = &[
    // [github-pat] [github-oauth] [github-app-token] [github-refresh-token]
    format("ghp_", is_alnum, (36, 255), false, 3.0),
    format("gho_", is_alnum, (36, 255), false, 3.0),
    format("ghu_", is_alnum, (36, 255), false, 3.0),
    format("ghs_", is_alnum, (36, 255), false, 3.0),
    format("ghr_", is_alnum, (36, 255), false, 3.0),
    // [github-fine-grained-pat]
    format("github_pat_", is_word, (82, 255), false, 3.0),
    // [gitlab-pat]
    format("glpat-", is_word_dash, (20, 255), false, 3.0),
    // [aws-access-token]
    format("AKIA", is_aws_body, (16, 16), true, 3.0),
    format("ASIA", is_aws_body, (16, 16), true, 3.0),
    format("ABIA", is_aws_body, (16, 16), true, 3.0),
    format("ACCA", is_aws_body, (16, 16), true, 3.0),
    // [anthropic-api-key] [anthropic-admin-api-key]: 93 characters, then `AA`.
    Format {
        prefix: "sk-ant-api03-",
        body: is_word_dash,
        min: 95,
        max: 95,
        end_boundary: true,
        suffix: "AA",
        entropy: 0.0,
    },
    Format {
        prefix: "sk-ant-admin01-",
        body: is_word_dash,
        min: 95,
        max: 95,
        end_boundary: true,
        suffix: "AA",
        entropy: 0.0,
    },
    // [gcp-api-key]
    format("AIza", is_word_dash, (35, 35), true, 4.0),
    // [stripe-access-token]
    format("sk_live_", is_alnum, (10, 99), true, 2.0),
    format("sk_test_", is_alnum, (10, 99), true, 2.0),
    format("sk_prod_", is_alnum, (10, 99), true, 2.0),
    format("rk_live_", is_alnum, (10, 99), true, 2.0),
    format("rk_test_", is_alnum, (10, 99), true, 2.0),
    format("rk_prod_", is_alnum, (10, 99), true, 2.0),
];

fn table_format_at(b: &[u8], i: usize) -> bool {
    FORMATS.iter().any(|f| {
        if !starts_with_at(b, i, f.prefix) {
            return false;
        }
        let start = i + f.prefix.len();
        // One byte past the maximum tells a longer body from an exact one.
        let body = run(b, start, f.body, f.max + 1);
        if body < f.min {
            return false;
        }
        let end = start + body.min(f.max);
        if f.end_boundary && body > f.max {
            return false;
        }
        if f.end_boundary && f.suffix.is_empty() && b.get(end).is_some_and(|&c| is_word(c)) {
            return false;
        }
        if !f.suffix.is_empty() && !b[..end].ends_with(f.suffix.as_bytes()) {
            return false;
        }
        entropy(&b[i..end]) >= f.entropy
    })
}

/// [npm-access-token]: case-insensitive prefix, 36 lowercase letters or digits.
fn npm_at(b: &[u8], i: usize) -> bool {
    let Some(prefix) = b.get(i..i + 4) else {
        return false;
    };
    if !prefix.eq_ignore_ascii_case(b"npm_") {
        return false;
    }
    let body = run(b, i + 4, |c| is_lower_alnum(c.to_ascii_lowercase()), 37);
    let lowered: Vec<u8> = b[i + 4..i + 4 + body]
        .iter()
        .map(u8::to_ascii_lowercase)
        .collect();
    body == 36 && entropy(&lowered) >= 2.0
}

/// [slack-bot-token] `xoxb-<10..13 digits>-<10..13 digits>` and
/// [slack-user-token] `xox[pe](-<10..13 digits>){3}-<28..34 [A-Za-z0-9-]>`.
fn slack_at(b: &[u8], i: usize) -> bool {
    let digits = |at: usize| run(b, at, is_digit, 14);
    if starts_with_at(b, i, "xoxb-") {
        let first = digits(i + 5);
        return (10..=13).contains(&first)
            && b.get(i + 5 + first) == Some(&b'-')
            && (10..=13).contains(&digits(i + 6 + first));
    }
    if !(starts_with_at(b, i, "xoxp") || starts_with_at(b, i, "xoxe")) {
        return false;
    }
    let mut at = i + 4;
    for _ in 0..3 {
        if b.get(at) != Some(&b'-') {
            return false;
        }
        let group = digits(at + 1);
        if !(10..=13).contains(&group) {
            return false;
        }
        at += 1 + group;
    }
    b.get(at) == Some(&b'-')
        && (28..=34).contains(&run(b, at + 1, |c| is_alnum(c) || c == b'-', 35))
}

/// [openai-api-key]: an `sk-` token holding the `T3BlbkFJ` marker. The
/// longest gitleaks shape is under 200 characters; only that window is read.
fn openai_at(b: &[u8], i: usize) -> bool {
    if !starts_with_at(b, i, "sk-") {
        return false;
    }
    let token = &b[i..i + 3 + run(b, i + 3, is_word_dash, 200)];
    token.len() >= 43 && token.windows(8).any(|w| w == b"T3BlbkFJ") && entropy(token) >= 3.0
}

pub(super) fn known_token_format(text: &str) -> bool {
    let b = text.as_bytes();
    (0..b.len()).any(|i| {
        (i == 0 || !is_word(b[i - 1]))
            && (table_format_at(b, i) || npm_at(b, i) || slack_at(b, i) || openai_at(b, i))
    })
}

/// The largest character boundary at or before `index`: windows over
/// untrusted text never split a multi-byte character.
fn boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// [private-key]: a PEM header followed by a body: at least 64 base64
/// characters (line breaks, JSON `\n`, `\r`, `\t` and `\/` escapes and
/// `Proc-Type:`/`DEK-Info:` header lines allowed), or at least 64 characters
/// and an end marker further on. Linear in the text length.
pub(super) fn private_key_block(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let last_end = ["key-----", "key block-----"]
        .iter()
        .filter_map(|marker| lower.rfind(marker))
        .max();
    let b = lower.as_bytes();
    let mut from = 0;
    while let Some(found) = lower[from..].find("-----begin") {
        let start = from + found;
        from = start + "-----begin".len();
        let window = &lower[start..boundary(&lower, start + 140)];
        let Some(label) = window.find("private key") else {
            continue;
        };
        let after = start + label + "private key".len();
        let header_end = if lower[after..].starts_with(" block-----") {
            after + " block-----".len()
        } else if lower[after..].starts_with("-----") {
            after + "-----".len()
        } else {
            continue;
        };
        if last_end.is_some_and(|end| end >= header_end + 64) {
            return true;
        }
        let (mut i, mut body) = (header_end, 0);
        let limit = header_end + 512;
        while i < b.len() && body < 64 && i < limit {
            let header = ["proc-type:", "dek-info:"]
                .iter()
                .any(|name| lower[i..].starts_with(name));
            match b[i] {
                _ if header => {
                    // Skip the header line up to its line break or `\n`.
                    while i < b.len() && i < limit && b[i] != b'\n' {
                        if b[i] == b'\\' && matches!(b.get(i + 1), Some(b'n' | b'r')) {
                            break;
                        }
                        i += 1;
                    }
                    continue;
                }
                c if c.is_ascii_alphanumeric() || matches!(c, b'+' | b'/' | b'=') => body += 1,
                b' ' | b'\t' | b'\r' | b'\n' => {}
                b'\\' if matches!(b.get(i + 1), Some(b'n' | b'r' | b't')) => i += 1,
                b'\\' if b.get(i + 1) == Some(&b'/') => {}
                _ => break,
            }
            i += 1;
        }
        if body >= 64 {
            return true;
        }
    }
    false
}

/// A private-key header without a body, as in documentation.
pub(super) fn private_key_header(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("-----begin") && lower.contains("private key")
}

const SECRET_NAMES: &[&str] = &[
    "api_key",
    "apikey",
    "api-key",
    "secret",
    "token",
    "password",
    "passwd",
    "private_key",
    "privatekey",
    "access_key",
    "accesskey",
    "credential",
    "auth",
];
const PLACEHOLDERS: &[&str] = &[
    "example",
    "your",
    "xxxx",
    "placeholder",
    "changeme",
    "redacted",
    "dummy",
    "sample",
    "...",
];

fn secret_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SECRET_NAMES.iter().any(|key| lower.contains(key))
}

fn is_name_byte(b: u8) -> bool {
    is_word(b) || b == b'-' || b == b'.'
}

/// A secret-like name assigned a literal, high-entropy value. The 3.5-bit
/// threshold is the one gitleaks uses for its generic-api-key rule.
pub(super) fn secret_assignment(text: &str) -> bool {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if !is_name_byte(b[i]) || (i > 0 && is_name_byte(b[i - 1])) {
            i += 1;
            continue;
        }
        let len = run(b, i, is_name_byte, usize::MAX);
        let name = &text[i..i + len];
        let mut j = i + len;
        i += len;
        if !secret_name(name) {
            continue;
        }
        while j < b.len() && matches!(b[j], b' ' | b'\t' | b'"' | b'\'' | b'`' | b']') {
            j += 1;
        }
        let rest = &text[j..];
        let Some(separator) = ["=>", ":=", "?=", "=", ":"]
            .iter()
            .find(|s| rest.starts_with(**s) && !rest.starts_with("=="))
        else {
            continue;
        };
        j += separator.len();
        while j < b.len() && matches!(b[j], b' ' | b'\t' | b'"' | b'\'' | b'`') {
            j += 1;
        }
        for scheme in ["bearer ", "basic ", "token "] {
            if text
                .get(j..j + scheme.len())
                .is_some_and(|s| s.eq_ignore_ascii_case(scheme))
            {
                j += scheme.len();
            }
        }
        let value = &text[j..j + run(b, j, is_value, usize::MAX)];
        // A name inside this value is part of it: continue after the value,
        // so the scan stays linear (review H6).
        i = i.max(j + value.len());
        let lower = value.to_ascii_lowercase();
        // No upper bound: a long bearer token or JWT is still a value (H4).
        if value.len() >= 16
            && value.bytes().any(|c| c.is_ascii_alphabetic())
            && value.bytes().any(|c| c.is_ascii_digit())
            && !PLACEHOLDERS.iter().any(|p| lower.contains(p))
            && entropy(value.as_bytes()) >= 3.5
        {
            return true;
        }
    }
    false
}

/// `.ssh` as a whole path part: `~/.ssh`, `$HOME/.ssh`, `-C ~ .ssh`,
/// `Path.home() / '.ssh'` (review H5).
fn ssh_folder(normal: &str) -> bool {
    let b = normal.as_bytes();
    normal.match_indices(".ssh").any(|(at, _)| {
        (at == 0 || !(is_word(b[at - 1]) || b[at - 1] == b'.'))
            && b.get(at + 4).is_none_or(|&c| !is_word(c))
    })
}

/// Named credential files: reading one is the step before exfiltration.
fn credential_file(normal: &str) -> bool {
    ssh_folder(normal)
        || [
            ".ssh/",
            ".ssh\\",
            "id_rsa",
            "id_ed25519",
            "id_ecdsa",
            ".aws/credentials",
            ".aws\\credentials",
            ".aws/config",
            ".netrc",
            ".git-credentials",
        ]
        .iter()
        .any(|needle| normal.contains(needle))
}

pub(super) fn strong_secret_form(text: &str) -> bool {
    known_token_format(text)
        || private_key_block(text)
        || secret_assignment(text)
        || credential_file(&normalize_text(text))
}

fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}
/// A shell name is ASCII: in `$Xé_token` the reference is `$X` (review B2).
fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `$name` that is assigned to or indexed (`$api_token = "…"`, `.=`, `+=`,
/// `||=`, `$h{k}`, `$a['k']`) is a variable being written or a part of one,
/// not a plain reference to a value (review B1, re-check). The old rules
/// apply to it in full.
fn assignment_target(line: &[char], end: usize) -> bool {
    if matches!(line.get(end), Some('[' | '{')) {
        return true;
    }
    let rest = &line[end..];
    let at = rest.iter().take_while(|c| c.is_whitespace()).count();
    let rest = &rest[at..];
    let starts = |op: &str| op.chars().enumerate().all(|(k, c)| rest.get(k) == Some(&c));
    (starts("=") && !starts("==") && !starts("=~"))
        || [
            ".=", "+=", "-=", "*=", "/=", "%=", "|=", "&=", "^=", "x=", "||=", "&&=", "//=", "??=",
            "**=", "<<=", ">>=",
        ]
        .iter()
        .any(|op| starts(op))
}

/// Spans of one line, in chars: the plain references `$NAME` and `${NAME}`
/// (`masked`), and the start of every `${` that opens such a reference or a
/// `${expr}` inside a JavaScript template literal (`approved`).
///
/// A template expression is read narrowly (review A1 and its re-check): only
/// in a JavaScript or TypeScript source file (`templates`), between an
/// opening and a closing backtick on the line, with no backtick between `${`
/// and its `}`, and only a member access or a call, a name followed by `.`
/// or `(` (`${row.id}`, `${esc(x)}`). Markdown inline code and shell
/// backticks are never template literals. A variable declared with Perl's
/// `my`, `our` or `local` is being written, not referenced.
/// One pass with a stack of open braces, so it is linear in the line length.
fn references(
    line: &[char],
    templates: bool,
    names: &environment::SecretNames,
) -> (Vec<(usize, usize)>, Vec<bool>) {
    let mut masked = Vec::new();
    let mut approved = vec![false; line.len()];
    // (position of `{`, opened by a template-shaped `${`, backticks before it)
    let mut open: Vec<(usize, bool, usize, Option<usize>)> = Vec::new();
    // Closed template-shaped `${…}`: approved once a closing backtick follows.
    let mut closed: Vec<(usize, usize)> = Vec::new();
    let mut backticks = 0;
    let mut substitutions = 0;
    // After `my`/`our`/`local`, and inside the list that follows one.
    let (mut declared, mut in_list) = (false, false);
    let mut i = 0;
    while i < line.len() {
        let c = line[i];
        let next = line.get(i + 1).copied();
        if c == '`' || (c == '$' && next == Some('(')) {
            substitutions += 1;
        }
        if c == '$' && next.is_some_and(is_name_start) {
            let end = i
                + 1
                + line[i + 1..]
                    .iter()
                    .take_while(|&&c| is_name_char(c))
                    .count();
            let name = line[i + 1..end].iter().collect::<String>();
            if !(declared || in_list || assignment_target(line, end) || names.holds(&name)) {
                masked.push((i, end));
                approved[i] = true;
            }
            declared = false;
            i = end;
            continue;
        }
        if c.is_ascii_alphabetic() && (i == 0 || !is_name_char(line[i - 1])) {
            let len = line[i..].iter().take_while(|&&c| is_name_char(c)).count();
            let word = line[i..i + len].iter().collect::<String>();
            declared = matches!(word.as_str(), "my" | "our" | "local");
            i += len;
            continue;
        }
        match c {
            '(' if declared => in_list = true,
            ')' | ';' | '=' => in_list = false,
            _ => {}
        }
        if !c.is_whitespace() && c != '(' {
            declared = false;
        }
        if c == '$' && next == Some('{') {
            let name = line[i + 2..]
                .iter()
                .take_while(|&&c| is_name_char(c))
                .count();
            let first = line.get(i + 2).copied();
            let end = i + 2 + name;
            let static_name = first.is_some_and(is_name_start)
                && !names.holds(&line[i + 2..end].iter().collect::<String>());
            if static_name && line.get(end) == Some(&'}') {
                masked.push((i, i + 3 + name));
                approved[i] = true;
                i += 3 + name;
                continue;
            }
            let after = line.get(end).copied();
            let template_shaped = templates
                && name > 0
                && first.is_some_and(is_name_start)
                && matches!(after, Some('.' | '('));
            let with_default = static_name && line.get(end..end + 2) == Some(&[':', '-']);
            if with_default {
                masked.push((i, end + 2));
            }
            open.push((
                i,
                template_shaped,
                backticks,
                with_default.then_some(substitutions),
            ));
            i += 2;
            continue;
        }
        match c {
            '`' => backticks += 1,
            '{' => open.push((i, false, backticks, None)),
            '}' => {
                if let Some((at, shaped, seen, default_substitutions)) = open.pop() {
                    if default_substitutions == Some(substitutions) {
                        approved[at] = true;
                    }
                    // After an opening backtick, none between `${` and `}`.
                    if shaped && seen % 2 == 1 && seen == backticks {
                        closed.push((at, seen));
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    // The literal must close: a backtick after the expression.
    for (at, seen) in closed {
        if backticks > seen {
            approved[at] = true;
        }
    }
    (masked, approved)
}

/// The old secret test without the matches that are a plain variable
/// reference: the `$` clause counts only a `$` that does not start one, and
/// the identifier clause does not read the name inside one. Every other clause
/// runs unchanged on the whole line.
fn old_secret_test_except_references(
    normal: &str,
    templates: bool,
    names: &environment::SecretNames,
) -> bool {
    let normal = environment::mask_static_reads(normal, names);
    let compact = normal
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>();
    let mut outside = String::with_capacity(normal.len());
    let mut unapproved = false;
    for line in normal.split('\n') {
        let chars = line.chars().collect::<Vec<_>>();
        let (masked, approved) = references(&chars, templates, names);
        let mut spans = masked.iter().peekable();
        let mut i = 0;
        while i < chars.len() {
            if let Some(&&(start, end)) = spans.peek()
                && start == i
            {
                spans.next();
                if approved[start] {
                    outside.push(' ');
                    i = end;
                    continue;
                }
            }
            if chars[i] == '$'
                && chars
                    .get(i + 1)
                    .is_some_and(|&c| c.is_ascii_alphabetic() || matches!(c, '_' | '{'))
                && !approved[i]
            {
                unapproved = true;
            }
            outside.push(chars[i]);
            i += 1;
        }
        outside.push('\n');
    }
    unapproved || secret_access_except_references(&normal, &compact, &words(&outside))
}

pub(super) fn dumps_environment(text: &str) -> bool {
    let tokens = shell_tokens(text);
    tokens.iter().enumerate().any(|(i, token)| {
        let name = command_name(token);
        if !matches!(name, "env" | "printenv") {
            return false;
        }
        let command_start = i == 0
            || matches!(
                tokens[i - 1].as_str(),
                ";" | "|" | "&" | "(" | "\n" | "$" | "sudo" | "command"
            );
        command_start
            && (name == "printenv"
                || environment_command_dumps(&tokens[i + 1..command_end(&tokens, i + 1)]))
    })
}

fn environment_command_dumps(args: &[String]) -> bool {
    let mut args = args.iter().map(String::as_str);
    while let Some(arg) = args.next() {
        match arg {
            "-u" | "--unset" | "-C" | "--chdir" => {
                args.next();
            }
            "-S" | "--split-string" => return true,
            ")" | "<" | ">" => return true,
            _ if arg.starts_with('-') || arg.contains('=') => {}
            _ => return command_name(arg) == "printenv",
        }
    }
    true
}

/// Strong secret evidence: the old test (less plain references) or a new
/// pattern. `templates`: the text is JavaScript or TypeScript source.
///
/// When the delete readings cannot expand the text's variables within their
/// budget, the exception is not applied and the old test runs in full: a
/// delete hidden behind plain references stays risky, as it was under the
/// old rules (re-check of review C).
pub(super) fn strong_secret(text: &str, templates: bool) -> bool {
    let normal = normalize_text(text);
    old_secret_test_except_references(&normal, templates, &environment::SecretNames::of(text))
        || strong_secret_form(text)
        || dumps_environment(&normal)
        || (has_variable_reference(&normal) && expand_variables(&normal).1)
}

/// Pointers to where secrets live, without a secret or a credential file.
pub(super) fn secret_location_mention(text: &str) -> bool {
    let normal = normalize_text(text);
    if [".ssh", ".aws/", ".aws\\", ".npmrc"]
        .iter()
        .any(|needle| normal.contains(needle))
    {
        return true;
    }
    let b = normal.as_bytes();
    normal.match_indices(".env").any(|(at, _)| {
        (at == 0 || !is_word(b[at - 1])) && b.get(at + 4).is_none_or(|&c| !is_word(c))
    })
}

// Destructive commands (owner decision 2026-11-20, round 5). A recursive
// forced delete is strong whatever its target; there is no target analysis
// and no exception for /tmp or for variables. Detection reads the line the
// way a shell, PowerShell or cmd could: quotes and escapes removed or taken
// as separators, `${IFS}` and unusual spaces taken as spaces, JSON escapes
// decoded, and a delete found after `;`, `&&`, `|`, `#`, backticks or `$(`.

fn is_unix_delete(name: &str) -> bool {
    name == "rm"
}

/// PowerShell's Remove-Item and its aliases.
fn is_powershell_delete(name: &str) -> bool {
    matches!(
        name,
        "remove-item" | "ri" | "rm" | "rmdir" | "rd" | "del" | "erase"
    )
}

fn is_cmd_delete(name: &str) -> bool {
    matches!(name, "rmdir" | "rd" | "del" | "erase")
}

fn delete_name(token: &str) -> &str {
    let name = command_name(token);
    name.strip_suffix(".exe").unwrap_or(name)
}

/// One reading of `normal` for delete detection.
/// - `split_quotes`: quotes and backslashes separate words (`"rm"-rf`), or
///   they are removed (`r''m`, `\rm`).
/// - `json`: `\n`, `\t`, `\r` and `\u00XX` are JSON escapes.
fn delete_reading(normal: &str, split_quotes: bool, json: bool) -> String {
    let chars = normal.chars().collect::<Vec<_>>();
    let mut out = String::with_capacity(normal.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if json && c == '\\' {
            if matches!(next, Some('n' | 't' | 'r')) {
                out.push(' ');
                i += 2;
                continue;
            }
            if next == Some('u') {
                let hex = chars
                    .get(i + 2..i + 6)
                    .map(|h| h.iter().collect::<String>());
                if let Some(decoded) = hex
                    .and_then(|h| u32::from_str_radix(&h, 16).ok())
                    .and_then(char::from_u32)
                {
                    out.push(decoded);
                    i += 6;
                    continue;
                }
            }
        }
        // A PowerShell backtick or a cmd caret at the end of a line continues
        // the command on the next line when the mark follows a space, or when
        // the next line starts with a parameter (`-Recurse`, `/s`). A Markdown
        // code span ending a line (`rm`) before prose is not a continuation.
        let spaced = i > 0 && matches!(chars[i - 1], ' ' | '\t');
        let parameter_next = || {
            let from = i + if next == Some('\r') { 3 } else { 2 };
            chars
                .get(from..)
                .and_then(|rest| rest.iter().find(|c| !matches!(c, ' ' | '\t')))
                .is_some_and(|c| matches!(c, '-' | '/'))
        };
        if matches!(c, '`' | '^') && (spaced || parameter_next()) {
            let skip = match (next, chars.get(i + 2)) {
                (Some('\n'), _) => 2,
                (Some('\r'), Some('\n')) => 3,
                _ => 0,
            };
            if skip > 0 {
                out.push(' ');
                i += skip;
                continue;
            }
        }
        match c {
            '\'' | '"' | '\\' => {
                if split_quotes {
                    out.push(' ');
                }
                i += 1;
            }
            '$' => {
                let rest = &chars[i + 1..];
                let starts = |word: &str| {
                    word.chars()
                        .enumerate()
                        .all(|(k, w)| rest.get(k) == Some(&w))
                };
                if starts("{ifs") {
                    // `${IFS}`, `${IFS%?}`, `${IFS:0:1}`: a separator.
                    let close = rest.iter().take(32).position(|&r| r == '}');
                    out.push(' ');
                    i += 1 + close.map_or(4, |p| p + 1);
                } else if starts("ifs")
                    && !rest
                        .get(3)
                        .is_some_and(|&r| r.is_alphanumeric() || r == '_')
                {
                    out.push(' ');
                    i += 4;
                } else if rest
                    .first()
                    .is_some_and(|&r| r.is_ascii_digit() || r == '@' || r == '*')
                {
                    // `$9`, `$@`, `$*`: usually empty, used to glue words.
                    i += 2;
                } else {
                    out.push(c);
                    i += 1;
                }
            }
            '\u{2010}'..='\u{2015}' | '\u{2212}' => {
                out.push('-');
                i += 1;
            }
            c if c.is_whitespace() || c.is_control() => {
                out.push(if c == '\n' { '\n' } else { ' ' });
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// `rm` flags in the new readings. A short cluster holds only letters `rm`
/// accepts, so prose such as `—frankly` is not `-f` (review E2); the old
/// reading keeps its broader test.
fn unix_recursive(w: &str) -> bool {
    unix_short(w, 'r') || (w.len() >= 3 && w.starts_with("--") && "--recursive".starts_with(w))
}
fn unix_force(w: &str) -> bool {
    unix_short(w, 'f') || (w.len() >= 3 && w.starts_with("--") && "--force".starts_with(w))
}
fn unix_short(w: &str, flag: char) -> bool {
    w.strip_prefix('-').is_some_and(|cluster| {
        !cluster.is_empty()
            && cluster.chars().all(|c| "dfiprvwx".contains(c))
            && cluster.contains(flag)
    })
}

fn powershell_parameter(a: &str, word: &str, min: usize) -> bool {
    let (name, value) = a.split_once(':').unwrap_or((a, ""));
    name.len() >= min && word.starts_with(name) && value != "$false"
}
fn powershell_recurse(w: &str) -> bool {
    powershell_parameter(w, "-recurse", 2)
}
fn powershell_force(w: &str) -> bool {
    powershell_parameter(w, "-force", 3)
}

fn cmd_switch(w: &str, letter: &str) -> bool {
    w.starts_with('/') && w.split('/').any(|part| part == letter)
}
fn cmd_subdirectories(w: &str) -> bool {
    cmd_switch(w, "s")
}
fn cmd_quiet(w: &str) -> bool {
    cmd_switch(w, "q")
}

fn is_variable_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `normal` with `$name` and `${name}` replaced by the value assigned to the
/// name in the text (`a=rm; $a -rf ~`, `f=-rf; rm $f ~`), or by nothing when
/// no short value is assigned (`r${u}m -rf ~`). `$IFS` is left to the
/// readings, and PowerShell's `$true`, `$false` and `$null` stay as written.
/// Values over 64 characters count as unassigned. The expansion may add at
/// most 16 KiB (review C); the second result is true when that budget runs
/// out and the text cannot be read in full.
fn expand_variables(normal: &str) -> (String, bool) {
    let mut values = std::collections::HashMap::new();
    for word in
        normal.split(|c: char| c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(' | ')'))
    {
        if let Some((name, value)) = word.split_once('=')
            && is_variable_name(name)
        {
            let value = value.trim_matches(|c| matches!(c, '\'' | '"'));
            if value.len() <= 64 && !value.contains('$') {
                values.insert(name, value);
            }
        }
    }
    let chars = normal.char_indices().collect::<Vec<_>>();
    let mut out = String::with_capacity(normal.len());
    let limit = normal.len() + EXPANSION_BUDGET;
    let mut i = 0;
    while i < chars.len() {
        let (at, c) = chars[i];
        if out.len() > limit {
            return (out, true);
        }
        if c == '$' {
            let braced = chars.get(i + 1).is_some_and(|&(_, n)| n == '{');
            let start = i + 1 + usize::from(braced);
            let len = chars[start..]
                .iter()
                .take_while(|&&(_, n)| n.is_ascii_alphanumeric() || n == '_')
                .count();
            let closed = !braced || chars.get(start + len).is_some_and(|&(_, n)| n == '}');
            if len > 0 && closed {
                let from = chars[start].0;
                let to = chars.get(start + len).map_or(normal.len(), |&(b, _)| b);
                let name = &normal[from..to];
                if is_variable_name(name) && !matches!(name, "ifs" | "true" | "false" | "null") {
                    out.push_str(values.get(name).copied().unwrap_or(""));
                    i = start + len + usize::from(braced);
                    continue;
                }
            }
        }
        out.push_str(&normal[at..at + c.len_utf8()]);
        i += 1;
    }
    (out, false)
}

const EXPANSION_BUDGET: usize = 16 * 1024;

/// A recursive forced delete in one reading: a delete name with both flags
/// after it in the same command, however far (an 80-word limit would let
/// padding hide the flags). Linear: the nearest
/// flag after each word is found in one backward pass per family.
fn reading_has_delete(reading: &str) -> bool {
    type Family = (fn(&str) -> bool, fn(&str) -> bool, fn(&str) -> bool);
    const FAMILIES: [Family; 3] = [
        (is_unix_delete, unix_recursive, unix_force),
        (is_powershell_delete, powershell_recurse, powershell_force),
        (is_cmd_delete, cmd_subdirectories, cmd_quiet),
    ];
    reading.split([';', '&', '|', '\n']).any(|command| {
        let mut words = command
            .split(|c: char| {
                c.is_whitespace() || matches!(c, '(' | ')' | '`' | '{' | '}' | ',' | '<' | '>')
            })
            .filter(|w| !w.is_empty());
        // Per family: the first delete name seen, then both flags after it.
        let mut state = [(false, false, false); 3];
        words.any(|word| {
            FAMILIES
                .iter()
                .zip(state.iter_mut())
                .any(|(&(name, first, second), (seen, a, b))| {
                    if *seen {
                        *a |= first(word);
                        *b |= second(word);
                    } else {
                        *seen = name(delete_name(word));
                    }
                    *seen && *a && *b
                })
        })
    })
}

/// The rules-version-2 destructive test, as it was at f40a88c, without its
/// `chmod` arm (approved exception: `chmod 777` is review).
fn old_destructive_test(tokens: &[String]) -> bool {
    for (index, token) in tokens.iter().enumerate() {
        let args = &tokens[index + 1..command_end(tokens, index + 1)];
        match command_name(token) {
            "rm" => {
                let recursive = args.iter().any(|arg| {
                    arg == "--recursive"
                        || (arg.starts_with('-') && !arg.starts_with("--") && arg.contains('r'))
                });
                let force = args.iter().any(|arg| {
                    arg == "--force"
                        || (arg.starts_with('-') && !arg.starts_with("--") && arg.contains('f'))
                });
                if recursive && force {
                    return true;
                }
            }
            "git" if args.iter().take(8).any(|arg| arg == "push") => {
                if args.iter().any(|arg| {
                    arg.starts_with("--force")
                        || (arg.starts_with('-') && !arg.starts_with("--") && arg.contains('f'))
                }) {
                    return true;
                }
            }
            "remove-item"
                if args.iter().any(|arg| arg == "-recurse")
                    && args.iter().any(|arg| arg == "-force") =>
            {
                return true;
            }
            "rmdir" | "rd"
                if args.iter().any(|arg| arg == "/s") && args.iter().any(|arg| arg == "/q") =>
            {
                return true;
            }
            "mkfs" | "mkfs.ext4" | "mkfs.apfs" | "diskpart" => return true,
            "dd" if args.iter().any(|arg| arg.starts_with("of=/dev/")) => return true,
            _ => {}
        }
    }
    false
}

/// Strong destructive evidence in `normal` (text after `normalize_text`): the
/// old token reading, or any of the delete readings.
pub(super) fn has_destructive_command(normal: &str) -> bool {
    old_destructive_test(&shell_tokens(normal)) || has_recursive_forced_delete(normal)
}

pub(super) fn has_recursive_forced_delete(normal: &str) -> bool {
    // Past the expansion budget the expanded reading is not used; the
    // secrets rule then applies the old test in full (`strong_secret`).
    let (expanded, overflow) = expand_variables(normal);
    let mut texts = vec![normal];
    if !overflow && expanded != normal {
        texts.push(&expanded);
    }
    texts.iter().any(|text| {
        // A reading that cannot differ from another is not repeated.
        let quotes = text.contains(['\'', '"', '\\']);
        let escapes = text.contains('\\');
        [(true, false), (false, false), (true, true), (false, true)]
            .iter()
            .filter(|&&(split, json)| (quotes || !split) && (escapes || !json))
            .any(|&(split, json)| reading_has_delete(&delete_reading(text, split, json)))
    })
}
