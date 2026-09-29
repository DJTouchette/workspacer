//! Preserve the legacy distinction between text and YAML timestamp scalars.
//! serde_yaml still owns document decoding. This safe event reader supplies only
//! root-field style evidence; nested user data is never rewritten.
use anyhow::{Result, bail};
use chrono::{Datelike, Timelike};
use serde_json::Value;
use std::{collections::BTreeMap, rc::Rc, sync::OnceLock};
use yaml_rust::{
    parser::{Event, MarkedEventReceiver, Parser},
    scanner::{Marker, Scanner, TScalarStyle, Token, TokenType},
};

#[derive(Clone)]
struct Scalar {
    text: String,
    style: TScalarStyle,
    tag: Option<String>,
    character: usize,
}
enum Node {
    Scalar(Rc<Scalar>),
    Mapping(BTreeMap<String, Rc<Scalar>>),
    Other,
}
struct Frame {
    mapping: bool,
    key: Option<Option<String>>,
    fields: BTreeMap<String, Rc<Scalar>>,
    anchor: usize,
}
#[derive(Default)]
struct Styles {
    frames: Vec<Frame>,
    anchors: BTreeMap<usize, Rc<Node>>,
    root: Option<Rc<Node>>,
    handles: BTreeMap<String, String>,
}
impl Styles {
    fn insert(&mut self, node: Rc<Node>, anchor: usize) {
        if anchor != 0 {
            self.anchors.insert(anchor, node.clone());
        }
        let Some(frame) = self.frames.last_mut() else {
            self.root = Some(node);
            return;
        };
        if !frame.mapping {
            return;
        }
        if let Some(key) = frame.key.take() {
            if let Some(key) = key {
                frame.fields.remove(&key);
                if let Node::Scalar(scalar) = node.as_ref() {
                    frame.fields.insert(key, scalar.clone());
                }
            }
        } else {
            frame.key = Some(match node.as_ref() {
                Node::Scalar(s) if matches!(s.text.as_str(), "timestamp" | "createdAt") => {
                    Some(s.text.clone())
                }
                _ => None,
            });
        }
    }
}
impl MarkedEventReceiver for Styles {
    fn on_event(&mut self, event: Event, mark: Marker) {
        match event {
            Event::MappingStart(anchor) | Event::SequenceStart(anchor) => self.frames.push(Frame {
                mapping: matches!(event, Event::MappingStart(_)),
                key: None,
                fields: BTreeMap::new(),
                anchor,
            }),
            Event::MappingEnd | Event::SequenceEnd => {
                let frame = self.frames.pop().expect("parser balanced collection");
                let node = if frame.mapping {
                    Node::Mapping(frame.fields)
                } else {
                    Node::Other
                };
                self.insert(Rc::new(node), frame.anchor);
            }
            Event::Scalar(text, style, anchor, tag) => self.insert(
                Rc::new(Node::Scalar(Rc::new(Scalar {
                    text,
                    style,
                    tag: tag.and_then(|tag| match tag {
                        TokenType::Tag(handle, suffix) if handle.is_empty() => Some(suffix),
                        TokenType::Tag(handle, suffix) => Some(format!(
                            "{}{suffix}",
                            self.handles.get(&handle).unwrap_or(&handle)
                        )),
                        _ => None,
                    }),
                    character: mark.index(),
                }))),
                anchor,
            ),
            Event::Alias(anchor) => self.insert(
                self.anchors
                    .get(&anchor)
                    .cloned()
                    .unwrap_or_else(|| Rc::new(Node::Other)),
                0,
            ),
            _ => (),
        }
    }
}
fn fields(source: &str) -> Result<BTreeMap<String, Rc<Scalar>>> {
    let mut styles = Styles::default();
    styles
        .handles
        .insert("!!".into(), "tag:yaml.org,2002:".into());
    // The parser emits tag handles; resolve declared prefixes from its scanner,
    // stopping at the document body instead of interpreting YAML with text rules.
    for Token(_, token) in Scanner::new(source.chars()) {
        match token {
            TokenType::TagDirective(handle, prefix) => {
                styles.handles.insert(handle, prefix);
            }
            TokenType::StreamStart(_) | TokenType::VersionDirective(_, _) => (),
            _ => break,
        }
    }
    Parser::new(source.chars()).load(&mut styles, false)?;
    Ok(match styles.root.as_deref() {
        Some(Node::Mapping(fields)) => fields.clone(),
        _ => BTreeMap::new(),
    })
}
fn timestamp(text: &str) -> Option<String> {
    // Exact scalar resolver grammar from js-yaml/lib/type/timestamp.js. This
    // interprets a parser-delivered scalar, never YAML structure or field paths.
    static DATE: OnceLock<regex::Regex> = OnceLock::new();
    static STAMP: OnceLock<regex::Regex> = OnceLock::new();
    let date =
        DATE.get_or_init(|| regex::Regex::new(r"^([0-9]{4})-([0-9]{2})-([0-9]{2})$").unwrap());
    let stamp = STAMP.get_or_init(|| regex::Regex::new(r"^([0-9]{4})-([0-9]{1,2})-([0-9]{1,2})(?:[Tt]|[ \t]+)([0-9]{1,2}):([0-9]{2}):([0-9]{2})(?:\.([0-9]*))?(?:[ \t]*(Z|([-+])([0-9]{1,2})(?::([0-9]{2}))?))?$").unwrap());
    let captures = date.captures(text).or_else(|| stamp.captures(text))?;
    let number = |index| {
        captures
            .get(index)
            .map(|c| c.as_str().parse::<i64>().unwrap())
            .unwrap_or(0)
    };
    let mut year = number(1);
    // JavaScript Date.UTC treats input years0..99 as1900..1999, before calendar
    // overflow is applied. Its month/day/time fields deliberately roll over.
    if year < 100 {
        year += 1900;
    }
    let month = number(2) - 1;
    year += month.div_euclid(12);
    let date = chrono::NaiveDate::from_ymd_opt(
        year.try_into().ok()?,
        (month.rem_euclid(12) + 1) as u32,
        1,
    )?;
    let fraction = captures.get(7).map(|c| c.as_str()).unwrap_or("");
    let mut millis = 0_i64;
    for digit in fraction.bytes().take(3) {
        millis = millis * 10 + i64::from(digit - b'0');
    }
    for _ in fraction.len().min(3)..3 {
        millis *= 10;
    }
    let sign = if captures.get(9).is_some_and(|c| c.as_str() == "-") {
        -1
    } else {
        1
    };
    let offset = sign * (number(10) * 3600 + number(11) * 60);
    let time = date.and_hms_opt(0, 0, 0)?.checked_add_signed(
        chrono::Duration::days(number(3) - 1)
            + chrono::Duration::seconds(number(4) * 3600 + number(5) * 60 + number(6) - offset)
            + chrono::Duration::milliseconds(millis),
    )?;
    let year = if (0..=9999).contains(&time.year()) {
        format!("{:04}", time.year())
    } else {
        format!("{:+07}", time.year())
    };
    Some(format!(
        "{year}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        time.month(),
        time.day(),
        time.hour(),
        time.minute(),
        time.second(),
        time.nanosecond() / 1_000_000
    ))
}

