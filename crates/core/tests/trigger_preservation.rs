use serde_json::Value;
use std::collections::BTreeSet;

fn normalized(text: &str) -> String {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
fn trigger_phrases(original: &str) -> BTreeSet<String> {
    original
        .split_once("Triggers on:")
        .unwrap()
        .1
        .split(',')
        .map(normalized)
        .filter(|phrase| !phrase.is_empty())
        .collect()
}
fn retained(original: &str, proposed: &str) -> usize {
    let text = format!(" {} ", normalized(proposed));
    trigger_phrases(original)
        .iter()
        .filter(|phrase| text.contains(&format!(" {phrase} ")))
        .count()
}
#[test]
fn live_foundations_fixture_preserves_at_least_half_of_original_trigger_phrases() {
    let old: Value = serde_json::from_str(include_str!(
        "../../../docs/evidence/phase-6/batch-2b/live-proposal.json"
    ))
    .unwrap();
    let new: Value = serde_json::from_str(include_str!(
        "../../../docs/evidence/phase-6/pre-3-core/live-trigger-proposal.json"
    ))
    .unwrap();
    let original = old["original"].as_str().unwrap();
    assert_eq!(new["original"], old["original"]);
    assert_eq!(trigger_phrases(original).len(), 54);
    assert_eq!(
        retained(
            original,
            old["proposal"]["improved_description"].as_str().unwrap()
        ),
        15
    );
    assert!(
        retained(
            original,
            old["proposal"]["improved_description"].as_str().unwrap()
        ) < 27
    );
    let proposed = new["proposal"]["improved_description"].as_str().unwrap();
    assert!(
        retained(original, proposed) >= 27,
        "Retained only {} of 54 trigger phrases",
        retained(original, proposed)
    );
    assert!(proposed.chars().count() <= 1024);
    assert_eq!(new["classified_by"], "claude");
    assert_eq!(new["active"], "original");
    assert_eq!(new["source_and_dist_unchanged"], true);
}
#[test]
fn phrase_matching_uses_whole_words_and_normalizes_case_and_hyphens() {
    let original = "Triggers on: UI, line-height, dark mode, UI.";
    assert_eq!(trigger_phrases(original).len(), 3);
    assert_eq!(
        retained(original, "Guide layout with Line Height; switch DARK MODE."),
        2
    );
    assert_eq!(
        retained(original, "Build GUID and line-heightened widgets."),
        0
    );
}
