use serde_json::Value;

pub fn document() -> Value {
    let text = include_str!("../../../../docs/cli-json.md");
    let block = text
        .split_once("<!-- cli-json-contract:start -->\n```json\n")
        .expect("documented contract start")
        .1
        .split_once("\n```\n<!-- cli-json-contract:end -->")
        .expect("documented contract end")
        .0;
    serde_json::from_str(block).expect("documented contract must be JSON")
}

pub fn validate(document: &Value, schema: &Value, value: &Value) -> Result<(), String> {
    validate_at(document, schema, value, "$")
}

fn validate_at(doc: &Value, schema: &Value, value: &Value, path: &str) -> Result<(), String> {
    if let Some(reference) = schema["$ref"].as_str() {
        let name = reference
            .strip_prefix("#/$defs/")
            .expect("local definition");
        return validate_at(doc, &doc["$defs"][name], value, path);
    }
    if let Some(choices) = schema["anyOf"].as_array() {
        if choices
            .iter()
            .any(|choice| validate_at(doc, choice, value, path).is_ok())
        {
            return Ok(());
        }
        return Err(format!("{path}: no allowed type matched {value}"));
    }
    let kind = schema["type"].as_str().expect("validated schema type");
    let matches = match kind {
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "null" => value.is_null(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        other => panic!("Unsupported schema type: {other}"),
    };
    if !matches {
        return Err(format!("{path}: expected {kind}, got {value}"));
    }
    if let Some(choices) = schema["enum"].as_array()
        && !choices.contains(value)
    {
        return Err(format!("{path}: value is outside the documented enum"));
    }
    if let Some(items) = value.as_array() {
        for (index, item) in items.iter().enumerate() {
            validate_at(doc, &schema["items"], item, &format!("{path}[{index}]"))?;
        }
    }
    if let Some(fields) = value.as_object() {
        let properties = schema["properties"].as_object().unwrap();
        for key in schema["required"].as_array().unwrap() {
            let key = key.as_str().unwrap();
            if !fields.contains_key(key) {
                return Err(format!("{path}.{key}: missing required field"));
            }
        }
        for (key, value) in fields {
            let field = properties
                .get(key)
                .ok_or_else(|| format!("{path}.{key}: undocumented field"))?;
            validate_at(doc, field, value, &format!("{path}.{key}"))?;
        }
    }
    Ok(())
}

// Refuse unsupported schema keywords rather than silently skipping a new rule.
pub fn check_schema(doc: &Value, schema: &Value) {
    let object = schema.as_object().expect("schema object");
    let allowed: &[&str] = if let Some(reference) = schema["$ref"].as_str() {
        let name = reference.strip_prefix("#/$defs/").unwrap();
        assert!(doc["$defs"].get(name).is_some(), "unknown {reference}");
        &["$ref"]
    } else if let Some(choices) = schema["anyOf"].as_array() {
        assert!(!choices.is_empty());
        for choice in choices {
            check_schema(doc, choice);
        }
        &["anyOf"]
    } else {
        match schema["type"].as_str().expect("schema type") {
            "object" => {
                assert_eq!(schema["additionalProperties"], false);
                let fields = schema["properties"].as_object().unwrap();
                let required = schema["required"].as_array().unwrap();
                let unique: std::collections::BTreeSet<_> =
                    required.iter().map(|v| v.as_str().unwrap()).collect();
                assert_eq!(unique.len(), required.len(), "duplicate required field");
                for key in unique {
                    assert!(fields.contains_key(key));
                }
                for field in fields.values() {
                    check_schema(doc, field);
                }
                &["type", "properties", "required", "additionalProperties"]
            }
            "array" => {
                check_schema(doc, &schema["items"]);
                &["type", "items"]
            }
            "string" if schema.get("enum").is_some() => {
                let choices = schema["enum"].as_array().unwrap();
                let unique: std::collections::BTreeSet<_> = choices
                    .iter()
                    .map(|value| value.as_str().unwrap())
                    .collect();
                assert!(!choices.is_empty());
                assert_eq!(unique.len(), choices.len(), "duplicate enum value");
                &["type", "enum"]
            }
            "string" | "boolean" | "integer" | "number" | "null" => &["type"],
            other => panic!("Unsupported schema type: {other}"),
        }
    };
    for key in object.keys() {
        assert!(
            allowed.contains(&key.as_str()),
            "unchecked schema keyword {key}"
        );
    }
}
