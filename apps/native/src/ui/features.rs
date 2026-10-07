//! Secondary views keep the conversation uncluttered and preserve its draft/scroll.
use super::*;
use gpui::AnyElement;
use serde_json::Value;
use wks_native::features::{Request, attention_transition};

const HANDOFF_DESCRIPTION: &str = "Start a new agent in this session’s folder with a brief of its work. The handoff message waits in the new agent’s composer for you to review and send.";
const MODEL_DESCRIPTION: &str = "Choose the model and reasoning effort for this session’s next work. A busy provider may queue the change.";
const HISTORY_DESCRIPTION: &str =
    "A snapshot of retained conversation history. Live messages continue in chat.";
const NAME_DESCRIPTION: &str =
    "Shown in the sidebar on this device. Leave empty to use the agent’s own title.";
const END_CONFIRMATION: &str =
    "End this agent? Its current work stops. You can resume its conversation later.";
const SESSION_FOOTER: &str = "Archiving hides the session from the list in every client of this hub and keeps it running. Ending stops the agent.";
const HISTORY_TRIMMED: &str =
    "The server has trimmed earlier events; this starts at its oldest retained event.";
const SETUP_INFO: &str = "Checking a connection sends a small test request and may use your provider allowance. Git is required for reviewing changes.";
const CLAUDE_SETUP: &str = "Install Claude Code, then run claude in a terminal to sign in.";
const CODEX_SETUP: &str = "Install Codex CLI, then run codex login in a terminal to sign in.";

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
    /// The Jobs row showing its spec and runs.
    pub job_expanded: Option<String>,
    /// The job whose Remove was clicked once; the second click removes it.
    pub job_confirm_remove: Option<String>,
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
    /// One write at a time per session; later clicks update the desired value.
    pub archive_inflight: std::collections::BTreeMap<String, bool>,
    pub archive_receipt: u64,
    /// Device-only archives already offered to the hub this run.
    pub archive_migrating: std::collections::BTreeSet<String>,
    /// Who writes the brief on Continue with…
    pub handoff_brief: wks_native::handoff::Brief,
    /// This window asked for a handoff and awaits a receipt newer than
    /// `handoff_seen` (the latest one when it asked).
    pub handoff_sent: bool,
    pub handoff_seen: u64,
    /// Session history's search, groups and keyboard cursor.
    pub recent: super::recent::RecentUi,
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
            job_expanded: None,
            job_confirm_remove: None,
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
            archive_inflight: Default::default(),
            archive_receipt: 0,
            archive_migrating: Default::default(),
            handoff_brief: Default::default(),
            handoff_sent: false,
            handoff_seen: 0,
            recent: super::recent::RecentUi::new(window, cx),
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
            Screen::Recent => {
                self.request(Request::Recent, cx);
                self.enter_recent(window, cx);
            }
            Screen::Jobs => {
                self.extras.job_confirm_remove = None;
                self.request(Request::Jobs, cx);
            }
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
            Screen::Handoff => self.open_handoff(window, cx),
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
            self.alert_attention(next, cx);
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
        self.sync_attachments(next, window, cx);
        self.sync_questions(next, window, cx);
        if self.view.selected != next.selected {
            self.extras.confirm_end = None;
            if matches!(
                self.screen,
                Screen::Changes
                    | Screen::History
                    | Screen::Session
                    | Screen::Model
                    | Screen::Handoff
            ) {
                self.screen = Screen::Conversation;
            }
        }
    }
    /// OS notifications for sessions that newly need the user, while this
    /// window is in the background.
    fn alert_attention(&self, next: &View, cx: &mut Context<Self>) {
        let alerts: Vec<_> = next
            .sessions
            .iter()
            .filter_map(|s| {
                let old = self.view.sessions.iter().find(|old| old.id == s.id)?;
                let title = attention_transition(old, s)?;
                Some((title.to_owned(), self.session_title(s)))
            })
            .collect();
        if !alerts.is_empty() {
            post_attention_alerts(alerts, cx);
        }
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
        let handoff_title = self.handoff_title();
        let (title, description): (&str, Option<SharedString>) = match self.screen {
            Screen::Recent => (
                "Session history",
                Some("Every session this connection knows about, including ended ones.".into()),
            ),
            Screen::Jobs => ("Jobs", Some(super::jobs::JOBS_DESCRIPTION.into())),
            Screen::History => ("Conversation history", Some(HISTORY_DESCRIPTION.into())),
            Screen::Session => ("Session details", session_cwd.map(Into::into)),
            Screen::Setup => (
                "Agent setup",
                Some("Connect your agents on the machine running this workspace.".into()),
            ),
            Screen::Handoff => (handoff_title.as_str(), Some(HANDOFF_DESCRIPTION.into())),
            Screen::Model => ("Model and effort", Some(MODEL_DESCRIPTION.into())),
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
            Screen::Jobs => Some(
                self.quiet_button(
                    "jobs-refresh",
                    "Refresh",
                    IconName::Redo,
                    self.view.connected,
                )
                .when(self.view.connected, |d| {
                    d.on_click(cx.listener(|this, _, _, cx| this.request(Request::Jobs, cx)))
                })
                .into_any_element(),
            ),
            Screen::Handoff => self.handoff_continue(cx).map(IntoElement::into_any_element),
            _ => None,
        };
        let body = match self.screen {
            Screen::Jobs => self.render_jobs(cx),
            Screen::History => self.render_history(window, cx),
            Screen::Session => self.render_session(cx),
            Screen::Setup => self.render_setup(cx),
            Screen::Model => self.render_model(cx),
            Screen::Handoff => self.render_handoff(cx),
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
        let header = self.page_header(
            Some(back),
            None,
            title.to_owned(),
            description,
            trailing,
            short,
        );
        if self.screen == Screen::Recent {
            // Its rows are the page's own children, so the keyboard cursor
            // can scroll one into view.
            let mut items = vec![header.pb_2().into_any_element()];
            items.extend(notice.map(|n| n.mb_2().into_any_element()));
            items.extend(self.render_recent(cx));
            return self.page_list(
                "feature-view",
                CHAT_WIDTH,
                short,
                &self.extras.recent.scroll,
                items,
            );
        }
        self.page_view(
            "feature-view",
            CHAT_WIDTH,
            short,
            div()
                .flex()
                .flex_col()
                .gap_5()
                .child(header)
                .children(notice)
                .child(body),
        )
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
            .child(card_heading("Name", Some(NAME_DESCRIPTION), p))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.extras.name)),
                    )
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
                    .child(
                        self.button(
                            "archive-session",
                            if archived {
                                "Restore from archive"
                            } else {
                                "Archive"
                            },
                            true,
                        )
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.toggle_archive(&archive, cx)),
                        ),
                    )
                    .when(s.stopped() && self.supported_session(), |d| {
                        d.child(
                            self.button("resume-session", "Resume session…", !busy)
                                .when(!busy, |d| {
                                    d.on_click(cx.listener(move |this, _, window, cx| {
                                        this.resume_session(&resume, window, cx)
                                    }))
                                }),
                        )
                    })
                    .when_some(
                        wks_native::handoff::target(s)
                            .ok()
                            .filter(|_| self.view.connected),
                        |d, target| {
                            d.child(
                                self.button(
                                    "handoff-session",
                                    format!(
                                        "Continue with {}…",
                                        wks_native::handoff::provider_name(target)
                                    ),
                                    !busy,
                                )
                                .debug_selector(|| "handoff-session".into())
                                .when(!busy, |d| {
                                    d.on_click(cx.listener(|this, _, window, cx| {
                                        this.open_feature(Screen::Handoff, window, cx)
                                    }))
                                }),
                            )
                        },
                    )
                    .when(!s.stopped() && !confirming, |d| {
                        d.child(
                            self.danger_button("end-session", "End session…", !busy)
                                .debug_selector(|| "end-session".into())
                                .when(!busy, |d| {
                                    d.on_click(cx.listener(move |this, _, _, cx| {
                                        this.extras.confirm_end = Some(end.clone());
                                        cx.notify();
                                    }))
                                }),
                        )
                    }),
            )
            .when(confirming, |d| {
                d.child(
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
                        .child(chrome::notice_line(
                            END_CONFIRMATION,
                            chrome::Tone::Warning,
                            p,
                            "confirm-end-copy",
                        ))
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(
                                    self.button("cancel-end", "Keep running", true)
                                        .debug_selector(|| "cancel-end".into())
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.extras.confirm_end = None;
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    self.danger_button("confirm-end", "End session", !busy)
                                        .debug_selector(|| "confirm-end".into())
                                        .when(!busy, |d| {
                                            d.on_click(cx.listener(|this, _, _, cx| {
                                                this.act(Action::Terminate, cx);
                                                this.extras.confirm_end = None;
                                            }))
                                        }),
                                ),
                        ),
                )
            })
            .child(
                div()
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.muted))
                    .child(SESSION_FOOTER),
            );
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
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.feature_message("history"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        self.quiet_button(
                            "older-history",
                            "Previous",
                            IconName::ChevronLeft,
                            page > 0,
                        )
                        .when(page > 0, |d| {
                            d.on_click(cx.listener(|this, _, _, cx| {
                                this.extras.history_page =
                                    this.extras.history_page.saturating_sub(1);
                                this.extras
                                    .history_scroll
                                    .set_offset(gpui::point(px(0.), px(0.)));
                                cx.notify();
                            }))
                        }),
                    )
                    .child(
                        div()
                            .text_size(px(chrome::scale::META))
                            .text_color(rgb(p.muted))
                            .child(format!("Page {} of {}", page + 1, pages)),
                    )
                    .child(
                        self.quiet_button(
                            "newer-history",
                            "Next",
                            IconName::ChevronRight,
                            page + 1 < pages,
                        )
                        .when(page + 1 < pages, |d| {
                            d.on_click(cx.listener(|this, _, _, cx| {
                                this.extras.history_page += 1;
                                this.extras
                                    .history_scroll
                                    .set_offset(gpui::point(px(0.), px(0.)));
                                cx.notify();
                            }))
                        }),
                    ),
            )
            .when(
                state.is_some_and(|s| s.value["first_seq"].as_u64().unwrap_or(0) > 1),
                |d| {
                    d.child(chrome::notice_line(
                        HISTORY_TRIMMED,
                        chrome::Tone::Info,
                        p,
                        "history-trimmed",
                    ))
                },
            )
            .when(
                count == 0 && state.is_some_and(|s| !s.loading && s.error.is_none()),
                |d| {
                    d.child(empty_note(
                        "No retained messages are available for this session.",
                        p,
                    ))
                },
            )
            .child(
                div()
                    .id("history-content")
                    .max_h(px(600.))
                    .overflow_y_scroll()
                    .track_scroll(&self.extras.history_scroll)
                    .children(items.into_iter().flatten().skip(start).take(50).filter_map(
                        |value| {
                            let row =
                                serde_json::from_value::<wks_native::model::Row>(value.clone())
                                    .ok()?;
                            Some(
                                div()
                                    .when(value["continued"] == true, |d| {
                                        d.child(overline(
                                            format!(
                                                "Long message · part {} · literal text",
                                                value["part"]
                                            ),
                                            p,
                                        ))
                                    })
                                    .child(self.render_message(
                                        &row,
                                        "history",
                                        value["continued"] == true,
                                        window,
                                        cx,
                                    )),
                            )
                        },
                    )),
            )
    }
    fn render_setup(&self, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let busy = self.view.requests.get("setup").is_some_and(|s| s.loading);
        let can_check = !busy && self.view.connected;
        div()
            .flex()
            .flex_col()
            .gap_4()
            .children(
                ["claude", "codex"]
                    .into_iter()
                    .map(|provider| self.render_setup_card(provider, cx)),
            )
            .child(chrome::notice_line(
                SETUP_INFO,
                chrome::Tone::Info,
                p,
                "setup-info",
            ))
            .child(
                div().flex().child(
                    self.quiet_button(
                        "setup-refresh",
                        "Recheck installed agents",
                        IconName::Redo,
                        can_check,
                    )
                    .when(can_check, |d| {
                        d.on_click(cx.listener(|this, _, _, cx| {
                            let provider = this.provider.into();
                            this.request(
                                Request::Setup {
                                    provider,
                                    check: false,
                                },
                                cx,
                            )
                        }))
                    }),
                ),
            )
    }

    /// One provider: whether it is installed, and its connection check.
    fn render_setup_card(&self, provider: &'static str, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let state = self.view.requests.get("setup");
        let busy = state.is_some_and(|s| s.loading);
        let can_check = !busy && self.view.connected;
        let value = state.map(|s| s.value.as_ref()).unwrap_or(&Value::Null);
        let claude = provider == "claude";
        let found = value["installed"]
            .as_array()
            .and_then(|rows| rows.iter().find(|r| r["provider"] == provider))
            .and_then(|r| r["found"].as_bool());
        let current = state.is_some_and(|s| {
            matches!(&s.request, Request::Setup { provider: checked, .. } if checked == provider)
        });
        let checking = busy && current;
        let status = if current && !busy {
            value["readiness"]["state"].as_str().unwrap_or("unchecked")
        } else {
            "unchecked"
        };
        let color = match status {
            "responding" => p.success,
            "unchecked" | "unsupported" => p.muted,
            _ => p.warning,
        };
        let description = if checking {
            "Checking this agent…"
        } else {
            match status {
                "responding" => "Ready · the agent responded",
                "unauthenticated" => "Sign-in required",
                "limited" => "Account limit reached",
                "timeout" => "The connection check timed out",
                "network-error" => "Network unavailable",
                "unchecked" => "Connection not verified",
                "unsupported" => "Connection check unavailable on this host",
                _ => "Connection check failed",
            }
        };
        let installed = if found == Some(true) {
            p.success
        } else {
            p.muted
        };
        let identity = div()
            .flex()
            .items_center()
            .gap_3()
            .child(chrome::provider_mark(provider, 40., p))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(chrome::scale::HEADING))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(if claude { "Claude" } else { "Codex" }),
                    )
                    .child(
                        div()
                            .text_size(px(chrome::scale::CAPTION))
                            .text_color(rgb(p.muted))
                            .child(if claude { "Claude Code" } else { "Codex CLI" }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(installed))
                    .child(status_dot(installed))
                    .child(match found {
                        Some(true) => "Installed",
                        Some(false) => "Not found",
                        None => "Not checked",
                    }),
            );
        let check = div()
            .pt_3()
            .border_t_1()
            .border_color(rgb(p.border))
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(chrome::scale::META))
                    .text_color(rgb(color))
                    .child(if checking {
                        let id = SharedString::from(format!("setup-{provider}-activity"));
                        brand_spinner(12., p, id)
                    } else {
                        status_dot(color)
                    })
                    .child(description),
            )
            .child(
                self.button(
                    if claude {
                        "setup-claude"
                    } else {
                        "setup-codex"
                    },
                    if checking {
                        "Checking…"
                    } else {
                        "Check connection"
                    },
                    can_check,
                )
                .when(can_check, |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| {
                        let provider = provider.into();
                        this.request(
                            Request::Setup {
                                provider,
                                check: true,
                            },
                            cx,
                        )
                    }))
                }),
            );
        chrome::card(p)
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(identity)
            .child(
                div()
                    .text_size(px(chrome::scale::META))
                    .text_color(rgb(p.muted))
                    .child(if claude { CLAUDE_SETUP } else { CODEX_SETUP }),
            )
            .child(check)
            .when(current && !busy, |d| {
                d.when_some(state.and_then(|s| s.error.as_ref()), |d, error| {
                    d.child(chrome::notice_line(
                        error.clone(),
                        chrome::Tone::Error,
                        p,
                        SharedString::from(format!("setup-{provider}-error")),
                    ))
                })
                .when_some(value["readinessError"].as_str(), |d, error| {
                    d.child(chrome::notice_line(
                        error.to_owned(),
                        chrome::Tone::Warning,
                        p,
                        SharedString::from(format!("setup-{provider}-readiness")),
                    ))
                })
            })
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
}

/// Muted explanatory line for an empty list or a missing selection.
pub(super) fn empty_note(text: &'static str, p: Palette) -> Div {
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
