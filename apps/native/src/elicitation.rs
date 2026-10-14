//! An MCP server asking the user for information (an MCP elicitation), as
//! the daemon puts it on the ordinary question card (`providers::elicitation`
//! in claudemon):
//!
//! - **Form mode**: one question per field, then a closing consent question
//!   headed `<server> asks` with the options `Submit` / `Decline`. Only an
//!   explicit Submit sends what the user typed to that server.
//! - **URL mode**: one question headed `<server> link` with `Open` (whose
//!   description is the URL) and `Decline`.
//!
//! This module only recognises those shapes so the card can make the
//! consent explicit; answers still go back as option labels through
//! `claude.answer`.
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The typed fields are the questions before the last one.
    Form,
    /// Opening this http(s) URL is what the server asks for.
    Url(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Elicitation {
    /// The MCP server's name, as the daemon wrote it.
    pub server: String,
    pub kind: Kind,
}

pub const SUBMIT: &str = "Submit";
pub const DECLINE: &str = "Decline";
pub const OPEN: &str = "Open";

fn labels(question: &Value) -> Vec<&str> {
    question["options"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|o| o["label"].as_str().or(o.as_str()))
        .collect()
}

/// Whether `url` is an absolute `http`/`https` URL with a host: the only
/// kind the card ever opens.
pub fn web_url(url: &str) -> bool {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"));
    rest.is_some_and(|r| {
        let host = r.split(['/', '?', '#']).next().unwrap_or("");
        !host.is_empty()
            && !host.starts_with('@')
            && !r.chars().any(|c| c.is_whitespace() || c.is_control())
    })
}

/// The elicitation a question list is, if it is one.
pub fn detect(questions: &[Value]) -> Option<Elicitation> {
    let last = questions.last()?;
    let header = last["header"].as_str().unwrap_or("").trim();
    let options = labels(last);
    if let Some(server) = header.strip_suffix(" asks")
        && options == [SUBMIT, DECLINE]
        && last["multiSelect"].as_bool() != Some(true)
    {
        return Some(Elicitation {
            server: crate::transcript::head(server.trim(), 128),
            kind: Kind::Form,
        });
    }
    if questions.len() == 1
        && let Some(server) = header.strip_suffix(" link")
        && options == [OPEN, DECLINE]
    {
        let url = last["options"][0]["description"]
            .as_str()
            .unwrap_or("")
            .trim();
        if web_url(url) {
            return Some(Elicitation {
                server: crate::transcript::head(server.trim(), 128),
                kind: Kind::Url(crate::transcript::head(url, 4_096)),
            });
        }
    }
    None
}

impl Elicitation {
    /// The index of the consent question (always the last).
    pub fn consent_index(&self, questions: &[Value]) -> usize {
        questions.len().saturating_sub(1)
    }

    /// The question's text without the url line the daemon appended (the
    /// card shows the URL on its own).
    pub fn message(&self, question: &Value) -> String {
        let text = question["question"].as_str().unwrap_or("").trim();
        match &self.kind {
            Kind::Url(url) => text
                .lines()
                .filter(|line| line.trim() != url && !line.contains("asks you to open:"))
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_owned(),
            Kind::Form => text.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn form() -> Vec<Value> {
        json!([
            {"header":"Repository","question":"Which repo? (required)","options":[]},
            {"header":"github asks","question":"Pick a repo\n\nSend these answers to the MCP server `github`?",
             "options":[{"label":"Submit","description":"Send the answers to `github`"},
                        {"label":"Decline","description":"Send nothing; the server is told you declined"}]}
        ])
        .as_array()
        .unwrap()
        .clone()
    }

    #[test]
    fn a_form_ends_with_its_consent_question() {
        let questions = form();
        let found = detect(&questions).unwrap();
        assert_eq!(found.server, "github");
        assert_eq!(found.kind, Kind::Form);
        assert_eq!(found.consent_index(&questions), 1);
        // An agent's own question with similar words is not an elicitation.
        let plain = json!([{"header":"Deploy","question":"Go?","options":[{"label":"Submit"},{"label":"Cancel"}]}]);
        assert!(detect(plain.as_array().unwrap()).is_none());
    }

    #[test]
    fn a_url_card_offers_only_web_links() {
        let card = |url: &str| {
            json!([{"header":"auth link","question":format!("Sign in\n\nThe MCP server `auth` asks you to open:\n{url}"),
                "options":[{"label":"Open","description":url},{"label":"Decline"}]}])
        };
        let ok = card("https://example.com/login?x=1");
        let found = detect(ok.as_array().unwrap()).unwrap();
        assert_eq!(found.kind, Kind::Url("https://example.com/login?x=1".into()));
        assert_eq!(found.message(&ok[0]), "Sign in");
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https://",
            "/tmp/x",
        ] {
            assert!(detect(card(bad).as_array().unwrap()).is_none(), "{bad}");
        }
        assert!(web_url("http://localhost:3000/cb"));
        assert!(!web_url("https://a b"));
    }
}
