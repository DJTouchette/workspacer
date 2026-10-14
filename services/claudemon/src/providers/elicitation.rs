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
//!   MCP form sends the user's data to a third party: ONLY an explicit
//!   `Submit` sends it, and anything else declines. Schema limits (`minimum`,
//!   `maximum`, `minLength`, `maxLength`, simple `format`s, `minItems`,
//!   `maxItems`) are checked before anything is sent.
//! * **url mode** — a single question whose text carries the URL, with
//!   options `Open` (its description is the URL itself, for a client that
//!   opens it) and `Decline`. Only `http(s)` URLs are offered.
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

/// The consent question's options. Only an explicit `Submit` (option 1, or
/// the label in any case) sends the form; everything else declines.
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
    limits: Limits,
}

/// The schema constraints checked before an answer is sent: `minimum` /
/// `maximum` (numbers), `minLength` / `maxLength` / `format` (strings),
/// `minItems` / `maxItems` (multi-selects).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Limits {
    min: Option<f64>,
    max: Option<f64>,
    min_len: Option<u64>,
    max_len: Option<u64>,
    format: Option<String>,
    min_items: Option<u64>,
    max_items: Option<u64>,
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
    let num = |k: &str| schema.get(k).and_then(Value::as_f64);
    let count = |k: &str| schema.get(k).and_then(Value::as_u64);
    Ok(Field {
        key: key.to_owned(),
        kind,
        required,
        default: schema.get("default").cloned().filter(|d| !d.is_null()),
        labels,
        limits: Limits {
            min: num("minimum"),
            max: num("maximum"),
            min_len: count("minLength"),
            max_len: count("maxLength"),
            format: schema
                .get("format")
                .and_then(Value::as_str)
                .map(str::to_owned),
            min_items: count("minItems"),
            max_items: count("maxItems"),
        },
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
            elicitation: Some(server.to_owned()),
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
        elicitation: Some(server.to_owned()),
    });
    Ok((ElicitationForm::Form { fields }, questions))
}

/// Whether `url` is an absolute `http`/`https` URL with a host — the only
/// kind a url-mode card offers to open.
fn web_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"));
    rest.is_some_and(|r| {
        let host = r.split(['/', '?', '#']).next().unwrap_or("");
        !host.is_empty() && !host.starts_with('@') && !r.contains(char::is_whitespace)
    })
}

/// Map a url-mode elicitation onto a single Open / Decline question. The
/// `Open` option's description is the URL, so a client can open it. Only an
/// `http(s)` URL is offered; anything else (`file:`, `javascript:`, a bare
/// path, nothing) is unsupported.
pub(crate) fn url_card(
    server: &str,
    message: &str,
    url: &str,
) -> Result<(ElicitationForm, Vec<PendingQuestion>), FormError> {
    if !web_url(url) {
        return Err(FormError::Unsupported("a url that is not http(s)".into()));
    }
    let url = url.trim();
    let ask = message.trim();
    let question = if ask.is_empty() {
        format!("The MCP server `{server}` asks you to open:\n{url}")
    } else {
        format!("{ask}\n\nThe MCP server `{server}` asks you to open:\n{url}")
    };
    Ok((
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
            elicitation: Some(server.to_owned()),
        }],
    ))
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

/// A trivially checkable `format`: `email` (`a@b`), `uri` (`scheme:…`),
/// `date` (`YYYY-MM-DD`), `date-time` (a date, then `T`). Unknown formats
/// pass — the server validates what we cannot.
fn format_ok(format: &str, v: &str) -> bool {
    let date = |d: &str| {
        let b = d.as_bytes();
        b.len() == 10
            && b[4] == b'-'
            && b[7] == b'-'
            && b.iter()
                .enumerate()
                .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
    };
    match format {
        "email" => {
            v.split_once('@')
                .is_some_and(|(a, b)| !a.is_empty() && b.contains('.') && !b.starts_with('.'))
                && !v.contains(char::is_whitespace)
        }
        "uri" => {
            v.split_once(':').is_some_and(|(scheme, rest)| {
                !rest.is_empty()
                    && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                    && scheme
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
            }) && !v.contains(char::is_whitespace)
        }
        "date" => date(v),
        "date-time" => {
            v.len() > 11 && date(&v[..10]) && v.as_bytes()[10].eq_ignore_ascii_case(&b'T')
        }
        _ => true,
    }
}

