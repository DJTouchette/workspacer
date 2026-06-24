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
use crate::config::Config;
use crate::keys::{Chord, Keymap};
use crate::library::LibraryItem;
use crate::profiles::Profile;
use crate::terminal::Term;
use crate::theme::Theme;
use crate::types::{Agent, FileStatus, StatusLine, Turn};

use tasks::{fetch_agents, fetch_git_diff, fetch_git_status, fetch_transcript};

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
    /// Git status for the review pane's work tree.
    GitStatus { cwd: String, branch: Option<String>, files: Vec<FileStatus> },
    /// A file's unified diff for the review pane.
    GitDiff { cwd: String, path: String, staged: bool, diff: String },
    /// Lightweight branch + changed-file count for the open agent's inspector.
    GitSummary { cwd: String, branch: Option<String>, changed: usize },
    /// A git read failed for a work tree (e.g. not a repo) — shown in review.
    GitError { cwd: String, message: String },
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

/// How tiled panes are arranged when more than one agent is on screen.
/// `Columns` = side by side (vim `Ctrl-w v`); `Rows` = stacked (`Ctrl-w s`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDir {
    Columns,
    Rows,
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

/// State of the rename overlay: a single text field editing the custom display
/// name for an agent's cwd.
pub struct RenameForm {
    pub cwd: String,
    pub input: String,
}

/// State of the notes scratchpad overlay (a per-cwd markdown note).
pub struct NotesState {
    pub cwd: String,
    pub text: String,
    /// True while typing (append-style editing); false is read/scroll mode.
    pub editing: bool,
    pub scroll: u16,
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

/// State of the git review pane (mirrors the desktop Review pane): the work
/// tree's branch + changed files on the left, the selected file's unified diff
/// on the right. Opened over an agent and keyed by that agent's cwd.
pub struct ReviewState {
    pub cwd: String,
    pub branch: Option<String>,
    pub files: Vec<FileStatus>,
    pub selected: usize,
    /// Raw unified diff for the selected file in the current staged/unstaged view.
    pub diff: String,
    pub diff_scroll: u16,
    /// false = unstaged (work tree) changes; true = staged (index) changes.
    pub staged_view: bool,
    /// When `Some`, the user is composing a commit message.
    pub commit_msg: Option<String>,
    /// Set when the status fetch failed — e.g. the cwd isn't a git work tree —
    /// so the pane says so instead of looking like a clean repo.
    pub error: Option<String>,
}

impl ReviewState {
    fn new(cwd: String) -> Self {
        ReviewState {
            cwd,
            branch: None,
            files: Vec::new(),
            selected: 0,
            diff: String::new(),
            diff_scroll: 0,
            staged_view: false,
            commit_msg: None,
            error: None,
        }
    }

    pub fn selected_file(&self) -> Option<&FileStatus> {
        self.files.get(self.selected)
    }
}

pub struct App {
    pub(super) claudemon: Claudemon,
    pub(super) tx: UnboundedSender<AppMsg>,
    /// Sender the PTY stream task pushes chunks into; the main loop drains it
    /// and calls [`App::feed_pty`].
    pub(super) pty_tx: UnboundedSender<PtyChunk>,

    pub profiles: Vec<Profile>,
    pub library: Vec<LibraryItem>,
    /// Resolved color theme; every renderer references it instead of literals.
    pub theme: Theme,
    /// Resolved keybindings; `input.rs` dispatches every key through this.
    pub keymap: Keymap,
    /// Whether the keybinding/help overlay is open.
    pub help: bool,
    pub spawn_form: Option<SpawnForm>,
    pub palette: Option<Palette>,
    /// The git review pane, when open (a modal over the agent view).
    pub review: Option<ReviewState>,
    /// The rename overlay, when open.
    pub rename: Option<RenameForm>,
    /// Custom per-cwd display names (persisted); empty when none set.
    pub names: HashMap<String, String>,
    /// The notes scratchpad overlay, when open.
    pub notes_view: Option<NotesState>,
    /// Per-cwd scratchpad text (persisted).
    pub notes: HashMap<String, String>,
    /// Inspector cache: cwd → (branch, changed-file count), for the open agent.
    pub git_summary: HashMap<String, (Option<String>, usize)>,

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
    /// Pending `(cols, rows)` per session id, pushed to claudemon after the next
    /// draw whenever a (focused or watch) pane resized that session's emulator.
    pub term_resizes: HashMap<String, (u16, u16)>,

