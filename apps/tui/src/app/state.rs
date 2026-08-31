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
    /// `config.projects` read off the hub bus — the human's overrides for each
    /// project's mark. Never sent when there's no bus; every mark is derived
    /// from its path until (and unless) this lands.
    Projects(crate::projects::Projects),
    /// `ui.pinnedAgentCwds` from the shared config document. `None` = the key
    /// is ABSENT (never written by any client), which gates the one-time
    /// migration off `tui-pins.json`; `Some(vec![])` is a real "all unpinned".
    PinnedCwds(Option<Vec<String>>),
    /// One peer hub's authoritative session roster, pulled over the federation
    /// link (`hub:<peer>/sessions.snapshots`). Replaces that hub's remote
    /// sessions wholesale — see [`crate::federation`].
    RemoteSeed {
        hub: String,
        agents: Vec<Agent>,
    },
    /// A peer hub reported disconnected (by `federation.peers` at seed time, or
    /// the `hub.peer.disconnected` event): its sessions become tombstones.
    HubDown {
        hub: String,
    },
    /// The hub's remote-node registry, from `nodes.list`. The authoritative
    /// roster — it replaces what we hold. `None` is the FEATURE-ABSENT answer
    /// (`no provider for nodes.list`), which is the normal state of every
    /// ordinary install and must render as no surface at all rather than an
    /// error. See [`crate::nodes`].
    Nodes(Option<Vec<crate::nodes::NodeView>>),
    /// Provider ids whose CLI the host reports as installed
    /// (`providers.checkAll`). Sets the app's list, which starts as `None` =
    /// "no answer, offer everything". See [`crate::providers`].
    InstalledProviders(Vec<String>),
    /// The outcome of a confirmed `nodes.wake`. `node` is the hub's answer
    /// (normally `waking`) and arrives BEFORE the machine is up — the rest
    /// follows on `node.state_changed`. `error` is a rendered sentence, and a
    /// refused wake must be shown rather than swallowed: it is the only notice
    /// that money was or was not spent.
    NodeWake {
        id: String,
        node: Option<Box<crate::nodes::NodeView>>,
        error: Option<String>,
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

/// State of the "spawn a new agent" modal. Profile-centric for claude: a working
/// directory plus a chosen profile (which carries model / skip-permissions in
/// its args). A non-claude `provider_idx` selects a managed backend instead, for
/// which the profile is ignored.
#[derive(Debug, Clone)]
pub struct SpawnForm {
    pub cwd: String,
    pub profile_idx: usize,
    /// Index into `App::spawn_provider_choices()` — the harnesses this machine
    /// can actually launch, NOT the full vocabulary. 0 is claude whenever
    /// claude is installed (the default PTY path).
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

/// State of the remote-nodes overlay: the hub's node registry with a cursor,
/// and the confirmation step that stands between a keypress and a bill.
///
/// `confirm` is the whole reason this struct is not just a `usize`. Waking a
/// node starts a billable machine and **this hub has no verb to stop one**, so
/// an accidental wake cannot be undone from inside the app at all. The desktop
/// and `/m` both put a confirmation step in front of it for exactly that
/// reason; in a vim-modal TUI, where a stray keystroke in normal mode is the
/// native failure, the same rule means no single keypress may spend money.
pub struct NodesState {
    /// Cursor into the registry, in the hub's own order.
    pub selected: usize,
    /// The node id awaiting an explicit `y`. Set only by `w` on a node whose
    /// wake would actually be allowed, and cleared by ANY other key.
    pub confirm: Option<String>,
    /// Wakes fired and not yet answered, by node id — so a second `w` can't
    /// fire a second cloud API start (Fly allows one action per second per
    /// machine, and a 429 reads to a person as "the button does nothing").
    pub pending: std::collections::HashSet<String>,
    /// The last wake failure per node id, shown on its row until the hub says
    /// something newer about that node.
    pub errors: HashMap<String, String>,
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
    /// all providers POST the structural `/model` endpoint; claudemon sends any Claude PTY command
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
