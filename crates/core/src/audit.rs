//! Read-only, static inspection of untrusted skill payloads.
//!
//! Findings are deliberately conservative hints, not a guarantee of safety. No
//! skill code, hooks, or instructions are executed, and links are never opened.

use crate::{Audit, AuditFinding, Error, Result, hash};
mod evidence;
use std::fs::{self, Metadata};
use std::path::{Component, Path, PathBuf};
use yaml_rust2::{Yaml, YamlLoader};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Severity {
    Clean,
    Review,
    Risky,
}

struct Findings {
    severity: Severity,
    retained_bytes: [usize; 2],
    omitted: usize,
    reasons: Vec<String>,
    details: Vec<AuditFinding>,
}

impl Findings {
    fn add(&mut self, severity: Severity, path: &Path, reason: &str) {
        let rule = if reason.contains("symbolic link") {
            "symlink"
        } else if reason.contains("frontmatter") {
            "hooks"
        } else if reason.contains("executable") {
            "executable"
        } else {
            "secrets"
        };
        self.record(rule, severity, path, reason, None, None, None);
    }
    #[allow(clippy::too_many_arguments)]
    fn record(
        &mut self,
        rule: &str,
        severity: Severity,
        path: &Path,
        reason: &str,
        line: Option<usize>,
        end_line: Option<usize>,
        excerpt: Option<String>,
    ) {
        self.severity = self.severity.max(severity);
        let strong = severity == Severity::Risky;
        // Independent budgets preserve strong evidence even after thousands of weak matches.
        let budget = &mut self.retained_bytes[usize::from(strong)];
        let size = 256
            + 12 * path.as_os_str().len()
            + 6 * (reason.len() + excerpt.as_ref().map_or(0, String::len));
        if *budget + size > 128 * 1024 {
            self.omitted += 1;
            return;
        }
        *budget += size;
        let reason = format!("{}: {reason}", path.display());
        if !self.reasons.contains(&reason) {
            self.reasons.push(reason);
        }
        self.details.push(AuditFinding {
            rule: rule.into(),
            strength: if strong { "strong" } else { "weak" }.into(),
            level: if strong { "risky" } else { "review" }.into(),
            reason_code: format!(
                "AUDIT_{}_{}",
                rule.to_ascii_uppercase(),
                if strong { "STRONG" } else { "WEAK" }
            ),
            path: path.into(),
            line,
            end_line,
            excerpt,
        });
    }
}

/// Inspect every file in a skill as data and report the highest finding level.
///
/// A symbolic link in the supplied root path is rejected. Child links receive a
/// finding based only on their stored target; even dangling targets are not read.
/// Repository `.git` metadata at the skill root is excluded from the payload.
pub fn audit_skill(path: &Path) -> Result<Audit> {
    let root = hash::checked_absolute_path(path)?;
    if !hash::checked_metadata(&root)?.is_dir() {
        return Err(Error::new(
            "NOT_A_DIRECTORY",
            format!("Skill path is not a directory: {}", root.display()),
        ));
    }
    let mut findings = Findings {
        severity: Severity::Clean,
        retained_bytes: [0; 2],
        omitted: 0,
        reasons: Vec::new(),
        details: Vec::new(),
    };
    inspect_directory(&root, Path::new(""), &mut findings)?;
    if findings.omitted != 0 {
        findings.reasons.push(format!(
            "{} additional audit findings omitted",
            findings.omitted
        ));
    }
    Ok(Audit {
        level: match findings.severity {
            Severity::Clean => "clean",
            Severity::Review => "review",
            Severity::Risky => "risky",
        }
        .into(),
        findings: findings.reasons,
        rules_version: 2,
        details: findings.details,
    })
}

