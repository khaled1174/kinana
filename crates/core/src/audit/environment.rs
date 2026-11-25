//! Recognize the narrow static environment reads that do not name secrets.
use std::{collections::HashSet, path::Path};

/// Whole parts that name a secret (owner, 2026-11-24, r7 item 4).
const SECRET_PARTS: [&str; 11] = [
    "key",
    "token",
    "secret",
    "password",
    "passwd",
    "auth",
    "credential",
    "private",
    "session",
    "cookie",
    "pat",
];

/// Words that name a secret anywhere in a name, without splitting it.
const SECRET_INSIDE: [&str; 8] = [
    "secret",
    "token",
    "password",
    "passwd",
    "credential",
    "apikey",
    "privatekey",
    "accesskey",
];

/// The camelCase pieces of one part: `apiKey` → `api`, `Key`; `APIKey` →
/// `API`, `Key`. A part with no case boundary is one piece.
fn camel_pieces(part: &str) -> Vec<&str> {
    let b = part.as_bytes();
    let mut pieces = Vec::new();
    let mut from = 0;
    for at in 1..b.len() {
        let lower_upper = b[at - 1].is_ascii_lowercase() && b[at].is_ascii_uppercase();
        let acronym_end = b[at - 1].is_ascii_uppercase()
            && b[at].is_ascii_uppercase()
            && b.get(at + 1).is_some_and(u8::is_ascii_lowercase);
        if lower_upper || acronym_end {
            pieces.push(&part[from..at]);
            from = at;
        }
    }
    pieces.push(&part[from..]);
    pieces
}

/// A name that holds a secret, for the plain-reference exception and the
/// static environment reads (owner, 2026-11-24, r7 item 4: the r3 split on
/// `_` alone missed `apiKey`, `GITHUBPAT` and `x-api-key`). The name is
/// split on `_`, `-`, digits and camelCase boundaries, and a whole part is
/// one of [`SECRET_PARTS`]; or the whole name contains one of
/// [`SECRET_INSIDE`]. Two additions keep it fail-closed:
/// - a part that ends in `pat` (`GITHUBPAT`, one of the owner's names that
///   must stay risky, has no boundary to split on); proposed to the owner in
///   docs/0.3-r7-proposals/01-secret-name-spec.md;
/// - a part between separators is also matched whole before its camelCase
///   split, so a mixed-case `kEy` stays a secret as it was before r7.
pub(super) fn secret_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if SECRET_INSIDE.iter().any(|word| lower.contains(word)) {
        return true;
    }
    let whole = |piece: &str| {
        SECRET_PARTS
            .iter()
            .any(|word| piece.eq_ignore_ascii_case(word))
    };
    name.split(|c: char| c == '_' || c == '-' || c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .any(|part| {
            whole(part)
                || camel_pieces(part)
                    .into_iter()
                    .any(|piece| whole(piece) || piece.to_ascii_lowercase().ends_with("pat"))
        })
}

/// The names that hold a secret in one text, read with their own case and
/// kept lowercased. The old secret test and its exceptions read lowercased
/// text (`normalize_text`), where `stageKey` (a secret: `stage`, `Key`) and
/// `stagekey` (not one) look the same; a name these exceptions read there is
/// a secret if [`secret_name`] says so or the set holds it.
pub(super) struct SecretNames(HashSet<String>);

impl SecretNames {
    pub(super) fn of(text: &str) -> Self {
        let kept = super::normalize_keep_case(text);
        Self(
            kept.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .filter(|name| !name.is_empty() && secret_name(name))
                .map(str::to_ascii_lowercase)
                .collect(),
        )
    }

    /// `name` as it appears in lowercased text.
    pub(super) fn holds(&self, name: &str) -> bool {
        secret_name(name) || self.0.contains(name)
    }
}

