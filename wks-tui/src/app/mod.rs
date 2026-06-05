//! Application state and input handling.
//!
//! The app owns no socket directly — it holds a [`Claudemon`] client and an
//! `AppMsg` sender, and every action that needs the network spawns a task that
//! calls claudemon and posts the outcome back as an [`AppMsg`]. The main loop
//! (in `main.rs`) drives `draw` + `handle_key` + `apply_msg`. This keeps
//! rendering synchronous and the network fully async, fire-and-refresh.

mod input;
pub mod tasks;

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedSender;

use crate::claudemon::{Claudemon, PtyChunk};
use crate::library::LibraryItem;
use crate::profiles::Profile;
use crate::terminal::Term;
use crate::types::{Agent, Turn};

use tasks::{fetch_agents, fetch_transcript};

/// Messages spawned tasks post back to the app loop.
#[derive(Debug)]
pub enum AppMsg {
    Agents(Vec<Agent>),
    Transcript { session_id: String, turns: Vec<Turn> },
    Toast(String),
    /// A session has no PTY to stream (external/observed-only) — fall back to
    /// the transcript view for it.
    TerminalUnavailable(String),
    /// A shell spawned for a `new terminal` tab is ready.
    ShellSpawned { agent_id: String, session_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    /// Sidebar focus: the content pane previews the selected item (the
    /// Dashboard overview, or an agent's details).
    List,
    /// An agent is open with its tab bar; the active tab drives the content.
    Agent { id: String },
}

/// What a tab renders. Both are claudemon sessions rendered as terminals;
/// `Shell` tabs are generic shells the TUI spawned alongside the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabKind {
    Claude,
    Shell,
}

#[derive(Debug, Clone)]
pub struct Tab {
    pub title: String,
    pub session_id: String,
    pub kind: TabKind,
}

/// The set of tabs open for one agent (its Claude session + any shells).
#[derive(Debug, Clone, Default)]
pub struct Workspace {
    pub tabs: Vec<Tab>,
    pub active: usize,
}

impl Workspace {
    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }
}

/// How the chat view renders an agent: the raw PTY ("terminal path", default)
/// or the parsed transcript ("GUI path"). Toggled with `t`, mirroring the
/// Electron pane's GUI/terminal switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatMode {
    Terminal,
    Transcript,
}

/// State of the "spawn a new agent" modal. Profile-centric: a working directory
/// plus a chosen profile (which carries model / skip-permissions in its args).
#[derive(Debug, Clone)]
pub struct SpawnForm {
    pub cwd: String,
    pub profile_idx: usize,
    /// Candidate directory names from the last `tab` completion, shown under the
    /// field when the path is ambiguous. Cleared on any edit.
    pub completions: Vec<String>,
    /// When set (spawn-from-library), the prompt is seeded into the new agent
    /// once it reaches its input prompt.
    pub initial_prompt: Option<String>,
}

/// What a command-palette entry does when chosen.
#[derive(Debug, Clone)]
pub enum PaletteAction {
    NewAgent,
    NewTerminal,
    Dashboard,
    OpenAgent(String),
    /// Insert this text into the focused agent's terminal.
    Insert(String),
    /// Spawn a new agent seeded with this prompt.
    SpawnWithPrompt(String),
}

#[derive(Debug, Clone)]
pub struct PaletteItem {
    pub label: String,
    pub hint: String,
    pub action: PaletteAction,
}

/// The Ctrl-K command palette: a fuzzy launcher over actions, agents, and
/// library items.
pub struct Palette {
    pub query: String,
    items: Vec<PaletteItem>,
    /// Indices into `items` matching `query`, recomputed on each edit.
    pub filtered: Vec<usize>,
    pub selected: usize,
}

impl Palette {
    pub(super) fn new(items: Vec<PaletteItem>) -> Self {
        let filtered = (0..items.len()).collect();
        Palette { query: String::new(), items, filtered, selected: 0 }
    }

    pub(super) fn refilter(&mut self) {
        let q = self.query.to_lowercase();
        self.filtered = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| fuzzy_match(&q, &it.label.to_lowercase()))
            .map(|(i, _)| i)
            .collect();
        if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len().saturating_sub(1);
        }
    }

    pub fn visible(&self) -> impl Iterator<Item = &PaletteItem> {
        self.filtered.iter().map(move |&i| &self.items[i])
    }

    pub(super) fn chosen(&self) -> Option<&PaletteItem> {
        self.filtered.get(self.selected).map(|&i| &self.items[i])
    }
}

