//! Controls on one card or one point of a conversation: the id that binds a
//! decision to the card the user saw, "Always allow" options, compaction,
//! rewinding files (Claude), undoing turns (Codex) and forking.
//!
//! The hub's contract (`.rivet/context/modules/claudemon-providers.md`):
//! - `pendingRequestId` / `pendingApproval.requestId` name the card on
//!   screen; `claude.approve` and `claude.answer` echo it as `requestId`, and
//!   a card that moved on answers 409 `{error, requestId, current}`. Such a
//!   refusal is never retried: the client re-reads and shows the new card.
//! - `pendingApproval.alwaysOptions` lists what a session-scoped grant would
//!   allow; the user picks exactly one by its id. Legacy `"always"` is a
//!   plain allow, so no generic "Allow all" control exists here.
//! - `claude.rewindFiles` restores FILES to a user message (Claude stream),
//!   `claude.revert` drops TURNS from one (Codex), `agents.fork` starts a new
//!   session from the source, optionally before a message. Each needs that
//!   message's provider `uuid`, which steered messages never carry.
use serde_json::Value;

/// The claudemon text of a 409 for a card that is no longer shown.
const STALE_CARD: &str = "that card is no longer the one awaiting the user";

/// Whether a refused decision or answer was aimed at a card that has since
/// been replaced (claudemon's 409 `{error, requestId, current}`), rather
/// than refused on its merits.
pub fn stale_card(error: &str) -> bool {
    error.contains(STALE_CARD)
        || (error.contains("409")
            && error.contains("\"current\"")
            && error.contains("\"requestId\""))
}

/// The short notice for a stale card. Nothing is resent: the user decides
/// again on the card that is shown now.
pub const STALE_NOTICE: &str = "That request changed before your answer reached it, so nothing was sent. The current one is shown now.";

/// One "Always allow …" choice on an approval card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlwaysOption {
    pub id: String,
    /// The hub's exact words (`Always allow Bash(npm test:*) this session`).
    pub label: String,
    pub kind: String,
}

/// At most this many options are shown on one card.
pub const MAX_ALWAYS_OPTIONS: usize = 6;

/// The card's session-scoped grants, in the hub's order. Options without an
/// id or a label are dropped: a button must say exactly what it allows.
pub fn always_options(approval: &Value) -> Vec<AlwaysOption> {
    approval["alwaysOptions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|option| {
            let id = option["id"].as_str().filter(|id| !id.is_empty())?;
            let label = option["label"]
                .as_str()
                .map(str::trim)
                .filter(|l| !l.is_empty())?;
            (option["scope"].as_str().unwrap_or("session") == "session").then(|| AlwaysOption {
                id: crate::transcript::head(id, 256),
                label: crate::transcript::head(label, 240),
                kind: crate::transcript::head(option["kind"].as_str().unwrap_or(""), 64),
            })
        })
        .take(MAX_ALWAYS_OPTIONS)
        .collect()
}

/// Token counts the way the compaction divider reads them: `950`, `2.8k`,
/// `25k`, `1.2M`.
pub fn tokens_short(tokens: u64) -> String {
    match tokens {
        t if t >= 1_000_000 => {
            let m = t as f64 / 1_000_000.;
            if m >= 10. {
                format!("{}M", m.round() as u64)
            } else {
                format!("{m:.1}M").replace(".0M", "M")
            }
        }
        t if t >= 10_000 => format!("{}k", (t as f64 / 1_000.).round() as u64),
        t if t >= 1_000 => format!("{:.1}k", t as f64 / 1_000.).replace(".0k", "k"),
        t => t.to_string(),
    }
}

/// The divider a `compaction` item reads as:
/// `Context compacted (auto) · 25k → 2.8k tokens`.
pub fn compaction_label(trigger: Option<&str>, pre: Option<u64>, post: Option<u64>) -> String {
    let mut label = "Context compacted".to_owned();
    if let Some(trigger) = trigger.map(str::trim).filter(|t| !t.is_empty()) {
        label.push_str(&format!(" ({})", crate::transcript::head(trigger, 24)));
    }
    match (pre, post) {
        (Some(pre), Some(post)) => label.push_str(&format!(
            " · {} → {} tokens",
            tokens_short(pre),
            tokens_short(post)
        )),
        (Some(pre), None) => label.push_str(&format!(" · from {} tokens", tokens_short(pre))),
        (None, Some(post)) => label.push_str(&format!(" · now {} tokens", tokens_short(post))),
        (None, None) => {}
    }
    label
}

/// What a `claude.rewindFiles` answer says: the preview's file list and
/// counts, or the reason it cannot rewind, verbatim.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rewind {
    pub can_rewind: bool,
    pub error: Option<String>,
    pub files: Vec<String>,
    pub insertions: Option<u64>,
    pub deletions: Option<u64>,
}

impl Rewind {
    /// The hub's reply (`{ok, result}`) or the bare result.
    pub fn parse(reply: &Value) -> Self {
        let result = reply.get("result").filter(|r| r.is_object()).unwrap_or(reply);
        let files: Vec<String> = match result.get("filesChanged") {
            Some(Value::Array(files)) => files
                .iter()
                .filter_map(Value::as_str)
                .take(200)
                .map(|f| crate::transcript::head(f, 512))
                .collect(),
            _ => Vec::new(),
        };
        Self {
            can_rewind: result["canRewind"].as_bool().unwrap_or(false),
            error: result["error"]
                .as_str()
                .map(str::trim)
                .filter(|e| !e.is_empty())
                .map(|e| crate::transcript::head(e, 2_000)),
            files,
            insertions: result["insertions"].as_u64(),
            deletions: result["deletions"].as_u64(),
        }
    }

