//! Read fixed project manifests, never scripts, to explain local suggestions.
use crate::{
    Error, ProjectSignal, Result, StackSignal, SuggestMatch, SuggestReport, SuggestedBundle,
    bundles, index, vault::Vault,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Read,
    path::Path,
};

const MAX_MANIFEST: u64 = 1024 * 1024;

pub fn suggest(vault: &Vault, project: &Path) -> Result<SuggestReport> {
    let project = index::validate_path(project)?;
    if !fs::symlink_metadata(&project)?.is_dir() {
        return Err(Error::new(
            "INVALID_PROJECT",
            "Project must be an existing real directory",
        ));
    }
    let allowed_stacks = vault.config()?.stacks;
    let records = vault.records()?;
    let mut report = SuggestReport {
        project: project.clone(),
        categories: vec![],
        stacks: vec![],
        signals: vec![],
        stack_signals: vec![],
        bundles: vec![],
        skills: vec![],
        matches: vec![],
        warnings: vec![],
    };
    if let Some(text) = manifest(&project.join("package.json"), &mut report.warnings) {
        match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(value) if value.is_object() => {
                let has = |name: &str| {
                    [
                        "dependencies",
                        "devDependencies",
                        "peerDependencies",
                        "optionalDependencies",
                    ]
                    .iter()
                    .any(|section| {
                        value[*section].as_object().is_some_and(|deps| {
                            deps.get(name).is_some_and(serde_json::Value::is_string)
                        })
                    })
                };
                // Any package.json is a Node.js project; its dependencies
                // name the frameworks. Values outside the list are skipped.
                stack_signal(
                    &mut report,
                    &allowed_stacks,
                    "package.json",
                    "node",
                    "package.json manifest",
                );
                for dependency in ["react", "next", "expo", "react-native"] {
                    if has(dependency) {
                        stack_signal(
                            &mut report,
                            &allowed_stacks,
                            "package.json",
                            dependency,
                            &format!("{dependency} dependency"),
                        );
                    }
                }
                let mobile = has("expo") || has("react-native");
                if mobile {
                    signal(
                        &mut report,
                        "package.json",
                        "mobile",
                        "Expo or React Native dependency",
                    );
                }
                if has("next") || (has("react") && !mobile) {
                    signal(
                        &mut report,
                        "package.json",
                        "web-frontend",
                        "React or Next.js dependency",
                    );
                }
            }
            _ => report
                .warnings
                .push("Ignored package.json: expected a valid JSON object".into()),
        }
    }
    if let Some(text) = manifest(&project.join("pubspec.yaml"), &mut report.warnings) {
        match yaml_rust2::YamlLoader::load_from_str(&text) {
            Ok(docs) if docs.len() == 1 && docs[0].as_hash().is_some() => {
                let doc = &docs[0];
                if !matches!(doc["dependencies"]["flutter"], yaml_rust2::Yaml::BadValue)
                    || !matches!(doc["flutter"], yaml_rust2::Yaml::BadValue)
                {
                    signal(&mut report, "pubspec.yaml", "mobile", "Flutter manifest");
                    stack_signal(
                        &mut report,
                        &allowed_stacks,
                        "pubspec.yaml",
                        "flutter",
                        "Flutter manifest",
                    );
                }
            }
            _ => report
                .warnings
                .push("Ignored pubspec.yaml: expected one valid YAML mapping".into()),
        }
    }
    for (name, keys, reason, stack) in [
        (
            "Cargo.toml",
            &["package", "workspace"][..],
            "Rust package or workspace",
            "rust",
        ),
        (
            "pyproject.toml",
            &["project", "build-system"][..],
            "Python project or build system",
            "python",
        ),
    ] {
        if let Some(text) = manifest(&project.join(name), &mut report.warnings) {
            match toml::from_str::<toml::Table>(&text) {
                Ok(table) => {
                    let poetry = name == "pyproject.toml"
                        && table
                            .get("tool")
                            .and_then(|v| v.get("poetry"))
                            .is_some_and(toml::Value::is_table);
                    if poetry
                        || keys
                            .iter()
                            .any(|key| table.get(*key).is_some_and(toml::Value::is_table))
                    {
                        signal(&mut report, name, "backend-api", reason);
                        stack_signal(&mut report, &allowed_stacks, name, stack, reason);
                    }
                }
                Err(_) => report
                    .warnings
                    .push(format!("Ignored {name}: invalid TOML")),
            }
        }
    }
    if manifest(&project.join("Dockerfile"), &mut report.warnings).is_some() {
        signal(&mut report, "Dockerfile", "devops", "Docker build file");
        stack_signal(
            &mut report,
            &allowed_stacks,
            "Dockerfile",
            "docker",
            "Docker build file",
        );
    }
    swift_markers(&project, &allowed_stacks, &mut report);
    for name in ["ios", "android"] {
        let path = project.join(name);
        match fs::symlink_metadata(&path) {
            Ok(m) if m.file_type().is_symlink() => report
                .warnings
                .push(format!("Ignored {name}: symbolic link")),
            Ok(m) if m.is_dir() => signal(
                &mut report,
                name,
                "mobile",
                "Native mobile project directory",
            ),
            Ok(_) => report
                .warnings
                .push(format!("Ignored {name}: expected a directory")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => report.warnings.push(format!("Ignored {name}: {e}")),
        }
    }
    let categories = report
        .signals
        .iter()
        .map(|s| s.category.clone())
        .collect::<BTreeSet<_>>();
    report.categories = categories.iter().cloned().collect();
    let by_id = records
        .iter()
        .map(|r| (r.id.as_str(), r))
        .collect::<BTreeMap<_, _>>();
    for bundle in bundles::list(vault)? {
        let matched_categories = bundle
            .skills
            .iter()
            .filter(|s| s.role == "primary")
            .filter_map(|s| by_id.get(s.id.as_str()))
            .map(|s| s.category.clone())
            .filter(|c| categories.contains(c))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if categories.is_empty() || !matched_categories.is_empty() {
            report.bundles.push(SuggestedBundle {
                match_count: matched_categories.len(),
                matched_categories,
                bundle,
            });
        }
    }
    report.bundles.sort_by(|a, b| {
        b.match_count
            .cmp(&a.match_count)
            .then_with(|| a.bundle.name.cmp(&b.bundle.name))
    });
    let mut detected = BTreeSet::new();
    for signal in &report.stack_signals {
        detected.insert(signal.stack.as_str());
    }
    // Configured-list order, like every stored `stack`.
    report.stacks = allowed_stacks
        .iter()
        .filter(|stack| detected.contains(stack.as_str()))
        .cloned()
        .collect();
    // A skill is suggested for a detected stack or a detected domain. Stack
    // matches come first (more matching stacks, then the domain as well),
    // then domain-only matches; ties keep id order.
    let mut matched = records
        .into_iter()
        .filter_map(|record| {
            let matched_stacks = report
                .stacks
                .iter()
                .filter(|stack| record.stack.contains(stack))
                .cloned()
                .collect::<Vec<_>>();
            let matched_category = categories.contains(&record.category);
            (!matched_stacks.is_empty() || matched_category).then_some((
                SuggestMatch {
                    id: record.id.clone(),
                    matched_stacks,
                    matched_category,
                },
                record,
            ))
        })
        .collect::<Vec<_>>();
    matched.sort_by(|(a, _), (b, _)| {
        b.matched_stacks
            .len()
            .cmp(&a.matched_stacks.len())
            .then_with(|| b.matched_category.cmp(&a.matched_category))
            .then_with(|| a.id.cmp(&b.id))
    });
    (report.matches, report.skills) = matched.into_iter().unzip();
    Ok(report)
}

