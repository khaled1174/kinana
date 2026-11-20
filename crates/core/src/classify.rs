//! Classify source text as data, with a deterministic offline fallback.
use crate::{
    BoxConfig, BuildReport, Classification, ClassificationProposal, ClassifiedItem,
    ClassifyOptions, ClassifyReport, Error, Result, SkillMetadata, agent_runner,
    agent_runner::AllowedValues, dimensions, edit, frontmatter, hash, vault::Vault,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
mod input;

struct Target {
    id: String,
    name: String,
    original_description: String,
    raw: String,
    files: Vec<String>,
    metadata: SkillMetadata,
    expected_metadata: Vec<u8>,
    classified_from: &'static str,
}

/// Cancellation is accepted until metadata publication starts. Provider process
/// groups observe this state too; cancellation never becomes keyword fallback.
/// A cancelled run stops before its next skill and saves the skills already
/// completed in one commit (owner, round 8); the skill in flight is dropped.
#[derive(Default)]
pub struct ClassifyControl {
    state: std::sync::atomic::AtomicU8,
    progress: std::sync::Mutex<ClassifyProgress>,
}
#[derive(Clone, Default, serde::Serialize)]
pub struct ClassifyProgress {
    pub total: usize,
    pub completed: usize,
    pub current_id: Option<String>,
}
impl ClassifyControl {
    pub fn progress(&self) -> ClassifyProgress {
        self.progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn cancel(&self) -> bool {
        self.state
            .compare_exchange(
                0,
                1,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_ok()
            || self.is_cancelled()
    }
    pub fn is_cancelled(&self) -> bool {
        self.state.load(std::sync::atomic::Ordering::SeqCst) == 1
    }
    pub fn is_committing(&self) -> bool {
        self.state.load(std::sync::atomic::Ordering::SeqCst) == 2
    }
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(Error::new(
                "CANCELLED",
                "Classification cancelled before publication",
            ))
        } else {
            Ok(())
        }
    }
    /// Publication starts, from a running or a cancelled run: completed
    /// skills are saved either way. Further cancellation is refused.
    fn begin_commit(&self) {
        self.state.store(2, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Select and validate the complete batch before invoking any local provider.
/// Dry runs preview current or keyword classification without probing or starting an agent.
pub fn classify(vault: &Vault, options: &ClassifyOptions) -> Result<ClassifyReport> {
    classify_inner(vault, options, None)
}
pub fn classify_with_control(
    vault: &Vault,
    options: &ClassifyOptions,
    control: &ClassifyControl,
) -> Result<ClassifyReport> {
    classify_inner(vault, options, Some(control))
}
/// Desktop confirmation uses the same core estimate and rechecks it before any
/// provider call. The CLI's existing single-skill confirmation behavior is unchanged.
pub fn classify_confirmed(
    vault: &Vault,
    options: &ClassifyOptions,
    estimate: &crate::TokenEstimate,
    control: &ClassifyControl,
) -> Result<ClassifyReport> {
    if !options.confirmed || options.dry_run {
        return Err(Error::decision(
            "CLASSIFY_CONFIRMATION_REQUIRED",
            "Preview the cost and confirm before invoking the agent",
        ));
    }
    control.check()?;
    let preview = classify_inner(
        vault,
        &ClassifyOptions {
            dry_run: true,
            ..options.clone()
        },
        Some(control),
    )?;
    if &preview.estimated_tokens != estimate {
        return Err(Error::decision(
            "CLASSIFY_ESTIMATE_CHANGED",
            "The classification estimate changed; preview the updated cost and confirm again",
        ));
    }
    classify_inner(vault, options, Some(control))
}

fn classify_inner(
    vault: &Vault,
    options: &ClassifyOptions,
    control: Option<&ClassifyControl>,
) -> Result<ClassifyReport> {
    let _state_guard = crate::state::write_guard(&vault.index_path, options.dry_run)?;
    if let Some(c) = control {
        c.check()?;
    }
    validate_options(options)?;
    let config = vault.config()?;
    validate_categories(&config.categories)?;
    dimensions::validate_lists(&config)?;
    let allowed = AllowedValues::from(&config);
    let existing = vault.records()?;
    let selected: Vec<String> = if options.all || options.pending {
        existing.iter().map(|record| record.id.clone()).collect()
    } else {
        let mut ids = options.ids.clone();
        ids.sort();
        ids
    };
    let mut targets = Vec::with_capacity(selected.len());
    for id in selected {
        crate::vault::validate_id(&id)?;
        if !existing.iter().any(|record| record.id == id) {
            return Err(Error::new(
                "SKILL_NOT_FOUND",
                format!("No skill with ID '{id}' exists in the vault"),
            ));
        }
        let (metadata, expected_metadata) = edit::read_snapshot(vault, &id)?;
        if options.pending && metadata.classification.classified_by != "pending" {
            continue;
        }
        let directory = hash::checked_absolute_path(&vault.root.join("skills").join(&id))?;
        let files = file_names(&directory)?;
        let skill_path = hash::checked_absolute_path(&directory.join("SKILL.md"))?;
        let raw = fs::read_to_string(skill_path)?;
        let manifest = frontmatter::parse_skill(&raw)?;
        let selected_input = input::select(&raw, options.full);
        targets.push(Target {
            id,
            name: manifest.name,
            original_description: manifest.description,
            raw: selected_input.text,
            classified_from: selected_input.classified_from,
            files,
            metadata,
            expected_metadata,
        });
    }

    let estimated_agent_calls = if options.agent == "claude" {
        targets.len()
    } else {
        0
    };
    let estimated_tokens = agent_runner::estimate_tokens(
        &targets
            .iter()
            .map(|target| {
                (
                    target.id.as_str(),
                    target.raw.as_str(),
                    target.files.as_slice(),
                )
            })
            .collect::<Vec<_>>(),
        &allowed,
        &existing,
        estimated_agent_calls,
    )?;
    if !options.dry_run
        && !options.confirmed
        && !targets.is_empty()
        && (options.all || targets.len() > 1)
    {
        return Err(Error::decision(
            "CLASSIFY_CONFIRMATION_REQUIRED",
            format!(
                "Classify {} skills: {} agent calls; estimated input tokens: {} fixed per call, {} fixed total + {} skill text = {} total (retry ceiling {}). Confirm with --yes before any agent is invoked.",
                targets.len(),
                estimated_agent_calls,
                estimated_tokens.fixed_tokens_per_call,
                estimated_tokens.fixed_tokens_total,
                estimated_tokens.skill_tokens_total,
                estimated_tokens.total_input_tokens,
                estimated_tokens.retry_limit_input_tokens
            ),
        ));
    }

    let mut report = ClassifyReport {
        dry_run: options.dry_run,
        selected: targets.len(),
        estimated_agent_calls,
        estimated_tokens,
        agent_calls: 0,
        items: Vec::with_capacity(targets.len()),
        committed: false,
        build: BuildReport {
            dry_run: options.dry_run,
            ..Default::default()
        },
        warnings: Vec::new(),
        stopped: None,
    };
    if options.dry_run && !targets.is_empty() && options.agent == "claude" {
        report.warnings.push(
            "Dry run did not obtain provider results. Items show their current classifications; no agent help probe or model call was made.".into(),
        );
    } else if options.dry_run && !targets.is_empty() && options.agent != "keywords" {
        report.warnings.push(
            "Dry run previews deterministic keyword classification. No agent help probe or model call was made; an agent result may differ.".into(),
        );
    }

    // The commit would refuse a dirty vault anyway; refuse before any model
    // call spends the user's subscription (review, sixth round).
    if !options.dry_run && options.agent != "keywords" && !targets.is_empty() {
        crate::gitlog::ensure_clean(&vault.root)?;
    }
    let mut updates = BTreeMap::new();
    let mut expected = BTreeMap::new();
    if !options.dry_run
        && let Some(c) = control
    {
        *c.progress.lock().unwrap_or_else(|e| e.into_inner()) = ClassifyProgress {
            total: targets.len(),
            ..Default::default()
        };
    }
    let order: Vec<String> = targets.iter().map(|target| target.id.clone()).collect();
    let mut stop: Option<Error> = None;
    for target in targets {
        if let Some(c) = control {
            if !options.dry_run && c.is_cancelled() {
                stop = Some(cancelled());
                break;
            }
            c.check()?;
            if !options.dry_run {
                c.progress
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .current_id = Some(target.id.clone());
            }
        }
        let mut metadata = target.metadata;
        if options.dry_run && options.agent == "claude" {
            report.items.push(ClassifiedItem {
                id: target.id.clone(),
                category: metadata.classification.category.clone(),
                classified_by: metadata.classification.classified_by.clone(),
                proposal_created: false,
                attempts: 0,
                warnings: vec!["Provider result was not obtained during this preview.".into()],
                reason_code: None,
                searched_locations: Vec::new(),
                classified_from: Some(target.classified_from.into()),
                estimated_skill_tokens: agent_runner::estimate_skill_tokens(
                    &target.id,
                    &target.raw,
                    &target.files,
                ),
            });
            expected.insert(target.id.clone(), target.expected_metadata);
            updates.insert(target.id, metadata);
            continue;
        }
        let mut warnings = Vec::new();
        let mut reason_code = None;
        let mut searched_locations = Vec::new();
        let mut attempts = 0;
        let mut provider_proposal = None;
        let mut classified_by = "keywords".to_string();
        if !options.dry_run && options.agent != "keywords" {
            let outcome = agent_runner::classify_controlled(
                vault,
                agent_runner::ClassificationRequest {
                    id: &target.id,
                    raw: &target.raw,
                    files: &target.files,
                    allowed,
                    existing: &existing,
                    requested: &options.agent,
                },
                control,
            )?;
            // Every model invocation counts, also one stopped midway.
            report.agent_calls += outcome.attempts;
            if control.is_some_and(ClassifyControl::is_cancelled) {
                stop = Some(cancelled());
                break;
            }
            if let Some(error) = outcome.stop {
                stop = Some(error);
                break;
            }
            attempts = outcome.attempts;
            warnings = outcome.warnings;
            reason_code = outcome.reason_code;
            searched_locations = outcome.searched_locations;
            classified_by = outcome.classified_by;
            if let Some(proposal) = outcome.proposal {
                match agent_runner::validate_proposal(&proposal, &allowed) {
                    Ok(()) => provider_proposal = Some(proposal),
                    Err(error) => {
                        warnings.push(format!(
                            "Agent proposal failed validation; using keyword classification: {error}"
                        ));
                        reason_code = Some("INVALID_AGENT_PROPOSAL".into());
                    }
                }
            }
        }
        let proposal_created = provider_proposal.is_some();
        if let Some(proposal) = provider_proposal {
            apply_proposal(&mut metadata, proposal, &classified_by, &config);
            metadata.classification.classified_from = Some(target.classified_from.into());
        } else {
            let mut overlaps = metadata.classification.overlaps_with.clone();
            if let Some(proposal) = &metadata.proposal {
                overlaps.extend(proposal.overlaps_with.iter().cloned());
            }
            overlaps.sort();
            overlaps.dedup();
            let previous = std::mem::take(&mut metadata.classification);
            metadata.classification =
                keyword_classification(&target.name, &target.original_description, &config)?;
            metadata.classification.overlaps_with = overlaps;
            // Keywords only have evidence that a technology is named, never
            // that one is absent, and no evidence at all about a stage.
            let mut stack = previous.stack;
            stack.extend(std::mem::take(&mut metadata.classification.stack));
            metadata.classification.stack = dimensions::ordered(&stack, &config.stacks);
            metadata.classification.stage = previous.stage;
            // A fresh offline classification supersedes any unreviewed proposal.
            // The approved active description and its timestamp remain intact.
            metadata.proposal = None;
            metadata.proposal_source_changed = None;
        }
        report.items.push(ClassifiedItem {
            id: target.id.clone(),
            category: metadata.classification.category.clone(),
            classified_by: metadata.classification.classified_by.clone(),
            proposal_created,
            attempts,
            warnings,
            reason_code: if proposal_created { None } else { reason_code },
            searched_locations,
            classified_from: metadata.classification.classified_from.clone(),
            estimated_skill_tokens: if estimated_agent_calls > 0 {
                agent_runner::estimate_skill_tokens(&target.id, &target.raw, &target.files)
            } else {
                0
            },
        });
        expected.insert(target.id.clone(), target.expected_metadata);
        updates.insert(target.id, metadata);
        if !options.dry_run
            && let Some(c) = control
        {
            let mut progress = c.progress.lock().unwrap_or_else(|e| e.into_inner());
            progress.completed += 1;
            progress.current_id = None;
        }
    }
    if let Some(error) = stop {
        report.warnings.push(format!(
            "{}: {} Classified {} of {}; {} not classified.",
            error.code,
            error.message,
            updates.len(),
            order.len(),
            order.len() - updates.len()
        ));
        report.stopped = Some(crate::ClassifyStop {
            reason_code: error.code,
            message: error.message,
            classified: updates.len(),
            remaining: order
                .iter()
                .filter(|id| !updates.contains_key(*id))
                .cloned()
                .collect(),
        });
    }
    if let Some(c) = control {
        if options.dry_run {
            c.check()?;
        } else if !updates.is_empty() {
            c.begin_commit();
        }
    }
    if !updates.is_empty() {
        let result = edit::apply_checked(
            vault,
            &updates,
            &expected,
            &format!("Classify {} skills", updates.len()),
            options.dry_run,
        )?;
        report.committed = result.committed;
        report.build = result.built;
    }
    Ok(report)
}

fn cancelled() -> Error {
    Error::new(
        "CANCELLED",
        "Classification was cancelled; completed skills are saved.",
    )
}

fn validate_options(options: &ClassifyOptions) -> Result<()> {
    if options.full && (options.ids.len() != 1 || options.all || options.pending) {
        return Err(Error::new(
            "INVALID_ARGUMENTS",
            "--full requires exactly one explicit skill ID and cannot be combined with --all or --pending",
        ));
    }
    let modes = usize::from(!options.ids.is_empty())
        + usize::from(options.all)
        + usize::from(options.pending);
    if modes != 1 {
        return Err(Error::new(
            "INVALID_SELECTION",
            "Choose exactly one of skill IDs, --all, or --pending",
        ));
    }
    if !matches!(
        options.agent.as_str(),
        "auto" | "keywords" | "claude" | "codex"
    ) {
        return Err(Error::new(
            "INVALID_AGENT",
            format!("Unsupported classification agent: {}", options.agent),
        ));
    }
    let mut ids = BTreeSet::new();
    for id in &options.ids {
        crate::vault::validate_id(id)?;
        if !ids.insert(id) {
            return Err(Error::new(
                "INVALID_SELECTION",
                format!("Skill ID was selected more than once: {id}"),
            ));
        }
    }
    Ok(())
}

fn validate_categories(categories: &[String]) -> Result<()> {
    let mut seen = BTreeSet::new();
    if categories.is_empty()
        || categories
            .iter()
            .any(|category| category.trim().is_empty() || !seen.insert(category))
    {
        return Err(Error::new(
            "INVALID_CATEGORIES",
            "box.toml must contain a nonempty list of distinct, nonblank categories",
        ));
    }
    Ok(())
}

fn file_names(root: &Path) -> Result<Vec<String>> {
    fn visit(root: &Path, directory: &Path, files: &mut Vec<String>) -> Result<()> {
        for entry in fs::read_dir(directory)? {
            let path = hash::checked_absolute_path(&entry?.path())?;
            let metadata = hash::checked_metadata(&path)?;
            if metadata.is_dir() {
                visit(root, &path, files)?;
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|error| Error::new("INVALID_PATH", error.to_string()))?;
                let name = relative.to_str().ok_or_else(|| {
                    Error::new("INVALID_PATH", "Skill filenames must be valid Unicode")
                })?;
                files.push(name.replace('\\', "/"));
            } else {
                return Err(Error::new(
                    "UNSUPPORTED_FILE",
                    format!(
                        "Skill payload contains a non-regular entry: {}",
                        path.display()
                    ),
                ));
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    visit(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

fn apply_proposal(
    metadata: &mut SkillMetadata,
    proposal: ClassificationProposal,
    provider: &str,
    config: &BoxConfig,
) {
    metadata.classification = Classification {
        category: proposal.category.clone(),
        stack: dimensions::ordered(&proposal.stack, &config.stacks),
        stage: dimensions::ordered(&proposal.stage, &config.stages),
        tags: proposal.tags.clone(),
        role: proposal.role.clone(),
        when_to_use: proposal.when_to_use.clone(),
        when_not: proposal.when_not.clone(),
        classified_by: provider.into(),
        classified_from: None,
        overlaps_with: proposal.overlaps_with.clone(),
    };
    // The proposal has its own description slot. Never replace an approved
    // description with a new proposal before the next explicit review.
    metadata.proposal = Some(proposal);
    metadata.proposal_source_changed = None;
}

fn normalized(text: &str) -> String {
    text.to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn has_term(text: &str, term: &str) -> bool {
    text.contains(&format!(" {} ", normalized(term)))
}

/// Aliases that name a default stack explicitly. `next` and `node` alone are
/// ordinary words, so only their unambiguous spellings count. Text is already
/// lowercased with punctuation turned into spaces: `Next.js` is `next js`.
const STACK_NAMES: &[(&str, &[&str])] = &[
    ("react", &["react", "reactjs", "react js"]),
    ("next", &["nextjs", "next js"]),
    ("expo", &["expo"]),
    ("react-native", &["react native", "reactnative"]),
    ("flutter", &["flutter"]),
    ("swift", &["swift", "swiftui"]),
    ("rust", &["rust"]),
    ("python", &["python", "python3"]),
    ("node", &["nodejs", "node js"]),
    ("docker", &["docker", "dockerfile"]),
];

/// Configured stacks named explicitly in the text, in list order. Longer
/// phrases consume their words first, so "React Native" is not also "react".
fn named_stacks(name: &str, description: &str, stacks: &[String]) -> Vec<String> {
    let text = normalized(&format!("{name} {description}"));
    let words: Vec<&str> = text.split(' ').filter(|word| !word.is_empty()).collect();
    let mut phrases: Vec<(Vec<String>, &String)> = Vec::new();
    for stack in stacks {
        match STACK_NAMES.iter().find(|(known, _)| known == stack) {
            Some((_, aliases)) => {
                for alias in *aliases {
                    phrases.push((alias.split(' ').map(str::to_owned).collect(), stack));
                }
            }
            // A value the user added matches its own name as a whole phrase.
            None => {
                let own: Vec<String> = normalized(stack)
                    .split(' ')
                    .filter(|word| !word.is_empty())
                    .map(str::to_owned)
                    .collect();
                if !own.is_empty() {
                    phrases.push((own, stack));
                }
            }
        }
    }
    phrases.sort_by_key(|(phrase, _)| std::cmp::Reverse(phrase.len()));
    let mut consumed = vec![false; words.len()];
    let mut found = Vec::new();
    for (phrase, stack) in &phrases {
        let mut start = 0;
        while start + phrase.len() <= words.len() {
            let range = start..start + phrase.len();
            if consumed[range.clone()].iter().all(|used| !used)
                && words[range.clone()]
                    .iter()
                    .zip(phrase)
                    .all(|(word, expected)| word == expected)
            {
                consumed[range].iter_mut().for_each(|used| *used = true);
                found.push((*stack).clone());
                start += phrase.len();
            } else {
                start += 1;
            }
        }
    }
    dimensions::ordered(&found, stacks)
}

fn keyword_classification(
    name: &str,
    description: &str,
    config: &BoxConfig,
) -> Result<Classification> {
    let categories = &config.categories;
    let text = format!(" {} ", normalized(&format!("{name} {description}")));
    let mut best: Option<(&str, Vec<String>)> = None;
    for &(category, terms) in KEYWORDS {
        if !categories
            .iter()
            .any(|allowed| allowed.as_str() == category)
        {
            continue;
        }
        let mut matches: Vec<String> = terms
            .iter()
            .filter(|term| has_term(&text, term))
            .map(|term| (*term).to_string())
            .collect();
        matches.sort();
        matches.dedup();
        if !matches.is_empty()
            && best
                .as_ref()
                .is_none_or(|(_, previous)| matches.len() > previous.len())
        {
            best = Some((category, matches));
        }
    }
    // A user category can match its complete slug as a phrase. Splitting the
    // phrase into individual words would turn generic words into guessed labels.
    for category in categories {
        if !KEYWORDS
            .iter()
            .any(|(known, _)| *known == category.as_str())
            && !normalized(category).is_empty()
            && has_term(&text, category)
            && best.is_none()
        {
            best = Some((category, vec![category.clone()]));
        }
    }
    let (category, tags) = match best {
        Some(found) => found,
        None if categories.iter().any(|category| category == "other") => ("other", Vec::new()),
        None => {
            return Err(Error::decision(
                "CATEGORY_FALLBACK_REQUIRED",
                format!(
                    "No configured category matches '{name}'. Add 'other' to box.toml or choose a category manually before keyword classification."
                ),
            ));
        }
    };
    Ok(Classification {
        category: category.into(),
        stack: named_stacks(name, description, &config.stacks),
        stage: vec![],
        tags,
        role: description.into(),
        when_to_use: String::new(),
        when_not: String::new(),
        classified_by: "keywords".into(),
        classified_from: None,
        overlaps_with: vec![],
    })
}

// Dictionary order also defines the stable tie break, independently of the
// configured category order. Matching only name/original description excludes
// instructions and misleading keywords in the body, filenames, and proposals.
const KEYWORDS: &[(&str, &[&str])] = &[
    (
        "mobile",
        &[
            "mobile",
            "ios",
            "android",
            "swift",
            "swiftui",
            "kotlin",
            "flutter",
            "react native",
            "xcode",
            "ipad",
            "iphone",
        ],
    ),
    (
        "web-frontend",
        &[
            "frontend",
            "front end",
            "react",
            "nextjs",
            "next js",
            "vue",
            "svelte",
            "html",
            "css",
            "browser",
            "web",
            "javascript",
            "typescript",
            "angular",
            "tailwind",
        ],
    ),
    (
        "backend-api",
        &[
            "backend",
            "back end",
            "api",
            "rest",
            "graphql",
            "endpoint",
            "endpoints",
            "server",
            "microservice",
            "grpc",
            "fastapi",
            "django",
            "express",
        ],
    ),
    (
        "data-db",
        &[
            "database",
            "databases",
            "sql",
            "postgres",
            "postgresql",
            "sqlite",
            "mysql",
            "mongodb",
            "redis",
            "data",
            "analytics",
            "etl",
            "warehouse",
            "pandas",
        ],
    ),
    (
        "devops",
        &[
            "devops",
            "deployment",
            "deploy",
            "docker",
            "kubernetes",
            "terraform",
            "infrastructure",
            "ci cd",
            "pipeline",
            "pipelines",
            "aws",
            "azure",
            "nginx",
            "cloud",
        ],
    ),
    (
        "testing-qa",
        &[
            "testing",
            "test",
            "tests",
            "qa",
            "quality assurance",
            "pytest",
            "jest",
            "vitest",
            "playwright",
            "cypress",
            "unit test",
            "regression",
        ],
    ),
    (
        "planning-specs",
        &[
            "planning",
            "plan",
            "spec",
            "specs",
            "specification",
            "requirements",
            "roadmap",
            "architecture",
            "architectural",
            "prd",
            "design doc",
        ],
    ),
    (
        "design-ui",
        &[
            "design",
            "ui",
            "ux",
            "figma",
            "wireframe",
            "prototype",
            "prototyping",
            "usability",
            "accessibility",
            "visual design",
            "user interface",
        ],
    ),
    (
        "docs-writing",
        &[
            "documentation",
            "docs",
            "writing",
            "technical writing",
            "readme",
            "copywriting",
            "editing",
            "proofreading",
            "markdown",
            "document",
        ],
    ),
    (
        "content-media",
        &[
            "content",
            "media",
            "video",
            "audio",
            "image",
            "images",
            "animation",
            "podcast",
            "social",
            "cinematic",
            "storyboard",
            "render",
        ],
    ),
    (
        "automation-agents",
        &[
            "automation",
            "automate",
            "agent",
            "agents",
            "workflow",
            "workflows",
            "mcp",
            "tool",
            "tools",
            "bot",
            "bots",
            "orchestration",
        ],
    ),
    (
        "security",
        &[
            "security",
            "secure",
            "vulnerability",
            "vulnerabilities",
            "threat",
            "auth",
            "authentication",
            "authorization",
            "oauth",
            "pentest",
            "penetration",
            "encryption",
            "cryptography",
            "secrets",
        ],
    ),
    ("other", &["general", "miscellaneous"]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proposal_preserves_approved_description_until_review() {
        let mut metadata: SkillMetadata = toml::from_str(
            "id='fixture'\ncontent_hash='original-hash'\nadded_at='original-date'\n[description]\noriginal='Original text'\nimproved='Previously approved text'\nactive='improved'\napproved_at='approval-date'\n",
        ).unwrap();
        let proposal = ClassificationProposal {
            category: "security".into(),
            stack: vec![],
            stage: vec![],
            tags: vec!["review".into()],
            role: "New role".into(),
            when_to_use: "Before release".into(),
            when_not: "During planning".into(),
            improved_description: "Unapproved new text".into(),
            overlaps_with: vec!["api-peer".into()],
            confidence: 0.8,
        };
        apply_proposal(
            &mut metadata,
            proposal.clone(),
            "claude",
            &BoxConfig::default(),
        );
        assert_eq!(metadata.classification.category, "security");
        assert_eq!(metadata.classification.classified_by, "claude");
        assert_eq!(metadata.classification.overlaps_with, ["api-peer"]);
        assert_eq!(metadata.proposal, Some(proposal));
        assert_eq!(
            metadata.description.improved.as_deref(),
            Some("Previously approved text")
        );
        assert_eq!(metadata.description.active, "improved");
        assert_eq!(
            metadata.description.approved_at.as_deref(),
            Some("approval-date")
        );
        assert_eq!(metadata.content_hash, "original-hash");
    }
}
