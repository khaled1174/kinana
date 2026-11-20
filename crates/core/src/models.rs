use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const CATEGORIES: &[&str] = &[
    "mobile",
    "web-frontend",
    "backend-api",
    "data-db",
    "devops",
    "testing-qa",
    "planning-specs",
    "design-ui",
    "docs-writing",
    "content-media",
    "automation-agents",
    "security",
    "other",
];

/// Default values of the two editable dimension lists in `box.toml`.
pub const STACKS: &[&str] = &[
    "react",
    "next",
    "expo",
    "react-native",
    "flutter",
    "swift",
    "rust",
    "python",
    "node",
    "docker",
];
pub const STAGES: &[&str] = &["plan", "design", "build", "test", "review", "ship"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillManifest {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoxConfig {
    pub version: u32,
    #[serde(default = "default_classification_agent")]
    pub classification_agent: String,
    #[serde(default = "record_locations")]
    pub record_found_in: bool,
    pub categories: Vec<String>,
    /// Absent in a `box.toml` written before 8.3; read as the defaults, never written on read.
    #[serde(default = "default_stacks")]
    pub stacks: Vec<String>,
    #[serde(default = "default_stages")]
    pub stages: Vec<String>,
    #[serde(default)]
    pub detected_agents: Vec<String>,
    #[serde(default)]
    pub scan_paths: Vec<PathBuf>,
}
impl Default for BoxConfig {
    fn default() -> Self {
        Self {
            version: 1,
            classification_agent: default_classification_agent(),
            record_found_in: true,
            categories: CATEGORIES.iter().map(|s| s.to_string()).collect(),
            stacks: default_stacks(),
            stages: default_stages(),
            detected_agents: vec![],
            scan_paths: vec![],
        }
    }
}

fn default_classification_agent() -> String {
    "claude".into()
}
fn record_locations() -> bool {
    true
}
fn default_stacks() -> Vec<String> {
    STACKS.iter().map(|s| s.to_string()).collect()
}
fn default_stages() -> Vec<String> {
    STAGES.iter().map(|s| s.to_string()).collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceInfo {
    #[serde(default = "unknown")]
    pub kind: String,
    #[serde(default, deserialize_with = "public_source_url")]
    pub url: Option<String>,
    #[serde(default)]
    pub subpath: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracking_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracking_kind: Option<String>,
    #[serde(default)]
    pub found_in: Vec<PathBuf>,
}
fn public_source_url<'de, D: serde::Deserializer<'de>>(
    reader: D,
) -> std::result::Result<Option<String>, D::Error> {
    Ok(Option::<String>::deserialize(reader)?
        .as_deref()
        .and_then(crate::gitlog::public_remote))
}
fn unknown() -> String {
    "unknown".into()
}
impl Default for SourceInfo {
    fn default() -> Self {
        Self {
            kind: unknown(),
            url: None,
            subpath: None,
            commit: None,
            tracking_ref: None,
            tracking_kind: None,
            found_in: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Classification {
    #[serde(default = "other")]
    pub category: String,
    /// Empty dimensions are omitted on write, so metadata from before 8.3
    /// gains no lines when an unrelated field is rewritten.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stack: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stage: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub when_to_use: String,
    #[serde(default)]
    pub when_not: String,
    #[serde(default = "pending")]
    pub classified_by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classified_from: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overlaps_with: Vec<String>,
}
fn other() -> String {
    "other".into()
}
fn pending() -> String {
    "pending".into()
}
impl Default for Classification {
    fn default() -> Self {
        Self {
            category: other(),
            stack: vec![],
            stage: vec![],
            tags: vec![],
            role: String::new(),
            when_to_use: String::new(),
            when_not: String::new(),
            classified_by: pending(),
            classified_from: None,
            overlaps_with: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Description {
    #[serde(default)]
    pub original: String,
    #[serde(default)]
    pub improved: Option<String>,
    #[serde(default = "original")]
    pub active: String,
    #[serde(default)]
    pub approved_at: Option<String>,
}
fn original() -> String {
    "original".into()
}
impl Default for Description {
    fn default() -> Self {
        Self {
            original: String::new(),
            improved: None,
            active: original(),
            approved_at: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Audit {
    #[serde(default)]
    pub rules_version: u32,
    #[serde(default)]
    pub details: Vec<AuditFinding>,
    #[serde(default = "unscanned")]
    pub level: String,
    #[serde(default)]
    pub findings: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditFinding {
    pub rule: String,
    pub strength: String,
    pub level: String,
    pub reason_code: String,
    pub path: PathBuf,
    pub line: Option<usize>,
    pub end_line: Option<usize>,
    pub excerpt: Option<String>,
}
fn unscanned() -> String {
    "unscanned".into()
}
impl Default for Audit {
    fn default() -> Self {
        Self {
            level: unscanned(),
            findings: vec![],
            rules_version: 0,
            details: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(from = "StoredSkillMetadata")]
pub struct SkillMetadata {
    pub id: String,
    #[serde(default)]
    pub pinned: bool,
    pub content_hash: String,
    pub added_at: String,
    #[serde(default)]
    pub source: SourceInfo,
    #[serde(default)]
    pub classification: Classification,
    #[serde(default)]
    pub description: Description,
    #[serde(default)]
    pub audit: Audit,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub empty_directories: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<ClassificationProposal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal_source_changed: Option<bool>,
}

/// A metadata file as it is stored. The `stack` and `stage` of a waiting
/// proposal live in `[proposal_dimensions]`, beside `[proposal]` and not in
/// it: the 0.1.0 beta rejects a key it does not know inside `[proposal]` and
/// can then read nothing in the vault, while it ignores and keeps a table it
/// does not know. `edit` writes the two tables; this reads them back.
#[derive(Deserialize)]
struct StoredSkillMetadata {
    id: String,
    #[serde(default)]
    pinned: bool,
    content_hash: String,
    added_at: String,
    #[serde(default)]
    source: SourceInfo,
    #[serde(default)]
    classification: Classification,
    #[serde(default)]
    description: Description,
    #[serde(default)]
    audit: Audit,
    #[serde(default)]
    empty_directories: Vec<PathBuf>,
    #[serde(default)]
    proposal: Option<ClassificationProposal>,
    #[serde(default)]
    proposal_source_changed: Option<bool>,
    #[serde(default)]
    proposal_dimensions: Option<ProposalDimensions>,
}
#[derive(Deserialize)]
struct ProposalDimensions {
    #[serde(default)]
    stack: Vec<String>,
    #[serde(default)]
    stage: Vec<String>,
}
impl From<StoredSkillMetadata> for SkillMetadata {
    fn from(stored: StoredSkillMetadata) -> Self {
        let mut proposal = stored.proposal;
        // Without a proposal the table is a leftover: the beta removes
        // `[proposal]` when it accepts or rejects and keeps what it ignores.
        if let (Some(proposal), Some(dimensions)) = (&mut proposal, stored.proposal_dimensions) {
            proposal.stack = dimensions.stack;
            proposal.stage = dimensions.stage;
        }
        Self {
            id: stored.id,
            pinned: stored.pinned,
            content_hash: stored.content_hash,
            added_at: stored.added_at,
            source: stored.source,
            classification: stored.classification,
            description: stored.description,
            audit: stored.audit,
            empty_directories: stored.empty_directories,
            proposal,
            proposal_source_changed: stored.proposal_source_changed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRecord {
    pub id: String,
    #[serde(default)]
    pub pinned: bool,
    pub name: String,
    pub description: String,
    pub category: String,
    /// Always present in JSON, empty for skills without the dimension.
    #[serde(default)]
    pub stack: Vec<String>,
    #[serde(default)]
    pub stage: Vec<String>,
    pub audit_level: String,
    pub content_hash: String,
    pub source: SourceInfo,
    pub added_at: String,
    pub role: String,
    pub tags: Vec<String>,
    /// Searchable, and compared with the index cache to detect stale rows.
    #[serde(default)]
    pub when_to_use: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScanStatus {
    New,
    Duplicate,
    Variant,
    Broken,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanEntry {
    pub path: PathBuf,
    pub name: Option<String>,
    pub description: Option<String>,
    pub content_hash: Option<String>,
    pub status: ScanStatus,
    pub skill_id: Option<String>,
    pub source: SourceInfo,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScanCounts {
    pub new: usize,
    pub duplicate: usize,
    pub variant: usize,
    pub broken: usize,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScanReport {
    pub entries: Vec<ScanEntry>,
    pub counts: ScanCounts,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ListFilter {
    pub pinned: bool,
    pub category: Option<String>,
    pub stack: Option<String>,
    pub stage: Option<String>,
    pub audit: Option<String>,
    pub agent: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SearchReport {
    pub query: String,
    pub items: Vec<SkillRecord>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ProjectSignal {
    pub path: PathBuf,
    pub category: String,
    pub reason: String,
}

/// One manifest fact that maps to a configured `stacks` value.
#[derive(Debug, Serialize)]
pub struct StackSignal {
    pub path: PathBuf,
    pub stack: String,
    pub reason: String,
}

/// Why a skill appears in `SuggestReport::skills`, in the same order.
#[derive(Debug, Serialize)]
pub struct SuggestMatch {
    pub id: String,
    pub matched_stacks: Vec<String>,
    pub matched_category: bool,
}

#[derive(Debug, Serialize)]
pub struct SuggestedBundle {
    pub bundle: Bundle,
    pub matched_categories: Vec<String>,
    pub match_count: usize,
}

#[derive(Debug, Serialize)]
pub struct SuggestReport {
    pub project: PathBuf,
    pub categories: Vec<String>,
    pub stacks: Vec<String>,
    pub signals: Vec<ProjectSignal>,
    pub stack_signals: Vec<StackSignal>,
    pub bundles: Vec<SuggestedBundle>,
    pub skills: Vec<SkillRecord>,
    pub matches: Vec<SuggestMatch>,
    pub warnings: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct SkillDetail {
    pub skill: SkillRecord,
    pub metadata: SkillMetadata,
    pub raw_skill_md: String,
    pub locations: Vec<PathBuf>,
    pub variants: Vec<ScanEntry>,
}
#[derive(Debug, Clone, Serialize)]
pub struct AgentInfo {
    pub name: String,
    pub available: bool,
    pub project_skills: String,
    pub global_skills: PathBuf,
}
#[derive(Debug, Serialize)]
pub struct InitReport {
    pub vault: PathBuf,
    pub index: PathBuf,
    pub agents: Vec<AgentInfo>,
    pub dry_run: bool,
    pub created: bool,
    pub planned_actions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub router: Option<RouterInstallReport>,
}

#[derive(Debug, Serialize)]
pub struct RouterTargetReport {
    pub agent: String,
    pub path: PathBuf,
    pub changed: bool,
    /// `install` (nothing there), `upgrade` (an owned, unmodified earlier
    /// version is replaced), `relocate` (the current text naming another CLI
    /// path, after the app moved) or `unchanged` (already the current text).
    pub action: String,
    /// For `relocate`: the CLI path the installed copy names now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_cli_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RouterInstallReport {
    pub dry_run: bool,
    pub changed: bool,
    pub skill_hash: String,
    /// The CLI path the installed text names.
    pub cli_path: String,
    pub targets: Vec<RouterTargetReport>,
    pub warnings: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct DoctorReport {
    pub vault: PathBuf,
    pub index: PathBuf,
    pub agents: Vec<AgentInfo>,
    pub checks: Vec<DoctorCheck>,
    pub healthy: bool,
    pub projects: Vec<ProjectStatus>,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rebuild: Option<IndexRebuildReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IndexRebuildReport {
    pub vault: PathBuf,
    pub index: PathBuf,
    pub dry_run: bool,
    pub rebuilt: bool,
    pub mode: String,
    pub skills: usize,
    pub groups: usize,
    pub projects: usize,
    pub activations: usize,
    pub other_vaults_preserved: bool,
    pub other_vaults_recovery_required: bool,
    pub observations: Vec<RegistryObservation>,
    pub warnings: Vec<String>,
    pub backup: Option<PathBuf>,
}
#[derive(Debug, Serialize)]
pub struct DoctorCheck {
    pub name: String,
    pub passed: bool,
    pub message: String,
    pub reason_code: String,
}

/// `kinana doctor --repair`: the plan that returns an interrupted change to
/// the last commit, and, once confirmed, what was done (owner decision
/// 2026-11-17, sixth round).
#[derive(Debug, Serialize)]
pub struct DoctorRepairReport {
    pub vault: PathBuf,
    pub dry_run: bool,
    /// Empty when there is nothing to repair.
    pub preview_token: String,
    /// Where the files about to be replaced are copied first; `None` when
    /// there is nothing to repair.
    pub backup_directory: Option<PathBuf>,
    pub restore: Vec<String>,
    pub unstage: Vec<String>,
    pub recreate_directories: Vec<String>,
    /// Moved into the backup directory, not deleted.
    pub staging_removed: Vec<String>,
    /// Files copied to the backup directory before being replaced.
    pub backed_up: Vec<String>,
    pub blocked: Vec<String>,
    pub untracked: Vec<String>,
    pub warnings: Vec<String>,
    pub after: Option<DoctorReport>,
}

#[derive(Debug, Serialize)]
pub struct UndoReport {
    pub vault: PathBuf,
    pub reverted_commit: String,
    pub resulting_commit: Option<String>,
    pub dry_run: bool,
    /// Untracked Finder `.DS_Store` files removed first because they were a
    /// skill's only difference from its recorded hash (would be removed, on a
    /// dry run). Absent when there were none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub finder_files_removed: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ImportOptions {
    pub dry_run: bool,
    pub all: bool,
    pub select: Vec<String>,
    pub allow_risky: bool,
    pub id_suffix: Option<String>,
    /// Scan imports only: for a name found with more than one content, the
    /// path whose content becomes the vault copy. Every other selected
    /// version of that name is recorded as a variant location of it.
    pub primary: Vec<String>,
    /// Desktop confirmation, keyed by the exact paths displayed in the preview.
    pub reviewed: Option<std::collections::BTreeMap<PathBuf, (String, String)>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportItem {
    #[serde(skip)]
    pub review_token: Option<String>,
    pub id: Option<String>,
    pub name: Option<String>,
    pub path: PathBuf,
    pub status: ScanStatus,
    pub content_hash: Option<String>,
    pub audit: Audit,
    pub selected: bool,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ImportReport {
    pub source: String,
    pub dry_run: bool,
    pub requires_preparation: bool,
    pub items: Vec<ImportItem>,
    pub imported: Vec<String>,
    pub duplicates: Vec<String>,
    pub variants: Vec<String>,
    pub committed: bool,
    pub index_updated: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ClassificationProposal {
    pub category: String,
    /// Required from the agent (checked on its JSON). The defaults only keep
    /// proposals stored before 8.3 readable and free of new lines on rewrite.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stack: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stage: Vec<String>,
    pub tags: Vec<String>,
    pub role: String,
    pub when_to_use: String,
    pub when_not: String,
    pub improved_description: String,
    pub overlaps_with: Vec<String>,
    pub confidence: f64,
}

#[derive(Debug, Clone)]
pub struct ClassifyOptions {
    pub ids: Vec<String>,
    pub all: bool,
    pub pending: bool,
    pub agent: String,
    pub confirmed: bool,
    pub dry_run: bool,
    pub full: bool,
}
impl Default for ClassifyOptions {
    fn default() -> Self {
        Self {
            ids: vec![],
            all: false,
            pending: false,
            agent: "auto".into(),
            confirmed: false,
            dry_run: false,
            full: false,
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct ClassifiedItem {
    pub id: String,
    pub category: String,
    pub classified_by: String,
    pub proposal_created: bool,
    pub attempts: usize,
    pub warnings: Vec<String>,
    /// Why this skill fell back to keywords, as a stable code the desktop
    /// translates. Absent for a provider proposal or a keywords-only run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    /// For `AGENT_EXECUTABLE_NOT_FOUND`: the directories searched, in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub searched_locations: Vec<String>,
    pub classified_from: Option<String>,
    pub estimated_skill_tokens: u64,
}
#[derive(Debug, Serialize)]
pub struct ClassifyReport {
    pub dry_run: bool,
    pub selected: usize,
    pub estimated_agent_calls: usize,
    pub estimated_tokens: TokenEstimate,
    pub agent_calls: usize,
    pub items: Vec<ClassifiedItem>,
    pub committed: bool,
    pub build: BuildReport,
    pub warnings: Vec<String>,
    /// Present when the run stopped early (cancellation, or a Claude usage
    /// limit or credit error). Completed skills were saved in one commit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped: Option<ClassifyStop>,
}

/// Why a classification run stopped before its last skill, and how far it got.
#[derive(Debug, Clone, Serialize)]
pub struct ClassifyStop {
    /// `CANCELLED`, `AGENT_USAGE_LIMIT` or `AGENT_CREDIT_EXHAUSTED`.
    pub reason_code: String,
    pub message: String,
    /// Skills whose result was saved.
    pub classified: usize,
    /// Skills not classified in this run, in order.
    pub remaining: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenCalibration {
    pub claude_version: String,
    pub model: String,
    pub source_log: String,
    pub input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub total_input_tokens: u64,
    pub prompt_utf8_bytes: u64,
    pub estimated_prompt_tokens: u64,
    pub cli_overhead_tokens: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TokenEstimate {
    pub fixed_tokens_per_call: u64,
    pub fixed_tokens_total: u64,
    pub skill_tokens_total: u64,
    pub total_input_tokens: u64,
    pub retry_limit_input_tokens: u64,
    pub calibration: Option<TokenCalibration>,
}
#[derive(Debug, Clone, Serialize)]
pub struct BuildItem {
    pub id: String,
    pub path: PathBuf,
    pub description_source: String,
    pub changed: bool,
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct BuildReport {
    pub dry_run: bool,
    pub items: Vec<BuildItem>,
    pub removed: Vec<String>,
    pub warnings: Vec<String>,
    /// `kinana build` only: untracked Finder `.DS_Store` files removed first
    /// because they were a skill's only difference from its recorded hash
    /// (would be removed, on a dry run). Absent when there were none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub finder_files_removed: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct EditResult {
    pub changed: Vec<String>,
    pub built: BuildReport,
    pub committed: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewDecision {
    Preview,
    Accept,
    Reject,
    Original,
}
#[derive(Debug, Serialize)]
pub struct ReviewReport {
    pub id: String,
    pub original: String,
    pub current: String,
    pub proposed: Option<String>,
    pub active: String,
    pub decision: String,
    pub dry_run: bool,
    pub committed: bool,
    pub build: BuildReport,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    pub name: String,
    pub title: String,
    pub agents: Vec<String>,
    pub skills: Vec<BundleSkill>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BundleSkill {
    pub id: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BundleSaveReport {
    pub dry_run: bool,
    pub operation: String,
    pub bundle: Bundle,
    pub path: PathBuf,
    pub changed: bool,
    pub committed: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupCandidate {
    pub skill_ids: Vec<String>,
    pub category: Option<String>,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct GroupsReport {
    pub candidates: Vec<GroupCandidate>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectLock {
    pub vault: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restoration: Option<ProjectRestoration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle: Option<String>,
    #[serde(default)]
    pub active: Vec<LockedSkill>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LockedSkill {
    pub id: String,
    pub agent: String,
    pub mode: String,
    pub name: String,
    /// Explicit for new Cursor entries. Absent Cursor entries retain their
    /// historical .cursor target; never reinterpret an old ownership proof.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_hash: Option<String>,
    #[serde(default = "automatic_invocation")]
    pub invocation: String,
    pub content_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ownership: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectRestoration {
    pub created_lock: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_lock_mode: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_empty_lock: Option<String>,
    #[serde(default)]
    pub directories: Vec<OwnedProjectDirectory>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OwnedProjectDirectory {
    pub path: String,
    pub identity: String,
}

#[derive(Debug, Clone)]
pub struct UnuseOptions {
    pub project: PathBuf,
    pub ids: Vec<String>,
    pub agent: Option<String>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnuseReport {
    pub project: PathBuf,
    pub dry_run: bool,
    pub changed: bool,
    pub removed: Vec<PathBuf>,
    pub removed_directories: Vec<PathBuf>,
    pub lock_removed: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActivationStatus {
    pub id: String,
    pub agent: String,
    pub target: PathBuf,
    pub state: String,
    pub message: String,
    pub reason_code: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectStatus {
    pub project: PathBuf,
    pub state: String,
    pub items: Vec<ActivationStatus>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusReport {
    pub projects: Vec<ProjectStatus>,
}

#[derive(Debug, Clone)]
pub struct AdoptOptions {
    pub project: PathBuf,
    pub home: PathBuf,
    pub agent: String,
    pub dry_run: bool,
    pub confirmed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdoptionCandidate {
    pub target: PathBuf,
    pub agent: String,
    pub id: Option<String>,
    pub content_hash: Option<String>,
    pub link_to: Option<PathBuf>,
    pub eligible: bool,
    pub reason: String,
    pub reason_code: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdoptReport {
    pub dry_run: bool,
    pub candidates: Vec<AdoptionCandidate>,
    pub activation: Option<ActivationReport>,
}

#[derive(Debug, Clone)]
pub struct ActivateOptions {
    pub project: PathBuf,
    pub home: PathBuf,
    pub selections: Vec<String>,
    pub agent: String,
    pub mode: String,
    pub include_secondary: Vec<String>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActivationItem {
    pub id: String,
    pub agent: String,
    pub mode: String,
    pub invocation: String,
    pub name: String,
    pub source: PathBuf,
    pub target: PathBuf,
    pub content_hash: String,
    pub changed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActivationReport {
    pub project: PathBuf,
    pub dry_run: bool,
    pub changed: bool,
    pub index_changed: bool,
    pub items: Vec<ActivationItem>,
    pub lock: ProjectLock,
    pub gitignore_suggestions: Vec<String>,
    pub warnings: Vec<String>,
}

fn automatic_invocation() -> String {
    "auto".into()
}

#[derive(Debug, Clone, Serialize)]
pub struct LockfileWriteReport {
    pub path: PathBuf,
    pub lock: ProjectLock,
    pub dry_run: bool,
    pub changed: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectRegistry {
    pub version: u32,
    #[serde(default)]
    pub projects: Vec<ProjectRegistration>,
}
impl Default for ProjectRegistry {
    fn default() -> Self {
        Self {
            version: 1,
            projects: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectRegistration {
    pub path: PathBuf,
    pub vault: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegistryWriteReport {
    pub path: PathBuf,
    pub registration: ProjectRegistration,
    pub dry_run: bool,
    pub changed: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegistryObservation {
    pub project: PathBuf,
    pub status: String,
    pub message: String,
    pub lock: Option<ProjectLock>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RegistryInspectReport {
    pub projects: Vec<RegistryObservation>,
    pub warnings: Vec<String>,
}

/// `stack` and `stage` are the values after the edit (or the preview of it).
#[derive(Debug, Serialize)]
pub struct DimensionsReport {
    pub id: String,
    pub stack: Vec<String>,
    pub stage: Vec<String>,
    pub dry_run: bool,
    pub changed: bool,
    pub committed: bool,
}

#[derive(Debug, Serialize)]
pub struct PinReport {
    pub id: String,
    pub pinned: bool,
    pub dry_run: bool,
    pub changed: bool,
    pub committed: bool,
}

/// `removed_paths` are relative to the vault root and use `/` separators.
#[derive(Debug, Serialize)]
pub struct RemoveReport {
    pub id: String,
    pub dry_run: bool,
    pub changed: bool,
    pub committed: bool,
    pub removed_paths: Vec<String>,
    pub bundles_changed: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct BundleDeleteReport {
    pub name: String,
    pub dry_run: bool,
    pub changed: bool,
    pub committed: bool,
}
