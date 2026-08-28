//! The model and handoff pickers, and the permission-mode cycle — the
//! overlays that change what the *session* is rather than what is on screen.

use super::*;

impl App {
    /// Open the live model-switch picker for the target session. Managed
    /// sessions (codex/opencode/pi this TUI spawned) fetch their launchable
    /// models from the daemon; claude/unknown sessions get a free-text field
    /// (their model switches via a `/model` slash command on the message path).
    pub(in crate::app) fn open_model_picker(&mut self) {
        let Some(sid) = self.target_session() else {
            self.set_toast("no agent selected");
            return;
        };
        // `claude.setModel` / provider model lists aren't routed across
        // federation — hidden rather than failing on apply.
        if self.is_remote_session(&sid) {
            self.set_toast("remote session — switch its model on its own machine");
            return;
        }
        let provider = self.provider_for(&sid);
        let managed = provider != "claude";
        let cwd = self
            .target_agent()
            .and_then(|a| a.cwd.clone())
            .unwrap_or_default();
        self.picker = Some(Picker {
            title: format!("model · {provider}"),
            kind: PickerKind::Model {
                provider: provider.clone(),
                effort: None,
            },
            session_id: sid.clone(),
            query: String::new(),
            items: Vec::new(),
            matched: Vec::new(),
            selected: 0,
            pending: managed,
            allow_free_text: true,
        });
        if managed {
            let cm = self.claudemon.clone();
            let tx = self.tx.clone();
            tokio::spawn(async move {
                let models = cm
                    .provider_models(&provider, &cwd)
                    .await
                    .unwrap_or_default();
                let _ = tx.send(AppMsg::PickerModels {
                    session_id: sid,
                    models,
                });
            });
        }
    }

    /// Open the handoff provider chooser for the target session: pick who takes
    /// over, then build a brief and spawn that provider primed to read it.
    pub(in crate::app) fn open_handoff_picker(&mut self) {
        let Some(sid) = self.target_session() else {
            self.set_toast("no agent selected");
            return;
        };
        // A handoff spawns the successor HERE, in a cwd that names the peer's
        // filesystem — spawning stays local-only.
        if self.is_remote_session(&sid) {
            self.set_toast("remote session — hand off on its own machine");
            return;
        }
        let Some(cwd) = self
            .target_agent()
            .and_then(|a| a.cwd.clone())
            .filter(|c| !c.is_empty())
        else {
            self.set_toast("no working directory for a handoff");
            return;
        };
        let items: Vec<PickerItem> = ["claude", "codex", "copilot", "opencode", "pi"]
            .iter()
            .map(|p| PickerItem {
                id: (*p).to_string(),
                label: (*p).to_string(),
            })
            .collect();
        let mut picker = Picker {
            title: "hand off to".into(),
            kind: PickerKind::Handoff { cwd },
            session_id: sid,
            query: String::new(),
            items,
            matched: Vec::new(),
            selected: 0,
            pending: false,
            allow_free_text: false,
        };
        picker.rematch();
        self.picker = Some(picker);
    }

