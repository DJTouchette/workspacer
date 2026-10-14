//! MCP elicitations (`elicitation/create`) shown as the question card.
//!
//! Both the Claude stream (`control_request {subtype:"elicitation"}`) and
//! Codex (`mcpServer/elicitation/request`) relay an MCP server's request for
//! user input. A session has ONE pending slot (see `session::state::Pending`),
//! and adding a third card kind would ripple through every client, so an
//! elicitation is mapped onto the `Pending::Question` card every client
//! already renders and answers through `/answer`:
//!
//! * **form mode** — one question per top-level property of the flat
//!   `requestedSchema` (MCP restricts it to primitives): an enum becomes its
//!   options (the `enumNames` / `oneOf[].title` labels), a boolean becomes
//!   `Yes` / `No`, a string / number / integer is free text. A multi-select
//!   (`type:"array"` of an enum) is a `multi_select` question. A final
//!   consent question — `Submit` / `Decline` — closes the card, because an
//!   MCP form sends the user's data to a third party and every MCP client
//!   offers an explicit decline.
//! * **url mode** — a single question whose text carries the URL, with
//!   options `Open` (its description is the URL itself, for a client that
//!   opens it) and `Decline`.
//!
//! Anything else (a nested object or array of objects, a provider-private
//! mode) is [`Unsupported`](FormError::Unsupported): the drivers decline it
//! and say so in the conversation, as they did for every elicitation before.
//!
//! The answer comes back as the ordinary [`ManagedAnswer`] (one string per
//! question, in display order) and is converted into the elicitation result
//! `{action, content}` with each value typed per the schema.

use serde_json::{json, Map, Value};

use crate::session::state::{PendingOption, PendingQuestion};
use crate::session::ManagedAnswer;

/// The consent question's options. `Submit` is option 1, so a client that
/// leaves it unanswered (or a bare `option: 1`) sends the form.
pub(crate) const SUBMIT_LABEL: &str = "Submit";
pub(crate) const DECLINE_LABEL: &str = "Decline";
/// url mode's options.
pub(crate) const OPEN_LABEL: &str = "Open";

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FieldKind {
    Text,
    Number {
        integer: bool,
    },
    Bool,
    /// `values[i]` is sent when `labels[i]` (the card option) is chosen.
    Enum {
        values: Vec<String>,
    },
    MultiEnum {
        values: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Field {
    key: String,
    kind: FieldKind,
    required: bool,
    default: Option<Value>,
    /// The card's option labels, index-aligned with an enum's `values`.
    labels: Vec<String>,
}

/// A parked elicitation's answer key: what each question of its card means.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ElicitationForm {
    Form { fields: Vec<Field> },
    Url,
}

/// Why an elicitation cannot be shown as a card.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FormError {
    Unsupported(String),
}

/// What to send back to the MCP server.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ElicitationReply {
    Accept(Option<Value>),
    Decline,
    /// The user answered, but not in a shape the schema accepts (a required
    /// field left empty, `"abc"` for a number, a value outside an enum).
    /// Declined — with this explanation surfaced — rather than sent wrong.
    Invalid(String),
}

