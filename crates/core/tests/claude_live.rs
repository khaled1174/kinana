use kinana_core::agent_runner::{AllowedValues, parse_proposal as parse_current};
use kinana_core::{ClassificationProposal, Result};
use serde_json::{Value, json};

const CAPTURES: [&str; 2] = [
    include_str!("fixtures/claude-live/model-1.jsonl"),
    include_str!("fixtures/claude-live/model-2.jsonl"),
];

fn categories() -> Vec<String> {
    vec!["web-frontend".into(), "other".into()]
}

/// The captures were recorded with the eight-field schema that was live
/// before 8.3. The files stay as recorded; every classification object in the
/// stream (the StructuredOutput call, `structured_output`, and the JSON text
/// in `result`) gets the two now-required dimension fields before parsing.
fn with_dimensions(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if object.contains_key("improved_description") {
                object.entry("stack").or_insert_with(|| json!(["react"]));
                object.entry("stage").or_insert_with(|| json!(["review"]));
                object
                    .entry("category_reason")
                    .or_insert_with(|| json!("Reviews browser components."));
            }
            if let Some(Value::String(text)) = object.get_mut("result")
                && let Ok(mut inner) = serde_json::from_str::<Value>(text)
                && inner.is_object()
            {
                with_dimensions(&mut inner);
                *text = inner.to_string();
            }
            object.values_mut().for_each(with_dimensions);
        }
        Value::Array(items) => items.iter_mut().for_each(with_dimensions),
        _ => {}
    }
}

fn parse_proposal(text: &str, categories: &[String]) -> Result<ClassificationProposal> {
    let upgraded: String = serde_json::Deserializer::from_str(text)
        .into_iter::<Value>()
        .map(|event| {
            let mut event = event.unwrap();
            with_dimensions(&mut event);
            format!("{event}\n")
        })
        .collect();
    let (stacks, stages) = (vec!["react".to_string()], vec!["review".to_string()]);
    parse_current(
        &upgraded,
        &AllowedValues {
            categories,
            stacks: &stacks,
            stages: &stages,
        },
    )
}

#[test]
fn captures_recorded_before_dimensions_are_rejected_until_both_fields_are_present() {
    let (stacks, stages) = (vec!["react".to_string()], vec!["review".to_string()]);
    let categories = categories();
    let allowed = AllowedValues {
        categories: &categories,
        stacks: &stacks,
        stages: &stages,
    };
    for capture in CAPTURES {
        let error = parse_current(capture, &allowed).unwrap_err();
        assert_eq!(error.code, "INVALID_AGENT_PROPOSAL");
        // The captures predate the dimensions and the 0.3 reason; the first
        // missing field is named.
        assert!(
            (error.message.contains("stack") || error.message.contains("category_reason"))
                && error.message.contains("required")
        );
        let proposal = parse_proposal(capture, &categories).unwrap();
        assert_eq!(proposal.stack, ["react"]);
        assert_eq!(proposal.stage, ["review"]);
    }
}

