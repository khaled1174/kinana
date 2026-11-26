mod output;
use clap::{Parser, error::ErrorKind};
use kinana::{Args, BundleCommands, Commands};
use kinana_core::{
    Error, ImportOptions, ImportReport, Result, ScanStatus, doctor, import, scan, vault::Vault,
};
use std::{path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    let json = std::env::args_os().any(|arg| arg == "--json");
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            if json {
                let value = if error.kind() == ErrorKind::DisplayVersion {
                    serde_json::json!({"version": error.to_string().trim_end()})
                } else {
                    serde_json::json!({"help": error.to_string()})
                };
                return match output::emit(&value, true, "") {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(_) => ExitCode::from(1),
                };
            }
            return match error.print() {
                Ok(()) => ExitCode::SUCCESS,
                Err(_) => ExitCode::from(1),
            };
        }
        Err(error) => {
            output::error(&Error::new("INVALID_ARGUMENTS", error.to_string()), json);
            return ExitCode::from(1);
        }
    };
    match run(&args) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            output::error(&error, args.json);
            ExitCode::from(if error.needs_decision { 2 } else { 1 })
        }
    }
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| Error::new("HOME_NOT_FOUND", "Cannot resolve the user's home directory"))
}

fn run(args: &Args) -> Result<u8> {
    let home = home_dir()?;
    if let Commands::Migrate {
        project,
        confirm,
        rollback,
    } = &args.command
    {
        let report = if args.dry_run || confirm.is_none() {
            kinana_core::migration::preview(&home, project, *rollback)?
        } else {
            kinana_core::migration::apply(
                &home,
                project,
                *rollback,
                confirm.as_deref().unwrap_or_default(),
            )?
        };
        let mut text = format!(
            "{}: {} -> {}\nChanged: {} | Preview: {}\nConfirmation: {}",
            report.action,
            report.source_state.display(),
            report.target_state.display(),
            report.changed,
            report.dry_run,
            report.preview_token
        );
        for item in &report.state_files {
            text.push_str(&format!(
                "\n{} {} ({}, {:o}) -> {}",
                item.operation,
                item.from.display(),
                item.kind,
                item.mode,
                item.to
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "retained".into())
            ));
        }
        for item in &report.projects {
            text.push_str(&format!(
                "\nProject [{}]: {} -> {}",
                item.state,
                item.from.display(),
                item.to.display()
            ));
        }
        for item in &report.routers {
            text.push_str(&format!(
                "\nRouter {} [{}]: {} -> {}",
                item.agent,
                item.state,
                item.from.display(),
                item.to.display()
            ));
        }
        for path in &report.control_paths {
            text.push_str(&format!("\nControl/recovery path: {}", path.display()));
        }
        for warning in &report.warnings {
            text.push_str(&format!("\nWarning: {warning}"));
        }
        output::emit(&report, args.json, &text)?;
        return Ok(if report.changed && report.dry_run && !args.dry_run {
            2
        } else {
            0
        });
    }
    let init_path = match &args.command {
        Commands::Init { path, .. } => path.clone(),
        _ => None,
    };
    if args.vault.is_some() && init_path.is_some() && args.vault != init_path {
        return Err(Error::new(
            "INVALID_ARGUMENTS",
            "Use either --vault or init --path, or specify the same path",
        ));
    }
    let root = args
        .vault
        .clone()
        .or(init_path)
        .map(Ok)
        .unwrap_or_else(|| kinana_core::settings::preferred_vault(&home))?;
    let vault = Vault::new(root, kinana_core::state::index_path(&home)?);
    let read_only = match &args.command {
        Commands::List { .. }
        | Commands::Show { .. }
        | Commands::Search { .. }
        | Commands::Suggest { .. }
        | Commands::Status { .. }
        | Commands::Outdated
        | Commands::Groups => true,
        Commands::Doctor {
            rebuild_index,
            repair,
            confirm,
        } => !*rebuild_index && !(*repair && confirm.is_some()),
        Commands::Scan { import_skills, .. } => !*import_skills,
        Commands::RefreshAudit { confirm } => confirm.is_none(),
        Commands::Review {
            ids,
            accept,
            reject,
            original,
            confirm,
        } => (!accept && !reject && !original) || (*accept && confirm.is_none() && ids.len() > 1),
        Commands::Bundle { command } => {
            matches!(command, BundleCommands::List | BundleCommands::Show { .. })
        }
        _ => false,
    };
    // Refused before anything touches the state or the index (0.3).
    if let Commands::Classify { agent, .. } = &args.command
        && agent == "codex"
    {
        return Err(kinana_core::agent_runner::codex_unsupported());
    }
    if !read_only && !args.dry_run {
        kinana_core::state::require_writable(&vault.index_path)?;
    }
    // After an upgrade from the 0.1.0 beta this version's index does not exist
    // yet; that version's file is left as it is. A preview writes nothing, so
    // only other commands build it.
    if !args.dry_run {
        prepare_index(&vault);
    }
    match &args.command {
        Commands::Migrate { .. } => unreachable!("handled before resolving active state"),
        Commands::Init { install_router, .. } => {
            let report = if *install_router {
                kinana_core::router::initialize(&vault, &home, args.dry_run)?
            } else {
                vault.init(args.dry_run)?
            };
            let mut text = format!(
                "{} vault: {}\nIndex: {}\nAgents: {}",
                if report.dry_run {
                    "Would initialize"
                } else if report.created {
                    "Initialized"
                } else {
                    "Existing"
                },
                report.vault.display(),
                report.index.display(),
                report
                    .agents
                    .iter()
                    .filter(|a| a.available)
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            if let Some(router) = &report.router {
                for target in &router.targets {
                    text.push_str(&format!(
                        "\nRouter {}: {} ({})",
                        target.agent,
                        target.path.display(),
                        target.action
                    ));
                }
                for warning in &router.warnings {
                    text.push_str(&format!("\nWarning: {warning}"));
                }
            }
            output::emit(&report, args.json, &text)?;
        }
        Commands::Add {
            input,
            all,
            select,
            allow_risky,
            suffix,
        } => {
            let options = ImportOptions {
                dry_run: args.dry_run,
                all: *all,
                select: select.clone(),
                allow_risky: *allow_risky,
                id_suffix: suffix.clone(),
                ..Default::default()
            };
            let report = import::add(
                &vault,
                input,
                &kinana_core::state::directory(&home)?.join("tmp"),
                &options,
            )?;
            output::emit(&report, args.json, &import_text(&report))?;
        }
        Commands::Scan {
            paths,
            import_skills,
            allow_risky,
        } => {
            if *import_skills {
                let options = ImportOptions {
                    dry_run: args.dry_run,
                    allow_risky: *allow_risky,
                    ..ImportOptions::default()
                };
                let report = import::import_scan(&vault, paths, &options)?;
                output::emit(&report, args.json, &import_text(&report))?;
                return Ok(0);
            }
            // A scan is read-only, including the index. Missing vault means an empty catalog.
            let known = if vault.root.join("box.toml").exists() {
                vault.records()?
            } else {
                vec![]
            };
            let report = scan::scan(paths, &known)?;
            let mut text = format!(
                "New: {}  Duplicate: {}  Variant: {}  Broken: {}",
                report.counts.new,
                report.counts.duplicate,
                report.counts.variant,
                report.counts.broken
            );
            for entry in &report.entries {
                let status = match entry.status {
                    ScanStatus::New => "new",
                    ScanStatus::Duplicate => "duplicate",
                    ScanStatus::Variant => "variant",
                    ScanStatus::Broken => "broken",
                };
                text.push_str(&format!(
                    "\n{status}\t{}{}",
                    kinana_core::names::visible(&entry.path.to_string_lossy()),
                    entry
                        .error
                        .as_ref()
                        .map(|error| format!(" — {}", kinana_core::names::visible(error)))
                        .unwrap_or_default()
                ));
            }
            for warning in &report.warnings {
                text.push_str(&format!(
                    "\nWarning: {}",
                    kinana_core::names::visible(warning)
                ));
            }
            output::emit(&report, args.json, &text)?;
        }
        Commands::List {
            brief,
            limit,
            page,
            query,
            state,
            pinned,
            category,
            stack,
            stage,
            agent,
            audit,
        } => {
            let filter = kinana_core::catalog::CatalogFilter {
                query: query.clone().unwrap_or_default(),
                state: state.clone(),
                category: category.clone(),
                stack: stack.clone(),
                stage: stage.clone(),
                agent: agent.clone(),
                audit: audit.clone(),
                pinned: *pinned,
            };
            if *brief {
                let report = kinana_core::catalog::brief(
                    &vault,
                    &filter,
                    limit.map_or(kinana_core::catalog::BRIEF_PAGE_SKILLS, usize::from),
                    page.map_or(1, |page| page as usize),
                )?;
                for warning in &report.warnings {
                    eprintln!("Warning: {warning}");
                }
                output::emit(&report, args.json, &brief_text(&report))?;
                return Ok(0);
            }
            let report = kinana_core::catalog::list(&vault, &filter)?;
            for warning in &report.warnings {
                eprintln!("Warning: {warning}");
            }
            let records = report
                .items
                .into_iter()
                .map(|item| item.skill)
                .collect::<Vec<_>>();
            let text = if records.is_empty() {
                "No skills in the vault.".into()
            } else {
                records
                    .iter()
                    .map(|s| format!("{}\t{}\t{}", s.id, s.category, s.description))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            output::emit(&records, args.json, &text)?;
        }
        Commands::Use {
            selections,
            project,
            agent,
            mode,
            include_secondary,
            confirm,
        } => {
            let report = kinana_core::activate::activate_cli(
                &vault,
                &kinana_core::ActivateOptions {
                    selections: selections.clone(),
                    project: project.clone(),
                    home,
                    agent: agent.clone(),
                    mode: mode.clone(),
                    include_secondary: include_secondary.clone(),
                    dry_run: args.dry_run,
                },
                confirm.as_deref(),
                args.dry_run,
            )?;
            let mut text = format!(
                "{} {} activation(s) in {}",
                if report.dry_run {
                    "Would apply"
                } else {
                    "Applied"
                },
                report.items.len(),
                report.project.display()
            );
            for item in &report.items {
                text.push_str(&format!(
                    "\n{}\t{}\t{}\t{}\t{}",
                    item.id,
                    item.agent,
                    item.mode,
                    item.invocation,
                    kinana_core::names::visible(&item.target.to_string_lossy())
                ));
            }
            for suggestion in &report.gitignore_suggestions {
                text.push_str(&format!("\nSuggested .gitignore entry: {suggestion}"));
            }
            for warning in &report.warnings {
                text.push_str(&format!("\nWarning: {warning}"));
            }
            output::emit(&report, args.json, &text)?;
        }
        Commands::Status { project } => {
            let report = kinana_core::status::status(&vault, project.as_deref())?;
            let mut text = String::new();
            for project in &report.projects {
                text.push_str(&format!(
                    "{}\t{}\n",
                    project.project.display(),
                    project.state
                ));
                for item in &project.items {
                    text.push_str(&format!(
                        "{}\t{}\t{}\t{}\n",
                        item.id, item.agent, item.state, item.message
                    ));
                }
                for warning in &project.warnings {
                    text.push_str(&format!("Warning: {warning}\n"));
                }
            }
            output::emit(&report, args.json, text.trim_end())?;
        }
        Commands::Unuse {
            ids,
            project,
            agent,
        } => {
            let report = kinana_core::unuse::unuse(
                &vault,
                &kinana_core::UnuseOptions {
                    project: project.clone(),
                    ids: ids.clone(),
                    agent: agent.clone(),
                    dry_run: args.dry_run,
                },
            )?;
            let mut text = format!(
                "{} {} activation(s) in {}",
                if report.dry_run {
                    "Would remove"
                } else {
                    "Removed"
                },
                report.removed.len(),
                report.project.display()
            );
            for path in &report.removed {
                text.push_str(&format!("\n{}", path.display()));
            }
            for warning in &report.warnings {
                text.push_str(&format!("\nWarning: {warning}"));
            }
            output::emit(&report, args.json, &text)?;
        }
        Commands::Adopt {
            project,
            agent,
            yes,
        } => {
            let report = kinana_core::adopt::adopt(
                &vault,
                &kinana_core::AdoptOptions {
                    project: project.clone(),
                    home,
                    agent: agent.clone(),
                    dry_run: args.dry_run,
                    confirmed: *yes,
                },
            )?;
            let mut text = String::new();
            for candidate in &report.candidates {
                text.push_str(&format!(
                    "{}\t{}\t{}\t{}\n",
                    kinana_core::names::visible(&candidate.target.to_string_lossy()),
                    if candidate.eligible {
                        "eligible"
                    } else {
                        "preserved"
                    },
                    candidate.content_hash.as_deref().unwrap_or("unknown"),
                    candidate.reason
                ));
                if let Some(target) = &candidate.link_to {
                    text.push_str(&format!("  Link to: {}\n", target.display()));
                }
            }
            output::emit(&report, args.json, text.trim_end())?;
        }
        Commands::Category { id, category } => {
            let report = kinana_core::edit::set_category(&vault, id, category, args.dry_run)?;
            output::emit(
                &report,
                args.json,
                &format!("{}: category={}", id, category),
            )?;
        }
        Commands::Dimensions {
            id,
            stack,
            stage,
            clear_stack,
            clear_stage,
        } => {
            // A named option replaces that dimension; an omitted one is left alone.
            fn requested(values: &[String], clear: bool) -> Option<&[String]> {
                if clear {
                    Some(&[])
                } else if values.is_empty() {
                    None
                } else {
                    Some(values)
                }
            }
            let report = kinana_core::edit::set_dimensions(
                &vault,
                id,
                requested(stack, *clear_stack),
                requested(stage, *clear_stage),
                args.dry_run,
            )?;
            output::emit(
                &report,
                args.json,
                &format!(
                    "{}: stack=[{}] stage=[{}]{}",
                    report.id,
                    report.stack.join(", "),
                    report.stage.join(", "),
                    if report.dry_run { " (preview)" } else { "" }
                ),
            )?;
        }
        Commands::Pin { id, unset } => {
            let report = kinana_core::edit::set_pinned(&vault, id, !unset, args.dry_run)?;
            output::emit(
                &report,
                args.json,
                &format!(
                    "{}: pinned={}{}",
                    id,
                    report.pinned,
                    if report.dry_run { " (preview)" } else { "" }
                ),
            )?;
        }
        Commands::Remove { id } => {
            let report = kinana_core::remove::remove(&vault, id, args.dry_run)?;
            let mut text = format!(
                "{} skill {}",
                if report.dry_run {
                    "Would remove"
                } else {
                    "Removed"
                },
                report.id
            );
            for path in &report.removed_paths {
                text.push_str(&format!("\n{path}"));
            }
            for bundle in &report.bundles_changed {
                text.push_str(&format!("\nUpdated bundle: {bundle}"));
            }
            output::emit(&report, args.json, &text)?;
        }
        Commands::Show { id } => {
            let detail = vault.show(id)?;
            let text = format!(
                "{} ({})\nCategory: {}\nHash: {}\n\n{}",
                kinana_core::names::visible(&detail.skill.name),
                detail.skill.id,
                detail.skill.category,
                detail.skill.content_hash,
                detail.raw_skill_md
            );
            output::emit(&detail, args.json, &text)?;
        }
        Commands::Search { query } => {
            let report = kinana_core::search::search(&vault, query)?;
            let text = if report.items.is_empty() {
                "No matching skills.".into()
            } else {
                report
                    .items
                    .iter()
                    .map(|s| format!("{}\t{}\t{}", s.id, s.category, s.description))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            output::emit(&report, args.json, &text)?;
        }
        Commands::Suggest { project } => {
            let report = kinana_core::suggest::suggest(&vault, project)?;
            let mut text = format!(
                "Categories: {}\nStacks: {}",
                report.categories.join(", "),
                report.stacks.join(", ")
            );
            for bundle in &report.bundles {
                text.push_str(&format!(
                    "\n{}\t{} category match(es)\t{}",
                    bundle.bundle.name, bundle.match_count, bundle.bundle.title
                ));
            }
            for skill in &report.skills {
                text.push_str(&format!(
                    "\n{}\t{}\t{}",
                    skill.id, skill.category, skill.description
                ));
            }
            for warning in &report.warnings {
                text.push_str(&format!("\nWarning: {warning}"));
            }
            output::emit(&report, args.json, &text)?;
        }
        Commands::Classify {
            ids,
            all,
            pending,
            agent,
            model,
            effort,
            claude_events,
            yes,
            full,
            confirm,
        } => {
            let report = kinana_core::classify::classify(
                &vault,
                &kinana_core::ClassifyOptions {
                    ids: ids.clone(),
                    all: *all,
                    pending: *pending,
                    agent: agent.clone(),
                    confirmed: *yes,
                    dry_run: args.dry_run,
                    full: *full,
                    claude_model: model.clone(),
                    claude_effort: effort.clone(),
                    claude_events: *claude_events,
                    pi_confirmation: confirm.clone(),
                },
            )?;
            let mut text = format!(
                "Selected: {}  Estimated agent calls: {}  Attempts: {}\nEstimated input tokens: {} fixed/call; {} fixed total + {} skill text = {} total (retry ceiling {})",
                report.selected,
                report.estimated_agent_calls,
                report.agent_calls,
                report.estimated_tokens.fixed_tokens_per_call,
                report.estimated_tokens.fixed_tokens_total,
                report.estimated_tokens.skill_tokens_total,
                report.estimated_tokens.total_input_tokens,
                report.estimated_tokens.retry_limit_input_tokens
            );
            if let Some(pi) = &report.pi_confirmation {
                text.push_str(&format!(
                    "\nPi (experimental) may classify {} skills if Claude does not: provider {}, model {}{}, up to {} calls. To allow it, repeat for exactly these skills with --confirm {}",
                    pi.skills,
                    pi.provider,
                    pi.model,
                    if pi.model_default {
                        " (Pi's default for this provider)"
                    } else {
                        ""
                    },
                    pi.max_calls,
                    pi.token
                ));
            }
            for item in &report.items {
                text.push_str(&format!(
                    "\n{}\t{}\t{}",
                    item.id, item.category, item.classified_by
                ));
                for warning in &item.warnings {
                    text.push_str(&format!("\nWarning: {warning}"));
                }
            }
            for warning in &report.warnings {
                text.push_str(&format!("\nWarning: {warning}"));
            }
            if let Some(stopped) = &report.stopped {
                output::warning(&format!(
                    "Classification stopped ({}): {} Classified {}; not classified: {}.",
                    stopped.reason_code,
                    stopped.message,
                    stopped.classified,
                    stopped.remaining.len()
                ))?;
            }
            for item in &report.skipped {
                text.push_str(&format!(
                    "\nSkipped {} ({}: {}, {} attempts)",
                    item.id, item.agent, item.reason_code, item.attempts
                ));
            }
            // Contract 11 (owner, 2026-11-24): `auto_approved` in --json.
            if let Some(approved) = &report.auto_approved {
                let one = approved.ids.len() == 1;
                text.push_str(&format!(
                    "\nApproved automatically: the proposed description{} of {} skill{} that {} not risky, in commit {}: {}. `kinana undo` reverts this approval only.",
                    if one { "" } else { "s" },
                    approved.ids.len(),
                    if one { "" } else { "s" },
                    if one { "is" } else { "are" },
                    approved.commit,
                    approved.ids.join(", ")
                ));
            }
            output::emit(&report, args.json, &text)?;
            if report.stopped.as_ref().is_some_and(|s| {
                kinana_core::classify::STOPS_THAT_ASK.contains(&s.reason_code.as_str())
            }) || (report.stopped.is_none() && !report.skipped.is_empty())
            {
                return Ok(2);
            }
        }
        Commands::Review {
            ids,
            accept,
            confirm,
            ..
        } if ids.len() > 1 || confirm.is_some() => {
            if !accept {
                return Err(Error::new(
                    "INVALID_ARGUMENTS",
                    "Several skills can only be reviewed together with --accept",
                ));
            }
            let report =
                kinana_core::review::accept_many(&vault, ids, confirm.as_deref(), args.dry_run)?;
            let mut text = String::new();
            for item in &report.items {
                text.push_str(&format!(
                    "{}\nCurrent:\n{}\nProposed:\n{}\n\n",
                    item.id, item.current, item.proposed
                ));
            }
            if report.committed {
                text.push_str(&format!(
                    "Accepted {} descriptions in one change; `kinana undo` reverts them together.",
                    report.items.len()
                ));
            } else if report.requires_confirmation {
                text.push_str(&format!(
                    "Review the {} descriptions above, then repeat with --confirm {}",
                    report.items.len(),
                    report.preview_token
                ));
            } else {
                text.push_str("Preview only; nothing was written.");
            }
            output::emit(&report, args.json, &text)?;
            return Ok(if report.requires_confirmation && !args.dry_run {
                2
            } else {
                0
            });
        }
        Commands::Review {
            ids,
            accept,
            reject,
            original,
            ..
        } => {
            let id = &ids[0];
            let decision = if *accept {
                kinana_core::ReviewDecision::Accept
            } else if *reject {
                kinana_core::ReviewDecision::Reject
            } else if *original {
                kinana_core::ReviewDecision::Original
            } else {
                kinana_core::ReviewDecision::Preview
            };
            let report = kinana_core::review::review(&vault, id, decision, args.dry_run)?;
            let text = format!(
                "{} — {}\nOriginal:\n{}\nCurrent:\n{}\nProposed:\n{}\nActive: {}",
                report.id,
                report.decision,
                report.original,
                report.current,
                report.proposed.as_deref().unwrap_or("(none)"),
                report.active
            );
            output::emit(&report, args.json, &text)?;
        }
        Commands::Outdated => {
            let report = kinana_core::source_updates::outdated(&vault, None)?;
            let text = report
                .items
                .iter()
                .map(|item| {
                    format!(
                        "{}\t{}\t{} -> {}{}",
                        item.id,
                        item.status,
                        item.old_commit.as_deref().unwrap_or("-"),
                        item.new_commit.as_deref().unwrap_or("-"),
                        item.error.as_ref().map_or(String::new(), |error| format!(
                            "\n  {}: {}",
                            error.reason_code, error.message
                        ))
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            output::emit(&report, args.json, &text)?;
            return Ok(if report.complete { 0 } else { 1 });
        }
        Commands::Update {
            id,
            reference,
            confirm,
            allow_risky,
        } => {
            let report = kinana_core::source_updates::update(
                &vault,
                &kinana_core::source_updates::UpdateOptions {
                    id: id.clone(),
                    reference: reference.clone(),
                    confirm: confirm.clone(),
                    allow_risky: *allow_risky,
                    dry_run: args.dry_run,
                },
                None,
            )?;
            let mut text = format!(
                "{}: {}\n{} -> {}\nAudit: {} -> {}",
                report.id,
                report.status,
                report.old_commit,
                report.new_commit,
                report.old_audit.level,
                report.new_audit.level
            );
            for file in &report.files {
                text.push_str(&format!(
                    "\n{} {}\nold={:?}\nnew={:?}",
                    file.change,
                    file.path.display(),
                    file.old,
                    file.new
                ));
                if let Some(diff) = &file.unified_diff {
                    text.push('\n');
                    text.push_str(diff);
                }
                if file.truncated {
                    text.push_str("\nWARNING: text omitted because the preview size limit was reached; hashes remain available.");
                }
            }
            for finding in &report.new_audit.details {
                text.push_str(&format!(
                    "\nAudit finding: {}",
                    serde_json::to_string(finding)
                        .map_err(|e| Error::new("JSON_ERROR", e.to_string()))?
                ));
            }
            for warning in &report.warnings {
                text.push_str(&format!(
                    "\nWARNING: {}\nOriginal before: {}\nOriginal after: {}",
                    warning.reason_code,
                    warning.old_description.as_deref().unwrap_or("-"),
                    warning.new_description.as_deref().unwrap_or("-")
                ));
            }
            if report.requires_confirmation {
                text.push_str(&format!(
                    "\nReview the preview, then repeat with --confirm {}{}",
                    report.preview_token,
                    if report.new_audit.level == "risky" {
                        " --allow-risky"
                    } else {
                        ""
                    }
                ));
            }
            output::emit(&report, args.json, &text)?;
            return Ok(if report.requires_confirmation && !args.dry_run {
                2
            } else {
                0
            });
        }
        Commands::RefreshAudit { confirm } => {
            let report = refresh_audit(&vault, &home, confirm.as_deref(), args.dry_run)?;
            output::emit(&report, args.json, &refresh_audit_text(&report))?;
            return Ok(if report.requires_confirmation && !args.dry_run {
                2
            } else {
                0
            });
        }
        Commands::Build { id } => {
            let ids = id.iter().cloned().collect::<Vec<_>>();
            let report = kinana_core::build::build_command(&vault, &ids, args.dry_run)?;
            let mut text = report
                .items
                .iter()
                .map(|item| {
                    format!(
                        "{}\t{}\t{}",
                        item.id,
                        item.description_source,
                        if item.changed { "changed" } else { "unchanged" }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            text.push_str(&finder_note(&report.finder_files_removed, report.dry_run));
            output::emit(&report, args.json, &text)?;
        }
        Commands::Groups => {
            let report = kinana_core::groups::discover(&vault)?;
            let mut text = report
                .candidates
                .iter()
                .map(|candidate| {
                    format!(
                        "{}\t{}\t{}",
                        candidate.skill_ids.join(", "),
                        candidate
                            .category
                            .as_deref()
                            .unwrap_or("(multiple categories)"),
                        candidate.reasons.join("; ")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            if text.is_empty() {
                text.push_str("No same-function candidates.");
            }
            for warning in &report.warnings {
                text.push_str(&format!("\nWarning: {warning}"));
            }
            output::emit(&report, args.json, &text)?;
        }
        Commands::Bundle { command } => match command {
            BundleCommands::Create { file } => {
                let report = kinana_core::bundles::create(&vault, file, args.dry_run)?;
                output::emit(&report, args.json, &bundle_text(&report))?;
            }
            BundleCommands::Edit { name, file } => {
                let report = kinana_core::bundles::edit(&vault, name, file, args.dry_run)?;
                output::emit(&report, args.json, &bundle_text(&report))?;
            }
            BundleCommands::List => {
                let report = kinana_core::bundles::views(&vault, kinana_core::bundles::list(&vault)?);
                let text = report
                    .iter()
                    .map(|bundle| {
                        format!(
                            "{}\t{}\t{} skills ({} risky)",
                            bundle.name,
                            bundle.title,
                            bundle.skills.len(), bundle.risky_members
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                output::emit(&report, args.json, &text)?;
            }
            BundleCommands::Show { name } => {
                let report = kinana_core::bundles::views(&vault, vec![kinana_core::bundles::read(&vault, name)?]).remove(0);
                let mut text = format!(
                    "{} — {}\nAgents: {}",
                    report.name,
                    report.title,
                    report.agents.join(", ")
                );
                for skill in &report.skills {
                    text.push_str(&format!(
                        "\n{}\t{}\t{}\t{}",
                        skill.member.id,
                        skill.member.role,
                        skill.member.group.as_deref().unwrap_or(""), skill.audit_level
                    ));
                }
                output::emit(&report, args.json, &text)?;
            }
            BundleCommands::Delete { name } => {
                let report = kinana_core::bundles::delete(&vault, name, args.dry_run)?;
                let text = format!(
                    "{} bundle {}",
                    if report.dry_run {
                        "Would delete"
                    } else {
                        "Deleted"
                    },
                    report.name
                );
                output::emit(&report, args.json, &text)?;
            }
        },
        Commands::Undo => {
            let report = vault.undo(args.dry_run)?;
            let mut text = format!(
                "{} {}",
                if report.dry_run {
                    "Would revert"
                } else {
                    "Reverted"
                },
                report.reverted_commit
            );
            text.push_str(&finder_note(&report.finder_files_removed, report.dry_run));
            output::emit(&report, args.json, &text)?;
        }
        Commands::Doctor {
            repair: true,
            confirm,
            ..
        } => {
            let report =
                kinana_core::repair::repair(&vault, &home, confirm.as_deref(), args.dry_run)?;
            output::emit(&report, args.json, &repair_text(&report))?;
            return Ok(0);
        }
        Commands::Doctor { rebuild_index, .. } => {
            let report = doctor::run(&vault, &home, *rebuild_index, args.dry_run)?;
            let mut text = report
                .checks
                .iter()
                .map(|c| {
                    // Messages name vault paths, which may come from a skill's
                    // author: nothing in them may act on the terminal.
                    format!(
                        "{} {}: {}",
                        if c.passed { "OK" } else { "FAIL" },
                        c.name,
                        output::single_line(&c.message)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            if let Some(rebuild) = &report.rebuild {
                text.push_str(&format!(
                    "\n{} index ({}): {} skills, {} group memberships, {} projects, {} activations",
                    if rebuild.dry_run {
                        "Would rebuild"
                    } else {
                        "Rebuilt"
                    },
                    rebuild.mode,
                    rebuild.skills,
                    rebuild.groups,
                    rebuild.projects,
                    rebuild.activations
                ));
                if let Some(backup) = &rebuild.backup {
                    text.push_str(&format!("\nPreserved previous index: {}", backup.display()));
                }
            }
            for warning in &report.warnings {
                text.push_str(&format!("\nWarning: {warning}"));
            }
            output::emit(&report, args.json, &text)?;
            return Ok(if report.healthy || (*rebuild_index && args.dry_run) {
                0
            } else {
                1
            });
        }
    }
    Ok(0)
}

/// One line per skill and per bundle. Every field is text written by a skill
/// or bundle author, so each one is confined to its own tab-separated column.
/// Build this version's index where an upgrade left none. Best effort: the
/// command still runs and reports the index as it finds it, and the reason a
/// build did not happen is shown rather than swallowed.
fn prepare_index(vault: &Vault) {
    // A writer lease held elsewhere is most likely another command building
    // this same index; wait for it briefly instead of finding no index.
    let mut waits = 30;
    loop {
        match kinana_core::index::ensure_built(vault) {
            Ok(_) => return,
            Err(error) if error.code == "STATE_BUSY" && waits > 0 => {
                waits -= 1;
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(error) => {
                let _ = output::warning(&format!(
                    "the index for this version was not built ({}: {})",
                    error.code, error.message
                ));
                return;
            }
        }
    }
}

/// Names the Finder files `build` or `undo` removed before checking hashes.
/// Each name comes from the disk, so nothing in it may act on the terminal.
/// Every path in the shared escape form, every list complete: what the user
/// confirms is exactly what will be done.
fn repair_text(report: &kinana_core::DoctorRepairReport) -> String {
    use kinana_core::names::visible;
    let mut text = String::new();
    let section = |text: &mut String, title: &str, paths: &[String]| {
        if paths.is_empty() {
            return;
        }
        text.push_str(&format!("{title} ({}):\n", paths.len()));
        for path in paths {
            text.push_str(&format!("  {}\n", visible(path)));
        }
    };
    let preview = report.dry_run;
    section(
        &mut text,
        if preview {
            "Would return to the last commit"
        } else {
            "Returned to the last commit"
        },
        &report.restore,
    );
    section(
        &mut text,
        if preview {
            "Would unstage (what is on disk stays)"
        } else {
            "Unstaged (what is on disk stays)"
        },
        &report.unstage,
    );
    section(
        &mut text,
        if preview {
            "Would recreate empty directories"
        } else {
            "Recreated empty directories"
        },
        &report.recreate_directories,
    );
    section(
        &mut text,
        if preview {
            "Would move private stages into the backup"
        } else {
            "Moved private stages into the backup"
        },
        &report.staging_removed,
    );
    section(
        &mut text,
        "Left alone, move these out of the vault yourself first",
        &report.blocked,
    );
    section(
        &mut text,
        "Not in the last commit, left where they are",
        &report.untracked,
    );
    if !preview {
        section(&mut text, "Backed up before replacing", &report.backed_up);
    }
    if let Some(directory) = &report.backup_directory {
        text.push_str(&format!(
            "{}: {}\n",
            if preview { "Backup folder" } else { "Backup" },
            visible(&directory.to_string_lossy())
        ));
    }
    for warning in &report.warnings {
        text.push_str(&format!("Warning: {}\n", output::single_line(warning)));
    }
    if preview {
        if report.preview_token.is_empty() {
            text.push_str("Nothing to repair.");
        } else {
            text.push_str(&format!(
                "To do exactly this, run:\nkinana doctor --repair --confirm {}",
                report.preview_token
            ));
        }
    } else if let Some(after) = &report.after {
        text.push_str(if after.healthy {
            "Doctor now reports a healthy vault."
        } else {
            "Doctor still reports problems; run kinana doctor (kinana build regenerates dist)."
        });
    }
    text.trim_end().to_owned()
}

fn finder_note(files: &[String], dry_run: bool) -> String {
    if files.is_empty() {
        return String::new();
    }
    format!(
        "\n{} Finder .DS_Store file(s) that were the only difference in their skill: {}",
        if dry_run { "Would remove" } else { "Removed" },
        files
            .iter()
            .map(|file| kinana_core::names::visible(file))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn brief_text(report: &kinana_core::catalog::BriefCatalog) -> String {
    use output::single_line as clean;
    let mut text = format!(
        "Skills: {} of {} (page {} of {})",
        report.skills.len(),
        report.total,
        report.page,
        report.pages
    );
    for skill in &report.skills {
        text.push_str(&format!(
            "\n{}\t{}\tstack={}\tstage={}\trole={}\tuse={}\tnot={}\tdesc={}",
            clean(&skill.id),
            clean(&skill.category),
            clean(&skill.stack.join(",")),
            clean(&skill.stage.join(",")),
            clean(&skill.role),
            clean(&skill.when_to_use),
            clean(&skill.when_not),
            clean(&skill.description)
        ));
    }
    text.push_str(&format!(
        "\nBundles: {} of {}",
        report.bundles.len(),
        report.bundles_total
    ));
    for bundle in &report.bundles {
        match (&bundle.title, &bundle.skills) {
            (Some(title), Some(skills)) => text.push_str(&format!(
                "\n{}\t{}\t{}",
                clean(&bundle.name),
                clean(title),
                skills
                    .iter()
                    .map(|member| format!("{}={}", clean(&member.id), clean(&member.role)))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            _ => text.push_str(&format!(
                "\n{}\tmembers={}",
                clean(&bundle.name),
                bundle.members
            )),
        }
    }
    if !report.hint.is_empty() {
        text.push_str(&format!("\nHint: {}", report.hint));
    }
    text
}

fn import_text(report: &ImportReport) -> String {
    let mut text = format!(
        "{}Imported: {}  Duplicates: {}  Variants: {}",
        if report.dry_run { "Preview — " } else { "" },
        report.imported.len(),
        report.duplicates.len(),
        report.variants.len()
    );
    if report.requires_preparation {
        text.push_str("\nSource preparation is required before listing and inspecting skills; no files were changed.");
    }
    for item in &report.items {
        text.push_str(&format!(
            "\n{}\t{}\t{:?}\t{}\t{}",
            kinana_core::names::visible(item.name.as_deref().unwrap_or("(invalid)")),
            kinana_core::names::visible(item.id.as_deref().unwrap_or("-")),
            item.status,
            item.audit.level,
            kinana_core::names::visible(&item.path.to_string_lossy())
        ));
    }
    for warning in &report.warnings {
        text.push_str(&format!("\nWarning: {warning}"));
    }
    text
}

fn bundle_text(report: &kinana_core::BundleSaveReport) -> String {
    let mut text = format!(
        "{} {}: {}\nPath: {}\nChanged: {}  Committed: {}",
        if report.dry_run { "Would" } else { "Bundle" },
        report.operation,
        report.bundle.name,
        report.path.display(),
        report.changed,
        report.committed
    );
    for warning in &report.warnings {
        text.push_str(&format!("\nWarning: {warning}"));
    }
    text
}

/// `refresh-audit`: the stored audits next to the current rules (rules version 3).
#[derive(serde::Serialize)]
struct RefreshAuditReport {
    #[serde(flatten)]
    upgrade: kinana_core::audit::RulesUpgrade,
    changed: bool,
    preview_token: String,
    requires_confirmation: bool,
    dry_run: bool,
    committed: bool,
}

fn refresh_audit(
    vault: &Vault,
    home: &std::path::Path,
    confirm: Option<&str>,
    dry_run: bool,
) -> Result<RefreshAuditReport> {
    use kinana_core::settings::{self, Action};
    let plan = settings::preview(vault, home, &Action::RefreshAudit)?;
    let upgrade = plan.audit.clone().unwrap_or_default();
    let mut report = RefreshAuditReport {
        upgrade,
        changed: plan.changed,
        preview_token: plan.token.clone(),
        requires_confirmation: plan.changed,
        dry_run,
        committed: false,
    };
    let Some(token) = confirm else {
        return Ok(report);
    };
    if token != plan.token {
        return Err(Error::new(
            "SETTINGS_PREVIEW_CHANGED",
            "The vault changed since the preview; preview the re-audit again",
        ));
    }
    report.requires_confirmation = false;
    if !dry_run {
        report.committed =
            settings::apply_controlled(vault, home, &Action::RefreshAudit, token, None)?.committed;
    }
    Ok(report)
}

fn refresh_audit_text(report: &RefreshAuditReport) -> String {
    let upgrade = &report.upgrade;
    let mut text = format!(
        "Audit rules version {}: {} skill(s) audited with older rules. Raised: {}  Lowered: {}  Unchanged: {}",
        upgrade.rules_version,
        upgrade.older,
        upgrade.raised.len(),
        upgrade.lowered.len(),
        upgrade.unchanged
    );
    for (label, changes) in [("Raised", &upgrade.raised), ("Lowered", &upgrade.lowered)] {
        for change in changes {
            text.push_str(&format!(
                "\n{label}: {} {} -> {}",
                change.id, change.from, change.to
            ));
        }
    }
    if report.committed {
        text.push_str("\nRe-audited every skill in one commit; `kinana undo` reverts it.");
    } else if report.requires_confirmation {
        text.push_str(&format!(
            "\nNothing changed. To re-audit every skill, repeat with --confirm {}",
            report.preview_token
        ));
    } else if !report.changed {
        text.push_str("\nStored audits already match the current rules.");
    }
    text
}