impl ElicitationReply {
    /// The MCP `ElicitResult` body, `{action, content}`.
    pub(crate) fn result(&self) -> Value {
        match self {
            Self::Accept(content) => json!({ "action": "accept", "content": content }),
            Self::Decline | Self::Invalid(_) => json!({ "action": "decline", "content": null }),
        }
    }
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn strings(v: Option<&Value>) -> Option<Vec<String>> {
    v.and_then(Value::as_array)?
        .iter()
        .map(|s| s.as_str().map(str::to_owned))
        .collect()
}

/// `oneOf` / `anyOf` const options: `(values, labels)`.
fn const_options(v: Option<&Value>) -> Option<(Vec<String>, Vec<String>)> {
    let list = v.and_then(Value::as_array)?;
    let mut values = Vec::new();
    let mut labels = Vec::new();
    for o in list {
        let value = o.get("const")?.as_str()?.to_owned();
        labels.push(text(o, "title").unwrap_or(&value).to_owned());
        values.push(value);
    }
    Some((values, labels))
}

fn field_from(key: &str, schema: &Value, required: bool) -> Result<Field, FormError> {
    let unsupported = || {
        FormError::Unsupported(format!(
            "field `{key}` is not a flat string, number, boolean or enum"
        ))
    };
    let ty = schema.get("type").and_then(Value::as_str);
    let (kind, labels) = match ty {
        Some("string") => {
            if let Some(values) = strings(schema.get("enum")) {
                let labels = strings(schema.get("enumNames"))
                    .filter(|n| n.len() == values.len())
                    .unwrap_or_else(|| values.clone());
                (FieldKind::Enum { values }, labels)
            } else if let Some((values, labels)) = const_options(schema.get("oneOf")) {
                (FieldKind::Enum { values }, labels)
            } else {
                (FieldKind::Text, Vec::new())
            }
        }
        Some("number") => (FieldKind::Number { integer: false }, Vec::new()),
        Some("integer") => (FieldKind::Number { integer: true }, Vec::new()),
        Some("boolean") => (FieldKind::Bool, vec!["Yes".into(), "No".into()]),
        Some("array") => {
            let items = schema.get("items").ok_or_else(unsupported)?;
            let (values, labels) = if let Some(values) = strings(items.get("enum")) {
                (values.clone(), values)
            } else {
                const_options(items.get("anyOf").or_else(|| items.get("oneOf")))
                    .ok_or_else(unsupported)?
            };
            (FieldKind::MultiEnum { values }, labels)
        }
        _ => return Err(unsupported()),
    };
    if labels.is_empty() && matches!(kind, FieldKind::Enum { .. } | FieldKind::MultiEnum { .. }) {
        return Err(unsupported());
    }
    Ok(Field {
        key: key.to_owned(),
        kind,
        required,
        default: schema.get("default").cloned().filter(|d| !d.is_null()),
        labels,
    })
}

/// Map a form-mode elicitation onto question-card questions. `order` is the
/// properties' order as the server listed them (Claude sends it as
/// `property_order`; serde_json's map is sorted); keys missing from it
/// follow in map order.
pub(crate) fn form_card(
    server: &str,
    message: &str,
    schema: &Value,
    order: Option<&[String]>,
) -> Result<(ElicitationForm, Vec<PendingQuestion>), FormError> {
    let empty = Map::new();
    let properties = match schema.get("properties") {
        None | Some(Value::Null) => &empty,
        Some(Value::Object(p)) => p,
        Some(_) => {
            return Err(FormError::Unsupported(
                "`properties` is not an object".into(),
            ))
        }
    };
    if let Some(ty) = schema.get("type").and_then(Value::as_str) {
        if ty != "object" {
            return Err(FormError::Unsupported(format!("schema type `{ty}`")));
        }
    }
    let required: Vec<String> = strings(schema.get("required")).unwrap_or_default();
    let mut keys: Vec<&String> = Vec::new();
    for key in order.into_iter().flatten() {
        if let Some((k, _)) = properties.get_key_value(key) {
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
    }
    for k in properties.keys() {
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
    let mut fields = Vec::new();
    let mut questions = Vec::new();
    for key in keys {
        let prop = &properties[key];
        let field = field_from(key, prop, required.contains(key))?;
        let title = text(prop, "title").unwrap_or(key);
        let mut question = text(prop, "description").unwrap_or(title).to_owned();
        if field.required {
            question.push_str(" (required)");
        }
        questions.push(PendingQuestion {
            question,
            header: Some(title.to_owned()),
            multi_select: matches!(field.kind, FieldKind::MultiEnum { .. }),
            options: field
                .labels
                .iter()
                .map(|label| PendingOption {
                    label: label.clone(),
                    description: None,
                })
                .collect(),
            secret: false,
        });
        fields.push(field);
    }
    let ask = message.trim();
    questions.push(PendingQuestion {
        question: if ask.is_empty() {
            format!("Send these answers to the MCP server `{server}`?")
        } else {
            format!("{ask}\n\nSend these answers to the MCP server `{server}`?")
        },
        header: Some(format!("{server} asks")),
        multi_select: false,
        options: vec![
            PendingOption {
                label: SUBMIT_LABEL.into(),
                description: Some(format!("Send the answers to `{server}`")),
            },
            PendingOption {
                label: DECLINE_LABEL.into(),
                description: Some("Send nothing; the server is told you declined".into()),
            },
        ],
        secret: false,
    });
    Ok((ElicitationForm::Form { fields }, questions))
}

/// Map a url-mode elicitation onto a single Open / Decline question. The
/// `Open` option's description is the URL, so a client can open it.
pub(crate) fn url_card(
    server: &str,
    message: &str,
    url: &str,
) -> (ElicitationForm, Vec<PendingQuestion>) {
    let ask = message.trim();
    let question = if ask.is_empty() {
        format!("The MCP server `{server}` asks you to open:\n{url}")
    } else {
        format!("{ask}\n\nThe MCP server `{server}` asks you to open:\n{url}")
    };
    (
        ElicitationForm::Url,
        vec![PendingQuestion {
            question,
            header: Some(format!("{server} link")),
            multi_select: false,
            options: vec![
                PendingOption {
                    label: OPEN_LABEL.into(),
                    description: Some(url.to_owned()),
                },
                PendingOption {
                    label: DECLINE_LABEL.into(),
                    description: None,
                },
            ],
            secret: false,
        }],
    )
}

/// The raw answer string for question `i` and whether it is literal text
/// (an `answer_kinds` `"text"` tag: never remapped as an option number).
fn raw_answer(answer: &ManagedAnswer, i: usize) -> (Option<String>, bool) {
    let raw = match &answer.answers {
        Some(list) => list.get(i).cloned(),
        None if i == 0 => answer
            .option
            .map(|o| o.to_string())
            .or_else(|| answer.text.clone()),
        None => None,
    };
    let literal = answer
        .answer_kinds
        .as_ref()
        .and_then(|k| k.get(i))
        .is_some_and(|k| k == "text");
    (
        raw.map(|r| r.trim().to_owned()).filter(|r| !r.is_empty()),
        literal,
    )
}

/// Resolve one picked entry against `labels`: a 1-indexed option number
/// (unless literal), an exact label, or an exact underlying value.
fn pick(entry: &str, literal: bool, labels: &[String], values: Option<&[String]>) -> Option<usize> {
    if !literal {
        if let Some(n) = entry.parse::<usize>().ok().and_then(|n| n.checked_sub(1)) {
            if n < labels.len() {
                return Some(n);
            }
        }
    }
    labels
        .iter()
        .position(|l| l == entry)
        .or_else(|| labels.iter().position(|l| l.eq_ignore_ascii_case(entry)))
        .or_else(|| values.and_then(|v| v.iter().position(|v| v == entry)))
}

fn field_value(field: &Field, raw: &str, literal: bool) -> Result<Value, String> {
    let bad = |what: &str| format!("`{}` needs {what}, got “{raw}”", field.key);
    match &field.kind {
        FieldKind::Text => Ok(Value::String(raw.to_owned())),
        FieldKind::Number { integer: true } => raw
            .parse::<i64>()
            .map(Value::from)
            .map_err(|_| bad("a whole number")),
        FieldKind::Number { integer: false } => raw
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .ok_or_else(|| bad("a number")),
        FieldKind::Bool => match pick(raw, literal, &field.labels, None) {
            Some(0) => Ok(Value::Bool(true)),
            Some(_) => Ok(Value::Bool(false)),
            None => match raw.to_ascii_lowercase().as_str() {
                "true" | "y" => Ok(Value::Bool(true)),
                "false" | "n" => Ok(Value::Bool(false)),
                _ => Err(bad("yes or no")),
            },
        },
        FieldKind::Enum { values } => pick(raw, literal, &field.labels, Some(values))
            .map(|i| Value::String(values[i].clone()))
            .ok_or_else(|| bad(&format!("one of {}", field.labels.join(", ")))),
        FieldKind::MultiEnum { values } => raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|entry| {
                pick(entry, literal, &field.labels, Some(values))
                    .map(|i| Value::String(values[i].clone()))
                    .ok_or_else(|| bad(&format!("some of {}", field.labels.join(", "))))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
    }
}

/// Convert the user's `/answer` into the elicitation result.
pub(crate) fn reply(form: &ElicitationForm, answer: &ManagedAnswer) -> ElicitationReply {
    match form {
        ElicitationForm::Url => match raw_answer(answer, 0) {
            (Some(raw), literal) => {
                let labels = [OPEN_LABEL.to_owned(), DECLINE_LABEL.to_owned()];
                match pick(&raw, literal, &labels, None) {
                    Some(0) => ElicitationReply::Accept(None),
                    _ => ElicitationReply::Decline,
                }
            }
            (None, _) => ElicitationReply::Decline,
        },
        ElicitationForm::Form { fields } => {
            // The consent question follows the fields. Unanswered = submitted
            // (the client sent the form); only an explicit Decline declines.
            let consent = [SUBMIT_LABEL.to_owned(), DECLINE_LABEL.to_owned()];
            if let (Some(raw), literal) = raw_answer(answer, fields.len()) {
                if pick(&raw, literal, &consent, None) == Some(1) {
                    return ElicitationReply::Decline;
                }
            }
            let mut content = Map::new();
            for (i, field) in fields.iter().enumerate() {
                match raw_answer(answer, i) {
                    (Some(raw), literal) => match field_value(field, &raw, literal) {
                        Ok(v) => {
                            content.insert(field.key.clone(), v);
                        }
                        Err(why) => return ElicitationReply::Invalid(why),
                    },
                    (None, _) => {
                        if let Some(default) = &field.default {
                            content.insert(field.key.clone(), default.clone());
                        } else if field.required {
                            return ElicitationReply::Invalid(format!(
                                "`{}` is required but was left empty",
                                field.key
                            ));
                        }
                    }
                }
            }
            ElicitationReply::Accept(Some(Value::Object(content)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answers(list: &[&str]) -> ManagedAnswer {
        ManagedAnswer {
            answers: Some(list.iter().map(|s| s.to_string()).collect()),
            ..Default::default()
        }
    }

    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "repo": { "type": "string", "title": "Repository", "description": "Which repository?" },
                "count": { "type": "integer", "title": "Count" },
                "ratio": { "type": "number" },
                "private": { "type": "boolean", "title": "Private?" },
                "color": { "type": "string", "enum": ["r", "g"], "enumNames": ["Red", "Green"] },
                "size": { "type": "string", "oneOf": [ { "const": "s", "title": "Small" }, { "const": "l", "title": "Large" } ] },
                "tags": { "type": "array", "items": { "type": "string", "enum": ["a", "b", "c"] } },
                "note": { "type": "string", "default": "none" }
            },
            "required": ["repo", "count"]
        })
    }

    const ORDER: [&str; 8] = [
        "repo", "count", "ratio", "private", "color", "size", "tags", "note",
    ];

    fn order() -> Vec<String> {
        ORDER.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_flat_form_maps_to_one_question_per_field_plus_consent() {
        let (_, qs) = form_card("gh", "Fill this in", &schema(), Some(&order())).unwrap();
        assert_eq!(qs.len(), 9);
        assert_eq!(qs[0].header.as_deref(), Some("Repository"));
        assert_eq!(qs[0].question, "Which repository? (required)");
        assert!(qs[0].options.is_empty(), "free text");
        assert_eq!(
            qs[3]
                .options
                .iter()
                .map(|o| o.label.as_str())
                .collect::<Vec<_>>(),
            ["Yes", "No"]
        );
        assert_eq!(
            qs[4]
                .options
                .iter()
                .map(|o| o.label.as_str())
                .collect::<Vec<_>>(),
            ["Red", "Green"]
        );
        assert_eq!(
            qs[5]
                .options
                .iter()
                .map(|o| o.label.as_str())
                .collect::<Vec<_>>(),
            ["Small", "Large"]
        );
        assert!(qs[6].multi_select);
        let consent = &qs[8];
        assert!(consent.question.starts_with("Fill this in"));
        assert_eq!(consent.options[0].label, SUBMIT_LABEL);
        assert_eq!(consent.options[1].label, DECLINE_LABEL);
    }

    #[test]
    fn answers_are_typed_per_schema() {
        let (form, _) = form_card("gh", "", &schema(), Some(&order())).unwrap();
        let got = reply(
            &form,
            &answers(&["acme/web", "3", "0.5", "1", "Green", "s", "a, 3", "", "1"]),
        );
        assert_eq!(
            got,
            ElicitationReply::Accept(Some(json!({
                "repo": "acme/web", "count": 3, "ratio": 0.5, "private": true,
                "color": "g", "size": "s", "tags": ["a", "c"], "note": "none"
            })))
        );
        assert_eq!(
            got.result(),
            json!({ "action": "accept", "content": {
                "repo": "acme/web", "count": 3, "ratio": 0.5, "private": true,
                "color": "g", "size": "s", "tags": ["a", "c"], "note": "none"
            } })
        );
        // Optional fields left empty are omitted; consent unanswered submits.
        let got = reply(&form, &answers(&["x", "1"]));
        assert_eq!(
            got,
            ElicitationReply::Accept(Some(json!({ "repo": "x", "count": 1, "note": "none" })))
        );
    }

    #[test]
    fn decline_and_invalid_answers_never_send_content() {
        let (form, _) = form_card("gh", "", &schema(), Some(&order())).unwrap();
        let declined = reply(
            &form,
            &answers(&["x", "1", "", "", "", "", "", "", "Decline"]),
        );
        assert_eq!(declined, ElicitationReply::Decline);
        assert_eq!(
            declined.result(),
            json!({ "action": "decline", "content": null })
        );
        assert!(matches!(
            reply(&form, &answers(&["x", "many"])),
            ElicitationReply::Invalid(_)
        ));
        assert!(matches!(
            reply(&form, &answers(&["", "1"])),
            ElicitationReply::Invalid(_)
        ));
        assert!(matches!(
            reply(&form, &answers(&["x", "1", "", "", "Blue"])),
            ElicitationReply::Invalid(_)
        ));
        assert_eq!(
            reply(&form, &answers(&["x", "1"])).result()["action"],
            "accept"
        );
    }

    #[test]
    fn a_literal_text_answer_is_not_an_option_number() {
        let s = json!({ "type": "object", "properties": { "pick": { "type": "string", "enum": ["1", "2", "3"] } } });
        let (form, _) = form_card("s", "", &s, None).unwrap();
        let mut a = answers(&["3"]);
        assert_eq!(
            reply(&form, &a),
            ElicitationReply::Accept(Some(json!({ "pick": "3" })))
        );
        a.answers = Some(vec!["1".into()]);
        a.answer_kinds = Some(vec!["text".into()]);
        assert_eq!(
            reply(&form, &a),
            ElicitationReply::Accept(Some(json!({ "pick": "1" })))
        );
    }

    #[test]
    fn nested_schemas_are_unsupported() {
        for s in [
            json!({ "type": "object", "properties": { "o": { "type": "object", "properties": {} } } }),
            json!({ "type": "object", "properties": { "a": { "type": "array", "items": { "type": "object" } } } }),
            json!({ "type": "array" }),
        ] {
            assert!(form_card("s", "", &s, None).is_err(), "{s}");
        }
        // An empty form is a bare confirmation.
        let (form, qs) = form_card(
            "s",
            "Proceed?",
            &json!({ "type": "object", "properties": {} }),
            None,
        )
        .unwrap();
        assert_eq!(qs.len(), 1);
        assert_eq!(
            reply(
                &form,
                &ManagedAnswer {
                    option: Some(1),
                    ..Default::default()
                }
            ),
            ElicitationReply::Accept(Some(json!({})))
        );
    }

    #[test]
    fn url_mode_is_open_or_decline() {
        let (form, qs) = url_card("auth", "Sign in", "https://example.test/login");
        assert!(qs[0].question.contains("https://example.test/login"));
        assert_eq!(
            qs[0].options[0].description.as_deref(),
            Some("https://example.test/login")
        );
        assert_eq!(
            reply(
                &form,
                &ManagedAnswer {
                    option: Some(1),
                    ..Default::default()
                }
            ),
            ElicitationReply::Accept(None)
        );
        assert_eq!(
            reply(&form, &answers(&["Open"])).result(),
            json!({ "action": "accept", "content": null })
        );
        assert_eq!(reply(&form, &answers(&["2"])), ElicitationReply::Decline);
        assert_eq!(
            reply(&form, &ManagedAnswer::default()),
            ElicitationReply::Decline
        );
    }
}
