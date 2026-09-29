use super::document::{self, re, trim};
use anyhow::{Result, bail};
use serde_json::{Value, json};
pub fn session_ref(raw: &str) -> Result<String> {
    let text = trim(raw);
    let bare = if text
        .get(..8)
        .is_some_and(|s| s.eq_ignore_ascii_case("session:"))
    {
        &text[8..]
    } else {
        text
    };
    let bare = trim(bare).to_lowercase();
    if re(r"^(?:[0-9a-f]{6,}|[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})$")
        .is_match(&bare)
    {
        return Ok(bare.chars().take(8).collect());
    }
    bail!(
        "brief.append: {raw:?} is not a session id; use a full UUID or at least six hexadecimal characters"
    )
}
pub fn check(content: &str, rows: &[Value], path: &str) -> Value {
    let live: Vec<_> = rows
        .iter()
        .filter(|r| {
            r.is_object()
                && r["archived"] != true
                && !matches!(r["mode"].as_str(), Some("ended" | "stopped"))
                && !matches!(r["status"].as_str(), Some("ended" | "stopped"))
        })
        .filter_map(|r| r["sessionId"].as_str())
        .map(|s| trim(s).to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    let doc = document::parse(content);
    let entries: Vec<_> = doc
        .entries
        .iter()
        .filter(|e| trim(&e.column).eq_ignore_ascii_case("Now"))
        .collect();
    let mut findings = Vec::new();
    let mut entries_live = 0;
    let token = re(r"session:([A-Za-z0-9_.-]+)");
    for entry in &entries {
        let mut good = Vec::new();
        let mut bad = Vec::new();
        for m in token.captures_iter(&entry.text) {
            match session_ref(&m[1]) {
                Ok(s) => {
                    if !good.contains(&s) {
                        good.push(s)
                    }
                }
                Err(_) => {
                    if !bad.contains(&m[1].to_owned()) {
                        bad.push(m[1].to_owned())
                    }
                }
            }
        }
        let resolves = good
            .iter()
            .any(|r| live.iter().any(|id| id.starts_with(r) || r.starts_with(id)));
        if resolves {
            entries_live += 1;
        }
        let (reason, refs, detail) = if !bad.is_empty() {
            let detail = format!(
                "{} is not a session id, so it links to nothing. Fix the reference by hand (or re-append the line with brief_append's sessionId param, which validates it) — this check does not edit the brief.",
                bad.iter()
                    .map(|r| format!("session:{r}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            ("malformed", bad, detail)
        } else if !good.is_empty() && !resolves {
            let detail = format!(
                "{} is not a session this host still knows about, so this Now line has outlived its dispatch. If the work landed, move the entry to Recently (or archive it); if it is still yours, re-dispatch and write the new session id. Nothing was changed.",
                good.iter()
                    .map(|r| format!("session:{r}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            ("stale", good, detail)
        } else if good.is_empty() && re(r"(?i)\bdispatch(?:ed|ing)?\b").is_match(&entry.text) {
            ("unreferenced",Vec::new(),"this reads like a dispatch but names no session:<id>, so nothing can tell you whether its worker is still alive. Add the reference when you next touch the line.".into())
        } else {
            continue;
        };
        findings.push(json!({"line":entry.start,"text":entry.lines[0],"reason":reason,"refs":refs,"detail":detail}));
    }
    let stale = findings.iter().filter(|f| f["reason"] == "stale").count();
    let note = if findings.is_empty() {
        format!(
            "Every ## Now entry ({}) either names a session this host still knows about or is not a dispatch line. Nothing to prune.",
            entries.len()
        )
    } else {
        format!(
            "{} of {} ## Now entries need YOUR judgement{}. This check only reports: it never edits, moves or deletes a line, because the user's own brief edits are authoritative. Act on them with a board move or an explicit edit.",
            findings.len(),
            entries.len(),
            if stale > 0 {
                format!(" ({stale} name sessions that are gone)")
            } else {
                String::new()
            }
        )
    };
    json!({"path":path,"section":"Now","entriesChecked":entries.len(),"entriesLive":entries_live,"findings":findings,"liveSessions":live.len(),"note":note})
}
fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => trim(&re(r"@s+").replace_all(s, " ")).into(),
        _ => v.to_string(),
    }
}
fn cut(s: String, uncapped: bool) -> String {
    let n = s.encode_utf16().count();
    if uncapped || n <= 200 {
        s
    } else {
        format!(
            "{}… ({n} chars)",
            String::from_utf16_lossy(&s.encode_utf16().take(200).collect::<Vec<_>>())
        )
    }
}
fn fact(key: &str, v: &Value) -> Option<String> {
    if v.is_null() {
        return None;
    }
    let uncapped = matches!(key.to_lowercase().as_str(), "caveat" | "caveats");
    let body = if let Some(values) = v.as_array() {
        let values: Vec<_> = values
            .iter()
            .map(scalar)
            .filter(|s| !s.is_empty())
            .collect();
        if values.is_empty() {
            return None;
        }
        let n = if uncapped {
            values.len()
        } else {
            values.len().min(3)
        };
        let mut s = values[..n].join(", ");
        if n < values.len() {
            s.push_str(&format!(", +{} more", values.len() - n));
        }
        cut(s, uncapped)
    } else {
        let text = scalar(v);
        if text.is_empty() {
            return None;
        }
        if key.eq_ignore_ascii_case("commit") && re(r"(?i)^[0-9a-f]{40}$").is_match(&text) {
            text[..12].into()
        } else {
            cut(text, uncapped)
        }
    };
    Some(format!("{key}: {body}"))
}
pub fn compose(line: &str, params: &Value, today: &str) -> Result<String> {
    let line = document::flatten(line);
    if line.is_empty() {
        bail!(
            "brief.append: line must contain your own significance sentence; a result alone cannot become a brief entry"
        );
    }
    let reference = if let Some(raw) = params.get("sessionId") {
        session_ref(raw.as_str().unwrap_or(""))?
    } else {
        String::new()
    };
    let mut facts = Vec::new();
    if let Some(result) = params.get("result") {
        if let Some(map) = result.as_object() {
            let ordered = [
                "commit",
                "filesChanged",
                "checksRun",
                "caveats",
                "followUps",
            ];
            for key in ordered {
                if let Some(v) = map.get(key) {
                    if let Some(f) = fact(key, v) {
                        facts.push(f);
                    }
                }
            }
            for (key, value) in map {
                if !ordered.contains(&key.as_str()) {
                    if let Some(f) = fact(key, value) {
                        facts.push(f);
                    }
                }
            }
        } else if let Some(f) = fact("result", result) {
            facts.push(f);
        }
    }
    let mut output = if re(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}\b").is_match(&line) {
        line
    } else {
        format!("{today}  {line}")
    };
    if !facts.is_empty() {
        output.push_str(" — ");
        output.push_str(&facts.join("; "));
    }
    if !reference.is_empty()
        && !output
            .to_lowercase()
            .contains(&format!("session:{reference}"))
    {
        output.push_str(&format!(" (session:{reference})"));
    }
    Ok(output)
}
