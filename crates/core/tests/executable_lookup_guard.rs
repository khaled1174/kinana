//! Static guard (owner decision, 2026-11-18): every lookup of an external
//! program goes through `kinana_core::executables`, which also searches the
//! trusted install locations an app opened from Finder needs. A new direct
//! `PATH` read or a program started by bare name would bring back the bug
//! where the app could not find `claude`.
use std::fs;
use std::path::{Path, PathBuf};

fn rust_files(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

#[test]
fn only_the_executables_module_reads_path_or_names_a_program() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for directory in ["crates/core/src", "crates/cli/src", "app/src-tauri/src"] {
        rust_files(&workspace.join(directory), &mut files);
    }
    assert!(files.len() > 20, "source tree not found");
    let forbidden = [
        "var_os(\"PATH\")",
        "var(\"PATH\")",
        "split_paths(",
        "Command::new(\"git\")",
        "Command::new(\"claude\")",
        "Command::new(\"codex\")",
        "Command::new(\"cursor\")",
    ];
    let mut found = Vec::new();
    for file in files {
        // Only the module itself, by its full path (review 2026-11-18).
        if file == workspace.join("crates/core/src/executables.rs") {
            continue;
        }
        let text = fs::read_to_string(&file).unwrap();
        for (number, line) in text.lines().enumerate() {
            let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
            for pattern in forbidden {
                if compact.contains(pattern) {
                    found.push(format!("{}:{}: {pattern}", file.display(), number + 1));
                }
            }
            // Any program started by a literal that is not an absolute path.
            if let Some(start) = compact.find("Command::new(") {
                let argument = &compact[start + "Command::new(".len()..];
                let literal = argument.trim_start_matches('r').trim_start_matches('#');
                if literal.starts_with('"') && !literal[1..].starts_with('/') {
                    found.push(format!(
                        "{}:{}: relative program name",
                        file.display(),
                        number + 1
                    ));
                }
            }
            // PATH read by any other route: vars_os()/option_env!/env!.
            if (compact.contains("vars_os()") || compact.contains("vars()"))
                && compact.contains("PATH")
                || compact.contains("option_env!(\"PATH\")")
                || compact.contains("env!(\"PATH\")")
            {
                found.push(format!(
                    "{}:{}: indirect PATH read",
                    file.display(),
                    number + 1
                ));
            }
        }
    }
    assert!(
        found.is_empty(),
        "use kinana_core::executables instead:\n{}",
        found.join("\n")
    );
}
