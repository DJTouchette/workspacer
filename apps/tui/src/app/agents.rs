//! The agent list: folding a fresh roster in, the sidebar filter, harpoon
//! resolution, and the naming/selection accessors.

use super::*;

impl App {
    /// Adopt a fresh LOCAL roster from claudemon and re-fold the fleet.
    pub(in crate::app) fn set_agents(&mut self, list: Vec<Agent>) {
        self.local_agents = list;
        self.fold_fleet();
    }

    /// Rebuild `all_agents` from the local roster + the remote (federated)
    /// store. Called on every claudemon poll and whenever the remote side
    /// changes, so a peer event doesn't wait for the next local poll.
    pub(in crate::app) fn fold_fleet(&mut self) {
        let mut list: Vec<Agent> = self.local_agents.clone();
        list.extend(self.remote.agents());
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
        // session). Remote rows go through the same rule: a peer's ended
        // session we never saw alive is history, not fleet. (Tombstones are
        // NOT stopped — an offline hub's rows keep their last-known mode, so
        // this filter never eats them.)
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
        let live: HashSet<String> = self
            .all_agents
            .iter()
            .map(|a| a.session_id.clone())
            .collect();
        self.prune_terminals(&live);
        self.no_terminal.retain(|sid| live.contains(sid));
        self.status_lines.retain(|sid, _| live.contains(sid));
        self.managed_providers.retain(|sid, _| live.contains(sid));
        self.perm_modes.retain(|sid, _| live.contains(sid));
        // Drop workspaces whose agent is gone (shell tabs may persist as their
        // own sessions, but the agent grouping is no longer meaningful).
        self.workspaces
            .retain(|agent_id, _| live.contains(agent_id));
        // Drop tiled panes whose session vanished, and keep the focus in range.
        self.tiles.retain(|sid| live.contains(sid));
        if self.tile_focus >= self.tiles.len() {
            self.tile_focus = self.tiles.len().saturating_sub(1);
        }
        self.term_resizes.retain(|sid, _| live.contains(sid));
        // Resolve the persisted pins against the live sessions (this also drops
        // pins whose agent isn't currently running).
        self.rebuild_harpoon();
        // Alternate / jump history follow the live session set.
        if self.prev_focus.as_ref().is_some_and(|s| !live.contains(s)) {
            self.prev_focus = None;
        }
        self.jumplist.retain(|sid| live.contains(sid));
        if self.jump_idx >= self.jumplist.len() {
            self.jump_idx = self.jumplist.len().saturating_sub(1);
        }
        // Drop the question stepper once its session's question set is gone
        // (answered, superseded by a different set — even one of the same
        // length — or the session ended).
        if let Some(flow) = &self.question_flow {
            let still_pending = self
                .all_agents
                .iter()
                .find(|a| a.session_id == flow.session_id)
                .and_then(|a| a.questions())
                .is_some_and(|qs| flow.tracks(&flow.session_id, qs));
            if !still_pending {
                self.question_flow = None;
            }
        }
    }

