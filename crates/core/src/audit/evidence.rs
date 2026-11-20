//! Rule strength and source locations. Inspection remains text-only.
use super::*;
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
fn strong_secrets(text: &str) -> bool {
    let (n, c) = data(text);
    has_secret_access(&n, &c, &words(&n))
}
fn weak_secrets(text: &str) -> bool {
    let (n, _) = data(text);
    words(&n)
        .iter()
        .any(|w| matches!(*w, "credentials" | "password" | "passwords"))
        || n.contains("environment variables")
}
fn strong_destructive(text: &str) -> bool {
    has_destructive_command(&shell_tokens(&normalize_text(text)))
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
fn excerpt(text: &str) -> Option<String> {
    // Do not log secret-bearing source lines, URL credentials, or query values.
    if strong_secrets(text)
        || text.contains('@')
        || text.contains('?')
        || (weak_secrets(text) && (text.contains('=') || text.contains(':')))
    {
        return None;
    }
    Some(text.to_string())
}
pub(super) fn collect_text(text: &str, path: &Path, findings: &mut Findings) {
    let lines = text.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return;
    }
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
            strong_secrets,
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
            "instruction_override",
            "instruction override or concealment from the user",
            strong_instruction,
            weak_instruction,
        ),
    ] {
        let mut any_strong = false;
        let mut any_weak = false;
        for (i, line) in lines.iter().enumerate() {
            let severity = if strong(line) {
                any_strong = true;
                Some(Severity::Risky)
            } else if weak(line) {
                any_weak = true;
                Some(Severity::Review)
            } else {
                None
            };
            if let Some(severity) = severity {
                findings.record(
                    rule,
                    severity,
                    path,
                    reason,
                    Some(i + 1),
                    Some(i + 1),
                    if sensitive_path(path) {
                        None
                    } else {
                        excerpt(line)
                    },
                );
            }
        }
        let global = if !any_strong && strong(text) {
            Some((Severity::Risky, strong))
        } else if !any_strong && !any_weak && weak(text) {
            Some((Severity::Review, weak))
        } else {
            None
        };
        if let Some((severity, predicate)) = global {
            let (start, end) = locate(&lines, predicate);
            findings.record(
                rule,
                severity,
                path,
                reason,
                Some(start + 1),
                Some(end),
                if sensitive_path(path) {
                    None
                } else {
                    excerpt(&lines[start..end].join("\n"))
                },
            );
        }
    }
}
