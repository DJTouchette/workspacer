//! Moving between agents: the sidebar selection, opening one, harpoon pins,
//! the alternate agent, and the jumplist.

use super::*;

impl App {
    pub(in crate::app) fn select_next(&mut self) {
        // Rows are [Dashboard, ..agents], so the max index is agents.len().
        self.selected = (self.selected + 1).min(self.agents.len());
    }

    pub(in crate::app) fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Move selection to the next agent (after the current one, wrapping) that
    /// needs the user — the `m` "jump to attention" key.
    pub(in crate::app) fn jump_to_attention(&mut self) {
        let n = self.agents.len();
        if n == 0 {
            return;
        }
        // Current agent index, or "before the first" when the Dashboard is selected.
        let cur = if self.selected == 0 {
            n - 1
        } else {
            self.selected - 1
        };
        for offset in 1..=n {
            let i = (cur + offset) % n;
            // `needs_you`, not `is_waiting`: a tombstoned remote session (hub
            // offline) can't be acted on, so the jump skips it.
            if self.agents[i].needs_you() {
                self.selected = i + 1;
                return;
            }
        }
        self.set_toast("Nothing waiting");
    }

    /// Open the selected agent into its tab view (creating its workspace with a
    /// single Claude tab the first time). The Dashboard row doesn't "open" — it's
    /// a live preview, so Enter there is a no-op.
    pub(in crate::app) fn open_agent(&mut self) {
        let Some(agent) = self.selected_agent() else {
            return;
        };
        let id = agent.session_id.clone();
        // Opening from the sidebar resets the layout to a single pane.
        self.tiles = vec![id.clone()];
        self.tile_focus = 0;
        self.focus_agent(id);
    }

    /// Make `id` the interactive (focused) agent: sync the view, ensure its
    /// workspace exists, and warm its terminal. Records the move in the jump
    /// history / alternate-agent. Shared by open / split / pane-focus.
    pub(in crate::app) fn focus_agent(&mut self, id: String) {
        self.focus_agent_inner(id, true);
    }

    /// Focus `id`, optionally recording it in the jump history. `record` is
    /// false when we're *navigating* the history itself (`Ctrl-o` / forward), so
    /// stepping back and forth doesn't keep appending.
    pub(in crate::app) fn focus_agent_inner(&mut self, id: String, record: bool) {
        if record {
            if let Some(prev) = self.open_agent_id() {
                if prev != id {
                    let prev = prev.to_string();
                    self.prev_focus = Some(prev);
                    // Drop any forward history, then append the destination.
                    self.jumplist.truncate(self.jump_idx + 1);
                    if self.jumplist.last().map(String::as_str) != Some(id.as_str()) {
                        self.jumplist.push(id.clone());
                    }
                    self.jump_idx = self.jumplist.len().saturating_sub(1);
                }
            } else if self.jumplist.last().map(String::as_str) != Some(id.as_str()) {
                // First agent opened — seed the history.
                self.jumplist.push(id.clone());
                self.jump_idx = self.jumplist.len().saturating_sub(1);
            }
        }
        self.view = View::Agent { id: id.clone() };
        self.workspaces
            .entry(id.clone())
            .or_insert_with(|| Workspace {
                tabs: vec![Tab {
                    title: "claude".into(),
                    session_id: id.clone(),
                    kind: TabKind::Claude,
                }],
                active: 0,
            });
        self.enter_active_tab();
    }

    // ── harpoon (pinned agents) + jump history ────────────────────────────

    /// Open `id` as a single full-content pane (collapsing any splits), the way
    /// a harpoon/jumplist teleport behaves.
    pub(in crate::app) fn open_single(&mut self, id: String, record: bool) {
        self.tiles = vec![id.clone()];
        self.tile_focus = 0;
        self.focus_agent_inner(id, record);
    }

    /// Pin or unpin the target agent — by its SESSION id, the shared slot key
    /// (see `crate::pins`: a cwd was ambiguous with two agents in one repo).
    /// Persists and rebuilds the live harpoon.
    pub(in crate::app) fn harpoon_toggle(&mut self) {
        let Some(sid) = self.target_agent().map(|a| a.session_id.clone()) else {
            self.set_toast("no agent to pin");
            return;
        };
        if let Some(pos) = self.pinned.iter().position(|c| c == &sid) {
            self.pinned.remove(pos);
            self.set_toast("Unpinned");
        } else {
            self.pinned.push(sid);
            self.set_toast(format!("Pinned #{}", self.pinned.len()));
        }
        self.persist_pins();
        self.rebuild_harpoon();
    }