    /// Rebuild the filtered `agents` view from `all_agents`, preserving the
    /// selected agent by id where possible. Called on every poll and on every
    /// `/`-filter keystroke.
    pub(in crate::app) fn apply_filter(&mut self) {
        let sel_id = self.selected_agent().map(|a| a.session_id.clone());
        let needle = self
            .filter
            .as_deref()
            .filter(|q| !q.is_empty())
            .map(str::to_lowercase);
        self.agents = self
            .all_agents
            .iter()
            // Hide TUI-spawned shells — they live inside their agent's tab bar,
            // not as their own sidebar rows — and apply the `/` filter.
            .filter(|a| !self.is_shell_session(&a.session_id))
            .filter(|a| match &needle {
                Some(q) => self.agent_matches(a, q),
                None => true,
            })
            .cloned()
            .collect();
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

    /// Whether `sid` is a TUI-spawned shell (a `Shell` tab) rather than an
    /// agent. Such sessions render inside their agent's tab bar, so they're kept
    /// out of the sidebar / dashboard / agent pickers — but stay in `all_agents`
    /// so the tab itself still resolves its title and terminal.
    pub fn is_shell_session(&self, sid: &str) -> bool {
        self.workspaces.values().any(|ws| {
            ws.tabs
                .iter()
                .any(|t| t.kind == TabKind::Shell && t.session_id == sid)
        })
    }

    /// Rebuild the live `harpoon` (running session ids) from the persisted `pinned`,
    /// in pin order: each cwd resolves to whatever live session is in it, and
    /// pins with no running agent are simply absent (so slot numbers stay
    /// gap-free for what's actually reachable).
    pub(in crate::app) fn rebuild_harpoon(&mut self) {
        // Upgrade any legacy cwd values first (and push a pending migration
        // once agents exist to resolve against), then keep the slots that
        // point at a LIVE agent — a pinned-but-stopped session keeps its pin,
        // it just has no reachable slot until it's back.
        self.normalize_pins();
        self.harpoon = self
            .pinned
            .iter()
            .filter(|sid| self.all_agents.iter().any(|a| &a.session_id == *sid))
            .cloned()
            .collect();
    }

    /// Whether an agent matches the sidebar filter `needle` (already lowercase):
    /// a subsequence match against its name, cwd, or state.
    pub(in crate::app) fn agent_matches(&self, a: &Agent, needle: &str) -> bool {
        fuzzy_match(needle, &self.agent_name(a).to_lowercase())
            || fuzzy_match(needle, &a.cwd_str().to_lowercase())
            || fuzzy_match(needle, a.state())
    }

    // ── daemon reactions ──────────────────────────────────────────────────

    /// The display name for an agent: a user-set custom name for its cwd, else
    /// (for remote sessions) the label its home hub assigned, else the short
    /// cwd. Remote cwds name a peer's filesystem, so their label — when the
    /// peer set one — says more than a truncated foreign path.
    pub fn agent_name(&self, a: &Agent) -> String {
        self.names
            .get(a.cwd_str())
            .filter(|s| !s.is_empty())
            .cloned()
            .or_else(|| {
                a.is_remote()
                    .then(|| a.label.clone())
                    .flatten()
                    .filter(|l| !l.is_empty())
            })
            .unwrap_or_else(|| a.short_cwd())
    }

    /// The project mark for an agent — initials and a colour for its cwd.
    ///
    /// `None` only for a session with no cwd. Independent of [`Self::agent_name`]
    /// on purpose: a renamed agent still belongs to a repo, and the mark is then
    /// the only thing on the row that says which one.
    pub fn project(&self, a: &Agent) -> Option<crate::projects::ResolvedProject> {
        crate::projects::resolve_project(a.cwd_str(), &self.projects)
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

    /// Resolve a session's provider (`"claude"`/`"codex"`/`"copilot"`/`"opencode"`/`"pi"`).
    ///
    /// Prefers claudemon's authoritative wire field (present for every session,
    /// however it was spawned), and falls back to the local managed-spawn map
    /// for the window between spawning a managed agent here and the first
    /// session-list refresh that carries it — and for daemons too old to emit
    /// `provider`. The map only ever holds non-Claude entries (managed spawns),
    /// so a wire `"claude"` defers to it; anything unknown resolves to
    /// `"claude"`.
    pub fn provider_for(&self, sid: &str) -> String {
        self.all_agents
            .iter()
            .find(|a| a.session_id == sid)
            .map(|a| a.provider.clone())
            .filter(|p| p != "claude")
            .or_else(|| self.managed_providers.get(sid).cloned())
            .unwrap_or_else(|| "claude".to_string())
    }

    // ── async actions (fire-and-forget; results arrive as AppMsg) ───────────

    pub fn dashboard_selected(&self) -> bool {
        self.selected == 0
    }
}
