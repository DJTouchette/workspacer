//! The App's satellite state: the message enum the event loop folds in, the
//! view/tab vocabulary, and the per-overlay state each modal carries while it
//! is open. Kept apart from `App` itself so the struct reads as a list of what
//! it holds rather than a wall of the things it holds.

use super::*;

/// Messages spawned tasks post back to the app loop.
#[derive(Debug)]
pub enum AppMsg {
    Agents(Vec<Agent>),
    /// A full conversation snapshot, adopted into the session's fold. Only sent
    /// on open and on a delta gap — the steady state is `ConvDelta`.
    Transcript {
        session_id: String,
        snapshot: Box<serde_json::Value>,
    },
    /// Subagent + workflow progress read off disk for a session.
    Runs {
        session_id: String,
        runs: Box<crate::runs::SessionRuns>,
    },
    /// One `conversation.delta` frame from claudemon's SSE feed.
    ConvDelta(Box<crate::claudemon::ConvDelta>),
    Toast(String),
    /// A chat send failed after its optimistic echo was drawn: drop the echo,
    /// restore the composer text, and toast the error.
    SendFailed {
        text: String,
        error: String,
    },
    /// A session has no PTY to stream (external/observed-only) — fall back to
    /// the transcript view for it.
    TerminalUnavailable(String),
    /// A shell spawned for a `new terminal` tab is ready.
    ShellSpawned {
        agent_id: String,
        session_id: String,
    },
    /// Git status for the review pane's work tree.
    GitStatus {
        cwd: String,
        branch: Option<String>,
        files: Vec<FileStatus>,
    },
    /// A file's unified diff for the review pane.
    GitDiff {
        cwd: String,
        path: String,
        staged: bool,
        diff: String,
    },
    /// Lightweight branch + changed-file count for the open agent's inspector.
    GitSummary {
        cwd: String,
        branch: Option<String>,
        changed: usize,
    },
    /// A git read failed for a work tree (e.g. not a repo) — shown in review.
    GitError {
        cwd: String,
        message: String,
    },
    /// A session's transcript lines, for the content-search index.
    SearchEntries {
        session_id: String,
        name: String,
        lines: Vec<String>,
    },
    /// The daemon settled a live permission-mode switch — remember it so the
    /// cycle advances from the real current mode next time.
    PermissionMode {
        session_id: String,
        mode: String,
    },
    /// A managed provider's launchable models arrived — fold them into the open
    /// model picker (if it's still for this session).
    PickerModels {
        session_id: String,
        models: Vec<crate::claudemon::ProviderModel>,
    },
    /// A managed (Codex/OpenCode/Pi) session spawned — record its provider so the
    /// model picker and permission-mode cycle pick the managed behaviour for it.
    ManagedSpawned {
        session_id: String,
        provider: String,
    },
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

/// Providers the spawn modal can launch. `claude` is a PTY session (profile +
/// argv); the rest are managed (adapter-driven) sessions via
/// `/sessions/spawn-managed`.
pub const SPAWN_PROVIDERS: &[&str] = &["claude", "codex", "opencode", "pi"];

/// State of the "spawn a new agent" modal. Profile-centric for claude: a working
/// directory plus a chosen profile (which carries model / skip-permissions in
/// its args). A non-claude `provider_idx` selects a managed backend instead, for
/// which the profile is ignored.
#[derive(Debug, Clone)]
pub struct SpawnForm {
    pub cwd: String,
    pub profile_idx: usize,
    /// Index into [`SPAWN_PROVIDERS`]; 0 = claude (the default PTY path).
    pub provider_idx: usize,
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
    /// Run an ex command (the `:`-line verb), e.g. `vsplit`.
    Command(String),
}

#[derive(Debug, Clone)]
pub struct PaletteItem {
    pub label: String,
    pub hint: String,
    pub action: PaletteAction,
}

/// One indexed transcript line, for cross-agent content search.
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub session_id: String,
    pub name: String,
    pub line: String,
}

/// State of the content-search modal: an index of transcript lines across the
/// fleet, grep-filtered live by `query`. `pending` counts sessions still being
/// fetched, so the UI can show that indexing is in progress.
pub struct SearchState {
    pub query: String,
    pub entries: Vec<SearchHit>,
    /// Indices into `entries` matching `query` (case-insensitive substring).
    pub matched: Vec<usize>,
    pub selected: usize,
    pub pending: usize,
}

impl SearchState {
    /// Re-run the grep over the index. An empty query matches nothing (a content
    /// search dumping the whole corpus isn't useful).
    pub fn rematch(&mut self) {
        const MAX_RESULTS: usize = 300;
        let q = self.query.to_lowercase();
        self.matched = if q.is_empty() {
            Vec::new()
        } else {
            self.entries
                .iter()
                .enumerate()
                .filter(|(_, h)| h.line.to_lowercase().contains(&q))
                .map(|(i, _)| i)
                .take(MAX_RESULTS)
                .collect()
        };
        if self.selected >= self.matched.len() {
            self.selected = self.matched.len().saturating_sub(1);
        }
    }

    pub fn chosen(&self) -> Option<&SearchHit> {
        self.matched.get(self.selected).map(|&i| &self.entries[i])
    }
}

/// One selectable row in a [`Picker`]: a stable `id` (what gets applied) and a
/// human `label`.
#[derive(Debug, Clone)]
pub struct PickerItem {
    pub id: String,
    pub label: String,
}