fn field_value(field: &Field, raw: &str, literal: bool) -> Result<Value, String> {
    let bad = |what: &str| format!("`{}` needs {what}, got “{raw}”", field.key);
    let l = &field.limits;
    let in_range = |n: f64| {
        if l.min.is_some_and(|m| n < m) || l.max.is_some_and(|m| n > m) {
            let range = match (l.min, l.max) {
                (Some(a), Some(b)) => format!("a number from {a} to {b}"),
                (Some(a), None) => format!("a number of at least {a}"),
                (None, Some(b)) => format!("a number of at most {b}"),
                (None, None) => unreachable!(),
            };
            Err(bad(&range))
        } else {
            Ok(())
        }
    };
    match &field.kind {
        FieldKind::Text => {
            let len = raw.chars().count() as u64;
            if l.min_len.is_some_and(|m| len < m) || l.max_len.is_some_and(|m| len > m) {
                return Err(bad(&format!(
                    "{}–{} characters",
                    l.min_len.unwrap_or(0),
                    l.max_len
                        .map(|m| m.to_string())
                        .unwrap_or_else(|| "any".into())
                )));
            }
            if let Some(format) = &l.format {
                if !format_ok(format, raw) {
                    return Err(bad(&format!("a valid {format}")));
                }
            }
            Ok(Value::String(raw.to_owned()))
        }
        FieldKind::Number { integer: true } => {
            let n = raw.parse::<i64>().map_err(|_| bad("a whole number"))?;
            in_range(n as f64)?;
            Ok(Value::from(n))
        }
        FieldKind::Number { integer: false } => {
            let n = raw
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite())
                .ok_or_else(|| bad("a number"))?;
            in_range(n)?;
            serde_json::Number::from_f64(n)
                .map(Value::Number)
                .ok_or_else(|| bad("a number"))
        }
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
        FieldKind::MultiEnum { values } => {
            let picked = raw
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|entry| {
                    pick(entry, literal, &field.labels, Some(values))
                        .map(|i| Value::String(values[i].clone()))
                        .ok_or_else(|| bad(&format!("some of {}", field.labels.join(", "))))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let n = picked.len() as u64;
            if l.min_items.is_some_and(|m| n < m) || l.max_items.is_some_and(|m| n > m) {
                return Err(bad(&format!(
                    "{}–{} choices",
                    l.min_items.unwrap_or(0),
                    l.max_items
                        .map(|m| m.to_string())
                        .unwrap_or_else(|| "any".into())
                )));
            }
            Ok(Value::Array(picked))
        }
    }
}