    /// `3 files · +0 −12`, or what the CLI said instead.
    pub fn summary(&self) -> String {
        if let Some(error) = &self.error {
            return error.clone();
        }
        if !self.can_rewind {
            return "Claude cannot rewind files to this message.".into();
        }
        let files = match self.files.len() {
            0 => "No files would change".to_owned(),
            1 => "1 file would be restored".to_owned(),
            n => format!("{n} files would be restored"),
        };
        match (self.insertions, self.deletions) {
            (None, None) => files,
            (added, removed) => format!(
                "{files} · +{} −{}",
                added.unwrap_or(0),
                removed.unwrap_or(0)
            ),
        }
    }
}

/// Which "go back" a message offers: Claude stream sessions restore files,
/// Codex sessions drop turns. Other transports offer neither.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GoBack {
    RewindFiles,
    Revert,
}

impl GoBack {
    pub fn of(provider: &str, transport: &str) -> Option<Self> {
        match (provider, transport) {
            ("claude" | "", "stream") => Some(Self::RewindFiles),
            ("codex", _) => Some(Self::Revert),
            _ => None,
        }
    }
    pub fn method(self) -> &'static str {
        match self {
            Self::RewindFiles => "claude.rewindFiles",
            Self::Revert => "claude.revert",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::RewindFiles => "Rewind files to here",
            Self::Revert => "Undo from here",
        }
    }
}

/// A uuid a provider can name: bounded plain text (the daemon refuses
/// anything else with a 400, so the client never offers it).
pub fn usable_uuid(uuid: &str) -> bool {
    !uuid.is_empty()
        && uuid.len() <= 128
        && uuid
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_:.".contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stale_cards_are_recognised_by_the_daemon_wording_or_the_409_shape() {
        assert!(stale_card(
            "daemon returned 409 Conflict: {\"error\":\"that card is no longer the one awaiting the user\",\"requestId\":\"a\",\"current\":\"b\"}"
        ));
        assert!(stale_card(
            "daemon returned 409 Conflict: {\"requestId\":\"a\",\"current\":null,\"error\":\"x\"}"
        ));
        assert!(!stale_card(
            "daemon returned 409 Conflict: {\"error\":\"session is not awaiting approval\"}"
        ));
        assert!(!stale_card("Request refused"));
    }

    #[test]
    fn always_options_keep_the_hub_words_and_need_an_id() {
        let approval = json!({"alwaysOptions":[
            {"id":"r1:0","label":"Always allow Bash(npm test:*) this session","scope":"session","kind":"addRules"},
            {"id":"","label":"No id"},
            {"id":"r1:2","label":"  "},
            {"id":"r1:3","label":"Forever","scope":"always"},
            {"id":"r1:1","label":"Allow all edits this session","scope":"session","kind":"setMode"}
        ]});
        let options = always_options(&approval);
        assert_eq!(
            options
                .iter()
                .map(|o| (o.id.as_str(), o.label.as_str()))
                .collect::<Vec<_>>(),
            [
                ("r1:0", "Always allow Bash(npm test:*) this session"),
                ("r1:1", "Allow all edits this session")
            ]
        );
        assert!(always_options(&json!({"toolName":"Bash"})).is_empty());
    }

    #[test]
    fn compaction_reads_trigger_and_tokens() {
        assert_eq!(
            compaction_label(Some("auto"), Some(25_000), Some(2_800)),
            "Context compacted (auto) · 25k → 2.8k tokens"
        );
        assert_eq!(compaction_label(None, None, None), "Context compacted");
        assert_eq!(
            compaction_label(Some("manual"), Some(1_200_000), None),
            "Context compacted (manual) · from 1.2M tokens"
        );
        assert_eq!(tokens_short(950), "950");
        assert_eq!(tokens_short(2_000), "2k");
        assert_eq!(tokens_short(12_400_000), "12M");
    }

    #[test]
    fn rewind_previews_summarise_files_and_keep_refusals_verbatim() {
        let preview = Rewind::parse(&json!({"ok":true,"result":{
            "canRewind":true,"filesChanged":["/w/a.rs","/w/b.rs"],"insertions":0,"deletions":12}}));
        assert_eq!(preview.summary(), "2 files would be restored · +0 −12");
        let refused = Rewind::parse(&json!({"ok":true,"result":{
            "canRewind":false,"error":"No file checkpoint found for this message."}}));
        assert!(!refused.can_rewind);
        assert_eq!(
            refused.summary(),
            "No file checkpoint found for this message."
        );
        assert_eq!(
            Rewind::parse(&json!({"canRewind":true})).summary(),
            "No files would change"
        );
    }

    #[test]
    fn go_back_follows_provider_and_transport() {
        assert_eq!(GoBack::of("claude", "stream"), Some(GoBack::RewindFiles));
        assert_eq!(GoBack::of("claude", "pty"), None);
        assert_eq!(GoBack::of("codex", "stream"), Some(GoBack::Revert));
        assert_eq!(GoBack::of("opencode", "stream"), None);
        assert!(usable_uuid("6f1c2e1a-0b5e-4bd8-9f7e-2d6a8a1b9c3d"));
        assert!(!usable_uuid(""));
        assert!(!usable_uuid("a b"));
    }
}
