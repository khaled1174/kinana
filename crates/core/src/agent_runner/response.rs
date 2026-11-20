use super::{AllowedValues, invalid_proposal, validate_proposal};
use crate::{ClassificationProposal, Error, Result};
use serde::Deserialize;
use serde::de::{MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fmt;

/// Transport metadata is ignored as data. Proposal fields remain strict, and
/// every event is inspected for prohibited tools, including events after result.
pub fn parse_proposal(text: &str, allowed: &AllowedValues<'_>) -> Result<ClassificationProposal> {
    let events = serde_json::Deserializer::from_str(text)
        .into_iter::<UniqueJson>()
        .enumerate()
        .map(|(index, event)| {
            event.map(|event| event.0).map_err(|error| {
                invalid_proposal(&format!(
                    "events[{index}]: JSON syntax/unique-field rule: {error}"
                ))
            })
        })
        .collect::<Result<Vec<_>>>()?;
    if events.is_empty() {
        return Err(invalid_proposal("$: a JSON response is required"));
    }
    validate_tools(&events, allowed)?;
    if events.len() == 1
        && events[0].get("type").is_none()
        && events[0].get("structured_output").is_none()
        && events[0].get("result").is_none()
    {
        return proposal(&events[0], allowed, "$");
    }
    let results = events
        .iter()
        .filter(|event| event.get("type").and_then(Value::as_str) == Some("result"))
        .collect::<Vec<_>>();
    if results.len() != 1 {
        return Err(invalid_proposal(
            "events.result: exactly one result envelope is required",
        ));
    }
    let result = results[0];
    if result.get("subtype").and_then(Value::as_str) != Some("success") {
        return Err(Error::new(
            "AGENT_RESPONSE_FAILED",
            "result.subtype: must be success",
        ));
    }
    if result.get("is_error").and_then(Value::as_bool) != Some(false) {
        return Err(Error::new(
            "AGENT_RESPONSE_FAILED",
            "result.is_error: must be false",
        ));
    }
    if let Some(denials) = result
        .get("permission_denials")
        .filter(|value| !value.is_null())
    {
        let denials = denials
            .as_array()
            .ok_or_else(|| invalid_proposal("result.permission_denials: must be an array"))?;
        if !denials.is_empty() {
            return Err(Error::new(
                "AGENT_TOOL_USE",
                "result.permission_denials: permission-gated actions are not allowed",
            ));
        }
    }
    if let Some(value) = result
        .get("structured_output")
        .filter(|value| !value.is_null())
    {
        return proposal(value, allowed, "structured_output");
    }
    let text = result
        .get("result")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            invalid_proposal("result.result: a JSON string or structured_output object is required")
        })?;
    let value: UniqueJson = serde_json::from_str(text).map_err(|error| {
        invalid_proposal(&format!(
            "result.result: JSON syntax/unique-field rule: {error}"
        ))
    })?;
    // The fallback string is also untrusted and must not hide a tool call.
    validate_tools(std::slice::from_ref(&value.0), allowed)?;
    proposal(&value.0, allowed, "result.result")
}

fn proposal(
    value: &Value,
    allowed: &AllowedValues<'_>,
    path: &str,
) -> Result<ClassificationProposal> {
    const STRINGS: &[&str] = &[
        "category",
        "role",
        "when_to_use",
        "when_not",
        "improved_description",
    ];
    // Both dimensions are required from the agent, even when empty.
    const ARRAYS: &[&str] = &["stack", "stage", "tags", "overlaps_with"];
    let object = value
        .as_object()
        .ok_or_else(|| invalid_proposal(&format!("{path}: must be an object")))?;
    for key in object.keys() {
        if !STRINGS.contains(&key.as_str())
            && !ARRAYS.contains(&key.as_str())
            && key != "confidence"
        {
            return Err(invalid_proposal(&format!(
                "{path}.{key}: unknown classification field is forbidden"
            )));
        }
    }
    for key in STRINGS
        .iter()
        .chain(ARRAYS.iter())
        .copied()
        .chain(["confidence"])
    {
        if !object.contains_key(key) {
            return Err(invalid_proposal(&format!(
                "{path}.{key}: required field is missing"
            )));
        }
    }
    for key in STRINGS {
        if !value[key].is_string() {
            return Err(invalid_proposal(&format!("{path}.{key}: must be a string")));
        }
    }
    for key in ARRAYS {
        let array = value[key].as_array().ok_or_else(|| {
            invalid_proposal(&format!("{path}.{key}: must be an array of strings"))
        })?;
        for (index, item) in array.iter().enumerate() {
            if !item.is_string() {
                return Err(invalid_proposal(&format!(
                    "{path}.{key}[{index}]: must be a string"
                )));
            }
        }
    }
    if !value["confidence"].is_number() {
        return Err(invalid_proposal(&format!(
            "{path}.confidence: must be a number"
        )));
    }
    let proposal: ClassificationProposal =
        serde_json::from_value(value.clone()).map_err(|error| {
            invalid_proposal(&format!("{path}: classification field type rule: {error}"))
        })?;
    validate_proposal(&proposal, allowed)
        .map_err(|error| invalid_proposal(&format!("{path}.{}", error.message)))?;
    Ok(proposal)
}