    /// Agents tiled in the content area (their session ids). 0/1 entries render
    /// as a single pane (today's behavior); 2+ tile the content. `tile_focus`
    /// is the interactive pane — it stays in sync with `View::Agent { id }`; the
    /// others render their live terminal read-only. See [`SplitDir`].
    pub tiles: Vec<String>,
    pub tile_focus: usize,
    pub split_dir: SplitDir,

    /// Harpoon-style pinned agents (session ids), in slot order. `<leader>1..9`
    /// teleports to a slot; the sidebar shows each pin's number.
    pub harpoon: Vec<String>,
    /// The agent focused just before the current one — vim's alternate buffer
    /// (`Ctrl-^`).
    pub prev_focus: Option<String>,
    /// Visited-agent history for the jumplist (`Ctrl-o` / `<leader>i`);
    /// `jump_idx` is the current position within it.
    pub jumplist: Vec<String>,
    pub jump_idx: usize,

    /// Session ids seen in a non-stopped mode this run. Used to hide hydrated
    /// "orphan" sessions (stopped history claudemon replays on restart) while
    /// keeping sessions that stopped while we watched them.
    pub seen_live: HashSet<String>,
    /// When true, the sidebar shows every session including stopped history.
    pub show_all_sessions: bool,
    /// How many stopped orphans the last [`set_agents`] hid (for the title).
    pub hidden_count: usize,

    /// Full live session set (orphan-filtered) — the source of truth for
    /// lifecycle and by-id lookups. `agents` is the `/`-filtered projection of
    /// this that the sidebar and selection use.
    pub all_agents: Vec<Agent>,
    /// Active sidebar filter query (`/`); `None` means no filter. `filter_editing`
    /// is true while the query is being typed.
    pub filter: Option<String>,
    pub filter_editing: bool,

    /// Agents, in a stable order: existing rows stay put across polls and new
    /// sessions are appended at the end (matches the Electron app).
    pub agents: Vec<Agent>,
    /// Claude's authoritative statusLine per session (context%/cost/model/rate
    /// limits), streamed live; preferred over transcript usage when present.
    pub status_lines: HashMap<String, StatusLine>,
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

    /// Chords typed so far toward a multi-key binding (e.g. after the leader).
    /// Empty when no sequence is in flight; drives the which-key popup.
    pub pending_keys: Vec<Chord>,