    /// Upgrade legacy cwd pin values to session ids once agents are known
    /// (old shared key / old tui-pins.json), dedupe, and drop paths nothing
    /// resolves. Ids whose agent isn't RUNNING right now are kept — that's a
    /// stopped-but-resumable pin, not a dead one. Also flushes a pending
    /// migration push (shared key was absent) once there's something
    /// resolved to push.
    pub(in crate::app) fn normalize_pins(&mut self) {
        if self.all_agents.is_empty() {
            return; // nothing to resolve against yet
        }
        let mut changed = false;
        let mut seen = std::collections::HashSet::new();
        let resolved: Vec<String> = self
            .pinned
            .iter()
            .filter_map(|v| {
                let sid = if self.all_agents.iter().any(|a| &a.session_id == v) {
                    v.clone()
                } else if let Some(a) = self.all_agents.iter().find(|a| a.cwd_str() == v) {
                    changed = true;
                    a.session_id.clone()
                } else if v.contains('/') {
                    changed = true; // unresolvable legacy path — convenience, drop
                    return None;
                } else {
                    v.clone() // an id whose agent isn't running right now
                };
                if seen.insert(sid.clone()) {
                    Some(sid)
                } else {
                    changed = true;
                    None
                }
            })
            .collect();
        if changed {
            self.pinned = resolved;
            self.persist_pins();
        } else if self.pins_push_pending && !self.pinned.is_empty() {
            self.persist_pins();
        }
        self.pins_push_pending = false;
    }

    /// Write the pins to whichever store is authoritative right now: the
    /// SHARED config (`ui.pinnedAgentCwds`, via the brain's `config.save` —
    /// the same slots the desktop's `prefix m` / `prefix 1-9` use) when the
    /// bus is up, or the legacy local `tui-pins.json` off-bus. Never both:
    /// on-bus, config is the truth and a stale local file must not shadow it
    /// on some future off-bus launch more than it already can.
    pub(in crate::app) fn persist_pins(&mut self) {
        if let Some(bus) = self.bus.clone() {
            let partial = crate::pins::config_partial(&self.pinned);
            tokio::spawn(async move {
                let _ = bus.call("config.save", partial).await;
            });
        } else if let Err(e) = crate::pins::save(&self.pinned) {
            self.set_toast(crate::store::save_toast("", "pins", Err(e)));
        }
    }

    /// Fold the shared pin store (from `config.get`) into the harpoon.
    ///
    /// `Some(list)` — the shared key exists: it IS the truth, local state
    /// follows (the desktop may have pinned/unpinned while we ran). `None` —
    /// the key has never been written by ANY client: if this TUI carries
    /// legacy `tui-pins.json` pins, push them up once so they become the
    /// shared truth instead of silently diverging from the desktop's empty
    /// set.
    pub(in crate::app) fn adopt_shared_pins(&mut self, shared: Option<Vec<String>>) {
        match shared {
            Some(pins) => {
                if self.pinned != pins {
                    self.pinned = pins;
                    self.rebuild_harpoon(); // normalizes legacy cwd values too
                }
            }
            None => {
                // Never written by any client: legacy pins become the shared
                // truth once agents are known to resolve them (rebuild flushes
                // the push).
                if !self.pinned.is_empty() {
                    self.pins_push_pending = true;
                    self.rebuild_harpoon();
                }
            }
        }
    }

    /// Teleport to the 1-based harpoon slot, if it's filled.
    pub(in crate::app) fn harpoon_jump(&mut self, slot: usize) {
        let Some(sid) = slot
            .checked_sub(1)
            .and_then(|i| self.harpoon.get(i))
            .cloned()
        else {
            self.set_toast(format!("no agent pinned at {slot}"));
            return;
        };
        self.open_single(sid, true);
    }

    /// Jump to the alternate agent (the one focused just before this one).
    pub(in crate::app) fn alt_agent(&mut self) {
        let Some(alt) = self.prev_focus.clone() else {
            self.set_toast("no alternate agent");
            return;
        };
        if !self.all_agents.iter().any(|a| a.session_id == alt) {
            self.set_toast("alternate agent is gone");
            return;
        }
        self.open_single(alt, true);
    }

    /// Step back / forward through the jump history.
    pub(in crate::app) fn jump_history(&mut self, delta: i32) {
        if self.jumplist.is_empty() {
            return;
        }
        let target = self.jump_idx as i32 + delta;
        if target < 0 || target as usize >= self.jumplist.len() {
            self.set_toast(if delta < 0 {
                "start of jumps"
            } else {
                "end of jumps"
            });
            return;
        }
        self.jump_idx = target as usize;
        let id = self.jumplist[self.jump_idx].clone();
        self.open_single(id, false);
    }

    // ── window splits (panes) ─────────────────────────────────────────────

