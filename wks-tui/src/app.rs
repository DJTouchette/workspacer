//! Application state and input handling.
//!
//! The app owns no socket directly — it holds a [`Claudemon`] client and an
//! `AppMsg` sender, and every action that needs the network spawns a task that
//! calls claudemon and posts the outcome back as an [`AppMsg`]. The main loop
//! (in `main.rs`) drives `draw` + `handle_key` + `apply_msg`. This keeps
//! rendering synchronous and the network fully async, fire-and-refresh.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc::UnboundedSender;

use crate::claudemon::{Claudemon, PtyChunk};
use crate::library::LibraryItem;
use crate::profiles::{self, Profile};
use crate::terminal::Term;
use crate::types::{turns_from_transcript, Agent, Turn};

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
    fn new(items: Vec<PaletteItem>) -> Self {
        let filtered = (0..items.len()).collect();
        Palette { query: String::new(), items, filtered, selected: 0 }
    }

    fn refilter(&mut self) {
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

    fn chosen(&self) -> Option<&PaletteItem> {
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
    claudemon: Claudemon,
    tx: UnboundedSender<AppMsg>,
    /// Sender the PTY stream task pushes chunks into; the main loop drains it
    /// and calls [`App::feed_pty`].
    pty_tx: UnboundedSender<PtyChunk>,

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
    term_tasks: HashMap<String, tokio::task::AbortHandle>,
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

    toast: Option<(String, Instant)>,
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

    fn set_toast(&mut self, msg: impl Into<String>) {
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

    fn set_agents(&mut self, mut list: Vec<Agent>) {
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

    fn open_agent_id(&self) -> Option<&str> {
        match &self.view {
            View::Agent { id } => Some(id),
            View::List => None,
        }
    }

    pub fn workspace(&self) -> Option<&Workspace> {
        self.workspaces.get(self.open_agent_id()?)
    }

    fn workspace_mut(&mut self) -> Option<&mut Workspace> {
        let id = self.open_agent_id()?.to_string();
        self.workspaces.get_mut(&id)
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.workspace()?.active_tab()
    }

    /// The session the active tab shows (its content). May be a shell.
    fn chat_session_id(&self) -> Option<String> {
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

    fn load_transcript(&self, session_id: String) {
        let cm = self.claudemon.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move { fetch_transcript(&cm, &tx, session_id).await });
    }

    /// Run a control future, toast the outcome, then refresh the list (and the
    /// open transcript). The universal "do something to an agent" path; pass a
    /// future built from a cloned [`Claudemon`].
    fn dispatch<F>(&self, ok_msg: &str, fut: F)
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

    // ── input handling ────────────────────────────────────────────────────

    pub fn handle_key(&mut self, key: KeyEvent) {
        // Modals capture everything while open.
        if self.spawn_form.is_some() {
            self.handle_spawn_key(key);
            return;
        }
        if self.palette.is_some() {
            self.handle_palette_key(key);
            return;
        }
        // When attached to the live terminal, every key goes to Claude (so
        // Ctrl-C interrupts the agent, not the TUI). Ctrl-] detaches.
        if self.term_attached() {
            self.handle_terminal_key(key);
            return;
        }
        // Otherwise Ctrl-C quits.
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        // Ctrl-K opens the command palette from anywhere (when not attached).
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('k') {
            self.open_palette();
            return;
        }
        match &self.view {
            View::List => self.handle_list_key(key),
            View::Agent { .. } if self.insert_mode => self.handle_insert_key(key),
            View::Agent { .. } => self.handle_agent_key(key),
        }
    }

    /// Forward a keystroke to the PTY, or detach on Ctrl-].
    fn handle_terminal_key(&mut self, key: KeyEvent) {
        if crate::terminal::is_detach(&key) {
            self.term_attached = false;
            return;
        }
        let Some(sid) = self.open_session_id() else { return };
        let Some(bytes) = crate::terminal::encode_key(&key) else { return };
        let cm = self.claudemon.clone();
        tokio::spawn(async move {
            let _ = cm.input_bytes(&sid, &bytes).await;
        });
    }

    fn handle_list_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('j') | KeyCode::Down => self.select_next(),
            KeyCode::Char('k') | KeyCode::Up => self.select_prev(),
            KeyCode::Char('g') => self.selected = 0,
            KeyCode::Char('G') => self.selected = self.agents.len(),
            KeyCode::Char('m') => self.jump_to_attention(),
            KeyCode::Char('c') => self.open_spawn(),
            KeyCode::Char('r') => self.refresh(),
            // Enter on the Dashboard row is a no-op (it's a live preview); on an
            // agent it opens that agent's tabs.
            KeyCode::Enter | KeyCode::Char('l') => self.open_agent(),
            // Open the selected agent straight into a fresh terminal tab.
            KeyCode::Char('T') => {
                if self.selected_agent().is_some() {
                    self.open_agent();
                    self.new_terminal_tab();
                }
            }
            KeyCode::Char('y') => self.approve("yes", "Approved"),
            KeyCode::Char('n') => self.approve("no", "Denied"),
            KeyCode::Char('a') => self.approve("always", "Approved (always)"),
            KeyCode::Char(c @ '1'..='9') => self.answer_option(c),
            _ => {}
        }
    }

    fn handle_agent_key(&mut self, key: KeyEvent) {
        // Keys common to every tab/mode.
        match key.code {
            KeyCode::Char('q') => {
                self.should_quit = true;
                return;
            }
            KeyCode::Esc | KeyCode::Char('h') => {
                self.close_chat();
                return;
            }
            KeyCode::Char('c') => {
                self.open_spawn();
                return;
            }
            // Tab management.
            KeyCode::Char(']') | KeyCode::Tab => {
                self.tab_next();
                return;
            }
            KeyCode::Char('[') | KeyCode::BackTab => {
                self.tab_prev();
                return;
            }
            KeyCode::Char('T') => {
                self.new_terminal_tab();
                return;
            }
            KeyCode::Char('w') => {
                self.close_tab();
                return;
            }
            _ => {}
        }
        // Shell tabs are always raw terminals — no transcript toggle.
        let on_shell = matches!(self.active_tab().map(|t| t.kind), Some(TabKind::Shell));
        if key.code == KeyCode::Char('t') && !on_shell {
            self.toggle_chat_mode();
            return;
        }
        match self.chat_mode {
            _ if on_shell => match key.code {
                KeyCode::Char('i') | KeyCode::Enter => self.term_attached = true,
                KeyCode::Char('x') => self.signal("SIGINT", "Interrupted"),
                KeyCode::Char('X') => self.signal("SIGTERM", "Stopped"),
                _ => {}
            },
            ChatMode::Terminal => match key.code {
                // Attach: hand the keyboard to Claude's terminal.
                KeyCode::Char('i') | KeyCode::Enter => {
                    if self.open_session_id().is_some() {
                        self.term_attached = true;
                    }
                }
                KeyCode::Char('x') => self.signal("SIGINT", "Interrupted"),
                KeyCode::Char('X') => self.signal("SIGTERM", "Stopped"),
                _ => {}
            },
            ChatMode::Transcript => match key.code {
                KeyCode::Char('i') => self.insert_mode = true,
                KeyCode::Char('j') | KeyCode::Down => {
                    self.chat_follow = false;
                    self.chat_scroll = self.chat_scroll.saturating_add(1);
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.chat_follow = false;
                    self.chat_scroll = self.chat_scroll.saturating_sub(1);
                }
                KeyCode::Char('r') => self.on_changed(),
                KeyCode::Char('y') => self.approve("yes", "Approved"),
                KeyCode::Char('n') => self.approve("no", "Denied"),
                KeyCode::Char('a') => self.approve("always", "Approved (always)"),
                KeyCode::Char('x') => self.signal("SIGINT", "Interrupted"),
                KeyCode::Char('X') => self.signal("SIGTERM", "Stopped"),
                KeyCode::Char(c @ '1'..='9') => self.answer_option(c),
                _ => {}
            },
        }
    }

    // ── terminal lifecycle ────────────────────────────────────────────────

    /// Ensure a warm emulator + PTY stream exists for this session. Idempotent:
    /// re-opening an agent reuses the already-current terminal instead of
    /// re-attaching (which is what caused the blank re-open).
    fn ensure_terminal(&mut self, session_id: String) {
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
            let mut backoff = Duration::from_millis(300);
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
                backoff = (backoff * 2).min(Duration::from_secs(5));
            }
        });
        self.term_tasks.insert(session_id, handle.abort_handle());
    }

    /// Drop cached terminals (and their stream tasks) for sessions that no
    /// longer exist, so we don't leak connections for ended agents.
    fn prune_terminals(&mut self, live: &HashSet<String>) {
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
    fn toggle_chat_mode(&mut self) {
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

    fn handle_spawn_key(&mut self, key: KeyEvent) {
        let n = self.profiles.len();
        let Some(form) = self.spawn_form.as_mut() else { return };
        match key.code {
            KeyCode::Esc => self.spawn_form = None,
            KeyCode::Enter => self.submit_spawn(),
            // Shell-style path completion on the cwd field.
            KeyCode::Tab => complete_path(form),
            // Cycle the chosen profile.
            KeyCode::Down | KeyCode::Right => {
                if n > 0 {
                    form.profile_idx = (form.profile_idx + 1) % n;
                }
            }
            KeyCode::Up | KeyCode::Left => {
                if n > 0 {
                    form.profile_idx = (form.profile_idx + n - 1) % n;
                }
            }
            KeyCode::Backspace => {
                form.cwd.pop();
                form.completions.clear();
            }
            KeyCode::Char(c) => {
                form.cwd.push(c);
                form.completions.clear();
            }
            _ => {}
        }
    }

    // ── command palette (Ctrl-K) ──────────────────────────────────────────

    fn open_palette(&mut self) {
        let mut items = vec![
            PaletteItem { label: "New agent".into(), hint: "spawn".into(), action: PaletteAction::NewAgent },
            PaletteItem { label: "New terminal".into(), hint: "shell tab".into(), action: PaletteAction::NewTerminal },
            PaletteItem { label: "Dashboard".into(), hint: "overview".into(), action: PaletteAction::Dashboard },
        ];
        // Jump to a live agent.
        for a in &self.agents {
            items.push(PaletteItem {
                label: format!("Go to {}", a.short_cwd()),
                hint: a.state().to_string(),
                action: PaletteAction::OpenAgent(a.session_id.clone()),
            });
        }
        // Library items — run in a new agent, or insert into the focused one.
        for item in &self.library {
            items.push(PaletteItem {
                label: format!("Run \"{}\" in new agent", item.title),
                hint: item.kind.clone(),
                action: PaletteAction::SpawnWithPrompt(item.body.clone()),
            });
            items.push(PaletteItem {
                label: format!("Insert \"{}\"  ({})", item.title, item.kind),
                hint: item.description.clone().unwrap_or_default(),
                action: PaletteAction::Insert(item.body.clone()),
            });
        }
        self.palette = Some(Palette::new(items));
    }

    fn handle_palette_key(&mut self, key: KeyEvent) {
        let Some(p) = self.palette.as_mut() else { return };
        match key.code {
            KeyCode::Esc => self.palette = None,
            KeyCode::Enter => {
                let action = p.chosen().map(|it| it.action.clone());
                self.palette = None;
                if let Some(action) = action {
                    self.run_palette_action(action);
                }
            }
            KeyCode::Down => {
                if !p.filtered.is_empty() {
                    p.selected = (p.selected + 1).min(p.filtered.len() - 1);
                }
            }
            KeyCode::Up => p.selected = p.selected.saturating_sub(1),
            KeyCode::Backspace => {
                p.query.pop();
                p.refilter();
            }
            KeyCode::Char(c) => {
                p.query.push(c);
                p.refilter();
            }
            _ => {}
        }
    }

    fn run_palette_action(&mut self, action: PaletteAction) {
        match action {
            PaletteAction::NewAgent => self.open_spawn(),
            PaletteAction::NewTerminal => {
                if self.open_agent_id().is_some() {
                    self.new_terminal_tab();
                } else if self.selected_agent().is_some() {
                    self.open_agent();
                    self.new_terminal_tab();
                } else {
                    self.set_toast("select an agent first");
                }
            }
            PaletteAction::Dashboard => {
                self.view = View::List;
                self.selected = 0;
            }
            PaletteAction::OpenAgent(sid) => {
                if let Some(i) = self.agents.iter().position(|a| a.session_id == sid) {
                    self.selected = i + 1;
                    self.open_agent();
                }
            }
            PaletteAction::Insert(body) => {
                let Some(sid) = self.open_session_id() else {
                    self.set_toast("open an agent's terminal to insert");
                    return;
                };
                let cm = self.claudemon.clone();
                let bytes = bracketed_paste(&body);
                self.set_toast("Inserted");
                tokio::spawn(async move {
                    let _ = cm.input_bytes(&sid, &bytes).await;
                });
            }
            PaletteAction::SpawnWithPrompt(body) => self.open_spawn_with_prompt(body),
        }
    }

    fn handle_insert_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.insert_mode = false,
            KeyCode::Enter => self.send_input(),
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char(c) => self.input.push(c),
            _ => {}
        }
    }

    // ── action helpers ────────────────────────────────────────────────────

    fn select_next(&mut self) {
        // Rows are [Dashboard, ..agents], so the max index is agents.len().
        self.selected = (self.selected + 1).min(self.agents.len());
    }

    fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Move selection to the next agent (after the current one, wrapping) that
    /// needs the user — the `m` "jump to attention" key.
    fn jump_to_attention(&mut self) {
        let n = self.agents.len();
        if n == 0 {
            return;
        }
        // Current agent index, or "before the first" when the Dashboard is selected.
        let cur = if self.selected == 0 { n - 1 } else { self.selected - 1 };
        for offset in 1..=n {
            let i = (cur + offset) % n;
            if self.agents[i].is_waiting() {
                self.selected = i + 1;
                return;
            }
        }
        self.set_toast("Nothing waiting");
    }

    /// Open the selected agent into its tab view (creating its workspace with a
    /// single Claude tab the first time). The Dashboard row doesn't "open" — it's
    /// a live preview, so Enter there is a no-op.
    fn open_agent(&mut self) {
        let Some(agent) = self.selected_agent() else { return };
        let id = agent.session_id.clone();
        self.view = View::Agent { id: id.clone() };
        self.workspaces.entry(id.clone()).or_insert_with(|| Workspace {
            tabs: vec![Tab { title: "claude".into(), session_id: id.clone(), kind: TabKind::Claude }],
            active: 0,
        });
        self.enter_active_tab();
    }

    /// Set up rendering for whatever the active tab points at: warm its terminal
    /// (or fall back to transcript for no-PTY sessions).
    fn enter_active_tab(&mut self) {
        self.turns.clear();
        self.chat_scroll = 0;
        self.chat_follow = true;
        self.insert_mode = false;
        self.term_attached = false;
        let Some(tab) = self.active_tab().cloned() else { return };
        if tab.kind == TabKind::Claude && self.no_terminal.contains(&tab.session_id) {
            self.chat_mode = ChatMode::Transcript;
            self.load_transcript(tab.session_id);
        } else {
            self.chat_mode = ChatMode::Terminal;
            self.ensure_terminal(tab.session_id);
        }
    }

    fn tab_next(&mut self) {
        if let Some(ws) = self.workspace_mut() {
            if !ws.tabs.is_empty() {
                ws.active = (ws.active + 1) % ws.tabs.len();
            }
        }
        self.enter_active_tab();
    }

    fn tab_prev(&mut self) {
        if let Some(ws) = self.workspace_mut() {
            let n = ws.tabs.len();
            if n > 0 {
                ws.active = (ws.active + n - 1) % n;
            }
        }
        self.enter_active_tab();
    }

    /// Open a new shell tab: spawn `$SHELL` via claudemon in the agent's cwd (so
    /// it's a real PTY we can stream, and shows in the system-wide list). The
    /// session id comes back async and is added as a tab then.
    fn new_terminal_tab(&mut self) {
        let Some(id) = self.open_agent_id().map(|s| s.to_string()) else { return };
        let cwd = self
            .chat_agent()
            .map(|a| a.cwd_str().to_string())
            .filter(|c| !c.is_empty())
            .or_else(|| std::env::current_dir().ok().map(|p| p.display().to_string()))
            .unwrap_or_else(|| "/".into());
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
        let cm = self.claudemon.clone();
        let tx = self.tx.clone();
        self.set_toast("Opening terminal…");
        tokio::spawn(async move {
            match cm.spawn(vec![shell], cwd, serde_json::Map::new()).await {
                Ok(sid) => {
                    let _ = tx.send(AppMsg::ShellSpawned { agent_id: id, session_id: sid });
                }
                Err(e) => {
                    let _ = tx.send(AppMsg::Toast(format!("Terminal failed: {e}")));
                }
            }
        });
    }

    /// Add a freshly-spawned shell as a tab under its agent and switch to it.
    fn add_shell_tab(&mut self, agent_id: String, session_id: String) {
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

    /// Close the active tab. Closing the primary Claude tab leaves the agent
    /// (back to the list); closing a shell tab stops that shell.
    fn close_tab(&mut self) {
        let Some(ws) = self.workspace() else { return };
        let Some(tab) = ws.active_tab().cloned() else { return };
        if tab.kind == TabKind::Claude {
            self.close_chat();
            return;
        }
        // Shell: stop the process and drop the tab.
        let sid = tab.session_id.clone();
        let cm = self.claudemon.clone();
        tokio::spawn(async move {
            let _ = cm.signal(&sid, "SIGTERM").await;
        });
        if let Some(ws) = self.workspace_mut() {
            let idx = ws.active;
            ws.tabs.remove(idx);
            if ws.active >= ws.tabs.len() {
                ws.active = ws.tabs.len().saturating_sub(1);
            }
        }
        self.enter_active_tab();
    }

    /// Open the spawn modal, prefilling the cwd with where the TUI was launched.
    fn open_spawn(&mut self) {
        self.open_spawn_inner(None);
    }

    /// Open the spawn modal carrying a prompt to seed into the new agent.
    fn open_spawn_with_prompt(&mut self, prompt: String) {
        self.open_spawn_inner(Some(prompt));
    }

    fn open_spawn_inner(&mut self, initial_prompt: Option<String>) {
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        self.spawn_form = Some(SpawnForm {
            cwd,
            profile_idx: 0,
            completions: Vec::new(),
            initial_prompt,
        });
    }

    /// Spawn a Claude session in the chosen cwd with the chosen profile, via
    /// claudemon's REST API. The new agent surfaces in the sidebar on the next
    /// state-change event (claudemon emits one once Claude starts up).
    fn submit_spawn(&mut self) {
        let Some(form) = self.spawn_form.clone() else { return };
        let cwd = profiles::normalize_cwd(&form.cwd);
        if cwd.is_empty() {
            self.set_toast("working directory required");
            return;
        }
        let Some(profile) = self.profiles.get(form.profile_idx).cloned() else {
            self.set_toast("no profile selected");
            return;
        };
        self.spawn_form = None;

        let argv = profiles::build_argv(&profile, None, false);
        let env = profiles::build_env(&profile);
        let initial_prompt = form.initial_prompt.clone();
        let claudemon = self.claudemon.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let sid = match claudemon.spawn(argv, cwd, env).await {
                Ok(sid) => {
                    let _ = tx.send(AppMsg::Toast("Spawned agent".into()));
                    fetch_agents(&claudemon, &tx).await;
                    sid
                }
                Err(e) => {
                    let _ = tx.send(AppMsg::Toast(format!("Spawn failed: {e}")));
                    return;
                }
            };
            if let Some(prompt) = initial_prompt {
                seed_prompt(&claudemon, &tx, &sid, &prompt).await;
            }
        });
    }

    fn close_chat(&mut self) {
        // Leave the terminal warm in the background so coming back is instant.
        self.view = View::List;
        self.chat_mode = ChatMode::Terminal;
        self.term_attached = false;
        self.insert_mode = false;
        self.input.clear();
        self.turns.clear();
    }

    /// The agent a key action targets: the chat's agent when in chat, else the
    /// selected sidebar agent.
    fn target_session(&self) -> Option<String> {
        match &self.view {
            View::Agent { .. } => self.chat_session_id(),
            View::List => self.selected_agent().map(|a| a.session_id.clone()),
        }
    }

    fn target_agent(&self) -> Option<&Agent> {
        match &self.view {
            View::Agent { .. } => self.chat_agent(),
            View::List => self.selected_agent(),
        }
    }

    fn approve(&mut self, decision: &str, ok: &str) {
        let Some(agent) = self.target_agent() else { return };
        if agent.approval().is_none() {
            return;
        }
        let sid = agent.session_id.clone();
        let cm = self.claudemon.clone();
        let decision = decision.to_string();
        self.dispatch(ok, async move { cm.approve(&sid, &decision, None).await });
    }

    /// Answer the first pending question with the option at 1-based key `c`.
    fn answer_option(&mut self, c: char) {
        let Some(agent) = self.target_agent() else { return };
        if !agent.has_question() {
            return;
        }
        let option = (c as u8 - b'0') as u64; // '1'..='9' → 1..=9
        let sid = agent.session_id.clone();
        let cm = self.claudemon.clone();
        self.dispatch("Answered", async move { cm.answer_option(&sid, option).await });
    }

    fn signal(&mut self, signal: &str, ok: &str) {
        let Some(sid) = self.target_session() else { return };
        let cm = self.claudemon.clone();
        let signal = signal.to_string();
        self.dispatch(ok, async move { cm.signal(&sid, &signal).await });
    }

    /// Send the composer's contents — as an answer if the agent is on a
    /// question, otherwise as a chat message. Mirrors the `/remote` heuristic.
    fn send_input(&mut self) {
        let text = self.input.trim().to_string();
        if text.is_empty() {
            return;
        }
        let Some(agent) = self.target_agent() else { return };
        let sid = agent.session_id.clone();
        let answering = agent.has_question();
        let cm = self.claudemon.clone();
        self.dispatch("Sent", async move {
            if answering {
                cm.answer_text(&sid, &text).await
            } else {
                cm.message(&sid, &text).await
            }
        });
        self.input.clear();
    }
}

