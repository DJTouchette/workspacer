use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
pub const PROTOCOL: u32 = 2;
pub const MAX_SEQ: u64 = 9_007_199_254_740_991;
pub fn valid_id(id: &str) -> bool {
    (16..=128).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
pub fn dispatch_id() -> Result<String> {
    use rand::RngCore;
    let mut bytes = [0; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| anyhow::anyhow!("dispatch random source unavailable"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteOrigin {
    pub protocol: u32,
    pub dispatch_id: String,
    pub owner_key: String,
}
impl RemoteOrigin {
    pub fn validate(&self) -> Result<()> {
        if self.protocol != PROTOCOL {
            bail!("remote dispatch protocol mismatch; upgrade the older endpoint")
        };
        if !valid_id(&self.dispatch_id) || self.owner_key.is_empty() {
            bail!("invalid authenticated dispatch identity")
        };
        Ok(())
    }
}
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    WorkerFinished,
    WorkerEscalated,
    Blocked,
    Progress,
}
impl Kind {
    pub fn terminal(self) -> bool {
        matches!(self, Self::WorkerFinished | Self::WorkerEscalated)
    }
}
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Update {
    pub protocol: u32,
    pub dispatch_id: String,
    pub kind: Kind,
    pub session_id: String,
    pub seq: u64,
    pub ts: i64,
    #[serde(rename = "final")]
    pub terminal: bool,
    pub entry: Value,
}
impl Update {
    pub fn validate(&self) -> Result<()> {
        if self.protocol != PROTOCOL {
            bail!("protocol-mismatch")
        };
        if !valid_id(&self.dispatch_id)
            || self.session_id.is_empty()
            || self.seq == 0
            || self.seq > MAX_SEQ
            || self.terminal != self.kind.terminal()
            || !self.entry.is_object()
            || self.entry["sessionId"] != self.session_id
            || !self.entry["label"].is_string()
        {
            bail!("malformed dispatch update")
        };
        Ok(())
    }
}
fn js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\x0b' | '\x0c' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}
fn cap(text: &str, max: usize) -> String {
    let mut count = 0;
    text.chars()
        .take_while(|c| {
            count += c.len_utf16();
            count <= max
        })
        .collect()
}
fn line(value: &Value) -> Option<String> {
    let raw = value.as_str()?;
    let flat = raw
        .split(js_space)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (!flat.is_empty()).then(|| cap(&flat, 400))
}
pub fn sanitize_entry(value: &Value, session: &str) -> Value {
    let mut out =
        json!({"label":line(&value["label"]).unwrap_or_else(||"Agent".into()),"sessionId":session});
    for key in [
        "cwd",
        "lastReply",
        "note",
        "failed",
        "crossed",
        "escalationError",
    ] {
        if let Some(value) = line(&value[key]) {
            out[key] = value.into();
        }
    }
    if value["blockedOn"] == "approval" || value["blockedOn"] == "question" {
        out["blockedOn"] = value["blockedOn"].clone();
    }
    for key in ["stopped", "needsDecision"] {
        if value[key] == true {
            out[key] = true.into();
        }
    }
    for key in ["fullReply", "escalation"] {
        if let Some(value) = value[key]
            .as_str()
            .filter(|s| !s.trim_matches(js_space).is_empty())
        {
            out[key] = cap(value, 24_000).into();
        }
    }
    out
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Provider {
    pub provider: String,
    pub found: bool,
    pub authenticated: Option<bool>,
    pub note: String,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Directory {
    pub path: String,
    pub source: String,
    pub git: bool,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub protocol: u32,
    pub exact_model: bool,
    pub executes: bool,
    pub scope: String,
    pub providers: Vec<Provider>,
    pub cwds: Vec<Directory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsupported_reason: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Lease {
    pub owner: String,
    pub repo: String,
    pub cwd: String,
    pub provider: String,
    pub worktree: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub branch: String,
    pub expires: i64,
    pub claimed: bool,
    #[serde(default = "yes")]
    pub prepared: bool,
}
fn yes() -> bool {
    true
}
impl Lease {
    pub fn view(&self) -> Value {
        json!({"cwd":self.cwd,"repo":self.repo,"provider":self.provider,"worktree":self.worktree,"branch":self.branch,"expires":self.expires})
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct WorkerRecord {
    pub id: String,
    #[serde(default)]
    pub session: String,
    #[serde(default)]
    pub seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<Update>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease: Option<Lease>,
    #[serde(default, rename = "ackedAt")]
    pub acked_at: i64,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OriginRecord {
    pub dispatch_id: String,
    pub peer: String,
    pub owner_session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_session_id: Option<String>,
    pub opened_at: i64,
    #[serde(default)]
    pub acked_seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivering_seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_update: Option<Update>,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_schema: Option<Value>,
}
