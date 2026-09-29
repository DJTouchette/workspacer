//! Dispatch template placeholders, preserving the desktop's ECMAScript trim.
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::OnceLock};
const JS_SPACE: &str = "\t\n\u{b}\u{c}\r \u{a0}\u{1680}\u{2000}\u{2001}\u{2002}\u{2003}\u{2004}\u{2005}\u{2006}\u{2007}\u{2008}\u{2009}\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}\u{feff}";
pub(crate) fn trim_js(value: &str) -> &str {
    value.trim_matches(|c| JS_SPACE.contains(c))
}
fn tokens() -> &'static regex::Regex {
    static TOKENS: OnceLock<regex::Regex> = OnceLock::new();
    TOKENS.get_or_init(|| regex::Regex::new(r"\{\{([^}]+?)\}\}").unwrap())
}
fn parse(raw: &str) -> (&str, Option<&str>) {
    let raw = trim_js(raw);
    let raw = raw.strip_prefix('?').unwrap_or(raw);
    match raw.split_once(':') {
        Some((name, default)) => (trim_js(name), Some(trim_js(default))),
        None => (trim_js(raw), None),
    }
}
fn auto(name: &str) -> bool {
    matches!(name, "cwd" | "projectCwd")
}
pub fn parameters(text: &str) -> Vec<Value> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for token in tokens().captures_iter(text) {
        let (name, default) = parse(&token[1]);
        if name.is_empty() || auto(name) || !seen.insert(name.to_owned()) {
            continue;
        }
        let mut parameter = json!({"name":name,"required":default.is_none()});
        if let Some(default) = default {
            parameter["default"] = json!(default);
        }
        out.push(parameter);
    }
    out
}
pub fn render(text: &str, params: &Value, cwd: &str, project_cwd: &str) -> Result<String> {
    let empty = serde_json::Map::new();
    let params = if params.is_null() {
        &empty
    } else {
        params
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("templateParams must be an object"))?
    };
    let declared: BTreeSet<String> = tokens()
        .captures_iter(text)
        .map(|t| parse(&t[1]).0.to_owned())
        .collect();
    for (name, value) in params {
        if auto(name) {
            bail!(
                "dispatch template: host-owned automatic variable {name:?} cannot be set through templateParams"
            );
        }
        if !declared.contains(name) {
            bail!("dispatch template: unknown templateParams {name:?}");
        }
        if !value.is_string() {
            bail!("templateParams {name:?} must be a string");
        }
    }
    let mut result = String::new();
    let mut last = 0;
    for token in tokens().captures_iter(text) {
        let matched = token.get(0).unwrap();
        let (name, default) = parse(&token[1]);
        result.push_str(&text[last..matched.start()]);
        let value = if name == "cwd" {
            cwd
        } else if name == "projectCwd" {
            project_cwd
        } else if let Some(value) = params.get(name).and_then(Value::as_str).filter(|s| {
            !s.trim_matches([' ', '\t', '\n', '\r', '\u{b}', '\u{c}'])
                .is_empty()
        }) {
            value
        } else if let Some(default) = default {
            default
        } else {
            bail!("dispatch template: required placeholder {{{{{name}}}}} is unfilled");
        };
        result.push_str(value);
        last = matched.end();
    }
    result.push_str(&text[last..]);
    Ok(result)
}
