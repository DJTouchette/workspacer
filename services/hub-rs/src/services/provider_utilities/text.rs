use serde_json::Value;
use std::sync::OnceLock;
fn re(slot: &'static OnceLock<regex::Regex>, pattern: &str) -> &'static regex::Regex {
    slot.get_or_init(|| regex::Regex::new(pattern).unwrap())
}
pub fn clip(text: &str, max: usize) -> String {
    let mut units = 0;
    text.chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= max
        })
        .collect()
}
fn clip_words(text: &str, max: usize) -> String {
    if text.encode_utf16().count() <= max {
        return text.into();
    }
    let cut = clip(text, max);
    match cut.rfind(' ') {
        Some(at) if cut[..at].encode_utf16().count() > max / 2 => cut[..at].trim().into(),
        _ => cut.trim().into(),
    }
}
fn undangle(mut words: Vec<&str>) -> String {
    const WORDS: &[&str] = &[
        "a", "an", "and", "at", "by", "for", "from", "in", "into", "of", "on", "or", "that", "the",
        "this", "to", "with",
    ];
    while words.len() > 1 && WORDS.contains(&words.last().unwrap().to_lowercase().as_str()) {
        words.pop();
    }
    words.join(" ")
}
pub fn sanitize(raw: &str) -> Option<String> {
    let line = raw.lines().map(str::trim).find(|s| !s.is_empty())?;
    static PREFIX: OnceLock<regex::Regex> = OnceLock::new();
    let text = re(&PREFIX, r"(?i)^[^:]{0,24}title[^:]{0,12}:\s*").replace(line, "");
    let text = text
        .trim_start_matches(|c: char| "#>*-".contains(c) || c.is_whitespace())
        .trim_matches(|c| "\"'`“”‘’".contains(c))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let text = text.trim_end_matches(['.', ',', ';', ':', '!']).trim();
    static REFUSAL: OnceLock<regex::Regex> = OnceLock::new();
    if text.is_empty()
        || text.encode_utf16().count() > 156
        || re(
            &REFUSAL,
            r"(?i)^(sorry|i'm sorry|i cannot|i can't|unfortunately|as an ai)\b",
        )
        .is_match(text)
    {
        return None;
    }
    let text = undangle(text.split(' ').take(7).collect());
    let text = clip_words(&text, 52);
    let text = undangle(text.split(' ').collect());
    (!text.is_empty()).then_some(text)
}
pub fn fallback(raw: &str) -> Option<String> {
    let line = raw
        .trim()
        .lines()
        .next()?
        .trim_start_matches(|c: char| "#>*-`".contains(c) || c.is_whitespace());
    let text = line.split_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty()).then(|| clip_words(&text, 104))
}
pub fn prompt(user: &str, assistant: &str) -> String {
    let mut text = format!(
        "Write a title for this coding-session conversation, from the first exchange below. Rules: 3 to 6 words. Imperative or noun phrase. No quotes, no trailing period, no preamble, no markdown. Name the actual task, not the tools. Reply with the title and nothing else.\n\nUser: {}",
        clip(user, 1200)
    );
    if !assistant.trim().is_empty() {
        text.push_str(&format!("\nAssistant: {}", clip(assistant, 600)));
    }
    text
}
pub fn serves(provider: &str, model: &str) -> bool {
    let model = model.trim().to_lowercase();
    match provider {
        "claude" => {
            [
                "default",
                "haiku",
                "sonnet",
                "sonnet[1m]",
                "opus",
                "opusplan",
                "fable",
            ]
            .contains(&model.as_str())
                || model.starts_with("claude-")
        }
        "codex" => {
            static RE: OnceLock<regex::Regex> = OnceLock::new();
            re(&RE, r"^(gpt-|o\d|codex-|gpt\d)").is_match(&model)
        }
        "copilot" => model == "auto",
        "opencode" | "pi" => {
            static RE: OnceLock<regex::Regex> = OnceLock::new();
            re(&RE, r"^[\w.-]+/[\w.:-]+$").is_match(&model)
        }
        _ => false,
    }
}
/// Providers with a one-shot title adapter (see `completion::title`).
pub const TITLE_PROVIDERS: &[&str] = &["claude", "codex", "opencode", "copilot", "pi"];

