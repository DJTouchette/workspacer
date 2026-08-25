//! Modal forms: rename, the notes scratchpad, and the spawn form with the
//! agent-launch paths behind it.

use super::dispatch::is_ctrl_c;
use super::*;

impl App {
    /// Open the rename overlay for the targeted agent, prefilled with its
    /// current custom name (if any).
    pub(in crate::app) fn open_rename(&mut self) {
        let Some(agent) = self.target_agent() else {
            return;
        };
        // Names are keyed by cwd in a LOCAL file; a remote cwd names the peer's
        // filesystem (and its home hub already owns the session's label).
        if agent.is_remote() {
            self.set_toast("remote session — rename it on its own machine");
            return;
        }
        let cwd = agent.cwd_str().to_string();
        if cwd.is_empty() {
            self.set_toast("no working directory to name");
            return;
        }
        let input = self.names.get(&cwd).cloned().unwrap_or_default();
        self.rename = Some(RenameForm { cwd, input });
    }

    pub(in crate::app) fn handle_rename_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.rename = None,
            KeyCode::Enter => self.submit_rename(),
            KeyCode::Backspace => {
                if let Some(f) = self.rename.as_mut() {
                    f.input.pop();
                }
            }
            KeyCode::Char(c) => {
                if let Some(f) = self.rename.as_mut() {
                    f.input.push(c);
                }
            }
            _ => {}
        }
    }

    /// Save the rename: store (or clear, when blank) the cwd's custom name and
    /// persist the map.
    pub(in crate::app) fn submit_rename(&mut self) {
        let Some(form) = self.rename.take() else {
            return;
        };
        let name = form.input.trim().to_string();
        if name.is_empty() {
            self.names.remove(&form.cwd);
        } else {
            self.names.insert(form.cwd.clone(), name);
        }
        // The toast must report what actually happened: a success message for a
        // write that returned ENOENT is how every rename on a TUI-only machine
        // vanished on the next launch without a word.
        let msg = crate::store::save_toast("Renamed", "rename", crate::names::save(&self.names));
        self.set_toast(msg);
    }

    // ── notes scratchpad ────────────────────────────────────────────────────

    pub(in crate::app) fn open_notes(&mut self) {
        let Some(agent) = self.target_agent() else {
            return;
        };
        // Notes are keyed by cwd, and a remote cwd could collide with a local
        // repo path — the note would silently attach to the wrong project.
        if agent.is_remote() {
            self.set_toast("remote session — notes are local-only");
            return;
        }
        let cwd = agent.cwd_str().to_string();
        if cwd.is_empty() {
            self.set_toast("no working directory for notes");
            return;
        }
        let text = self.notes.get(&cwd).cloned().unwrap_or_default();
        self.notes_view = Some(NotesState {
            cwd,
            text,
            editing: false,
            scroll: 0,
        });
    }

    pub(in crate::app) fn handle_notes_key(&mut self, key: KeyEvent) {
        let editing = self.notes_view.as_ref().is_some_and(|n| n.editing);
        if editing {
            match key.code {
                // esc leaves edit mode (stays in the pane) and saves.
                KeyCode::Esc => {
                    if let Some(n) = self.notes_view.as_mut() {
                        n.editing = false;
                    }
                    self.save_notes();
                }
                KeyCode::Enter => {
                    if let Some(n) = self.notes_view.as_mut() {
                        n.text.push('\n');
                    }
                }
                KeyCode::Backspace => {
                    if let Some(n) = self.notes_view.as_mut() {
                        n.text.pop();
                    }
                }
                KeyCode::Char(c) => {
                    if let Some(n) = self.notes_view.as_mut() {
                        n.text.push(c);
                    }
                }
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Char('i') | KeyCode::Char('e') | KeyCode::Enter => {
                if let Some(n) = self.notes_view.as_mut() {
                    n.editing = true;
                }
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(n) = self.notes_view.as_mut() {
                    n.scroll = n.scroll.saturating_add(1);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(n) = self.notes_view.as_mut() {
                    n.scroll = n.scroll.saturating_sub(1);
                }
            }
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('h') => self.close_notes(),
            _ => {}
        }
    }

    /// The remote-nodes overlay.
    ///
    /// **The confirmation branch is the safety property, not decoration.** A
    /// wake starts a billable machine that this hub has no verb to stop, so
    /// while a confirmation is armed EVERY key that is not the confirm key
    /// stands it down — including `j`/`k`, `enter` and `w` itself. That is
    /// deliberately stricter than "esc cancels": the failure this guards
    /// against is a keystroke aimed at something else landing here, and a
    /// navigation key that silently left the confirmation armed would be
    /// exactly that trap. `y` is the confirm key because `y` already means
    /// approve everywhere else in this TUI.
    pub(in crate::app) fn handle_nodes_key(&mut self, key: KeyEvent) {
        // Ctrl-C still quits, so an overlay can never trap the user — and it
        // reads as "get me out", never as consent.
        if is_ctrl_c(&key) {
            self.cancel_wake();
            self.should_quit = true;
            return;
        }
        if self
            .nodes_view
            .as_ref()
            .is_some_and(|n| n.confirm.is_some())
        {
            match key.code {
                KeyCode::Char('y') => self.confirm_wake(),
                _ => self.cancel_wake(),
            }
            return;
        }
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.nodes_select_next(),
            KeyCode::Char('k') | KeyCode::Up => self.nodes_select_prev(),
            // Arms the confirmation; spends nothing. See `App::request_wake`.
            KeyCode::Char('w') => self.request_wake(),
            KeyCode::Char('r') => self.seed_nodes(),
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('h') | KeyCode::Char('N') => {
                self.close_nodes()
            }
            _ => {}
        }
    }

    pub(in crate::app) fn close_notes(&mut self) {
        self.save_notes();
        self.notes_view = None;
    }

    /// Persist the open note (clearing the entry when blank).
    pub(in crate::app) fn save_notes(&mut self) {
        let Some(n) = self.notes_view.as_ref() else {
            return;
        };
        let cwd = n.cwd.clone();
        let text = n.text.trim_end().to_string();
        if text.is_empty() {
            self.notes.remove(&cwd);
        } else {
            self.notes.insert(cwd, text);
        }
        if let Err(e) = crate::notes::save(&self.notes) {
            self.set_toast(crate::store::save_toast("", "note", Err(e)));
        }
    }

    pub(in crate::app) fn handle_spawn_key(&mut self, key: KeyEvent) {
        let n = self.profiles.len();
        let np = crate::app::SPAWN_PROVIDERS.len();
        let Some(form) = self.spawn_form.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.spawn_form = None,
            KeyCode::Enter => self.submit_spawn(),
            // Shell-style path completion on the cwd field.
            KeyCode::Tab => complete_path(form),
            // ←/→ cycle the provider; ↑/↓ cycle the (claude) profile.
            KeyCode::Right => form.provider_idx = (form.provider_idx + 1) % np,
            KeyCode::Left => form.provider_idx = (form.provider_idx + np - 1) % np,
            KeyCode::Down => {
                if n > 0 {
                    form.profile_idx = (form.profile_idx + 1) % n;
                }
            }
            KeyCode::Up => {
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

    /// Open the spawn modal, prefilling the cwd with where the TUI was launched.
    pub(in crate::app) fn open_spawn(&mut self) {
        self.open_spawn_inner(None);
    }

    /// Open the spawn modal carrying a prompt to seed into the new agent.
    pub(in crate::app) fn open_spawn_with_prompt(&mut self, prompt: String) {
        self.open_spawn_inner(Some(prompt));
    }

    pub(in crate::app) fn open_spawn_inner(&mut self, initial_prompt: Option<String>) {
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        self.spawn_form = Some(SpawnForm {
            cwd,
            profile_idx: 0,
            provider_idx: 0,
            completions: Vec::new(),
            initial_prompt,
        });
    }

    /// Spawn a Claude session in the chosen cwd with the chosen profile, via
    /// claudemon's REST API. The new agent surfaces in the sidebar on the next
    /// state-change event (claudemon emits one once Claude starts up).
    pub(in crate::app) fn submit_spawn(&mut self) {
        let Some(form) = self.spawn_form.clone() else {
            return;
        };
        // Tilde expansion happens HERE, where a human typed the keystrokes — not
        // in normalize_cwd, which is the seam normalizer the two other providers
        // share and which BINDING DECISION 1 forbids from expanding anything
        // (contracts/path-containment-cases.json, spawnCwds). The dialog is a
        // local picker, so `~/proj` should mean what the user's shell means by
        // it; the string that then goes to claudemon is normalized by the same
        // rule the brain and the desktop apply.
        let cwd = profiles::normalize_cwd(&profiles::expand_tilde(&form.cwd));
        if cwd.is_empty() {
            self.set_toast("working directory required");
            return;
        }
        let provider = crate::app::SPAWN_PROVIDERS
            .get(form.provider_idx)
            .copied()
            .unwrap_or("claude");
        let initial_prompt = form.initial_prompt.clone();
        self.spawn_form = None;
        if provider == "claude" {
            let Some(profile) = self.profiles.get(form.profile_idx).cloned() else {
                self.set_toast("no profile selected");
                return;
            };
            self.spawn_agent_in(cwd, profile, initial_prompt, None);
        } else {
            self.spawn_managed_agent_in(provider, cwd, initial_prompt);
        }
    }

    /// Spawn a managed (Codex/OpenCode/Pi) agent via `/sessions/spawn-managed`,
    /// record its provider (so the model picker + permission-mode cycle pick the
    /// managed behaviour), and optionally seed a first prompt once it's up.
    pub(in crate::app) fn spawn_managed_agent_in(
        &self,
        provider: &str,
        cwd: String,
        initial_prompt: Option<String>,
    ) {
        let drv = self.driver();
        let tx = self.tx.clone();
        let provider = provider.to_string();
        tokio::spawn(async move {
            match drv.spawn_managed(&provider, &cwd, None, None, false).await {
                Ok(sid) => {
                    let _ = tx.send(AppMsg::ManagedSpawned {
                        session_id: sid.clone(),
                        provider: provider.clone(),
                    });
                    let _ = tx.send(AppMsg::Toast(format!("Spawned {provider} agent")));
                    fetch_agents(&drv.claudemon, &tx).await;
                    if let Some(prompt) = initial_prompt {
                        // Managed adapters boot async; the message pipeline queues
                        // until the agent is ready.
                        let _ = drv.message(&sid, &prompt).await;
                    }
                }
                Err(e) => {
                    let _ = tx.send(AppMsg::Toast(format!("Spawn failed: {e}")));
                }
            }
        });
    }

    /// Spawn a fresh Claude session in `cwd` with `profile`, optionally seeding a
    /// prompt once it reaches its input prompt. Shared by the spawn modal and the
    /// respawn action.
    pub(in crate::app) fn spawn_agent_in(
        &self,
        cwd: String,
        profile: profiles::Profile,
        initial_prompt: Option<String>,
        resume_session_id: Option<String>,
    ) {
        // Resuming reuses the prior session id (which is also claude's transcript
        // uuid, since we pin `--session-id` at spawn) and passes it as `--resume`
        // so claude reopens that conversation. A fresh spawn mints a new id and
        // pins it up front so claude's transcript file, claudemon's id, and the
        // id we track all agree — no cwd-based guessing.
        // The driver builds the argv (claudemon-direct) or hands the profile id to
        // the brain (bus mode); either way it pins/returns the session id.
        let resume = resume_session_id.is_some();
        // A resume respawns a session that already exists, so it keeps that
        // row's transport; the configured transport is for fresh spawns.
        let drv = match &resume_session_id {
            Some(sid) => self.driver_on(self.respawn_transport(sid)),
            None => self.driver(),
        };
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let sid = match drv.spawn(cwd, &profile, resume_session_id).await {
                Ok(sid) => {
                    let verb = if resume { "Resumed" } else { "Spawned" };
                    let _ = tx.send(AppMsg::Toast(format!("{verb} agent")));
                    fetch_agents(&drv.claudemon, &tx).await;
                    sid
                }
                Err(e) => {
                    let _ = tx.send(AppMsg::Toast(format!("Spawn failed: {e}")));
                    return;
                }
            };
            if let Some(prompt) = initial_prompt {
                seed_prompt(&drv.claudemon, &tx, &sid, &prompt).await;
            }
        });
    }

    /// Restart a stopped agent by resuming its conversation: spawn
    /// `claude --resume <id>` in its cwd with the default profile. The id is the
    /// agent's own session id, which doubles as claude's transcript uuid (we pin
    /// `--session-id` at spawn), so claude reopens the prior conversation instead
    /// of starting blank. (claudemon keeps the old stopped session in its list
    /// until it's pruned; this adds a live one in the same directory.)
    pub(in crate::app) fn respawn(&mut self) {
        let Some(agent) = self.target_agent() else {
            return;
        };
        // A respawn would launch a NEW process on THIS machine in a directory
        // that names the peer's filesystem — spawning stays local-only.
        if agent.is_remote() {
            self.set_toast("remote session — respawn it on its own machine");
            return;
        }
        if agent.state() != "stopped" {
            self.set_toast("agent is still running");
            return;
        }
        let session_id = agent.session_id.clone();
        let cwd = agent.cwd_str().to_string();
        if cwd.is_empty() {
            self.set_toast("no working directory");
            return;
        }
        let Some(profile) = self
            .profiles
            .iter()
            .find(|p| p.is_default)
            .or_else(|| self.profiles.first())
            .cloned()
        else {
            self.set_toast("no profile available");
            return;
        };
        self.set_toast("Resuming…");
        self.spawn_agent_in(cwd, profile, None, Some(session_id));
    }
}
