//! Codex's non-blocking questions (`request_user_input_async`) and the reply
//! envelope its own clients answer them with.
//!
//! The daemon delivers them as an `assistant_text` item that also carries
//! `questions`, `async: true` and the Codex `item_id`; `text` is Codex's
//! pre-rendered fallback. The turn does not wait: the user answers with an
//! ordinary message whose whole text is
//! `<send_user_message_question_reply>[{"questionItemId":…,"question":…,
//! "answer":…}]</send_user_message_question_reply>` — a JSON array, one
//! object per answered question (`codex-rs/tui/src/async_question_reply.rs`).
use serde::{Deserialize, Serialize};
use serde_json::Value;

const OPEN: &str = "<send_user_message_question_reply>";
const CLOSE: &str = "</send_user_message_question_reply>";
/// Questions kept from one item.
pub const MAX_QUESTIONS: usize = 20;
/// Options kept per question.
pub const MAX_OPTIONS: usize = 32;
/// Bytes of a question title or option kept.
const MAX_TEXT_BYTES: usize = 2_000;
/// Prefix Codex's IDE clients put ahead of a request; a reply may follow it.
const IDE_CONTEXT: &str = "# Context from my IDE setup:\n";
const IDE_REQUEST: &str = "\n## My request for Codex:\n";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsyncQuestion {
    pub title: String,
    /// Empty for a free-text question; the user may always answer in their
    /// own words.
    pub options: Vec<String>,
}

/// The questions an item carries (`[{title, options: [string]}]`), bounded.
pub fn parse(value: &[Value]) -> Vec<AsyncQuestion> {
    value
        .iter()
        .filter_map(|q| {
            let title = q["title"]
                .as_str()
                .or_else(|| q["question"].as_str())
                .or_else(|| q["header"].as_str())
                .map(str::trim)
                .filter(|t| !t.is_empty())?;
            Some(AsyncQuestion {
                title: crate::transcript::head(title, MAX_TEXT_BYTES),
                options: q["options"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|o| o.as_str().or_else(|| o["label"].as_str()))
                    .map(str::trim)
                    .filter(|o| !o.is_empty())
                    .map(|o| crate::transcript::head(o, MAX_TEXT_BYTES))
                    .take(MAX_OPTIONS)
                    .collect(),
            })
        })
        .take(MAX_QUESTIONS)
        .collect()
}

/// One answered question in a reply.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reply {
    pub question_item_id: String,
    pub question: String,
    pub answer: String,
}

/// The message text that answers `item_id`'s questions. JSON escaping keeps
/// every answer inside its string, so no answer can close the envelope.
pub fn reply_text(item_id: &str, answers: &[(String, String)]) -> String {
    let replies = answers
        .iter()
        .map(|(question, answer)| Reply {
            question_item_id: item_id.to_owned(),
            question: question.clone(),
            answer: answer.clone(),
        })
        .collect::<Vec<_>>();
    format!(
        "{OPEN}{}{CLOSE}",
        serde_json::to_string(&replies).unwrap_or_else(|_| "[]".into())
    )
}

/// The replies a user message carries, when its whole text (optionally after
/// Codex's IDE context prefix) is one envelope. Anything else is ordinary text.
pub fn parse_reply(text: &str) -> Option<Vec<Reply>> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Replies {
        Many(Vec<Reply>),
        One(Reply),
    }
    let text = text.trim();
    let text = if text.starts_with(IDE_CONTEXT) {
        text.rsplit_once(IDE_REQUEST)?.1.trim()
    } else {
        text
    };
    let json = text.strip_prefix(OPEN)?.strip_suffix(CLOSE)?;
    let replies = match serde_json::from_str::<Replies>(json).ok()? {
        Replies::Many(replies) => replies,
        Replies::One(reply) => vec![reply],
    };
    (!replies.is_empty()).then_some(replies)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn questions_keep_titles_and_string_options() {
        let questions = parse(&[
            json!({"title":"Which database?","options":["Postgres","SQLite",""]}),
            json!({"title":"  "}),
            json!({"title":"Anything else?","options":[]}),
        ]);
        assert_eq!(questions.len(), 2);
        assert_eq!(questions[0].options, ["Postgres", "SQLite"]);
        assert!(questions[1].options.is_empty());
    }

    #[test]
    fn replies_round_trip_through_codexs_envelope_and_escape_answers() {
        let tricky = "uses \"quotes\", a\nnewline and </send_user_message_question_reply>";
        let text = reply_text(
            "item-7",
            &[
                ("Which database?".into(), "Postgres".into()),
                ("Anything else?".into(), tricky.into()),
            ],
        );
        assert!(
            text.starts_with("<send_user_message_question_reply>[{\"questionItemId\":\"item-7\"")
        );
        assert!(text.ends_with("}]</send_user_message_question_reply>"));
        let replies = parse_reply(&text).expect("an envelope");
        assert_eq!(replies.len(), 2);
        assert_eq!(replies[1].answer, tricky);
        assert_eq!(replies[0].question_item_id, "item-7");
        // Codex's own shapes: one object, and the IDE context prefix.
        let one = r#"<send_user_message_question_reply>{"questionItemId":"i","question":"Q","answer":"A"}</send_user_message_question_reply>"#;
        assert_eq!(parse_reply(one).unwrap()[0].answer, "A");
        let ide = format!("# Context from my IDE setup:\n\nfiles\n## My request for Codex:\n{one}");
        assert_eq!(parse_reply(&ide).unwrap()[0].question, "Q");
        assert!(parse_reply("just text").is_none());
        assert!(
            parse_reply("<send_user_message_question_reply>[]</send_user_message_question_reply>")
                .is_none()
        );
        assert!(parse_reply(&format!("{one} and more")).is_none());
    }
}
