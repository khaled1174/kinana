use crate::{
    BuildReport, Error, Result, ReviewDecision, ReviewReport, edit, frontmatter, vault::Vault,
};
use std::collections::BTreeMap;

pub fn review(
    vault: &Vault,
    id: &str,
    decision: ReviewDecision,
    dry_run: bool,
) -> Result<ReviewReport> {
    let (mut metadata, bytes) = edit::read_snapshot(vault, id)?;
    let manifest = frontmatter::read_skill(&vault.root.join("skills").join(id).join("SKILL.md"))?;
    let current =
        match metadata.description.active.as_str() {
            "original" => manifest.description.clone(),
            "improved"
                if metadata
                    .description
                    .approved_at
                    .as_ref()
                    .is_some_and(|v| !v.trim().is_empty()) =>
            {
                metadata.description.improved.clone().ok_or_else(|| {
                    Error::new("INVALID_METADATA", "Approved description is missing")
                })?
            }
            _ => {
                return Err(Error::new(
                    "INVALID_METADATA",
                    "Invalid active description or approval",
                ));
            }
        };
    let proposed = metadata
        .proposal
        .as_ref()
        .map(|p| p.improved_description.clone());
    let mut report = ReviewReport {
        id: id.into(),
        original: manifest.description.clone(),
        current,
        proposed,
        active: metadata.description.active.clone(),
        decision: match decision {
            ReviewDecision::Preview => "preview",
            ReviewDecision::Accept => "accept",
            ReviewDecision::Reject => "reject",
            ReviewDecision::Original => "original",
        }
        .into(),
        dry_run,
        committed: false,
        build: BuildReport {
            dry_run,
            ..BuildReport::default()
        },
    };
    if decision == ReviewDecision::Preview {
        return Ok(report);
    }
    // Old phase-three proposals may predate the persistent overlap field.
    // Description decisions do not discard their independent grouping evidence.
    if let Some(proposal) = &metadata.proposal {
        metadata
            .classification
            .overlaps_with
            .extend(proposal.overlaps_with.iter().cloned());
        metadata.classification.overlaps_with.sort();
        metadata.classification.overlaps_with.dedup();
    }
    match decision {
        ReviewDecision::Accept => {
            let proposal = metadata.proposal.clone().ok_or_else(|| {
                Error::decision(
                    "NO_DESCRIPTION_PROPOSAL",
                    "No pending description to accept",
                )
            })?;
            crate::agent_runner::validate_proposal(&proposal, &(&vault.config()?).into())?;
            metadata.description.improved = Some(proposal.improved_description);
            metadata.description.active = "improved".into();
            metadata.description.approved_at = Some(crate::import::timestamp()?);
            metadata.proposal = None;
            metadata.proposal_source_changed = None;
        }
        ReviewDecision::Reject => {
            if metadata.proposal.is_none() {
                return Err(Error::decision(
                    "NO_DESCRIPTION_PROPOSAL",
                    "No pending description to reject",
                ));
            }
            metadata.proposal = None;
            metadata.proposal_source_changed = None;
        }
        ReviewDecision::Original => {
            metadata.description.active = "original".into();
        }
        ReviewDecision::Preview => {}
    }
    metadata.description.original = manifest.description;
    let active = metadata.description.active.clone();
    let result = edit::apply_checked(
        vault,
        &BTreeMap::from([(id.to_string(), metadata)]),
        &BTreeMap::from([(id.to_string(), bytes)]),
        &format!("Review description for {id}: {}", report.decision),
        dry_run,
    )?;
    report.active = active;
    report.committed = result.committed;
    report.build = result.built;
    Ok(report)
}

/// The most proposals one batch accept takes; a larger selection is split.
pub const BATCH_REVIEW_LIMIT: usize = 200;

fn batch_token(entries: &[(String, Vec<u8>, String)]) -> String {
    let mut sorted = entries.iter().collect::<Vec<_>>();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"kinana-review-batch-v1\0");
    for (id, bytes, original) in sorted {
        for part in [id.as_bytes(), bytes.as_slice(), original.as_bytes()] {
            hasher.update(&(part.len() as u64).to_le_bytes());
            hasher.update(part);
        }
    }
    format!("r1.{}", hasher.finalize().to_hex())
}

