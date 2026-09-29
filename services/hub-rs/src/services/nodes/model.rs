use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::{path::Path, time::Duration};
#[derive(Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CloudConfig {
    pub app: String,
    pub machine_id: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub token_file: String,
    #[serde(default)]
    pub base_url: String,
}
#[derive(Clone, Deserialize)]
pub struct Node {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub fly: Option<CloudConfig>,
}
impl Node {
    pub fn label(&self) -> &str {
        if self.label.is_empty() {
            &self.id
        } else {
            &self.label
        }
    }
    pub fn coordinates(&self) -> bool {
        self.fly
            .as_ref()
            .is_some_and(|cloud| !cloud.app.is_empty() && !cloud.machine_id.is_empty())
    }
}
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
pub fn load(path: &Path) -> Result<Vec<Node>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(_) => return Err(anyhow!("node registry could not be read")),
    };
    anyhow::ensure!(
        bytes.len() <= 4 * 1024 * 1024,
        "node registry exceeds size limit"
    );
    let mut nodes: Vec<Node> = serde_json::from_slice::<Option<Vec<Node>>>(&bytes)
        .map_err(|error| {
            anyhow!(
                "invalid node registry at line {} column {}",
                error.line(),
                error.column()
            )
        })?
        .unwrap_or_default();
    anyhow::ensure!(nodes.len() <= 256, "node registry exceeds256 entries");
    let mut ids = std::collections::BTreeSet::new();
    for node in &mut nodes {
        node.id = node.id.trim().into();
        node.label = node.label.trim().into();
        anyhow::ensure!(
            valid_id(&node.id),
            "node id requires1–64 letters, digits, - or _"
        );
        anyhow::ensure!(ids.insert(node.id.clone()), "duplicate node id");
        if let Some(cloud) = &mut node.fly {
            cloud.app = cloud.app.trim().into();
            cloud.machine_id = cloud.machine_id.trim().into();
            cloud.token = cloud.token.trim().into();
            cloud.token_file = cloud.token_file.trim().into();
            cloud.base_url = cloud.base_url.trim().into();
            anyhow::ensure!(
                cloud.app.is_empty() == cloud.machine_id.is_empty(),
                "node cloud coordinates require both app and machineId"
            );
        }
    }
    Ok(nodes)
}
pub(crate) fn token(cloud: &CloudConfig, fallback: impl FnOnce() -> String) -> Result<String> {
    if !cloud.token.is_empty() {
        return Ok(cloud.token.clone());
    }
    if !cloud.token_file.is_empty() {
        let metadata = std::fs::metadata(&cloud.token_file)
            .map_err(|_| anyhow!("node cloud credential file unreadable"))?;
        anyhow::ensure!(
            metadata.is_file() && metadata.len() <= 64 * 1024,
            "node cloud credential file invalid"
        );
        return Ok(std::fs::read_to_string(&cloud.token_file)
            .map_err(|_| anyhow!("node cloud credential file unreadable"))?
            .trim()
            .into());
    }
    Ok(fallback().trim().into())
}
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Available,
    Waking,
    Stopping,
    Stopped,
    Unreachable,
}
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExitRecord {
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub at: String,
}
impl ExitRecord {
    pub fn clean(&self) -> bool {
        self.reason.starts_with("signal-")
    }
    pub fn describe(&self) -> String {
        let at = if self.at.is_empty() {
            String::new()
        } else {
            format!(" at {}", line(&self.at))
        };
        if self.clean() {
            format!(
                "its previous run ended cleanly ({}){at}",
                line(&self.reason)
            )
        } else {
            format!(
                "ITS PREVIOUS RUN DID NOT END CLEANLY ({}){at} — the machine failed rather than being put to sleep",
                line(&self.reason)
            )
        }
    }
}
#[derive(Clone, Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub id: String,
    pub label: String,
    pub state: State,
    #[serde(skip_serializing_if = "zero")]
    pub since: i64,
    #[serde(skip_serializing_if = "zero")]
    pub last_seen: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub detail: String,
    pub wakeable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_exit: Option<ExitRecord>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub slept_by_hub: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub may_be_running: bool,
    #[serde(skip_serializing_if = "zero_u32")]
    pub wake_failures: u32,
}
fn zero(value: &i64) -> bool {
    *value == 0
}
fn zero_u32(value: &u32) -> bool {
    *value == 0
}
pub(crate) fn line(value: &str) -> String {
    let flat = value.replace(['\r', '\n'], " ");
    if flat.chars().count() > 240 {
        format!("{}…", flat.trim().chars().take(240).collect::<String>())
    } else {
        flat.trim().into()
    }
}
#[derive(Clone)]
pub struct Timings {
    pub poll: Duration,
    pub probe: Duration,
    pub register: Duration,
    pub register_poll: Duration,
    pub silent_strikes: u32,
    pub start_retries: u32,
    pub retry_delay: Duration,
    pub stop_grace: Duration,
    pub stop_timeout: Duration,
    pub keep_failed_wakes_running: bool,
}
impl Default for Timings {
    fn default() -> Self {
        Self {
            poll: Duration::from_secs(30),
            probe: Duration::from_secs(8),
            register: Duration::from_secs(90),
            register_poll: Duration::from_secs(2),
            silent_strikes: 2,
            start_retries: 1,
            retry_delay: Duration::from_secs(2),
            stop_grace: Duration::from_secs(45),
            stop_timeout: Duration::from_secs(120),
            keep_failed_wakes_running: false,
        }
    }
}
