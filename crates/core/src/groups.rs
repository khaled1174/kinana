//! Read-only overlap candidates. Confirmation and priorities belong to bundles.
use crate::{GroupCandidate, GroupsReport, Result, SkillMetadata, edit, vault::Vault};
use std::collections::{BTreeMap, BTreeSet};

type Pair = (String, String);

/// Discover candidate pairs from recorded overlaps and conservative role similarity.
/// Reference strings are matched against known IDs and are never used as paths.
pub fn discover(vault: &Vault) -> Result<GroupsReport> {
    let mut metadata = BTreeMap::new();
    for record in vault.records()? {
        metadata.insert(record.id.clone(), edit::read_metadata(vault, &record.id)?);
    }
    let mut candidates: BTreeMap<Pair, BTreeSet<String>> = BTreeMap::new();
    let mut warnings = BTreeSet::new();
    for (id, skill) in &metadata {
        add_references(
            id,
            &skill.classification.overlaps_with,
            "classification.overlaps_with",
            &metadata,
            &mut candidates,
            &mut warnings,
        );
        if let Some(proposal) = &skill.proposal {
            add_references(
                id,
                &proposal.overlaps_with,
                "proposal.overlaps_with (legacy pending proposal)",
                &metadata,
                &mut candidates,
                &mut warnings,
            );
        }
    }

    let roles: Vec<_> = metadata
        .iter()
        .map(|(id, skill)| (id, skill, role_tokens(&skill.classification.role)))
        .collect();
    for (position, (left_id, left, left_tokens)) in roles.iter().enumerate() {
        if left_tokens.len() < 2 || left.classification.category.trim().is_empty() {
            continue;
        }
        for (right_id, right, right_tokens) in roles.iter().skip(position + 1) {
            if left.classification.category != right.classification.category
                || right_tokens.len() < 2
            {
                continue;
            }
            let shared = left_tokens.intersection(right_tokens).count();
            let union = left_tokens.len() + right_tokens.len() - shared;
            if shared >= 2 && shared * 5 >= union * 3 {
                candidates
                    .entry(pair(left_id, right_id))
                    .or_default()
                    .insert(format!(
                        "role_similarity: {shared} shared substantive tokens; Jaccard {shared}/{union} >= 0.6"
                    ));
            }
        }
    }

    let candidates = candidates
        .into_iter()
        .map(|((left, right), reasons)| {
            let left_category = &metadata[&left].classification.category;
            let right_category = &metadata[&right].classification.category;
            GroupCandidate {
                skill_ids: vec![left, right],
                category: (left_category == right_category && !left_category.trim().is_empty())
                    .then(|| left_category.clone()),
                reasons: reasons.into_iter().collect(),
            }
        })
        .collect();
    Ok(GroupsReport {
        candidates,
        warnings: warnings.into_iter().collect(),
    })
}

fn add_references(
    id: &str,
    references: &[String],
    field: &str,
    known: &BTreeMap<String, SkillMetadata>,
    candidates: &mut BTreeMap<Pair, BTreeSet<String>>,
    warnings: &mut BTreeSet<String>,
) {
    for reference in references {
        if crate::vault::validate_id(reference).is_err() {
            warnings.insert(format!(
                "Ignored malformed overlap reference {reference:?} in {field} for {id}."
            ));
        } else if reference == id {
            continue;
        } else if !known.contains_key(reference) {
            warnings.insert(format!(
                "Ignored missing skill overlap reference {reference:?} in {field} for {id}."
            ));
        } else {
            candidates
                .entry(pair(id, reference))
                .or_default()
                .insert(format!("agent_overlap: {id} -> {reference} ({field})"));
        }
    }
}

fn pair(left: &str, right: &str) -> Pair {
    if left < right {
        (left.into(), right.into())
    } else {
        (right.into(), left.into())
    }
}

fn role_tokens(role: &str) -> BTreeSet<String> {
    role.to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| {
            token.chars().count() >= 2
                && token.chars().any(char::is_alphabetic)
                && !ROLE_GLUE_WORDS.contains(token)
        })
        .map(str::to_string)
        .collect()
}

// Shared grammar and generic self-descriptions are insufficient evidence of a
// shared purpose. Domain words remain, including Unicode words and acronyms.
const ROLE_GLUE_WORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "help", "helps", "in", "is",
    "it", "of", "on", "or", "skill", "skills", "that", "the", "this", "to", "use", "with",
];