fn events() -> Vec<Value> {
    CAPTURES[0]
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn encoded(events: &[Value]) -> String {
    events.iter().map(|event| format!("{event}\n")).collect()
}

#[test]
fn captured_claude_envelopes_and_event_streams_accept_real_structured_output() {
    for (capture, confidence) in CAPTURES.into_iter().zip([0.85, 0.9]) {
        let result = capture
            .lines()
            .find(|line| serde_json::from_str::<Value>(line).unwrap()["type"] == "result")
            .unwrap();
        for text in [result, capture] {
            let proposal = parse_proposal(text, &categories()).unwrap();
            assert_eq!(proposal.category, "web-frontend");
            assert_eq!(proposal.confidence, confidence);
            assert_eq!(
                proposal.role,
                "Frontend accessibility reviewer for React/TypeScript components"
            );
            assert!(
                proposal
                    .improved_description
                    .contains("React and TypeScript")
            );
        }
    }
}

#[test]
fn unknown_future_envelope_fields_are_ignored_but_proposal_fields_remain_strict() {
    let mut captured = events();
    let original = parse_proposal(&encoded(&captured), &categories()).unwrap();
    let result = captured
        .iter_mut()
        .find(|event| event["type"] == "result")
        .unwrap();
    result["imaginary_future_claude_field"] =
        json!({"version": 99, "details": ["new transport metadata"]});
    let extended_result = result.to_string();
    assert_eq!(
        parse_proposal(&extended_result, &categories()).unwrap(),
        original
    );
    assert_eq!(
        parse_proposal(&encoded(&captured), &categories()).unwrap(),
        original
    );

    let result = captured
        .iter_mut()
        .find(|event| event["type"] == "result")
        .unwrap();
    result["structured_output"]["imaginary_future_claude_field"] = json!(true);
    let error = parse_proposal(&encoded(&captured), &categories()).unwrap_err();
    assert_eq!(error.code, "INVALID_AGENT_PROPOSAL");
    assert!(
        error
            .message
            .contains("structured_output.imaginary_future_claude_field")
    );
    assert!(error.message.contains("unknown classification field"));
}

#[test]
fn any_tool_other_than_structured_output_refuses_an_otherwise_valid_live_result() {
    for tool in [
        "Read",
        "Bash",
        "WebFetch",
        "mcp__files__read",
        "structuredoutput",
    ] {
        let mut captured = events();
        for event in &mut captured {
            if let Some(content) = event
                .pointer_mut("/message/content")
                .and_then(Value::as_array_mut)
            {
                for block in content {
                    if block["type"] == "tool_use" {
                        block["name"] = json!(tool);
                    }
                }
            }
        }
        let error = parse_proposal(&encoded(&captured), &categories()).unwrap_err();
        assert_eq!(error.code, "AGENT_TOOL_USE");
        assert!(error.message.contains(tool));
        assert!(error.message.contains("StructuredOutput"));
        assert!(error.message.contains(".name"));
    }
    let mut captured = events();
    captured.push(json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","id":"late-call","input":{"file_path":"canary.txt"}}]}}));
    assert_eq!(
        parse_proposal(&encoded(&captured), &categories())
            .unwrap_err()
            .code,
        "AGENT_TOOL_USE"
    );
    let mut captured = events();
    captured.push(json!({"type":"assistant","message":{"content":[{"type":"server_tool_use","name":"StructuredOutput","id":"server-call","input":{}}]}}));
    let error = parse_proposal(&encoded(&captured), &categories()).unwrap_err();
    assert_eq!(error.code, "AGENT_TOOL_USE");
    assert!(error.message.contains(".type") && error.message.contains("server-side"));
}

#[test]
fn proposal_errors_identify_the_field_and_violated_rule() {
    let original = events()
        .into_iter()
        .find(|event| event["type"] == "result")
        .unwrap();
    let mut wrong_type = original.clone();
    wrong_type["structured_output"]["confidence"] = json!("high");
    let error = parse_proposal(&wrong_type.to_string(), &categories()).unwrap_err();
    assert!(error.message.contains("structured_output.confidence"));
    assert!(error.message.contains("number"));
    let mut unknown = original.clone();
    unknown["structured_output"]["extra"] = json!(true);
    let error = parse_proposal(&unknown.to_string(), &categories()).unwrap_err();
    assert!(error.message.contains("structured_output.extra"));
    assert!(error.message.contains("unknown"));
    let mut missing = original;
    missing["structured_output"]
        .as_object_mut()
        .unwrap()
        .remove("role");
    let error = parse_proposal(&missing.to_string(), &categories()).unwrap_err();
    assert!(error.message.contains("structured_output.role"));
    assert!(error.message.contains("required"));
}

#[test]
fn tool_results_must_belong_to_a_recorded_structured_output_call() {
    let mut captured = events();
    for event in &mut captured {
        if event["type"] == "user" {
            event["message"]["content"][0]["tool_use_id"] = json!("unknown-call");
        }
    }
    assert_eq!(
        parse_proposal(&encoded(&captured), &categories())
            .unwrap_err()
            .code,
        "AGENT_TOOL_USE"
    );
}

#[test]
fn a_recorded_stream_whose_init_event_offers_another_tool_is_refused() {
    let mut captured = events();
    let original = parse_proposal(&encoded(&captured), &categories()).unwrap();
    assert_eq!(original.category, "web-frontend");
    let init = captured
        .iter_mut()
        .find(|event| event["subtype"] == "init")
        .unwrap();
    init["tools"] = json!(["StructuredOutput", "WebFetch"]);
    let error = parse_proposal(&encoded(&captured), &categories()).unwrap_err();
    assert_eq!(error.code, "AGENT_TOOLS_NOT_ISOLATED");
    assert!(error.message.contains("WebFetch"), "{}", error.message);
}