/// Which harness writes a title, and with which model.
///
/// `agents.autoTitle.provider` names a fixed harness; blank (the default) is
/// the titled agent's own provider, which is what Electron has always done.
/// The model is `autoTitle.models[harness]` — an explicit per-harness choice
/// that is passed EXACTLY, never swapped for another: if the CLI rejects it the
/// call fails and the caller records that, rather than a title quietly written
/// by a model nobody picked. Only the legacy single `autoTitle.model` (default
/// `haiku`, a Claude alias) is filtered by [`serves`], because it predates
/// multi-provider titling and is not a choice made for this harness.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitleTarget {
    pub provider: String,
    pub model: Option<String>,
    /// True when the model came from `autoTitle.models[provider]`.
    pub explicit: bool,
}
pub fn title_target(config: &Value, agent_provider: &str) -> TitleTarget {
    let auto = &config["agents"]["autoTitle"];
    let provider = auto["provider"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(agent_provider)
        .to_owned();
    let chosen = auto["models"][provider.as_str()]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(model) = chosen {
        return TitleTarget {
            provider,
            model: Some(model.into()),
            explicit: true,
        };
    }
    let model = auto["model"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty() && serves(&provider, s))
        .map(str::to_owned)
        .or_else(|| (provider == "claude").then(|| "haiku".into()));
    TitleTarget {
        provider,
        model,
        explicit: false,
    }
}
pub fn strip_ansi(raw: &str) -> String {
    static ANSI: OnceLock<regex::Regex> = OnceLock::new();
    re(&ANSI, "\x1b\\[[0-9;?]*[ -/]*[@-~]")
        .replace_all(raw, "")
        .into_owned()
}
pub fn extract(provider: &str, raw: &str) -> String {
    let raw = strip_ansi(raw);
    if !["codex", "opencode"].contains(&provider) {
        return raw;
    }
    let mut json_seen = false;
    let mut result = String::new();
    for line in raw.lines().map(str::trim).filter(|s| s.starts_with('{')) {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        json_seen = true;
        if provider == "codex" && value["item"]["type"] == "agent_message" {
            if let Some(text) = value["item"]["text"].as_str() {
                result = text.into();
            }
        } else if provider == "opencode" && value["type"] == "text" {
            if let Some(text) = value["part"]["text"].as_str() {
                result.push_str(text);
            }
        }
    }
    if result.trim().is_empty() && !json_seen {
        raw
    } else {
        result
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Unsupported,
    Missing,
    Authentication,
    Limited,
    Timeout,
    Network,
    Empty,
    Error,
}
impl Failure {
    pub fn state(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::Missing => "unchecked",
            Self::Authentication => "unauthenticated",
            Self::Limited => "limited",
            Self::Timeout => "timeout",
            Self::Network => "network-error",
            Self::Empty | Self::Error => "error",
        }
    }
    /// Why a title call produced no model title, as recorded on the session.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::Missing => "missing",
            Self::Authentication => "unauthenticated",
            Self::Limited => "limited",
            Self::Timeout => "timeout",
            Self::Network => "network-error",
            Self::Empty => "empty",
            Self::Error => "error",
        }
    }
}
pub fn classify(raw: &str) -> Failure {
    static AUTH: OnceLock<regex::Regex> = OnceLock::new();
    static LIMIT: OnceLock<regex::Regex> = OnceLock::new();
    static MODEL: OnceLock<regex::Regex> = OnceLock::new();
    static MISSING: OnceLock<regex::Regex> = OnceLock::new();
    static NETWORK: OnceLock<regex::Regex> = OnceLock::new();
    if re(&AUTH,r"(?i)no api key|not logged in|not authenticated|please (run )?(`?codex )?login|run /login|use /login|invalid api key|unauthorized|authentication (failed|required)|\b401\b").is_match(raw) { Failure::Authentication }
    else if re(&LIMIT,r"(?i)rate.?limit|too many requests|\b429\b|quota|usage limit|out of credits|overage").is_match(raw) {Failure::Limited}
    else if re(&MODEL,r"(?i)unknown model|invalid model|model not found|unsupported model|no such model|model[^\n]*(not supported|not available|does not exist)").is_match(raw) {Failure::Unsupported}
    else if re(&MISSING,r"(?i)command not found|not recognized as an internal|\benoent\b").is_match(raw) {Failure::Missing}
    else if re(&NETWORK,r"(?i)ECONN|ENOTFOUND|EAI_AGAIN|network|connection|fetch failed").is_match(raw) {Failure::Network}
    else {Failure::Error}
}
