//! TUI application state + actions.

use std::collections::HashMap;

use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::session::SessionState;

#[derive(Clone, Debug)]
pub enum AppEvent {
    /// SSE delivered a `session.update` frame.
    Update(SessionUpdate),
    /// SSE stream disconnected.
    SseDisconnected,
    /// SSE stream connected (initial or reconnect).
    SseConnected,
    /// Show a transient one-line toast.
    Toast(String),
}

#[derive(Clone, Debug, Deserialize)]
pub struct SessionUpdate {
    pub session_id: String,
    pub event: String,
    pub state: SessionState,
}

pub struct App {
    pub api_url: String,
    pub client: Client,
    pub sessions: HashMap<String, SessionState>,
    pub order: Vec<String>, // stable display order
    pub selected: usize,
    pub connected: bool,
    pub toast: Option<(String, std::time::Instant)>,
    /// Whether the gate is on for each session, mirrored locally so we can
    /// render it. The daemon is source of truth — we set this when we
    /// toggle, and clear on `refresh_initial`.
    pub gates: HashMap<String, bool>,
}

impl App {
    pub fn new(api_url: String) -> Self {
        Self {
            api_url: api_url.trim_end_matches('/').to_string(),
            client: Client::new(),
            sessions: HashMap::new(),
            order: Vec::new(),
            selected: 0,
            connected: false,
            toast: None,
            gates: HashMap::new(),
        }
    }

    pub async fn refresh_initial(&mut self) -> Result<()> {
        let url = format!("{}/sessions", self.api_url);
        let sessions: Vec<SessionState> = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?
            .error_for_status()?
            .json()
            .await?;
        self.sessions.clear();
        self.order.clear();
        for s in sessions {
            self.order.push(s.session_id.clone());
            self.sessions.insert(s.session_id.clone(), s);
        }
        if self.selected >= self.order.len() {
            self.selected = self.order.len().saturating_sub(1);
        }
        Ok(())
    }

    pub fn apply_event(&mut self, evt: AppEvent) {
        match evt {
            AppEvent::Update(upd) => {
                if !self.sessions.contains_key(&upd.session_id) {
                    self.order.push(upd.session_id.clone());
                }
                self.sessions.insert(upd.session_id.clone(), upd.state);
            }
            AppEvent::SseConnected => self.connected = true,
            AppEvent::SseDisconnected => self.connected = false,
            AppEvent::Toast(msg) => {
                self.toast = Some((msg, std::time::Instant::now()));
            }
        }
    }

    pub fn selected_session(&self) -> Option<&SessionState> {
        self.order.get(self.selected).and_then(|id| self.sessions.get(id))
    }

    pub fn select_prev(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn select_next(&mut self) {
        if self.selected + 1 < self.order.len() {
            self.selected += 1;
        }
    }

    pub async fn act_approve(&mut self, yes: bool) {
        let Some(id) = self.order.get(self.selected).cloned() else { return };
        let url = format!("{}/sessions/{}/approve", self.api_url, id);
        let body = if yes {
            json!({ "decision": "yes" })
        } else {
            json!({ "decision": "no" })
        };
        match self.client.post(&url).json(&body).send().await {
            Ok(r) if r.status().is_success() => {
                self.toast(format!("{} {}", if yes { "approved" } else { "denied" }, &id[..8]));
            }
            Ok(r) => {
                let status = r.status();
                let body = r.text().await.unwrap_or_default();
                self.toast(format!("{status}: {body}"));
            }
            Err(err) => self.toast(format!("approve failed: {err}")),
        }
    }

    pub async fn act_answer(&mut self, option: u8) {
        let Some(id) = self.order.get(self.selected).cloned() else { return };
        let url = format!("{}/sessions/{}/answer", self.api_url, id);
        let body = json!({ "option": option });
        match self.client.post(&url).json(&body).send().await {
            Ok(r) if r.status().is_success() => {
                self.toast(format!("answered option {option}"));
            }
            Ok(r) => {
                let status = r.status();
                let body = r.text().await.unwrap_or_default();
                self.toast(format!("{status}: {body}"));
            }
            Err(err) => self.toast(format!("answer failed: {err}")),
        }
    }

    pub async fn act_toggle_gate(&mut self) {
        let Some(id) = self.order.get(self.selected).cloned() else { return };
        let next = !self.gates.get(&id).copied().unwrap_or(false);
        let url = format!("{}/sessions/{}/gate", self.api_url, id);
        let body = json!({ "on": next });
        match self.client.post(&url).json(&body).send().await {
            Ok(r) if r.status().is_success() => {
                self.gates.insert(id.clone(), next);
                self.toast(format!("gate {}", if next { "ON" } else { "OFF" }));
            }
            Ok(r) => self.toast(format!("gate failed: {}", r.status())),
            Err(err) => self.toast(format!("gate failed: {err}")),
        }
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), std::time::Instant::now()));
    }

    pub fn current_toast(&self) -> Option<&str> {
        let (msg, t) = self.toast.as_ref()?;
        if t.elapsed() < std::time::Duration::from_secs(4) {
            Some(msg.as_str())
        } else {
            None
        }
    }

    pub fn gate_on(&self, session_id: &str) -> bool {
        self.gates.get(session_id).copied().unwrap_or(false)
    }
}

/// Helper kept here so tests don't need the network: parse one SSE
/// `data: {...}` payload into a SessionUpdate.
#[allow(dead_code)]
pub fn parse_sse_data(data: &str) -> Option<SessionUpdate> {
    let v: Value = serde_json::from_str(data).ok()?;
    serde_json::from_value(v).ok()
}
