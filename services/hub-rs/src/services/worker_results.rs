//! Terminal report contracts. Result schemas deliberately validate only the
//! desktop's supported subset; unsupported keywords never reject a report.
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::OnceLock;
pub const RESULT_SCHEMA_MAX: usize = 4096;
pub const RESULT_MAX: usize = 8192;
pub const ESCALATION_MAX: usize = 4096;

#[derive(Default, Debug, Serialize)]
pub struct Report {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
impl Report {
    fn error(error: impl Into<String>) -> Self {
        Self {
            error: Some(error.into()),
            ..Self::default()
        }
    }
}
#[derive(Default, Debug, Serialize)]
pub struct Escalation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
fn units(text: &str) -> usize {
    text.encode_utf16().count()
}
pub fn check_schema(schema: &Value) -> Result<(), String> {
    if !schema.is_object() {
        return Err("resultSchema must be a JSON Schema object".into());
    }
    let size = units(&schema.to_string());
    if size > RESULT_SCHEMA_MAX {
        return Err(format!(
            "resultSchema is {size} bytes; the limit is {RESULT_SCHEMA_MAX}"
        ));
    }
    Ok(())
}
pub fn result_contract(schema: &Value) -> Result<String, String> {
    check_schema(schema)?;
    let pretty = serde_json::to_string_pretty(schema).map_err(|e| e.to_string())?;
    Ok(format!(
        "STRUCTURED RESULT CONTRACT. Whoever dispatched you asked for a machine-readable result as well as your prose. When your work is finished, write your normal summary first. It is read by a human and must not be dropped. Then END your final message with a fenced code block tagged `wks-result` containing ONE JSON object that validates against this schema:\n\n{pretty}\n\nFormat exactly:\n\n```wks-result\n{{ ... }}\n```\n\nEmit the block only in your FINAL message (not mid-task), only once, and put nothing after it. Report what actually happened. An empty list or a null is a truthful answer; an invented value is not. If you could not complete the task, still emit the block with whatever fields are true and say so in the prose."
    ))
}
pub const ESCALATION_CONTRACT: &str = "STRUCTURED WORKER ESCALATION CONTRACT. If you cannot safely complete the task because you lack authority or need a manager/user decision, do not only refuse in prose. Stop and write a concise explanation first, then END your final message with exactly one fenced `wks-escalation` JSON block in this exact shape:\n\n```wks-escalation\n{\n  \"type\": \"worker-escalation\",\n  \"status\": \"blocked\",\n  \"reason\": \"concise blocker\",\n  \"requiredAuthorityOrDecision\": \"specific authority or decision needed\",\n  \"changed\": false,\n  \"nextAction\": \"useful next action for the manager\"\n}\n```\n\nUse this only as a terminal escalation, not for a successful completion or a routine progress update. Report truthfully whether anything changed. If a separate `wks-result` contract is also present, emit `wks-result` when you complete the task; when you escalate, emit `wks-escalation` instead. Emit the chosen terminal block only once and put nothing after it.";

fn fences() -> &'static regex::Regex {
    static FENCES: OnceLock<regex::Regex> = OnceLock::new();
    FENCES.get_or_init(|| {
        regex::Regex::new(r"(?s)```[ \t]*([A-Za-z0-9_-]*)[ \t]*\r?\n(.*?)```").unwrap()
    })
}
pub fn extract_result(text: &str) -> Option<&str> {
    let (mut tagged, mut fallback) = (None, None);
    for capture in fences().captures_iter(text) {
        let body = capture.get(2).unwrap().as_str();
        match capture[1].to_ascii_lowercase().as_str() {
            "wks-result" => tagged = Some(body),
            "json" => fallback = Some(body),
            _ => (),
        }
    }
    tagged.or(fallback)
}
fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
        Value::Number(n) => {
            if n.as_f64().is_some_and(|n| n.fract() == 0.) {
                "integer"
            } else {
                "number"
            }
        }
    }
}
fn matches_type(value: &Value, requested: &str) -> bool {
    match requested {
        "object" | "array" | "string" | "boolean" | "null" => kind(value) == requested,
        "integer" => kind(value) == "integer",
        "number" => value.is_number(),
        _ => true,
    }
}
fn equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        _ => a == b,
    }
}
fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.into()
    } else {
        format!("{path}.{key}")
    }
}
pub fn validate(value: &Value, schema: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    walk(value, schema, "", &mut errors);
    errors.truncate(8);
    errors
}
fn walk(value: &Value, schema: &Value, path: &str, errors: &mut Vec<String>) {
    if errors.len() >= 8 || !schema.is_object() {
        return;
    }
    let at = if path.is_empty() { "result" } else { path };
    let types: Vec<_> = match &schema["type"] {
        Value::String(s) => vec![s.as_str()],
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    if !types.is_empty() && !types.iter().any(|t| matches_type(value, t)) {
        errors.push(format!(
            "{at}: expected {}, got {}",
            types.join(" or "),
            kind(value)
        ));
        return;
    }
    if let Some(choices) = schema["enum"].as_array() {
        if !choices.iter().any(|choice| equal(choice, value)) {
            errors.push(format!("{at}: {value} is not one of {}", schema["enum"]));
        }
    }
    if let Some(map) = value.as_object() {
        for key in schema["required"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if !map.contains_key(key) {
                errors.push(format!("{}: required property missing", join(path, key)));
            }
        }
        for (key, child) in map {
            if let Some(prop) = schema["properties"]
                .as_object()
                .and_then(|properties| properties.get(key))
            {
                walk(child, prop, &join(path, key), errors);
            } else if schema["additionalProperties"] == false {
                errors.push(format!("{}: unexpected property", join(path, key)));
            }
        }
    }
    if let (Some(array), Some(items)) = (value.as_array(), schema.get("items")) {
        for (index, child) in array.iter().enumerate() {
            walk(child, items, &format!("{at}[{index}]"), errors);
        }
    }
}
pub fn read_result(message: &str, schema: &Value) -> Report {
    let Some(block) = extract_result(message) else {
        return Report::error(
            "no `wks-result` block in the worker's final message — it did not honor the result contract",
        );
    };
    let parsed: Value = match serde_json::from_str(block) {
        Ok(value) => value,
        Err(error) => {
            return Report::error(format!("the `wks-result` block is not valid JSON: {error}"));
        }
    };
    let errors = validate(&parsed, schema);
    if !errors.is_empty() {
        return Report::error(format!(
            "the result does not match the requested schema: {}",
            errors.join("; ")
        ));
    }
    let mut json = serde_json::to_string_pretty(&parsed).expect("JSON value serializes");
    let size = units(&json);
    if size > RESULT_MAX {
        let utf16: Vec<_> = json.encode_utf16().take(RESULT_MAX).collect();
        json = format!(
            "{}\n[truncated: {size} bytes of validated result]",
            String::from_utf16_lossy(&utf16)
        );
    }
    Report {
        json: Some(json),
        error: None,
    }
}
pub fn read_escalation(message: &str) -> Option<Escalation> {
    static ESCALATIONS: OnceLock<regex::Regex> = OnceLock::new();
    let pattern = ESCALATIONS.get_or_init(|| {
        regex::Regex::new(r"(?is)```[ \t]*wks-escalation[ \t]*\r?\n(.*?)```").unwrap()
    });
    let body = pattern
        .captures_iter(message)
        .last()?
        .get(1)
        .unwrap()
        .as_str();
    let result = (|| -> Result<Value, String> {
        let size = units(body);
        if size > ESCALATION_MAX {
            return Err(format!(
                "the `wks-escalation` block is {size} bytes; the limit is {ESCALATION_MAX}"
            ));
        }
        let mut value: Value = serde_json::from_str(body)
            .map_err(|e| format!("the `wks-escalation` block is not valid JSON: {e}"))?;
        let map = value
            .as_object_mut()
            .ok_or_else(|| "the `wks-escalation` block must contain one JSON object".to_owned())?;
        let keys = [
            "type",
            "status",
            "reason",
            "requiredAuthorityOrDecision",
            "changed",
            "nextAction",
        ];
        let extra: Vec<_> = map
            .keys()
            .filter(|key| !keys.contains(&key.as_str()))
            .cloned()
            .collect();
        if !extra.is_empty() {
            return Err(format!(
                "the escalation has unexpected properties: {}",
                extra.join(", ")
            ));
        }
        for (key, expected) in [("type", "worker-escalation"), ("status", "blocked")] {
            if map.get(key) != Some(&json!(expected)) {
                return Err(format!("{key}: expected {expected:?}"));
            }
        }
        for key in ["reason", "requiredAuthorityOrDecision", "nextAction"] {
            let value = map
                .get(key)
                .and_then(Value::as_str)
                .map(|value| value.trim_matches(super::progress::js_space))
                .filter(|s| !s.is_empty())
                .ok_or_else(|| format!("{key}: expected a non-empty string"))?
                .to_owned();
            map.insert(key.into(), value.into());
        }
        if !map.get("changed").is_some_and(Value::is_boolean) {
            return Err("changed: expected boolean".into());
        }
        Ok(value)
    })();
    Some(match result {
        Ok(value) => Escalation {
            json: Some(serde_json::to_string_pretty(&value).unwrap()),
            value: Some(value),
            error: None,
        },
        Err(error) => Escalation {
            error: Some(error),
            ..Escalation::default()
        },
    })
}
