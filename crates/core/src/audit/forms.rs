//! The names and access forms behind a secrets finding, for display (owner,
//! 2026-11-24, r5 item 4a): an environment variable's name and the form that
//! reads it (`$CODEX_HOME`, `os.environ.get("X")`) are shown; a literal
//! value never is. Text only, one pass over the line; nothing is executed.
use super::{environment, normalize_text, precision};

/// Shown in place of a literal secret value or a private key: never the value.
pub(super) const LITERAL_SECRET: &str = "literal secret value (hidden)";
pub(super) const PRIVATE_KEY: &str = "private key block (hidden)";
const LIMIT: usize = 8;

fn start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn part(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// The identifier at `at`, if one starts there.
fn identifier(b: &[u8], at: usize) -> Option<&str> {
    if !b.get(at).copied().is_some_and(start) {
        return None;
    }
    let end = at + b[at..].iter().take_while(|&&c| part(c)).count();
    std::str::from_utf8(&b[at..end]).ok()
}

/// A name that may be shown: an ASCII identifier of at most 64 characters
/// that does not itself look like a secret value.
fn shown(name: &str) -> bool {
    let digits = name.bytes().filter(u8::is_ascii_digit).count();
    !name.is_empty()
        && name.len() <= 64
        && !(name.len() > 32 && digits >= 4)
        && !precision::strong_secret_form(name)
        && !precision::known_token_format(name)
}

fn spaces(b: &[u8], mut at: usize) -> usize {
    while matches!(b.get(at), Some(b' ' | b'\t')) {
        at += 1;
    }
    at
}

/// `"NAME"` or `'NAME'` at `at` (after spaces), followed by `close` or `,`.
fn quoted(b: &[u8], at: usize) -> Option<&str> {
    let at = spaces(b, at);
    let quote = *b.get(at)?;
    if !matches!(quote, b'"' | b'\'') {
        return None;
    }
    let name = identifier(b, at + 1)?;
    (b.get(at + 1 + name.len()) == Some(&quote) && shown(name)).then_some(name)
}

fn boundary(b: &[u8], at: usize) -> bool {
    at == 0 || !(part(b[at - 1]) || b[at - 1] == b'.')
}

fn starts(b: &[u8], at: usize, needle: &str) -> bool {
    b.len() >= at + needle.len() && b[at..at + needle.len()].eq_ignore_ascii_case(needle.as_bytes())
}

/// `prefix` followed by `(` with a quoted name: `prefix("NAME")`, or
/// `prefix(…)` for a name computed at run time.
fn call(b: &[u8], after: usize, shown_prefix: &str) -> Option<String> {
    let open = spaces(b, after);
    (b.get(open) == Some(&b'(')).then(|| match quoted(b, open + 1) {
        Some(name) => format!("{shown_prefix}(\"{name}\")"),
        None => format!("{shown_prefix}(…)"),
    })
}

/// How much of a finding's text is read for its forms: the forms are for
/// display, and every scan here stays linear in this bound (owner,
/// 2026-11-20, round 6: every audit check is linear).
const SCAN_LIMIT: usize = 16 * 1024;
/// Names remembered to avoid listing a name twice; bounded like the forms.
const REMEMBERED: usize = 64;

/// The collected forms, unique, at most [`LIMIT`], each at most 80 characters.
struct Forms {
    list: Vec<String>,
    seen: std::collections::HashSet<String>,
}

impl Forms {
    fn full(&self) -> bool {
        self.list.len() >= LIMIT
    }
    fn push(&mut self, form: String) {
        if !self.full() && form.chars().count() <= 80 && self.seen.insert(form.clone()) {
            self.list.push(form);
        }
    }
}

/// The value spans of assignments to a name that holds a secret
/// (`PASSWORD="…"`, `api_key: …`, `Authorization: …`): a `$` inside one is
/// part of a value, never shown (review of 3d3f2ed).
fn secret_values(b: &[u8]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut at = 0;
    while at < b.len() {
        if !start(b[at]) || (at > 0 && part(b[at - 1])) {
            at += 1;
            continue;
        }
        let end = at + b[at..].iter().take_while(|&&c| part(c)).count();
        let name = std::str::from_utf8(&b[at..end]).unwrap_or_default();
        let lower = name.to_ascii_lowercase();
        let op = spaces(b, end);
        let assigned = match b.get(op) {
            Some(b'=') => b.get(op + 1) != Some(&b'='),
            Some(b':') => b.get(op + 1) != Some(&b':') && (op == 0 || b[op - 1] != b':'),
            _ => false,
        };
        if assigned
            && (environment::secret_name(name) || lower == "token" || lower == "authorization")
        {
            let value = spaces(b, op + 1);
            let close = match b.get(value) {
                Some(&q @ (b'"' | b'\'' | b'`')) => b[value + 1..]
                    .iter()
                    .position(|&c| c == q)
                    .map_or(b.len(), |n| value + 2 + n),
                _ => {
                    value
                        + b[value..]
                            .iter()
                            .take_while(|&&c| !c.is_ascii_whitespace() && c != b';')
                            .count()
                }
            };
            spans.push((value, close));
            at = close.max(end);
            continue;
        }
        at = end;
    }
    spans
}

/// The environment reads, references and secret-bearing names on `text`.
/// `sensitive`: the file is a credential file (`.env`, `.ssh/…`); then, and
/// for a line that holds a literal secret, only the assigned names and the
/// fixed markers are given, never anything read from a value.
pub(super) fn secret_forms(text: &str, sensitive: bool) -> Vec<String> {
    // A long text is read up to the last space before the bound, so no name
    // is cut in two.
    let text = if text.len() > SCAN_LIMIT {
        let bytes = &text.as_bytes()[..SCAN_LIMIT];
        let cut = bytes
            .iter()
            .rposition(u8::is_ascii_whitespace)
            .unwrap_or_else(|| (0..=SCAN_LIMIT).rev().find(|&n| text.is_char_boundary(n)).unwrap_or(0));
        &text[..cut]
    } else {
        text
    };
    let b = text.as_bytes();
    let mut forms = Forms {
        list: Vec::new(),
        seen: std::collections::HashSet::new(),
    };
    let mut referenced = std::collections::HashSet::new();
    let literal = precision::strong_secret_form(text);
    let values_only = sensitive || literal;
    if !values_only {
        let values = secret_values(b);
        let mut next_value = 0;
        let mut single = false;
        let mut double = false;
        let mut i = 0;
        while i < b.len() && !forms.full() {
            let c = b[i];
            while next_value < values.len() && values[next_value].1 <= i {
                next_value += 1;
            }
            let in_value = values
                .get(next_value)
                .is_some_and(|(from, to)| *from <= i && i < *to);
            match c {
                b'\'' if !double => single = !single,
                b'"' if !single => double = !double,
                b'\n' => {
                    single = false;
                    double = false;
                }
                _ => {}
            }
            if c == b'$' && !single && !in_value {
                if starts(b, i, "$env:") {
                    if let Some(name) = identifier(b, i + 5).filter(|n| shown(n)) {
                        forms.push(format!("$env:{name}"));
                        if referenced.len() < REMEMBERED {
                            referenced.insert(name.to_owned());
                        }
                        i += 5 + name.len();
                        continue;
                    }
                } else if b.get(i + 1) == Some(&b'{') {
                    let at = i + 2 + usize::from(matches!(b.get(i + 2), Some(b'#' | b'!')));
                    // A positional parameter: `${1:-default}` reads argument 1.
                    let digits = b[at.min(b.len())..]
                        .iter()
                        .take(4)
                        .take_while(|c| c.is_ascii_digit())
                        .count();
                    if (1..=3).contains(&digits) {
                        let number = std::str::from_utf8(&b[at..at + digits]).unwrap_or_default();
                        forms.push(format!("${{{number}}}"));
                        i = at + digits;
                        continue;
                    }
                    if let Some(name) = identifier(b, at).filter(|n| shown(n)) {
                        let after = b.get(at + name.len()).copied();
                        let form = if after == Some(b'[') {
                            Some(format!("${{{name}[…]}}"))
                        } else if after.is_some_and(|c| {
                            matches!(
                                c,
                                b'}' | b':'
                                    | b'-'
                                    | b'='
                                    | b'?'
                                    | b'+'
                                    | b'%'
                                    | b'#'
                                    | b'/'
                                    | b'^'
                                    | b','
                                    | b'@'
                            )
                        }) {
                            Some(format!("${{{name}}}"))
                        } else {
                            None
                        };
                        if let Some(form) = form {
                            forms.push(form);
                            if referenced.len() < REMEMBERED {
                                referenced.insert(name.to_owned());
                            }
                            i = at + name.len();
                            continue;
                        }
                    }
                } else if let Some(name) = identifier(b, i + 1).filter(|n| shown(n)) {
                    forms.push(format!("${name}"));
                    if referenced.len() < REMEMBERED {
                        referenced.insert(name.to_owned());
                    }
                    i += 1 + name.len();
                    continue;
                }
            } else if boundary(b, i) && starts(b, i, "process.env") {
                let after = i + "process.env".len();
                let form = match b.get(after) {
                    Some(b'.') => identifier(b, after + 1)
                        .filter(|n| shown(n))
                        .map(|name| format!("process.env.{name}")),
                    Some(b'[') => Some(match quoted(b, after + 1) {
                        Some(name) => format!("process.env[\"{name}\"]"),
                        None => "process.env[…]".into(),
                    }),
                    Some(c) if part(*c) => None,
                    _ => Some("process.env".into()),
                };
                if let Some(form) = form {
                    forms.push(form);
                }
                i = after;
                continue;
            } else if boundary(b, i) && starts(b, i, "os.environ") {
                let after = i + "os.environ".len();
                let mut form = Some("os.environ".to_owned());
                for method in [".get", ".setdefault", ".pop"] {
                    if starts(b, after, method)
                        && !b.get(after + method.len()).is_some_and(|c| part(*c))
                    {
                        form = call(b, after + method.len(), &format!("os.environ{method}"));
                    }
                }
                if b.get(after) == Some(&b'[') {
                    form = Some(match quoted(b, after + 1) {
                        Some(name) => format!("os.environ[\"{name}\"]"),
                        None => "os.environ[…]".into(),
                    });
                } else if b.get(after).is_some_and(|c| part(*c)) {
                    form = None;
                }
                if let Some(form) = form {
                    forms.push(form);
                }
                i = after;
                continue;
            } else if (i == 0 || !part(b[i - 1])) && starts(b, i, "getenv") {
                let prefix = if i >= 3 && &b[i - 3..i] == b"os." {
                    "os.getenv"
                } else {
                    "getenv"
                };
                if let Some(form) = call(b, i + "getenv".len(), prefix) {
                    forms.push(form);
                }
            } else if boundary(b, i) && starts(b, i, "env::var") {
                let rest = i + "env::var".len();
                let (suffix, after) = if starts(b, rest, "s_os") {
                    ("s_os", rest + 4)
                } else if starts(b, rest, "s") {
                    ("s", rest + 1)
                } else if starts(b, rest, "_os") {
                    ("_os", rest + 3)
                } else {
                    ("", rest)
                };
                if suffix.starts_with('s') {
                    forms.push(format!("env::var{suffix}()"));
                } else if let Some(form) = call(b, after, &format!("env::var{suffix}")) {
                    forms.push(form);
                }
                i = after;
                continue;
            } else if starts(b, i, "getenvironmentvariable") {
                if let Some(form) = call(
                    b,
                    i + "getenvironmentvariable".len(),
                    "GetEnvironmentVariable",
                ) {
                    forms.push(form);
                }
            } else if (i == 0 || !part(b[i - 1]))
                && starts(b, i, "printenv")
                && !b.get(i + 8).is_some_and(|c| part(*c))
            {
                let at = spaces(b, i + 8);
                match identifier(b, at).filter(|n| at > i + 8 && shown(n)) {
                    Some(name) => forms.push(format!("printenv {name}")),
                    None => forms.push("printenv".into()),
                }
            }
            i += 1;
        }
        // A name already shown inside a read (`process.env.GITHUB_TOKEN`,
        // `os.environ["API_TOKEN"]`) is not listed again on its own.
        for form in &forms.list {
            let inner = form
                .trim_end_matches(['"', ')', ']'])
                .rsplit(['.', '"', '(', '[', ' ', ':'])
                .next()
                .unwrap_or_default();
            if !inner.is_empty() && inner.bytes().all(part) && referenced.len() < REMEMBERED {
                referenced.insert(inner.to_owned());
            }
        }
        if !forms.full()
            && precision::dumps_environment(&normalize_text(text))
            && !forms.list.iter().any(|f| f.starts_with("printenv"))
        {
            forms.push("env".into());
        }
    }
    // Names that hold a secret by their name (`GITHUB_TOKEN`, `aws_*`) or are
    // assigned or sent as one; the value after them is never shown.
    let mut at = 0;
    while at < b.len() && !forms.full() {
        if !start(b[at]) || (at > 0 && part(b[at - 1])) {
            at += 1;
            continue;
        }
        let next = at + b[at..].iter().take_while(|&&c| part(c)).count();
        let name = std::str::from_utf8(&b[at..next]).unwrap_or_default();
        // A word in a value's place (`key = "NAME"`) is that value.
        let mut before = at;
        if before > 0 && matches!(b[before - 1], b'"' | b'\'') {
            before -= 1;
        }
        while before > 0 && matches!(b[before - 1], b' ' | b'\t') {
            before -= 1;
        }
        if before > 0
            && matches!(b[before - 1], b'=' | b':')
            && (before < 2 || b[before - 2] != b':')
        {
            at = next;
            continue;
        }
        let value = spaces(b, next);
        let assigned = match b.get(value) {
            Some(b'=') => b.get(value + 1) != Some(&b'='),
            Some(b':') => {
                b.get(value + 1) != Some(&b':') && (value == 0 || b[value - 1] != b':')
            }
            _ => false,
        };
        let lower = name.to_ascii_lowercase();
        let token_name =
            lower.starts_with("aws_") || (lower.contains('_') && lower.split('_').any(|p| p == "token"));
        let secret_key = assigned && (environment::secret_name(name) || lower == "token");
        if lower == "authorization" && assigned {
            forms.push(format!("{name}: …"));
        } else if (secret_key || (token_name && !values_only))
            && !referenced.contains(name)
            && shown(name)
        {
            if secret_key {
                forms.push(format!("{name} = …"));
            } else {
                forms.push(name.to_owned());
            }
        }
        at = next;
    }
    for needle in [
        ".aws/credentials",
        ".aws/config",
        ".git-credentials",
        ".netrc",
        ".npmrc",
        ".ssh",
        "id_rsa",
        "id_ed25519",
        "id_ecdsa",
    ] {
        if text.contains(needle) {
            forms.push(needle.into());
        }
    }
    if !forms.list.iter().any(|f| f.starts_with(".aws/"))
        && (text.contains(".aws/") || text.contains(".aws\\"))
    {
        forms.push(".aws/".into());
    }
    let lower = text.to_ascii_lowercase();
    if lower.match_indices(".env").any(|(at, _)| {
        (at == 0 || !part(lower.as_bytes()[at - 1]))
            && lower.as_bytes().get(at + 4).is_none_or(|c| !part(*c))
    }) {
        forms.push(".env".into());
    }
    // The words of a weak finding (owner's examples: credentials, passwords).
    for word in ["credentials", "passwords", "password", "environment variables"] {
        if lower.contains(word)
            && !forms
                .list
                .iter()
                .any(|f| f.to_ascii_lowercase().contains(word))
        {
            forms.push(word.into());
        }
    }
    // The markers come last but are never crowded out.
    let mut list = forms.list;
    let mut marker = |m: &str| {
        if !list.iter().any(|f| f == m) {
            if list.len() >= LIMIT {
                list.pop();
            }
            list.push(m.to_owned());
        }
    };
    if precision::private_key_block(text) || precision::private_key_header(text) {
        marker(PRIVATE_KEY);
    }
    if precision::known_token_format(text) || precision::secret_assignment(text) {
        marker(LITERAL_SECRET);
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forms(text: &str) -> Vec<String> {
        secret_forms(text, false)
    }

    #[test]
    fn names_and_forms_are_shown_and_values_never() {
        assert_eq!(
            forms(r#"CODEX_HOME="${CODEX_HOME:-$HOME/.codex}""#),
            ["${CODEX_HOME}", "$HOME"]
        );
        assert_eq!(forms("echo $CODEX_HOME"), ["$CODEX_HOME"]);
        assert_eq!(
            forms(r#"home = os.environ.get("CODEX_HOME", "~/.codex")"#),
            [r#"os.environ.get("CODEX_HOME")"#]
        );
        assert_eq!(
            forms("key = os.environ.get(name)"),
            ["os.environ.get(…)", "key = …"]
        );
        assert_eq!(forms(r#"os.environ["API_KEY"]"#), [r#"os.environ["API_KEY"]"#]);
        assert_eq!(forms("env = dict(os.environ)"), ["os.environ"]);
        assert_eq!(forms("const t = process.env.GITHUB_TOKEN;"), ["process.env.GITHUB_TOKEN"]);
        assert_eq!(forms("spawn(cmd, {env: process.env})"), ["process.env"]);
        assert_eq!(forms("process.env[key]"), ["process.env[…]"]);
        assert_eq!(forms(r#"os.getenv("HOME")"#), [r#"os.getenv("HOME")"#]);
        assert_eq!(forms(r#"std::env::var("TERM")"#), [r#"env::var("TERM")"#]);
        assert_eq!(forms("for (k, v) in std::env::vars() {"), ["env::vars()"]);
        assert_eq!(forms("$env:USERPROFILE"), ["$env:USERPROFILE"]);
        assert_eq!(forms("printenv | sort"), ["printenv"]);
        assert_eq!(forms("printenv HOME"), ["printenv HOME"]);
        assert_eq!(forms("env > all.txt"), ["env"]);
        assert_eq!(forms("export GITHUB_TOKEN"), ["GITHUB_TOKEN"]);
        assert_eq!(forms("echo $GITHUB_TOKEN"), ["$GITHUB_TOKEN"], "named once");
        assert_eq!(forms("cat ~/.ssh/id_rsa"), [".ssh", "id_rsa"]);
        assert_eq!(forms("source .env"), [".env"]);
        assert_eq!(forms("cp .aws/credentials x"), [".aws/credentials"]);
        assert_eq!(forms(r#"MINUTES="${1:-60}""#), ["${1}"]);
        assert_eq!(forms(r#"cd "$(dirname "${BASH_SOURCE[0]}")""#), ["${BASH_SOURCE[…]}"]);
        assert_eq!(forms("log('codex_core::session::turn')"), Vec::<String>::new());
        assert_eq!(forms("Do not invent credentials."), ["credentials"]);
    }

    #[test]
    fn literal_values_are_replaced_by_their_kind() {
        let token = format!("ghp_{}", "aB3dE5fG7hJ9kL1mN3pQ5rS7tU9vW1xY3z5A");
        let shown = forms(&format!("GITHUB_TOKEN={token}")).join(" ");
        assert!(!shown.contains(&token), "{shown}");
        assert!(shown.contains("GITHUB_TOKEN = …"), "{shown}");
        assert!(shown.contains(LITERAL_SECRET), "{shown}");
        // Publish review (2026-11-25): values a secret scanner takes for a key
        // are joined at run time, so this file holds no such literal.
        let value = ["Zx81kQ0pLm4Rt7Vb", "2Nc6Hy9Wd3Fg5Js1"].concat();
        let shown = forms(&format!(r#"api_key = "{value}""#)).join(" ");
        assert!(!shown.contains(&value), "{shown}");
        assert!(shown.contains("api_key = …"), "{shown}");
        let shown = forms("Authorization: Bearer abc.def").join(" ");
        assert_eq!(shown, "Authorization: …");
        // A name in a value's place is a value.
        assert_eq!(
            forms(r#"bearer_token_env_var = "GITHUB_PAT_TOKEN""#),
            ["bearer_token_env_var = …"]
        );
        assert_eq!(
            forms(&format!("-----BEGIN {}PRIVATE KEY-----", "RSA ")),
            [PRIVATE_KEY]
        );
        // A "name" that is itself a token is never shown.
        assert!(forms(&format!("echo ${token}")).iter().all(|f| !f.contains(&token)));
        // Review of 3d3f2ed: a `$` inside a value is part of that value.
        let shown = forms("PASSWORD='p@ss$wordXYZ'").join(" ");
        assert!(!shown.contains("wordXYZ"), "{shown}");
        assert!(shown.contains("PASSWORD = …"), "{shown}");
        let shown = forms(r#"db_password="abc$defGHI" ./run"#).join(" ");
        assert!(!shown.contains("defGHI"), "{shown}");
        assert_eq!(forms("echo 'costs $HOME dollars'"), Vec::<String>::new());
        // A credential file gives the assigned names only.
        assert_eq!(secret_forms("API_KEY=abc$def\nHOME_DIR=$HOME", true), ["API_KEY = …"]);
    }

    /// Review of 3d3f2ed: linear in the text, and bounded (owner, round 6).
    #[test]
    fn many_references_stay_bounded_and_fast() {
        let same = "$HOME ".repeat(200_000);
        let distinct = (0..100_000).map(|n| format!("$v{n} ")).collect::<String>();
        let names = (0..100_000).map(|n| format!("a_token_{n} ")).collect::<String>();
        for text in [&same, &distinct, &names] {
            let started = std::time::Instant::now();
            let found = forms(text);
            assert!(found.len() <= LIMIT, "{found:?}");
            assert!(started.elapsed() < std::time::Duration::from_secs(2), "{:?}", started.elapsed());
        }
        assert_eq!(forms(&same), ["$HOME"]);
        assert_eq!(forms(&distinct).len(), LIMIT);
    }
}
