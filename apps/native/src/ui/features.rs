//! Secondary views keep the conversation uncluttered and preserve its draft/scroll.
use super::*;
use serde_json::Value;
use wks_native::features::{AttachmentSource, Request, attention_transition};

pub(super) struct Extras {
    pub name: Entity<InputState>,
    pub return_launch: bool,
    pub approval_details: bool,
    pub selected_options: Vec<std::collections::BTreeSet<usize>>,
    pub notice: String,
    pub show_archived: bool,
    pub confirm_end: Option<String>,
    pub resume: Option<String>,
    pub local_paths: bool,
    pub keep_running: std::rc::Rc<std::cell::Cell<bool>>,
    pub upload_receipt: u64,
    pub attachments: HashMap<String, Vec<(String, String)>>,
    pub sent_drafts: HashMap<String, String>,
    pub answer_watches: Vec<gpui::Subscription>,
    pub sent_attachments: HashMap<String, Vec<(String, String)>>,
    pub history_page: usize,
    pub history_scroll: gpui::ScrollHandle,
    pub diff_scroll: gpui::ScrollHandle,
    pub question_signature: String,
    pub answers: Vec<Entity<InputState>>,
}
impl Extras {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        Self {
            name: cx.new(|cx| InputState::new(window, cx).placeholder("Session name")),
            return_launch: false,
            approval_details: false,
            selected_options: Vec::new(),
            notice: String::new(),
            show_archived: false,
            confirm_end: None,
            resume: None,
            local_paths: false,
            keep_running: Default::default(),
            upload_receipt: 0,
            attachments: HashMap::new(),
            sent_drafts: HashMap::new(),
            answer_watches: Vec::new(),
            sent_attachments: HashMap::new(),
            history_page: 0,
            history_scroll: gpui::ScrollHandle::new(),
            diff_scroll: gpui::ScrollHandle::new(),
            question_signature: String::new(),
            answers: Vec::new(),
        }
    }
}
fn questions(value: &Value) -> Vec<Value> {
    value
        .as_array()
        .or_else(|| value.get("questions").and_then(Value::as_array))
        .cloned()
        .unwrap_or_default()
}
impl Workspace {
    pub fn configure_local(
        &mut self,
        local_paths: bool,
        keep_running: std::rc::Rc<std::cell::Cell<bool>>,
    ) {
        self.extras.local_paths = local_paths;
        self.settings.keep_running = keep_running.get();
        self.extras.keep_running = keep_running;
    }
    pub(super) fn selected_session(&self) -> Option<&Session> {
        self.view
            .sessions
            .iter()
            .find(|s| Some(&s.id) == self.view.selected.as_ref())
    }
    pub(super) fn session_title(&self, session: &Session) -> String {
        self.settings
            .names
            .get(&self.project_scope)
            .and_then(|m| m.get(&session.id))
            .cloned()
            .unwrap_or_else(|| session.title().into())
    }
    pub(super) fn archived(&self, id: &str) -> bool {
        self.settings
            .archived
            .get(&self.project_scope)
            .is_some_and(|ids| ids.iter().any(|s| s == id))
    }
    pub(super) fn request(&mut self, request: Request, cx: &mut Context<Self>) {
        self.extras.notice.clear();
        self.command(Command::Request(request), cx);
    }
    pub(super) fn open_feature(
        &mut self,
        screen: Screen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.spawn_pending || self.view.creating {
            return;
        }
        if self.new_session && screen == Screen::Setup {
            self.extras.return_launch = true;
        }
        self.show_screen(screen, window, cx);
        self.extras.notice.clear();
        self.extras.confirm_end = None;
        match screen {
            Screen::Recent => self.request(Request::Recent, cx),
            Screen::Changes => {
                if let Some(s) = self.selected_session() {
                    self.request(Request::Changes { cwd: s.cwd.clone() }, cx);
                }
            }
            Screen::History => {
                if let Some(s) = self.selected_session().cloned() {
                    self.extras.history_page = 0;
                    self.request(
                        Request::History {
                            session: s.id.clone(),
                        },
                        cx,
                    );
                }
            }
            Screen::Session => {
                if let Some(s) = self.selected_session() {
                    let name = self.session_title(s);
                    self.extras.name.update(cx, |input, cx| {
                        input.set_value(name, window, cx);
                        input.focus(window, cx);
                    });
                }
            }
            Screen::Setup => self.request(
                Request::Setup {
                    provider: self.provider.into(),
                    check: false,
                },
                cx,
            ),
            Screen::Model => {
                if !self.supported_session() {
                    self.extras.notice =
                        "Model controls are available for Claude and Codex sessions.".into();
                    return;
                }
                if let Some(s) = self.selected_session().cloned() {
                    let provider = if s.provider == "codex" {
                        "codex"
                    } else {
                        "claude"
                    };
                    self.choose_provider(provider, window, cx);
                    self.project
                        .update(cx, |input, cx| input.set_value(s.cwd, window, cx));
                    self.model_choice = "__custom".into();
                    self.model_picker.update(cx, |picker, cx| {
                        picker.set_selected_value(&String::from("__custom"), window, cx)
                    });
                    self.model
                        .update(cx, |input, cx| input.set_value(s.model, window, cx));
                    self.context_window = s.context_window;
                    self.load_models(true, cx);
                }
            }
            _ => {}
        }
        cx.notify();
    }
    pub(super) fn back_from_feature(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.extras.return_launch {
            self.extras.return_launch = false;
            self.screen = Screen::Conversation;
            self.new_session = true;
            self.project.update(cx, |input, cx| input.focus(window, cx));
            self.load_models(false, cx);
            cx.notify();
        } else {
            self.show_screen(Screen::Conversation, window, cx);
        }
    }
    pub(super) fn supported_session(&self) -> bool {
        self.selected_session()
            .is_some_and(|s| matches!(s.provider.as_str(), "claude" | "codex" | ""))
    }
    pub(super) fn sync_features(
        &mut self,
        next: &View,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings.notifications
            && self.view.connected
            && next.connected
            && !window.is_window_active()
        {
            let mut alerts = Vec::new();
            for s in next.sessions.iter() {
                if let Some(old) = self.view.sessions.iter().find(|old| old.id == s.id)
                    && let Some(title) = attention_transition(old, s)
                {
                    alerts.push((title.to_owned(), self.session_title(s)));
                }
            }
            if !alerts.is_empty() {
                cx.background_executor()
                    .spawn(async move {
                        for (title, body) in alerts.into_iter().take(5) {
                            let mut notification = notify_rust::Notification::new();
                            notification
                                .summary(&title)
                                .body(&body)
                                .appname("Workspacer Native");
                            #[cfg(target_os = "windows")]
                            notification.app_id(if cfg!(feature = "rust-hub") {
                                "Workspacer.Native.RustPreview"
                            } else {
                                "Workspacer.Native"
                            });
                            if let Err(error) = notification.show() {
                                eprintln!("Native notification unavailable: {error}");
                            }
                        }
                    })
                    .detach();
            }
        }
        if let Some(state) = next.requests.get("upload")
            && !state.loading
            && state.number > self.extras.upload_receipt
        {
            self.extras.upload_receipt = state.number;
            if let Some(error) = &state.error {
                self.extras.notice = format!("Attachment failed: {error}");
            } else if let Request::Upload { session, .. } = &state.request
                && let (Some(name), Some(path)) =
                    (state.value["name"].as_str(), state.value["path"].as_str())
            {
                self.extras
                    .attachments
                    .entry(session.clone())
                    .or_default()
                    .push((name.into(), path.into()));
            }
        }
        if let Some(receipt) = &next.receipt
            && receipt.number > self.last_receipt
        {
            if receipt.error.is_none() && matches!(receipt.action, Action::Send(_)) {
                if let Some(sent) = self.extras.sent_drafts.remove(&receipt.session) {
                    if self.view.selected.as_ref() == Some(&receipt.session) {
                        if self.composer.read(cx).value().as_ref() == sent {
                            self.composer
                                .update(cx, |i, cx| i.set_value("", window, cx));
                        }
                    } else if self.drafts.get(&receipt.session) == Some(&sent) {
                        self.drafts.remove(&receipt.session);
                    }
                }
                if let Some(sent) = self.extras.sent_attachments.remove(&receipt.session)
                    && let Some(draft) = self.extras.attachments.get_mut(&receipt.session)
                {
                    draft.retain(|item| !sent.contains(item));
                }
            } else {
                self.extras.sent_attachments.remove(&receipt.session);
                self.extras.sent_drafts.remove(&receipt.session);
            }
        }
        let pending = next
            .sessions
            .iter()
            .find(|s| Some(&s.id) == next.selected.as_ref())
            .and_then(|s| s.questions.as_ref());
        let signature = format!(
            "{:?}:{}",
            next.selected,
            pending.map(Value::to_string).unwrap_or_default()
        );
        if signature != self.extras.question_signature {
            self.extras.question_signature = signature;
            self.extras.answer_watches.clear();
            self.extras.selected_options =
                vec![Default::default(); pending.map(questions).unwrap_or_default().len()];
            self.extras.answers = pending
                .map(questions)
                .unwrap_or_default()
                .iter()
                .map(|_| {
                    cx.new(|cx| InputState::new(window, cx).placeholder("Or type your answer"))
                })
                .collect();
        }
        self.extras.answer_watches = self
            .extras
            .answers
            .iter()
            .map(|input| cx.observe(input, |_, _, cx| cx.notify()))
            .collect();
        if self.view.selected != next.selected {
            self.extras.confirm_end = None;
            if matches!(
                self.screen,
                Screen::Changes | Screen::History | Screen::Session | Screen::Model
            ) {
                self.screen = Screen::Conversation;
            }
        }
    }
    pub(super) fn pick_folder(
        &mut self,
        bookmark: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.extras.local_paths {
            return;
        }
        let pick = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose project folder".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = pick.await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.first() {
                            let input = if bookmark {
                                &this.project_path
                            } else {
                                &this.project
                            };
                            input.update(cx, |input, cx| {
                                input.set_value(path.to_string_lossy().into_owned(), window, cx)
                            });
                        }
                    }
                    Ok(Ok(None)) => {}
                    _ => {
                        this.extras.notice =
                            "Could not open the folder picker. You can enter the path instead."
                                .into()
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn pick_attachment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.view.selected.clone() else {
            return;
        };
        if self.uploading() {
            return;
        }
        let pick = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Attach image or PDF (up to 8 MiB)".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = pick.await;
            let _ = this.update_in(cx, |this, _, cx| match result {
                Ok(Ok(Some(paths))) => {
                    if let Some(path) = paths.first() {
                        this.request(
                            Request::Upload {
                                session,
                                source: AttachmentSource::File(path.clone()),
                            },
                            cx,
                        );
                    }
                }
                Ok(Ok(None)) => {}
                _ => {
                    this.extras.notice = "Could not open the attachment picker.".into();
                    cx.notify();
                }
            });
        })
        .detach();
    }
    pub(super) fn paste_image(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(session) = self.view.selected.clone() else {
            return false;
        };
        if self.uploading() {
            return false;
        }
        if let Some(clipboard) = cx.read_from_clipboard() {
            if clipboard.text().is_some() {
                return false;
            }
            for entry in clipboard.entries() {
                if let gpui::ClipboardEntry::Image(image) = entry {
                    let name = format!(
                        "Screenshot.{}",
                        match image.format() {
                            gpui::ImageFormat::Png => "png",
                            gpui::ImageFormat::Jpeg => "jpg",
                            gpui::ImageFormat::Webp => "webp",
                            gpui::ImageFormat::Gif => "gif",
                            gpui::ImageFormat::Tiff => "tiff",
                            gpui::ImageFormat::Bmp => "bmp",
                            _ => {
                                self.extras.notice =
                                    "Paste a PNG, JPEG, GIF, or WebP screenshot.".into();
                                cx.notify();
                                return true;
                            }
                        }
                    );
                    if image.bytes().len() > wks_native::features::MAX_ATTACHMENT_BYTES {
                        self.extras.notice =
                            "Screenshot exceeds 8 MiB. Save a smaller image and attach it.".into();
                        cx.notify();
                        return true;
                    }
                    self.request(
                        Request::Upload {
                            session,
                            source: AttachmentSource::Image {
                                name,
                                bytes: Arc::new(image.bytes().to_vec()),
                            },
                        },
                        cx,
                    );
                    return true;
                }
            }
        }
        #[cfg(target_os = "windows")]
        {
            self.request(
                Request::Upload {
                    session,
                    source: AttachmentSource::WindowsClipboard,
                },
                cx,
            );
            true
        }
        #[cfg(not(target_os = "windows"))]
        false
    }
    pub(super) fn uploading(&self) -> bool {
        self.view.requests.get("upload").is_some_and(|s| s.loading)
    }
    pub(super) fn attachment_text(&self, id: &str, text: &str) -> String {
        let mut result = String::new();
        for (name, path) in self.extras.attachments.get(id).into_iter().flatten() {
            let kind = if name.to_lowercase().ends_with(".pdf") {
                "PDF"
            } else {
                "Image"
            };
            result.push_str(&format!("[{kind}: {path}]\n"));
        }
        result.push_str(text);
        result
    }
    pub(super) fn feature_message(&self, key: &str) -> Div {
        let p = self.appearance.palette();
        let message = match self.view.requests.get(key) {
            Some(s) if s.loading => "Loading…".into(),
            Some(s) => s.error.clone().unwrap_or_default(),
            None => "".into(),
        };
        div()
            .text_color(rgb(p.warning))
            .text_size(px(12.))
            .child(message)
    }
    pub(super) fn render_feature(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        let title = match self.screen {
            Screen::Recent => "Session history",
            Screen::Changes => "Changes",
            Screen::History => "Conversation history",
            Screen::Session => "Session",
            Screen::Setup => "Agent setup",
            Screen::Model => "Change model",
            _ => "",
        };
        let body = match self.screen {
            Screen::Recent => self.render_recent(cx),
            Screen::Changes => self.render_changes(cx),
            Screen::History => self.render_history(window, cx),
            Screen::Session => self.render_session(cx),
            Screen::Setup => self.render_setup(cx),
            Screen::Model => self.render_model(cx),
            _ => div(),
        };
        div()
            .id("feature-view")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .bg(rgb(p.chat))
            .p_5()
            .child(
                div()
                    .max_w(px(CHAT_WIDTH))
                    .mx_auto()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(24.))
                                    .font_weight(FontWeight::BOLD)
                                    .child(title),
                            )
                            .child(
                                self.button(
                                    "feature-back",
                                    if self.extras.return_launch {
                                        "Back to session setup"
                                    } else {
                                        "Back to chat"
                                    },
                                    true,
                                )
                                .on_click(cx.listener(
                                    |this, _, window, cx| this.back_from_feature(window, cx),
                                )),
                            ),
                    )
                    .when(!self.extras.notice.is_empty(), |d| {
                        d.child(
                            div()
                                .text_color(rgb(p.warning))
                                .child(self.extras.notice.clone()),
                        )
                    })
                    .child(body),
            )
    }
    fn render_recent(&self, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let rows = self
            .view
            .requests
            .get("recent")
            .and_then(|s| s.value.as_array())
            .cloned()
            .unwrap_or_default();
        let query = self.search.read(cx).value().to_lowercase();
        let sessions: Vec<_> = rows
            .iter()
            .map(|row| {
                let mut s = Session::default();
                s.merge(row);
                s
            })
            .filter(|s| {
                !s.id.is_empty()
                    && self.archived(&s.id) == self.extras.show_archived
                    && (self.session_title(s).to_lowercase().contains(&query)
                        || s.cwd.to_lowercase().contains(&query))
            })
            .collect();
        div().flex().flex_col().gap_3()
            .child(div().flex().gap_2().child(self.button("history-active", "All sessions", true).when(!self.extras.show_archived, |d| d.bg(rgb(p.selected))).on_click(cx.listener(|this, _, _, cx| { this.extras.show_archived = false; cx.notify(); })))
                .child(self.button("history-archived", "Archived", true).when(self.extras.show_archived, |d| d.bg(rgb(p.selected))).on_click(cx.listener(|this, _, _, cx| { this.extras.show_archived = true; cx.notify(); })))
                .child(self.button("history-refresh", "Refresh", self.view.connected).on_click(cx.listener(|this, _, _, cx| this.request(Request::Recent, cx)))))
            .child(self.feature_message("recent"))
            .when(sessions.is_empty(), |d| d.child(div().text_color(rgb(p.muted)).child("No matching sessions. Try clearing the filter or refreshing.")))
            .children(sessions.into_iter().take(500).enumerate().map(|(ix, s)| {
                let open = s.clone(); let resume = s.clone(); let id = s.id.clone();
                div().id(("recent-row", ix)).p_3().rounded_md().bg(rgb(p.surface)).flex().flex_col().gap_2()
                    .child(div().flex().justify_between().child(self.session_title(&s)).child(session_badge(&s, p, self.view.connected)))
                    .child(div().text_size(px(12.)).text_color(rgb(p.muted)).child(s.cwd.clone()))
                    .child(div().flex().gap_2()
                        .child(self.button("open-recent", "Open", self.view.connected).on_click(cx.listener(move |this, _, window, cx| {
                            this.show_screen(Screen::Conversation, window, cx); this.command(Command::OpenRecent(Box::new(open.clone())), cx);
                        })))
                        .when(s.stopped() && matches!(s.provider.as_str(), "claude" | "codex"), |d| d.child(self.button("resume-recent", "Resume…", self.view.connected).on_click(cx.listener(move |this, _, window, cx| this.resume_session(&resume, window, cx)))))
                        .child(self.button("archive-recent", if self.archived(&s.id) { "Restore" } else { "Archive" }, true).on_click(cx.listener(move |this, _, _, cx| this.toggle_archive(&id, cx)))))
            }))
            .child(div().text_size(px(12.)).text_color(rgb(p.muted)).child("Names and archives are saved on this device for this connection. Archiving keeps the conversation and does not stop an agent."))
    }
    fn toggle_archive(&mut self, id: &str, cx: &mut Context<Self>) {
        let ids = self
            .settings
            .archived
            .entry(self.project_scope.clone())
            .or_default();
        if ids.iter().any(|s| s == id) {
            ids.retain(|s| s != id);
        } else {
            ids.push(id.into());
        }
        self.save_settings(cx);
    }
    pub(super) fn resume_session(
        &mut self,
        session: &Session,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.requested_session.is_some() {
            return;
        }
        if !matches!(session.provider.as_str(), "claude" | "codex" | "") {
            self.extras.notice = "This provider cannot be resumed by the native client yet.".into();
            cx.notify();
            return;
        }
        self.show_new_session(window, cx);
        self.extras.resume = Some(session.id.clone());
        self.choose_provider(
            if session.provider == "codex" {
                "codex"
            } else {
                "claude"
            },
            window,
            cx,
        );
        self.project.update(cx, |input, cx| {
            input.set_value(session.cwd.clone(), window, cx)
        });
        self.label.update(cx, |input, cx| {
            input.set_value(session.title().to_owned(), window, cx)
        });
        self.model_choice = if session.model.is_empty() {
            String::new()
        } else {
            "__custom".into()
        };
        self.model.update(cx, |input, cx| {
            input.set_value(session.model.clone(), window, cx)
        });
        self.model_picker.update(cx, |picker, cx| {
            picker.set_selected_value(&self.model_choice, window, cx)
        });
        self.context_window = session.context_window;
        self.permission = Permission::Ask;
        self.load_models(true, cx);
    }
    fn render_session(&self, cx: &mut Context<Self>) -> Div {
        let Some(s) = self.selected_session() else {
            return div().child("Select a session first.");
        };
        let id = s.id.clone();
        let archive = id.clone();
        let end = id.clone();
        let resume = s.clone();
        let busy = self.view.busy || !self.view.connected;
        div().flex().flex_col().gap_3().child("Name on this device").child(Input::new(&self.extras.name))
            .child(self.button("save-session-name", "Save name", true).on_click(cx.listener(move |this, _, _, cx| {
                let name = this.extras.name.read(cx).value().trim().to_owned();
                if name.len() > 200 { this.extras.notice = "Use a name of at most 200 characters.".into(); }
                else { let names = this.settings.names.entry(this.project_scope.clone()).or_default();
                    if name.is_empty() { names.remove(&id); } else { names.insert(id.clone(), name); }
                    this.save_settings(cx); this.extras.notice = "Name saved".into(); }
                cx.notify();
            })))
            .child(self.button("archive-session", if self.archived(&s.id) { "Restore from archive" } else { "Archive on this device" }, true).on_click(cx.listener(move |this, _, _, cx| this.toggle_archive(&archive, cx))))
            .when(s.stopped() && self.supported_session(), |d| d.child(self.button("resume-session", "Resume session…", !busy).on_click(cx.listener(move |this, _, window, cx| this.resume_session(&resume, window, cx)))))
            .when(!s.stopped(), |d| d.child(self.button("end-session", "End session…", !busy).on_click(cx.listener(move |this, _, _, cx| { this.extras.confirm_end = Some(end.clone()); cx.notify(); }))))
            .when(self.extras.confirm_end.as_ref() == Some(&s.id), |d| d.child(div().p_3().rounded_md().bg(rgb(self.appearance.palette().surface)).flex().flex_col().gap_2()
                .child("End this agent? Its current work will stop. You can resume its conversation later.")
                .child(div().flex().gap_2().child(self.button("confirm-end", "End session", !busy).on_click(cx.listener(|this, _, _, cx| { this.act(Action::Terminate, cx); this.extras.confirm_end = None; })))
                    .child(self.button("cancel-end", "Keep running", true).on_click(cx.listener(|this, _, _, cx| { this.extras.confirm_end = None; cx.notify(); }))))))
            .child(div().text_size(px(12.)).text_color(rgb(self.appearance.palette().muted)).child(s.cwd.clone()))
            .child(self.view.notice.clone())
    }
    fn render_changes(&self, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let state = self.view.requests.get("changes");
        let cwd = state
            .and_then(|state| match &state.request {
                Request::Changes { cwd } => Some(cwd.clone()),
                _ => None,
            })
            .or_else(|| self.selected_session().map(|s| s.cwd.clone()));
        let Some(cwd) = cwd else {
            return div().child("Select a session or request a project review first.");
        };
        let refresh_cwd = cwd.clone();
        let value = state.map(|s| s.value.as_ref()).unwrap_or(&Value::Null);
        let files = value["files"].as_array().cloned().unwrap_or_default();
        let diff = self
            .view
            .requests
            .get("diff")
            .filter(|s| matches!(&s.request, Request::Diff { cwd: c, .. } if c == &cwd));
        div().flex().flex_col().gap_3()
            .child(div().text_size(px(12.)).text_color(rgb(p.muted)).child(format!("{} · {}", cwd, value["branch"].as_str().unwrap_or("Repository"))))
            .child("Working tree changes, including edits made outside this session.")
            .child(self.button("changes-refresh", "Refresh changes", self.view.connected).on_click(cx.listener(move |this, _, _, cx| this.request(Request::Changes { cwd: refresh_cwd.clone() }, cx))))
            .child(self.feature_message("changes"))
            .when(files.is_empty() && state.is_some_and(|s| !s.loading && s.error.is_none()), |d| d.child("No uncommitted changes."))
            .children(files.into_iter().take(1000).enumerate().map(|(ix, file)| {
                let path = file["path"].as_str().unwrap_or("").to_owned();
                let staged = file["staged"].as_str().unwrap_or(" ");
                let unstaged = file["unstaged"].as_str().unwrap_or(" ");
                let untracked = staged == "?" || unstaged == "?";
                let c = cwd.clone(); let file_path = path.clone(); let c2 = c.clone(); let path2 = path.clone();
                div().id(("changed-file", ix)).p_2().rounded_md().bg(rgb(p.surface)).flex().flex_wrap().items_center().gap_2().child(div().flex_1().min_w_0().child(path))
                    .when(untracked || !unstaged.trim().is_empty(), |d| d.child(self.button("diff-working", if untracked { "New file" } else { "Unstaged" }, true).on_click(cx.listener(move |this, _, _, cx| {
                        this.extras.diff_scroll.set_offset(gpui::point(px(0.), px(0.)));
                        this.request(Request::Diff { cwd: c.clone(), path: file_path.clone(), staged: false, untracked }, cx);
                    }))))
                    .when(!untracked && !staged.trim().is_empty(), |d| d.child(self.button("diff-staged", "Staged", true).on_click(cx.listener(move |this, _, _, cx| this.request(Request::Diff { cwd: c2.clone(), path: path2.clone(), staged: true, untracked: false }, cx)))))
            }))
            .when_some(diff, |d, state| {
                let text = state.value["diff"].as_str().unwrap_or("");
                let path = if let Request::Diff { path, staged, .. } = &state.request { format!("{} · {}", path, if *staged { "Staged" } else { "Working tree" }) } else { String::new() };
                d.child(div().text_size(px(16.)).child(path)).child(self.feature_message("diff"))
                    .when(text.is_empty() && !state.loading && state.error.is_none(), |d| d.child("No text diff available. The file may be binary or have changed since refresh."))
                    .child(div().id("diff-content").max_h(px(500.)).overflow_y_scroll().track_scroll(&self.extras.diff_scroll).font_family(gpui_component::Theme::global(cx).mono_font_family.clone()).text_size(px(12.)).bg(rgb(p.surface)).p_3()
                        .children(text.lines().take(3000).map(|line| div().text_color(rgb(if line.starts_with('+') { p.success } else if line.starts_with('-') { p.warning } else if line.starts_with("@@") { p.accent } else { p.text })).child(line.to_owned()))))
                    .when(text.lines().count() > 3000, |d| d.child("Showing the first 3,000 diff lines. Review the full file in your editor."))
            })
    }
    fn render_history(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let history = self.view.requests.get("history").cloned();
        let state = history.as_ref();
        let items = state.and_then(|s| s.value["rows"].as_array());
        let count = items.map(Vec::len).unwrap_or(0);
        let pages = count.div_ceil(50).max(1);
        let page = self.extras.history_page.min(pages - 1);
        let start = page * 50;
        div().flex().flex_col().gap_3().child(self.feature_message("history"))
            .child(div().text_size(px(12.)).text_color(rgb(p.muted)).child("A snapshot of retained conversation history. Live messages continue in chat."))
            .child(div().flex().items_center().gap_2()
                .child(self.button("older-history", "Previous", page > 0).when(page > 0, |d| d.on_click(cx.listener(|this, _, _, cx| { this.extras.history_page = this.extras.history_page.saturating_sub(1); this.extras.history_scroll.set_offset(gpui::point(px(0.), px(0.))); cx.notify(); }))))
                .child(format!("Page {} of {}", page + 1, pages))
                .child(self.button("newer-history", "Next", page + 1 < pages).when(page + 1 < pages, |d| d.on_click(cx.listener(|this, _, _, cx| { this.extras.history_page += 1; this.extras.history_scroll.set_offset(gpui::point(px(0.), px(0.))); cx.notify(); })))))
            .when(state.is_some_and(|s| s.value["first_seq"].as_u64().unwrap_or(0) > 1), |d| d.child("The server has trimmed earlier events; this starts at its oldest retained event."))
            .when(count == 0 && state.is_some_and(|s| !s.loading && s.error.is_none()), |d| d.child("No retained messages are available for this session."))
            .child(div().id("history-content").max_h(px(600.)).overflow_y_scroll().track_scroll(&self.extras.history_scroll)
                .children(items.into_iter().flatten().skip(start).take(50).filter_map(|value| {
                    let row = serde_json::from_value::<wks_native::model::Row>(value.clone()).ok()?;
                    Some(div().when(value["continued"] == true, |d| d.child(overline(format!("Long message · part {} · literal text", value["part"]), p)))
                        .child(self.render_message(&row, "history", value["continued"] == true, window, cx)))
                })))
    }
    fn render_setup(&self, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let state = self.view.requests.get("setup");
        let busy = state.is_some_and(|s| s.loading);
        let value = state.map(|s| s.value.as_ref()).unwrap_or(&Value::Null);
        let detected = value["installed"].as_array();
        div().flex().flex_col().gap_4()
            .child(div().text_size(px(12.)).text_color(rgb(p.muted)).child("Connect your agents on the machine running this workspace."))
            .children(["claude", "codex"].into_iter().map(|provider| {
                let found = detected.and_then(|rows| rows.iter().find(|r| r["provider"] == provider)).and_then(|r| r["found"].as_bool());
                let label = if provider == "claude" { "Claude" } else { "Codex" };
                let current = state.is_some_and(|s| matches!(&s.request, Request::Setup { provider: checked, .. } if checked == provider));
                let status = if current && !busy { value["readiness"]["state"].as_str().unwrap_or("unchecked") } else { "unchecked" };
                let color = match status {
                    "responding" => p.success,
                    "unchecked" | "unsupported" => p.muted,
                    _ => p.warning,
                };
                let description = if busy && current { "Checking this agent…" } else { match status {
                    "responding" => "Ready · the agent responded",
                    "unauthenticated" => "Sign-in required",
                    "limited" => "Account limit reached",
                    "timeout" => "The connection check timed out",
                    "network-error" => "Network unavailable",
                    "unchecked" => "Connection not verified",
                    "unsupported" => "Connection check unavailable on this host",
                    _ => "Connection check failed",
                }};
                div().py_5().border_b_1().border_color(rgb(p.border)).flex().flex_col().gap_4()
                    .child(div().flex().items_center().gap_3()
                        .child(div().size(px(40.)).rounded(px(12.)).bg(rgb(p.selected)).flex().items_center().justify_center()
                            .child(Icon::new(IconName::Bot).size(px(20.)).text_color(rgb(p.accent))))
                        .child(div().flex_1().min_w_0().flex().flex_col().gap_1()
                            .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(label))
                            .child(div().text_size(px(11.)).text_color(rgb(p.muted)).child(if provider == "claude" { "Claude Code" } else { "Codex CLI" })))
                        .child(div().flex().items_center().gap_2().text_size(px(11.)).text_color(rgb(if found == Some(true) { p.success } else { p.muted }))
                            .child(status_dot(if found == Some(true) { p.success } else { p.muted }))
                            .child(match found { Some(true) => "Installed", Some(false) => "Not found", None => "Not checked" })))
                    .child(div().text_size(px(12.)).text_color(rgb(p.muted)).child(if provider == "claude" { "Install Claude Code, then run claude in a terminal to sign in." } else { "Install Codex CLI, then run codex login in a terminal to sign in." }))
                    .child(div().flex().flex_wrap().items_center().justify_between().gap_3()
                        .child(div().flex().items_center().gap_2().text_size(px(12.)).text_color(rgb(color))
                            .child(if busy && current { brand_spinner(12., p, SharedString::from(format!("setup-{provider}-activity"))) } else { status_dot(color) })
                            .child(description))
                        .child(self.button(if provider == "claude" { "setup-claude" } else { "setup-codex" }, if busy && current { "Checking…" } else { "Check connection" }, !busy && self.view.connected)
                            .when(!busy && self.view.connected, |d| d.on_click(cx.listener(move |this, _, _, cx| this.request(Request::Setup { provider: provider.into(), check: true }, cx))))))
                    .when(current && !busy, |d| d
                        .when_some(state.and_then(|s| s.error.as_ref()), |d, error| d.child(div().text_size(px(12.)).text_color(rgb(p.warning)).child(error.clone())))
                        .when_some(value["readinessError"].as_str(), |d, error| d.child(div().text_size(px(12.)).text_color(rgb(p.warning)).child(error.to_owned()))))
            }))
            .child(div().flex().items_start().gap_2().text_size(px(12.)).text_color(rgb(p.muted))
                .child(Icon::new(IconName::Info).size(px(14.)).flex_shrink_0())
                .child(div().flex_1().min_w_0().whitespace_normal().child("Checking a connection sends a small test request and may use your provider allowance. Git is required for reviewing changes.")))
            .child(div().flex().child(self.quiet_button("setup-refresh", "Recheck installed agents", IconName::Redo, !busy && self.view.connected)
                .when(!busy && self.view.connected, |d| d.on_click(cx.listener(|this, _, _, cx| this.request(Request::Setup { provider: this.provider.into(), check: false }, cx))))))
    }
    fn render_model(&self, cx: &mut Context<Self>) -> Div {
        if !self.supported_session() {
            return div().child("Model switching is available for Claude and Codex sessions.");
        }
        let busy = self.view.busy || !self.view.connected;
        div().flex().flex_col().gap_3().child("Choose the model for this session's next work. A busy provider may queue the change.")
            .child(self.render_launch_options(busy, cx))
            .child(self.button("apply-model", "Apply model", !busy).when(!busy, |d| d.on_click(cx.listener(|this, _, _, cx| {
                let model = if this.model_choice == "__custom" { this.model.read(cx).value().trim().to_owned() } else { this.model_choice.clone() };
                if model.is_empty() { this.extras.notice = "Choose a model or enter its exact ID.".into(); cx.notify(); return; }
                this.act(Action::SetModel { model, context_window: this.context_window }, cx);
            }))))
            .child(self.view.notice.clone())
    }
    pub(super) fn question_answers(&self, cx: &App) -> Vec<String> {
        let qs = self
            .selected_session()
            .and_then(|s| s.questions.as_ref())
            .map(questions)
            .unwrap_or_default();
        self.extras
            .answers
            .iter()
            .enumerate()
            .map(|(ix, input)| {
                let custom = input.read(cx).value().to_string();
                if !custom.trim().is_empty() {
                    return custom;
                }
                self.extras
                    .selected_options
                    .get(ix)
                    .into_iter()
                    .flatten()
                    .filter_map(|option_ix| {
                        let option = qs.get(ix)?.get("options")?.get(*option_ix)?;
                        option["label"]
                            .as_str()
                            .or(option.as_str())
                            .map(str::to_owned)
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .collect()
    }
    pub(super) fn render_questions(
        &self,
        session: &Session,
        enabled: bool,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        let list = session
            .questions
            .as_ref()
            .map(questions)
            .unwrap_or_default();
        let ready = !self.extras.answers.is_empty()
            && self
                .question_answers(cx)
                .iter()
                .all(|s| !s.trim().is_empty());
        div()
            .id("question-choices")
            .max_h(px(if compact { 120. } else { 240. }))
            .overflow_y_scroll()
            .px_5()
            .py_2()
            .flex()
            .flex_col()
            .gap_2()
            .children(list.iter().enumerate().map(|(ix, q)| {
                let multiple = q["multiSelect"].as_bool().unwrap_or(false);
                let options = q["options"].as_array().cloned().unwrap_or_default();
                div()
                    .id(("question", ix))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div().text_color(rgb(p.warning)).child(
                            q["question"]
                                .as_str()
                                .or(q["header"].as_str())
                                .unwrap_or("Your answer is needed")
                                .to_owned(),
                        ),
                    )
                    .child(div().flex().flex_wrap().gap_2().children(
                        options.iter().enumerate().map(|(option_ix, option)| {
                            let label = option["label"]
                                .as_str()
                                .or(option.as_str())
                                .unwrap_or("")
                                .to_owned();
                            let description =
                                option["description"].as_str().unwrap_or("").to_owned();
                            let chosen = self
                                .extras
                                .selected_options
                                .get(ix)
                                .is_some_and(|selected| selected.contains(&option_ix))
                                && self
                                    .extras
                                    .answers
                                    .get(ix)
                                    .is_some_and(|input| input.read(cx).value().is_empty());
                            div()
                                .id(("question-option", option_ix))
                                .px_3()
                                .py_2()
                                .rounded_md()
                                .bg(rgb(if chosen { p.selected } else { p.surface }))
                                .cursor_pointer()
                                .child(label.clone())
                                .when(!description.is_empty(), |d| {
                                    d.child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(rgb(p.muted))
                                            .child(description),
                                    )
                                })
                                .when(enabled, |d| {
                                    d.on_click(cx.listener(move |this, _, window, cx| {
                                        if let Some(input) = this.extras.answers.get(ix) {
                                            if let Some(selected) =
                                                this.extras.selected_options.get_mut(ix)
                                            {
                                                if multiple && selected.contains(&option_ix) {
                                                    selected.remove(&option_ix);
                                                } else {
                                                    if !multiple {
                                                        selected.clear();
                                                    }
                                                    selected.insert(option_ix);
                                                }
                                            }
                                            input.update(cx, |input, cx| {
                                                input.set_value("", window, cx)
                                            });
                                            cx.notify();
                                        }
                                    }))
                                })
                        }),
                    ))
                    .when_some(self.extras.answers.get(ix), |d, input| {
                        d.child(Input::new(input).disabled(!enabled))
                    })
            }))
            .child(
                self.button("submit-answers", "Send answers", enabled && ready)
                    .when(enabled && ready, |d| {
                        d.on_click(cx.listener(|this, _, _, cx| {
                            let answers = this.question_answers(cx);
                            this.act(Action::Answers(answers), cx);
                        }))
                    }),
            )
            .when(list.is_empty(), |d| {
                d.child("Use the composer to answer this question.").child(
                    self.button("answer-text", "Answer with composer", enabled)
                        .when(enabled, |d| {
                            d.on_click(cx.listener(|this, _, _, cx| {
                                this.act(
                                    Action::Answer(this.composer.read(cx).value().to_string()),
                                    cx,
                                )
                            }))
                        }),
                )
            })
    }
}
