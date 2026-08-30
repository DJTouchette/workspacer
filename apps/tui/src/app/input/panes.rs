//! Window splits and tabs inside an agent workspace, plus the key handlers
//! for the surfaces they host (terminal, changes).

use super::*;

impl App {
    /// First agent not already tiled, for bringing another agent into view.
    /// Uses the full set so splits can pull in agents the `/` filter hides, but
    /// skips shells (they aren't standalone agents).
    pub(in crate::app) fn next_untiled_agent(&self) -> Option<String> {
        self.all_agents
            .iter()
            .map(|a| &a.session_id)
            .find(|sid| !self.tiles.contains(sid) && !self.is_shell_session(sid))
            .cloned()
    }

    /// Split the focused pane: tile another agent beside/below it and move focus
    /// to the new pane (vim's `Ctrl-w v` / `Ctrl-w s`). No-op outside an agent
    /// view; capped so cells stay usable.
    pub(in crate::app) fn split_pane(&mut self, dir: SplitDir) {
        const MAX_TILES: usize = 4;
        if self.open_agent_id().is_none() {
            return;
        }
        if self.tiles.len() >= MAX_TILES {
            self.set_toast("max splits reached");
            return;
        }
        let Some(next) = self.next_untiled_agent() else {
            self.set_toast("no other agent to split");
            return;
        };
        self.split_dir = dir;
        let at = (self.tile_focus + 1).min(self.tiles.len());
        self.tiles.insert(at, next.clone());
        self.tile_focus = at;
        self.focus_agent(next);
    }

    /// Move focus to another tiled pane (wrapping). `delta` is +1 / -1.
    pub(in crate::app) fn focus_pane(&mut self, delta: i32) {
        // A docked pane is the last stop in the cycle: from the agent tiles,
        // `Ctrl-w l` steps into it rather than reporting nothing to do.
        if self.side.is_some() && !self.side_focused() {
            self.focus_side(true);
            return;
        }
        let n = self.tiles.len();
        if n <= 1 {
            return;
        }
        let next = (self.tile_focus as i32 + delta).rem_euclid(n as i32) as usize;
        self.tile_focus = next;
        let id = self.tiles[next].clone();
        self.focus_agent(id);
    }

    /// Close the focused pane. The last pane closing leaves the agent view.
    pub(in crate::app) fn close_pane(&mut self) {
        if self.tiles.len() <= 1 {
            self.close_chat();
            return;
        }
        self.tiles.remove(self.tile_focus);
        if self.tile_focus >= self.tiles.len() {
            self.tile_focus = self.tiles.len() - 1;
        }
        let id = self.tiles[self.tile_focus].clone();
        self.focus_agent(id);
    }

    /// Keep only the focused pane (vim's `Ctrl-w o`).
    pub(in crate::app) fn only_pane(&mut self) {
        if self.tiles.len() <= 1 {
            return;
        }
        let id = self.tiles[self.tile_focus].clone();
        self.tiles = vec![id];
        self.tile_focus = 0;
    }

    /// Set up rendering for whatever the active tab points at: warm its terminal
    /// (or fall back to transcript for no-PTY sessions).
    pub(in crate::app) fn enter_active_tab(&mut self) {
        self.chat_scroll = 0;
        self.chat_follow = true;
        self.insert_mode = false;
        self.term_attached = false;
        self.pending_echo = None;
        self.invalidate_transcript_cache();
        let Some(tab) = self.active_tab().cloned() else {
            return;
        };
        // Headless stream sessions, known no-PTY sessions, and remote
        // (peer-hub) sessions are proactively transcript-only — never warm a
        // PTY stream that can't exist here.
        let transcript_only = tab.kind == TabKind::Claude
            && (self.no_terminal.contains(&tab.session_id)
                || self.is_stream_session(&tab.session_id)
                || self.is_remote_session(&tab.session_id));
        if transcript_only {
            self.chat_mode = ChatMode::Transcript;
            self.load_transcript(tab.session_id);
        } else {
            self.chat_mode = ChatMode::Terminal;
            self.ensure_terminal(tab.session_id);
        }
    }

    pub(in crate::app) fn tab_next(&mut self) {
        if let Some(ws) = self.workspace_mut() {
            if !ws.tabs.is_empty() {
                ws.active = (ws.active + 1) % ws.tabs.len();
            }
        }
        self.enter_active_tab();
    }

