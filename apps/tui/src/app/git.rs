//! Git: the review pane's status/diff loading, staging, and commit flow.

use super::*;

impl App {
    /// Fetch the branch + changed-file count for an agent's cwd (the inspector
    /// strip). Cheap; called when opening an agent.
    pub(in crate::app) fn load_git_summary(&self, cwd: String) {
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
    pub(in crate::app) fn apply_git_status(
        &mut self,
        cwd: String,
        branch: Option<String>,
        files: Vec<FileStatus>,
    ) {
        let Some(r) = self.review.as_mut() else {
            return;
        };
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
    pub(in crate::app) fn apply_git_diff(
        &mut self,
        cwd: String,
        path: String,
        staged: bool,
        diff: String,
    ) {
        let Some(r) = self.review.as_mut() else {
            return;
        };
        if r.cwd != cwd || r.staged_view != staged {
            return;
        }
        if r.selected_file().map(|f| f.path.as_str()) != Some(path.as_str()) {
            return;
        }
        r.diff = diff;
        r.diff_scroll = 0;
    }

    pub(in crate::app) fn open_review(&mut self) {
        let Some(cwd) = self
            .target_agent()
            .and_then(|a| a.cwd.clone())
            .filter(|c| !c.is_empty())
        else {
            self.set_toast("no working directory for this agent");
            return;
        };
        self.review = Some(ReviewState::new(cwd.clone()));
        // The review is a docked pane, not a takeover: the conversation it is a
        // review *of* stays on screen beside it.
        let sid = self.target_session().unwrap_or_default();
        self.side = Some(SidePane {
            kind: SideKind::Review,
            session_id: sid,
            focused: true,
            scroll: 0,
        });
        self.load_git_status(cwd);
    }

    pub(in crate::app) fn close_review(&mut self) {
        self.review = None;
        if self
            .side
            .as_ref()
            .is_some_and(|p| p.kind == SideKind::Review)
        {
            self.side = None;
        }
    }

    /// Re-pull status for the open review pane (after a stage/commit/etc.).
    pub(in crate::app) fn review_reload(&self) {
        if let Some(r) = &self.review {
            self.load_git_status(r.cwd.clone());
        }
    }

    pub(in crate::app) fn load_git_status(&self, cwd: String) {
        let cm = self.claudemon.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move { fetch_git_status(&cm, &tx, cwd).await });
    }

    /// Load the selected file's diff for the current staged/unstaged view.
    pub(in crate::app) fn review_load_diff(&self) {
        let Some(r) = &self.review else { return };
        let Some(file) = r.selected_file() else {
            return;
        };
        let cm = self.claudemon.clone();
        let tx = self.tx.clone();
        let cwd = r.cwd.clone();
        let path = file.path.clone();
        let staged = r.staged_view;
        // Untracked files have no index/HEAD baseline — render them all-added.
        let untracked = !staged && file.is_untracked();
        tokio::spawn(async move { fetch_git_diff(&cm, &tx, cwd, path, staged, untracked).await });
    }

    /// Move the file selection by `delta` and load the newly-selected diff.
    pub(in crate::app) fn review_select(&mut self, delta: i32) {
        let Some(r) = self.review.as_mut() else {
            return;
        };
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

    pub(in crate::app) fn review_scroll(&mut self, delta: i32) {
        if let Some(r) = self.review.as_mut() {
            r.diff_scroll = if delta >= 0 {
                r.diff_scroll.saturating_add(delta as u16)
            } else {
                r.diff_scroll.saturating_sub((-delta) as u16)
            };
        }
    }

    pub(in crate::app) fn review_toggle_staged(&mut self) {
        if let Some(r) = self.review.as_mut() {
            r.staged_view = !r.staged_view;
            r.diff = String::new();
            r.diff_scroll = 0;
        }
        self.review_load_diff();
    }

    pub(in crate::app) fn review_stage(&mut self) {
        let Some(r) = &self.review else { return };
        let Some(file) = r.selected_file() else {
            return;
        };
        let (cwd, path) = (r.cwd.clone(), file.path.clone());
        let cm = self.claudemon.clone();
        self.git_dispatch(
            "Staged",
            async move { cm.git_stage(&cwd, Some(&path)).await },
        );
    }

    pub(in crate::app) fn review_unstage(&mut self) {
        let Some(r) = &self.review else { return };
        let Some(file) = r.selected_file() else {
            return;
        };
        let (cwd, path) = (r.cwd.clone(), file.path.clone());
        let cm = self.claudemon.clone();
        self.git_dispatch("Unstaged", async move {
            cm.git_unstage(&cwd, Some(&path)).await
        });
    }

    pub(in crate::app) fn review_stage_all(&mut self) {
        let Some(r) = &self.review else { return };
        let cwd = r.cwd.clone();
        let cm = self.claudemon.clone();
        self.git_dispatch("Staged all", async move { cm.git_stage(&cwd, None).await });
    }

    pub(in crate::app) fn review_push(&mut self) {
        let Some(r) = &self.review else { return };
        let cwd = r.cwd.clone();
        let cm = self.claudemon.clone();
        self.set_toast("Pushing…");
        self.git_dispatch("Pushed", async move { cm.git_push(&cwd).await });
    }

    pub(in crate::app) fn review_submit_commit(&mut self) {
        let Some(r) = self.review.as_mut() else {
            return;
        };
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
    pub(in crate::app) fn git_dispatch<F>(&self, ok_msg: &str, fut: F)
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
}
