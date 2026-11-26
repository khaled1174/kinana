//! Rule strength and source locations. Inspection remains text-only.
use super::*;
use std::borrow::Cow;

pub(super) const FORCED_DELETE: &str = "destructive command: recursive forced deletion";
fn data(text: &str) -> (String, String) {
    let normal = normalize_text(text);
    let compact = normal.chars().filter(|c| !c.is_whitespace()).collect();
    (normal, compact)
}
fn strong_network(text: &str) -> bool {
    let (normal, compact) = data(text);
    let tokens = shell_tokens(&normal);
    for (i, token) in tokens.iter().enumerate().filter(|(_, t)| is_downloader(t)) {
        let _ = token;
        if tokens[i + 1..command_end(&tokens, i + 1)].iter().any(|t| {
            ["http://", "https://", "ftp://"]
                .iter()
                .any(|prefix| t.starts_with(prefix))
        }) {
            return true;
        }
    }
    [
        "fetch(",
        "urlopen(",
        "http.get(",
        "https.get(",
        "requests.get(",
        "requests.post(",
        "requests.put(",
        "requests.patch(",
        "requests.delete(",
        "requests.request(",
        "urllib.request.urlopen(",
    ]
    .iter()
    .any(|call| compact.contains(call))
}
fn weak_network(text: &str) -> bool {
    let (n, c) = data(text);
    has_network_access(&n, &c, &words(&n))
}
/// The old secret test less plain `$VAR` references, or a new secret pattern.
fn strong_secrets(text: &str) -> bool {
    precision::strong_secret(text, false)
}
/// The same in JavaScript or TypeScript source, where `${expr}` inside a
/// template literal is a plain reference (approved exception).
fn strong_secrets_in_script(text: &str) -> bool {
    precision::strong_secret(text, true)
}
fn javascript_source(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "js" | "mjs"
                | "cjs"
                | "jsx"
                | "ts"
                | "mts"
                | "cts"
                | "tsx"
                | "vue"
                | "svelte"
                | "astro"
        )
    })
}
/// The broad rules-version-2 test; it still decides which excerpts are hidden,
/// so narrowing the strong rule never reveals a value that was withheld.
fn may_hold_secret(text: &str) -> bool {
    let (n, c) = data(text);
    has_secret_access(&n, &c, &words(&n))
}
fn weak_secrets(text: &str) -> bool {
    let (n, _) = data(text);
    words(&n)
        .iter()
        .any(|w| matches!(*w, "credentials" | "password" | "passwords"))
        || n.contains("environment variables")
        || precision::private_key_header(text)
        || precision::secret_location_mention(text)
}
fn strong_destructive(text: &str) -> bool {
    precision::has_destructive_command(&normalize_text(text))
}
fn weak_destructive(text: &str) -> bool {
    let t = shell_tokens(&normalize_text(text));
    t.iter().any(|w| {
        matches!(
            command_name(w),
            "rm" | "chmod" | "remove-item" | "rd" | "rmdir"
        )
    }) || t.windows(2).any(|w| w[0] == "git" && w[1] == "push")
}
fn strong_download(text: &str) -> bool {
    let (n, c) = data(text);
    has_download_execution(&shell_tokens(&n), &c)
}
fn weak_download(text: &str) -> bool {
    let t = shell_tokens(&normalize_text(text));
    t.iter().any(|t| is_downloader(t)) && t.iter().any(|t| is_interpreter(t))
}
fn strong_instruction(text: &str) -> bool {
    text.split("\n\n")
        .any(|part| has_instruction_override(&words(&normalize_text(part))))
}
fn weak_instruction(text: &str) -> bool {
    text.split("\n\n")
        .any(|part| has_instruction_override_legacy(&words(&normalize_text(part))))
}
type Predicate = fn(&str) -> bool;
fn never(_: &str) -> bool {
    false
}
fn locate(lines: &[&str], predicate: Predicate) -> (usize, usize) {
    // Find the first matching prefix, then the shortest suffix ending there.
    // Every candidate is verified; fall back to the full span for unusual grammars.
    let mut lo = 1;
    let mut hi = lines.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        if predicate(&lines[..mid].join("\n")) {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    let end = lo;
    let mut lo = 0;
    let mut hi = end - 1;
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if predicate(&lines[mid..end].join("\n")) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    if predicate(&lines[lo..end].join("\n")) {
        (lo, end)
    } else {
        (0, lines.len())
    }
}
fn simple_safe_delete(text: &str) -> bool {
    let mut parts = text.split_ascii_whitespace();
    if !matches!(parts.next(), Some("rm" | "/bin/rm"))
        || !matches!(parts.next(), Some("-rf" | "-fr"))
    {
        return false;
    }
    let Some(target) = parts
        .next()
        .and_then(|target| target.strip_prefix('"'))
        .and_then(|target| target.strip_suffix('"'))
        .and_then(|target| target.strip_prefix('$'))
    else {
        return false;
    };
    let name = target
        .strip_prefix('{')
        .and_then(|target| target.strip_suffix('}'))
        .unwrap_or(target);
    !name.is_empty()
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !environment::secret_name(name)
        && parts.next().is_none()
}

#[derive(Clone, Copy)]
enum Withheld {
    Credential,
    PrivateKey,
}

fn credential_option(normalized: &str) -> bool {
    let mut segment_has_command = false;
    let mut mkdir = false;
    for token in shell_tokens(normalized) {
        if matches!(token.as_str(), "|" | ";" | "&" | "\n" | "(" | ")") {
            segment_has_command = false;
            mkdir = false;
        } else if !segment_has_command {
            segment_has_command = true;
            mkdir = command_name(&token) == "mkdir";
        }
        if mkdir
            && token.strip_prefix('-').is_some_and(|flags| {
                !flags.is_empty() && flags.bytes().all(|flag| matches!(flag, b'p' | b'v'))
            })
        {
            continue;
        }
        if matches!(
            token.split('=').next(),
            Some("--password" | "--user" | "--proxy-user" | "--proxy-password")
        ) || token.strip_prefix('-').is_some_and(|option| {
            !option.starts_with('-')
                && option
                    .bytes()
                    .take_while(u8::is_ascii_alphabetic)
                    .any(|c| matches!(c, b'p' | b'u'))
        }) {
            return true;
        }
    }
    false
}

fn update_key_span(text: &str, key_label: &mut Option<String>) -> bool {
    let mut private = key_label.is_some();
    for (at, _) in text.match_indices("-----") {
        let rest = &text[at + 5..];
        let (begins, label) = if let Some(label) = rest.strip_prefix("begin ") {
            (true, label)
        } else if let Some(label) = rest.strip_prefix("end ") {
            (false, label)
        } else {
            continue;
        };
        // The boundary is bounded even for malformed input full of markers.
        let Some(end) = label.as_bytes()[..label.len().min(140)]
            .windows(5)
            .position(|window| window == b"-----")
        else {
            continue;
        };
        let label = &label[..end];
        if begins
            && (label == "private key"
                || label == "private key block"
                || label.ends_with(" private key")
                || label.ends_with(" private key block"))
        {
            private = true;
            if key_label.is_none() {
                *key_label = Some(label.to_owned());
            }
        } else if !begins && key_label.as_deref() == Some(label) {
            private = true;
            *key_label = None;
        }
    }
    private
}

#[derive(Clone, Copy)]
enum HeaderTail {
    Separator,
    Value,
    Quoted(u8),
}

#[derive(Default)]
struct HeaderContext {
    tail: Option<HeaderTail>,
    in_comment: bool,
}

impl HeaderContext {
    fn continue_line(&mut self, mut text: &str) {
        loop {
            match self.tail {
                None => return,
                Some(HeaderTail::Quoted(delimiter)) => {
                    let mut escaped = false;
                    let closing = text.bytes().position(|byte| {
                        let closes = !escaped && byte == delimiter;
                        escaped = !escaped && byte == b'\\';
                        closes
                    });
                    let Some(closing) = closing else {
                        return;
                    };
                    text = text[closing + 1..].trim_start();
                    if let Some(after) = text.strip_prefix('+') {
                        self.tail = Some(HeaderTail::Value);
                        text = after;
                        continue;
                    }
                    self.tail = None;
                    return;
                }
                _ => {}
            }
            if self.in_comment {
                let Some((_, after)) = text.split_once("*/") else {
                    return;
                };
                self.in_comment = false;
                text = after;
                continue;
            }
            text = text.trim_start();
            if let Some(rest) = text.strip_prefix("/*") {
                self.in_comment = true;
                text = rest;
                continue;
            }
            if text.is_empty() || text.starts_with("//") {
                return;
            }
            match self.tail {
                Some(HeaderTail::Separator) => {
                    if let Some(after) = text.strip_prefix([':', ',', '=']) {
                        self.tail = Some(HeaderTail::Value);
                        text = after;
                    } else {
                        self.tail = None;
                    }
                }
                Some(HeaderTail::Value) => {
                    self.tail = None;
                    if let Some(delimiter @ (b'"' | b'\'' | b'\x60')) = text.bytes().next() {
                        self.tail = Some(HeaderTail::Quoted(delimiter));
                        text = &text[1..];
                    }
                }
                _ => return,
            }
        }
    }

    fn withholds(&mut self, text: &str) -> bool {
        let continued = self.tail.is_some();
        self.continue_line(text);
        if self.tail.is_some() {
            return true;
        }
        let mut found = false;
        for (at, key) in text.match_indices("x-api-key") {
            found = true;
            // A quoted object key or tuple value can be followed by a comma
            // or colon, whitespace and comments before the value starts.
            let quote = text.as_bytes().get(at.wrapping_sub(1)).copied();
            let after = &text[at + key.len()..];
            let after = match quote {
                Some(q @ (b'"' | b'\'' | b'`')) if after.as_bytes().first() == Some(&q) => {
                    &after[1..]
                }
                _ => after,
            };
            self.tail = Some(HeaderTail::Separator);
            self.in_comment = false;
            self.continue_line(after);
            if self.tail.is_some() {
                break;
            }
        }
        continued || found
    }
}

fn withheld_lines(lines: &[&str]) -> Vec<Option<Withheld>> {
    let mut key_label = None;
    let mut start = 0;
    let mut header = HeaderContext::default();
    let mut findings = vec![None; lines.len()];
    for (i, line) in lines.iter().enumerate() {
        if i + 1 != lines.len() && line.trim_end().ends_with('\\') {
            continue;
        }
        let logical = normalize_text(&lines[start..=i].join("\n"));
        let private = update_key_span(&logical, &mut key_label);
        let credentials = header.withholds(&logical) || credential_option(&logical);
        if private {
            findings[start..=i].fill(Some(Withheld::PrivateKey));
        } else if credentials {
            findings[start..=i].fill(Some(Withheld::Credential));
        }
        start = i + 1;
    }
    findings
}

fn display_evidence(
    text: &str,
    rule: &str,
    sensitive_path: bool,
    withheld: Option<Withheld>,
) -> (Option<String>, Vec<String>) {
    if let Some(reason) = withheld {
        use super::forms::{LITERAL_SECRET, PRIVATE_KEY};
        let marker = match reason {
            Withheld::Credential => LITERAL_SECRET,
            Withheld::PrivateKey => PRIVATE_KEY,
        };
        return (
            None,
            if rule == "secrets" {
                vec![marker.into()]
            } else {
                vec![]
            },
        );
    }
    (
        if sensitive_path {
            None
        } else {
            excerpt(text, rule)
        },
        if rule == "secrets" {
            super::forms::secret_forms(text, sensitive_path)
        } else {
            vec![]
        },
    )
}

fn excerpt(text: &str, rule: &str) -> Option<String> {
    // Do not log secret-bearing source lines, URL credentials, or query values.
    if (may_hold_secret(text) && !(rule == "destructive" && simple_safe_delete(text)))
        || strong_secrets(text)
        || text.contains('@')
        || text.contains('?')
        || (weak_secrets(text) && (text.contains('=') || text.contains(':')))
    {
        return None;
    }
    let mut chars =
        crate::catalog::printable(&text.split_whitespace().collect::<Vec<_>>().join(" "))
            .chars()
            .take(161)
            .collect::<Vec<_>>();
    if chars.len() > 160 {
        chars.truncate(160);
        chars.push('…');
    }
    Some(chars.into_iter().collect())
}

fn finding_reason(
    rule: &str,
    reason: &'static str,
    text: &str,
    severity: Severity,
) -> &'static str {
    if rule == "destructive"
        && severity == Severity::Risky
        && precision::has_recursive_forced_delete(&normalize_text(text))
    {
        FORCED_DELETE
    } else {
        reason
    }
}

