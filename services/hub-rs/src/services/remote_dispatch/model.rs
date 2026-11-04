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
/// One directory choice a host advertises (`fleet.dispatchCapabilities`
/// cwds, `queue.targets[].cwds`). `name` and `remote` identify the same
/// project across machines (docs/host-work-queue.md, "Project links"); older
/// peers send neither, so both default.
#[derive(Clone, Serialize, Deserialize, Debug, Default, PartialEq, Eq)]
pub struct Directory {
    pub path: String,
    /// The final path component. Empty from an older peer: use
    /// [`Directory::display_name`].
    #[serde(default)]
    pub name: String,
    /// The normalized `origin` URL ([`normalize_remote`]); `None` when there
    /// is none or the host could not tell (older peers).
    #[serde(default)]
    pub remote: Option<String>,
    pub source: String,
    pub git: bool,
}
impl Directory {
    /// The final component of `path`, with either separator (a peer may run
    /// another OS than this hub).
    pub fn name_of(path: &str) -> String {
        path.trim_end_matches(['/', '\\'])
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("")
            .to_owned()
    }
    /// `name`, or the final path component when an older peer sent none.
    pub fn display_name(&self) -> String {
        if self.name.is_empty() {
            Self::name_of(&self.path)
        } else {
            self.name.clone()
        }
    }
}
/// Normalize a Git remote URL into a machine-independent project identity:
/// the host lowercased, the scheme, `user@` (and any password) dropped, a
/// trailing `.git` or `/` dropped, and scp form `git@host:owner/repo` turned
/// into `host/owner/repo`. A port is kept. Owner/repo case is kept. Local
/// remotes (`file://`, plain paths) name a place on one machine, not a
/// project, so they have no identity: `None`.
pub fn normalize_remote(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() || url.chars().any(char::is_control) {
        return None;
    }
    let (authority, path) = if let Some((scheme, rest)) = url.split_once("://") {
        if scheme.is_empty() || scheme.eq_ignore_ascii_case("file") {
            return None;
        }
        rest.split_once('/').unwrap_or((rest, ""))
    } else {
        // scp form: `[user@]host:path`, where nothing before the first `:`
        // is a path separator. A single letter is a Windows drive (`C:\\x`).
        let (authority, path) = url.split_once(':')?;
        let host = authority.rsplit('@').next().unwrap_or("");
        if authority.contains(['/', '\\']) || host.len() <= 1 {
            return None;
        }
        (authority, path)
    };
    let host = authority
        .rsplit('@')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if host.is_empty() || host.starts_with(':') {
        return None;
    }
    let path = path.trim_matches('/');
    let path = path
        .strip_suffix(".git")
        .unwrap_or(path)
        .trim_end_matches('/');
    let path: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    if path.is_empty() {
        return None;
    }
    Some(format!("{host}/{}", path.join("/")))
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

#[cfg(test)]
mod identity_tests {
    use super::*;

    #[test]
    fn remotes_normalize_to_one_identity_across_spellings() {
        let same = "github.com/DJ/app";
        for url in [
            "git@github.com:DJ/app.git",
            "git@github.com:DJ/app",
            "git@GitHub.com:DJ/app.git/",
            "github.com:DJ/app.git",
            "https://github.com/DJ/app",
            "https://github.com/DJ/app.git",
            "https://github.com/DJ/app/",
            "https://GITHUB.COM/DJ/app.git/",
            "http://github.com/DJ/app",
            "https://dj@github.com/DJ/app.git",
            "https://dj:secret-token@github.com/DJ/app.git",
            "ssh://git@github.com/DJ/app.git",
            "ssh://github.com/DJ/app",
            "git://github.com/DJ/app.git",
            "git+ssh://git@github.com/DJ/app.git",
            "  https://github.com//DJ/app.git  ",
        ] {
            assert_eq!(normalize_remote(url).as_deref(), Some(same), "{url}");
        }
    }

    #[test]
    fn owner_case_ports_and_nesting_are_kept() {
        for (url, want) in [
            ("git@github.com:dj/app.git", "github.com/dj/app"),
            (
                "ssh://git@git.corp:2222/team/app.git",
                "git.corp:2222/team/app",
            ),
            (
                "https://git.corp:8443/Group/Sub/app.git",
                "git.corp:8443/Group/Sub/app",
            ),
            ("ssh://git@[::1]:22/team/app.git", "[::1]:22/team/app"),
            (
                "https://dev.azure.com/org/proj/_git/app",
                "dev.azure.com/org/proj/_git/app",
            ),
        ] {
            assert_eq!(normalize_remote(url).as_deref(), Some(want), "{url}");
        }
        assert_ne!(
            normalize_remote("git@github.com:DJ/app.git"),
            normalize_remote("git@github.com:dj/app.git")
        );
    }

    #[test]
    fn local_and_malformed_remotes_have_no_identity() {
        for url in [
            "",
            "   ",
            "/srv/git/app.git",
            "../app",
            "file:///srv/git/app.git",
            "FILE:///srv/git/app.git",
            "C:\\repos\\app",
            "C:/repos/app",
            "https://github.com",
            "https://github.com/",
            "https:///DJ/app",
            "git@github.com:",
            "git@github.com:.git",
            "://github.com/DJ/app",
            "https://github.com/DJ/app\n.git",
            "dir/sub:thing",
        ] {
            assert_eq!(normalize_remote(url), None, "{url:?}");
        }
    }

    #[test]
    fn directories_parse_from_older_peers_and_name_themselves() {
        let old: Directory = serde_json::from_value(
            json!({"path":"/Users/dj/code/app","source":"project","git":true}),
        )
        .unwrap();
        assert_eq!(old.name, "");
        assert_eq!(old.remote, None);
        assert_eq!(old.display_name(), "app");
        let new = serde_json::to_value(Directory {
            path: "/a/app".into(),
            name: "app".into(),
            remote: None,
            source: "active".into(),
            git: false,
        })
        .unwrap();
        assert_eq!(
            new,
            json!({"path":"/a/app","name":"app","remote":null,"source":"active","git":false})
        );
        assert_eq!(Directory::name_of("C:\\Users\\dj\\app\\"), "app");
        assert_eq!(Directory::name_of("/home/dj/app/"), "app");
        assert_eq!(Directory::name_of("/"), "");
    }
}
