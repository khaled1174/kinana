//! The strong secret and destructive tests of rules version 2, copied
//! verbatim from `crates/core/src/audit.rs` and `audit/evidence.rs` at commit
//! f40a88c (lines 322-347 and 381-523 of audit.rs). Do not edit: the
//! old-versus-new tests compare the current rules with exactly these.
//!
//! Only these two rules changed between versions 2 and 3; every other rule,
//! and the file-level checks, are the same code in both.
#![allow(dead_code, clippy::all)]

/// `strong_secrets` of rules version 2 (evidence.rs).
pub fn strong_secrets(text: &str) -> bool {
    let normal = normalize_text(text);
    let compact: String = normal.chars().filter(|c| !c.is_whitespace()).collect();
    has_secret_access(&normal, &compact, &words(&normal))
}

/// The text as the rules version 2 read it (lowercase, line continuations
/// and invisible characters removed).
pub fn normalize(text: &str) -> String {
    normalize_text(text)
}

/// `strong_destructive` of rules version 2 (evidence.rs).
pub fn strong_destructive(text: &str) -> bool {
    has_destructive_command(&shell_tokens(&normalize_text(text)))
}

fn normalize_text(text: &str) -> String {
    text.replace("\\\r\n", "")
        .replace("\\\n", "")
        .chars()
        .filter_map(|c| match c {
            // Arabic vocalization and formatting must not hide an instruction.
            '\u{0640}'
            | '\u{064b}'..='\u{065f}'
            | '\u{0670}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
            | '\u{feff}' => None,
            'أ' | 'إ' | 'آ' | 'ٱ' => Some('ا'),
            'ى' => Some('ي'),
            _ => Some(c),
        })
        .collect::<String>()
        .to_lowercase()
}

fn words(text: &str) -> Vec<&str> {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|word| !word.is_empty())
        .collect()
}

fn has_secret_access(text: &str, compact: &str, words: &[&str]) -> bool {
    [
        ".ssh",
        ".aws/",
        ".aws\\",
        ".env",
        ".netrc",
        ".git-credentials",
        "-----begin private key",
        "-----begin rsa private key",
    ]
    .iter()
    .any(|needle| text.contains(needle))
        || words.iter().any(|word| {
            word.starts_with("aws_")
                || (word.contains('_') && word.split('_').any(|part| part == "token"))
        })
        || has_credential_key_syntax(text, "token", &['='])
        || has_credential_key_syntax(text, "authorization", &[':', '='])
        || [
            "process.env",
            "os.environ",
            "os.getenv(",
            "getenv(",
            "env::var(",
            "env::vars(",
            "env::var_os(",
            "env::vars_os(",
            "$env:",
            "environment.getenvironmentvariable(",
        ]
        .iter()
        .any(|needle| compact.contains(needle))
        || text.as_bytes().windows(2).any(|pair| {
            pair[0] == b'$' && (pair[1].is_ascii_alphabetic() || matches!(pair[1], b'_' | b'{'))
        })
}

// Bare prose such as "color token" is not a credential. Assignment/header
// syntax and compound identifiers remain signals; values are never logged.
fn has_credential_key_syntax(text: &str, key: &str, separators: &[char]) -> bool {
    text.match_indices(key).any(|(start, _)| {
        let end = start + key.len();
        let identifier = |c: char| c.is_alphanumeric() || c == '_';
        if text[..start].chars().next_back().is_some_and(identifier)
            || text[end..].chars().next().is_some_and(identifier)
        {
            return false;
        }
        text[end..]
            .trim_start_matches(|c: char| c.is_whitespace() || matches!(c, '\'' | '"' | '`'))
            .chars()
            .next()
            .is_some_and(|c| separators.contains(&c))
    })
}

fn shell_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    for c in text.chars() {
        let separator = matches!(c, '|' | ';' | '&' | '(' | ')' | '<' | '>' | '\n');
        if c.is_whitespace() || separator || matches!(c, '\'' | '"' | '`') {
            if !token.is_empty() {
                tokens.push(std::mem::take(&mut token));
            }
            if separator {
                tokens.push(c.to_string());
            }
        } else {
            token.push(c);
        }
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    tokens
}

fn command_name(token: &str) -> &str {
    token.rsplit(['/', '\\']).next().map_or(token, |name| name)
}

fn command_end(tokens: &[String], start: usize) -> usize {
    tokens
        .iter()
        .enumerate()
        .skip(start)
        .take(80)
        .find(|(_, token)| matches!(token.as_str(), ";" | "&" | "|" | "\n"))
        .map_or(tokens.len().min(start + 80), |(index, _)| index)
}

fn has_destructive_command(tokens: &[String]) -> bool {
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
            "chmod"
                if args
                    .iter()
                    .any(|arg| matches!(arg.as_str(), "777" | "0777" | "0o777" | "a+rwx")) =>
            {
                return true;
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