/// Subsequence fuzzy match: every char of `needle` appears in `haystack` in
/// order. Empty needle matches everything. Both expected lowercase.
fn fuzzy_match(needle: &str, haystack: &str) -> bool {
    let mut hay = haystack.chars();
    needle.chars().all(|c| hay.any(|h| h == c))
}

const TOAST_TTL: Duration = Duration::from_millis(2500);

pub struct App {
    pub(super) claudemon: Claudemon,
    pub(super) tx: UnboundedSender<AppMsg>,
    /// Sender the PTY stream task pushes chunks into; the main loop drains it
    /// and calls [`App::feed_pty`].
    pub(super) pty_tx: UnboundedSender<PtyChunk>,

    pub profiles: Vec<Profile>,
    pub library: Vec<LibraryItem>,
    pub spawn_form: Option<SpawnForm>,
    pub palette: Option<Palette>,

    pub connected: bool,
    pub should_quit: bool,

    /// Chat rendering mode.
    pub chat_mode: ChatMode,
    /// Live terminal emulators, kept warm per session so re-opening an agent is
    /// instant and correct (no re-attach / blank screen). Created lazily the
    /// first time you open an agent; fed continuously in the background by their
    /// PTY stream tasks; pruned when the session disappears.
    pub terms: HashMap<String, Term>,
    pub(super) term_tasks: HashMap<String, tokio::task::AbortHandle>,
    /// Sessions known to have no PTY (external/observed-only); they render the
    /// transcript instead of a terminal.
    pub no_terminal: HashSet<String>,
    /// Tabs per open agent, keyed by the agent's (Claude) session id.
    pub workspaces: HashMap<String, Workspace>,
    pub term_attached: bool,
    /// A pending `(cols, rows)` to push to claudemon after the next draw, set
    /// when the pane resized the open emulator.
    pub term_resize: Option<(u16, u16)>,

    /// Agents, kept sorted with the ones needing attention first.
    pub agents: Vec<Agent>,
    pub selected: usize,

    pub view: View,
    pub turns: Vec<Turn>,
    /// Top-line offset of the transcript viewport. Authoritative after each
    /// render, which clamps it to the content height.
    pub chat_scroll: u16,
    /// When true, the transcript sticks to the bottom as new content streams
    /// in; the renderer keeps `chat_scroll` pinned to the max. Any manual
    /// scroll clears it.
    pub chat_follow: bool,
    /// True when the composer is capturing keystrokes (vim insert mode).
    pub insert_mode: bool,
    pub input: String,

    pub(super) toast: Option<(String, Instant)>,
}

impl App {
    pub fn new(
        claudemon: Claudemon,
        profiles: Vec<Profile>,
        library: Vec<LibraryItem>,
        tx: UnboundedSender<AppMsg>,
        pty_tx: UnboundedSender<PtyChunk>,
    ) -> Self {
        Self {
            claudemon,
            tx,
            pty_tx,
            profiles,
            library,
            spawn_form: None,
            palette: None,
            connected: false,
            should_quit: false,
            chat_mode: ChatMode::Terminal,
            terms: HashMap::new(),
            term_tasks: HashMap::new(),
            no_terminal: HashSet::new(),
            workspaces: HashMap::new(),
            term_attached: false,
            term_resize: None,
            agents: Vec::new(),
            selected: 0,
            view: View::List,
            turns: Vec::new(),
            chat_scroll: 0,
            chat_follow: false,
            insert_mode: false,
            input: String::new(),
            toast: None,
        }
    }

    // ── live toast ──────────────────────────────────────────────────────────

    pub fn toast(&self) -> Option<&str> {
        match &self.toast {
            Some((msg, at)) if at.elapsed() < TOAST_TTL => Some(msg),
            _ => None,
        }
    }

    pub(super) fn set_toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    // ── inbound messages ──────────────────────────────────────────────────

