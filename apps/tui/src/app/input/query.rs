//! The type-to-filter overlays: cross-agent search, the `/` sidebar filter,
//! the `:` command line, and the command palette.

use super::*;

impl App {
    /// Open the content-search modal and kick off indexing: fetch each non-shell
    /// session's transcript in the background; lines stream in as `SearchEntries`.
    pub(in crate::app) fn open_search(&mut self) {
        let targets: Vec<(String, String, String)> = self
            .all_agents
            .iter()
            .filter(|a| !self.is_shell_session(&a.session_id))
            .map(|a| {
                (
                    a.session_id.clone(),
                    self.agent_name(a),
                    a.transport.clone(),
                )
            })
            .collect();
        self.search = Some(SearchState {
            query: String::new(),
            entries: Vec::new(),
            matched: Vec::new(),
            selected: 0,
            pending: targets.len(),
        });
        for (sid, name, transport) in targets {
            let cm = self.claudemon.clone();
            let tx = self.tx.clone();
            tokio::spawn(async move { fetch_search_index(&cm, &tx, sid, name, transport).await });
        }
    }

    pub(in crate::app) fn handle_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.search = None,
            KeyCode::Enter => {
                let sid = self
                    .search
                    .as_ref()
                    .and_then(|s| s.chosen())
                    .map(|h| h.session_id.clone());
                if let Some(sid) = sid {
                    self.search = None;
                    self.open_session_transcript(sid);
                }
            }
            KeyCode::Down => {
                if let Some(s) = self.search.as_mut() {
                    if !s.matched.is_empty() {
                        s.selected = (s.selected + 1).min(s.matched.len() - 1);
                    }
                }
            }
            KeyCode::Up => {
                if let Some(s) = self.search.as_mut() {
                    s.selected = s.selected.saturating_sub(1);
                }
            }
            KeyCode::Backspace => {
                if let Some(s) = self.search.as_mut() {
                    s.query.pop();
                    s.rematch();
                }
            }
            KeyCode::Char(c) => {
                if let Some(s) = self.search.as_mut() {
                    s.query.push(c);
                    s.rematch();
                }
            }
            _ => {}
        }
    }

    /// Start (or resume) typing the sidebar filter.
    pub(in crate::app) fn open_filter(&mut self) {
        self.filter_editing = true;
        if self.filter.is_none() {
            self.filter = Some(String::new());
        }
    }

    /// Keys while the `/` filter input is active. Live-filters as you type;
    /// `enter` keeps the filter and returns to navigation, `esc` clears it.
    pub(in crate::app) fn handle_filter_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.filter = None;
                self.filter_editing = false;
                self.apply_filter();
            }
            KeyCode::Enter => {
                self.filter_editing = false;
                // An empty query is the same as no filter.
                if self.filter.as_deref().is_some_and(str::is_empty) {
                    self.filter = None;
                }
                self.apply_filter();
            }
            KeyCode::Backspace => {
                if let Some(q) = self.filter.as_mut() {
                    q.pop();
                }
                self.apply_filter();
            }
            KeyCode::Char(c) => {
                if let Some(q) = self.filter.as_mut() {
                    q.push(c);
                }
                self.apply_filter();
            }
            _ => {}
        }
    }

    /// Show / hide stopped (incl. hydrated history) sessions in the sidebar, then
    /// re-pull so the change takes effect immediately.
    pub(in crate::app) fn toggle_stopped(&mut self) {
        self.show_all_sessions = !self.show_all_sessions;
        self.set_toast(if self.show_all_sessions {
            "Showing stopped sessions"
        } else {
            "Hiding stopped sessions"
        });
        self.refresh();
    }

    /// Keys while the `:` command line is open. `enter` runs the command,
    /// `esc` cancels.
    pub(in crate::app) fn handle_cmdline_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.cmdline = None,
            KeyCode::Enter => {
                let cmd = self.cmdline.take().unwrap_or_default();
                self.run_command(&cmd);
            }
            KeyCode::Backspace => {
                if let Some(c) = self.cmdline.as_mut() {
                    c.pop();
                }
            }
            KeyCode::Char(c) => {
                if let Some(s) = self.cmdline.as_mut() {
                    s.push(c);
                }
            }
            _ => {}
        }
    }

    /// Run an ex command. Verbs map to existing actions; unknown ones toast.
    pub(in crate::app) fn run_command(&mut self, cmd: &str) {
        let cmd = cmd.trim();
        if cmd.is_empty() {
            return;
        }
        let (verb, arg) = cmd.split_once(char::is_whitespace).unwrap_or((cmd, ""));
        let arg = arg.trim();
        match verb {
            "q" | "quit" => self.should_quit = true,
            "vs" | "vsplit" => self.split_pane(SplitDir::Columns),
            "sp" | "split" => self.split_pane(SplitDir::Rows),
            "on" | "only" => self.only_pane(),
            "clo" | "close" => self.close_pane(),
            "new" | "spawn" => self.open_spawn(),
            "term" | "terminal" => self.new_terminal_tab(),
            "notes" => self.open_notes(),
            "nodes" => self.open_nodes(),
            "review" => self.open_review(),
            "runs" => self.open_runs(),
            "changes" => self.toggle_side(crate::app::SideKind::Changes),
            "pin" => self.harpoon_toggle(),
            "search" | "grep" => self.open_search(),
            "model" => self.open_model_picker(),
            "perm" | "permission" => self.cycle_permission_mode(),
            "handoff" => self.open_handoff_picker(),
            "help" | "h" => self.help = true,
            "ls" | "dashboard" => {
                self.view = View::List;
                self.selected = 0;
            }
            "rename" => {
                if arg.is_empty() {
                    self.open_rename();
                } else if let Some(cwd) = self.target_agent().map(|a| a.cwd_str().to_string()) {
                    if cwd.is_empty() {
                        self.set_toast("no working directory to name");
                    } else {
                        self.names.insert(cwd, arg.to_string());
                        let msg = crate::store::save_toast(
                            "Renamed",
                            "rename",
                            crate::names::save(&self.names),
                        );
                        self.set_toast(msg);
                    }
                }
            }
            "filter" => {
                if arg.is_empty() {
                    self.open_filter();
                } else {
                    self.filter = Some(arg.to_string());
                    self.filter_editing = false;
                    self.apply_filter();
                }
            }
            other => self.set_toast(format!("unknown command: {other}")),
        }
    }

    // ── sidebar filter (`/`) ──────────────────────────────────────────────

    pub(in crate::app) fn open_palette(&mut self) {
        let mut items = vec![
            PaletteItem {
                label: "New agent".into(),
                hint: "spawn".into(),
                action: PaletteAction::NewAgent,
            },
            PaletteItem {
                label: "New terminal".into(),
                hint: "shell tab".into(),
                action: PaletteAction::NewTerminal,
            },
            PaletteItem {
                label: "Dashboard".into(),
                hint: "overview".into(),
                action: PaletteAction::Dashboard,
            },
        ];
        // Commands — the `:`-line verbs, so Ctrl-K is a real command palette.
        for (verb, desc) in COMMAND_PALETTE {
            items.push(PaletteItem {
                label: format!(": {verb}"),
                hint: (*desc).to_string(),
                action: PaletteAction::Command((*verb).to_string()),
            });
        }
        // Jump to a live agent (the full set, so the palette reaches agents the
        // `/` filter is hiding). The cwd goes in the hint so fuzzy search finds
        // an agent by its path, not just its short name.
        for a in &self.all_agents {
            if self.is_shell_session(&a.session_id) {
                continue; // shells live in their agent's tab bar, not here
            }
            items.push(PaletteItem {
                label: format!("Go to {}", self.agent_name(a)),
                hint: format!("{}  {}", a.state(), a.cwd_str()),
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
        self.palette = Some(crate::app::Palette::new(items));
    }

    pub(in crate::app) fn handle_palette_key(&mut self, key: KeyEvent) {
        let Some(p) = self.palette.as_mut() else {
            return;
        };
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

    pub(in crate::app) fn run_palette_action(&mut self, action: PaletteAction) {
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
                // Open by id (works even if the `/` filter is hiding it).
                if self.all_agents.iter().any(|a| a.session_id == sid) {
                    self.open_single(sid, true);
                }
            }
            PaletteAction::Insert(body) => {
                let Some(sid) = self.open_session_id() else {
                    self.set_toast("open an agent's terminal to insert");
                    return;
                };
                let drv = self.driver();
                let bytes = bracketed_paste(&body);
                self.set_toast("Inserted");
                tokio::spawn(async move {
                    let _ = drv.terminal_input(&sid, &bytes).await;
                });
            }
            PaletteAction::SpawnWithPrompt(body) => self.open_spawn_with_prompt(body),
            PaletteAction::Command(cmd) => self.run_command(&cmd),
        }
    }

    // ── model / handoff picker ────────────────────────────────────────────

    /// Jump to a session and show its transcript (content search lands on text,
    /// not the raw terminal).
    pub(in crate::app) fn open_session_transcript(&mut self, sid: String) {
        self.open_single(sid, true);
        if let Some(open) = self.chat_session_id() {
            self.chat_mode = ChatMode::Transcript;
            self.term_attached = false;
            self.chat_follow = true;
            self.load_transcript(open);
        }
    }

    // ── ex command line (`:`) ─────────────────────────────────────────────
}

#[cfg(test)]
mod tests {
    use crate::app::input::testutil::*;
    use crossterm::event::KeyCode;

    #[test]
    fn slash_opens_the_filter_and_typing_edits_it() {
        let mut app = app_with_agents(2);
        app.handle_key(ch('/'));
        assert!(app.filter_editing, "/ starts editing the sidebar filter");
        feed(&mut app, "s1");
        assert_eq!(
            app.filter.as_deref(),
            Some("s1"),
            "characters extend the query"
        );
        app.handle_key(code(KeyCode::Backspace));
        assert_eq!(app.filter.as_deref(), Some("s"), "backspace trims it");
    }

    #[test]
    fn filter_enter_keeps_it_and_esc_clears_it() {
        let mut app = app_with_agents(2);
        app.handle_key(ch('/'));
        feed(&mut app, "s1");
        app.handle_key(code(KeyCode::Enter));
        assert!(!app.filter_editing, "enter stops editing");
        assert_eq!(app.filter.as_deref(), Some("s1"), "but keeps the filter");

        app.handle_key(ch('/'));
        app.handle_key(code(KeyCode::Esc));
        assert!(
            !app.filter_editing && app.filter.is_none(),
            "esc clears the filter entirely"
        );
    }

    // ── `:` ex-command line entry / exit ────────────────────────────────────

    #[test]
    fn colon_opens_the_cmdline() {
        let mut app = app_with_agents(1);
        app.handle_key(ch(':'));
        assert_eq!(
            app.cmdline.as_deref(),
            Some(""),
            ": opens an empty command line"
        );
    }

    #[test]
    fn cmdline_esc_cancels_without_running() {
        let mut app = app_with_agents(1);
        app.handle_key(ch(':'));
        feed(&mut app, "q");
        app.handle_key(code(KeyCode::Esc));
        assert!(app.cmdline.is_none(), "esc closes the command line");
        assert!(!app.should_quit, "and the typed command never ran");
    }

    #[test]
    fn cmdline_enter_runs_the_command() {
        let mut app = app_with_agents(1);
        app.handle_key(ch(':'));
        feed(&mut app, "q");
        app.handle_key(code(KeyCode::Enter));
        assert!(app.cmdline.is_none(), "enter closes the command line");
        assert!(app.should_quit, ":q quits");
    }

    // ── question-mode digit gating ──────────────────────────────────────────
}