    pub(in crate::app) fn tab_prev(&mut self) {
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
    pub(in crate::app) fn new_terminal_tab(&mut self) {
        let Some(id) = self.open_agent_id().map(|s| s.to_string()) else {
            return;
        };
        // A shell would spawn on THIS machine in a directory that names the
        // peer's filesystem — hidden rather than failing on use.
        if self.is_remote_session(&id) {
            self.set_toast("remote session — terminals are local-only");
            return;
        }
        let cwd = self
            .chat_agent()
            .map(|a| a.cwd_str().to_string())
            .filter(|c| !c.is_empty())
            .or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|p| p.display().to_string())
            })
            .unwrap_or_else(|| "/".into());
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
        let cm = self.claudemon.clone();
        let tx = self.tx.clone();
        self.set_toast("Opening terminal…");
        tokio::spawn(async move {
            match cm.spawn(vec![shell], cwd, serde_json::Map::new(), "").await {
                Ok(sid) => {
                    let _ = tx.send(AppMsg::ShellSpawned {
                        agent_id: id,
                        session_id: sid,
                    });
                }
                Err(e) => {
                    let _ = tx.send(AppMsg::Toast(format!("Terminal failed: {e}")));
                }
            }
        });
    }

    /// Close the active tab. Closing the primary Claude tab leaves the agent
    /// (back to the list); closing a shell tab stops that shell.
    pub(in crate::app) fn close_tab(&mut self) {
        let Some(ws) = self.workspace() else { return };
        let Some(tab) = ws.active_tab().cloned() else {
            return;
        };
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

    /// Forward a keystroke to the PTY, or detach on Ctrl-].
    pub(in crate::app) fn handle_terminal_key(&mut self, key: KeyEvent) {
        if crate::terminal::is_detach(&key) {
            self.term_attached = false;
            return;
        }
        let Some(sid) = self.open_session_id() else {
            return;
        };
        let Some(bytes) = crate::terminal::encode_key(&key) else {
            return;
        };
        let drv = self.driver();
        tokio::spawn(async move {
            let _ = drv.terminal_input(&sid, &bytes).await;
        });
    }

    /// Keys for the docked changes pane — read-only, so it only scrolls and closes.
    pub(in crate::app) fn handle_changes_key(&mut self, key: KeyEvent) {
        let scroll = |p: &mut crate::app::SidePane, delta: i32| {
            p.scroll = (p.scroll as i32 + delta).max(0) as u16;
        };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('C') => {
                self.side = None;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(p) = self.side.as_mut() {
                    scroll(p, 1)
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(p) = self.side.as_mut() {
                    scroll(p, -1)
                }
            }
            KeyCode::Char('g') => {
                if let Some(p) = self.side.as_mut() {
                    p.scroll = 0
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::input::testutil::*;

    #[tokio::test]
    async fn split_pane_tiles_the_next_agent_and_focuses_it() {
        let mut app = app_in_agent_view(3).await;
        assert_eq!(app.tiles, vec!["s1"]);

        app.split_pane(SplitDir::Columns);
        assert_eq!(app.tiles, vec!["s1", "s2"], "next untiled agent joins");
        assert_eq!(app.tile_focus, 1, "focus follows the new pane");
        assert_eq!(app.split_dir, SplitDir::Columns);

        // The next split takes the next *untiled* agent, not s2 again.
        app.split_pane(SplitDir::Rows);
        assert_eq!(app.tiles, vec!["s1", "s2", "s3"]);
        assert_eq!(app.tile_focus, 2);
    }

    #[tokio::test]
    async fn split_pane_stops_at_four_tiles() {
        let mut app = app_in_agent_view(6).await;
        for _ in 0..3 {
            app.split_pane(SplitDir::Columns);
        }
        assert_eq!(app.tiles.len(), 4);
        app.split_pane(SplitDir::Columns);
        assert_eq!(app.tiles.len(), 4, "capped so cells stay usable");
    }

    #[tokio::test]
    async fn split_pane_says_so_when_there_is_nobody_left_to_tile() {
        let mut app = app_in_agent_view(1).await;
        app.split_pane(SplitDir::Columns);
        assert_eq!(app.tiles, vec!["s1"], "no second agent to bring in");
    }

    #[tokio::test]
    async fn split_pane_does_nothing_outside_an_agent_view() {
        let mut app = app_with_agents(3); // list view, nothing open
        app.split_pane(SplitDir::Columns);
        assert!(app.tiles.is_empty());
    }

    #[tokio::test]
    async fn focus_pane_wraps_both_ways() {
        let mut app = app_in_agent_view(3).await;
        app.split_pane(SplitDir::Columns);
        app.split_pane(SplitDir::Columns); // tiles = [s1, s2, s3], focus 2

        app.focus_pane(1);
        assert_eq!(app.tile_focus, 0, "past the end wraps to the front");
        app.focus_pane(-1);
        assert_eq!(app.tile_focus, 2, "before the front wraps to the end");
    }

    #[tokio::test]
    async fn focus_pane_is_a_noop_with_a_single_tile() {
        let mut app = app_in_agent_view(2).await;
        app.focus_pane(1);
        assert_eq!(app.tile_focus, 0);
    }

    #[tokio::test]
    async fn close_pane_keeps_the_focus_in_range() {
        let mut app = app_in_agent_view(3).await;
        app.split_pane(SplitDir::Columns);
        app.split_pane(SplitDir::Columns); // [s1, s2, s3], focus 2

        app.close_pane();
        assert_eq!(app.tiles, vec!["s1", "s2"]);
        assert_eq!(app.tile_focus, 1, "clamped to the new last tile");

        app.close_pane();
        assert_eq!(app.tiles, vec!["s1"]);
        assert_eq!(app.tile_focus, 0);
    }

    #[tokio::test]
    async fn closing_the_last_pane_leaves_the_agent_view() {
        let mut app = app_in_agent_view(2).await;
        assert_eq!(app.tiles.len(), 1);
        app.close_pane();
        assert!(
            matches!(app.view, View::List),
            "the last pane closing is close_chat, not an empty tile set"
        );
    }

    #[tokio::test]
    async fn only_pane_keeps_the_focused_one() {
        let mut app = app_in_agent_view(3).await;
        app.split_pane(SplitDir::Columns);
        app.split_pane(SplitDir::Columns);
        app.tile_focus = 1;

        app.only_pane();
        assert_eq!(app.tiles, vec!["s2"], "the focused tile is the survivor");
        assert_eq!(app.tile_focus, 0);
    }

    // ── harpoon / alternate / jumplist ──────────────────────────────────────
}
