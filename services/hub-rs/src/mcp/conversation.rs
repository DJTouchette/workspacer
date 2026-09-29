use serde_json::{Value, json};

pub(super) fn reduce(value: Value, last_message: bool, text_only: bool) -> Value {
    if !last_message && !text_only {
        return value;
    }
    let Some(seq) = value["seq"].as_u64() else {
        return value;
    };
    let items = match value.get("items") {
        Some(Value::Array(items)) => items.as_slice(),
        None | Some(Value::Null) => &[],
        _ => return value,
    };
    if last_message {
        let mut blocks = Vec::new();
        for item in items.iter().rev() {
            match item["kind"].as_str().unwrap_or("") {
                "usage" | "plan" => (),
                "assistant_text" => {
                    if let Some(text) = item["text"].as_str().filter(|s| !s.is_empty()) {
                        blocks.push(text);
                    }
                }
                _ => break,
            }
        }
        if blocks.is_empty() {
            return json!({"seq":seq,"lastMessage":null,"note":"no assistant message in range (the session may not have replied yet, or sinceSeq is past its last reply)"});
        }
        blocks.reverse();
        return json!({"seq":seq,"lastMessage":blocks.join("\n\n")});
    }
    json!({"seq":seq,"items":items.iter().filter(|item| matches!(item["kind"].as_str(),Some("user_message" | "assistant_text"))).collect::<Vec<_>>()})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reductions_preserve_sequence_and_only_join_the_final_assistant_run() {
        let value = json!({"seq":42,"items":[{"kind":"assistant_text","text":"old"},{"kind":"tool_result"},{"kind":"assistant_text","text":"part one"},{"kind":"usage"},{"kind":"assistant_text","text":"part two"},{"kind":"plan"}]});
        assert_eq!(
            reduce(value.clone(), true, true),
            json!({"seq":42,"lastMessage":"part one\n\npart two"})
        );
        assert_eq!(
            reduce(value, false, true)["items"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        let pending = json!({"seq":43,"items":[{"kind":"assistant_text","text":"old"},{"kind":"user_message","text":"new request"}]});
        assert!(reduce(pending, true, false)["lastMessage"].is_null());
        for odd in [
            json!({"items":[]}),
            json!({"seq":-1}),
            json!({"seq":2,"items":"unknown"}),
            json!([1, 2]),
        ] {
            assert_eq!(reduce(odd.clone(), true, true), odd);
        }
        assert_eq!(
            reduce(json!({"seq":0}), false, true),
            json!({"seq":0,"items":[]})
        );
    }
}