pub(super) fn collect_text(text: &str, path: &Path, findings: &mut Findings) {
    let lines = text.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return;
    }
    let withheld = withheld_lines(&lines);
    let sensitive = sensitive_path(path);
    let multiline_templates = environment::supports_multiline_templates(path);
    let ignore_file = environment::is_ignore_file(path);
    let secret_view = if ignore_file {
        Cow::Owned(environment::mask_ignored_env_paths(text))
    } else if multiline_templates {
        let tsx = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("tsx"));
        Cow::Owned(environment::mask_templates(text, tsx))
    } else {
        Cow::Borrowed(text)
    };
    let secret_lines = secret_view.lines().collect::<Vec<_>>();
    for (rule, reason, strong, weak) in [
        (
            "network",
            "external network access string",
            strong_network as Predicate,
            weak_network as Predicate,
        ),
        (
            "secrets",
            "secret or environment variable access",
            if javascript_source(path) && !multiline_templates {
                strong_secrets_in_script
            } else {
                strong_secrets
            },
            weak_secrets,
        ),
        (
            "destructive",
            "destructive command string",
            strong_destructive,
            weak_destructive,
        ),
        (
            "download_execution",
            "download followed by execution",
            strong_download,
            weak_download,
        ),
        (
            "package_execution",
            "unpinned package execution or direct-source install",
            never,
            packages::requires_review,
        ),
        (
            "instruction_override",
            "instruction override or concealment from the user",
            strong_instruction,
            weak_instruction,
        ),
    ] {
        let mut any_strong = false;
        let mut any_weak = false;
        let is_secret = rule == "secrets";
        let ignore_env_reference = is_secret && ignore_file;
        let tested_lines = if is_secret { &secret_lines } else { &lines };
        let tested_text = if is_secret {
            secret_view.as_ref()
        } else {
            text
        };
        for (i, line) in lines.iter().enumerate() {
            let tested = tested_lines[i];
            let severity = if strong(tested)
                || (is_secret && line != &tested && precision::strong_secret_form(line))
            {
                any_strong = true;
                Some(Severity::Risky)
            } else if weak(if ignore_env_reference { tested } else { line }) {
                any_weak = true;
                Some(Severity::Review)
            } else {
                None
            };
            if let Some(severity) = severity {
                let (excerpt, forms) = display_evidence(line, rule, sensitive, withheld[i]);
                findings.record(
                    rule,
                    severity,
                    path,
                    finding_reason(rule, reason, line, severity),
                    Some(i + 1),
                    Some(i + 1),
                    excerpt,
                    forms,
                );
            }
        }
        let global = if !any_strong && strong(tested_text) {
            Some((Severity::Risky, strong, tested_lines))
        } else if !any_strong
            && is_secret
            && tested_text != text
            && precision::strong_secret_form(text)
        {
            Some((
                Severity::Risky,
                precision::strong_secret_form as Predicate,
                &lines,
            ))
        } else if !any_strong
            && !any_weak
            && weak(if ignore_env_reference {
                tested_text
            } else {
                text
            })
        {
            Some((
                Severity::Review,
                weak,
                if ignore_env_reference {
                    tested_lines
                } else {
                    &lines
                },
            ))
        } else {
            None
        };
        if let Some((severity, predicate, locations)) = global {
            let (start, end) = locate(locations, predicate);
            let source = lines[start..end].join("\n");
            let (excerpt, forms) = display_evidence(
                &source,
                rule,
                sensitive,
                withheld[start..end].iter().copied().flatten().next(),
            );
            findings.record(
                rule,
                severity,
                path,
                finding_reason(rule, reason, &source, severity),
                Some(start + 1),
                Some(end),
                excerpt,
                forms,
            );
        }
    }
}