/// Whether a raw multi-select answer picks nothing (`""`, `" , ,"`): it is
/// then an EMPTY answer — required fields fail, optional ones are omitted.
fn picks_nothing(field: &Field, raw: &str) -> bool {
    matches!(field.kind, FieldKind::MultiEnum { .. }) && raw.split(',').all(|s| s.trim().is_empty())
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
            // The consent question follows the fields. ONLY an explicit
            // Submit — its option number or its label (any case) — sends the
            // user's data; anything else (unanswered, Decline, free text, an
            // out-of-range number) declines.
            let consent = [SUBMIT_LABEL.to_owned(), DECLINE_LABEL.to_owned()];
            let submitted = match raw_answer(answer, fields.len()) {
                (Some(raw), literal) => pick(&raw, literal, &consent, None) == Some(0),
                (None, _) => false,
            };
            if !submitted {
                return ElicitationReply::Decline;
            }
            let mut content = Map::new();
            for (i, field) in fields.iter().enumerate() {
                let raw = match raw_answer(answer, i) {
                    (Some(raw), _) if picks_nothing(field, &raw) => (None, false),
                    other => other,
                };
                match raw {
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

    fn option(n: u8) -> ManagedAnswer {
        ManagedAnswer {
            option: Some(n),
            ..Default::default()
        }
    }

    fn labels(q: &PendingQuestion) -> Vec<&str> {
        q.options.iter().map(|o| o.label.as_str()).collect()
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

    fn form() -> ElicitationForm {
        form_card("gh", "", &schema(), Some(&order())).unwrap().0
    }

    /// Eight field answers, then the consent answer.
    fn with_consent(fields: &[&str], consent: &str) -> ManagedAnswer {
        let mut list: Vec<&str> = fields.to_vec();
        list.resize(8, "");
        list.push(consent);
        answers(&list)
    }

    #[test]
    fn a_flat_form_maps_to_one_question_per_field_plus_consent() {
        let (_, qs) = form_card("gh", "Fill this in", &schema(), Some(&order())).unwrap();
        assert_eq!(qs.len(), 9);
        assert_eq!(qs[0].header.as_deref(), Some("Repository"));
        assert_eq!(qs[0].question, "Which repository? (required)");
        assert!(qs[0].options.is_empty(), "free text");
        assert_eq!(labels(&qs[3]), ["Yes", "No"]);
        assert_eq!(labels(&qs[4]), ["Red", "Green"]);
        assert_eq!(labels(&qs[5]), ["Small", "Large"]);
        assert!(qs[6].multi_select);
        let consent = &qs[8];
        assert!(consent.question.starts_with("Fill this in"));
        assert_eq!(labels(consent), [SUBMIT_LABEL, DECLINE_LABEL]);
    }

    #[test]
    fn answers_are_typed_per_schema() {
        let got = reply(
            &form(),
            &answers(&["acme/web", "3", "0.5", "1", "Green", "s", "a, 3", "", "1"]),
        );
        let content = json!({
            "repo": "acme/web", "count": 3, "ratio": 0.5, "private": true,
            "color": "g", "size": "s", "tags": ["a", "c"], "note": "none"
        });
        assert_eq!(got, ElicitationReply::Accept(Some(content.clone())));
        assert_eq!(
            got.result(),
            json!({ "action": "accept", "content": content })
        );
        // Optional fields left empty are omitted (or take their default).
        assert_eq!(
            reply(&form(), &with_consent(&["x", "1"], "submit")),
            ElicitationReply::Accept(Some(json!({ "repo": "x", "count": 1, "note": "none" })))
        );
    }

    #[test]
    fn only_an_explicit_submit_sends_the_form() {
        let fields = ["x", "1"];
        for consent in ["1", "Submit", "SUBMIT", " submit "] {
            assert!(
                matches!(
                    reply(&form(), &with_consent(&fields, consent)),
                    ElicitationReply::Accept(_)
                ),
                "{consent:?} submits"
            );
        }
        for consent in [
            "", "2", "3", "0", "-1", "Decline", "yes", "ok", "send it", "Submit!",
        ] {
            assert_eq!(
                reply(&form(), &with_consent(&fields, consent)),
                ElicitationReply::Decline,
                "{consent:?} must decline"
            );
        }
        // Unanswered consent (the answers stop at the fields) declines.
        assert_eq!(
            reply(&form(), &answers(&["x", "1"])),
            ElicitationReply::Decline
        );
        // A literal-text "1" is not option 1.
        let mut literal = with_consent(&fields, "1");
        let mut kinds = vec!["text".to_string(); 9];
        kinds[8] = "text".into();
        literal.answer_kinds = Some(kinds);
        assert_eq!(reply(&form(), &literal), ElicitationReply::Decline);
        // An empty form: only option 1 / "Submit" accepts; free text declines.
        let (empty, qs) = form_card(
            "s",
            "Proceed?",
            &json!({ "type": "object", "properties": {} }),
            None,
        )
        .unwrap();
        assert_eq!(qs.len(), 1);
        assert_eq!(
            reply(&empty, &option(1)),
            ElicitationReply::Accept(Some(json!({})))
        );
        for bad in [
            option(2),
            option(9),
            answers(&["sure"]),
            ManagedAnswer::default(),
        ] {
            assert_eq!(reply(&empty, &bad), ElicitationReply::Decline, "{bad:?}");
        }
    }

    #[test]
    fn invalid_answers_never_send_content() {
        assert_eq!(
            reply(&form(), &with_consent(&["x", "1"], "Decline")).result(),
            json!({ "action": "decline", "content": null })
        );
        for bad in [
            with_consent(&["x", "many"], "1"),
            with_consent(&["", "1"], "1"),
            with_consent(&["x", "1", "", "", "Blue"], "1"),
            with_consent(&["x", "1", "abc"], "1"),
        ] {
            assert!(
                matches!(reply(&form(), &bad), ElicitationReply::Invalid(_)),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn schema_limits_and_formats_are_enforced() {
        let s = json!({
            "type": "object",
            "properties": {
                "n": { "type": "integer", "minimum": 1, "maximum": 5 },
                "r": { "type": "number", "minimum": 0.5 },
                "name": { "type": "string", "minLength": 2, "maxLength": 4 },
                "mail": { "type": "string", "format": "email" },
                "site": { "type": "string", "format": "uri" },
                "day": { "type": "string", "format": "date" },
                "at": { "type": "string", "format": "date-time" },
                "tags": { "type": "array", "minItems": 1, "maxItems": 2,
                          "items": { "type": "string", "enum": ["a", "b", "c"] } }
            },
            "required": ["tags"]
        });
        let order: Vec<String> = ["n", "r", "name", "mail", "site", "day", "at", "tags"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (form, _) = form_card("s", "", &s, Some(&order)).unwrap();
        let ok = [
            "3",
            "0.5",
            "abc",
            "a@b.io",
            "https://x.test",
            "2026-10-13",
            "2026-10-13T10:00:00Z",
            "a,b",
            "1",
        ];
        assert!(matches!(
            reply(&form, &answers(&ok)),
            ElicitationReply::Accept(_)
        ));
        for (i, bad) in [
            (0, "9"),
            (0, "0"),
            (1, "0.1"),
            (1, "NaN"),
            (2, "a"),
            (2, "abcde"),
            (3, "nobody"),
            (4, "not a uri"),
            (5, "13/10/2026"),
            (6, "2026-10-13"),
            (7, "a,b,c"),
        ] {
            let mut list = ok;
            list[i] = bad;
            assert!(
                matches!(reply(&form, &answers(&list)), ElicitationReply::Invalid(_)),
                "field {i} = {bad:?} must be rejected"
            );
        }
        // A comma-only multi-select picks nothing: required → invalid.
        let mut list = ok;
        list[7] = " , ,";
        assert!(matches!(
            reply(&form, &answers(&list)),
            ElicitationReply::Invalid(_)
        ));
    }

    #[test]
    fn a_literal_text_answer_is_not_an_option_number() {
        let s = json!({ "type": "object", "properties": { "pick": { "type": "string", "enum": ["1", "2", "3"] } } });
        let (form, _) = form_card("s", "", &s, None).unwrap();
        let mut a = answers(&["3", "1"]);
        assert_eq!(
            reply(&form, &a),
            ElicitationReply::Accept(Some(json!({ "pick": "3" })))
        );
        a.answers = Some(vec!["1".into(), "1".into()]);
        a.answer_kinds = Some(vec!["text".into(), "option".into()]);
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
    }

    #[test]
    fn url_mode_is_open_or_decline_and_only_http() {
        let (form, qs) = url_card("auth", "Sign in", "https://example.test/login").unwrap();
        assert!(qs[0].question.contains("https://example.test/login"));
        assert_eq!(
            qs[0].options[0].description.as_deref(),
            Some("https://example.test/login")
        );
        assert_eq!(reply(&form, &option(1)), ElicitationReply::Accept(None));
        assert_eq!(
            reply(&form, &answers(&["open"])).result(),
            json!({ "action": "accept", "content": null })
        );
        for bad in [
            option(2),
            option(3),
            answers(&["sure"]),
            ManagedAnswer::default(),
        ] {
            assert_eq!(reply(&form, &bad), ElicitationReply::Decline, "{bad:?}");
        }
        assert!(url_card("a", "", "http://localhost:8080/cb").is_ok());
        for url in [
            "",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "/relative",
            "https://",
            "https://@evil",
            "ftp://x.test",
            "https://x.test/a b",
        ] {
            assert!(url_card("a", "", url).is_err(), "{url:?}");
        }
    }

    #[test]
    fn elicitation_cards_carry_a_mark_input_can_never_forge() {
        let schema = json!({"type":"object","properties":{"repo":{"type":"string"}}});
        let (_, form) = form_card("github", "Pick", &schema, None).unwrap();
        assert!(form
            .iter()
            .all(|q| q.elicitation.as_deref() == Some("github")));
        let (_, url) = url_card("auth", "", "https://example.com/x").unwrap();
        let wire = serde_json::to_value(&url).unwrap();
        assert_eq!(wire[0]["elicitation"], "auth");
        // An agent's AskUserQuestion input naming the mark is read without it.
        let forged: Vec<PendingQuestion> = serde_json::from_value(json!([{
            "question":"Open?","header":"github link","elicitation":"github",
            "options":[{"label":"Open","description":"https://evil.example"},{"label":"Decline"}]
        }]))
        .unwrap();
        assert!(forged[0].elicitation.is_none());
        assert!(serde_json::to_value(&forged).unwrap()[0]
            .get("elicitation")
            .is_none());
    }
}