    pub(in crate::app) fn close_chat(&mut self) {
        // Leave the terminal warm in the background so coming back is instant.
        self.view = View::List;
        self.chat_mode = ChatMode::Terminal;
        self.term_attached = false;
        self.insert_mode = false;
        self.input.clear();
        self.pending_echo = None;
        self.invalidate_transcript_cache();
        self.tiles.clear();
        self.tile_focus = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::input::testutil::*;

    #[test]
    fn adopt_shared_pins_makes_the_config_document_the_truth() {
        let mut app = app_with_agents(2);
        app.harpoon_toggle(); // local pin: session s1
                              // The desktop pinned a different set while we ran — shared wins.
        app.adopt_shared_pins(Some(vec!["s2".to_string()]));
        assert_eq!(app.pinned, vec!["s2"]);
        assert_eq!(app.harpoon, vec!["s2"]);
        // An explicit empty list is a real answer (everything unpinned)…
        app.adopt_shared_pins(Some(Vec::new()));
        assert!(app.pinned.is_empty());
    }

    #[test]
    fn adopt_shared_pins_resolves_legacy_cwd_values_to_sessions() {
        // The shared key may still carry cwds written by the deprecated key's
        // brief life — normalize upgrades them against the live agents.
        let mut app = app_with_agents(2);
        app.adopt_shared_pins(Some(vec![
            "/work/s2".to_string(),
            "/nowhere/gone".to_string(),
        ]));
        assert_eq!(app.pinned, vec!["s2"], "cwd resolved, dead path dropped");
        assert_eq!(app.harpoon, vec!["s2"]);
    }

    #[tokio::test]
    async fn adopt_shared_pins_keeps_legacy_pins_when_the_key_was_never_written() {
        let mut app = app_with_agents(2);
        app.harpoon_toggle(); // legacy-era local pin
                              // …but an ABSENT key must not wipe them: they're the migration source.
        app.adopt_shared_pins(None);
        assert_eq!(app.pinned, vec!["s1"]);
    }

    #[test]
    fn harpoon_toggle_pins_then_unpins_the_selected_agent() {
        let mut app = app_with_agents(2);
        app.harpoon_toggle();
        assert_eq!(app.pinned, vec!["s1"], "pinned by session id, not cwd");
        app.harpoon_toggle();
        assert!(app.pinned.is_empty(), "toggling again unpins");
    }

    #[tokio::test]
    async fn harpoon_jump_to_an_empty_slot_goes_nowhere() {
        let mut app = app_with_agents(2);
        app.harpoon_jump(1);
        assert!(
            matches!(app.view, View::List),
            "nothing pinned at 1, so no agent opens"
        );
        // Slot 0 doesn't exist either — the slots are 1-based.
        app.harpoon_jump(0);
        assert!(matches!(app.view, View::List));
    }

    #[tokio::test]
    async fn alt_agent_returns_to_the_previously_focused_one() {
        let mut app = app_with_agents(2);
        app.open_single("s1".into(), true);
        app.open_single("s2".into(), true);
        assert_eq!(app.prev_focus.as_deref(), Some("s1"));

        app.alt_agent();
        assert_eq!(app.open_session_id().as_deref(), Some("s1"));
    }

    #[tokio::test]
    async fn alt_agent_declines_when_the_alternate_has_gone_away() {
        let mut app = app_with_agents(2);
        app.open_single("s1".into(), true);
        app.open_single("s2".into(), true);
        // s1 disappears from the fleet while we are on s2.
        app.set_agents(vec![agent("s2", "responding")]);

        app.alt_agent();
        assert_eq!(
            app.open_session_id().as_deref(),
            Some("s2"),
            "stays put rather than opening a dead session"
        );
    }

    #[tokio::test]
    async fn jump_history_walks_the_list_and_stops_at_both_ends() {
        let mut app = app_with_agents(3);
        app.open_single("s1".into(), true);
        app.open_single("s2".into(), true);
        app.open_single("s3".into(), true);
        let end = app.jump_idx;

        app.jump_history(-1);
        assert_eq!(app.jump_idx, end - 1, "steps back one");

        // Walk off the front and stay there.
        for _ in 0..10 {
            app.jump_history(-1);
        }
        assert_eq!(app.jump_idx, 0, "clamped at the start of jumps");

        for _ in 0..20 {
            app.jump_history(1);
        }
        assert_eq!(
            app.jump_idx,
            app.jumplist.len() - 1,
            "clamped at the end of jumps"
        );
    }

    #[test]
    fn jump_history_is_a_noop_with_an_empty_jumplist() {
        let mut app = app_with_agents(2);
        assert!(app.jumplist.is_empty());
        app.jump_history(-1);
        app.jump_history(1);
        assert_eq!(app.jump_idx, 0);
    }
}