    pub(super) toast: Option<(String, Instant)>,
}

impl App {
    pub fn new(
        claudemon: Claudemon,
        profiles: Vec<Profile>,
        library: Vec<LibraryItem>,
        config: Config,
        tx: UnboundedSender<AppMsg>,
        pty_tx: UnboundedSender<PtyChunk>,
    ) -> Self {
        Self {
            claudemon,
            tx,
            pty_tx,
            profiles,
            library,
            theme: config.theme,
            keymap: config.keymap,
            help: false,
            spawn_form: None,
            palette: None,
            review: None,
            rename: None,
            names: crate::names::load(),
            notes_view: None,
            notes: crate::notes::load(),
            git_summary: HashMap::new(),
            connected: false,
            should_quit: false,
            chat_mode: ChatMode::Terminal,
            terms: HashMap::new(),
            term_tasks: HashMap::new(),
            no_terminal: HashSet::new(),
            workspaces: HashMap::new(),
            term_attached: false,
            term_resizes: HashMap::new(),
            tiles: Vec::new(),
            tile_focus: 0,
            split_dir: SplitDir::Columns,
            harpoon: Vec::new(),
            prev_focus: None,
            jumplist: Vec::new(),
            jump_idx: 0,
            seen_live: HashSet::new(),
            show_all_sessions: false,
            hidden_count: 0,
            all_agents: Vec::new(),
            filter: None,
            filter_editing: false,
            agents: Vec::new(),
            status_lines: HashMap::new(),
            selected: 0,
            view: View::List,
            turns: Vec::new(),
            chat_scroll: 0,
            chat_follow: false,
            insert_mode: false,
            input: String::new(),
            pending_keys: Vec::new(),
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
            AppMsg::GitStatus { cwd, branch, files } => self.apply_git_status(cwd, branch, files),
            AppMsg::GitDiff { cwd, path, staged, diff } => {
                self.apply_git_diff(cwd, path, staged, diff)
            }
            AppMsg::GitSummary { cwd, branch, changed } => {
                self.git_summary.insert(cwd, (branch, changed));
            }
            AppMsg::GitError { cwd, message } => {
                if let Some(r) = self.review.as_mut() {
                    if r.cwd == cwd {
                        r.error = Some(message);
                    }
                }
            }
        }
    }

    /// Fetch the branch + changed-file count for an agent's cwd (the inspector
    /// strip). Cheap; called when opening an agent.
    pub(super) fn load_git_summary(&self, cwd: String) {
        if cwd.is_empty() {
            return;
        }
        let cm = self.claudemon.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move { tasks::fetch_git_summary(&cm, &tx, cwd).await });
    }

    // ── git review pane ─────────────────────────────────────────────────────

    /// Fold a fresh status into the open review pane (ignored if the pane closed
    /// or moved to another work tree), then load the selected file's diff.
    fn apply_git_status(&mut self, cwd: String, branch: Option<String>, files: Vec<FileStatus>) {
        let Some(r) = self.review.as_mut() else { return };
        if r.cwd != cwd {
            return;
        }
        r.error = None;
        r.branch = branch;
        r.files = files;
        if r.selected >= r.files.len() {
            r.selected = r.files.len().saturating_sub(1);
        }
        self.review_load_diff();
    }