fn collect_tools(
    value: &Value,
    path: &str,
    calls: &mut Vec<(String, Value)>,
    results: &mut Vec<(String, Value)>,
) {
    match value {
        Value::Object(object) => {
            match object.get("type").and_then(Value::as_str) {
                Some("tool_use" | "server_tool_use") => calls.push((path.into(), value.clone())),
                Some("tool_result") => results.push((path.into(), value.clone())),
                _ => {}
            }
            for (key, value) in object {
                collect_tools(value, &format!("{path}.{key}"), calls, results);
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                collect_tools(value, &format!("{path}[{index}]"), calls, results);
            }
        }
        _ => {}
    }
}

fn validate_tools(events: &[Value], allowed: &AllowedValues<'_>) -> Result<()> {
    // The runner already stopped any session whose first init event offered
    // another tool; a later init event in the same stream gets the same rule.
    for (index, event) in events.iter().enumerate() {
        if event.get("type").and_then(Value::as_str) == Some("system")
            && event.get("subtype").and_then(Value::as_str) == Some("init")
        {
            super::init_tools(event).map_err(|detail| {
                Error::new(
                    "AGENT_TOOLS_NOT_ISOLATED",
                    format!("events[{index}]: {detail}"),
                )
            })?;
        }
    }
    let mut calls = Vec::new();
    let mut results = Vec::new();
    for (index, event) in events.iter().enumerate() {
        collect_tools(event, &format!("events[{index}]"), &mut calls, &mut results);
    }
    // Check all names before any schema errors can trigger a retry.
    for (path, call) in &calls {
        if call["type"] != "tool_use" {
            return Err(Error::new(
                "AGENT_TOOL_USE",
                format!(
                    "{path}.type: server-side tools are forbidden; only a local StructuredOutput tool_use is allowed"
                ),
            ));
        }
        if call["name"] != "StructuredOutput" {
            let name = call["name"]
                .as_str()
                .unwrap_or("<missing>")
                .chars()
                .take(80)
                .collect::<String>();
            return Err(Error::new(
                "AGENT_TOOL_USE",
                format!("{path}.name: only StructuredOutput is allowed; received {name}"),
            ));
        }
    }
    let mut ids = BTreeSet::new();
    for (path, call) in &calls {
        let id = call["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                Error::new(
                    "AGENT_TOOL_USE",
                    format!("{path}.id: StructuredOutput requires a nonempty call ID"),
                )
            })?;
        if !ids.insert(id) {
            return Err(Error::new(
                "AGENT_TOOL_USE",
                format!("{path}.id: duplicate tool call ID is forbidden"),
            ));
        }
    }
    for (path, result) in results {
        if !result["tool_use_id"]
            .as_str()
            .is_some_and(|id| ids.contains(id))
        {
            return Err(Error::new(
                "AGENT_TOOL_USE",
                format!("{path}.tool_use_id: must refer to a recorded StructuredOutput call"),
            ));
        }
    }
    for (path, call) in calls {
        proposal(&call["input"], allowed, &format!("{path}.input"))?;
    }
    Ok(())
}

// Value normally overwrites duplicate object keys. Reject them before selecting
// the final result, preserving the strict contract for nested proposal fields.
// The init gate parses with it too: with plain `Value` the last of two `tools`
// keys would win, and a tool list could hide behind a harmless one.
pub(super) struct UniqueJson(pub(super) Value);

impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        struct JsonVisitor;
        impl<'de> Visitor<'de> for JsonVisitor {
            type Value = Value;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("JSON with unique object fields")
            }
            fn visit_bool<E: serde::de::Error>(self, value: bool) -> std::result::Result<Value, E> {
                Ok(value.into())
            }
            fn visit_i64<E: serde::de::Error>(self, value: i64) -> std::result::Result<Value, E> {
                Ok(value.into())
            }
            fn visit_u64<E: serde::de::Error>(self, value: u64) -> std::result::Result<Value, E> {
                Ok(value.into())
            }
            fn visit_f64<E: serde::de::Error>(self, value: f64) -> std::result::Result<Value, E> {
                serde_json::Number::from_f64(value)
                    .map(Value::Number)
                    .ok_or_else(|| E::custom("number must be finite"))
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> std::result::Result<Value, E> {
                Ok(value.into())
            }
            fn visit_string<E: serde::de::Error>(
                self,
                value: String,
            ) -> std::result::Result<Value, E> {
                Ok(value.into())
            }
            fn visit_none<E: serde::de::Error>(self) -> std::result::Result<Value, E> {
                Ok(Value::Null)
            }
            fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Value, E> {
                Ok(Value::Null)
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> std::result::Result<Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element::<UniqueJson>()? {
                    values.push(value.0);
                }
                Ok(Value::Array(values))
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(serde::de::Error::custom(format!(
                            "field {key}: duplicate object fields are forbidden"
                        )));
                    }
                    values.insert(key, map.next_value::<UniqueJson>()?.0);
                }
                Ok(Value::Object(values))
            }
        }
        deserializer.deserialize_any(JsonVisitor).map(Self)
    }
}

/// A Claude usage-limit or credit error in the stream-json output, if any:
/// a rejected `rate_limit_event`, an assistant event whose `error` is
/// `rate_limit` or `billing_error`, or an error result whose text names a
/// limit or credits. Only a fixed message and a sanitized reset time leave
/// this function, never the raw output.
pub(super) fn usage_limit(stdout: &[u8]) -> Option<Error> {
    let text = String::from_utf8_lossy(stdout);
    let mut credit = false;
    let mut limit = false;
    let mut reset = None;
    for line in text.lines() {
        let Ok(UniqueJson(event)) = serde_json::from_str::<UniqueJson>(line) else {
            continue;
        };
        match event.get("type").and_then(Value::as_str) {
            Some("rate_limit_event") => {
                limit |= event
                    .pointer("/rate_limit_info/status")
                    .and_then(Value::as_str)
                    == Some("rejected");
            }
            Some("assistant") => match event.get("error").and_then(Value::as_str) {
                Some("rate_limit") => limit = true,
                Some("billing_error") => credit = true,
                _ => {}
            },
            Some("result") if event.get("is_error").and_then(Value::as_bool) == Some(true) => {
                let message = event
                    .get("result")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_lowercase();
                if message.contains("credit balance")
                    || message.contains("usage credits")
                    || message.contains("billing")
                {
                    credit = true;
                } else if message.contains("limit") || message.contains("429") {
                    limit = true;
                }
                reset = reset.or_else(|| reset_time(&message));
            }
            _ => {}
        }
    }
    let when = reset.map(|r| format!(" (resets {r})")).unwrap_or_default();
    if credit {
        Some(Error::new(
            "AGENT_CREDIT_EXHAUSTED",
            format!(
                "Claude Code reported that its credit balance is exhausted{when}. Classification stopped; Kinana uses your Claude Code plan and credits."
            ),
        ))
    } else if limit {
        Some(Error::new(
            "AGENT_USAGE_LIMIT",
            format!(
                "Claude Code reported a usage limit{when}. Classification stopped; Kinana uses your Claude Code plan limits."
            ),
        ))
    } else {
        None
    }
}

/// Claude Code reported that it has no valid sign-in: the stream-json
/// assistant error `authentication_failed`, or a failed result whose fixed
/// wording asks to run `/login`. The wording was not observed live (owner
/// decision, 2026-11-18: no extra live call for it); only the code is reported.
pub(super) fn not_logged_in(stdout: &[u8]) -> Option<Error> {
    let text = String::from_utf8_lossy(stdout);
    let found = text.lines().any(|line| {
        let Ok(UniqueJson(event)) = serde_json::from_str::<UniqueJson>(line) else {
            return false;
        };
        match event.get("type").and_then(Value::as_str) {
            Some("assistant") => {
                event.get("error").and_then(Value::as_str) == Some("authentication_failed")
            }
            Some("result") if event.get("is_error").and_then(Value::as_bool) == Some(true) => {
                let message = event
                    .get("result")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_lowercase();
                [
                    "/login",
                    "not logged in",
                    "invalid api key",
                    "oauth token has expired",
                    "authentication_error",
                ]
                .iter()
                .any(|wording| message.contains(wording))
            }
            _ => false,
        }
    });
    found.then(|| {
        Error::new(
            "AGENT_NOT_LOGGED_IN",
            "Claude Code is not signed in for this app. Kinana uses the sign-in Claude Code saves with `claude /login`, not API keys or settings from your shell. Classification stopped.",
        )
    })
}

