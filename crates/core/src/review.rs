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