fn inspect_directory(root: &Path, relative: &Path, findings: &mut Findings) -> Result<()> {
    let directory = root.join(relative);
    hash::checked_absolute_path(&directory)?;
    let mut children = fs::read_dir(&directory)
        .map_err(|error| io_error(&directory, error))?
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|error| io_error(&directory, error))?;
    children.sort_by_key(|child| child.file_name());
    for child in children {
        let child_relative = relative.join(child.file_name());
        let path = root.join(&child_relative);
        let metadata = fs::symlink_metadata(&path).map_err(|error| io_error(&path, error))?;
        if metadata.file_type().is_symlink() {
            inspect_link(root, &child_relative, findings)?;
            continue;
        }
        // Do not inspect credential-bearing repository config or executable git
        // hooks. Nested .git names are payload and are not silently excluded.
        if relative.as_os_str().is_empty()
            && child.file_name() == ".git"
            && (metadata.is_dir() || metadata.is_file())
        {
            continue;
        }
        if metadata.is_dir() {
            inspect_directory(root, &child_relative, findings)?;
        } else if metadata.is_file() {
            inspect_file(root, &child_relative, &metadata, findings)?;
        } else {
            return Err(Error::new(
                "UNSUPPORTED_FILE",
                format!(
                    "Only regular files, directories, and links can be audited: {}",
                    path.display()
                ),
            ));
        }
    }
    Ok(())
}

fn inspect_link(root: &Path, relative: &Path, findings: &mut Findings) -> Result<()> {
    let path = root.join(relative);
    let target = fs::read_link(&path).map_err(|error| io_error(&path, error))?;
    let parent = match path.parent() {
        Some(parent) => parent,
        None => root,
    };
    let resolved = lexical_normalize(&parent.join(&target));
    let (severity, reason) = if resolved.starts_with(root) {
        (
            Severity::Review,
            "symbolic link points inside the skill; target was not followed",
        )
    } else {
        (
            Severity::Risky,
            "symbolic link escapes the skill directory; target was not followed",
        )
    };
    findings.add(severity, relative, reason);
    Ok(())
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn inspect_file(
    root: &Path,
    relative: &Path,
    metadata: &Metadata,
    findings: &mut Findings,
) -> Result<()> {
    let path = root.join(relative);
    // Recheck every component immediately before reading. In particular, never
    // open a link discovered while walking the tree.
    hash::checked_absolute_path(&path)?;
    let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
    if is_script(relative)
        || is_executable(metadata)
        || bytes.starts_with(b"#!")
        || is_binary_executable(&bytes)
    {
        findings.add(
            Severity::Review,
            relative,
            "executable or script file requires review",
        );
    }
    let text = String::from_utf8_lossy(&bytes);
    if is_binary_executable(&bytes) {
        findings.add(
            Severity::Risky,
            relative,
            "binary executable content requires review",
        );
    }
    if sensitive_path(relative) {
        findings.add(
            Severity::Risky,
            relative,
            "secret or environment variable access",
        );
    }
    evidence::collect_text(&text, relative, findings);
    if relative.file_name().is_some_and(|name| name == "SKILL.md") {
        inspect_frontmatter(&text, relative, findings);
    }
    Ok(())
}

fn is_script(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|extension| {
            matches!(
                extension.as_str(),
                "sh" | "bash"
                    | "zsh"
                    | "fish"
                    | "ksh"
                    | "csh"
                    | "command"
                    | "py"
                    | "pyw"
                    | "js"
                    | "mjs"
                    | "cjs"
                    | "ts"
                    | "tsx"
                    | "jsx"
                    | "rb"
                    | "pl"
                    | "pm"
                    | "php"
                    | "lua"
                    | "r"
                    | "swift"
                    | "ps1"
                    | "psm1"
                    | "psd1"
                    | "bat"
                    | "cmd"
                    | "vbs"
                    | "vbe"
                    | "wsf"
                    | "applescript"
                    | "scpt"
                    | "exe"
                    | "dll"
                    | "so"
                    | "dylib"
                    | "bin"
                    | "jar"
                    | "class"
                    | "wasm"
            )
        })
}

#[cfg(unix)]
fn is_executable(metadata: &Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &Metadata) -> bool {
    false
}

fn is_binary_executable(bytes: &[u8]) -> bool {
    [
        b"\x7fELF".as_slice(),
        b"MZ".as_slice(),
        b"\xfe\xed\xfa\xce".as_slice(),
        b"\xce\xfa\xed\xfe".as_slice(),
        b"\xfe\xed\xfa\xcf".as_slice(),
        b"\xcf\xfa\xed\xfe".as_slice(),
        b"\xca\xfe\xba\xbe".as_slice(),
        b"\0asm".as_slice(),
    ]
    .iter()
    .any(|magic| bytes.starts_with(magic))
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

fn has_network_access(text: &str, compact: &str, words: &[&str]) -> bool {
    words.iter().any(|word| {
        matches!(
            *word,
            "curl" | "wget" | "urlopen" | "invoke_webrequest" | "iwr" | "irm"
        )
    }) || ["http://", "https://", "ftp://"]
        .iter()
        .any(|needle| text.contains(needle))
        || [
            "fetch(",
            "requests.",
            "urllib.",
            "http.get(",
            "https.get(",
            "invoke-webrequest",
            "invoke-restmethod",
        ]
        .iter()
        .any(|needle| compact.contains(needle))
}

fn sensitive_path(path: &Path) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy().to_ascii_lowercase();
        matches!(
            name.as_str(),
            ".ssh" | ".aws" | ".env" | ".netrc" | ".npmrc" | ".git-credentials"
        ) || name.starts_with(".env.")
    })
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

