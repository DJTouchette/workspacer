use super::{remove, text};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
fn fields<'a>(v: &'a Value, allowed: &[&str]) -> Result<&'a serde_json::Map<String, Value>> {
    let m = v.as_object().context("Invalid task reference fields")?;
    if m.keys().any(|k| !allowed.contains(&k.as_str())) {
        bail!("Invalid task reference fields");
    }
    Ok(m)
}
fn label(v: &Value) -> Result<String> {
    let s = v.as_str().context("Reference label must be text")?;
    if s.trim().is_empty() || s.encode_utf16().count() > 200 {
        bail!("Reference labels must be 1-200 characters");
    }
    Ok(s.trim().into())
}
pub fn url(v: &Value) -> Result<String> {
    let s = v.as_str().context("Reference URL must be text")?;
    if s != s.trim() || s.encode_utf16().count() > 2048 {
        bail!("Enter an http(s) URL of at most 2048 characters");
    }
    let url = url::Url::parse(s)?;
    if !["http", "https"].contains(&url.scheme())
        || !url.username().is_empty()
        || url.password().is_some()
    {
        bail!("Only http(s) URLs without credentials are supported");
    }
    Ok(url.to_string())
}
pub fn validate(value: &Value) -> Result<Value> {
    let input = fields(value, &["pullRequest", "tickets", "references"])?;
    let mut links = json!({});
    let mut urls = BTreeSet::new();
    let mut unique_url = |v: &Value| -> Result<String> {
        let u = url(v)?;
        if !urls.insert(u.clone()) {
            bail!("Duplicate reference URL");
        }
        Ok(u)
    };
    if let Some(pr) = input.get("pullRequest") {
        let pr = fields(pr, &["number", "url"])?;
        if pr.is_empty() {
            bail!("Enter a PR number or URL");
        }
        let mut out = json!({});
        if let Some(n) = pr.get("number") {
            let n = n.as_str().context("PR number must be a string")?;
            if n.is_empty()
                || n.len() > 10
                || n.starts_with('0')
                || !n.bytes().all(|b| b.is_ascii_digit())
            {
                bail!("PR number must be positive with at most10 digits");
            }
            out["number"] = n.into();
        }
        if let Some(u) = pr.get("url") {
            out["url"] = unique_url(u)?.into();
        }
        links["pullRequest"] = out;
    }
    for key in ["tickets", "references"] {
        if let Some(rows) = input.get(key) {
            let rows = rows.as_array().context("References must be array")?;
            if rows.len() > 20 {
                bail!("Use at most20 tickets or references");
            }
            let name = if key == "tickets" { "id" } else { "label" };
            let mut labels = BTreeSet::new();
            let mut output = Vec::new();
            for row in rows {
                let row = fields(row, &[name, "url"])?;
                let n = label(row.get(name).unwrap_or(&Value::Null))?;
                if !labels.insert(n.to_lowercase()) {
                    bail!("Duplicate reference label");
                }
                let mut out = json!({});
                out[name] = n.into();
                if let Some(u) = row.get("url") {
                    out["url"] = unique_url(u)?.into();
                } else if key == "references" {
                    bail!("Reference URL required");
                }
                output.push(out);
            }
            links[key] = output.into();
        }
    }
    Ok(links)
}
fn rows(value: Option<&Value>) -> Result<Vec<Value>> {
    match value {
        None => Ok(vec![]),
        Some(v) => {
            let a = v.as_array().context("Reference edits must be arrays")?;
            if a.len() > 20 {
                bail!("Use at most20 reference edits");
            }
            Ok(a.clone())
        }
    }
}
fn kind(row: &Value, removing: bool) -> Result<&str> {
    let k = text(&row["kind"]);
    let allowed: &[&str] = match (k, removing) {
        ("pullRequest", true) => &["kind"],
        ("pullRequest", false) => &["kind", "number", "url"],
        ("ticket", true) => &["kind", "id"],
        ("ticket", false) => &["kind", "id", "url"],
        ("reference", true) => &["kind", "label"],
        ("reference", false) => &["kind", "label", "url"],
        _ => bail!("Unknown reference kind"),
    };
    fields(row, allowed)?;
    Ok(k)
}
pub fn apply(
    current: Option<&Value>,
    upsert: Option<&Value>,
    remove_rows: Option<&Value>,
) -> Result<Value> {
    let upserts = rows(upsert)?;
    let removals = rows(remove_rows)?;
    if upserts.is_empty() && removals.is_empty() {
        bail!("Supply at least one reference to upsert or remove");
    }
    let mut links = validate(current.unwrap_or(&json!({})))?;
    for row in removals {
        let k = kind(&row, true)?;
        if k == "pullRequest" {
            remove(&mut links, "pullRequest");
            continue;
        }
        let (key, name) = if k == "ticket" {
            ("tickets", "id")
        } else {
            ("references", "label")
        };
        let value = row[name]
            .as_str()
            .context("Removing reference requires its name")?
            .trim()
            .to_lowercase();
        if let Some(a) = links[key].as_array_mut() {
            a.retain(|r| text(&r[name]).trim().to_lowercase() != value);
            if a.is_empty() {
                remove(&mut links, key);
            }
        }
    }
    for row in upserts {
        let k = kind(&row, false)?;
        if k == "pullRequest" {
            if row.get("number").is_none() && row.get("url").is_none() {
                bail!("A pull request reference needs a number or URL");
            }
            if !links["pullRequest"].is_object() {
                links["pullRequest"] = json!({});
            }
            for field in ["number", "url"] {
                if let Some(v) = row.get(field) {
                    links["pullRequest"][field] = v.clone();
                }
            }
            continue;
        }
        let (key, name) = if k == "ticket" {
            ("tickets", "id")
        } else {
            ("references", "label")
        };
        let value = label(&row[name])?;
        if !links[key].is_array() {
            links[key] = json!([]);
        }
        let entries = links[key].as_array_mut().unwrap();
        let at = entries.iter().position(|r| {
            text(&r[name]).trim().eq_ignore_ascii_case(value.trim())
                || text(&r[name]).trim().to_lowercase() == value.to_lowercase()
        });
        let mut next = json!({});
        next[name] = value.into();
        if let Some(v) = row.get("url") {
            next["url"] = v.clone();
        } else if k == "reference" {
            bail!("Named reference needs a URL");
        } else if let Some(at) = at {
            if let Some(v) = entries[at].get("url") {
                next["url"] = v.clone();
            }
        }
        if let Some(at) = at {
            entries[at] = next;
        } else {
            entries.push(next);
        }
    }
    validate(&links)
}
fn raw_urls(content: &str) -> regex::Regex {
    let _ = content;
    regex::Regex::new(r#"(?i)https?://[^\s<>"`]+"#).unwrap()
}
fn request_urls(content: &str) -> Vec<String> {
    let mut result = Vec::new();
    for m in raw_urls(content).find_iter(content) {
        let candidate = m
            .as_str()
            .trim_end_matches(&['.', ',', ';', ':', '!', '?', ']', ')', '}'][..]);
        if let Ok(u) = url(&json!(candidate)) {
            if !result.contains(&u) {
                result.push(u);
            }
        }
    }
    result
}
fn inferred(u: &str) -> Option<Value> {
    let parsed = url::Url::parse(u).ok()?;
    let host = parsed.host_str()?;
    if !["github.com", "gitlab.com", "dev.azure.com"].contains(&host)
        && !host.ends_with(".visualstudio.com")
        && !parsed.path().contains("/-/merge_requests/")
    {
        return None;
    }
    let regex = regex::Regex::new(
        r"(?i)(?:/pull/|/merge_requests/|/_git/[^/]+/pullrequest/)([1-9][0-9]{0,9})/?$",
    )
    .unwrap();
    let captures = regex.captures(parsed.path())?;
    Some(json!({"kind":"pullRequest","number":&captures[1],"url":u}))
}
fn contains_identifier(content: &str, id: &str) -> bool {
    let pattern = format!(
        r"(?:^|[^\p{{L}}\p{{N}}\p{{M}}\p{{Pc}}\p{{Pd}}\u{{200c}}\u{{200d}}]){}(?:$|[^\p{{L}}\p{{N}}\p{{M}}\p{{Pc}}\p{{Pd}}\u{{200c}}\u{{200d}}])",
        regex::escape(id.trim())
    );
    regex::Regex::new(&pattern).unwrap().is_match(content)
}
pub fn map_request(content: &str, intents: &[Value]) -> Result<BTreeMap<String, Vec<Value>>> {
    let urls = request_urls(content);
    let work: Vec<_> = intents
        .iter()
        .filter(|i| matches!(text(&i["kind"]), "create" | "followUp" | "update"))
        .collect();
    let mut result = BTreeMap::new();
    if work.is_empty() {
        return Ok(result);
    }
    let mut numbers: BTreeSet<String> = urls
        .iter()
        .filter_map(|u| inferred(u).map(|r| text(&r["number"]).into()))
        .collect();
    let prose = raw_urls(content).replace_all(content, " ");
    let regex=regex::Regex::new(r"(?i)(?:PR|MR|pull\s+request|merge\s+request)\s*(?:[#!]|(?:number|no\.?)\s+)?\s*([1-9][0-9]{0,9})").unwrap();
    for caps in regex.captures_iter(&prose) {
        let matched = caps.get(0).unwrap();
        if contains_identifier(&prose, matched.as_str()) {
            numbers.insert(caps[1].into());
        }
    }
    let explicit = work.iter().any(|i| i.get("references").is_some());
    if !urls.is_empty() && !explicit {
        if work.len() == 1 && urls.len() == 1 {
            if let Some(reference) = inferred(&urls[0]) {
                result.insert(text(&work[0]["key"]).into(), vec![reference]);
                return Ok(result);
            }
        }
        bail!(
            "Reference mapping required on every work intent; empty array leaves URLs unassigned"
        );
    }
    for intent in work {
        if explicit && intent.get("references").is_none() {
            bail!("Reference mapping required on every work intent");
        }
        let refs = rows(intent.get("references"))?;
        let mut output = Vec::new();
        let mut pr = None;
        for r in refs {
            let validated = apply(None, Some(&json!([r])), None)?;
            if r.get("url").is_some() && !urls.contains(&url(&r["url"])?) {
                bail!("Mapped reference URL must occur in original submitted request");
            }
            match text(&r["kind"]) {
                "pullRequest" => {
                    let parsed = if r.get("url").is_some() {
                        inferred(&url(&r["url"])?)
                    } else {
                        None
                    };
                    if r.get("url").is_some() && parsed.is_none() {
                        bail!("PR URL must identify supported original pull/merge request");
                    }
                    if let Some(n) = r.get("number") {
                        if !numbers.contains(text(n))
                            || parsed.as_ref().is_some_and(|p| p["number"] != *n)
                        {
                            bail!("PR number must match original PR identifier and URL");
                        }
                    }
                    let identity = validated["pullRequest"].clone();
                    if let Some(pr) = &pr {
                        if pr != &identity {
                            bail!("Map at most one distinct PR per task");
                        }
                        continue;
                    }
                    pr = Some(identity);
                }
                "ticket" if !contains_identifier(content, text(&r["id"])) => {
                    bail!("Ticket ID must occur as a whole identifier in original request")
                }
                _ => {}
            }
            output.push(r);
        }
        if !output.is_empty() {
            apply(None, Some(&json!(output)), None)?;
        }
        result.insert(text(&intent["key"]).into(), output);
    }
    Ok(result)
}
pub fn attach(current: Option<&Value>, refs: &[Value], replace: bool) -> Result<Option<Value>> {
    let mut links = current.cloned();
    for r in refs {
        let u = r.get("url").map(url).transpose()?;
        if let (Some(u), Some(existing)) = (&u, &links) {
            let duplicated = existing["pullRequest"]["url"] == *u
                || ["tickets", "references"].iter().any(|key| {
                    existing[*key]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|row| row["url"] == *u)
                });
            if duplicated {
                continue;
            }
        }
        let mut removals = None;
        if r["kind"] == "pullRequest"
            && links
                .as_ref()
                .is_some_and(|l| l.get("pullRequest").is_some())
        {
            if !replace {
                bail!("Task already has a different PR; explicit replacePullRequest required");
            }
            removals = Some(json!([{"kind":"pullRequest"}]));
        }
        if let Some(existing) = &links {
            for (kind, key, name) in [
                ("reference", "references", "label"),
                ("ticket", "tickets", "id"),
            ] {
                if r["kind"] == kind
                    && existing[key].as_array().into_iter().flatten().any(|e| {
                        text(&e[name]).to_lowercase() == text(&r[name]).trim().to_lowercase()
                            && (kind == "reference"
                                || e.get("url").is_some() && e["url"] != r["url"])
                    })
                {
                    bail!("Reference name already belongs to another URL");
                }
            }
        }
        links = Some(apply(links.as_ref(), Some(&json!([r])), removals.as_ref())?);
    }
    Ok(links)
}
