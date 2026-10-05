//! Secondary views keep the conversation uncluttered and preserve its draft/scroll.
use super::*;
use gpui::AnyElement;
use gpui_component::scroll::ScrollableElement;
use serde_json::Value;
use wks_native::features::{AttachmentSource, Request, attention_transition};

/// A draft attachment: name, hub path, and its thumbnail (`None` loading,
/// `Some(None)` unavailable or not an image).
pub(super) type DraftFile = (String, String, Option<Option<Arc<gpui::Image>>>);

/// Starts the installer helper; returns once it is waiting for the app.
pub(super) trait UpdateStarter:
    Fn(&wks_native::updates::Handoff) -> anyhow::Result<wks_native::updates::ReadyHelper> + Send + Sync
{
}
impl<
    F: Fn(&wks_native::updates::Handoff) -> anyhow::Result<wks_native::updates::ReadyHelper>
        + Send
        + Sync,
> UpdateStarter for F
{
}

pub(super) struct Extras {
    pub name: Entity<InputState>,
    pub return_launch: bool,
    pub approval_details: bool,
    pub selected_options: Vec<std::collections::BTreeSet<usize>>,
    pub notice: String,
    /// Title-island notices the user closed, by slot and text: a slot that
    /// changes, or later repeats the same text, shows again.
    pub dismissed_notices: Vec<(&'static str, String)>,
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
    /// The hub accepted answers for the current question set; the picker
    /// stays read-only until the set changes or the user chooses to edit.
    pub answers_sent: bool,
    /// Signature and literal answers accepted into the command queue. Keep it
    /// across question changes so a late receipt cannot lock a new question.
    pub answer_submission: Option<(String, Vec<String>)>,
    pub answer_error: Option<String>,
    pub question_scroll: gpui::ScrollHandle,
    /// Keyboard focus for each question's option rows, so moving focus can
    /// scroll the focused row into view inside the question list.
    pub option_focus: Vec<Vec<FocusHandle>>,
    /// The list child that last held focus, scrolled into view once.
    pub question_focused: std::cell::Cell<Option<usize>>,
    /// Background update checks (started by the app, never by tests).
    pub _update_timer: Option<Task<()>>,
    /// The download request already handed to the installer helper.
    pub update_handoff: u64,
    /// A downloaded, verified installer and its version, until handed off.
    pub update_installer: Option<(String, String)>,
    /// The helper is starting; the app quits once it reports it is waiting.
    pub update_handing_off: bool,
    /// An accepted helper, retained even if it later fails: never start a
    /// competing installer after an ambiguous handoff.
    pub update_helper_ready: Option<wks_native::updates::ReadyHelper>,
    /// Hand-off progress, failures and the last update's outcome.
    pub update_notice: String,
    /// Starts the installer helper (tests substitute a recorder).
    pub update_starter: std::sync::Arc<dyn UpdateStarter>,
    /// The session's model, context window and effort when Change model
    /// opened, so Apply sends only what the user actually changed.
    pub model_base: Option<(String, Option<u64>, String)>,
    /// The hub's `agents.childFullAccess` / `fleetFullAccess` as last read or
    /// saved; `None` until read. Never assumed from a pending toggle.
    pub child_access: Option<(bool, bool)>,
    pub child_access_receipt: u64,
    /// The hub's `agents.autoTitle` as last read or saved (see
    /// `features::title_settings`); `None` until read.
    pub titles: Option<Value>,
    pub titles_receipt: u64,
    /// Harness whose title model is edited while titles follow each agent;
    /// seeded from the default agent on the first Settings visit.
    pub title_harness: &'static str,
    pub title_harness_seeded: bool,
    pub title_picker: Entity<SelectState<SearchableVec<super::launch::PickerItem>>>,
    pub _title_picker_subscription: gpui::Subscription,
    /// Provider, choice and rows the picker was last built for.
    pub title_picker_key: String,
    /// The last change sent, until a newer titles receipt arrives.
    pub title_sent: Option<wks_native::features::TitleChange>,
    pub title_sent_after: u64,
    /// Archive changes sent to the hub and not yet confirmed: shown at once,
    /// settled by the hub's document or by the request's receipt.
    pub archive_pending: std::collections::BTreeMap<String, bool>,
    pub archive_receipt: u64,
    /// Device-only archives already offered to the hub this run.
    pub archive_migrating: std::collections::BTreeSet<String>,
}
impl Extras {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let title_picker = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(super::titles::title_model_items("", &[], "")),
                Some(gpui_component::IndexPath::new(0)),
                window,
                cx,
            )
            .searchable(true)
        });
        let _title_picker_subscription =
            cx.subscribe_in(&title_picker, window, Workspace::on_title_pick);
        Self {
            name: cx.new(|cx| InputState::new(window, cx).placeholder("Session name")),
            return_launch: false,
            approval_details: false,
            selected_options: Vec::new(),
            notice: String::new(),
            dismissed_notices: Vec::new(),
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
            answers_sent: false,
            answer_submission: None,
            answer_error: None,
            question_scroll: gpui::ScrollHandle::new(),
            option_focus: Vec::new(),
            question_focused: Default::default(),
            _update_timer: None,
            update_handoff: 0,
            update_installer: None,
            update_handing_off: false,
            update_helper_ready: None,
            update_notice: String::new(),
            update_starter: std::sync::Arc::new(wks_native::updates::hand_off),
            model_base: None,
            child_access: None,
            child_access_receipt: 0,
            titles: None,
            titles_receipt: 0,
            title_harness: "claude",
            title_harness_seeded: false,
            title_picker,
            _title_picker_subscription,
            title_picker_key: String::new(),
            title_sent: None,
            title_sent_after: 0,
            archive_pending: Default::default(),
            archive_receipt: 0,
            archive_migrating: Default::default(),
        }
    }
}
/// OS notifications for sessions that need attention while the window is in
/// the background. Shown off the UI thread, at most five per update.
#[cfg(not(all(test, feature = "ui-tests")))]
fn post_attention_alerts(alerts: Vec<(String, String)>, cx: &mut Context<Workspace>) {
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

/// The UI tests record alerts instead of showing them. A real one is
/// platform FFI on the GPUI test thread, which runs "background" tasks: a
/// WinRT toast in the COM apartment each test platform opens and closes (the
/// serial Windows suite died with an access violation starting the second
/// test to raise one, on a fresh thread), D-Bus on the developer's desktop.
#[cfg(all(test, feature = "ui-tests"))]
fn post_attention_alerts(alerts: Vec<(String, String)>, _: &mut Context<Workspace>) {
    POSTED_ALERTS.with_borrow_mut(|posted| posted.extend(alerts.into_iter().take(5)));
}

#[cfg(all(test, feature = "ui-tests"))]
thread_local! {
    pub(super) static POSTED_ALERTS: std::cell::RefCell<Vec<(String, String)>> =
        const { std::cell::RefCell::new(Vec::new()) };
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
    /// Hidden from the normal list. The hub's shared archive (so web and
    /// every other client agree) plus any device-only archive not yet moved
    /// there; an unconfirmed change shows as already made.
    pub(super) fn archived(&self, id: &str) -> bool {
        if let Some(&pending) = self.extras.archive_pending.get(id) {
            return pending;
        }
        self.locally_archived(id)
            || self
                .view
                .session_archive
                .as_ref()
                .is_some_and(|doc| wks_native::features::archive_contains(doc, id))
    }
    fn locally_archived(&self, id: &str) -> bool {
        self.settings
            .archived
            .get(&self.project_scope)
            .is_some_and(|ids| ids.iter().any(|s| s == id))
    }
    /// The connected hub keeps a shared archive (older hubs do not).
    fn hub_archive(&self) -> bool {
        self.view.connected && !self.demo && self.view.session_archive.is_some()
    }
    /// Settle sent archive changes and move device-only archives to the hub,
    /// so a session archived here before archives were shared hides on the
    /// web too. A device copy is dropped only once the hub holds it.
    fn sync_archive(&mut self, next: &View, cx: &mut Context<Self>) {
        for receipt in next
            .archive_receipts
            .iter()
            .filter(|r| r.number > self.extras.archive_receipt)
        {
            if self.extras.archive_pending.get(&receipt.session) == Some(&receipt.archived) {
                self.extras.archive_pending.remove(&receipt.session);
            }
            if let Some(error) = &receipt.error {
                let verb = if receipt.archived {
                    "archive"
                } else {
                    "restore"
                };
                self.extras.notice = format!("Could not {verb} the session: {error}");
            }
        }
        if let Some(last) = next.archive_receipts.back() {
            self.extras.archive_receipt = self.extras.archive_receipt.max(last.number);
        }
        let Some(doc) = next.session_archive.clone() else {
            return;
        };
        self.extras
            .archive_pending
            .retain(|id, archived| wks_native::features::archive_contains(&doc, id) != *archived);
        if !next.connected || self.demo {
            return;
        }
        let local = self
            .settings
            .archived
            .get(&self.project_scope)
            .cloned()
            .unwrap_or_default();
        let mut moved = false;
        for id in local {
            if wks_native::features::archive_contains(&doc, &id) {
                if let Some(ids) = self.settings.archived.get_mut(&self.project_scope) {
                    ids.retain(|s| s != &id);
                }
                moved = true;
            } else if !self.extras.archive_pending.contains_key(&id)
                && self.extras.archive_migrating.insert(id.clone())
            {
                let _ = self
                    .controller
                    .command(Command::Request(Request::SetArchive {
                        session: id,
                        archived: true,
                    }));
            }
        }
        if moved {
            self.settings.archived.retain(|_, ids| !ids.is_empty());
            self.save_settings(cx);
        }
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
                        .update(cx, |input, cx| input.set_value(s.model.clone(), window, cx));
                    self.context_window = s.context_window;
                    self.effort = s.effort.clone();
                    self.extras.model_base =
                        Some((s.model.clone(), s.context_window, s.effort.clone()));
                    self.select_exact_model(window, cx);
                    self.reconcile_effort(window, cx);
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
        self.sync_archive(next, cx);
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
                post_attention_alerts(alerts, cx);
            }
        }
        if let Some(state) = next.requests.get("child-access")
            && !state.loading
            && state.number > self.extras.child_access_receipt
        {
            self.extras.child_access_receipt = state.number;
            if state.error.is_none() {
                self.extras.child_access = Some((
                    state.value["childFullAccess"] == true,
                    state.value["fleetFullAccess"] == true,
                ));
            }
        }
        if let Some(state) = next.requests.get("titles")
            && !state.loading
            && state.number > self.extras.titles_receipt
        {
            self.extras.titles_receipt = state.number;
            // A failed write leaves the last verified state, never the guess.
            if state.error.is_none() {
                self.extras.titles = Some((*state.value).clone());
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
            self.extras.answers_sent = false;
            self.extras.answer_error = None;
            self.extras
                .question_scroll
                .set_offset(gpui::Point::default());
            self.extras.question_focused.set(None);
            self.extras.option_focus = pending
                .map(questions)
                .unwrap_or_default()
                .iter()
                .map(|q| {
                    let count = q["options"].as_array().map_or(0, Vec::len);
                    (0..count)
                        .map(|_| cx.focus_handle().tab_stop(true))
                        .collect()
                })
                .collect();
            self.extras.answers = pending
                .map(questions)
                .unwrap_or_default()
                .iter()
                .map(|q| {
                    let has_options = q["options"].as_array().is_some_and(|o| !o.is_empty());
                    cx.new(|cx| {
                        InputState::new(window, cx).placeholder(if has_options {
                            "Or type a different answer"
                        } else {
                            "Type your answer"
                        })
                    })
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
    /// Draft attachments above the composer: images as thumbnails (read back
    /// from the hub, so they show the uploaded file), PDFs and images whose
    /// preview is loading or unavailable as named chips. Each keeps Remove;
    /// a thumbnail opens the image in the viewer.
    pub(super) fn render_draft_attachments(
        &self,
        files: Vec<DraftFile>,
        cwd: &str,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let busy = self.view.busy;
        let remove = |this: &Self, ix: usize, cx: &mut Context<Self>| {
            this.icon_button(
                "remove-attachment",
                "Remove attachment",
                IconName::Close,
                !busy,
            )
            .size(px(24.))
            .when(!busy, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(id) = &this.view.selected
                        && let Some(files) = this.extras.attachments.get_mut(id)
                        && ix < files.len()
                    {
                        files.remove(ix);
                    }
                    cx.notify();
                }))
            })
        };
        let uploading = self.uploading()
            && self.view.requests.get("upload").is_some_and(|state| {
                matches!(&state.request, Request::Upload { session, .. }
                    if Some(session) == self.view.selected.as_ref())
            });
        div()
            .flex()
            .flex_wrap()
            .items_end()
            .gap_2()
            .children(
                files
                    .into_iter()
                    .enumerate()
                    .map(|(ix, (name, path, preview))| {
                        match preview {
                            Some(Some(image)) => {
                                let link = wks_native::links::tool_file(cwd, &path, None);
                                let owner = self.view.selected.clone().unwrap_or_default();
                                div()
                                    .id(("attachment", ix))
                                    .debug_selector(move || format!("draft-thumbnail-{ix}"))
                                    .relative()
                                    .rounded_md()
                                    .overflow_hidden()
                                    .border_1()
                                    .border_color(rgb(p.border))
                                    .bg(rgb(p.base))
                                    .child(
                                        div()
                                            .id(("attachment-preview", ix))
                                            .cursor_pointer()
                                            .tooltip({
                                                let name = SharedString::from(name.clone());
                                                move |window, cx| {
                                                    gpui_component::tooltip::Tooltip::new(
                                                        name.clone(),
                                                    )
                                                    .build(window, cx)
                                                }
                                            })
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.open_link(&owner, link.clone(), cx)
                                            }))
                                            .child(
                                                // A fixed tile: the decode is async, and
                                                // the row must not jump when it lands.
                                                gpui::img(image)
                                                    .w(px(96.))
                                                    .h(px(72.))
                                                    .object_fit(gpui::ObjectFit::Cover),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .absolute()
                                            .top(px(2.))
                                            .right(px(2.))
                                            .rounded_full()
                                            .bg(rgb(p.surface))
                                            .child(remove(self, ix, cx)),
                                    )
                            }
                            preview => div()
                                .id(("attachment", ix))
                                .debug_selector(move || format!("draft-attachment-{ix}"))
                                .pl_2()
                                .pr_1()
                                .py_1()
                                .rounded_md()
                                .bg(rgb(p.selected))
                                .max_w_full()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(match preview {
                                    None => brand_spinner(
                                        12.,
                                        p,
                                        SharedString::from(format!("draft-preview-{ix}")),
                                    )
                                    .into_any_element(),
                                    Some(_) => Icon::new(IconName::File)
                                        .size(px(14.))
                                        .text_color(rgb(p.accent))
                                        .into_any_element(),
                                })
                                .child(div().min_w_0().truncate().text_size(px(12.)).child(name))
                                .child(remove(self, ix, cx)),
                        }
                    }),
            )
            .when(uploading, |d| {
                d.child(
                    div()
                        .debug_selector(|| "draft-uploading".into())
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .bg(rgb(p.selected))
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.))
                        .text_color(rgb(p.muted))
                        .child(brand_spinner(12., p, "draft-uploading-spinner"))
                        .child("Attaching…"),
                )
            })
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
                "Model and effort",
                Some("Choose the model and reasoning effort for this session’s next work. A busy provider may queue the change.".into()),
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
                    .child("Names are saved on this device. Archives are shared with every client of this hub, web included. Archiving keeps the conversation and does not stop an agent."),
            )
    }

    /// Archive or restore. Only ever a visibility change: no stop, signal,
    /// selection change or forget is sent, and a running session keeps
    /// running. Shared through the hub when it supports it; otherwise kept on
    /// this device for this connection, as before.
    pub(super) fn toggle_archive(&mut self, id: &str, cx: &mut Context<Self>) {
        let archive = !self.archived(id);
        if !archive && self.locally_archived(id) {
            if let Some(ids) = self.settings.archived.get_mut(&self.project_scope) {
                ids.retain(|s| s != id);
            }
            self.settings.archived.retain(|_, ids| !ids.is_empty());
            self.save_settings(cx);
        }
        let shared = !archive
            && self
                .view
                .session_archive
                .as_ref()
                .is_some_and(|doc| wks_native::features::archive_contains(doc, id));
        if self.hub_archive() && (archive || shared || self.extras.archive_pending.contains_key(id))
        {
            match self
                .controller
                .command(Command::Request(Request::SetArchive {
                    session: id.into(),
                    archived: archive,
                })) {
                Ok(()) => {
                    self.extras.archive_pending.insert(id.into(), archive);
                    self.extras.notice.clear();
                }
                Err(error) => self.extras.notice = error.to_string(),
            }
        } else if archive {
            self.settings
                .archived
                .entry(self.project_scope.clone())
                .or_default()
                .push(id.into());
            self.save_settings(cx);
        }
        cx.notify();
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
                    .child(self.button("archive-session", if archived { "Restore from archive" } else { "Archive" }, true)
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
                "Archiving hides the session from the list in every client of this hub and keeps it running. Ending stops the agent.",
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
                            .child(self.primary_button("apply-model", "Apply", !busy).when(
                                !busy,
                                |d| {
                                    d.on_click(
                                        cx.listener(|this, _, _, cx| this.apply_model_change(cx)),
                                    )
                                },
                            )),
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
    /// Send exactly what changed on Change model: the model (with its context
    /// window), the effort, or both in that order. An unchanged form sends
    /// nothing, and a model the user did not pick is never substituted.
    pub(super) fn apply_model_change(&mut self, cx: &mut Context<Self>) {
        let model = if self.model_choice == "__custom" {
            self.model.read(cx).value().trim().to_owned()
        } else {
            self.model_choice.clone()
        };
        let (base_model, base_window, base_effort) =
            self.extras.model_base.clone().unwrap_or_default();
        let model_changed =
            !model.is_empty() && (model != base_model || self.context_window != base_window);
        let effort_changed = !self.effort.is_empty() && self.effort != base_effort;
        if model.is_empty() && !effort_changed {
            self.extras.notice = "Choose a model or enter its exact ID.".into();
            cx.notify();
            return;
        }
        let action = match (model_changed, effort_changed) {
            (true, effort) => Action::SetModel {
                model,
                context_window: self.context_window,
                effort: effort.then(|| self.effort.clone()),
            },
            (false, true) => Action::SetEffort(self.effort.clone()),
            (false, false) => {
                self.extras.notice = "The session already uses this model and effort.".into();
                cx.notify();
                return;
            }
        };
        self.act(action, cx);
    }
    /// Move the Change model baseline to what the hub accepted, so a repeated
    /// Apply does not resend it; a refused change leaves it as it was.
    pub(super) fn note_model_receipt(&mut self, receipt: &wks_native::controller::Receipt) {
        if self.view.selected.as_ref() != Some(&receipt.session) {
            return;
        }
        let Some(base) = &mut self.extras.model_base else {
            return;
        };
        let model_applied = receipt.error.is_none()
            || receipt
                .error
                .as_deref()
                .is_some_and(|e| e.contains("model change was accepted"));
        match &receipt.action {
            Action::SetModel {
                model,
                context_window,
                effort,
            } => {
                if model_applied {
                    base.0 = model.clone();
                    base.1 = *context_window;
                }
                if receipt.error.is_none()
                    && let Some(effort) = effort
                {
                    base.2 = effort.clone();
                }
            }
            Action::SetEffort(effort) if receipt.error.is_none() => base.2 = effort.clone(),
            _ => {}
        }
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
    /// Every question has a non-empty answer (a choice or typed text).
    pub(super) fn answers_ready(&self, cx: &App) -> bool {
        !self.extras.answers.is_empty()
            && self
                .question_answers(cx)
                .iter()
                .all(|s| !s.trim().is_empty())
    }

    /// The only way answers leave the picker: the Send button, Ctrl/Cmd+Enter
    /// inside the picker, or Enter in a typed answer. All of them need every
    /// question answered and nothing already sent for this question set.
    pub(super) fn submit_answers(&mut self, cx: &mut Context<Self>) {
        // The same gate as the rendered controls: keyboard sends must not
        // reach a session the user is navigating away from.
        if self.extras.answers_sent
            || self.extras.answer_submission.is_some()
            || self.view.child.is_some()
            || self.view.busy
            || !self.view.connected
            || self.view.loading
            || self
                .navigation_selected
                .as_ref()
                .is_some_and(|id| Some(id) != self.view.selected.as_ref())
            || !self
                .selected_session()
                .is_some_and(|s| s.questions.is_some() && !s.stopped())
            || !self.answers_ready(cx)
        {
            return;
        }
        let answers = self.question_answers(cx);
        let Some(session) = self.view.selected.clone() else {
            return;
        };
        match self.controller.command(Command::Act {
            session,
            action: Action::Answers(answers.clone()),
        }) {
            Ok(()) => {
                self.extras.answer_submission =
                    Some((self.extras.question_signature.clone(), answers));
                self.extras.answer_error = None;
                self.local_notice.clear();
            }
            Err(error) => self.local_notice = error.to_string(),
        }
        cx.notify();
    }

    /// Toggle (multiple choice) or choose (single choice) one option. A choice
    /// replaces typed text for that question, as typing replaces a choice.
    fn pick_option(
        &mut self,
        ix: usize,
        option_ix: usize,
        multiple: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.extras.answers_sent
            || self.extras.answer_submission.is_some()
            || self.view.busy
            || !self.view.connected
        {
            return;
        }
        let Some(input) = self.extras.answers.get(ix).cloned() else {
            return;
        };
        if let Some(selected) = self.extras.selected_options.get_mut(ix) {
            let typed = !input.read(cx).value().is_empty();
            if multiple && !typed && selected.contains(&option_ix) {
                selected.remove(&option_ix);
            } else {
                if !multiple || typed {
                    selected.clear();
                }
                selected.insert(option_ix);
            }
        }
        input.update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    /// Enter in a typed answer: send when every question is answered,
    /// otherwise move to the next unanswered question's answer field. The
    /// key is consumed either way, so it never becomes text in a field;
    /// during an IME composition it is left to the input method.
    pub(super) fn answer_enter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self
            .extras
            .answers
            .iter()
            .position(|input| input.read(cx).focus_handle(cx).is_focused(window))
        else {
            cx.propagate();
            return;
        };
        let composing = self.extras.answers[ix]
            .update(cx, |input, cx| {
                gpui::EntityInputHandler::marked_text_range(input, window, cx)
            })
            .is_some_and(|range| !range.is_empty());
        if composing {
            cx.propagate();
            return;
        }
        if self.answers_ready(cx) {
            self.submit_answers(cx);
            return;
        }
        let answers = self.question_answers(cx);
        let count = answers.len();
        if let Some(next) = (1..count)
            .map(|step| (ix + step) % count)
            .find(|i| answers[*i].trim().is_empty())
        {
            let handle = self.extras.answers[next].read(cx).focus_handle(cx);
            window.focus(&handle);
        }
    }

    /// Docked AskUserQuestion picker: one themed card above the composer.
    /// Questions scroll inside the card; the header and the Send row stay
    /// visible so a long set can never push the submit control out of view.
    pub(super) fn render_questions(
        &self,
        session: &Session,
        enabled: bool,
        compact: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        let list = session
            .questions
            .as_ref()
            .map(questions)
            .unwrap_or_default();
        let answers = self.question_answers(cx);
        let total = answers.len();
        let answered = answers.iter().filter(|a| !a.trim().is_empty()).count();
        let ready = self.answers_ready(cx);
        let sent = self.extras.answers_sent;
        let interactive = enabled && !sent && self.extras.answer_submission.is_none();
        let failed = self.extras.answer_error.clone();
        let pad = px(if compact { 8. } else { 12. });
        let body_max = window.viewport_size().height * if compact { 0.4 } else { 0.46 };
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(12.))
            .child(status_dot(p.warning))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(p.warning))
                    .child(if total > 1 && !compact {
                        format!("Needs your input · {total} questions")
                    } else {
                        "Needs your input".into()
                    }),
            )
            .when(total > 1, |d| {
                d.child(
                    div()
                        .debug_selector(|| "question-progress".into())
                        .flex_shrink_0()
                        .text_size(px(chrome::scale::CAPTION))
                        .text_color(rgb(if answered == total {
                            p.success
                        } else {
                            p.muted
                        }))
                        .child(format!("{answered} of {total} answered")),
                )
            });
        let card = div()
            .id("question-choices")
            .debug_selector(|| "question-card".into())
            .key_context("QuestionPicker")
            .on_action(cx.listener(|this, _: &SubmitAnswers, _, cx| this.submit_answers(cx)))
            .on_action(
                cx.listener(|this, _: &AnswerEnter, window, cx| this.answer_enter(window, cx)),
            )
            .occlude()
            .w_full()
            .min_h_0()
            .flex_shrink()
            .p(pad)
            .rounded(px(p.panel_radius))
            .shadow(chrome::floating_shadow(p))
            .bg(rgb(p.surface))
            .border_1()
            .border_color(rgb(p.border))
            .flex()
            .flex_col()
            .gap(px(if compact { 6. } else { 10. }));
        if list.is_empty() {
            // A question the hub could not structure: answer from the composer.
            return card
                .child(header)
                .child(
                    div()
                        .text_size(px(chrome::scale::META))
                        .text_color(rgb(p.muted))
                        .child(
                            "The agent asked for input. Type your answer in the composer below.",
                        ),
                )
                .child(
                    div().flex().justify_end().child(
                        self.primary_button("answer-text", "Answer with composer", enabled)
                            .when(enabled, |d| {
                                d.on_click(cx.listener(|this, _, _, cx| {
                                    this.act(
                                        Action::Answer(this.composer.read(cx).value().to_string()),
                                        cx,
                                    )
                                }))
                            }),
                    ),
                );
        }
        let failed = failed.filter(|_| !sent);
        let status: Option<AnyElement> = if let Some(error) = failed {
            Some(
                chrome::notice_line(error, chrome::Tone::Error, p, "question-error")
                    .into_any_element(),
            )
        } else if sent {
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(chrome::notice_line(
                        "Answers sent. Waiting for the agent…",
                        chrome::Tone::Loading,
                        p,
                        "question-sent",
                    )))
                    .child(
                        self.button("edit-answers", "Edit answers", enabled)
                            .debug_selector(|| "edit-answers".into())
                            .when(enabled, |d| {
                                d.on_click(cx.listener(|this, _, _, cx| {
                                    this.extras.answers_sent = false;
                                    cx.notify();
                                }))
                            }),
                    )
                    .into_any_element(),
            )
        } else if self.view.busy || self.extras.answer_submission.is_some() {
            Some(
                chrome::notice_line("Sending…", chrome::Tone::Loading, p, "question-sending")
                    .into_any_element(),
            )
        } else if compact {
            // Short windows: the header's progress and Send carry this.
            None
        } else if ready {
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.muted))
                    .child(keycap(
                        if cfg!(target_os = "macos") {
                            "⌘ Enter"
                        } else {
                            "Ctrl Enter"
                        },
                        p,
                    ))
                    .child("to send")
                    .into_any_element(),
            )
        } else {
            Some(
                div()
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.muted))
                    .child(match total - answered {
                        _ if total == 1
                            && list[0]["options"].as_array().is_none_or(Vec::is_empty) =>
                        {
                            "Type an answer to send".to_owned()
                        }
                        _ if total == 1 => "Choose an option or type an answer".to_owned(),
                        1 => "Answer 1 more question to send".to_owned(),
                        left => format!("Answer {left} more questions to send"),
                    })
                    .into_any_element(),
            )
        };
        let submit = self
            .primary_button(
                "submit-answers",
                if total > 1 {
                    "Send answers"
                } else {
                    "Send answer"
                },
                interactive && ready && !self.view.busy,
            )
            .debug_selector(|| "submit-answers".into())
            .flex_shrink_0()
            .when(interactive && ready, |d| {
                d.on_click(cx.listener(|this, _, _, cx| this.submit_answers(cx)))
            });
        // Rows are direct children of the list so the focused one (Tab,
        // arrows, Enter to the next answer) can be scrolled into view.
        let mut children = Vec::new();
        let mut focused = None;
        for (ix, q) in list.iter().enumerate() {
            self.push_question(
                ix,
                q,
                total,
                interactive,
                compact,
                window,
                cx,
                &mut children,
                &mut focused,
            );
        }
        if focused != self.extras.question_focused.get() {
            self.extras.question_focused.set(focused);
            if let Some(child) = focused {
                self.extras.question_scroll.scroll_to_item(child);
            }
        }
        let body = div()
            .id("question-list")
            .debug_selector(|| "question-list".into())
            .relative()
            .min_h_0()
            .flex_shrink()
            .max_h(body_max)
            .pr_2()
            .overflow_y_scroll()
            .track_scroll(&self.extras.question_scroll)
            .flex()
            .flex_col()
            .gap(px(if compact { 3. } else { 5. }))
            .children(children)
            // After the rows: the scrollbar layer is a child too, and
            // scroll_to_item indexes children.
            .vertical_scrollbar(&self.extras.question_scroll);
        if compact {
            // One header row holds progress and Send; any status sits under it.
            return card
                .child(header.child(submit))
                .children(status.map(|status| div().flex_shrink_0().child(status)))
                .child(body);
        }
        card.child(header).child(body).child(
            div()
                .debug_selector(|| "question-footer".into())
                .flex_shrink_0()
                .pt(px(8.))
                .border_t_1()
                .border_color(rgb(p.border))
                .flex()
                .items_center()
                .gap_2()
                .child(div().flex_1().min_w_0().children(status))
                .child(submit),
        )
    }

    /// One question as list rows: overline (position, header, how to answer,
    /// answered mark), the question, its options, then the typed answer.
    #[allow(clippy::too_many_arguments)]
    fn push_question(
        &self,
        ix: usize,
        q: &Value,
        total: usize,
        interactive: bool,
        compact: bool,
        window: &Window,
        cx: &mut Context<Self>,
        children: &mut Vec<AnyElement>,
        focused: &mut Option<usize>,
    ) {
        let p = self.appearance.palette();
        let multiple = q["multiSelect"].as_bool().unwrap_or(false);
        let options = q["options"].as_array().cloned().unwrap_or_default();
        let header = q["header"]
            .as_str()
            .map(str::trim)
            .filter(|h| !h.is_empty());
        let text = q["question"]
            .as_str()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .or(header)
            .unwrap_or("Your answer is needed")
            .to_owned();
        let typed = self
            .extras
            .answers
            .get(ix)
            .is_some_and(|input| !input.read(cx).value().trim().is_empty());
        let answered = typed
            || self
                .extras
                .selected_options
                .get(ix)
                .is_some_and(|s| !s.is_empty());
        // Free text needs no instruction: its field's placeholder says it.
        let how = match (options.is_empty(), multiple) {
            (true, _) => "",
            (false, true) => "Choose any",
            (false, false) => "Choose one",
        };
        let mut overline = String::new();
        if total > 1 {
            overline.push_str(&format!("{} of {total}", ix + 1));
        }
        if let Some(header) = header {
            if !overline.is_empty() {
                overline.push_str(" · ");
            }
            overline.push_str(header);
        }
        children.push(
            div()
                .debug_selector(move || format!("question-{ix}"))
                .flex()
                .items_center()
                .gap_2()
                .when(ix > 0, |d| {
                    d.mt(px(if compact { 5. } else { 7. }))
                        .pt(px(if compact { 8. } else { 12. }))
                        .border_t_1()
                        .border_color(rgb(p.border))
                })
                .text_size(px(chrome::scale::OVERLINE))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(p.muted))
                .when(!overline.is_empty(), |d| {
                    d.child(div().min_w_0().truncate().child(overline.to_uppercase()))
                })
                .when(!how.is_empty(), |d| {
                    d.child(
                        div()
                            .flex_shrink_0()
                            .font_weight(FontWeight::NORMAL)
                            .text_size(px(chrome::scale::CAPTION))
                            .child(how),
                    )
                })
                .child(div().flex_1())
                .when(answered, |d| {
                    d.child(
                        div()
                            .debug_selector(move || format!("question-{ix}-answered"))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_size(px(chrome::scale::CAPTION))
                            .font_weight(FontWeight::NORMAL)
                            .text_color(rgb(p.success))
                            .child(Icon::new(IconName::CircleCheck).size(px(12.)))
                            .child("Answered"),
                    )
                })
                .into_any_element(),
        );
        children.push(
            div()
                .debug_selector(move || format!("question-{ix}-text"))
                .mb(px(if compact { 1. } else { 3. }))
                .text_size(px(14.))
                .line_height(gpui::relative(1.4))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(p.text))
                .child(text)
                .into_any_element(),
        );
        for (option_ix, option) in options.iter().enumerate() {
            let chosen = !typed
                && self
                    .extras
                    .selected_options
                    .get(ix)
                    .is_some_and(|selected| selected.contains(&option_ix));
            let handle = self
                .extras
                .option_focus
                .get(ix)
                .and_then(|row| row.get(option_ix));
            if handle.is_some_and(|h| h.is_focused(window)) {
                *focused = Some(children.len());
            }
            children.push(
                self.render_option(
                    ix,
                    option_ix,
                    option,
                    options.len(),
                    multiple,
                    chosen,
                    interactive,
                    compact,
                    handle,
                    cx,
                )
                .into_any_element(),
            );
        }
        if let Some(input) = self.extras.answers.get(ix) {
            if input.read(cx).focus_handle(cx).is_focused(window) {
                *focused = Some(children.len());
            }
            children.push(
                div()
                    .debug_selector(move || format!("question-{ix}-answer"))
                    .mt(px(if compact { 1. } else { 3. }))
                    .text_size(px(chrome::scale::BODY))
                    .child(Input::new(input).disabled(!interactive))
                    .into_any_element(),
            );
        }
    }

    /// An option row: a number (single choice) or checkbox (multiple choice)
    /// badge, the literal label and its description. Tab/arrows move between
    /// rows, Enter/Space choose, and 1–9 choose within the same question.
    #[allow(clippy::too_many_arguments)]
    fn render_option(
        &self,
        ix: usize,
        option_ix: usize,
        option: &Value,
        count: usize,
        multiple: bool,
        chosen: bool,
        interactive: bool,
        compact: bool,
        focus: Option<&FocusHandle>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        let label = option["label"]
            .as_str()
            .or(option.as_str())
            .unwrap_or("")
            .to_owned();
        let description = option["description"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_owned();
        let badge_size = if compact { 16. } else { 18. };
        let badge = div()
            .debug_selector(move || format!("question-{ix}-option-{option_ix}-badge"))
            .flex_shrink_0()
            .mt(px(1.))
            .size(px(badge_size))
            .rounded(px(if multiple { 4. } else { badge_size / 2. }))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(10.))
            .font_weight(FontWeight::SEMIBOLD)
            .map(|d| {
                if chosen {
                    d.bg(rgb(if interactive { p.primary } else { p.disabled }))
                        .text_color(rgb(p.on_primary))
                } else {
                    d.border_1()
                        .border_color(gpui::Hsla::from(rgb(p.muted)).opacity(0.5))
                        .text_color(rgb(p.muted))
                }
            })
            .map(|d| match (multiple, chosen) {
                (true, true) => d.child(Icon::new(IconName::Check).size(px(11.))),
                (true, false) => d,
                (false, _) => d.child(format!("{}", option_ix + 1)),
            });
        chrome::interactive_control(
            div().id(("question-option", ix * 1000 + option_ix)),
            p,
            interactive,
        )
        .when_some(focus.filter(|_| interactive), |d, focus| {
            d.track_focus(focus)
        })
        .debug_selector(move || format!("question-{ix}-option-{option_ix}"))
        .w_full()
        .flex()
        .items_start()
        .gap_2()
        .px(px(if compact { 6. } else { 8. }))
        .py(px(if compact { 3. } else { 5. }))
        .rounded(px(p.control_radius))
        .bg(if chosen {
            gpui::Hsla::from(rgb(p.accent)).opacity(0.16)
        } else {
            rgb(p.base).into()
        })
        .when(interactive && !chosen, |d| {
            d.hover(|s| s.bg(rgb(p.selected)))
        })
        .child(badge)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(1.))
                .child(
                    div()
                        .text_size(px(chrome::scale::BODY))
                        .line_height(gpui::relative(1.4))
                        .font_weight(if chosen {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::MEDIUM
                        })
                        .text_color(rgb(if interactive { p.text } else { p.muted }))
                        .child(label),
                )
                .when(!description.is_empty(), |d| {
                    d.child(
                        div()
                            .text_size(px(chrome::scale::META))
                            .line_height(gpui::relative(1.4))
                            .text_color(rgb(p.muted))
                            .child(description),
                    )
                }),
        )
        .when(interactive, |d| {
            d.on_click(cx.listener(move |this, _, window, cx| {
                this.pick_option(ix, option_ix, multiple, window, cx)
            }))
            .on_key_down(cx.listener(
                move |this, event: &gpui::KeyDownEvent, window, cx| {
                    let stroke = &event.keystroke;
                    if stroke.modifiers.modified() {
                        return;
                    }
                    match stroke.key.as_str() {
                        "down" => window.focus_next(),
                        "up" => window.focus_prev(),
                        key => match key.parse::<usize>() {
                            Ok(n) if (1..=count.min(9)).contains(&n) => {
                                this.pick_option(ix, n - 1, multiple, window, cx)
                            }
                            _ => return,
                        },
                    }
                    cx.stop_propagation();
                },
            ))
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
