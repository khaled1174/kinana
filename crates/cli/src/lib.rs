use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "kinana", version, about = "Your personal Agent Skills vault")]
pub struct Args {
    /// Vault directory (saved choice, existing vault, or ~/KinanaVault).
    #[arg(long, global = true)]
    pub vault: Option<PathBuf>,
    /// Emit machine-readable JSON.
    #[arg(long, global = true)]
    pub json: bool,
    /// Preview without changing any files.
    #[arg(long, global = true)]
    pub dry_run: bool,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Preview and explicitly migrate legacy application state and project locks.
    Migrate {
        /// Include this project in addition to registered projects; never scan home.
        #[arg(long)]
        project: Vec<PathBuf>,
        /// Confirm the exact preview fingerprint.
        #[arg(long)]
        confirm: Option<String>,
        /// Preview or confirm rollback of the most recent migration.
        #[arg(long)]
        rollback: bool,
    },
    /// Initialize the vault and its local index.
    Init {
        #[arg(long)]
        path: Option<PathBuf>,
        /// Install the router skill in the fixed global Claude and Codex skill folders.
        #[arg(long)]
        install_router: bool,
    },
    /// Discover skills in explicitly chosen folders without importing them.
    Scan {
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Import new skills and record discovered locations after inspection.
        #[arg(long = "import")]
        import_skills: bool,
        #[arg(long, requires = "import_skills")]
        allow_risky: bool,
    },
    /// Import skills from a local folder, GitHub URL or ZIP archive.
    Add {
        input: String,
        #[arg(long, conflicts_with = "select")]
        all: bool,
        #[arg(long)]
        select: Vec<String>,
        #[arg(long)]
        allow_risky: bool,
        /// Source suffix for an explicitly selected conflicting skill name.
        #[arg(long)]
        suffix: Option<String>,
    },
    /// List the skills in the vault.
    List {
        /// Compact catalog for an agent: role, when to use, when not, a short
        /// description and the dimensions of each skill, plus every bundle.
        #[arg(long)]
        brief: bool,
        /// Skills on one page of `--brief` (default 20, at most 50).
        #[arg(long, requires = "brief", value_parser = clap::value_parser!(u16).range(1..=50))]
        limit: Option<u16>,
        /// Page of `--brief` to show, counted from 1.
        #[arg(long, requires = "brief", value_parser = clap::value_parser!(u32).range(1..))]
        page: Option<u32>,
        #[arg(long)]
        query: Option<String>,
        #[arg(long, value_parser = ["pending", "proposal", "approved", "original"])]
        state: Option<String>,
        /// Show only pinned skills.
        #[arg(long)]
        pinned: bool,
        #[arg(long)]
        category: Option<String>,
        /// Show only skills holding this value of the `stacks` list in box.toml.
        #[arg(long)]
        stack: Option<String>,
        /// Show only skills holding this value of the `stages` list in box.toml.
        #[arg(long)]
        stage: Option<String>,
        #[arg(long, value_parser = ["claude", "codex", "cursor"])]
        agent: Option<String>,
        #[arg(long)]
        audit: Option<String>,
    },
    /// Set a skill's favorite flag without touching source or generated files.
    Pin {
        id: String,
        #[arg(long)]
        unset: bool,
    },
    /// Delete one skill from the vault; refused while a registered project uses it.
    Remove { id: String },
    /// Set a configured category without approving the description.
    Category { id: String, category: String },
    /// Set a skill's technology (stack) and work-stage values from the lists in box.toml.
    Dimensions {
        id: String,
        /// Replace the skill's stack values; repeat the option for several values.
        #[arg(long, conflicts_with = "clear_stack")]
        stack: Vec<String>,
        /// Replace the skill's stage values; repeat the option for several values.
        #[arg(long, conflicts_with = "clear_stage")]
        stage: Vec<String>,
        /// Remove every stack value from the skill.
        #[arg(long)]
        clear_stack: bool,
        /// Remove every stage value from the skill.
        #[arg(long)]
        clear_stage: bool,
    },
    /// Show a skill's metadata and raw SKILL.md.
    Show { id: String },
    /// Search names, active descriptions, roles, tags and when-to-use text using literal full-text terms.
    Search { query: String },
    /// Suggest bundles and skills from fixed project manifests without agent calls.
    Suggest {
        #[arg(long)]
        project: PathBuf,
    },
    /// Classify skills and save a description proposal for manual review.
    Classify {
        #[arg(conflicts_with_all = ["all", "pending"])]
        ids: Vec<String>,
        #[arg(long, conflicts_with = "pending")]
        all: bool,
        #[arg(long)]
        pending: bool,
        #[arg(long, default_value = "auto", value_parser = ["auto", "keywords", "claude", "codex", "pi"])]
        agent: String,
        /// Claude model alias or full model name for this run.
        #[arg(long)]
        model: Option<String>,
        /// Claude effort override for this run.
        #[arg(long)]
        effort: Option<String>,
        /// Emit only Claude model and stop metadata for diagnosis.
        #[arg(long, requires = "json")]
        claude_events: bool,
        /// Confirm the displayed batch count before provider calls.
        #[arg(long)]
        yes: bool,
        /// Send complete SKILL.md for one explicit skill, including long files.
        #[arg(long, conflicts_with_all = ["all", "pending"])]
        full: bool,
        /// Confirm Pi as the `auto` fallback: the token from the preview or
        /// from a run that stopped with PI_CONFIRMATION_REQUIRED.
        #[arg(long)]
        confirm: Option<String>,
    },
    /// Show the description difference or explicitly accept/reject a proposal.
    /// Several IDs with --accept preview the batch; --confirm accepts it in one commit.
    Review {
        #[arg(required = true, num_args = 1..)]
        ids: Vec<String>,
        #[arg(long, conflicts_with_all = ["reject", "original"])]
        accept: bool,
        /// Accept exactly the previewed batch (token from the preview).
        #[arg(long, requires = "accept")]
        confirm: Option<String>,
        #[arg(long, conflicts_with = "original")]
        reject: bool,
        #[arg(long)]
        original: bool,
    },
    /// Generate ready-to-use copies from immutable originals and approved metadata.
    Build { id: Option<String> },
    /// Inspect GitHub source revisions without modifying permanent state.
    Outdated,
    /// Preview a pinned source update, then confirm that exact reviewed plan.
    Update {
        id: String,
        /// branch:name (or name), tag:name, commit:SHA, or default.
        #[arg(long = "ref")]
        reference: Option<String>,
        #[arg(long)]
        confirm: Option<String>,
        #[arg(long)]
        allow_risky: bool,
    },
    /// Show same-function candidates for explicit confirmation in a bundle.
    Groups,
    /// Show current, stale, modified and broken project activations without writes.
    Status {
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Remove explicitly selected activations only when their ownership still matches.
    Unuse {
        #[arg(required = true)]
        ids: Vec<String>,
        #[arg(long)]
        project: PathBuf,
        #[arg(long, value_parser = ["claude", "codex", "cursor", "all"])]
        agent: Option<String>,
    },
    /// Preview or confirm replacement of identical project skill copies with links.
    Adopt {
        #[arg(long)]
        project: PathBuf,
        #[arg(long, default_value = "all", value_parser = ["claude", "codex", "cursor", "all"])]
        agent: String,
        #[arg(long)]
        yes: bool,
    },
    /// Activate a bundle or explicitly selected skills in a project.
    Use {
        #[arg(required = true)]
        selections: Vec<String>,
        #[arg(long)]
        project: PathBuf,
        #[arg(long, value_parser = ["claude", "codex", "cursor", "all"])]
        agent: String,
        #[arg(long, default_value = "link", value_parser = ["link", "copy"])]
        mode: String,
        /// Explicitly activate a secondary bundle member in manual mode.
        #[arg(long)]
        include_secondary: Vec<String>,
    },
    /// Manage bundle membership and primary/secondary priorities.
    Bundle {
        #[command(subcommand)]
        command: BundleCommands,
    },
    /// Preview re-auditing every skill with the current audit rules: how many
    /// levels go down or up, raised skills first. With --confirm, re-audit all
    /// of them in one commit that `kinana undo` reverts. Nothing changes otherwise.
    RefreshAudit {
        /// Confirm the exact preview token.
        #[arg(long)]
        confirm: Option<String>,
    },
    /// Revert the last vault change using Git, preserving history.
    Undo,
    /// Read-only diagnostics for the vault, index and installed agents.
    Doctor {
        /// Rebuild the selected vault's cache from metadata, bundles and registered locks.
        #[arg(long, conflicts_with = "repair")]
        rebuild_index: bool,
        /// Preview returning an interrupted change to the last commit; with
        /// --confirm, back up what it replaces to ~/.kinana/repair-backups and do it.
        #[arg(long)]
        repair: bool,
        /// Confirm the exact repair preview token.
        #[arg(long, requires = "repair")]
        confirm: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum BundleCommands {
    /// Create a bundle from a complete TOML file.
    Create {
        #[arg(long)]
        file: PathBuf,
    },
    /// Replace a bundle's settings from a complete TOML file.
    Edit {
        name: String,
        #[arg(long)]
        file: PathBuf,
    },
    /// List file-backed bundles.
    List,
    /// Show one bundle and its priorities.
    Show { name: String },
    /// Delete one bundle file; its skills and project activations are untouched.
    Delete { name: String },
}