    pub fn apply_msg(&mut self, msg: AppMsg) {
        match msg {
            AppMsg::Agents(list) => self.set_agents(list),
            AppMsg::Transcript { session_id, turns } => {
                // Ignore late transcripts for a session we've navigated away from.
                // While following, the renderer keeps us pinned to the bottom.
                if self.chat_session_id().as_deref() == Some(session_id.as_str()) {
                    self.turns = turns;
                }
            }
            AppMsg::Toast(t) => self.set_toast(t),
            AppMsg::TerminalUnavailable(sid) => self.mark_no_terminal(sid),
            AppMsg::ShellSpawned { agent_id, session_id } => {
                self.add_shell_tab(agent_id, session_id)
            }
        }
    }

    /// Record that a session has no PTY and drop its (useless) warm terminal. If
    /// it's the open chat in terminal mode, fall back to the transcript.
    fn mark_no_terminal(&mut self, session_id: String) {
        self.no_terminal.insert(session_id.clone());
        self.terms.remove(&session_id);
        if let Some(h) = self.term_tasks.remove(&session_id) {
            h.abort();
        }
        if self.open_session_id().as_deref() == Some(session_id.as_str()) {
            self.chat_mode = ChatMode::Transcript;
            self.term_attached = false;
            self.chat_follow = true;
            self.load_transcript(session_id);
        }
    }

    pub(super) fn set_agents(&mut self, mut list: Vec<Agent>) {
        // Waiting agents float to the top, like the remote client's sort.
        list.sort_by(|a, b| (b.is_waiting() as u8).cmp(&(a.is_waiting() as u8)));
        // Preserve selection on the same session id where possible.
        let prev_id = self.agents.get(self.selected).map(|a| a.session_id.clone());
        self.agents = list;
        if let Some(id) = prev_id {
            if let Some(i) = self.agents.iter().position(|a| a.session_id == id) {
                self.selected = i;
            }
        }
        if self.selected >= self.agents.len() {
            self.selected = self.agents.len().saturating_sub(1);
        }
        // Drop warm terminals (and no-PTY marks) for sessions that have gone away.
        let live: HashSet<String> = self.agents.iter().map(|a| a.session_id.clone()).collect();
        self.prune_terminals(&live);
        self.no_terminal.retain(|sid| live.contains(sid));
        // Drop workspaces whose agent is gone (shell tabs may persist as their
        // own sessions, but the agent grouping is no longer meaningful).
        self.workspaces.retain(|agent_id, _| live.contains(agent_id));
    }

    // ── daemon reactions ──────────────────────────────────────────────────

    pub fn on_connected(&mut self) {
        self.connected = true;
        self.refresh();
        self.maybe_load_transcript();
    }

    pub fn on_disconnected(&mut self) {
        self.connected = false;
    }

    /// A session changed — re-pull the list and, if we're reading a transcript,
    /// refresh it. (Terminal mode updates live over its own PTY stream.)
    pub fn on_changed(&mut self) {
        self.refresh();
        self.maybe_load_transcript();
    }

    fn maybe_load_transcript(&self) {
        if self.chat_mode == ChatMode::Transcript {
            if let Some(sid) = self.chat_session_id() {
                self.load_transcript(sid);
            }
        }
    }

    /// Feed a PTY chunk into its session's emulator. Feeds background terminals
    /// too, so every cached agent stays current even while you're elsewhere.
    pub fn feed_pty(&mut self, chunk: PtyChunk) {
        if let Some(term) = self.terms.get_mut(&chunk.session_id) {
            term.feed(&chunk.bytes);
        }
    }

    /// Push a pending PTY resize to claudemon (called after each draw, since the
    /// renderer is what learns the pane size). Reflows Claude's TUI to the pane.
    pub fn flush_term_resize(&mut self) {
        let Some((cols, rows)) = self.term_resize.take() else { return };
        let Some(sid) = self.open_session_id() else { return };
        let cm = self.claudemon.clone();
        tokio::spawn(async move {
            let _ = cm.resize(&sid, cols, rows).await;
        });
    }

    // ── workspace / tab selectors ─────────────────────────────────────────

    pub(super) fn open_agent_id(&self) -> Option<&str> {
        match &self.view {
            View::Agent { id } => Some(id),
            View::List => None,
        }
    }