fn identifier(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn name_end(bytes: &[u8], start: usize) -> Option<usize> {
    let first = *bytes.get(start)?;
    if !first.is_ascii_alphabetic() && first != b'_' {
        return None;
    }
    Some(
        start
            + bytes[start..]
                .iter()
                .take_while(|&&b| identifier(b))
                .count(),
    )
}

fn horizontal_space(bytes: &[u8], mut at: usize) -> usize {
    while matches!(bytes.get(at), Some(b' ' | b'\t')) {
        at += 1;
    }
    at
}

fn quoted_name(bytes: &[u8], start: usize, close: u8) -> Option<(usize, usize, usize)> {
    let quote = *bytes.get(start)?;
    if !matches!(quote, b'\'' | b'"') {
        return None;
    }
    let name = start + 1;
    let end = name_end(bytes, name)?;
    if bytes.get(end) != Some(&quote) {
        return None;
    }
    let finish = horizontal_space(bytes, end + 1);
    (bytes.get(finish) == Some(&close)).then_some((name, end, finish + 1))
}

fn static_read(bytes: &[u8], at: usize) -> Option<(usize, usize, usize)> {
    let rest = &bytes[at..];
    if rest.starts_with(b"process.env.") {
        let start = at + b"process.env.".len();
        let end = name_end(bytes, start)?;
        if bytes
            .get(end)
            .is_some_and(|b| !b.is_ascii() || matches!(b, b'\\' | b'$'))
        {
            return None;
        }
        return Some((start, end, end));
    }
    for (prefix, open, close) in [
        (b"os.environ.get".as_slice(), b'(', b')'),
        (b"os.environ".as_slice(), b'[', b']'),
        (b"getenv".as_slice(), b'(', b')'),
    ] {
        if rest.starts_with(prefix) {
            let open_at = horizontal_space(bytes, at + prefix.len());
            if bytes.get(open_at) == Some(&open) {
                return quoted_name(bytes, horizontal_space(bytes, open_at + 1), close);
            }
        }
    }
    None
}

fn looks_like_write(bytes: &[u8], end: usize) -> bool {
    let rest = &bytes[end..];
    let rest = &rest[rest
        .iter()
        .take_while(|b| b.is_ascii_whitespace() || **b == b')')
        .count()..];
    rest.starts_with(b":")
        || [b"of".as_slice(), b"in"].iter().any(|prefix| {
            rest.strip_prefix(*prefix)
                .is_some_and(|tail| tail.first().is_none_or(|b| !identifier(*b)))
        })
        || (rest.starts_with(b"=") && !rest.starts_with(b"=="))
        || [
            "++", "--", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "**=", "<<=", ">>=", "&&=",
            "||=", "??=",
        ]
        .iter()
        .any(|op| rest.starts_with(op.as_bytes()))
}

fn comments_as_space(bytes: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    if !bytes.windows(2).any(|pair| pair == b"//" || pair == b"/*") {
        return std::borrow::Cow::Borrowed(bytes);
    }
    let mut view = bytes.to_vec();
    let mut at = 0;
    while at < bytes.len() {
        if matches!(bytes[at], b'\'' | b'"') {
            at = quoted_end(bytes, at, bytes[at]);
            continue;
        }
        let start = at;
        if bytes[at..].starts_with(b"//") {
            at += bytes[at..]
                .iter()
                .position(|b| *b == b'\n')
                .unwrap_or(bytes.len() - at);
        } else if bytes[at..].starts_with(b"/*") {
            at += 2;
            at += bytes[at..]
                .windows(2)
                .position(|b| b == b"*/")
                .map_or(bytes.len() - at, |n| n + 2);
        } else {
            at += 1;
            continue;
        }
        for b in &mut view[start..at] {
            if !b.is_ascii_whitespace() {
                *b = b' ';
            }
        }
    }
    std::borrow::Cow::Owned(view)
}

fn has_write_prefix(bytes: &[u8], at: usize) -> bool {
    let prefix = &bytes[..at];
    let tail = prefix
        .iter()
        .rev()
        .take_while(|b| b.is_ascii_whitespace() || **b == b'(')
        .count();
    let prefix = &prefix[..prefix.len() - tail];
    if prefix.ends_with(b"++") || prefix.ends_with(b"--") {
        return true;
    }
    let word = prefix.iter().rev().take_while(|&&b| identifier(b)).count();
    matches!(&prefix[prefix.len() - word..], b"delete" | b"del")
}

fn destructuring_targets(bytes: &[u8]) -> Vec<(usize, usize)> {
    let mut opens = Vec::new();
    let mut targets = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'\'' | b'"' => {
                at = quoted_end(bytes, at, bytes[at]);
                continue;
            }
            b'[' | b'{' => opens.push((at, bytes[at])),
            b']' | b'}' => {
                if let Some((start, open)) = opens.pop()
                    && matches!((open, bytes[at]), (b'[', b']') | (b'{', b'}'))
                    && looks_like_write(bytes, at + 1)
                {
                    targets.push((start, at));
                }
            }
            _ => {}
        }
        at += 1;
    }
    targets.sort_unstable();
    targets
}