fn is_downloader(token: &str) -> bool {
    matches!(
        command_name(token),
        "curl" | "wget" | "invoke-webrequest" | "invoke-restmethod" | "iwr" | "irm"
    )
}

fn is_interpreter(token: &str) -> bool {
    matches!(
        command_name(token),
        "sh" | "bash"
            | "zsh"
            | "fish"
            | "ksh"
            | "dash"
            | "python"
            | "python3"
            | "ruby"
            | "perl"
            | "node"
            | "php"
            | "eval"
            | "exec"
            | "source"
            | "."
            | "iex"
            | "invoke-expression"
            | "powershell"
            | "pwsh"
    )
}

fn has_download_execution(tokens: &[String], compact: &str) -> bool {
    for (index, token) in tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| is_downloader(token))
    {
        let limit = tokens.len().min(index + 160);
        let following = &tokens[index + 1..limit];
        // A pipe into an interpreter, including common command wrappers.
        for (offset, next) in following.iter().enumerate() {
            if next == "|" {
                let after = &following[offset + 1..];
                if after
                    .iter()
                    .take(5)
                    .take_while(|arg| !matches!(arg.as_str(), ";" | "&" | "\n" | "|"))
                    .any(|arg| is_interpreter(arg))
                {
                    return true;
                }
            }
            if matches!(next.as_str(), ";" | "&" | "\n") {
                break;
            }
        }
        // bash <(curl ...), eval "$(curl ...)", and similar substitutions.
        let before = &tokens[index.saturating_sub(12)..index];
        if before.iter().any(|arg| is_interpreter(arg)) && before.iter().any(|arg| arg == "(") {
            return true;
        }
        let end = command_end(tokens, index + 1);
        let args = &tokens[index + 1..end];
        let mut targets = Vec::new();
        for (offset, arg) in args.iter().enumerate() {
            if matches!(
                arg.as_str(),
                "-o" | "--output" | "--output-document" | "-outfile" | ">"
            ) {
                if let Some(target) = args.get(offset + 1).filter(|target| target.as_str() != "-") {
                    targets.push(target.as_str());
                }
            } else if let Some(target) = arg
                .strip_prefix("--output=")
                .or_else(|| arg.strip_prefix("--output-document="))
            {
                targets.push(target);
            } else if let Some(target) = arg
                .strip_prefix("-o")
                .filter(|target| !target.is_empty() && *target != "-")
            {
                targets.push(target);
            }
        }
        // wget also writes the URL basename when no output option is supplied.
        if command_name(token) == "wget" && targets.is_empty() {
            for arg in args.iter().filter(|arg| arg.contains("://")) {
                if let Some(name) = arg.rsplit('/').next().filter(|name| !name.is_empty()) {
                    targets.push(name);
                }
            }
        }
        for execution in end..limit {
            if targets
                .iter()
                .any(|target| same_script_path(target, &tokens[execution]))
            {
                let start = execution.saturating_sub(8).max(end);
                let earlier = &tokens[start..execution];
                if earlier
                    .iter()
                    .rev()
                    .take_while(|arg| !matches!(arg.as_str(), ";" | "&" | "\n" | "|"))
                    .any(|arg| is_interpreter(arg))
                    || (execution > 0
                        && matches!(tokens[execution - 1].as_str(), ";" | "&" | "\n" | "|"))
                {
                    return true;
                }
            }
        }
    }
    // Downloads evaluated directly by common scripting runtimes.
    [
        "exec(requests.",
        "eval(requests.",
        "exec(urllib.",
        "eval(urllib.",
        "exec(urlopen(",
        "eval(awaitfetch(",
        "eval(fetch(",
    ]
    .iter()
    .any(|needle| compact.contains(needle))
}

