//! Redact before bounding. See the UTF-16 wire contract and shared adversarial
//! fixtures in contracts/intent-report-cases.json; keep intentReport.ts in sync.
const LIMIT: usize = 4000;
fn space(c: char) -> bool {
    // ECMAScript whitespace, deliberately not Rust's broader is_whitespace().
    matches!(c, '\u{0009}'..='\u{000d}' | ' ' | '\u{00a0}' | '\u{1680}' |
        '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' |
        '\u{205f}' | '\u{3000}' | '\u{feff}')
}
fn consume(text: &str, mut start: usize, allowed: impl Fn(char) -> bool) -> usize {
    for c in text[start..].chars() {
        if !allowed(c) {
            break;
        }
        start += c.len_utf8();
    }
    start
}
pub fn bound(text: &str) -> (String, bool, bool) {
    let bytes = text.as_bytes();
    let lower = text.to_ascii_lowercase();
    let mut output = String::new();
    let mut copied = 0;
    let mut i = 0;
    while i < bytes.len() {
        let mut end = i;
        let mut replacement = String::new();
        if bytes[i..].starts_with(b"-----BEGIN ") {
            if let Some(header) = text[i + 11..].find("PRIVATE KEY-----").map(|n| i + 11 + n) {
                if !text[i + 11..header].contains('-') {
                    let close = text[header + 16..].find("-----END ").and_then(|footer| {
                        let start = header + 16 + footer + 9;
                        text[start..]
                            .find("PRIVATE KEY-----")
                            .map(|n| start + n + 16)
                    });
                    end = close.unwrap_or(text.len());
                    replacement = "[redacted private key]".into();
                }
            }
        }
        if end == i {
            for prefix in ["bearer", "basic"] {
                let start = i + prefix.len();
                if lower.as_bytes()[i..].starts_with(prefix.as_bytes())
                    && text[start..].chars().next().is_some_and(space)
                {
                    let value = consume(text, start, space);
                    end = consume(text, value, |c| {
                        c.is_ascii_alphanumeric() || "+/_=.-".contains(c)
                    });
                    if end > value {
                        replacement = "[redacted authorization]".into();
                    }
                }
            }
            for prefix in ["sk-", "ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_"] {
                if bytes[i..].starts_with(prefix.as_bytes()) {
                    let value = i + prefix.len();
                    let candidate = consume(text, value, |c| {
                        c.is_ascii_alphanumeric() || "_-".contains(c)
                    });
                    if candidate - value >= if prefix == "github_pat_" { 1 } else { 16 } {
                        end = candidate;
                        replacement = "[redacted token]".into();
                    }
                }
            }
            for key in [
                "password",
                "apikey",
                "api_key",
                "api-key",
                "accesstoken",
                "access_token",
                "access-token",
                "secret",
            ] {
                if !lower.as_bytes()[i..].starts_with(key.as_bytes()) {
                    continue;
                }
                let mut separator = consume(text, i + key.len(), space);
                if bytes.get(separator).is_some_and(|c| b"\"'".contains(c)) {
                    separator = consume(text, separator + 1, space);
                }
                if !bytes.get(separator).is_some_and(|c| b"=:".contains(c)) {
                    continue;
                }
                let mut value = consume(text, separator + 1, space);
                let quote = bytes.get(value).copied().filter(|c| b"\"'".contains(c));
                if quote.is_some() {
                    value += 1;
                }
                let candidate = if let Some(quote) = quote {
                    let mut cursor = value;
                    while cursor < bytes.len() && bytes[cursor] != quote {
                        cursor += if bytes[cursor] == b'\\' && cursor + 1 < bytes.len() {
                            2
                        } else {
                            1
                        };
                    }
                    cursor
                } else {
                    consume(text, value, |c| !space(c) && !",;\"'}".contains(c))
                };
                if candidate > value && &text[value..candidate] != "[redacted]" {
                    end = candidate;
                    replacement = format!("{}[redacted]", &text[i..value]);
                }
            }
            for scheme in ["http://", "https://"] {
                if !lower.as_bytes()[i..].starts_with(scheme.as_bytes()) {
                    continue;
                }
                let start = i + scheme.len();
                let stop = consume(text, start, |c| !space(c) && c != '/' && c != '@');
                if bytes.get(stop) == Some(&b'@') && bytes[start..stop].contains(&b':') {
                    end = stop + 1;
                    replacement = format!("{}[redacted]@", &text[i..start]);
                }
            }
        }
        if !replacement.is_empty() {
            output.push_str(&text[copied..i]);
            output.push_str(&replacement);
            copied = end;
            i = end;
        } else {
            i += 1;
        }
    }
    output.push_str(&text[copied..]);
    let redacted = output != text;
    let mut units = 0;
    let end = output
        .char_indices()
        .find_map(|(index, c)| {
            units += c.len_utf16();
            (units > LIMIT).then_some(index)
        })
        .unwrap_or(output.len());
    let truncated = end < output.len();
    output.truncate(end);
    (output, redacted, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn prefix(text: &str) -> String {
        let mut units = 0;
        text.chars()
            .take_while(|c| {
                units += c.len_utf16();
                units <= LIMIT
            })
            .collect()
    }
    fn check(input: &str, output: &str) {
        let actual = bound(input);
        assert_eq!(
            actual,
            (
                prefix(output),
                input != output,
                output.encode_utf16().count() > LIMIT
            )
        );
        assert!(actual.0.encode_utf16().count() <= LIMIT);
        assert_eq!(bound(&actual.0).0, actual.0);
        // Exercise the real wire projection for every shared case as well.
        let conv = super::super::conversation::ConversationStore::new();
        conv.push(
            "fixture",
            vec![
                super::super::conversation::ConversationItem::AssistantText {
                    text: input.into(),
                    timestamp: None,
                },
            ],
        );
        let wire = conv.completion_source("fixture");
        assert_eq!(wire["text"], actual.0);
        assert_eq!(wire["redacted"], actual.1);
        assert_eq!(wire["truncated"], actual.2);
        assert_eq!(wire["redactionVersion"], 1);
    }
    #[test]
    fn intent_report_shared_adversarial_contract() {
        let fixtures: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../contracts/intent-report-cases.json"
        ))
        .unwrap();
        for case in fixtures["cases"].as_array().unwrap() {
            let input = case["input"].as_str().unwrap();
            let output = case["output"].as_str().unwrap();
            check(input, output);
            for anchor in [3999, 4000, 4001] {
                for split in 0..=input.encode_utf16().count() {
                    let padding = format!("😀{}\n", ".".repeat(anchor - split - 3));
                    check(&format!("{padding}{input}"), &format!("{padding}{output}"));
                }
            }
        }
        for n in [3998, 3999, 4000, 4001] {
            let input = format!("{}😀\n", "x".repeat(n));
            check(&input, &input);
        }
        check(
            &format!("password={}", "x".repeat(5000)),
            "password=[redacted]",
        );
        check("Bearer\u{00a0}abcdef", "[redacted authorization]");
    }
}
