//! Secondary views keep the conversation uncluttered and preserve its draft/scroll.
use super::*;
use gpui::AnyElement;
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
    pub question_signature: String,
    pub answers: Vec<Entity<InputState>>,
    /// Background update checks (started by the app, never by tests).
    pub _update_timer: Option<Task<()>>,
    /// The download request already handed to the installer helper.
    pub update_handoff: u64,
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
            question_signature: String::new(),
            answers: Vec::new(),
            _update_timer: None,
            update_handoff: 0,
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
                    // The catalog is scoped to the session's own folder.
                    self.projects.cwd = s.cwd.clone();
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
            if self.projects.picker_open {
                self.project_query
                    .update(cx, |input, cx| input.focus(window, cx));
            } else {
                self.prompt.update(cx, |input, cx| input.focus(window, cx));
            }
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
        self.sync_file_viewer(next, window, cx);
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
                            let path = path.to_string_lossy().into_owned();
                            if bookmark {
                                this.project_path
                                    .update(cx, |input, cx| input.set_value(path, window, cx));
                            } else {
                                this.select_project(&path, window, cx);
                            }
                        }
                    }
                    Ok(Ok(None)) => {}
                    _ if !bookmark && this.new_session => {
                        // The hub is on this machine, so its listing is the
                        // same filesystem the system dialog would have shown.
                        this.projects.notice =
                            "The system folder picker is unavailable; browsing folders on the hub instead.".into();
                        let start = this.projects.cwd.clone();
                        this.browse_to(start, cx);
                    }
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
    /// The request's loading or error line, toned; empty when settled.
    pub(super) fn feature_message(&self, key: &str) -> Div {
        let p = self.appearance.palette();
        match self.view.requests.get(key) {
            Some(s) if s.loading => chrome::notice_line(
                "Loading…",
                chrome::Tone::Loading,
                p,
                SharedString::from(format!("{key}-loading")),
            ),
            Some(s) => match s.error.as_ref().filter(|e| !e.is_empty()) {
                Some(error) => chrome::notice_line(
                    error.clone(),
                    chrome::Tone::Error,
                    p,
                    SharedString::from(format!("{key}-error")),
                ),
                None => div(),
            },
            None => div(),
        }
    }

    pub(super) fn render_feature(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let short = window.viewport_size().height < px(620.);
        let session_cwd = self.selected_session().map(|s| s.cwd.clone());
        let (title, description): (&str, Option<SharedString>) = match self.screen {
            Screen::Recent => (
                "Session history",
                Some("Every session this connection knows about, including ended ones.".into()),
            ),
            Screen::History => (
                "Conversation history",
                Some("A snapshot of retained conversation history. Live messages continue in chat.".into()),
            ),
            Screen::Session => ("Session details", session_cwd.map(Into::into)),
            Screen::Setup => (
                "Agent setup",
                Some("Connect your agents on the machine running this workspace.".into()),
            ),
            Screen::Model => (
                "Change model",
                Some("Choose the model for this session’s next work. A busy provider may queue the change.".into()),
            ),
            _ => ("", None),
        };
        let trailing = match self.screen {
            Screen::Recent => Some(
                self.quiet_button(
                    "history-refresh",
                    "Refresh",
                    IconName::Redo,
                    self.view.connected,
                )
                .when(self.view.connected, |d| {
                    d.on_click(cx.listener(|this, _, _, cx| this.request(Request::Recent, cx)))
                })
                .into_any_element(),
            ),
            _ => None,
        };
        let body = match self.screen {
            Screen::Recent => self.render_recent(cx),
            Screen::History => self.render_history(window, cx),
            Screen::Session => self.render_session(cx),
            Screen::Setup => self.render_setup(cx),
            Screen::Model => self.render_model(cx),
            _ => div(),
        };
        let back = self
            .quiet_button(
                "feature-back",
                if self.extras.return_launch {
                    "Back to new agent"
                } else {
                    "Back to chat"
                },
                IconName::ArrowLeft,
                true,
            )
            .debug_selector(|| "feature-back".into())
            .on_click(cx.listener(|this, _, window, cx| this.back_from_feature(window, cx)));
        let notice = (!self.extras.notice.is_empty()).then(|| {
            let tone = chrome::notice_tone(&self.extras.notice);
            chrome::notice_line(self.extras.notice.clone(), tone, p, "feature-notice")
                .debug_selector(|| "feature-notice".into())
        });
        self.page_view(
            "feature-view",
            CHAT_WIDTH,
            short,
            div()
                .flex()
                .flex_col()
                .gap_5()
                .child(self.page_header(Some(back), None, title, description, trailing, short))
                .children(notice)
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
        let loading = self.view.requests.get("recent").is_some_and(|s| s.loading);
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div().flex().child(self.segmented(
                    "history-filter",
                    vec![(false, "All sessions".to_owned()), (true, "Archived".to_owned())],
                    self.extras.show_archived,
                    |this, archived, _, cx| {
                        this.extras.show_archived = archived;
                        cx.notify();
                    },
                    cx,
                )),
            )
            .child(self.feature_message("recent"))
            .when(sessions.is_empty() && !loading, |d| {
                d.child(empty_note(
                    if self.extras.show_archived {
                        "No archived sessions match. Archive a session to tuck it away without stopping it."
                    } else {
                        "No matching sessions. Try clearing the sidebar search or refreshing."
                    },
                    p,
                ))
            })
            .children(sessions.into_iter().take(500).enumerate().map(|(ix, s)| {
                let open = s.clone();
                let resume = s.clone();
                let id = s.id.clone();
                chrome::card(p)
                    .id(("recent-row", ix))
                    .px_4()
                    .py_3()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            // Actions wrap below a title that would otherwise
                            // be truncated to a word in narrow windows.
                            .flex_1()
                            .min_w(px(220.))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(px(chrome::scale::BODY))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(self.session_title(&s)),
                                    )
                                    .child(div().flex_shrink_0().child(session_badge(&s, p, self.view.connected))),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .min_w_0()
                                    .text_size(px(chrome::scale::CAPTION))
                                    .text_color(rgb(p.muted))
                                    .child(div().flex_shrink_0().child(chrome::model_badge(&s, p, 11.)))
                                    .child(div().flex_shrink_0().text_color(rgb(p.disabled)).child("·"))
                                    .child(div().min_w_0().truncate().font_family(mono_font()).child(s.cwd.clone())),
                            ),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(self.button("archive-recent", if self.archived(&s.id) { "Restore" } else { "Archive" }, true)
                                .on_click(cx.listener(move |this, _, _, cx| this.toggle_archive(&id, cx))))
                            .when(s.stopped() && matches!(s.provider.as_str(), "claude" | "codex"), |d| {
                                d.child(self.button("resume-recent", "Resume…", self.view.connected)
                                    .on_click(cx.listener(move |this, _, window, cx| this.resume_session(&resume, window, cx))))
                            })
                            .child(self.primary_button("open-recent", "Open", self.view.connected)
                                .when(self.view.connected, |d| d.on_click(cx.listener(move |this, _, window, cx| {
                                    this.show_screen(Screen::Conversation, window, cx);
                                    this.command(Command::OpenRecent(Box::new(open.clone())), cx);
                                })))),
                    )
            }))
            .child(
                div()
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.muted))
                    .child("Names and archives are saved on this device for this connection. Archiving keeps the conversation and does not stop an agent."),
            )
    }

    pub(super) fn toggle_archive(&mut self, id: &str, cx: &mut Context<Self>) {
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
        self.seed_project(&session.cwd, cx);
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
    /// Save the Session details name for this device; empty restores the
    /// agent's own title.
    pub(super) fn save_session_name(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.view.selected.clone() else {
            return;
        };
        let name = self.extras.name.read(cx).value().trim().to_owned();
        if name.chars().count() > 200 {
            self.extras.notice = "Use a name of at most 200 characters.".into();
        } else {
            let names = self
                .settings
                .names
                .entry(self.project_scope.clone())
                .or_default();
            if name.is_empty() {
                names.remove(&id);
            } else {
                names.insert(id, name);
            }
            self.save_settings(cx);
            self.extras.notice = "Name saved".into();
        }
        cx.notify();
    }

    fn render_session(&self, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let Some(s) = self.selected_session() else {
            return empty_note("Select a session in the sidebar first.", p);
        };
        let id = s.id.clone();
        let archive = id.clone();
        let end = id.clone();
        let resume = s.clone();
        let busy = self.view.busy || !self.view.connected;
        let archived = self.archived(&s.id);
        let confirming = self.extras.confirm_end.as_ref() == Some(&s.id);
        let fact = |label: &'static str, value: AnyElement| {
            div()
                .flex()
                .items_center()
                .gap_4()
                .min_w_0()
                .py_2()
                .child(
                    div()
                        .w(px(120.))
                        .flex_shrink_0()
                        .text_size(px(chrome::scale::META))
                        .text_color(rgb(p.muted))
                        .child(label),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(chrome::scale::BODY))
                        .child(value),
                )
        };
        let name = chrome::card(p)
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(card_heading("Name", Some("Shown in the sidebar on this device. Leave empty to use the agent’s own title."), p))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(Input::new(&self.extras.name)))
                    .child(
                        self.primary_button("save-session-name", "Save name", true)
                            .flex_shrink_0()
                            .on_click(cx.listener(|this, _, _, cx| this.save_session_name(cx))),
                    ),
            );
        let (status, status_color) = if self.view.connected {
            session_status(s, p)
        } else {
            ("Offline", p.muted)
        };
        let details = chrome::card(p)
            .px_4()
            .py_2()
            .flex()
            .flex_col()
            .child(fact(
                "Status",
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_color(rgb(status_color))
                    .child(status_dot(status_color))
                    .child(status.to_owned())
                    .into_any_element(),
            ))
            .child(fact(
                "Agent",
                chrome::model_badge(s, p, 12.).into_any_element(),
            ))
            .child(fact(
                "Folder",
                div()
                    .truncate()
                    .font_family(mono_font())
                    .text_size(px(chrome::scale::META))
                    .child(s.cwd.clone())
                    .into_any_element(),
            ));
        let actions = chrome::card(p)
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(card_heading("Actions", None, p))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(self.button("archive-session", if archived { "Restore from archive" } else { "Archive on this device" }, true)
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_archive(&archive, cx))))
                    .when(s.stopped() && self.supported_session(), |d| d.child(self.button("resume-session", "Resume session…", !busy)
                        .when(!busy, |d| d.on_click(cx.listener(move |this, _, window, cx| this.resume_session(&resume, window, cx))))))
                    .when(!s.stopped() && !confirming, |d| d.child(self.danger_button("end-session", "End session…", !busy).debug_selector(|| "end-session".into())
                        .when(!busy, |d| d.on_click(cx.listener(move |this, _, _, cx| { this.extras.confirm_end = Some(end.clone()); cx.notify(); }))))),
            )
            .when(confirming, |d| d.child(
                div()
                    .debug_selector(|| "confirm-end-panel".into())
                    .p_3()
                    .rounded(px(p.control_radius))
                    .border_1()
                    .border_color(gpui::Hsla::from(rgb(p.error)).opacity(0.45))
                    .bg(gpui::Hsla::from(rgb(p.error)).opacity(0.08))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(chrome::notice_line("End this agent? Its current work stops. You can resume its conversation later.", chrome::Tone::Warning, p, "confirm-end-copy"))
                    .child(div().flex().gap_2()
                        .child(self.button("cancel-end", "Keep running", true).debug_selector(|| "cancel-end".into()).on_click(cx.listener(|this, _, _, cx| { this.extras.confirm_end = None; cx.notify(); })))
                        .child(self.danger_button("confirm-end", "End session", !busy).debug_selector(|| "confirm-end".into()).when(!busy, |d| d.on_click(cx.listener(|this, _, _, cx| { this.act(Action::Terminate, cx); this.extras.confirm_end = None; }))))),
            ))
            .child(div().text_size(px(chrome::scale::CAPTION)).text_color(rgb(p.muted)).child(
                "Archiving hides the session on this device and keeps it running. Ending stops the agent.",
            ));
        let notice = self.view.notice.clone();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(name)
            .child(details)
            .child(actions)
            .when(!notice.is_empty(), |d| {
                d.child(chrome::notice_line(
                    notice.clone(),
                    chrome::notice_tone(&notice),
                    p,
                    "session-notice",
                ))
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
            .child(div().flex().items_center().gap_2()
                .child(self.quiet_button("older-history", "Previous", IconName::ChevronLeft, page > 0).when(page > 0, |d| d.on_click(cx.listener(|this, _, _, cx| { this.extras.history_page = this.extras.history_page.saturating_sub(1); this.extras.history_scroll.set_offset(gpui::point(px(0.), px(0.))); cx.notify(); }))))
                .child(div().text_size(px(chrome::scale::META)).text_color(rgb(p.muted)).child(format!("Page {} of {}", page + 1, pages)))
                .child(self.quiet_button("newer-history", "Next", IconName::ChevronRight, page + 1 < pages).when(page + 1 < pages, |d| d.on_click(cx.listener(|this, _, _, cx| { this.extras.history_page += 1; this.extras.history_scroll.set_offset(gpui::point(px(0.), px(0.))); cx.notify(); })))))
            .when(state.is_some_and(|s| s.value["first_seq"].as_u64().unwrap_or(0) > 1), |d| d.child(chrome::notice_line("The server has trimmed earlier events; this starts at its oldest retained event.", chrome::Tone::Info, p, "history-trimmed")))
            .when(count == 0 && state.is_some_and(|s| !s.loading && s.error.is_none()), |d| d.child(empty_note("No retained messages are available for this session.", p)))
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
                chrome::card(p).p_4().flex().flex_col().gap_3()
                    .child(div().flex().items_center().gap_3()
                        .child(chrome::provider_mark(provider, 40., p))
                        .child(div().flex_1().min_w_0().flex().flex_col().gap_1()
                            .child(div().text_size(px(chrome::scale::HEADING)).font_weight(FontWeight::SEMIBOLD).child(label))
                            .child(div().text_size(px(chrome::scale::CAPTION)).text_color(rgb(p.muted)).child(if provider == "claude" { "Claude Code" } else { "Codex CLI" })))
                        .child(div().flex().items_center().gap_2().text_size(px(chrome::scale::CAPTION)).text_color(rgb(if found == Some(true) { p.success } else { p.muted }))
                            .child(status_dot(if found == Some(true) { p.success } else { p.muted }))
                            .child(match found { Some(true) => "Installed", Some(false) => "Not found", None => "Not checked" })))
                    .child(div().text_size(px(chrome::scale::META)).text_color(rgb(p.muted)).child(if provider == "claude" { "Install Claude Code, then run claude in a terminal to sign in." } else { "Install Codex CLI, then run codex login in a terminal to sign in." }))
                    .child(div().pt_3().border_t_1().border_color(rgb(p.border)).flex().flex_wrap().items_center().justify_between().gap_3()
                        .child(div().flex().items_center().gap_2().text_size(px(chrome::scale::META)).text_color(rgb(color))
                            .child(if busy && current { brand_spinner(12., p, SharedString::from(format!("setup-{provider}-activity"))) } else { status_dot(color) })
                            .child(description))
                        .child(self.button(if provider == "claude" { "setup-claude" } else { "setup-codex" }, if busy && current { "Checking…" } else { "Check connection" }, !busy && self.view.connected)
                            .when(!busy && self.view.connected, |d| d.on_click(cx.listener(move |this, _, _, cx| this.request(Request::Setup { provider: provider.into(), check: true }, cx))))))
                    .when(current && !busy, |d| d
                        .when_some(state.and_then(|s| s.error.as_ref()), |d, error| d.child(chrome::notice_line(error.clone(), chrome::Tone::Error, p, SharedString::from(format!("setup-{provider}-error")))))
                        .when_some(value["readinessError"].as_str(), |d, error| d.child(chrome::notice_line(error.to_owned(), chrome::Tone::Warning, p, SharedString::from(format!("setup-{provider}-readiness"))))))
            }))
            .child(chrome::notice_line("Checking a connection sends a small test request and may use your provider allowance. Git is required for reviewing changes.", chrome::Tone::Info, p, "setup-info"))
            .child(div().flex().child(self.quiet_button("setup-refresh", "Recheck installed agents", IconName::Redo, !busy && self.view.connected)
                .when(!busy && self.view.connected, |d| d.on_click(cx.listener(|this, _, _, cx| this.request(Request::Setup { provider: this.provider.into(), check: false }, cx))))))
    }
    fn render_model(&self, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        if !self.supported_session() {
            return empty_note(
                "Model switching is available for Claude and Codex sessions.",
                p,
            );
        }
        let busy = self.view.busy || !self.view.connected;
        let notice = self.view.notice.clone();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                chrome::card(p)
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(self.render_launch_options(busy, cx))
                    .child(
                        div()
                            .pt_3()
                            .border_t_1()
                            .border_color(rgb(p.border))
                            .flex()
                            .justify_end()
                            .child(
                                self.primary_button("apply-model", "Apply model", !busy)
                                    .when(!busy, |d| {
                                        d.on_click(cx.listener(|this, _, _, cx| {
                                            let model = if this.model_choice == "__custom" {
                                                this.model.read(cx).value().trim().to_owned()
                                            } else {
                                                this.model_choice.clone()
                                            };
                                            if model.is_empty() {
                                                this.extras.notice =
                                                    "Choose a model or enter its exact ID.".into();
                                                cx.notify();
                                                return;
                                            }
                                            this.act(
                                                Action::SetModel {
                                                    model,
                                                    context_window: this.context_window,
                                                },
                                                cx,
                                            );
                                        }))
                                    }),
                            ),
                    ),
            )
            .when(!notice.is_empty(), |d| {
                d.child(chrome::notice_line(
                    notice.clone(),
                    chrome::notice_tone(&notice),
                    p,
                    "model-notice",
                ))
            })
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

/// Muted explanatory line for an empty list or a missing selection.
fn empty_note(text: &'static str, p: Palette) -> Div {
    div()
        .py_6()
        .text_center()
        .text_size(px(chrome::scale::META))
        .text_color(rgb(p.muted))
        .child(text)
}

/// Heading and optional description at the top of a page card.
fn card_heading(title: &'static str, description: Option<&'static str>, p: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(px(chrome::scale::HEADING))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .children(description.map(|text| {
            div()
                .text_size(px(chrome::scale::META))
                .text_color(rgb(p.muted))
                .child(text)
        }))
}