fn same_script_path(left: &str, right: &str) -> bool {
    left.trim_start_matches("./") == right.trim_start_matches("./")
}

fn has_instruction_override_legacy(words: &[&str]) -> bool {
    for (index, word) in words.iter().enumerate() {
        let next = &words[index + 1..words.len().min(index + 13)];
        if matches!(*word, "ignore" | "disregard" | "override" | "bypass")
            && next
                .iter()
                .take(8)
                .any(|next| matches!(*next, "instruction" | "instructions" | "rules" | "safety"))
        {
            return true;
        }
        if matches!(*word, "hide" | "conceal") && next.contains(&"user") {
            return true;
        }
        if *word == "keep" && next.contains(&"secret") && next.contains(&"user") {
            return true;
        }
        if (*word == "not" || *word == "don")
            && next
                .first()
                .is_some_and(|next| matches!(*next, "tell" | "t" | "disclose" | "inform"))
            && (next.contains(&"user") || next.contains(&"them"))
        {
            return true;
        }
        if matches!(*word, "تجاهل" | "وتجاهل" | "تجاوز" | "اهمل")
            && next.iter().take(8).any(|next| next.contains("تعليمات"))
        {
            return true;
        }
        if (word.starts_with("اخف")
            || word.starts_with("واخف")
            || matches!(*word, "اكتم" | "واكتم"))
            && next.iter().any(|next| next.contains("مستخدم"))
        {
            return true;
        }
        if *word == "لا"
            && next
                .first()
                .is_some_and(|next| matches!(*next, "تخبر" | "تبلغ" | "تعلم" | "تكشف" | "تظهر"))
            && next.iter().any(|next| next.contains("مستخدم"))
        {
            return true;
        }
    }
    false
}

fn has_instruction_override(words: &[&str]) -> bool {
    for (index, word) in words.iter().enumerate() {
        let next = &words[index + 1..words.len().min(index + 13)];
        if matches!(*word, "ignore" | "disregard" | "override" | "bypass")
            && next
                .iter()
                .take(8)
                .any(|w| matches!(*w, "instruction" | "instructions" | "rules" | "safety"))
        {
            return true;
        }
        if matches!(*word, "hide" | "conceal") && next.contains(&"from") && next.contains(&"user") {
            return true;
        }
        if *word == "keep"
            && next.contains(&"secret")
            && next.contains(&"from")
            && next.contains(&"user")
        {
            return true;
        }
        let verb = if *word == "don" && next.first() == Some(&"t") {
            next.get(1)
        } else if *word == "not" {
            next.first()
        } else {
            None
        };
        if verb.is_some_and(|w| matches!(*w, "tell" | "disclose" | "inform"))
            && (next.contains(&"user") || next.contains(&"them"))
        {
            return true;
        }
        if matches!(*word, "تجاهل" | "وتجاهل" | "تجاوز" | "اهمل")
            && next.iter().take(8).any(|w| w.contains("تعليمات"))
        {
            return true;
        }
        if (word.starts_with("اخف")
            || word.starts_with("واخف")
            || matches!(*word, "اكتم" | "واكتم"))
            && next.iter().any(|w| w.contains("مستخدم"))
        {
            return true;
        }
        if *word == "لا"
            && next
                .first()
                .is_some_and(|w| matches!(*w, "تخبر" | "تبلغ" | "تعلم" | "تكشف" | "تظهر"))
            && next.iter().any(|w| w.contains("مستخدم"))
        {
            return true;
        }
    }
    false
}

fn inspect_frontmatter(text: &str, relative: &Path, findings: &mut Findings) {
    let text = text.strip_prefix('\u{feff}').map_or(text, |text| text);
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return;
    }
    let mut yaml = String::new();
    let mut closed = false;
    for line in lines {
        if line.trim_end() == "---" {
            closed = true;
            break;
        }
        yaml.push_str(line);
        yaml.push('\n');
    }
    if !closed {
        findings.add(
            Severity::Review,
            relative,
            "unclosed YAML frontmatter could not be inspected",
        );
        return;
    }
    match YamlLoader::load_from_str(&yaml) {
        Ok(documents) if documents.len() == 1 && documents[0].as_hash().is_some() => {
            if has_hooks_key(&documents[0]) {
                let severity = if executable_hooks(&documents[0]) {
                    Severity::Risky
                } else {
                    Severity::Review
                };
                let line = hooks_source_line(&yaml);
                findings.record(
                    "hooks",
                    severity,
                    relative,
                    "hooks defined in YAML frontmatter",
                    line,
                    line,
                    None,
                );
            }
        }
        _ => findings.add(
            Severity::Review,
            relative,
            "invalid YAML frontmatter could not be inspected",
        ),
    }
}

