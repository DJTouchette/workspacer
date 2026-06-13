//! Test helpers for rendering the TUI into a string buffer.
//!
//! Used by integration tests to snapshot every interesting state of the
//! UI without launching a real terminal. The output is plain text — color
//! and modifiers are lost, but layout and content are preserved.

use ratatui::{backend::TestBackend, Terminal};

use crate::session::{transcript::Transcript, SessionMode, SessionState};
use crate::tui::app::{App, ChatState, View};
use crate::tui::editor::Editor;
use crate::tui::view::render;

/// Builder for an App in a known state. Use `.session()` to add sessions,
/// `.connected()` to mark the SSE stream as live, `.chat_for(id)` to put
/// the view into Chat for that session, and `.typed(s)` to seed the
/// editor buffer.
pub struct ScenarioBuilder {
    sessions: Vec<SessionState>,
    chat_session: Option<String>,
    chat_transcript: Transcript,
    typed: String,
    connected: bool,
}

impl ScenarioBuilder {
    pub fn new() -> Self {
        Self {
            sessions: Vec::new(),
            chat_session: None,
            chat_transcript: Transcript::default(),
            typed: String::new(),
            connected: false,
        }
    }

    pub fn session(mut self, s: SessionState) -> Self {
        self.sessions.push(s);
        self
    }

    pub fn connected(mut self) -> Self {
        self.connected = true;
        self
    }

    pub fn chat_for(mut self, session_id: impl Into<String>) -> Self {
        self.chat_session = Some(session_id.into());
        self
    }

    pub fn chat_for_with_transcript(
        mut self,
        session_id: impl Into<String>,
        transcript: Transcript,
    ) -> Self {
        self.chat_session = Some(session_id.into());
        self.chat_transcript = transcript;
        self
    }

    pub fn typed(mut self, s: impl Into<String>) -> Self {
        self.typed = s.into();
        self
    }

    fn build(self) -> (App, Option<String>) {
        let mut app = App::new("http://127.0.0.1:7891".into());
        app.connected = self.connected;
        for s in &self.sessions {
            app.order.push(s.session_id.clone());
            app.sessions.insert(s.session_id.clone(), s.clone());
        }
        if let Some(id) = &self.chat_session {
            let last = app
                .sessions
                .get(id)
                .map(|s| s.mode)
                .unwrap_or(SessionMode::Unknown);
            let mut editor = Editor::new();
            for ch in self.typed.chars() {
                editor.insert(ch);
            }
            app.view = View::Chat(ChatState {
                session_id: id.clone(),
                transcript: self.chat_transcript,
                editor,
                transcript_focus: false,
                expand_tool_results: false,
                scroll_offset: 0,
                last_seen_mode: last,
                render_cache: std::cell::RefCell::new(None),
            });
        }
        (app, self.chat_session)
    }
}

impl Default for ScenarioBuilder {
    fn default() -> Self {
        Self::new()
    }
}

fn render_to_string(app: &App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| render(f, app)).unwrap();
    let buffer = terminal.backend().buffer();
    let mut out = String::with_capacity((width as usize + 1) * height as usize);
    for y in 0..height {
        for x in 0..width {
            out.push_str(buffer[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

pub fn snapshot_dashboard(b: ScenarioBuilder, w: u16, h: u16) -> String {
    let (app, _) = b.build();
    render_to_string(&app, w, h)
}

pub fn snapshot_chat(b: ScenarioBuilder, w: u16, h: u16) -> String {
    let (app, _) = b.build();
    render_to_string(&app, w, h)
}