    pub fn workspace(&self) -> Option<&Workspace> {
        self.workspaces.get(self.open_agent_id()?)
    }

    pub(super) fn workspace_mut(&mut self) -> Option<&mut Workspace> {
        let id = self.open_agent_id()?.to_string();
        self.workspaces.get_mut(&id)
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.workspace()?.active_tab()
    }

    /// The session the active tab shows (its content). May be a shell.
    pub(super) fn chat_session_id(&self) -> Option<String> {
        self.active_tab().map(|t| t.session_id.clone())
    }

    /// The session whose terminal is on screen: a Shell tab always, or a Claude
    /// tab in terminal mode.
    pub fn open_session_id(&self) -> Option<String> {
        let tab = self.active_tab()?;
        match tab.kind {
            TabKind::Shell => Some(tab.session_id.clone()),
            TabKind::Claude => {
                (self.chat_mode == ChatMode::Terminal).then(|| tab.session_id.clone())
            }
        }
    }

    pub fn term_attached(&self) -> bool {
        self.term_attached && self.open_session_id().is_some()
    }

    // ── sidebar (row 0 = Dashboard, rows 1.. = agents) ──────────────────────

    pub fn dashboard_selected(&self) -> bool {
        self.selected == 0
    }

    pub fn selected_agent(&self) -> Option<&Agent> {
        if self.selected == 0 {
            None
        } else {
            self.agents.get(self.selected - 1)
        }
    }

    /// The agent/session the active tab points at (may be a shell session).
    pub fn chat_agent(&self) -> Option<&Agent> {
        let sid = self.chat_session_id()?;
        self.agents.iter().find(|a| a.session_id == sid)
    }

    // ── async actions (fire-and-forget; results arrive as AppMsg) ───────────