// ── path completion ─────────────────────────────────────────────────────────

/// Shell-style directory completion for the spawn modal's cwd field. Completes
/// the trailing component to the longest common prefix of matching directories;
/// fills a single match fully (with a trailing `/`), or records the candidates
/// for display when ambiguous. Only the newly-resolved characters are appended,
/// so the user's literal text (including a leading `~`) is preserved.
fn complete_path(form: &mut SpawnForm) {
    let input = form.cwd.clone();
    let (dir_part, partial) = match input.rfind('/') {
        Some(i) => (input[..=i].to_string(), input[i + 1..].to_string()),
        None => (String::new(), input.clone()),
    };

    let real_dir: PathBuf = if dir_part.is_empty() {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    } else {
        PathBuf::from(profiles::expand_tilde(&dir_part))
    };

    // Hidden entries only surface when the user has started typing a dot,
    // matching how shells behave.
    let want_hidden = partial.starts_with('.');
    let mut names: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&real_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with(&partial) || (!want_hidden && name.starts_with('.')) {
                continue;
            }
            if entry.path().is_dir() {
                names.push(name);
            }
        }
    }
    names.sort();

    if names.is_empty() {
        return;
    }
    let prefix = longest_common_prefix(&names);
    if prefix.len() > partial.len() {
        form.cwd.push_str(&prefix[partial.len()..]);
    }
    if names.len() == 1 {
        form.cwd.push('/');
        form.completions.clear();
    } else {
        // Multiple matches: leave them on screen so the user can keep typing.
        form.completions = names;
    }
}

