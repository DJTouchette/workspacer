//! TUI application state + actions.

use std::collections::HashMap;

use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::session::{transcript::Transcript, SessionMode, SessionState};

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

/// Which screen the TUI is currently showing.
pub enum View {
    /// Fleet dashboard: list of sessions + details panel.
    Dashboard,
    /// Focused chat with a specific session: transcript scrollback + input box.
    Chat(ChatState),
}

pub struct ChatState {
    pub session_id: String,
    pub transcript: Transcript,
    pub input: String,
    /// Number of lines scrolled up from the bottom. 0 = follow tail.
    pub scroll_offset: u16,
    /// Last mode we observed for this session — used to detect mode→input
    /// transitions so we auto-refresh the transcript when the assistant
    /// finishes a turn.
    pub last_seen_mode: SessionMode,
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
    pub view: View,
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
            view: View::Dashboard,
        }
    }

    pub fn in_chat(&self) -> bool {
        matches!(self.view, View::Chat(_))
    }

    pub fn chat_session_id(&self) -> Option<String> {
        match &self.view {
            View::Chat(c) => Some(c.session_id.clone()),
            _ => None,
        }
    }

    /// Enter chat view for the currently selected session.
    pub async fn enter_chat(&mut self) {
        let Some(id) = self.order.get(self.selected).cloned() else { return };
        let last_mode = self
            .sessions
            .get(&id)
            .map(|s| s.mode)
            .unwrap_or(SessionMode::Unknown);
        self.view = View::Chat(ChatState {
            session_id: id.clone(),
            transcript: Transcript::default(),
            input: String::new(),
            scroll_offset: 0,
            last_seen_mode: last_mode,
        });
        self.fetch_transcript_for_chat().await;
    }

    pub fn leave_chat(&mut self) {
        self.view = View::Dashboard;
    }

    pub async fn fetch_transcript_for_chat(&mut self) {
        let View::Chat(chat) = &self.view else { return };
        let session_id = chat.session_id.clone();
        let url = format!("{}/sessions/{}/transcript", self.api_url, session_id);
        match self.client.get(&url).send().await {
            Ok(r) if r.status().is_success() => match r.json::<Transcript>().await {
                Ok(t) => {
                    if let View::Chat(chat) = &mut self.view {
                        chat.transcript = t;
                    }
                }
                Err(err) => self.toast(format!("transcript parse: {err}")),
            },
            Ok(r) => self.toast(format!("transcript {}", r.status())),
            Err(err) => self.toast(format!("transcript fetch: {err}")),
        }
    }

    pub async fn act_send_message(&mut self) {
        let View::Chat(chat) = &mut self.view else { return };
        if chat.input.is_empty() {
            return;
        }
        let session_id = chat.session_id.clone();
        let text = std::mem::take(&mut chat.input);

        // Decide the right endpoint based on mode. /message requires
        // mode=input; if Claude is responding the daemon would 409. Fall
        // back to raw /input in that case so the user can still type
        // (e.g. into Claude's first-run pickers, OAuth, etc.).
        let mode = self
            .sessions
            .get(&session_id)
            .map(|s| s.mode)
            .unwrap_or(SessionMode::Unknown);
        let (endpoint, body) = match mode {
            SessionMode::Input => (
                format!("{}/sessions/{}/message", self.api_url, session_id),
                json!({ "text": text.clone() }),
            ),
            _ => (
                format!("{}/sessions/{}/input", self.api_url, session_id),
                json!({ "text": text.clone() }),
            ),
        };
        match self.client.post(&endpoint).json(&body).send().await {
            Ok(r) if r.status().is_success() => {
                self.toast(format!("sent: {}", truncate(&text, 40)));
            }
            Ok(r) => {
                let status = r.status();
                let body = r.text().await.unwrap_or_default();
                self.toast(format!("{status}: {}", truncate(&body, 80)));
                // Restore the input on failure so the user can edit + retry.
                if let View::Chat(chat) = &mut self.view {
                    chat.input = text;
                }
            }
            Err(err) => {
                self.toast(format!("send failed: {err}"));
                if let View::Chat(chat) = &mut self.view {
                    chat.input = text;
                }
            }
        }
    }

    pub fn input_push(&mut self, ch: char) {
        if let View::Chat(chat) = &mut self.view {
            chat.input.push(ch);
        }
    }

    pub fn input_backspace(&mut self) {
        if let View::Chat(chat) = &mut self.view {
            chat.input.pop();
        }
    }

    pub fn chat_scroll(&mut self, delta: i32) {
        if let View::Chat(chat) = &mut self.view {
            if delta < 0 {
                chat.scroll_offset = chat.scroll_offset.saturating_add(-delta as u16);
            } else {
                chat.scroll_offset = chat.scroll_offset.saturating_sub(delta as u16);
            }
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

    /// Apply an incoming SSE/app event. Returns true if the caller should
    /// trigger an async side-effect (currently: refetch transcript when the
    /// chat session's mode flips back to `input`, i.e. the assistant just
    /// finished a turn).
    pub fn apply_event(&mut self, evt: AppEvent) -> bool {
        match evt {
            AppEvent::Update(upd) => {
                if !self.sessions.contains_key(&upd.session_id) {
                    self.order.push(upd.session_id.clone());
                }
                let new_mode = upd.state.mode;
                self.sessions.insert(upd.session_id.clone(), upd.state);

                if let View::Chat(chat) = &mut self.view {
                    if chat.session_id == upd.session_id {
                        let was = chat.last_seen_mode;
                        chat.last_seen_mode = new_mode;
                        // The assistant just stopped streaming → fetch the
                        // updated transcript on the next tick.
                        if was != SessionMode::Input && new_mode == SessionMode::Input {
                            return true;
                        }
                    }
                }
            }
            AppEvent::SseConnected => self.connected = true,
            AppEvent::SseDisconnected => self.connected = false,
            AppEvent::Toast(msg) => {
                self.toast = Some((msg, std::time::Instant::now()));
            }
        }
        false
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

    /// Pick the session id to act on: the chat session if we're in chat
    /// view, otherwise the dashboard selection.
    fn active_session_id(&self) -> Option<String> {
        match &self.view {
            View::Chat(c) => Some(c.session_id.clone()),
            View::Dashboard => self.order.get(self.selected).cloned(),
        }
    }

    pub async fn act_approve(&mut self, yes: bool) {
        let Some(id) = self.active_session_id() else { return };
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
        let Some(id) = self.active_session_id() else { return };
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
        let Some(id) = self.active_session_id() else { return };
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

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
