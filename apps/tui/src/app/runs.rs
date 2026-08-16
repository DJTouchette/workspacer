//! The runs overlay and the docked side pane — the two things that take
//! over the content column.

use super::*;

impl App {
    /// Open the review pane over the targeted agent's work tree. No-op (with a
    /// toast) when the agent has no cwd.
    /// Open the runs overlay on the agent in focus, and kick off the first read.
    pub(in crate::app) fn open_runs(&mut self) {
        let Some(sid) = self.target_session() else {
            self.set_toast("no agent selected");
            return;
        };
        // Run artifacts are files beside the transcript on the agent's OWN
        // machine; a remote session's are unreadable from here.
        if self.is_remote_session(&sid) {
            self.set_toast("remote session — runs live on its own machine");
            return;
        }
        self.runs_open = Some(sid);
        self.refresh_runs();
    }

    pub(in crate::app) fn close_runs(&mut self) {
        self.runs_open = None;
    }

    /// Re-read the open session's artifacts. Called on the refresh tick, so the
    /// overlay tracks a live workflow without the renderer ever touching disk.
    pub fn refresh_runs(&self) {
        let Some(sid) = self.runs_open.clone() else {
            return;
        };
        // A session with no transcript yet (cold start, or a provider that
        // doesn't write one) simply has no artifacts to read.
        let Some(path) = self
            .all_agents
            .iter()
            .find(|a| a.session_id == sid)
            .and_then(|a| a.transcript_path.clone())
        else {
            return;
        };
        let tx = self.tx.clone();
        // Blocking file IO off the UI thread: the run dir of a big workflow holds
        // dozens of transcripts, and a frame must never wait on a stat.
        tokio::task::spawn_blocking(move || {
            let runs = crate::runs::read(&path);
            let _ = tx.send(AppMsg::Runs {
                session_id: sid,
                runs: Box::new(runs),
            });
        });
    }

    /// The runs for the open overlay, with each plain subagent's state resolved
    /// against the conversation.
    ///
    /// A subagent's meta file records the `tool_use` id of the `Task` call that
    /// spawned it, and that call is finished exactly when its result has landed —
    /// which the fold already knows. Resolving it here rather than storing a
    /// status keeps one source of truth: the transcript.
    pub fn open_runs_view(&self) -> Option<(&str, crate::runs::SessionRuns)> {
        let sid = self.runs_open.as_deref()?;
        let mut runs = self.runs.get(sid)?.clone();
        let fold = self.folds.get(sid);
        for sub in &mut runs.subagents {
            sub.state = match (&sub.tool_use_id, fold) {
                (Some(id), Some(f)) if f.tool_settled(id) => crate::runs::RunState::Done,
                // No id, or a fold we haven't loaded: don't claim it finished.
                _ => crate::runs::RunState::Running,
            };
        }
        runs.subagents.sort_by_key(|a| a.state);
        Some((sid, runs))
    }

    /// Dock (or un-dock) a side pane for the agent in focus. Opening one takes
    /// focus, so its keys work immediately — the same feel the modal had, without
    /// hiding the conversation behind it.
    pub(in crate::app) fn toggle_side(&mut self, kind: SideKind) {
        if self.side.as_ref().is_some_and(|p| p.kind == kind) {
            self.side = None;
            if kind == SideKind::Review {
                self.review = None;
            }
            return;
        }
        let Some(sid) = self.target_session() else {
            self.set_toast("no agent selected");
            return;
        };
        self.side = Some(SidePane {
            kind,
            session_id: sid,
            focused: true,
            scroll: 0,
        });
    }

    /// Move focus between the agent tiles and the docked pane.
    pub(in crate::app) fn focus_side(&mut self, to_side: bool) -> bool {
        match self.side.as_mut() {
            Some(pane) if pane.focused != to_side => {
                pane.focused = to_side;
                true
            }
            _ => false,
        }
    }

    /// True when keys belong to the docked pane rather than the chat.
    pub fn side_focused(&self) -> bool {
        self.side.as_ref().is_some_and(|p| p.focused)
    }

    /// The docked pane, when it is showing `kind`.
    pub fn side_of(&self, kind: SideKind) -> Option<&SidePane> {
        self.side.as_ref().filter(|p| p.kind == kind)
    }
}