/// Mask only matched legacy evidence, never text used by literal-secret rules.
pub(super) fn mask_static_reads(normal: &str, names: &SecretNames) -> String {
    let bytes = normal.as_bytes();
    let mut view = bytes.to_vec();
    if !normal.contains("process.env")
        && !normal.contains("os.environ")
        && !normal.contains("getenv")
    {
        return normal.into();
    }
    let write_view = comments_as_space(bytes);
    let targets = destructuring_targets(&write_view);
    let mut targets = targets.into_iter().peekable();
    let mut target_end = 0;
    let mut at = 0;
    while at < bytes.len() {
        while targets.peek().is_some_and(|(start, _)| *start <= at) {
            target_end = target_end.max(targets.next().expect("peeked target").1);
        }
        if (at == 0 || !identifier(bytes[at - 1]))
            && at >= target_end
            && let Some((name, end, finish)) = static_read(bytes, at)
            && !names.holds(&normal[name..end])
            && !looks_like_write(&write_view, finish)
            && !has_write_prefix(&write_view, at)
        {
            view[at..finish].fill(b' ');
            at = finish;
        } else {
            at += 1;
        }
    }
    String::from_utf8(view).expect("only ASCII spans are masked")
}

pub(super) fn is_ignore_file(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == ".gitignore")
}

pub(super) fn mask_ignored_env_paths(text: &str) -> String {
    let mut view = text.as_bytes().to_vec();
    for (at, token) in text.as_bytes().windows(4).enumerate() {
        if token.eq_ignore_ascii_case(b".env")
            && (at == 0
                || matches!(
                    text.as_bytes()[at - 1],
                    b'/' | b'\\' | b'!' | b' ' | b'\t' | b'\n'
                ))
            && text
                .as_bytes()
                .get(at + 4)
                .is_none_or(|b| matches!(b, b'.' | b'/' | b'\\' | b' ' | b'\t' | b'\n' | b'\r'))
        {
            view[at..at + 4].fill(b' ');
        }
    }
    String::from_utf8(view).expect("only ASCII spans are masked")
}

pub(super) fn supports_multiline_templates(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "js" | "mjs" | "cjs" | "ts" | "tsx"
            )
        })
}

enum Context {
    Code {
        expression: Option<usize>,
        braces: usize,
        nested_template: bool,
        jsx_expression: bool,
    },
    Template {
        approved: Vec<usize>,
    },
    Jsx {
        depth: usize,
        tag: Option<bool>,
    },
}

fn quoted_end(bytes: &[u8], start: usize, delimiter: u8) -> usize {
    let mut at = start + 1;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' => at += 2,
            b'\n' => break,
            b if b == delimiter => return at + 1,
            _ => at += 1,
        }
    }
    at
}

fn regex_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut at = start + 1;
    let mut character_class = false;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' => at += 2,
            b'\n' => break,
            b'[' => {
                character_class = true;
                at += 1;
            }
            b']' => {
                character_class = false;
                at += 1;
            }
            b'/' if !character_class => return Some(at + 1),
            // Ambiguous division may have crossed a true template opener.
            // Stop approving later spans rather than re-entering code there.
            b'`' => return None,
            _ => at += 1,
        }
    }
    Some(at)
}

fn approved_expression(text: &str, marker: usize, close: usize) -> bool {
    let bytes = text.as_bytes();
    let start = marker + 2;
    name_end(bytes, start).is_some_and(|end| {
        end < close
            && matches!(bytes[end], b'.' | b'(')
            && !super::words(&text[start..close])
                .iter()
                .any(|name| secret_name(name))
    })
}