    /// Fold a diff into the pane only if it still matches the current selection
    /// and staged/unstaged view (a stale response for a since-changed selection
    /// is dropped).
    fn apply_git_diff(&mut self, cwd: String, path: String, staged: bool, diff: String) {
        let Some(r) = self.review.as_mut() else { return };
        if r.cwd != cwd || r.staged_view != staged {
            return;
        }
        if r.selected_file().map(|f| f.path.as_str()) != Some(path.as_str()) {
            return;
        }
        r.diff = diff;
        r.diff_scroll = 0;
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
        // Keep a stable order, like the Electron app: agents stay where they are
        // across polls and new sessions are appended at the end, rather than
        // re-sorting (e.g. floating waiting agents up) and making rows jump.
        // State changes are still visible via the per-row markers.
        let order: HashMap<&str, usize> = self
            .all_agents
            .iter()
            .enumerate()
            .map(|(i, a)| (a.session_id.as_str(), i))
            .collect();
        let next = self.all_agents.len();
        list.sort_by_key(|a| order.get(a.session_id.as_str()).copied().unwrap_or(next));

        // Drop orphans: on restart, claudemon hydrates up to 100 prior sessions
        // from its db as `stopped`. We want a live dashboard, not that history,
        // so hide stopped sessions we never saw alive this run — while keeping
        // ones that stopped *while we were watching* (still respawnable). The
        // `show_all_sessions` toggle reveals everything (e.g. to resume an old
        // session).
        for a in &list {
            if !a.is_stopped() {
                self.seen_live.insert(a.session_id.clone());
            }
        }
        let total = list.len();
        if !self.show_all_sessions {
            let seen = &self.seen_live;
            list.retain(|a| !a.is_stopped() || seen.contains(&a.session_id));
        }
        self.hidden_count = total - list.len();

        // `all_agents` is the live source of truth (lifecycle, by-id lookups);
        // `agents` is the text-filtered view the sidebar/selection use.
        self.all_agents = list;
        self.apply_filter();

        // Drop warm terminals (and no-PTY marks) for sessions that have gone
        // away — keyed off the full live set, NOT the filtered view (a session
        // merely hidden by the `/` filter is still alive).
        let live: HashSet<String> = self.all_agents.iter().map(|a| a.session_id.clone()).collect();
        self.prune_terminals(&live);
        self.no_terminal.retain(|sid| live.contains(sid));
        self.status_lines.retain(|sid, _| live.contains(sid));
        // Drop workspaces whose agent is gone (shell tabs may persist as their
        // own sessions, but the agent grouping is no longer meaningful).
        self.workspaces.retain(|agent_id, _| live.contains(agent_id));
        // Drop tiled panes whose session vanished, and keep the focus in range.
        self.tiles.retain(|sid| live.contains(sid));
        if self.tile_focus >= self.tiles.len() {
            self.tile_focus = self.tiles.len().saturating_sub(1);
        }
        self.term_resizes.retain(|sid, _| live.contains(sid));
        // Pinned/alternate/jump history follow the live session set.
        self.harpoon.retain(|sid| live.contains(sid));
        if self.prev_focus.as_ref().is_some_and(|s| !live.contains(s)) {
            self.prev_focus = None;
        }
        self.jumplist.retain(|sid| live.contains(sid));
        if self.jump_idx >= self.jumplist.len() {
            self.jump_idx = self.jumplist.len().saturating_sub(1);
        }
    }

    /// Rebuild the filtered `agents` view from `all_agents`, preserving the
    /// selected agent by id where possible. Called on every poll and on every
    /// `/`-filter keystroke.
    pub(super) fn apply_filter(&mut self) {
        let sel_id = self.selected_agent().map(|a| a.session_id.clone());
        self.agents = match self.filter.as_deref() {
            Some(q) if !q.is_empty() => {
                let needle = q.to_lowercase();
                self.all_agents
                    .iter()
                    .filter(|a| self.agent_matches(a, &needle))
                    .cloned()
                    .collect()
            }
            _ => self.all_agents.clone(),
        };
        self.selected = match sel_id {
            Some(id) => self
                .agents
                .iter()
                .position(|a| a.session_id == id)
                .map(|i| i + 1)
                .unwrap_or(0),
            None => 0,
        };
    }

    /// Whether an agent matches the sidebar filter `needle` (already lowercase):
    /// a subsequence match against its name, cwd, or state.
    fn agent_matches(&self, a: &Agent, needle: &str) -> bool {
        fuzzy_match(needle, &self.agent_name(a).to_lowercase())
            || fuzzy_match(needle, &a.cwd_str().to_lowercase())
            || fuzzy_match(needle, a.state())
    }

    // ── daemon reactions ──────────────────────────────────────────────────

