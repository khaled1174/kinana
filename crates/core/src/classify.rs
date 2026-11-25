//! Classify source text as data with the selected provider or explicit keywords.
use crate::{
    BoxConfig, BuildReport, Classification, ClassificationProposal, ClassifiedItem,
    ClassifyOptions, ClassifyReport, Error, Result, SkillMetadata, SkippedClassification,
    agent_runner, agent_runner::AllowedValues, dimensions, edit, frontmatter, hash, vault::Vault,
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
    pub skipped: usize,
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
    classify_inner(vault, options, None, PromptTemplate::product(), None)
}
pub fn classify_with_control(
    vault: &Vault,
    options: &ClassifyOptions,
    control: &ClassifyControl,
) -> Result<ClassifyReport> {
    classify_inner(
        vault,
        options,
        Some(control),
        PromptTemplate::product(),
        None,
    )
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
    classify_inner(
        vault,
        options,
        Some(control),
        PromptTemplate::product(),
        Some(estimate),
    )
}

/// Which instructions a provider classification sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptTemplate {
    /// Template 2: fixed instructions, general rule and the taxonomy guide.
    /// Evaluation arm B; the product sends it only after the gate passes.
    Definitions,
    /// Template 1: the allowed values by name only. Evaluation arm A, and
    /// what the product sends until then.
    NamesOnly,
}

impl PromptTemplate {
    /// The template of [`agent_runner::PRODUCT_PROMPT_VERSION`].
    pub fn product() -> Self {
        if agent_runner::PRODUCT_PROMPT_VERSION == agent_runner::PROMPT_VERSION {
            Self::Definitions
        } else {
            Self::NamesOnly
        }
    }
}

/// Classification with an explicit template, for the evaluation tool. The
/// product always uses [`PromptTemplate::product`].
pub fn classify_with_template(
    vault: &Vault,
    options: &ClassifyOptions,
    template: PromptTemplate,
) -> Result<ClassifyReport> {
    classify_inner(vault, options, None, template, None)
}