/// Keep the old member-access/call exception within closed JS template
/// literals, including ones spanning lines. Only the opening dollar is
/// masked; the expression is still checked for other risk signals.
pub(super) fn mask_templates(text: &str, tsx: bool) -> String {
    let bytes = text.as_bytes();
    let mut view = bytes.to_vec();
    let mut stack = vec![Context::Code {
        expression: None,
        braces: 0,
        nested_template: false,
        jsx_expression: false,
    }];
    let mut at = 0;
    while at < bytes.len() {
        match stack.last_mut().expect("top-level code context stays") {
            Context::Template { approved } => match bytes[at] {
                b'\\' => at += 2,
                b'`' => {
                    for marker in std::mem::take(approved) {
                        view[marker] = b' ';
                    }
                    stack.pop();
                    at += 1;
                }
                b'$' if bytes.get(at + 1) == Some(&b'{') => {
                    stack.push(Context::Code {
                        expression: Some(at),
                        braces: 0,
                        nested_template: false,
                        jsx_expression: false,
                    });
                    at += 2;
                }
                _ => at += 1,
            },
            Context::Jsx { depth, tag } => match bytes[at] {
                b'{' => {
                    stack.push(Context::Code {
                        expression: None,
                        braces: 0,
                        nested_template: false,
                        jsx_expression: true,
                    });
                    at += 1;
                }
                b'<' if tag.is_none() => {
                    *tag = Some(bytes.get(at + 1) == Some(&b'/'));
                    at += 1;
                }
                b'\'' | b'"' if tag.is_some() => {
                    let delimiter = bytes[at];
                    at += 1;
                    at += bytes[at..]
                        .iter()
                        .position(|b| *b == delimiter)
                        .map_or(bytes.len() - at, |n| n + 1);
                }
                b'>' if let Some(closing) = tag.take() => {
                    if closing {
                        *depth = depth.saturating_sub(1);
                    } else if bytes.get(at.wrapping_sub(1)) != Some(&b'/') {
                        *depth += 1;
                    }
                    if *depth == 0 {
                        stack.pop();
                    }
                    at += 1;
                }
                _ => at += 1,
            },
            Context::Code {
                expression,
                braces,
                nested_template,
                jsx_expression,
            } => match bytes[at] {
                b'`' => {
                    *nested_template = true;
                    stack.push(Context::Template {
                        approved: Vec::new(),
                    });
                    at += 1;
                }
                b'\'' | b'"' => at = quoted_end(bytes, at, bytes[at]),
                b'/' if bytes.get(at + 1) == Some(&b'/') => {
                    at += bytes[at..]
                        .iter()
                        .position(|b| *b == b'\n')
                        .unwrap_or(bytes.len() - at);
                }
                b'/' if bytes.get(at + 1) == Some(&b'*') => {
                    at += 2;
                    at += bytes[at..]
                        .windows(2)
                        .position(|b| b == b"*/")
                        .map_or(bytes.len() - at, |n| n + 2);
                }
                // Ambiguous slash syntax may be a regex. Prefer leaving the
                // old risk finding to mistaking its delimiters for JS code.
                b'/' => match regex_end(bytes, at) {
                    Some(end) => at = end,
                    None => break,
                },
                b'<' if tsx
                    && bytes
                        .get(at + 1)
                        .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'>') =>
                {
                    stack.push(Context::Jsx {
                        depth: 0,
                        tag: Some(false),
                    });
                    at += 1;
                }
                b'{' => {
                    *braces += 1;
                    at += 1;
                }
                b'}' if *braces > 0 => {
                    *braces -= 1;
                    at += 1;
                }
                b'}' if *jsx_expression => {
                    stack.pop();
                    at += 1;
                }
                b'}' if let Some(marker) = *expression => {
                    let keep = !*nested_template && approved_expression(text, marker, at);
                    stack.pop();
                    if keep && let Some(Context::Template { approved }) = stack.last_mut() {
                        approved.push(marker);
                    }
                    at += 1;
                }
                _ => at += 1,
            },
        }
    }
    String::from_utf8(view).expect("only ASCII dollars are masked")
}

#[cfg(test)]
mod tests {
    use super::secret_name;

    /// Owner, 2026-11-24 (r7 item 4): these stay risky.
    #[test]
    fn the_owners_secret_names_are_secret() {
        for name in [
            "apiKey",
            "apikey",
            "APIKEY",
            "GITHUBPAT",
            "githubToken",
            "myPassword",
            "clientSecret",
            "x-api-key",
        ] {
            assert!(secret_name(name), "{name}");
        }
    }

    /// Owner, 2026-11-24 (r7 item 4): recorded as they come out.
    #[test]
    fn the_owners_recorded_names() {
        for (name, secret) in [
            ("EXPO_OS", false),
            ("NODE_ENV", false),
            ("HOME", false),
            ("CODEX_HOME", false),
            ("author", false),
            ("tokenizer", true),
        ] {
            assert_eq!(secret_name(name), secret, "{name}");
        }
    }

    #[test]
    fn parts_are_split_on_separators_digits_and_case() {
        for name in [
            "GITHUB_TOKEN",
            "GH_PAT",
            "githubPat",
            "API2KEY",
            "OAUTH2_TOKEN",
            "APIKey",
            "SESSION_ID",
            "sessionId",
            "COOKIE_JAR",
            "privateKey",
            "AWS_SECRET_ACCESS_KEY",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "db-password",
            "kEy",
        ] {
            assert!(secret_name(name), "{name}");
        }
        for name in [
            "PATH",
            "PATTERN",
            "DISPATCH",
            "KEYBOARD",
            "MONKEY",
            "AWS_REGION",
            "TEST_ROOT",
            "LANG",
            "authority",
            "keys",
            "sessions",
            "XMLHttpRequest",
        ] {
            assert!(!secret_name(name), "{name}");
        }
    }
}
