//! Review hints for package executors and direct-source installs.
use super::{command_end, command_name, normalize_text, shell_tokens};

fn numeric_version(version: &str) -> bool {
    let core = version
        .strip_prefix('v')
        .unwrap_or(version)
        .split(['-', '+'])
        .next()
        .unwrap_or_default();
    !core.is_empty()
        && core
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

fn explicit_pin(package: &str, python: bool) -> bool {
    let version = if python {
        package
            .rsplit_once("==")
            .or_else(|| package.rsplit_once('@'))
    } else {
        package.rsplit_once('@')
    };
    version.is_some_and(|(name, version)| {
        !name.is_empty() && numeric_version(version) && !version.contains("://")
    })
}

fn unpinned(args: &[String], package_flags: &[&str], python: bool) -> bool {
    let mut explicit = false;
    let mut args = args.iter().map(String::as_str);
    while let Some(arg) = args.next() {
        if package_flags.contains(&arg) {
            explicit = true;
            if !args.next().is_some_and(|pkg| explicit_pin(pkg, python)) {
                return true;
            }
        } else if let Some((flag, value)) = arg.split_once('=')
            && package_flags.contains(&flag)
        {
            explicit = true;
            if !explicit_pin(value, python) {
                return true;
            }
        } else if !arg.starts_with('-') {
            return !explicit && !explicit_pin(arg, python);
        }
    }
    false
}

pub(super) fn requires_review(text: &str) -> bool {
    let tokens = shell_tokens(&normalize_text(text));
    tokens.iter().enumerate().any(|(index, token)| {
        let name = command_name(token);
        let is_pip = name
            .strip_prefix("pip")
            .is_some_and(|suffix| suffix.is_empty() || numeric_version(suffix));
        if !is_pip && !matches!(name, "npx" | "npm" | "uvx" | "pipx") {
            return false;
        }
        let end = command_end(&tokens, index + 1);
        let args = &tokens[index + 1..end];
        match name {
            "npx" => unpinned(args, &["-p", "--package"], false),
            "npm" => args.first().is_some_and(|arg| arg == "exec"),
            "uvx" => unpinned(args, &["--from"], true),
            "pipx" if args.first().is_some_and(|arg| arg == "run") => {
                unpinned(&args[1..], &["--spec"], true)
            }
            _ if is_pip && args.first().is_some_and(|arg| arg == "install") => args[1..]
                .iter()
                .any(|arg| arg.contains("://") || arg.starts_with("git+")),
            _ => false,
        }
    })
}