fn classify_inner(
    vault: &Vault,
    options: &ClassifyOptions,
    control: Option<&ClassifyControl>,
    template: PromptTemplate,
    expected_estimate: Option<&crate::TokenEstimate>,
) -> Result<ClassifyReport> {
    // Codex cannot prove it reads nothing: refused before the state lock,
    // any read or any call, never a silent switch to keywords (owner
    // decision, 2026-11-18).
    if options.agent == "codex" {
        return Err(agent_runner::codex_unsupported());
    }
    let _state_guard = crate::state::write_guard(&vault.index_path, options.dry_run)?;
    if let Some(c) = control {
        c.check()?;
    }
    validate_options(options)?;
    let config = vault.config()?;
    let uses_claude_settings = matches!(options.agent.as_str(), "claude" | "auto");
    let mut claude = crate::ClaudeSettings::default();
    if uses_claude_settings {
        claude = config.classification_claude.clone();
        if let Some(model) = &options.claude_model {
            claude.model.clone_from(model);
        }
        if let Some(effort) = &options.claude_effort {
            claude.effort.clone_from(effort);
        }
        agent_runner::validate_claude_settings(&claude)?;
    } else if options.claude_model.is_some()
        || options.claude_effort.is_some()
        || options.claude_events
    {
        return Err(Error::new(
            "INVALID_CLAUDE_SETTINGS",
            "--model, --effort and --claude-events require --agent claude or auto",
        ));
    }
    if options.agent == "pi" && !config.classification_pi.enabled {
        return Err(agent_runner::pi_disabled());
    }
    // A run that may use Pi checks its settings first, also when box.toml
    // was edited by hand: no preview, confirmation or call with an empty
    // provider or model (owner, 2026-11-18).
    if matches!(options.agent.as_str(), "pi" | "auto") && config.classification_pi.enabled {
        agent_runner::pi::validate_settings(&config.classification_pi)?;
    }
    validate_categories(&config.categories)?;
    dimensions::validate_lists(&config)?;
    let allowed = AllowedValues::from(&config);
    // Definitions are checked before any provider call; keywords ignore them.
    let guide = match template {
        PromptTemplate::Definitions => {
            let in_effect = crate::taxonomy::in_effect(&config);
            if options.agent != "keywords" {
                crate::taxonomy::validate(&in_effect)?;
            }
            Some(crate::taxonomy::guide(&in_effect))
        }
        PromptTemplate::NamesOnly => None,
    };
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

    let uses_claude = matches!(options.agent.as_str(), "claude" | "auto" | "pi");
    let estimated_agent_calls = if uses_claude { targets.len() } else { 0 };
    let providers = providers(
        &options.agent,
        estimated_agent_calls,
        config.classification_pi.enabled,
    );
    let mut estimated_tokens = agent_runner::estimate_tokens(
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
        guide.as_deref(),
        options.agent == "pi",
    )?;
    estimated_tokens.claude = claude.clone();
    if expected_estimate.is_some_and(|expected| expected != &estimated_tokens) {
        return Err(Error::decision(
            "CLASSIFY_ESTIMATE_CHANGED",
            "Classification settings or the estimate changed; preview and confirm again",
        ));
    }
    // Pi under `auto` needs the user's explicit confirmation before its
    // first call (owner, 2026-11-18). A confirmation is checked before any
    // call: one for other settings or skills is refused, not ignored.
    let pi_possible = options.agent == "auto" && config.classification_pi.enabled;
    let target_ids: Vec<&str> = targets.iter().map(|t| t.id.as_str()).collect();
    // The token binds what Pi would receive (owner, 2026-11-19, item 2).
    let digests: BTreeMap<String, String> = targets
        .iter()
        .map(|t| (t.id.clone(), pi_content_digest(&t.raw, &t.files)))
        .collect();
    let pi_skills = |ids: &[&str]| -> Vec<(String, String)> {
        ids.iter()
            .map(|id| {
                (
                    (*id).to_owned(),
                    digests.get(*id).cloned().unwrap_or_default(),
                )
            })
            .collect()
    };
    let pi_confirmed = match &options.pi_confirmation {
        Some(_) if !pi_possible => {
            return Err(Error::new(
                "INVALID_ARGUMENTS",
                "--confirm confirms Pi for `auto` with Pi enabled; it does not apply to this run",
            ));
        }
        Some(token)
            if *token
                == pi_confirmation(
                    &config.classification_pi,
                    &claude,
                    options.full,
                    &pi_skills(&target_ids),
                )
                .token =>
        {
            true
        }
        Some(_) => {
            return Err(Error::decision(
                "PI_CONFIRMATION_CHANGED",
                "The Pi settings, the selected skills, their content or --full changed since the Pi confirmation; review the new preview and confirm again",
            ));
        }
        None => false,
    };
    if !options.dry_run
        && !options.confirmed
        && !targets.is_empty()
        && (options.all || targets.len() > 1)
    {
        return Err(Error::decision(
            "CLASSIFY_CONFIRMATION_REQUIRED",
            format!(
                "Classify {} skills: {} agent calls{}; estimated input tokens: {} fixed per call, {} fixed total + {} skill text = {} total (retry ceiling {}). Confirm with --yes before any agent is invoked.",
                targets.len(),
                estimated_agent_calls,
                if options.agent == "auto" && config.classification_pi.enabled {
                    format!(
                        " (Pi, if Claude fails, may add up to {} with one retry each; its tokens are not in this estimate)",
                        estimated_agent_calls * 2
                    )
                } else {
                    String::new()
                },
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
        skipped: Vec::new(),
        claude_events: Vec::new(),
        committed: false,
        build: BuildReport {
            dry_run: options.dry_run,
            ..Default::default()
        },
        warnings: Vec::new(),
        stopped: None,
        providers: providers.clone(),
        auto_approved: None,
        pi_confirmation: (options.dry_run && pi_possible && !target_ids.is_empty()).then(|| {
            pi_confirmation(
                &config.classification_pi,
                &claude,
                options.full,
                &pi_skills(&target_ids),
            )
        }),
    };
    if options.dry_run && !targets.is_empty() && uses_claude {
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
    let mut stop_providers = Vec::new();
    // `auto`: Claude stopped the run (usage limit, credit, login).
    let mut claude_stopped: Option<(String, String)> = None;
    // Distinguish an unavailable Claude from one that disappeared mid-run.
    let mut claude_seen = false;
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
        if options.dry_run && uses_claude {
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
        let mut prompt_version = None;
        if !options.dry_run && options.agent != "keywords" {
            let request = |requested: &'static str| agent_runner::ClassificationRequest {
                id: &target.id,
                raw: &target.raw,
                files: &target.files,
                allowed,
                existing: &existing,
                requested,
                guide: guide.as_deref(),
                pi: Some(&config.classification_pi),
                claude: Some(&claude),
                claude_events: options.claude_events,
            };
            // `auto` may try enabled Pi after explicit confirmation.
            let mut failures: Vec<(&str, String, String)> = Vec::new();
            let mut chosen: Option<agent_runner::AgentOutcome> = None;
            let mut skill_attempts = 0;
            if matches!(options.agent.as_str(), "claude" | "auto") {
                if let Some((code, message)) = &claude_stopped {
                    // Claude stopped the run earlier (limit or login): every
                    // later skill would stop the same way.
                    failures.push(("claude", code.clone(), message.clone()));
                } else {
                    // A cancellation that lands just before the call stops the
                    // run like any other: completed skills are still saved.
                    let mut outcome = match agent_runner::classify_controlled(
                        vault,
                        request("claude"),
                        control,
                    ) {
                        Err(error) if error.code == "CANCELLED" => {
                            stop = Some(cancelled());
                            break;
                        }
                        result => result?,
                    };
                    // Every model invocation counts, also one stopped midway.
                    report.agent_calls += outcome.attempts;
                    skill_attempts += outcome.attempts;
                    report.claude_events.append(&mut outcome.claude_events);
                    if control.is_some_and(ClassifyControl::is_cancelled) {
                        stop = Some(cancelled());
                        break;
                    }
                    let not_here = outcome.stop.is_none()
                        && outcome
                            .reason_code
                            .as_deref()
                            .is_some_and(|code| CLAUDE_ABSENT.contains(&code));
                    claude_seen |= !not_here;
                    match provider_result(outcome, &allowed) {
                        ProviderResult::Proposal(outcome) => chosen = Some(outcome),
                        ProviderResult::Stopped(error) if options.agent == "claude" => {
                            // Named Claude at a limit, out of credit or signed
                            // out: the run stops with that code and asks
                            // (owner, 2026-11-18); completed skills are saved.
                            stop_providers = failed_provider(&providers, "claude", &error.code);
                            stop = Some(error);
                            break;
                        }
                        ProviderResult::Stopped(error) => {
                            failures.push(("claude", error.code.clone(), error.message.clone()));
                            claude_stopped = Some((error.code, error.message));
                        }
                        ProviderResult::Failed(outcome) => {
                            let (code, message) = failure(&outcome);
                            failures.push(("claude", code, message));
                        }
                    }
                }
            }
            let try_pi = chosen.is_none()
                && !failures
                    .iter()
                    .any(|(_, code, _)| skill_failure(code, skill_attempts))
                && (options.agent == "pi"
                    || (options.agent == "auto" && config.classification_pi.enabled));
            if try_pi && options.agent == "auto" && !pi_confirmed {
                // Ask before the first Pi call: nothing is saved for this
                // skill or the ones after it until the user confirms.
                let remaining: Vec<&str> = order
                    .iter()
                    .filter(|id| !updates.contains_key(*id))
                    .map(String::as_str)
                    .collect();
                let confirmation = pi_confirmation(
                    &config.classification_pi,
                    &claude,
                    options.full,
                    &pi_skills(&remaining),
                );
                stop_providers = providers
                    .iter()
                    .map(|p| match p.agent.as_str() {
                        "claude" => crate::ProviderStatus {
                            status: "failed".into(),
                            reason_code: failures.first().map(|(_, code, _)| code.clone()),
                            ..p.clone()
                        },
                        "pi" => crate::ProviderStatus {
                            reason_code: Some("PI_CONFIRMATION_REQUIRED".into()),
                            planned_calls: confirmation.max_calls,
                            ..p.clone()
                        },
                        _ => p.clone(),
                    })
                    .collect();
                stop = Some(Error::decision(
                    "PI_CONFIRMATION_REQUIRED",
                    format!(
                        "Claude did not classify '{}' ({}). Pi is enabled as the fallback: provider {}, model {}{}, up to {} calls for the {} remaining skills. Nothing was saved for this skill or the ones after it. To use Pi, confirm by classifying the remaining skills (listed in `stopped.remaining`) with --confirm {}; otherwise classify them with Claude again, or choose keywords (approximate) explicitly.",
                        target.id,
                        failures
                            .iter()
                            .map(|(_, code, _)| code.as_str())
                            .collect::<Vec<_>>()
                            .join("; "),
                        confirmation.provider,
                        confirmation.model,
                        if confirmation.model_default {
                            " (Pi's default for this provider)"
                        } else {
                            ""
                        },
                        confirmation.max_calls,
                        confirmation.skills,
                        confirmation.token
                    ),
                ));
                report.pi_confirmation = Some(confirmation);
                break;
            }
            if try_pi {
                let outcome = match agent_runner::classify_controlled(vault, request("pi"), control)
                {
                    Err(error) if error.code == "CANCELLED" => {
                        stop = Some(cancelled());
                        break;
                    }
                    result => result?,
                };
                report.agent_calls += outcome.attempts;
                skill_attempts += outcome.attempts;
                if control.is_some_and(ClassifyControl::is_cancelled) {
                    stop = Some(cancelled());
                    break;
                }
                match provider_result(outcome, &allowed) {
                    ProviderResult::Proposal(outcome) => chosen = Some(outcome),
                    ProviderResult::Stopped(error) if options.agent == "pi" => {
                        stop_providers = failed_provider(&providers, "pi", &error.code);
                        stop = Some(error);
                        break;
                    }
                    ProviderResult::Stopped(error) => {
                        failures.push(("pi", error.code, error.message));
                    }
                    ProviderResult::Failed(outcome) => {
                        let (code, message) = failure(&outcome);
                        failures.push(("pi", code, message));
                    }
                }
            }
            let Some(outcome) = chosen else {
                if let [(agent, code, _)] = failures.as_slice()
                    && skill_failure(code, skill_attempts)
                {
                    report.skipped.push(SkippedClassification {
                        id: target.id,
                        agent: (*agent).into(),
                        reason_code: code.clone(),
                        attempts: skill_attempts,
                    });
                    if let Some(c) = control {
                        let mut progress = c.progress.lock().unwrap_or_else(|e| e.into_inner());
                        progress.skipped += 1;
                        progress.current_id = None;
                    }
                    continue;
                }
                // Ask instead of saving an approximate result: the run stops
                // here, completed skills stay saved, this one is untouched.
                stop_providers = providers
                    .iter()
                    .map(
                        |p| match failures.iter().find(|(agent, ..)| *agent == p.agent) {
                            Some((_, reason, _)) => crate::ProviderStatus {
                                status: "failed".into(),
                                reason_code: Some(reason.clone()),
                                ..p.clone()
                            },
                            None => p.clone(),
                        },
                    )
                    .collect();
                let reasons = failures
                    .iter()
                    .map(|(agent, reason, message)| {
                        if message.is_empty() {
                            format!("{agent}: {reason}")
                        } else {
                            format!("{agent}: {reason}: {message}")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                let claude_absent = failures.iter().any(|(agent, code, _)| {
                    *agent == "claude" && CLAUDE_ABSENT.contains(&code.as_str())
                });
                let (code, next) = match options.agent.as_str() {
                    "claude" => (
                        "AGENT_CLAUDE_UNAVAILABLE",
                        "classify the remaining skills (listed in `stopped.remaining`) with Claude again, or choose keywords (approximate) explicitly",
                    ),
                    "pi" => (
                        "AGENT_PI_UNAVAILABLE",
                        "classify the remaining skills (listed in `stopped.remaining`) with Claude, or choose keywords (approximate) explicitly",
                    ),
                    _ if claude_absent && failures.iter().any(|(agent, ..)| *agent == "pi") => (
                        "AGENT_AUTO_UNAVAILABLE",
                        "Claude is not found on this computer and Pi did not classify this skill: install Claude and classify the remaining skills (listed in `stopped.remaining`) again, or choose keywords (approximate) explicitly. Codex is not supported for classification",
                    ),
                    _ if claude_absent && claude_seen => (
                        "AGENT_AUTO_UNAVAILABLE",
                        "Claude is no longer found on this computer: install or restore it and classify the remaining skills (listed in `stopped.remaining`) again, or choose keywords (approximate) explicitly. Codex is not supported for classification",
                    ),
                    _ if claude_absent => (
                        "AGENT_AUTO_UNAVAILABLE",
                        "Claude is not found on this computer: install it and classify the remaining skills (listed in `stopped.remaining`) again, or choose keywords (approximate) explicitly",
                    ),
                    _ => (
                        "AGENT_AUTO_UNAVAILABLE",
                        "classify the remaining skills (listed in `stopped.remaining`) with Claude again, or choose keywords (approximate) explicitly. Codex is not supported for classification",
                    ),
                };
                stop = Some(Error::decision(
                    code,
                    format!(
                        "No requested provider could classify '{}'. Nothing was saved for this skill or the ones after it; {next}. ({reasons})",
                        target.id
                    ),
                ));
                break;
            };
            attempts = skill_attempts;
            warnings = outcome.warnings;
            for (agent, code, _) in &failures {
                warnings.push(format!(
                    "{agent} did not classify this skill ({code}); classified by {}.",
                    outcome.classified_by
                ));
            }
            reason_code = outcome.reason_code;
            searched_locations = outcome.searched_locations;
            classified_by = outcome.classified_by;
            prompt_version = outcome.prompt_version;
            provider_proposal = outcome.proposal;
        }
        let provider_classified = provider_proposal.is_some();
        let mut proposal_created = provider_classified;
        if let Some(proposal) = provider_proposal {
            let reason = proposal.category_reason.clone();
            // Owner, 2026-11-24 (r6): a proposal that matches the description
            // the user chose, after whitespace normalization, is no proposal;
            // the skill does not go back to waiting for review.
            let unchanged = crate::review::chosen_description(
                &metadata.description,
                &target.original_description,
            )
            .is_some_and(|chosen| {
                crate::review::same_description(&proposal.improved_description, &chosen)
            });
            apply_proposal(&mut metadata, proposal, &classified_by, &config);
            if unchanged {
                metadata.proposal = None;
                proposal_created = false;
                warnings.push(
                    "The proposed description matches the one you chose; nothing waits for review."
                        .into(),
                );
            }
            metadata.classification.classified_from = Some(target.classified_from.into());
            metadata.classification_context =
                prompt_version.map(|prompt_version| crate::ClassificationContext {
                    prompt_version,
                    category_reason: reason,
                    binds: crate::context_binding(&metadata.classification),
                });
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
            metadata.classification_context = None;
        }
        report.items.push(ClassifiedItem {
            id: target.id.clone(),
            category: metadata.classification.category.clone(),
            classified_by: metadata.classification.classified_by.clone(),
            proposal_created,
            attempts,
            warnings,
            reason_code: if provider_classified { None } else { reason_code },
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
            providers: stop_providers,
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
        // Owner, 2026-11-24 (r5, widened in r6): the proposed descriptions
        // of skills that are not risky are approved in a commit of their own,
        // which `undo` reverts. A failure there leaves them pending and the
        // classification saved. It runs after a run that changed nothing
        // too (r6): a proposal identical to the pending one is still this
        // run's proposal.
        if !options.dry_run && config.auto_approve_descriptions {
            // Only descriptions a provider proposed in this run; keywords
            // propose none.
            let ids = report
                .items
                .iter()
                .filter(|item| item.proposal_created && updates.contains_key(&item.id))
                .map(|item| item.id.clone())
                .collect::<Vec<_>>();
            match crate::review::approve_not_risky(vault, &ids) {
                Ok(approved) => report.auto_approved = approved,
                Err(error) => report.warnings.push(format!(
                    "AUTO_APPROVAL_FAILED: {}: {} The proposed descriptions wait for review.",
                    error.code, error.message
                )),
            }
        }
    }
    Ok(report)
}

/// Providers in the order `auto` tries them. Claude first; Codex is listed
/// as unsupported so the reason is visible, and never called.
fn providers(agent: &str, calls: usize, pi_enabled: bool) -> Vec<crate::ProviderStatus> {
    let status = |agent: &str, status: &str, reason: Option<&str>, calls| crate::ProviderStatus {
        agent: agent.into(),
        status: status.into(),
        reason_code: reason.map(str::to_owned),
        planned_calls: calls,
    };
    match agent {
        "claude" => vec![status("claude", "planned", None, calls)],
        "pi" => vec![status("pi", "planned", None, calls)],
        // Pi is tried only for a skill Claude could not classify; its count
        // is the most it could add, shown before the run is confirmed.
        "auto" => vec![
            status("claude", "planned", None, calls),
            if pi_enabled {
                status("pi", "fallback", None, calls)
            } else {
                status("pi", "disabled", Some("AGENT_PI_DISABLED"), 0)
            },
            status("codex", "unsupported", Some("AGENT_CODEX_UNSUPPORTED"), 0),
        ],
        _ => vec![],
    }
}

/// What Pi would receive for one skill: the selected text (all of
/// SKILL.md with `--full`, else its excerpt) and the file names.
pub fn pi_content_digest(raw: &str, files: &[String]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"kinana-pi-skill-content-v1\0");
    for part in std::iter::once(raw).chain(files.iter().map(String::as_str)) {
        hasher.update(part.len().to_string().as_bytes());
        hasher.update(b"\0");
        hasher.update(part.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

/// The confirmation `auto` needs before it sends `skills` (id, content
/// digest) to Pi. The token binds every Pi setting, `--full` and each
/// skill's [`pi_content_digest`], like Claude's confirmed estimate (owner,
/// 2026-11-19, item 2); ids and settings hold no NUL.
pub fn pi_confirmation(
    settings: &crate::PiSettings,
    claude: &crate::ClaudeSettings,
    full: bool,
    skills: &[(String, String)],
) -> crate::PiConfirmation {
    let mut skills = skills.to_vec();
    skills.sort_unstable();
    skills.dedup_by(|a, b| a.0 == b.0);
    // The model Pi is given, and whether it is Pi's default for the
    // provider (0.3, r5): a different default or a newly chosen model is a
    // different confirmation.
    let resolved = agent_runner::pi::resolved_model(settings);
    let model = resolved
        .as_ref()
        .map(|resolved| resolved.model.clone())
        .unwrap_or_default();
    let model_default = resolved.is_some_and(|resolved| resolved.pi_default);
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"kinana-pi-confirmation-v3\0");
    for part in [
        settings.provider.as_str(),
        &settings.model,
        &model,
        if model_default { "pi-default" } else { "chosen" },
        &settings.thinking,
        &settings.account,
        &claude.model,
        &claude.effort,
    ] {
        hasher.update(part.as_bytes());
        hasher.update(b"\0");
    }
    hasher.update(if full { b"full\0" } else { b"part\0" });
    hasher.update(skills.len().to_string().as_bytes());
    for (id, digest) in &skills {
        hasher.update(b"\0");
        hasher.update(id.as_bytes());
        hasher.update(b"\0");
        hasher.update(digest.as_bytes());
    }
    crate::PiConfirmation {
        provider: settings.provider.clone(),
        model,
        model_default,
        skills: skills.len(),
        max_calls: skills.len() * 2,
        token: format!("pc3.{}", hasher.finalize().to_hex()),
    }
}

/// Reasons that mean Claude is not on this computer (or cannot run on this
/// platform at all). Only these let `auto` use keywords (owner, 2026-11-18).
const CLAUDE_ABSENT: [&str; 2] = ["AGENT_EXECUTABLE_NOT_FOUND", "AGENT_PLATFORM_UNSUPPORTED"];

fn skill_failure(code: &str, attempts: usize) -> bool {
    matches!(code, "AGENT_TIMEOUT" | "AGENT_REFUSED")
        || (code == "INVALID_AGENT_PROPOSAL" && attempts == 2)
}

/// Stop codes after which the user chooses the next step: the CLI exits 2
/// and the window offers the choice. The limit, credit and sign-in codes
/// reach `stopped` only with a named agent; under `auto` they are part of
/// `AGENT_AUTO_UNAVAILABLE` (owner, 2026-11-18).
pub const STOPS_THAT_ASK: [&str; 8] = [
    "AGENT_AUTO_UNAVAILABLE",
    "AGENT_PI_UNAVAILABLE",
    "AGENT_CLAUDE_UNAVAILABLE",
    "PI_CONFIRMATION_REQUIRED",
    "AGENT_USAGE_LIMIT",
    "AGENT_CREDIT_EXHAUSTED",
    "AGENT_NOT_LOGGED_IN",
    // Pi only (owner, 2026-11-19): the model in Pi's settings is not usable.
    "AGENT_MODEL_UNAVAILABLE",
];

/// The run's providers with `agent` marked as failed for `code`.
fn failed_provider(
    providers: &[crate::ProviderStatus],
    agent: &str,
    code: &str,
) -> Vec<crate::ProviderStatus> {
    providers
        .iter()
        .map(|p| {
            if p.agent == agent {
                crate::ProviderStatus {
                    status: "failed".into(),
                    reason_code: Some(code.into()),
                    ..p.clone()
                }
            } else {
                p.clone()
            }
        })
        .collect()
}

enum ProviderResult {
    /// A proposal that passed validation.
    Proposal(agent_runner::AgentOutcome),
    /// The provider stopped the run (limit, credit, login).
    Stopped(Error),
    /// No usable proposal: refused, failed, or invalid after the retry.
    Failed(agent_runner::AgentOutcome),
}

fn provider_result(
    mut outcome: agent_runner::AgentOutcome,
    allowed: &AllowedValues<'_>,
) -> ProviderResult {
    if let Some(error) = outcome.stop.take() {
        return ProviderResult::Stopped(error);
    }
    match &outcome.proposal {
        Some(proposal) => match agent_runner::validate_proposal(proposal, allowed) {
            Ok(()) => ProviderResult::Proposal(outcome),
            Err(error) => {
                outcome.proposal = None;
                outcome.refuse_with(&error);
                ProviderResult::Failed(outcome)
            }
        },
        None => ProviderResult::Failed(outcome),
    }
}

/// The code and the runner's last diagnostic, without its sentences about a
/// keyword fallback that no longer happens here. The diagnostic names what
/// failed (for example the event Pi's gate refused), which the owner needs
/// to decide what to do next (Pi live test, 2026-11-18).
fn failure(outcome: &agent_runner::AgentOutcome) -> (String, String) {
    let detail = outcome
        .warnings
        .last()
        .map(|text| {
            text.split_inclusive(". ")
                .filter(|sentence| !sentence.to_ascii_lowercase().contains("keyword"))
                .collect::<String>()
                .trim()
                .to_owned()
        })
        .unwrap_or_default();
    (
        outcome
            .reason_code
            .clone()
            .unwrap_or_else(|| "AGENT_UNAVAILABLE".into()),
        detail,
    )
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
        "auto" | "keywords" | "claude" | "codex" | "pi"
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
    // The proposal has its own description slot. Only a review, or the
    // automatic approval after the classification commit (owner, r6), makes
    // it the active description.
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
    fn skill_failure_does_not_treat_provider_failures_as_skippable() {
        assert!(skill_failure("AGENT_TIMEOUT", 1));
        assert!(skill_failure("INVALID_AGENT_PROPOSAL", 2));
        assert!(!skill_failure("INVALID_AGENT_PROPOSAL", 1));
        for code in [
            "AGENT_USAGE_LIMIT",
            "AGENT_NOT_LOGGED_IN",
            "AGENT_CREDIT_EXHAUSTED",
            "AGENT_TOOLS_NOT_ISOLATED",
            "AGENT_TOOL_USE",
            "AGENT_EXECUTABLE_NOT_FOUND",
            "AGENT_EXECUTABLE_UNTRUSTED",
        ] {
            assert!(!skill_failure(code, 1), "{code}");
        }
    }

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
            category_reason: String::new(),
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
