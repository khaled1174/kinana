//! Classification quality evaluation (0.3, batch 2; owner decisions 3-9 of
//! 2026-11-18). Three arms on a private copy of the owner's vault:
//! K = keywords (no calls), A = names-only template (prompt 1, what the
//! product sends), B = template with the taxonomy guide (prompt 2). The
//! response schema, with `category_reason`, is the same in A and B. The
//! product switches to template 2 only after this gate passes (owner,
//! 2026-11-18); the switch is `agent_runner::PRODUCT_PROMPT_VERSION`.
//!
//! Steps, in order, each a separate command:
//!   prepare   clone the source vault at a commit into <work>, add the extra skills
//!   estimate  count calls and input tokens per arm; no agent is started
//!   run       one live arm; refused unless the labels are committed and complete,
//!             KINANA_EVAL_LIVE=1 is set and --confirm-calls names the exact count
//!   report    compare every arm with the owner's labels; no agent is started.
//!             Writes report.md and b-disagreements.json: every skill where B
//!             differs from the owner's label, with B's category_reason.
//!
//! The source vault is only read (`git clone`); every write lands in <work>.
use kinana_core::{
    ClassifyOptions, ImportOptions, Result,
    classify::{self, PromptTemplate},
    edit, import,
    vault::Vault,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Most model calls in one round: 30 planned, each may retry once (decision 5).
const MAX_CALLS: usize = 60;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
struct Label {
    id: String,
    category: String,
    #[serde(default)]
    stack: Vec<String>,
    #[serde(default)]
    stage: Vec<String>,
}
#[derive(Debug, Deserialize)]
struct Labels {
    #[serde(default)]
    skill: Vec<Label>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    source_vault: String,
    commit: String,
    ids: Vec<String>,
    imported: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ArmItem {
    id: String,
    category: String,
    stack: Vec<String>,
    stage: Vec<String>,
    classified_by: String,
    category_reason: String,
    prompt_version: Option<u32>,
    reason_code: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
struct ArmResult {
    arm: String,
    agent_calls: usize,
    claude_version: Option<String>,
    /// The labels the arm ran against: their commit and a hash of the file.
    /// `report` refuses arms run against different labels.
    #[serde(default)]
    labels: String,
    /// The stop code when the arm did not classify every skill (since 0.3
    /// a named Claude stops instead of saving keywords). Such an arm still
    /// counts its calls but `report` refuses to score it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stopped: Option<String>,
    items: Vec<ArmItem>,
}

fn labels_identity(path: &Path, commit: &str) -> Result<String> {
    let hash = blake3::hash(&fs::read(path)?).to_hex();
    Ok(format!("{commit}:{hash}"))
}

fn fail(message: impl Into<String>) -> kinana_core::Error {
    kinana_core::Error::new("EVAL_REFUSED", message)
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}
fn args_all(args: &[String], name: &str) -> Vec<String> {
    args.iter()
        .enumerate()
        .filter(|(_, a)| *a == name)
        .filter_map(|(i, _)| args.get(i + 1).cloned())
        .collect()
}
fn required(args: &[String], name: &str) -> Result<String> {
    arg(args, name).ok_or_else(|| fail(format!("missing {name}")))
}

fn vault_at(work: &Path, name: &str) -> Result<Vault> {
    let vault = Vault::new(
        work.join(name),
        work.join("state").join(format!("{name}-index.db")),
    );
    if !vault.index_path.exists() {
        let records = vault
            .records()
            .map_err(|e| fail(format!("reading the copied vault: {}", e.message)))?;
        kinana_core::index::rebuild_for_vault(&vault.index_path, &vault.root, &records)
            .map_err(|e| fail(format!("building the copy's index: {}", e.message)))?;
    }
    Ok(vault)
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_symlink() {
            return Err(fail(format!(
                "{} is a symbolic link; copy skills without links",
                entry.path().display()
            )));
        } else if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").arg("-C").arg(dir).args(args).output()?;
    if !output.status.success() {
        return Err(fail(format!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn read_labels(path: &Path) -> Result<Vec<Label>> {
    let labels: Labels = toml::from_str(&fs::read_to_string(path)?)?;
    Ok(labels.skill)
}

/// Labels are usable for a live run only when every skill has a category,
/// IDs are unique, and the file is committed unchanged (decision 9).
fn check_labels(path: &Path, ids: &[String], categories: &[String]) -> Result<String> {
    let labels = read_labels(path)?;
    let named = labels.iter().map(|l| l.id.clone()).collect::<BTreeSet<_>>();
    if named.len() != labels.len() {
        return Err(fail("a skill is labelled twice"));
    }
    if named != ids.iter().cloned().collect::<BTreeSet<_>>() {
        return Err(fail("the labels must name exactly the evaluation skills"));
    }
    for label in &labels {
        if !categories.contains(&label.category) {
            return Err(fail(format!(
                "'{}' has no valid category label yet",
                label.id
            )));
        }
    }
    let directory = path.parent().unwrap_or(Path::new("."));
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    if !git(directory, &["status", "--porcelain", "--", &name])?.is_empty() {
        return Err(fail("commit the labels before any live run"));
    }
    let commit = git(directory, &["log", "-1", "--format=%H", "--", &name])?;
    if commit.is_empty() {
        return Err(fail("the labels file is not committed"));
    }
    Ok(commit)
}

fn prepare(args: &[String]) -> Result<()> {
    let work = PathBuf::from(required(args, "--work")?);
    let source = PathBuf::from(required(args, "--vault-source")?);
    let commit = required(args, "--commit")?;
    let ids = args_all(args, "--id");
    let extra = args_all(args, "--add-skill");
    if work.exists() {
        return Err(fail("--work must not exist yet"));
    }
    fs::create_dir_all(work.join("state"))?;
    // Read-only for the source: a clone, then a checkout of the commit.
    let status = Command::new("git")
        .args(["clone", "--quiet", "--no-hardlinks"])
        .arg(&source)
        .arg(work.join("pristine"))
        .status()?;
    if !status.success() {
        return Err(fail("git clone of the source vault failed"));
    }
    git(
        &work.join("pristine"),
        &["checkout", "--quiet", "-B", "eval", &commit],
    )?;
    git(&work.join("pristine"), &["remote", "remove", "origin"])?;
    // A clone lacks the ignored `dist/` (and `bundles/` when it held none).
    for directory in ["dist", "bundles"] {
        fs::create_dir_all(work.join("pristine").join(directory))?;
    }
    let vault = vault_at(&work, "pristine")?;
    let mut imported = Vec::new();
    for skill in &extra {
        let from = PathBuf::from(skill);
        let name = from
            .file_name()
            .ok_or_else(|| fail("bad --add-skill"))?
            .to_owned();
        let copy = work.join("sources").join(&name);
        copy_tree(&from, &copy)?;
        let report = import::add(
            &vault,
            &copy.to_string_lossy(),
            &work.join("state"),
            &ImportOptions::default(),
        )
        .map_err(|e| {
            fail(format!(
                "adding {}: {}: {}",
                copy.display(),
                e.code,
                e.message
            ))
        })?;
        imported.extend(report.imported);
    }
    let existing = vault
        .records()?
        .into_iter()
        .map(|r| r.id)
        .collect::<BTreeSet<_>>();
    for id in &ids {
        if !existing.contains(id) {
            return Err(fail(format!("'{id}' is not in the prepared vault")));
        }
    }
    let manifest = Manifest {
        source_vault: source.display().to_string(),
        commit,
        ids,
        imported,
    };
    fs::write(
        work.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )?;
    println!("{}", serde_json::to_string_pretty(&manifest).unwrap());
    Ok(())
}

fn manifest(work: &Path) -> Result<Manifest> {
    serde_json::from_str(&fs::read_to_string(work.join("manifest.json"))?)
        .map_err(|e| fail(e.to_string()))
}

fn options(ids: &[String], agent: &str, dry_run: bool) -> ClassifyOptions {
    ClassifyOptions {
        ids: ids.to_vec(),
        agent: agent.into(),
        dry_run,
        confirmed: true,
        ..Default::default()
    }
}

fn template(arm: &str) -> Result<PromptTemplate> {
    match arm {
        "A" => Ok(PromptTemplate::NamesOnly),
        "B" => Ok(PromptTemplate::Definitions),
        _ => Err(fail("--arm is A or B")),
    }
}

/// Extra output tokens the reason may cost: at most 160 characters each,
/// counted with the engine's rule of one token per four bytes, ASCII assumed.
fn reason_tokens(calls: usize) -> u64 {
    (calls as u64) * (kinana_core::agent_runner::CATEGORY_REASON_LIMIT as u64).div_ceil(4)
}

fn estimate(args: &[String]) -> Result<()> {
    let work = PathBuf::from(required(args, "--work")?);
    let manifest = manifest(&work)?;
    let vault = vault_at(&work, "pristine")?;
    let mut arms = BTreeMap::new();
    for arm in ["A", "B"] {
        let report = classify::classify_with_template(
            &vault,
            &options(&manifest.ids, "claude", true),
            template(arm)?,
        )?;
        arms.insert(
            arm,
            serde_json::json!({
                "calls": report.estimated_agent_calls,
                "input_tokens": report.estimated_tokens.total_input_tokens,
                "retry_limit_input_tokens": report.estimated_tokens.retry_limit_input_tokens,
                "fixed_tokens_per_call": report.estimated_tokens.fixed_tokens_per_call,
            }),
        );
    }
    let calls = arms
        .values()
        .map(|a| a["calls"].as_u64().unwrap())
        .sum::<u64>();
    let summary = serde_json::json!({
        "skills": manifest.ids.len(),
        "arms": arms,
        "K_calls": 0,
        "total_calls": calls,
        "max_calls": calls * 2,
        "max_allowed": MAX_CALLS,
        "guide_extra_input_tokens": arms["B"]["input_tokens"].as_u64().unwrap()
            .saturating_sub(arms["A"]["input_tokens"].as_u64().unwrap()),
        "reason_output_tokens_at_most": reason_tokens(calls as usize),
        "agent_calls": 0,
    });
    fs::write(
        work.join("estimate.json"),
        serde_json::to_string_pretty(&summary).unwrap(),
    )?;
    println!("{}", serde_json::to_string_pretty(&summary).unwrap());
    Ok(())
}

fn arm_items(
    vault: &Vault,
    ids: &[String],
    report: &kinana_core::ClassifyReport,
) -> Result<Vec<ArmItem>> {
    let mut items = Vec::new();
    for id in ids {
        let metadata = edit::read_metadata(vault, id)?;
        let reported = report.items.iter().find(|i| &i.id == id);
        items.push(ArmItem {
            id: id.clone(),
            category: metadata.classification.category.clone(),
            stack: metadata.classification.stack.clone(),
            stage: metadata.classification.stage.clone(),
            // A skill this arm did not classify keeps the copy's earlier
            // classification: never count it as the arm's answer.
            classified_by: if reported.is_some() {
                metadata.classification.classified_by.clone()
            } else {
                "not-classified-by-this-arm".into()
            },
            category_reason: metadata
                .classification_context
                .as_ref()
                .map(|c| c.category_reason.clone())
                .unwrap_or_default(),
            prompt_version: metadata
                .classification_context
                .as_ref()
                .map(|c| c.prompt_version),
            reason_code: reported.and_then(|i| i.reason_code.clone()),
        });
    }
    Ok(items)
}

fn fresh_copy(work: &Path, name: &str) -> Result<Vault> {
    let target = work.join(name);
    if target.exists() {
        return Err(fail(format!("{name} already exists; one run per arm")));
    }
    let status = Command::new("git")
        .args(["clone", "--quiet", "--no-hardlinks"])
        .arg(work.join("pristine"))
        .arg(&target)
        .status()?;
    if !status.success() {
        return Err(fail("could not copy the prepared vault"));
    }
    git(&target, &["remote", "remove", "origin"])?;
    for directory in ["dist", "bundles"] {
        fs::create_dir_all(target.join(directory))?;
    }
    vault_at(work, name)
}

fn keywords(args: &[String]) -> Result<()> {
    let work = PathBuf::from(required(args, "--work")?);
    let manifest = manifest(&work)?;
    let vault = fresh_copy(&work, "arm-K")?;
    let report = classify::classify(&vault, &options(&manifest.ids, "keywords", false))?;
    let result = ArmResult {
        arm: "K".into(),
        agent_calls: report.agent_calls,
        claude_version: None,
        labels: String::new(),
        stopped: None,
        items: arm_items(&vault, &manifest.ids, &report)?,
    };
    fs::write(
        work.join("arm-K.json"),
        serde_json::to_string_pretty(&result).unwrap(),
    )?;
    println!(
        "arm K: {} skills, agent_calls = {}",
        result.items.len(),
        result.agent_calls
    );
    Ok(())
}

fn run(args: &[String]) -> Result<()> {
    let work = PathBuf::from(required(args, "--work")?);
    let arm = required(args, "--arm")?;
    // Checked before anything else can run or be copied.
    let prompt_template = template(&arm)?;
    let labels = PathBuf::from(required(args, "--labels")?);
    let manifest = manifest(&work)?;
    let pristine = vault_at(&work, "pristine")?;
    let labels_commit = check_labels(&labels, &manifest.ids, &pristine.config()?.categories)?;
    if std::env::var("KINANA_EVAL_LIVE").as_deref() != Ok("1") {
        return Err(fail(
            "a live arm needs KINANA_EVAL_LIVE=1 (owner permission)",
        ));
    }
    let planned = manifest.ids.len();
    if arg(args, "--confirm-calls").and_then(|v| v.parse::<usize>().ok()) != Some(planned) {
        return Err(fail(format!(
            "confirm the exact count with --confirm-calls {planned} (one retry each at most)"
        )));
    }
    let previous = ["A", "B"]
        .iter()
        .filter_map(|a| fs::read_to_string(work.join(format!("arm-{a}.json"))).ok())
        .filter_map(|text| serde_json::from_str::<ArmResult>(&text).ok())
        .map(|r| r.agent_calls)
        .sum::<usize>();
    if previous + planned * 2 > MAX_CALLS {
        return Err(fail(format!(
            "this arm could exceed the {MAX_CALLS}-call limit ({previous} already used)"
        )));
    }
    let identity = labels_identity(&labels, &labels_commit)?;
    let vault = fresh_copy(&work, &format!("arm-{arm}"))?;
    // The same trusted lookup the classification uses, not a PATH search.
    let plan = kinana_core::executables::SearchPlan::current();
    let version = kinana_core::executables::find_in(&plan, "claude", &[])
        .found
        .and_then(|claude| Command::new(claude).arg("--version").output().ok())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    let report = classify::classify_with_template(
        &vault,
        &options(&manifest.ids, "claude", false),
        prompt_template,
    )?;
    let result = ArmResult {
        arm: arm.clone(),
        agent_calls: report.agent_calls,
        claude_version: version,
        labels: identity,
        stopped: report.stopped.as_ref().map(|s| s.reason_code.clone()),
        items: arm_items(&vault, &manifest.ids, &report)?,
    };
    fs::write(
        work.join(format!("arm-{arm}.json")),
        serde_json::to_string_pretty(&result).unwrap(),
    )?;
    println!(
        "arm {arm}: agent_calls = {}, labels commit {labels_commit}",
        result.agent_calls
    );
    Ok(())
}

fn jaccard(a: &[String], b: &[String]) -> f64 {
    let a = a.iter().collect::<BTreeSet<_>>();
    let b = b.iter().collect::<BTreeSet<_>>();
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    a.intersection(&b).count() as f64 / a.union(&b).count() as f64
}

#[derive(Debug, Serialize, PartialEq)]
struct Score {
    arm: String,
    category_matches: usize,
    total: usize,
    stack_exact: usize,
    stack_jaccard: f64,
    stage_exact: usize,
    stage_jaccard: f64,
    agent_calls: usize,
    not_by_agent: Vec<String>,
}

fn score(result: &ArmResult, labels: &[Label]) -> Score {
    let mut s = Score {
        arm: result.arm.clone(),
        category_matches: 0,
        total: labels.len(),
        stack_exact: 0,
        stack_jaccard: 0.0,
        stage_exact: 0,
        stage_jaccard: 0.0,
        agent_calls: result.agent_calls,
        not_by_agent: vec![],
    };
    for label in labels {
        let Some(item) = result.items.iter().find(|i| i.id == label.id) else {
            continue;
        };
        s.category_matches += usize::from(item.category == label.category);
        let set = |v: &[String]| v.iter().cloned().collect::<BTreeSet<_>>();
        s.stack_exact += usize::from(set(&item.stack) == set(&label.stack));
        s.stage_exact += usize::from(set(&item.stage) == set(&label.stage));
        s.stack_jaccard += jaccard(&item.stack, &label.stack);
        s.stage_jaccard += jaccard(&item.stage, &label.stage);
        // The arm's answer: by the agent, from this arm's own template.
        let template = match result.arm.as_str() {
            "A" => Some(1),
            "B" => Some(2),
            _ => None,
        };
        if result.arm != "K" && (item.classified_by != "claude" || item.prompt_version != template)
        {
            s.not_by_agent.push(item.id.clone());
        }
    }
    if !labels.is_empty() {
        s.stack_jaccard /= labels.len() as f64;
        s.stage_jaccard /= labels.len() as f64;
    }
    s
}

/// One skill where arm B chose another category than the owner's label.
#[derive(Debug, Serialize, PartialEq)]
struct Disagreement {
    id: String,
    label: String,
    model: String,
    category_reason: String,
}

/// Every skill where B differs from the owner's label, with the reason the
/// model gave (owner, 2026-11-18). A skill B did not return is listed too.
fn b_disagreements(b: &ArmResult, labels: &[Label]) -> Vec<Disagreement> {
    labels
        .iter()
        .filter_map(|label| {
            let item = b.items.iter().find(|i| i.id == label.id);
            match item {
                Some(item) if item.category == label.category => None,
                Some(item) => Some(Disagreement {
                    id: label.id.clone(),
                    label: label.category.clone(),
                    model: item.category.clone(),
                    category_reason: item.category_reason.clone(),
                }),
                None => Some(Disagreement {
                    id: label.id.clone(),
                    label: label.category.clone(),
                    model: String::new(),
                    category_reason: String::new(),
                }),
            }
        })
        .collect()
}

/// Decision 3: B matches at least 13 of 15 categories and is not below A;
/// B's stage Jaccard is not below A's; every A and B skill came from the
/// agent (a refused isolation gate falls back and is counted here).
/// Owner, 2026-11-18: the gate also needs the report of B's disagreements,
/// each with the model's reason; one without a reason cannot be reviewed.
fn verdict(a: &Score, b: &Score, disagreements: &[Disagreement]) -> (bool, Vec<String>) {
    let mut reasons = vec![];
    let unexplained = disagreements
        .iter()
        .filter(|d| d.category_reason.trim().is_empty())
        .map(|d| d.id.clone())
        .collect::<Vec<_>>();
    if !unexplained.is_empty() {
        reasons.push(format!(
            "B disagrees without a category_reason for: {unexplained:?}"
        ));
    }
    let needed = (b.total * 13).div_ceil(15);
    if b.category_matches < needed {
        reasons.push(format!(
            "B matched {} of {}, needs {needed}",
            b.category_matches, b.total
        ));
    }
    if b.category_matches < a.category_matches {
        reasons.push(format!(
            "B ({}) is below A ({})",
            b.category_matches, a.category_matches
        ));
    }
    if b.stage_jaccard + 1e-9 < a.stage_jaccard {
        reasons.push(format!(
            "B stage Jaccard {:.3} is below A {:.3}",
            b.stage_jaccard, a.stage_jaccard
        ));
    }
    for s in [a, b] {
        if !s.not_by_agent.is_empty() {
            reasons.push(format!(
                "arm {} has skills not classified by the agent: {:?}",
                s.arm, s.not_by_agent
            ));
        }
    }
    (reasons.is_empty(), reasons)
}

/// Model text in a Markdown table cell: one line, no column breaks.
fn cell(text: &str) -> String {
    text.replace('|', "/").replace(['\n', '\r'], " ")
}

fn report(args: &[String]) -> Result<()> {
    let work = PathBuf::from(required(args, "--work")?);
    let labels = read_labels(Path::new(&required(args, "--labels")?))?;
    let load = |arm: &str| -> Result<ArmResult> {
        serde_json::from_str(&fs::read_to_string(work.join(format!("arm-{arm}.json")))?)
            .map_err(|e| fail(e.to_string()))
    };
    let (k, a, b) = (load("K")?, load("A")?, load("B")?);
    let path = PathBuf::from(required(args, "--labels")?);
    let directory = path.parent().unwrap_or(Path::new("."));
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    let commit = git(directory, &["log", "-1", "--format=%H", "--", &name])?;
    let current = labels_identity(&path, &commit)?;
    for arm in [&a, &b] {
        if let Some(code) = &arm.stopped {
            return Err(fail(format!(
                "arm {} stopped ({code}) before classifying every skill; it cannot be scored",
                arm.arm
            )));
        }
        if arm.labels != current {
            return Err(fail(format!(
                "arm {} ran against other labels ({}); the labels changed since",
                arm.arm, arm.labels
            )));
        }
    }
    let scores = [score(&k, &labels), score(&a, &labels), score(&b, &labels)];
    let disagreements = b_disagreements(&b, &labels);
    let (accepted, reasons) = verdict(&scores[1], &scores[2], &disagreements);
    fs::write(
        work.join("b-disagreements.json"),
        serde_json::to_string_pretty(&disagreements).map_err(|e| fail(e.to_string()))? + "\n",
    )?;
    let mut text = String::from(
        "# Classification evaluation report\n\n| Arm | Category | Stack exact | Stack Jaccard | Stage exact | Stage Jaccard | agent_calls |\n| --- | --- | --- | --- | --- | --- | --- |\n",
    );
    for s in &scores {
        text.push_str(&format!(
            "| {} | {}/{} | {} | {:.3} | {} | {:.3} | {} |\n",
            s.arm,
            s.category_matches,
            s.total,
            s.stack_exact,
            s.stack_jaccard,
            s.stage_exact,
            s.stage_jaccard,
            s.agent_calls
        ));
    }
    text.push_str(&format!(
        "\n**Verdict:** {}\n",
        if accepted {
            "accepted. The owner reads the B disagreement report below before template 2 is switched on (PRODUCT_PROMPT_VERSION).".to_string()
        } else {
            format!(
                "not accepted: {}. Template 2 does not ship in 0.3; the decision returns to the owner.",
                reasons.join("; ")
            )
        }
    ));
    text.push_str(&format!(
        "\n## Gate report: every skill where B differs from the owner's label ({})\n\n| Skill | Owner's label | B | B's category_reason |\n| --- | --- | --- | --- |\n",
        disagreements.len()
    ));
    for d in &disagreements {
        text.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            d.id,
            d.label,
            if d.model.is_empty() {
                "(no result)"
            } else {
                &d.model
            },
            cell(&d.category_reason)
        ));
    }
    if disagreements.is_empty() {
        text.push_str("| (none) | | | |\n");
    }
    text.push_str("\n## Where a model arm differs from the owner's label\n\n| Arm | Skill | Label | Model | Model's reason |\n| --- | --- | --- | --- | --- |\n");
    for result in [&a, &b] {
        for label in &labels {
            if let Some(item) = result.items.iter().find(|i| i.id == label.id)
                && item.category != label.category
            {
                text.push_str(&format!(
                    "| {} | {} | {} | {} | {} |\n",
                    result.arm,
                    label.id,
                    label.category,
                    item.category,
                    cell(&item.category_reason)
                ));
            }
        }
    }
    text.push_str("\nbackend-api has no skill in this set: the owner's vault holds no backend skill (decision 7).\n");
    fs::write(work.join("report.md"), &text)?;
    println!("{text}");
    Ok(())
}

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let result = match args.first().map(String::as_str) {
        Some("prepare") => prepare(&args),
        Some("estimate") => estimate(&args),
        Some("keywords") => keywords(&args),
        Some("run") => run(&args),
        Some("report") => report(&args),
        _ => Err(fail(
            "usage: prepare | estimate | keywords | run | report (see the file header)",
        )),
    };
    if let Err(error) = result {
        eprintln!("{}: {}", error.code, error.message);
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, category: &str, stage: &[&str], by: &str) -> ArmItem {
        ArmItem {
            id: id.into(),
            category: category.into(),
            stack: vec![],
            stage: stage.iter().map(|s| s.to_string()).collect(),
            classified_by: by.into(),
            category_reason: format!("because {category}"),
            prompt_version: None,
            reason_code: None,
        }
    }
    fn label(id: &str, category: &str, stage: &[&str]) -> Label {
        Label {
            id: id.into(),
            category: category.into(),
            stack: vec![],
            stage: stage.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn scores_and_the_acceptance_rule_follow_the_owner_decision() {
        let labels = (0..15)
            .map(|i| label(&format!("s{i}"), "design-ui", &["review"]))
            .collect::<Vec<_>>();
        let arm = |name: &str, wrong: usize, by: &str| ArmResult {
            arm: name.into(),
            agent_calls: 15,
            claude_version: None,
            labels: String::new(),
            stopped: None,
            items: (0..15)
                .map(|i| ArmItem {
                    prompt_version: if name == "A" { Some(1) } else { Some(2) },
                    ..item(
                        &format!("s{i}"),
                        if i < wrong { "other" } else { "design-ui" },
                        &["review"],
                        by,
                    )
                })
                .collect(),
        };
        let a = score(&arm("A", 3, "claude"), &labels);
        let b_arm = arm("B", 2, "claude");
        let b = score(&b_arm, &labels);
        let dis = b_disagreements(&b_arm, &labels);
        assert_eq!((a.category_matches, b.category_matches), (12, 13));
        assert_eq!(b.stage_exact, 15);
        assert!((b.stage_jaccard - 1.0).abs() < 1e-9);
        assert!(verdict(&a, &b, &dis).0);
        // 12 of 15 is below the bar even when A is lower.
        let low = score(&arm("B", 3, "claude"), &labels);
        assert!(!verdict(&score(&arm("A", 4, "claude"), &labels), &low, &[]).0);
        // B below A is refused even above 13.
        assert!(
            !verdict(
                &score(&arm("A", 0, "claude"), &labels),
                &score(&arm("B", 1, "claude"), &labels),
                &[]
            )
            .0
        );
        // A skill that fell back to keywords counts against the round.
        let fallback = score(&arm("B", 0, "keywords"), &labels);
        assert_eq!(fallback.not_by_agent.len(), 15);
        assert!(!verdict(&a, &fallback, &[]).0);
        // An answer from another template, or one the arm did not produce
        // (it stopped and the copy kept an older Claude result), counts too
        // (review of 8d7a5a4, M2).
        let mut stale = arm("B", 0, "claude");
        stale.items[3].prompt_version = None;
        stale.items[4].prompt_version = Some(1);
        stale.items[5].classified_by = "not-classified-by-this-arm".into();
        let stale = score(&stale, &labels);
        assert_eq!(stale.not_by_agent, ["s3", "s4", "s5"]);
        assert!(!verdict(&a, &stale, &[]).0);
    }

    #[test]
    fn the_gate_reports_every_b_disagreement_with_its_reason() {
        let labels = vec![
            label("s0", "design-ui", &[]),
            label("s1", "testing-qa", &[]),
            label("s2", "docs-writing", &[]),
        ];
        let b = ArmResult {
            arm: "B".into(),
            agent_calls: 2,
            claude_version: None,
            labels: String::new(),
            stopped: None,
            items: vec![
                ArmItem {
                    prompt_version: Some(2),
                    ..item("s0", "design-ui", &[], "claude")
                },
                ArmItem {
                    prompt_version: Some(2),
                    ..item("s1", "web-frontend", &[], "claude")
                },
            ],
        };
        let dis = b_disagreements(&b, &labels);
        assert_eq!(
            dis,
            vec![
                Disagreement {
                    id: "s1".into(),
                    label: "testing-qa".into(),
                    model: "web-frontend".into(),
                    category_reason: "because web-frontend".into(),
                },
                Disagreement {
                    id: "s2".into(),
                    label: "docs-writing".into(),
                    model: String::new(),
                    category_reason: String::new(),
                },
            ]
        );
        // A disagreement without the model's reason fails the gate.
        let s = score(&b, &labels);
        let (ok, reasons) = verdict(&s, &s, &dis);
        assert!(!ok);
        assert!(
            reasons
                .iter()
                .any(|r| r.contains("without a category_reason") && r.contains("s2"))
        );
        assert_eq!(cell("a|b\nc"), "a/b c");
    }

    #[test]
    fn jaccard_of_two_empty_lists_is_one() {
        assert_eq!(jaccard(&[], &[]), 1.0);
        assert_eq!(jaccard(&["a".into()], &["a".into(), "b".into()]), 0.5);
    }

    #[test]
    fn the_reason_costs_at_most_forty_tokens_per_call() {
        assert_eq!(reason_tokens(30), 1200);
    }
}