    /// Fold a live statusLine tick into the per-session map (the renderer reads
    /// it via `derive_stats`).
    pub fn apply_status_line(&mut self, session_id: String, status_line: StatusLine) {
        self.status_lines.insert(session_id, status_line);
    }

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
        if self.term_resizes.is_empty() {
            return;
        }
        let pending: Vec<(String, (u16, u16))> = self.term_resizes.drain().collect();
        let cm = self.claudemon.clone();
        tokio::spawn(async move {
            for (sid, (cols, rows)) in pending {
                let _ = cm.resize(&sid, cols, rows).await;
            }
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

    /// The display name for an agent: a user-set custom name for its cwd, else
    /// the short cwd.
    pub fn agent_name(&self, a: &Agent) -> String {
        self.names
            .get(a.cwd_str())
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| a.short_cwd())
    }

    pub fn selected_agent(&self) -> Option<&Agent> {
        if self.selected == 0 {
            None
        } else {
            self.agents.get(self.selected - 1)
        }
    }

    /// The agent/session the active tab points at (may be a shell session).
    /// Resolved against the full set so an agent hidden by the `/` filter (but
    /// still open in a pane) keeps rendering.
    pub fn chat_agent(&self) -> Option<&Agent> {
        let sid = self.chat_session_id()?;
        self.all_agents.iter().find(|a| a.session_id == sid)
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

    // ── git review pane actions ─────────────────────────────────────────────

    /// Open the review pane over the targeted agent's work tree. No-op (with a
    /// toast) when the agent has no cwd.
    pub(super) fn open_review(&mut self) {
        let Some(cwd) = self.target_agent().and_then(|a| a.cwd.clone()).filter(|c| !c.is_empty())
        else {
            self.set_toast("no working directory for this agent");
            return;
        };
        self.review = Some(ReviewState::new(cwd.clone()));
        self.load_git_status(cwd);
    }

    pub(super) fn close_review(&mut self) {
        self.review = None;
    }

    /// Re-pull status for the open review pane (after a stage/commit/etc.).
    pub(super) fn review_reload(&self) {
        if let Some(r) = &self.review {
            self.load_git_status(r.cwd.clone());
        }
    }

    fn load_git_status(&self, cwd: String) {
        let cm = self.claudemon.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move { fetch_git_status(&cm, &tx, cwd).await });
    }

    /// Load the selected file's diff for the current staged/unstaged view.
    pub(super) fn review_load_diff(&self) {
        let Some(r) = &self.review else { return };
        let Some(file) = r.selected_file() else { return };
        let cm = self.claudemon.clone();
        let tx = self.tx.clone();
        let cwd = r.cwd.clone();
        let path = file.path.clone();
        let staged = r.staged_view;
        // Untracked files have no index/HEAD baseline — render them all-added.
        let untracked = !staged && file.is_untracked();
        tokio::spawn(async move {
            fetch_git_diff(&cm, &tx, cwd, path, staged, untracked).await
        });
    }

    /// Move the file selection by `delta` and load the newly-selected diff.
    pub(super) fn review_select(&mut self, delta: i32) {
        let Some(r) = self.review.as_mut() else { return };
        if r.files.is_empty() {
            return;
        }
        let n = r.files.len() as i32;
        let next = (r.selected as i32 + delta).clamp(0, n - 1);
        r.selected = next as usize;
        r.diff = String::new();
        r.diff_scroll = 0;
        self.review_load_diff();
    }

    pub(super) fn review_scroll(&mut self, delta: i32) {
        if let Some(r) = self.review.as_mut() {
            r.diff_scroll = if delta >= 0 {
                r.diff_scroll.saturating_add(delta as u16)
            } else {
                r.diff_scroll.saturating_sub((-delta) as u16)
            };
        }
    }

    pub(super) fn review_toggle_staged(&mut self) {
        if let Some(r) = self.review.as_mut() {
            r.staged_view = !r.staged_view;
            r.diff = String::new();
            r.diff_scroll = 0;
        }
        self.review_load_diff();
    }

    pub(super) fn review_stage(&mut self) {
        let Some(r) = &self.review else { return };
        let Some(file) = r.selected_file() else { return };
        let (cwd, path) = (r.cwd.clone(), file.path.clone());
        let cm = self.claudemon.clone();
        self.git_dispatch("Staged", async move { cm.git_stage(&cwd, Some(&path)).await });
    }

    pub(super) fn review_unstage(&mut self) {
        let Some(r) = &self.review else { return };
        let Some(file) = r.selected_file() else { return };
        let (cwd, path) = (r.cwd.clone(), file.path.clone());
        let cm = self.claudemon.clone();
        self.git_dispatch("Unstaged", async move { cm.git_unstage(&cwd, Some(&path)).await });
    }

    pub(super) fn review_stage_all(&mut self) {
        let Some(r) = &self.review else { return };
        let cwd = r.cwd.clone();
        let cm = self.claudemon.clone();
        self.git_dispatch("Staged all", async move { cm.git_stage(&cwd, None).await });
    }

    pub(super) fn review_push(&mut self) {
        let Some(r) = &self.review else { return };
        let cwd = r.cwd.clone();
        let cm = self.claudemon.clone();
        self.set_toast("Pushing…");
        self.git_dispatch("Pushed", async move { cm.git_push(&cwd).await });
    }

    pub(super) fn review_submit_commit(&mut self) {
        let Some(r) = self.review.as_mut() else { return };
        let msg = r.commit_msg.take().unwrap_or_default();
        let msg = msg.trim().to_string();
        if msg.is_empty() {
            self.set_toast("empty commit message");
            return;
        }
        let cwd = r.cwd.clone();
        let cm = self.claudemon.clone();
        self.git_dispatch("Committed", async move { cm.git_commit(&cwd, &msg).await });
    }

    /// Run a git mutation, toast the outcome, and reload the review status.
    fn git_dispatch<F>(&self, ok_msg: &str, fut: F)
    where
        F: std::future::Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        let tx = self.tx.clone();
        let cm = self.claudemon.clone();
        let ok_msg = ok_msg.to_string();
        let cwd = self.review.as_ref().map(|r| r.cwd.clone());
        tokio::spawn(async move {
            match fut.await {
                Ok(_) => {
                    let _ = tx.send(AppMsg::Toast(ok_msg));
                    if let Some(cwd) = cwd {
                        fetch_git_status(&cm, &tx, cwd).await;
                    }
                }
                Err(e) => {
                    let _ = tx.send(AppMsg::Toast(format!("git: {e}")));
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
        App::new(cm, Vec::new(), Vec::new(), Config::default(), tx, ptx)
    }

    fn agent(id: &str) -> Agent {
        serde_json::from_value(serde_json::json!({ "session_id": id, "mode": "responding" })).unwrap()
    }

    fn agent_cwd(id: &str, cwd: &str, mode: &str) -> Agent {
        serde_json::from_value(serde_json::json!({ "session_id": id, "cwd": cwd, "mode": mode }))
            .unwrap()
    }

    #[tokio::test]
    async fn review_opens_for_selected_agent_and_closes() {
        let mut app = test_app();
        app.set_agents(vec![agent_cwd("s1", "/repo", "responding")]);
        app.selected = 1;
        app.open_review();
        assert_eq!(app.review.as_ref().map(|r| r.cwd.as_str()), Some("/repo"));
        app.close_review();
        assert!(app.review.is_none());
    }

    #[tokio::test]
    async fn open_review_on_dashboard_row_is_noop() {
        let mut app = test_app();
        app.set_agents(vec![agent_cwd("s1", "/repo", "responding")]);
        app.selected = 0; // Dashboard row — no agent
        app.open_review();
        assert!(app.review.is_none());
        assert_eq!(app.toast(), Some("no working directory for this agent"));
    }

    #[tokio::test]
    async fn respawn_refuses_a_running_agent() {
        let mut app = test_app();
        app.set_agents(vec![agent_cwd("s1", "/repo", "responding")]);
        app.selected = 1;
        app.respawn();
        assert_eq!(app.toast(), Some("agent is still running"));
    }

    #[test]
    fn git_error_surfaces_in_open_review() {
        let mut app = test_app();
        app.review = Some(ReviewState::new("/repo".into()));
        app.apply_msg(AppMsg::GitError {
            cwd: "/repo".into(),
            message: "cwd is not inside a git work tree".into(),
        });
        assert_eq!(
            app.review.as_ref().and_then(|r| r.error.as_deref()),
            Some("cwd is not inside a git work tree")
        );
        // A successful status clears the error.
        app.apply_msg(AppMsg::GitStatus { cwd: "/repo".into(), branch: Some("main".into()), files: vec![] });
        assert!(app.review.as_ref().unwrap().error.is_none());
    }

    #[test]
    fn status_line_applied_and_pruned_with_session() {
        let mut app = test_app();
        app.set_agents(vec![agent("s1")]);
        app.apply_status_line(
            "s1".into(),
            StatusLine { context_used_pct: Some(50.0), ..Default::default() },
        );
        assert!(app.status_lines.contains_key("s1"));
        app.set_agents(vec![]); // session gone → statusline pruned
        assert!(!app.status_lines.contains_key("s1"));
    }

    #[tokio::test]
    async fn notes_open_loads_existing_text() {
        let mut app = test_app();
        app.set_agents(vec![agent_cwd("s1", "/repo", "responding")]);
        app.selected = 1;
        app.notes.insert("/repo".into(), "remember this".into());
        app.open_notes();
        assert_eq!(app.notes_view.as_ref().map(|n| n.text.as_str()), Some("remember this"));
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

    #[tokio::test]
    async fn splits_tile_focus_and_collapse() {
        let mut app = test_app();
        app.set_agents(vec![agent("s1"), agent("s2"), agent("s3")]);
        app.selected = 1; // first agent (s1)
        app.open_agent();
        assert_eq!(app.tiles, vec!["s1".to_string()]);
        assert_eq!(app.tile_focus, 0);

        // Split brings the next untiled agent into a new pane and focuses it.
        app.split_pane(SplitDir::Columns);
        assert_eq!(app.tiles, vec!["s1".to_string(), "s2".to_string()]);
        assert_eq!(app.tile_focus, 1);
        assert_eq!(app.open_agent_id(), Some("s2"));

        // Focus wraps around the tiles.
        app.focus_pane(1);
        assert_eq!(app.tile_focus, 0);
        assert_eq!(app.open_agent_id(), Some("s1"));

        // A third split, then close the focused pane.
        app.focus_pane(-1); // back to s2
        app.split_pane(SplitDir::Rows);
        assert_eq!(app.tiles, vec!["s1".to_string(), "s2".to_string(), "s3".to_string()]);
        assert_eq!(app.tile_focus, 2);
        app.close_pane();
        assert_eq!(app.tiles, vec!["s1".to_string(), "s2".to_string()]);

        // only_pane keeps just the focused tile; the last close leaves the view.
        app.only_pane();
        assert_eq!(app.tiles.len(), 1);
        app.close_pane();
        assert_eq!(app.view, View::List);
        assert!(app.tiles.is_empty());
    }

    #[tokio::test]
    async fn split_with_no_other_agent_toasts() {
        let mut app = test_app();
        app.set_agents(vec![agent("s1")]);
        app.selected = 1;
        app.open_agent();
        app.split_pane(SplitDir::Columns);
        assert_eq!(app.tiles.len(), 1);
        assert_eq!(app.toast(), Some("no other agent to split"));
    }

    #[tokio::test]
    async fn tiles_prune_when_a_session_vanishes() {
        let mut app = test_app();
        app.set_agents(vec![agent("s1"), agent("s2")]);
        app.selected = 1;
        app.open_agent();
        app.split_pane(SplitDir::Columns); // tiles = [s1, s2], focus s2
        assert_eq!(app.tiles.len(), 2);
        // s2 goes away — its tile drops and focus stays in range.
        app.set_agents(vec![agent("s1")]);
        assert_eq!(app.tiles, vec!["s1".to_string()]);
        assert!(app.tile_focus < app.tiles.len());
    }

    #[tokio::test]
    async fn harpoon_pin_jump_and_alternate() {
        let mut app = test_app();
        app.set_agents(vec![agent("s1"), agent("s2"), agent("s3")]);
        app.selected = 1;
        app.open_agent(); // s1
        app.harpoon_toggle();
        app.selected = 2;
        app.open_agent(); // s2
        app.harpoon_toggle();
        assert_eq!(app.harpoon, vec!["s1".to_string(), "s2".to_string()]);

        // Teleport to slot 1, then the alternate-agent toggles back and forth.
        app.harpoon_jump(1);
        assert_eq!(app.open_agent_id(), Some("s1"));
        app.alt_agent();
        assert_eq!(app.open_agent_id(), Some("s2"));
        app.alt_agent();
        assert_eq!(app.open_agent_id(), Some("s1"));

        // Unpin the focused agent; an empty slot just toasts.
        app.harpoon_toggle();
        assert_eq!(app.harpoon, vec!["s2".to_string()]);
        app.harpoon_jump(5);
        assert_eq!(app.toast(), Some("no agent pinned at 5"));
    }

    #[tokio::test]
    async fn hides_hydrated_stopped_orphans() {
        let mut app = test_app();
        // A live agent plus two stopped orphans hydrated from history.
        app.set_agents(vec![
            agent_cwd("s1", "/repo", "responding"),
            agent_cwd("old1", "/a", "stopped"),
            agent_cwd("old2", "/b", "stopped"),
        ]);
        assert_eq!(app.agents.len(), 1);
        assert_eq!(app.agents[0].session_id, "s1");
        assert_eq!(app.hidden_count, 2);

        // An agent we watched live stays visible once it stops (respawnable).
        app.set_agents(vec![agent_cwd("s1", "/repo", "stopped")]);
        assert_eq!(app.agents.len(), 1, "s1 was seen live, so it survives stopping");
        assert_eq!(app.hidden_count, 0);

        // Toggling show-all reveals stopped history again.
        app.show_all_sessions = true;
        app.set_agents(vec![
            agent_cwd("s1", "/repo", "stopped"),
            agent_cwd("old1", "/a", "stopped"),
        ]);
        assert_eq!(app.agents.len(), 2);
        assert_eq!(app.hidden_count, 0);
    }

    #[tokio::test]
    async fn sidebar_filter_narrows_and_preserves_the_full_set() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = test_app();
        app.set_agents(vec![
            agent_cwd("s1", "/work/alpha", "responding"),
            agent_cwd("s2", "/work/beta", "responding"),
            agent_cwd("s3", "/other/gamma", "responding"),
        ]);
        assert_eq!(app.agents.len(), 3);

        // Filter by a cwd subsequence — the view narrows, the full set is intact.
        app.open_filter();
        assert!(app.filter_editing);
        for c in "beta".chars() {
            app.handle_filter_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        assert_eq!(app.agents.len(), 1);
        assert_eq!(app.agents[0].session_id, "s2");
        assert_eq!(app.all_agents.len(), 3, "filter is a view; full set is untouched");

        // A poll while filtered keeps the filter applied.
        app.handle_filter_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!app.filter_editing);
        app.set_agents(vec![
            agent_cwd("s1", "/work/alpha", "responding"),
            agent_cwd("s2", "/work/beta", "responding"),
        ]);
        assert_eq!(app.agents.len(), 1);
        assert_eq!(app.agents[0].session_id, "s2");

        // Esc clears the filter and restores the full view.
        app.open_filter();
        app.handle_filter_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.filter.is_none());
        assert_eq!(app.agents.len(), 2);
    }

    #[tokio::test]
    async fn jumplist_steps_back_and_forward() {
        let mut app = test_app();
        app.set_agents(vec![agent("s1"), agent("s2"), agent("s3")]);
        app.selected = 1;
        app.open_agent(); // s1
        app.selected = 2;
        app.open_agent(); // s2
        app.selected = 3;
        app.open_agent(); // s3
        app.jump_history(-1);
        assert_eq!(app.open_agent_id(), Some("s2"));
        app.jump_history(-1);
        assert_eq!(app.open_agent_id(), Some("s1"));
        app.jump_history(1);
        assert_eq!(app.open_agent_id(), Some("s2"));
    }
}