/// `resets 5pm (asia/riyadh)` from a limit message: at most 48 characters of
/// letters, digits, spaces and `:()/_+-.`, or nothing.
fn reset_time(message: &str) -> Option<String> {
    let start = message.find("resets ")? + "resets ".len();
    let value: String = message[start..]
        .chars()
        .take_while(|c| *c != '\n')
        .take(48)
        .collect();
    let value = value.trim().trim_end_matches('.').to_owned();
    (!value.is_empty()
        && value
            .chars()
            .all(|c| c.is_alphanumeric() || " :()/_+-.".contains(c)))
    .then_some(value)
}

#[cfg(test)]
mod usage_limit_tests {
    use super::usage_limit;

    #[test]
    fn limits_and_credits_are_recognized_from_each_event_form() {
        let limit = usage_limit(b"{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":true,\"result\":\"You've hit your session limit \\u00b7 resets 5pm (Asia/Riyadh)\"}\n").unwrap();
        assert_eq!(limit.code, "AGENT_USAGE_LIMIT");
        assert!(
            limit.message.contains("(resets 5pm (asia/riyadh))"),
            "{}",
            limit.message
        );
        let event = usage_limit(
            b"{\"type\":\"rate_limit_event\",\"rate_limit_info\":{\"status\":\"rejected\"}}\n",
        )
        .unwrap();
        assert_eq!(event.code, "AGENT_USAGE_LIMIT");
        let assistant =
            usage_limit(b"{\"type\":\"assistant\",\"error\":\"billing_error\"}\n").unwrap();
        assert_eq!(assistant.code, "AGENT_CREDIT_EXHAUSTED");
        let credit = usage_limit(
            b"{\"type\":\"result\",\"is_error\":true,\"result\":\"Credit balance is too low\"}",
        )
        .unwrap();
        assert_eq!(credit.code, "AGENT_CREDIT_EXHAUSTED");
    }

    #[test]
    fn ordinary_output_and_allowed_events_are_not_limits() {
        assert!(usage_limit(b"{\"type\":\"rate_limit_event\",\"rate_limit_info\":{\"status\":\"allowed\"}}\n{\"type\":\"result\",\"is_error\":false,\"result\":\"usage limit\"}\n").is_none());
        assert!(
            usage_limit(
                b"{\"type\":\"result\",\"is_error\":true,\"result\":\"prompt is too long\"}"
            )
            .is_none()
        );
        assert!(usage_limit(b"not json").is_none());
        for stream in [
            &b"{\"type\":\"assistant\",\"error\":\"authentication_failed\"}\n"[..],
            b"{\"type\":\"result\",\"is_error\":true,\"result\":\"Not logged in \\u00b7 Please run /login\"}",
            b"{\"type\":\"result\",\"is_error\":true,\"result\":\"Invalid API key \\u00b7 Please run /login\"}",
        ] {
            let error = super::not_logged_in(stream).unwrap();
            assert_eq!(error.code, "AGENT_NOT_LOGGED_IN");
            assert!(usage_limit(stream).is_none());
        }
        assert!(
            super::not_logged_in(
                b"{\"type\":\"result\",\"is_error\":false,\"result\":\"run /login later\"}"
            )
            .is_none()
        );
        assert!(
            super::not_logged_in(b"{\"type\":\"assistant\",\"error\":\"rate_limit\"}").is_none()
        );
        // The raw message never reaches the error beyond a sanitized reset.
        let odd = usage_limit(b"{\"type\":\"result\",\"is_error\":true,\"result\":\"limit hit; token sk-abc resets `rm -rf`\"}").unwrap();
        assert!(
            !odd.message.contains("sk-abc") && !odd.message.contains("rm"),
            "{}",
            odd.message
        );
    }
}