    pub(in crate::app) fn handle_picker_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.picker = None,
            KeyCode::Enter => self.submit_picker(),
            KeyCode::Down => {
                if let Some(p) = self.picker.as_mut() {
                    if !p.matched.is_empty() {
                        p.selected = (p.selected + 1).min(p.matched.len() - 1);
                    }
                }
            }
            KeyCode::Up => {
                if let Some(p) = self.picker.as_mut() {
                    p.selected = p.selected.saturating_sub(1);
                }
            }
            KeyCode::Backspace => {
                if let Some(p) = self.picker.as_mut() {
                    p.query.pop();
                    p.rematch();
                }
            }
            KeyCode::Char(c) => {
                if let Some(p) = self.picker.as_mut() {
                    p.query.push(c);
                    p.rematch();
                }
            }
            _ => {}
        }
    }

    /// Apply the picker's selection: switch the model, or run the handoff.
    pub(in crate::app) fn submit_picker(&mut self) {
        let Some(p) = self.picker.take() else { return };
        // A highlighted list row wins; free text (the model picker) is the fallback.
        let chosen_id = p.chosen().map(|it| it.id.clone());
        let Picker {
            kind,
            session_id,
            query,
            ..
        } = p;
        match kind {
            PickerKind::Model { provider, effort } => {
                let model = chosen_id.or_else(|| {
                    let q = query.trim();
                    (!q.is_empty()).then(|| q.to_string())
                });
                let Some(model) = model else {
                    self.set_toast("no model chosen");
                    return;
                };
                self.apply_model_switch(session_id, provider, effort, model);
            }
            PickerKind::Handoff { cwd } => {
                let Some(target) = chosen_id else { return };
                self.do_handoff(session_id, cwd, target);
            }
        }
    }

    /// Push a model switch: managed providers hit `POST /model`; claude/unknown
    /// sessions send a `/model <id>` slash command on the message path (their PTY
    /// 409s the endpoint).
    pub(in crate::app) fn apply_model_switch(
        &mut self,
        sid: String,
        provider: String,
        effort: Option<String>,
        model: String,
    ) {
        let drv = self.driver();
        if provider == "claude" {
            let msg = format!("/model {model}");
            self.dispatch(
                "Model switch sent",
                async move { drv.message(&sid, &msg).await },
            );
        } else {
            self.dispatch("Model switched", async move {
                drv.set_model(&sid, Some(&model), effort.as_deref()).await
            });
        }
    }

    /// Build a handoff brief from `sid`, then spawn `target` (in `cwd`) primed to
    /// read it and continue — any harness → any harness. A claude successor seeds
    /// the brief into its composer (pasted, unsent) like the library-spawn flow;
    /// a managed successor receives it as its first message.
    pub(in crate::app) fn do_handoff(&mut self, sid: String, cwd: String, target: String) {
        let drv = self.driver();
        let tx = self.tx.clone();
        let default_profile = self
            .profiles
            .iter()
            .find(|p| p.is_default)
            .or_else(|| self.profiles.first())
            .cloned();
        self.set_toast("Building handoff brief…");
        tokio::spawn(async move {
            let brief = match drv.handoff(&sid).await {
                Ok(b) => b,
                Err(e) => {
                    let _ = tx.send(AppMsg::Toast(format!("Handoff failed: {e}")));
                    return;
                }
            };
            let path = brief.path.unwrap_or_default();
            let prompt = format!(
                "You are taking over an in-progress session from another AI coding agent. \
                 First read the handoff brief at {path}, then continue the work from where it \
                 left off — don't start over or redo completed steps. Reply with a one-paragraph \
                 summary of the state and your next step."
            );
            if target == "claude" {
                let Some(profile) = default_profile else {
                    let _ = tx.send(AppMsg::Toast("no claude profile to hand off to".into()));
                    return;
                };
                match drv.spawn(cwd, &profile, None).await {
                    Ok(new_sid) => {
                        let _ = tx.send(AppMsg::Toast(format!("Handed off → {target}")));
                        seed_prompt(&drv.claudemon, &tx, &new_sid, &prompt).await;
                    }
                    Err(e) => {
                        let _ = tx.send(AppMsg::Toast(format!("Successor spawn failed: {e}")));
                    }
                }
            } else {
                match drv.spawn_managed(&target, &cwd, None, None, false).await {
                    Ok(new_sid) => {
                        let _ = tx.send(AppMsg::ManagedSpawned {
                            session_id: new_sid.clone(),
                            provider: target.clone(),
                        });
                        let _ = tx.send(AppMsg::Toast(format!("Handed off → {target}")));
                        // Managed adapters boot asynchronously; the message
                        // pipeline queues until the agent is ready, so this lands.
                        let _ = drv.message(&new_sid, &prompt).await;
                        fetch_agents(&drv.claudemon, &tx).await;
                    }
                    Err(e) => {
                        let _ = tx.send(AppMsg::Toast(format!("Successor spawn failed: {e}")));
                    }
                }
            }
        });
    }

    /// Cycle the target session's permission mode one step and push it to the
    /// daemon. Managed sessions cycle ask⇄yolo; PTY (claude) sessions cycle
    /// default→acceptEdits→plan. A capability cliff (yolo→ask when spawned in
    /// bypass, opencode/pi) surfaces as a toast rather than crashing.
    pub(in crate::app) fn cycle_permission_mode(&mut self) {
        let Some(sid) = self.target_session() else {
            self.set_toast("no agent selected");
            return;
        };
        // `claude.setPermissionMode` isn't routed across federation.
        if self.is_remote_session(&sid) {
            self.set_toast("remote session — change its mode on its own machine");
            return;
        }
        let managed = self.provider_for(&sid) != "claude";
        let cycle: &[&str] = if managed {
            &["ask", "yolo"]
        } else {
            &["default", "acceptEdits", "plan"]
        };
        let cur = self
            .perm_modes
            .get(&sid)
            .map(String::as_str)
            .unwrap_or(cycle[0]);
        let idx = cycle.iter().position(|m| *m == cur).unwrap_or(0);
        let next = cycle[(idx + 1) % cycle.len()].to_string();
        let drv = self.driver();
        let tx = self.tx.clone();
        let sid2 = sid.clone();
        tokio::spawn(async move {
            match drv.set_permission_mode(&sid2, &next).await {
                Ok(mode) => {
                    let _ = tx.send(AppMsg::PermissionMode {
                        session_id: sid2,
                        mode,
                    });
                }
                Err(e) => {
                    let _ = tx.send(AppMsg::Toast(format!("mode: {e}")));
                }
            }
        });
    }
}