pub(super) fn list_timestamp(raw: &[u8], document: &Value, key: &str) -> Option<String> {
    let Some(text) = document[key].as_str() else {
        return Some(String::new());
    };
    let Some(scalar) = std::str::from_utf8(raw)
        .ok()
        .and_then(|source| fields(source).ok())
        .and_then(|mut f| f.remove(key))
    else {
        // Auxiliary style evidence must not turn existing saved text into an
        // undated record. Preserve it if the style reader cannot classify it.
        return Some(text.to_owned());
    };
    if scalar.tag.as_deref() == Some("tag:yaml.org,2002:timestamp") {
        // An invalid explicitly typed timestamp is rejected by js-yaml rather
        // than silently treated as text. Skip that LIST row without rewriting it.
        return timestamp(&scalar.text);
    }
    Some(
        if scalar.style == TScalarStyle::Plain && scalar.tag.is_none() {
            timestamp(&scalar.text).unwrap_or_else(|| text.to_owned())
        } else {
            text.to_owned()
        },
    )
}

pub(super) fn encode(document: &Value, key: &str) -> Result<Vec<u8>> {
    let mut source = serde_yaml::to_string(document)?;
    let Some(text) = document[key].as_str() else {
        bail!("stored {key} must be text");
    };
    let styles = fields(&source)?;
    let scalar = styles
        .get(key)
        .ok_or_else(|| anyhow::anyhow!("stored {key} has no scalar span"))?;
    if scalar.style == TScalarStyle::Plain {
        let start = source
            .char_indices()
            .nth(scalar.character)
            .map(|(i, _)| i)
            .ok_or_else(|| anyhow::anyhow!("stored {key} scalar span is out of bounds"))?;
        if scalar.text != text || !source[start..].starts_with(text) {
            bail!("stored {key} scalar span does not match generated text");
        }
        // JSON string quoting is valid YAML double-quoted scalar syntax. This
        // changes only the generated root timestamp, preserving the YAML emitter.
        source.replace_range(start..start + text.len(), &serde_json::to_string(text)?);
    }
    Ok(source.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn implicit_and_explicit_dates_are_normalized_but_quoted_strings_stay_exact() {
        for (source, expected) in [
            (
                "timestamp: 2026-03-01T00:00:00.000Z\n",
                "2026-03-01T00:00:00.000Z",
            ),
            (
                "timestamp: !!timestamp '2026-03-01T00:00:00.000Z'\n",
                "2026-03-01T00:00:00.000Z",
            ),
            (
                "%TAG !t! tag:yaml.org,2002:\n---\ntimestamp: !t!timestamp '2026-03-01'\n",
                "2026-03-01T00:00:00.000Z",
            ),
            ("timestamp: 2026-3-1t4:5:6Z\n", "2026-3-1t4:5:6Z"),
            ("timestamp: 2026-3-1 4:5:6\n", "2026-3-1 4:5:6"),
            (
                "timestamp: '2026-03-01T00:00:00.000Z'\n",
                "2026-03-01T00:00:00.000Z",
            ),
            (
                "timestamp: !!str 2026-03-01T00:00:00.000Z\n",
                "2026-03-01T00:00:00.000Z",
            ),
            (
                "%TAG !s! tag:yaml.org,2002:\n---\ntimestamp: !s!str 2026-03-01\n",
                "2026-03-01",
            ),
            (
                "{timestamp: 2026-03-01T00:00:00.000Z}",
                "2026-03-01T00:00:00.000Z",
            ),
            (
                "value: &when 2026-03-01\ntimestamp: *when\n",
                "2026-03-01T00:00:00.000Z",
            ),
            (
                "value: &when '2026-03-01'\ntimestamp: *when\n",
                "2026-03-01",
            ),
            (
                "value: &when !!str 2026-03-01\ntimestamp: *when\n",
                "2026-03-01",
            ),
            ("timestamp: ordinary\n", "ordinary"),
            ("timestamp: 2026-99-01\n", "2034-03-01T00:00:00.000Z"),
            ("timestamp: +10000-03-01\n", "+10000-03-01"),
            ("nested: {timestamp: 2026-03-01}\ntimestamp: text\n", "text"),
            ("timestamp: |\n  2026-03-01\n", "2026-03-01\n"),
            ("timestamp: 5\n", ""),
        ] {
            let document: Value = serde_yaml::from_str(source).unwrap();
            assert_eq!(
                list_timestamp(source.as_bytes(), &document, "timestamp").unwrap(),
                expected,
                "{source}"
            );
        }
    }
    #[test]
    fn generated_root_string_is_quoted_using_character_spans_after_unicode() {
        let data = serde_json::json!({"name":"😀 café", "timestamp":"2026-03-01T00:00:00.000Z", "nested":{"timestamp":"ordinary"}});
        let raw = encode(&data, "timestamp").unwrap();
        let decoded: Value = serde_yaml::from_slice(&raw).unwrap();
        assert_eq!(decoded, data);
        assert_eq!(
            list_timestamp(&raw, &decoded, "timestamp").unwrap(),
            data["timestamp"].as_str().unwrap()
        );
        assert!(
            String::from_utf8(raw)
                .unwrap()
                .contains("timestamp: \"2026-03-01T00:00:00.000Z\"")
        );
    }
}