/// The longest common (character-wise) prefix shared by every string.
fn longest_common_prefix(names: &[String]) -> String {
    let mut iter = names.iter();
    let Some(first) = iter.next() else { return String::new() };
    let mut prefix = first.clone();
    for s in iter {
        let common: String = prefix
            .chars()
            .zip(s.chars())
            .take_while(|(a, b)| a == b)
            .map(|(a, _)| a)
            .collect();
        prefix = common;
        if prefix.is_empty() {
            break;
        }
    }
    prefix
}

// ── free async helpers (shared by methods and spawned tasks) ────────────────

async fn fetch_agents(cm: &Claudemon, tx: &UnboundedSender<AppMsg>) {
    let Ok(mut list) = cm.list().await else { return };

    // Usage (model / context / cost) isn't in the /sessions payload — the
    // desktop app derives it per-session from the transcript. Mirror that:
    // fetch each live session's transcript concurrently and fold it in. Stopped
    // sessions are skipped (no point re-parsing a finished log every refresh).
    let futs = list
        .iter()
        .filter(|a| a.state() != "stopped")
        .map(|a| {
            let cm = cm.clone();
            let id = a.session_id.clone();
            async move {
                let usage = cm.transcript(&id).await.ok().and_then(|t| crate::usage::from_transcript(&t));
                (id, usage)
            }
        });
    let usages: std::collections::HashMap<String, Option<crate::usage::Usage>> =
        futures_util::future::join_all(futs).await.into_iter().collect();
    for a in &mut list {
        if let Some(u) = usages.get(&a.session_id) {
            a.usage = u.clone();
        }
    }

    let _ = tx.send(AppMsg::Agents(list));
}