fn hooks_source_line(yaml: &str) -> Option<usize> {
    use yaml_rust2::{
        parser::{Event, MarkedEventReceiver, Parser},
        scanner::Marker,
    };
    #[derive(Default)]
    struct Keys {
        stack: Vec<Option<bool>>,
        root: Vec<usize>,
        nested: Vec<usize>,
    }
    impl Keys {
        fn finish(&mut self) {
            if let Some(Some(key)) = self.stack.last_mut() {
                *key = !*key;
            }
        }
    }
    impl MarkedEventReceiver for Keys {
        fn on_event(&mut self, event: Event, mark: Marker) {
            match event {
                Event::MappingStart(..) => self.stack.push(Some(true)),
                Event::SequenceStart(..) => self.stack.push(None),
                Event::MappingEnd | Event::SequenceEnd => {
                    self.stack.pop();
                    self.finish();
                }
                Event::Scalar(value, ..) => {
                    if value == "hooks" && self.stack.last() == Some(&Some(true)) {
                        // YAML starts after the opening SKILL.md delimiter.
                        if self.stack.len() == 1 {
                            self.root.push(mark.line() + 1);
                        } else {
                            self.nested.push(mark.line() + 1);
                        }
                    }
                    self.finish();
                }
                Event::Alias(..) => self.finish(),
                _ => {}
            }
        }
    }
    let mut keys = Keys::default();
    Parser::new_from_str(yaml).load(&mut keys, true).ok()?;
    keys.root
        .first()
        .copied()
        .or_else(|| (keys.nested.len() == 1).then(|| keys.nested[0]))
}

fn has_hooks_key(yaml: &Yaml) -> bool {
    yaml.as_hash().is_some_and(|mapping| {
        mapping.iter().any(|(key, value)| {
            key.as_str() == Some("hooks")
                || (key.as_str() == Some("<<")
                    && (has_hooks_key(value)
                        || value
                            .as_vec()
                            .is_some_and(|values| values.iter().any(has_hooks_key))))
        })
    })
}

fn io_error(path: &Path, error: std::io::Error) -> Error {
    Error::new("IO_ERROR", format!("{}: {error}", path.display()))
}

fn executable_hooks(yaml: &Yaml) -> bool {
    fn executable(value: &Yaml) -> bool {
        match value {
            Yaml::Hash(map) => map.iter().any(|(key, value)| {
                matches!(key.as_str(), Some("command" | "prompt"))
                    || (key.as_str() == Some("type")
                        && value
                            .as_str()
                            .is_some_and(|v| matches!(v, "command" | "prompt" | "agent" | "http")))
                    || executable(value)
            }),
            Yaml::Array(values) => values.iter().any(executable),
            _ => false,
        }
    }
    yaml.as_hash().is_some_and(|mapping| {
        mapping.iter().any(|(key, value)| {
            (key.as_str() == Some("hooks") && executable(value))
                || (key.as_str() == Some("<<")
                    && (executable_hooks(value)
                        || value
                            .as_vec()
                            .is_some_and(|values| values.iter().any(executable_hooks))))
        })
    })
}

/// Explicit metadata refresh. Reading a vault never migrates stored audits.
pub fn refresh(
    vault: &crate::vault::Vault,
    ids: &[String],
    dry_run: bool,
) -> Result<crate::EditResult> {
    let _state_guard = crate::state::write_guard(&vault.index_path, dry_run)?;
    let selected = if ids.is_empty() {
        vault.records()?.into_iter().map(|r| r.id).collect()
    } else {
        ids.to_vec()
    };
    let mut updates = std::collections::BTreeMap::new();
    for id in selected {
        crate::vault::validate_id(&id)?;
        let mut metadata = crate::edit::read_metadata(vault, &id)?;
        metadata.audit = audit_skill(&vault.root.join("skills").join(&id))?;
        updates.insert(id, metadata);
    }
    crate::edit::apply(
        vault,
        &updates,
        "Refresh static audit with strong and weak evidence",
        dry_run,
    )
}