    pub fn refresh(&self) {
        let cm = self.claudemon.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move { fetch_agents(&cm, &tx).await });
    }

    pub(super) fn load_transcript(&self, session_id: String) {
        let cm = self.claudemon.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move { fetch_transcript(&cm, &tx, session_id).await });
    }

    /// Run a control future, toast the outcome, then refresh the list (and the
    /// open transcript). The universal "do something to an agent" path; pass a
    /// future built from a cloned [`Claudemon`].
    pub(super) fn dispatch<F>(&self, ok_msg: &str, fut: F)
    where
        F: std::future::Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        let cm = self.claudemon.clone();
        let tx = self.tx.clone();
        let ok_msg = ok_msg.to_string();
        let reopen = self.chat_session_id();
        tokio::spawn(async move {
            match fut.await {
                Ok(_) => {
                    let _ = tx.send(AppMsg::Toast(ok_msg));
                    fetch_agents(&cm, &tx).await;
                    if let Some(sid) = reopen {
                        fetch_transcript(&cm, &tx, sid).await;
                    }
                }
                Err(e) => {
                    let _ = tx.send(AppMsg::Toast(format!("Failed: {e}")));
                }
            }
        });
    }

    // ── terminal lifecycle ────────────────────────────────────────────────

    /// Ensure a warm emulator + PTY stream exists for this session. Idempotent:
    /// re-opening an agent reuses the already-current terminal instead of
    /// re-attaching (which is what caused the blank re-open).
    pub(super) fn ensure_terminal(&mut self, session_id: String) {
        if self.terms.contains_key(&session_id) || self.no_terminal.contains(&session_id) {
            return;
        }
        self.terms.insert(session_id.clone(), Term::new());
        let cm = self.claudemon.clone();
        let pty_tx = self.pty_tx.clone();
        let msg_tx = self.tx.clone();
        let sid = session_id.clone();
        let handle = tokio::spawn(async move {
            use crate::claudemon::StreamEnd;
            let mut backoff = std::time::Duration::from_millis(300);
            loop {
                match cm.read_pty_stream(&sid, &pty_tx).await {
                    // No PTY for this session — tell the app to use the
                    // transcript and stop trying.
                    Ok(StreamEnd::NoPty) => {
                        let _ = msg_tx.send(AppMsg::TerminalUnavailable(sid.clone()));
                        return;
                    }
                    Ok(StreamEnd::Disconnected) | Err(_) => {}
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(std::time::Duration::from_secs(5));
            }
        });
        self.term_tasks.insert(session_id, handle.abort_handle());
    }

    /// Drop cached terminals (and their stream tasks) for sessions that no
    /// longer exist, so we don't leak connections for ended agents.
    pub(super) fn prune_terminals(&mut self, live: &HashSet<String>) {
        self.terms.retain(|sid, _| live.contains(sid));
        self.term_tasks.retain(|sid, handle| {
            let keep = live.contains(sid);
            if !keep {
                handle.abort();
            }
            keep
        });
    }

    /// Switch between the raw terminal and the parsed transcript. The terminal
    /// stays warm in the background either way.
    pub(super) fn toggle_chat_mode(&mut self) {
        let Some(sid) = self.chat_session_id() else { return };
        match self.chat_mode {
            ChatMode::Terminal => {
                self.chat_mode = ChatMode::Transcript;
                self.term_attached = false;
                self.chat_follow = true;
                self.load_transcript(sid);
            }
            ChatMode::Transcript => {
                if self.no_terminal.contains(&sid) {
                    self.set_toast("no terminal — external session (transcript only)");
                    return;
                }
                self.chat_mode = ChatMode::Terminal;
                self.ensure_terminal(sid);
            }
        }
    }

    /// Add a freshly-spawned shell as a tab under its agent and switch to it.
    pub(super) fn add_shell_tab(&mut self, agent_id: String, session_id: String) {
        if let Some(ws) = self.workspaces.get_mut(&agent_id) {
            let n = ws.tabs.iter().filter(|t| t.kind == TabKind::Shell).count() + 1;
            ws.tabs.push(Tab {
                title: format!("sh{n}"),
                session_id,
                kind: TabKind::Shell,
            });
            ws.active = ws.tabs.len() - 1;
            // Only switch into it if that agent is the one on screen.
            if self.open_agent_id() == Some(agent_id.as_str()) {
                self.enter_active_tab();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app() -> App {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let (ptx, _prx) = tokio::sync::mpsc::unbounded_channel();
        // Points at an unused port; the background stream tasks just fail and
        // retry harmlessly, which is fine for exercising app state.
        let cm = Claudemon::new("http://127.0.0.1:59999".into());
        App::new(cm, Vec::new(), Vec::new(), tx, ptx)
    }

    fn agent(id: &str) -> Agent {
        serde_json::from_value(serde_json::json!({ "session_id": id, "mode": "responding" })).unwrap()
    }

    #[tokio::test]
    async fn terminal_stays_warm_across_close_and_prunes_when_gone() {
        let mut app = test_app();
        app.set_agents(vec![agent("s1")]);
        app.selected = 1; // row 0 is the Dashboard; the agent is row 1

        app.open_agent();
        assert!(app.terms.contains_key("s1"), "opening creates a warm terminal");
        assert!(app.workspaces.contains_key("s1"), "opening creates a workspace");

        app.close_chat();
        assert!(
            app.terms.contains_key("s1"),
            "terminal stays warm after leaving the pane (so re-open is instant)"
        );

        // Agent disappears from the list → its terminal is pruned.
        app.set_agents(vec![]);
        assert!(!app.terms.contains_key("s1"), "terminal pruned once the session is gone");
    }

    #[tokio::test]
    async fn shell_tabs_add_switch_and_close() {
        let mut app = test_app();
        app.set_agents(vec![agent("s1")]);
        app.selected = 1;
        app.open_agent();
        assert_eq!(app.workspace().unwrap().tabs.len(), 1, "starts with the claude tab");

        app.add_shell_tab("s1".into(), "sh-1".into());
        let ws = app.workspace().unwrap();
        assert_eq!(ws.tabs.len(), 2);
        assert_eq!(ws.active, 1, "switches to the new shell tab");
        assert_eq!(app.chat_session_id().as_deref(), Some("sh-1"));

        app.tab_prev();
        assert_eq!(app.chat_session_id().as_deref(), Some("s1"), "back to claude tab");
        app.tab_next();
        assert_eq!(app.chat_session_id().as_deref(), Some("sh-1"));

        app.close_tab(); // closes the active shell tab
        assert_eq!(app.workspace().unwrap().tabs.len(), 1);
        assert_eq!(app.chat_session_id().as_deref(), Some("s1"));
    }
}