async fn fetch_transcript(cm: &Claudemon, tx: &UnboundedSender<AppMsg>, session_id: String) {
    if let Ok(v) = cm.transcript(&session_id).await {
        let turns = turns_from_transcript(&v);
        let _ = tx.send(AppMsg::Transcript { session_id, turns });
    }
}

/// Wrap text in bracketed-paste markers so a multi-line prompt is inserted into
/// Claude's input as one paste (newlines stay newlines instead of submitting).
fn bracketed_paste(text: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(text.len() + 12);
    v.extend_from_slice(b"\x1b[200~");
    v.extend_from_slice(text.as_bytes());
    v.extend_from_slice(b"\x1b[201~");
    v
}

/// Seed a prompt into a freshly-spawned agent: wait until it reaches its input
/// prompt (claudemon reports mode `input`), then paste — without submitting, so
/// the user reviews and presses enter.
async fn seed_prompt(cm: &Claudemon, tx: &UnboundedSender<AppMsg>, sid: &str, prompt: &str) {
    for _ in 0..40 {
        if cm.session_mode(sid).await.as_deref() == Some("input") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    let _ = cm.input_bytes(sid, &bracketed_paste(prompt)).await;
    let _ = tx.send(AppMsg::Toast("Prompt seeded — open the agent and press enter".into()));
    fetch_agents(cm, tx).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn form(cwd: &str) -> SpawnForm {
        SpawnForm {
            cwd: cwd.into(),
            profile_idx: 0,
            completions: Vec::new(),
            initial_prompt: None,
        }
    }

    #[test]
    fn lcp() {
        assert_eq!(
            longest_common_prefix(&["project-a".into(), "project-b".into()]),
            "project-"
        );
        assert_eq!(longest_common_prefix(&["abc".into()]), "abc");
        assert_eq!(longest_common_prefix(&["a".into(), "b".into()]), "");
        assert_eq!(longest_common_prefix(&[]), "");
    }

    #[test]
    fn completes_unique_directory_with_trailing_slash() {
        let base = std::env::temp_dir().join("wkstui_unique");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("alpha")).unwrap();

        let mut f = form(&format!("{}/al", base.display()));
        complete_path(&mut f);

        assert_eq!(f.cwd, format!("{}/alpha/", base.display()));
        assert!(f.completions.is_empty());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn completes_common_prefix_and_records_candidates() {
        let base = std::env::temp_dir().join("wkstui_multi");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("proj-a")).unwrap();
        fs::create_dir_all(base.join("proj-b")).unwrap();
        fs::create_dir_all(base.join("other")).unwrap();

        let mut f = form(&format!("{}/pr", base.display()));
        complete_path(&mut f);

        assert_eq!(f.cwd, format!("{}/proj-", base.display()));
        assert_eq!(f.completions, vec!["proj-a".to_string(), "proj-b".to_string()]);
        let _ = fs::remove_dir_all(&base);
    }

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

    #[test]
    fn hides_dotfiles_unless_dot_typed() {
        let base = std::env::temp_dir().join("wkstui_hidden");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join(".secret")).unwrap();
        fs::create_dir_all(base.join("visible")).unwrap();

        // No leading dot in the partial → hidden dir excluded, "visible" completes.
        let mut f = form(&format!("{}/", base.display()));
        complete_path(&mut f);
        assert_eq!(f.cwd, format!("{}/visible/", base.display()));

        // Leading dot → the hidden dir is the only match.
        let mut f = form(&format!("{}/.", base.display()));
        complete_path(&mut f);
        assert_eq!(f.cwd, format!("{}/.secret/", base.display()));
        let _ = fs::remove_dir_all(&base);
    }
}