/// Record a manifest fact as a stack value, only if the user's list has it.
fn stack_signal(
    report: &mut SuggestReport,
    allowed: &[String],
    name: &str,
    stack: &str,
    reason: &str,
) {
    if allowed.iter().any(|value| value == stack) {
        report.stack_signals.push(StackSignal {
            path: report.project.join(name),
            stack: stack.into(),
            reason: reason.into(),
        });
    }
}

/// Swift package or Xcode project markers. Only the names and kinds of the
/// project root's own entries are read; nothing is opened or followed.
fn swift_markers(project: &Path, allowed: &[String], report: &mut SuggestReport) {
    let mut found = Vec::new();
    match fs::read_dir(project) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let Ok(name) = entry.file_name().into_string() else {
                    continue;
                };
                let xcode = name.ends_with(".xcodeproj") || name.ends_with(".xcworkspace");
                if !xcode && name != "Package.swift" {
                    continue;
                }
                match fs::symlink_metadata(entry.path()) {
                    Ok(m) if m.file_type().is_symlink() => report
                        .warnings
                        .push(format!("Ignored {name}: symbolic link")),
                    Ok(m) if xcode && m.is_dir() => found.push((name, "Xcode project")),
                    Ok(m) if !xcode && m.is_file() => found.push((name, "Swift package manifest")),
                    _ => {}
                }
            }
        }
        Err(error) => report
            .warnings
            .push(format!("Ignored Swift project markers: {error}")),
    }
    found.sort();
    for (name, reason) in found {
        stack_signal(report, allowed, &name, "swift", reason);
    }
}

fn signal(report: &mut SuggestReport, name: &str, category: &str, reason: &str) {
    report.signals.push(ProjectSignal {
        path: report.project.join(name),
        category: category.into(),
        reason: reason.into(),
    });
}

fn manifest(path: &Path, warnings: &mut Vec<String>) -> Option<String> {
    match read_manifest(path) {
        Ok(value) => value,
        Err(error) => {
            warnings.push(format!("Ignored {}: {error}", path.display()));
            None
        }
    }
}

fn read_manifest(path: &Path) -> Result<Option<String>> {
    index::validate_path(path)?;
    let before = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if !before.is_file() || before.len() > MAX_MANIFEST {
        return Err(Error::new(
            "INVALID_PROJECT_MANIFEST",
            "Expected a regular file no larger than 1 MiB",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        #[cfg(target_os = "macos")]
        options.custom_flags(0x100 | 0x4); // O_NOFOLLOW | O_NONBLOCK
        #[cfg(target_os = "linux")]
        options.custom_flags(0x20000 | 0x800);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options.open(path)?;
    let opened = file.metadata()?;
    if !opened.is_file() || opened.len() > MAX_MANIFEST || !same_file(&before, &opened) {
        return Err(Error::new(
            "PROJECT_CHANGED",
            "Project manifest changed before reading",
        ));
    }
    let mut data = Vec::new();
    file.take(MAX_MANIFEST + 1).read_to_end(&mut data)?;
    if data.len() as u64 > MAX_MANIFEST {
        return Err(Error::new(
            "INVALID_PROJECT_MANIFEST",
            "Manifest exceeds 1 MiB",
        ));
    }
    String::from_utf8(data)
        .map(Some)
        .map_err(|_| Error::new("INVALID_PROJECT_MANIFEST", "Manifest must be UTF-8"))
}

#[cfg(unix)]
fn same_file(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev() && a.ino() == b.ino()
}
#[cfg(not(unix))]
fn same_file(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    a.created()
        .ok()
        .zip(b.created().ok())
        .is_some_and(|(a, b)| a == b)
}
