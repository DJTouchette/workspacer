use super::document::{self, Entry, re, trim};
use serde_json::{Value, json};
fn len(s: &str) -> usize {
    s.encode_utf16().count()
}
fn prefix(s: &str, n: usize) -> String {
    String::from_utf16_lossy(&s.encode_utf16().take(n).collect::<Vec<_>>())
}
pub fn plain(s: &str) -> String {
    let bold = re(r"\*\*(.+?)\*\*").replace_all(s, "$1");
    let code = re(r"`+([^`]+)`+").replace_all(&bold, |m: &regex::Captures| {
        let matched = m.get(0).unwrap().as_str();
        if matched.bytes().take_while(|b| *b == b'`').count() == 1
            && matched.bytes().rev().take_while(|b| *b == b'`').count() == 1
        {
            m[1].to_owned()
        } else {
            matched.to_owned()
        }
    });
    let emphasis = re(r"\*(.+?)\*").replace_all(&code, "$1");
    trim(&re(r"@s+").replace_all(&emphasis, " ")).to_owned()
}
fn truncate(s: &str, n: usize) -> String {
    if len(s) <= n {
        return s.into();
    }
    let cut = prefix(s, n);
    let final_cut = match cut.rfind(' ') {
        Some(at) if len(&cut[..at]) as f64 > n as f64 * 0.6 => &cut[..at],
        _ => &cut,
    };
    format!("{}…", final_cut.trim_end())
}
fn headline(s: &str) -> String {
    if let Some(m) = re(r"^(.{12,}?)@s+[—–]@s+").captures(s) {
        return m[1].into();
    }
    if let Some(m) = re(r"^(.{20,}?[.!?])(?:@s|$)").captures(s) {
        if !re(r"(?i)\b(?:e\.g|i\.e|vs|etc|no|fig|approx)\.$").is_match(&m[1]) {
            return m[1].into();
        }
    }
    s.split('\n').next().unwrap_or(s).into()
}
fn status(text: &str) -> Option<&'static str> {
    if re(r"(?i)\b(?:needs? (?:a |an )?(?:human|you\b|the user|decision|diagnosis|answer|rebuild)|human action|waiting on (?:you|the user)|awaiting|blocked on|escalat|user must|ask the user|for the user to|one human)\b").is_match(text){return Some("waiting-on-you");}
    let mut found = None;
    let mut first = usize::MAX;
    for (status, pattern) in [
        (
            "in-flight",
            r"(?i)\b(?:in flight|in-flight|dispatched|dispatch(?:es)? out|in progress|underway|running now|being (?:built|written|fixed)|wip)\b",
        ),
        (
            "landed",
            r"(?i)\b(?:resolved|fixed|merged|landed|shipped|committed|pushed|closed|built)\b",
        ),
        (
            "next-up",
            r"(?i)\b(?:not yet dispatched|next thing to build|to be dispatched|next up|backlog|queued|to dispatch|would dispatch next)\b",
        ),
    ] {
        for m in re(pattern).find_iter(text) {
            if m.as_str().eq_ignore_ascii_case("dispatched")
                && text[..m.start()].to_lowercase().ends_with("not yet ")
            {
                continue;
            }
            if m.start() < first {
                first = m.start();
                found = Some(status);
            }
            break;
        }
    }
    found
}
fn marker(s: &str) -> (Option<String>, &str) {
    let pattern = re(
        r"^(?:[\x{1F000}-\x{1FAFF}\x{2600}-\x{27BF}\x{2B00}-\x{2BFF}\x{FE0F}\x{200D}\x{20E3}\x{2190}-\x{21FF}]+@s*)+",
    );
    match pattern.find(s) {
        Some(m) => (Some(trim(m.as_str()).into()), &s[m.end()..]),
        None => (None, s),
    }
}
fn refs(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let references = re(r"session:[0-9a-f]{6,}|\b[0-9a-f]{7,12}\b");
    for m in re(r"`([^`]+)`").captures_iter(s) {
        for hit in references.find_iter(&m[1]) {
            let value = hit.as_str().to_owned();
            if !out.contains(&value) {
                out.push(value);
            }
        }
    }
    for hit in re(r"\bsession:[0-9a-f]{6,}\b").find_iter(s) {
        let value = hit.as_str().to_owned();
        if !out.contains(&value) {
            out.push(value);
        }
    }
    out.truncate(5);
    out
}
fn parse_status(s: &str) -> Option<String> {
    let normalized = re(r"(?:@s|_)+")
        .replace_all(&trim(s).to_lowercase(), "-")
        .into_owned();
    ["in-flight", "waiting-on-you", "landed", "next-up"]
        .contains(&normalized.as_str())
        .then_some(normalized)
}
pub fn derive(entry: &Entry, column: &str, index: &Value) -> Value {
    let raw = &entry.text;
    let body = re(r"^ {0,3}(?:[-*+]|[0-9]+[.)])@s+")
        .replace(raw, "")
        .into_owned();
    let (glyph, mut body) = marker(&body);
    let date_pattern = re(r"^([0-9]{4}-[0-9]{2}-[0-9]{2})[:.,]?@s+");
    let date = date_pattern
        .captures(body)
        .map(|m| (m[1].to_owned(), m.get(0).unwrap().end()));
    if let Some((_, end)) = &date {
        body = &body[*end..];
    }
    let (glyph2, body) = marker(body);
    let glyphs = format!(
        "{}{}",
        glyph.as_deref().unwrap_or(""),
        glyph2.as_deref().unwrap_or("")
    );
    let retracted =
        re(r"(?i)^(?:[^A-Za-z0-9]{0,8})(?:WRONG|RETRACTED|SUPERSEDED|OBSOLETE|INCORRECT)\b")
            .is_match(&prefix(&plain(body), 80))
            || glyphs.contains(['❌', '✖', '✗', '🚫']);
    let mut title = String::new();
    if !retracted {
        if let Some(bold) = re(r"(?s)\*\*(.+?)\*\*").captures(body) {
            let text = plain(&bold[1]);
            if len(&body[..bold.get(0).unwrap().start()]) <= 12 && (8..=120).contains(&len(&text)) {
                title = text;
            }
        }
    }
    if title.is_empty() {
        title = plain(&headline(&plain(body)));
    }
    if title.is_empty() {
        title = plain(body.split('\n').next().unwrap_or(""));
    }
    if title.is_empty() {
        title = plain(raw);
        if title.is_empty() {
            title = "(empty entry)".into();
        }
    }
    title = truncate(&title, 96);
    let plain_body = plain(body);
    let after = plain_body
        .strip_prefix(title.strip_suffix('…').unwrap_or(&title))
        .unwrap_or(&plain_body);
    let summary = truncate(&re(r"^(?:@s|[—–\-:.,])+").replace(after, ""), 220);
    let state = if glyphs.contains(['⚠', '🚨', '❓', '❗']) {
        Some("waiting-on-you")
    } else if glyphs.contains(['🚧', '🔨', '🏗']) {
        Some("in-flight")
    } else if glyphs.contains(['✅', '✔', '☑', '🎉']) {
        Some("landed")
    } else {
        status(raw)
    };
    let mut card = json!({"id":entry.id,"column":column,"title":title,"summary":summary,"refs":refs(raw),"text":raw,"synthesized":false});
    if let Some(group) = &entry.group {
        card["group"] = json!(group);
    }
    if let Some(state) = state {
        card["status"] = json!(state);
    }
    if retracted {
        card["retracted"] = json!(true);
    }
    if let Some((date, _)) = date {
        card["date"] = json!(date);
    }
    if let Some(glyph) = glyph {
        card["marker"] = json!(glyph);
    }
    let map = index
        .get("cards")
        .filter(|v| v.is_object())
        .unwrap_or(index);
    if let Some(row) = map.get(&entry.id).filter(|v| v.is_object()) {
        let mut synthesized = false;
        for (key, limit) in [("title", 96), ("summary", 220)] {
            if let Some(text) = row[key].as_str().filter(|s| !trim(s).is_empty()) {
                card[key] = json!(truncate(&plain(trim(text)), limit));
                synthesized = true;
            }
        }
        if let Some(value) = row["status"].as_str() {
            if let Some(status) = parse_status(value) {
                card["status"] = json!(status);
            }
            synthesized |= !value.is_empty();
        }
        if let Some(refs) = row["refs"].as_array() {
            let refs: Vec<_> = refs
                .iter()
                .filter(|r| r.is_string())
                .take(5)
                .cloned()
                .collect();
            if !refs.is_empty() {
                card["refs"] = json!(refs);
            }
        }
        card["synthesized"] = json!(synthesized);
    }
    card
}
fn rank(v: &Value) -> usize {
    match v["status"].as_str() {
        Some("waiting-on-you") => 0,
        Some("in-flight") => 1,
        Some("next-up") => 2,
        Some("landed") => 3,
        _ => 4,
    }
}
pub fn cards(text: &str, index: &Value, archive: bool) -> (Vec<Value>, Vec<Value>) {
    let mut cards = Vec::new();
    let mut extras = Vec::new();
    for entry in document::parse(text).entries {
        if archive {
            let mut card = derive(&entry, "archive", index);
            card["group"] = json!(entry.column);
            card["archived"] = json!(true);
            cards.push(card);
        } else if let Some(column) = ["Now", "Direction", "Recently"]
            .into_iter()
            .find(|c| c.eq_ignore_ascii_case(trim(&entry.column)))
        {
            cards.push(derive(&entry, column, index));
        } else {
            let mut card = derive(&entry, "Now", index);
            card["group"] = json!(entry.column);
            extras.push(card);
        }
    }
    if !archive {
        cards.sort_by_key(rank);
    }
    (cards, extras)
}
