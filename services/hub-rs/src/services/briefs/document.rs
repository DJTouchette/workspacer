//! Byte-preserving brief model, matching shared/briefBoard.ts.
use anyhow::{Result, anyhow, bail};
use regex::Regex;
use serde::Serialize;
use serde_json::{Value, json};
pub const WS: &str = r"[\t\n\x0b\x0c\r \u{00a0}\u{1680}\u{2000}-\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}\u{feff}]";
pub fn re(pattern: &str) -> Regex {
    Regex::new(&pattern.replace("@s", WS).replace(r"\b", r"(?-u:\b)"))
        .expect("static brief pattern")
}
pub fn trim(s: &str) -> &str {
    super::super::dispatch_templates::trim_js(s)
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Section {
    pub title: String,
    pub level: usize,
    pub heading_line: usize,
    pub body_start: usize,
    pub body_end: usize,
    pub column: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct Entry {
    pub id: String,
    pub column: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub start: usize,
    pub end: usize,
    pub lines: Vec<String>,
    pub text: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct Document {
    pub lines: Vec<String>,
    pub sections: Vec<Section>,
    pub entries: Vec<Entry>,
}
pub fn entry_id(text: &str) -> String {
    fn hash(text: &str, mut h: u32) -> u32 {
        for n in text.encode_utf16() {
            for byte in [n as u8, (n >> 8) as u8] {
                h = (h ^ byte as u32).wrapping_mul(0x01000193);
            }
        }
        h
    }
    format!(
        "{:08x}{:08x}",
        hash(text, 0x811c9dc5),
        hash(text, 0x9dc5811c)
    )
}
pub fn is_heading(line: &str) -> bool {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    (1..=6).contains(&hashes)
        && line[hashes..]
            .chars()
            .next()
            .is_some_and(|c| trim(&c.to_string()).is_empty())
        && !trim(&line[hashes..]).is_empty()
}
pub fn parse(text: &str) -> Document {
    let lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
    let heading = re(r"^(#{1,6})@s+(.*?)@s*$");
    let bullet = re(r"^ {0,3}(?:[-*+]|[0-9]+[.)])@s+");
    let is_entry = |line: &str| {
        bullet
            .find(line)
            .is_some_and(|m| !trim(&line[m.end()..]).is_empty())
            && !heading.is_match(line)
    };
    let mut sections: Vec<Section> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some(m) = heading.captures(line) else {
            continue;
        };
        let level = m[1].len();
        if level < 2 {
            continue;
        }
        let end = (i + 1..lines.len())
            .find(|j| is_heading(&lines[*j]))
            .unwrap_or(lines.len());
        let title = m[2].to_owned();
        let column = if level > 2 {
            sections
                .iter()
                .rev()
                .find(|s| s.level < level)
                .map(|s| s.column.clone())
                .unwrap_or(title.clone())
        } else {
            title.clone()
        };
        sections.push(Section {
            title,
            level,
            heading_line: i,
            body_start: i + 1,
            body_end: end,
            column,
        });
    }
    let mut entries = Vec::new();
    for sec in &sections {
        let mut i = sec.body_start;
        while i < sec.body_end {
            if !is_entry(&lines[i]) {
                i += 1;
                continue;
            }
            let mut end = i + 1;
            while end < sec.body_end
                && !trim(&lines[end]).is_empty()
                && !is_entry(&lines[end])
                && !is_heading(&lines[end])
            {
                end += 1;
            }
            let slice = lines[i..end].to_vec();
            let text = slice.join("\n");
            entries.push(Entry {
                id: entry_id(&text),
                column: sec.column.clone(),
                group: (sec.level > 2).then(|| sec.title.clone()),
                start: i,
                end,
                lines: slice,
                text,
            });
            i = end;
        }
    }
    Document {
        lines,
        sections,
        entries,
    }
}
pub fn insertion(doc: &Document, column: &str) -> Option<usize> {
    let sec = doc
        .sections
        .iter()
        .find(|s| s.level == 2 && s.title.eq_ignore_ascii_case(column))?;
    let mut at = if column.eq_ignore_ascii_case("Recently") {
        sec.body_start
    } else {
        sec.body_end
    };
    if column.eq_ignore_ascii_case("Recently") {
        while at < sec.body_end && trim(&doc.lines[at]).is_empty() {
            at += 1;
        }
    } else {
        while at > sec.body_start && trim(&doc.lines[at - 1]).is_empty() {
            at -= 1;
        }
    }
    Some(at)
}
pub fn move_entry(text: &str, id: &str, column: &str) -> Result<String> {
    let doc = parse(text);
    let entry = doc
        .entries
        .iter()
        .find(|e| e.id == id)
        .ok_or_else(|| anyhow!("brief board: entry moved or was edited; reload the board"))?;
    if entry.column.eq_ignore_ascii_case(column) && entry.group.is_none() {
        return Ok(text.into());
    }
    let mut lines = doc.lines.clone();
    lines.drain(entry.start..entry.end);
    let at = insertion(&parse(&lines.join("\n")), column)
        .ok_or_else(|| anyhow!("brief board: missing ## {column} section"))?;
    lines.splice(at..at, entry.lines.clone());
    Ok(lines.join("\n"))
}
pub fn append_archive(archive: &str, lines: &[String], date: &str) -> String {
    let base = if trim(archive).is_empty() {
        "# Brief archive\n\nCold storage for entries pruned from the brief — never rewritten, only appended.\n"
    } else {
        archive
    };
    let doc = parse(base);
    if let Some(sec) = doc
        .sections
        .iter()
        .find(|s| s.level == 2 && trim(&s.title) == date)
    {
        let mut at = sec.body_end;
        while at > sec.body_start && trim(&doc.lines[at - 1]).is_empty() {
            at -= 1;
        }
        let mut out = doc.lines;
        out.splice(at..at, lines.iter().cloned());
        out.join("\n")
    } else {
        format!(
            "{base}{}\n## {date}\n{}\n",
            if base.ends_with('\n') { "" } else { "\n" },
            lines.join("\n")
        )
    }
}
pub fn archive(
    text: &str,
    old: &str,
    section: &str,
    count: Option<usize>,
    keep: Option<usize>,
    date: &str,
) -> Result<(String, String, usize)> {
    if count.is_some() == keep.is_some() {
        bail!(
            "brief.archive: give either count (archive this many of the oldest) or keep (leave this many of the newest), and not both"
        );
    }
    let doc = parse(text);
    if !doc
        .sections
        .iter()
        .any(|s| s.level == 2 && s.title.eq_ignore_ascii_case(section))
    {
        bail!(
            "brief board: this brief has no {:?} section",
            format!("## {section}")
        );
    }
    let mut entries: Vec<_> = doc
        .entries
        .iter()
        .filter(|e| e.column.eq_ignore_ascii_case(section))
        .collect();
    let n = count.unwrap_or_else(|| entries.len().saturating_sub(keep.unwrap()));
    if section.eq_ignore_ascii_case("Recently") {
        entries.reverse();
    }
    entries.truncate(n);
    entries.sort_by_key(|e| e.start);
    let mut archived = old.to_owned();
    for entry in &entries {
        archived = append_archive(&archived, &entry.lines, date);
    }
    let mut lines = doc.lines;
    for entry in entries.iter().rev() {
        lines.drain(entry.start..entry.end);
    }
    Ok((lines.join("\n"), archived, entries.len()))
}
pub fn stats(text: &str, section: &str) -> Value {
    let doc = parse(text);
    let body = doc
        .sections
        .iter()
        .find(|s| s.level == 2 && s.title.eq_ignore_ascii_case(trim(section)))
        .map(|s| doc.lines[s.body_start..s.body_end].join("\n"))
        .unwrap_or_default();
    json!({"entriesInSection":doc.entries.iter().filter(|e|e.column.eq_ignore_ascii_case(trim(section))).count(),"bytesInSection":body.len(),"bytesInBrief":text.len()})
}
pub fn flatten(line: &str) -> String {
    trim(&re(r"[\t\x0b\x0c]+").replace_all(
        &re(r"[ \t\x0c\x0b]*[\r\n]+[ \t\x0c\x0b]*").replace_all(line, " "),
        " ",
    ))
    .to_owned()
}
pub fn normalized(line: &str) -> Result<String> {
    let line = flatten(line);
    if line.is_empty() {
        bail!("brief.append: line is empty");
    }
    if line.encode_utf16().count() > 4000 {
        bail!("brief.append: line exceeds 4000 characters; nothing written; split the entry");
    }
    Ok(if line.starts_with("- ") || line.starts_with('#') {
        line
    } else {
        format!("- {line}")
    })
}
pub fn append(text: &str, section: &str, line: &str) -> Result<String> {
    let bullet = normalized(line)?;
    if trim(text).is_empty() {
        return Ok(["Now", "Direction", "Recently", "User"]
            .map(|s| {
                format!(
                    "## {s}\n{}",
                    if s == section {
                        format!("{bullet}\n")
                    } else {
                        String::new()
                    }
                )
            })
            .join("\n"));
    }
    let doc = parse(text);
    let heading = re(r"^#{1,6}@s+(.*?)@s*$");
    let Some(i) = doc.lines.iter().position(|line| {
        heading
            .captures(line)
            .is_some_and(|m| m[1].eq_ignore_ascii_case(section))
    }) else {
        return Ok(format!(
            "{text}{}\n## {section}\n{bullet}\n",
            if text.ends_with('\n') { "" } else { "\n" }
        ));
    };
    let end = (i + 1..doc.lines.len())
        .find(|j| is_heading(&doc.lines[*j]))
        .unwrap_or(doc.lines.len());
    let mut at = if section == "Recently" { i + 1 } else { end };
    if section == "Recently" {
        while at < end && trim(&doc.lines[at]).is_empty() {
            at += 1;
        }
    } else {
        while at > i + 1 && trim(&doc.lines[at - 1]).is_empty() {
            at -= 1;
        }
    }
    let mut lines = doc.lines;
    lines.insert(at, bullet);
    Ok(lines.join("\n"))
}