/// Accepts the pending descriptions of `ids` together. Without `confirm` (or
/// with `dry_run`) this is a preview that writes nothing and returns the token
/// binding the exact proposals shown. With the matching token every proposal
/// is checked again and all of them are accepted in one commit; one failing
/// skill stops the whole batch before any write.
pub fn accept_many(
    vault: &Vault,
    ids: &[String],
    confirm: Option<&str>,
    dry_run: bool,
) -> Result<crate::BatchReviewReport> {
    if ids.is_empty() {
        return Err(Error::new(
            "INVALID_ARGUMENTS",
            "Choose at least one skill whose description to accept",
        ));
    }
    if ids.len() > BATCH_REVIEW_LIMIT {
        return Err(Error::new(
            "INVALID_ARGUMENTS",
            format!("Accept at most {BATCH_REVIEW_LIMIT} descriptions at a time"),
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for id in ids {
        if !seen.insert(id.as_str()) {
            return Err(Error::new(
                "INVALID_ARGUMENTS",
                format!("Skill '{id}' is listed more than once"),
            ));
        }
    }
    let config = vault.config()?;
    let settings = (&config).into();
    let mut items = Vec::with_capacity(ids.len());
    let mut entries = Vec::with_capacity(ids.len());
    let mut updates = BTreeMap::new();
    let mut expected = BTreeMap::new();
    for id in ids {
        let (mut metadata, bytes) = edit::read_snapshot(vault, id)?;
        let manifest =
            frontmatter::read_skill(&vault.root.join("skills").join(id).join("SKILL.md"))?;
        let Some(proposal) = metadata.proposal.clone() else {
            return Err(Error::decision(
                "NO_DESCRIPTION_PROPOSAL",
                format!("Skill '{id}' has no pending description to accept"),
            ));
        };
        // A proposal written for the previous source may describe content
        // that is gone; it is reviewed on its own, never inside a batch.
        if metadata.proposal_source_changed == Some(true) {
            return Err(Error::decision(
                "PROPOSAL_SOURCE_CHANGED",
                format!(
                    "The pending description of '{id}' was proposed for the previous source; review it on its own"
                ),
            ));
        }
        crate::agent_runner::validate_proposal(&proposal, &settings).map_err(|error| Error {
            message: format!("{id}: {}", error.message),
            ..error
        })?;
        let current = match metadata.description.active.as_str() {
            "original" => manifest.description.clone(),
            "improved"
                if metadata
                    .description
                    .approved_at
                    .as_ref()
                    .is_some_and(|v| !v.trim().is_empty()) =>
            {
                metadata.description.improved.clone().ok_or_else(|| {
                    Error::new("INVALID_METADATA", "Approved description is missing")
                })?
            }
            _ => {
                return Err(Error::new(
                    "INVALID_METADATA",
                    "Invalid active description or approval",
                ));
            }
        };
        items.push(crate::BatchReviewItem {
            id: id.clone(),
            original: manifest.description.clone(),
            current,
            proposed: proposal.improved_description.clone(),
            active: metadata.description.active.clone(),
        });
        entries.push((id.clone(), bytes.clone(), manifest.description.clone()));
        metadata
            .classification
            .overlaps_with
            .extend(proposal.overlaps_with.iter().cloned());
        metadata.classification.overlaps_with.sort();
        metadata.classification.overlaps_with.dedup();
        metadata.description.improved = Some(proposal.improved_description);
        metadata.description.active = "improved".into();
        metadata.description.approved_at = Some(crate::import::timestamp()?);
        metadata.description.original = manifest.description;
        metadata.proposal = None;
        metadata.proposal_source_changed = None;
        updates.insert(id.clone(), metadata);
        expected.insert(id.clone(), bytes);
    }
    let token = batch_token(&entries);
    let mut report = crate::BatchReviewReport {
        decision: "accept".into(),
        items,
        preview_token: token.clone(),
        requires_confirmation: true,
        dry_run,
        committed: false,
        build: BuildReport {
            dry_run: true,
            ..BuildReport::default()
        },
    };
    let Some(confirm) = confirm else {
        return Ok(report);
    };
    if confirm != token {
        return Err(Error::decision(
            "REVIEW_PREVIEW_CHANGED",
            "The pending descriptions changed after the preview; review the new preview and confirm it",
        ));
    }
    report.requires_confirmation = false;
    if dry_run {
        return Ok(report);
    }
    let mut named = ids.iter().take(10).cloned().collect::<Vec<_>>().join(", ");
    if ids.len() > 10 {
        named.push_str(&format!(" and {} more", ids.len() - 10));
    }
    let message = format!(
        "Accept descriptions for {} skill{}: {named}",
        ids.len(),
        if ids.len() == 1 { "" } else { "s" },
    );
    let result = edit::apply_checked(vault, &updates, &expected, &message, false)?;
    for item in &mut report.items {
        item.active = "improved".into();
    }
    report.committed = result.committed;
    report.build = result.built;
    Ok(report)
}

/// One pending description for the desktop review list.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PendingDescription {
    pub id: String,
    pub name: String,
    pub current: String,
    pub proposed: String,
    /// The proposal was made for the previous source; it is reviewed alone
    /// and a batch that includes it is refused (`PROPOSAL_SOURCE_CHANGED`).
    pub source_changed: bool,
}

/// Every skill with a pending description, read only. Writes nothing.
pub fn pending(vault: &Vault) -> Result<Vec<PendingDescription>> {
    let report = crate::catalog::list(
        vault,
        &crate::catalog::CatalogFilter {
            state: Some("proposal".into()),
            ..Default::default()
        },
    )?;
    let mut items = Vec::with_capacity(report.items.len());
    for item in report.items {
        let id = item.skill.id.clone();
        let (metadata, _) = edit::read_snapshot(vault, &id)?;
        let Some(proposal) = metadata.proposal else {
            continue;
        };
        items.push(PendingDescription {
            id,
            name: item.skill.name.clone(),
            current: item.skill.description.clone(),
            proposed: proposal.improved_description,
            source_changed: metadata.proposal_source_changed == Some(true),
        });
    }
    Ok(items)
}

/// The descriptions a classification approved automatically (0.3, r5) and
/// the commit that holds only them, which `undo` reverts.
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct AutoApproval {
    pub ids: Vec<String>,
    pub commit: String,
}

/// The audit levels whose proposed descriptions are approved automatically
/// (owner, 2026-11-24, r6): every level but `risky`.
const AUTO_APPROVED_LEVELS: [&str; 2] = ["clean", "review"];

/// A description text with every run of whitespace made one space and the
/// ends trimmed (owner, 2026-11-24, r6): two descriptions that differ only in
/// spacing or line breaks are the same description.
pub(crate) fn same_description(a: &str, b: &str) -> bool {
    a.split_whitespace().eq(b.split_whitespace())
}

/// The description the user chose for a skill, when they chose one: the
/// approved improved description, or the original they returned to after
/// an improvement. `None` for a skill whose description was never reviewed.
/// `original` is the description in SKILL.md.
pub(crate) fn chosen_description(
    description: &crate::Description,
    original: &str,
) -> Option<String> {
    let approved = description
        .approved_at
        .as_ref()
        .is_some_and(|at| !at.trim().is_empty());
    match description.active.as_str() {
        "improved" if approved => description.improved.clone(),
        "original" if approved || description.improved.is_some() => Some(original.to_owned()),
        _ => None,
    }
}

/// Why a description proposed in this run was not approved automatically;
/// it stays pending.
fn auto_refusal(
    vault: &Vault,
    id: &str,
    metadata: &crate::SkillMetadata,
    settings: &crate::agent_runner::AllowedValues<'_>,
) -> Result<Option<&'static str>> {
    let Some(proposal) = &metadata.proposal else {
        return Ok(Some("no proposal"));
    };
    if metadata.proposal_source_changed == Some(true) {
        return Ok(Some("proposed for the previous source"));
    }
    // An approved description is replaced by a different proposal under the
    // same rule as a first one (owner, 2026-11-24, r6). A return to the
    // original after an improvement is the user's explicit choice, which the
    // owner's r6 rule does not name: that proposal waits, as in r5.
    if metadata.description.active == "original" && metadata.description.improved.is_some() {
        return Ok(Some("the user chose the original description"));
    }
    // Not risky in the stored audit, nor in a fresh one with the current rules.
    if !AUTO_APPROVED_LEVELS.contains(&metadata.audit.level.as_str()) {
        return Ok(Some("risky or not audited"));
    }
    let skill = vault.root.join("skills").join(id);
    if !AUTO_APPROVED_LEVELS.contains(&crate::audit::audit_skill(&skill)?.level.as_str()) {
        return Ok(Some("risky in a fresh audit"));
    }
    // The checks of a manual approval, done here so one unusable description
    // leaves only its own skill pending.
    if crate::agent_runner::validate_proposal(proposal, settings).is_err() {
        return Ok(Some("the proposal does not pass validation"));
    }
    let description = &proposal.improved_description;
    if description.trim().is_empty() || description.chars().any(char::is_control) {
        return Ok(Some("empty or with control characters"));
    }
    let text = std::fs::read_to_string(skill.join("SKILL.md"))?;
    if !crate::build::description_fits(&text, description) {
        return Ok(Some("does not fit the SKILL.md frontmatter"));
    }
    Ok(None)
}

/// After a classification commit (0.3; r5, scope widened in r6 by the owner
/// on 2026-11-24): approves the proposed descriptions of `ids` whose skill is
/// not risky in the audit, in one commit of their own. Risky skills, a
/// proposal for the previous source, a return to the original, and a
/// description that fails the checks stay pending. Returns `None` when
/// nothing was approved.
pub fn approve_not_risky(vault: &Vault, ids: &[String]) -> Result<Option<AutoApproval>> {
    let config = vault.config()?;
    let settings = (&config).into();
    let mut updates = BTreeMap::new();
    let mut expected = BTreeMap::new();
    for id in ids {
        let (mut metadata, bytes) = edit::read_snapshot(vault, id)?;
        if auto_refusal(vault, id, &metadata, &settings)?.is_some() {
            continue;
        }
        let manifest =
            frontmatter::read_skill(&vault.root.join("skills").join(id).join("SKILL.md"))?;
        let proposal = metadata.proposal.take().expect("checked above");
        metadata
            .classification
            .overlaps_with
            .extend(proposal.overlaps_with.iter().cloned());
        metadata.classification.overlaps_with.sort();
        metadata.classification.overlaps_with.dedup();
        metadata.description.improved = Some(proposal.improved_description);
        metadata.description.active = "improved".into();
        metadata.description.approved_at = Some(crate::import::timestamp()?);
        metadata.description.original = manifest.description;
        metadata.proposal_source_changed = None;
        updates.insert(id.clone(), metadata);
        expected.insert(id.clone(), bytes);
    }
    if updates.is_empty() {
        return Ok(None);
    }
    let approved = updates.keys().cloned().collect::<Vec<_>>();
    let mut named = approved
        .iter()
        .take(10)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if approved.len() > 10 {
        named.push_str(&format!(" and {} more", approved.len() - 10));
    }
    let message = format!(
        "Approve descriptions automatically for {} skill{} that {} not risky: {named}",
        approved.len(),
        if approved.len() == 1 { "" } else { "s" },
        if approved.len() == 1 { "is" } else { "are" },
    );
    let result = edit::apply_checked(vault, &updates, &expected, &message, false)?;
    if !result.committed {
        return Ok(None);
    }
    Ok(Some(AutoApproval {
        ids: approved,
        commit: crate::gitlog::head(&vault.root)?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptions_that_differ_only_in_whitespace_are_the_same() {
        assert!(same_description(
            "Review  React\ncomponents ",
            " Review React components"
        ));
        assert!(same_description("a\tb", "a b"));
        assert!(!same_description(
            "Review React components",
            "Review react components"
        ));
        assert!(!same_description(
            "Review React components",
            "Review React component"
        ));
        assert!(!same_description("ab", "a b"));
    }

    #[test]
    fn the_chosen_description_is_the_approved_one_or_a_return_to_the_original() {
        let mut description = crate::Description::default();
        assert_eq!(chosen_description(&description, "Original"), None);
        description.improved = Some("Improved".into());
        description.active = "improved".into();
        // Improved but never approved: not a choice.
        assert_eq!(chosen_description(&description, "Original"), None);
        description.approved_at = Some("2026-11-24T00:00:00Z".into());
        assert_eq!(
            chosen_description(&description, "Original").as_deref(),
            Some("Improved")
        );
        description.active = "original".into();
        assert_eq!(
            chosen_description(&description, "Original").as_deref(),
            Some("Original")
        );
    }
}