/// What a [`Picker`] is choosing, and the context each choice needs to act on.
#[derive(Debug, Clone)]
pub enum PickerKind {
    /// Live model switch for `session_id`. `provider` decides how it applies:
    /// a managed provider POSTs `/model`; `claude` sends a `/model` slash command
    /// on the message path (its PTY 409s the endpoint). `effort` rides along for
    /// providers that map it (codex).
    Model {
        provider: String,
        effort: Option<String>,
    },
    /// Cross-provider handoff: build a brief from `session_id`, then spawn the
    /// chosen provider as the successor (in `cwd`) primed to read it.
    Handoff { cwd: String },
}

/// A modal fuzzy list bound to one session — the model picker and the handoff
/// provider chooser. Mirrors [`Palette`]/[`SearchState`]: a query line filters
/// `items` live. `allow_free_text` lets the model picker apply a typed id that
/// isn't in the (provider-supplied) list. `pending` is true while the async list
/// is still loading.
pub struct Picker {
    pub title: String,
    pub kind: PickerKind,
    pub session_id: String,
    pub query: String,
    pub items: Vec<PickerItem>,
    pub matched: Vec<usize>,
    pub selected: usize,
    pub pending: bool,
    pub allow_free_text: bool,
}

impl Picker {
    /// Re-run the substring filter over `items` (case-insensitive). An empty
    /// query matches everything (unlike content search — the list is small).
    pub fn rematch(&mut self) {
        let q = self.query.to_lowercase();
        self.matched = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                q.is_empty()
                    || it.label.to_lowercase().contains(&q)
                    || it.id.to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect();
        if self.selected >= self.matched.len() {
            self.selected = self.matched.len().saturating_sub(1);
        }
    }

    pub fn chosen(&self) -> Option<&PickerItem> {
        self.matched.get(self.selected).map(|&i| &self.items[i])
    }
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
        Palette {
            query: String::new(),
            items,
            filtered,
            selected: 0,
        }
    }

    pub(super) fn refilter(&mut self) {
        let q = self.query.to_lowercase();
        self.filtered = self
            .items
            .iter()
            .enumerate()
            // Match across label + hint so agents are findable by cwd/state and
            // commands by their description, not just the visible label.
            .filter(|(_, it)| fuzzy_match(&q, &format!("{} {}", it.label, it.hint).to_lowercase()))
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
pub(super) fn fuzzy_match(needle: &str, haystack: &str) -> bool {
    let mut hay = haystack.chars();
    needle.chars().all(|c| hay.any(|h| h == c))
}

pub(super) const TOAST_TTL: Duration = Duration::from_millis(2500);

/// Progress through a pending multi-question / multi-select set (desktop
/// `/answer` parity): one question renders at a time, digits answer or toggle,
/// and the collected raw answers post as `{answers: [...]}` after the last.
/// Raw per question: a 1-indexed digit string for a pick, free text otherwise,
/// or the chosen labels joined `", "` for a multi-select.
#[derive(Debug, Clone)]
/// The memoized transcript render (see [`App::transcript_cache`]).
pub struct TranscriptCache {
    /// The wrap width the lines were built for.
    pub width: usize,
    /// The session these lines belong to — switching chats must not show the
    /// previous one's transcript while the commit counters happen to agree.
    pub session_id: Option<String>,
    /// `ConvFold::commits` when these lines were built.
    pub commits: u64,
    /// The COMMITTED turns, folded + wrapped. The live tail (streaming
    /// assistant text, the optimistic send echo) is deliberately not here: it
    /// changes on every token, and including it meant every token threw away
    /// the wrapped render of the entire conversation and re-parsed it — at ~700
    /// KB of transcript that is ~35-40 ms per frame, which the delta rate
    /// outruns. The renderer appends the tail per frame and slices the viewport
    /// out of the two together.
    pub lines: Vec<ratatui::text::Line<'static>>,
}

pub struct QuestionFlow {
    /// The session whose pending question set this tracks.
    pub session_id: String,
    /// Content fingerprint of the tracked set (see
    /// [`crate::types::question_fingerprint`]) — a superseded set of the same
    /// length must not inherit this flow's position and recorded answers.
    pub fingerprint: u64,
    /// Index of the question currently on screen.
    pub idx: usize,
    /// Raw answer per question, filled as the user advances (kept when
    /// stepping back, so a revisited pick renders highlighted).
    pub answers: Vec<Option<String>>,
    /// Per-question multi-select toggles (indices into that question's
    /// options), in option order.
    pub picks: Vec<std::collections::BTreeSet<usize>>,
}

impl QuestionFlow {
    pub fn new(session_id: String, qs: &[crate::types::Question]) -> Self {
        QuestionFlow {
            session_id,
            fingerprint: crate::types::question_fingerprint(qs),
            idx: 0,
            answers: vec![None; qs.len()],
            picks: vec![std::collections::BTreeSet::new(); qs.len()],
        }
    }

    /// Whether this flow tracks exactly `sid`'s question set `qs` (same
    /// session, same length, same content).
    pub fn tracks(&self, sid: &str, qs: &[crate::types::Question]) -> bool {
        self.session_id == sid
            && self.answers.len() == qs.len()
            && self.fingerprint == crate::types::question_fingerprint(qs)
    }
}

/// What a docked side pane is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideKind {
    /// The git work tree: what the repo looks like now. Interactive — its keys
    /// apply while it holds focus.
    Review,
    /// The files the agent changed, from the transcript. Read-only.
    Changes,
}

/// A pane docked to the right of (or below) the agent tiles.
#[derive(Debug, Clone)]
pub struct SidePane {
    pub kind: SideKind,
    /// The session it belongs to — a docked pane follows one agent, not the view.
    pub session_id: String,
    /// True while keys route here instead of to the chat.
    pub focused: bool,
    pub scroll: u16,
}

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
    pub(super) fn new(cwd: String) -> Self {
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
