//! Secondary views keep the conversation uncluttered and preserve its draft/scroll.
use super::primitives::{self, ButtonSize, ButtonVariant, StatusBadge, StatusKind};
use super::*;
use gpui::AnyElement;
use serde_json::Value;
use wks_native::features::{Request, attention_transition};

const HANDOFF_DESCRIPTION: &str = "Start a new agent in this session’s folder with a brief of its work. The handoff message waits in the new agent’s composer for you to review and send.";
const MODEL_DESCRIPTION: &str = "Choose the model, reasoning effort and access for this session.";
const MODEL_APPLY_NOTE: &str =
    "Applies to the session’s next work. A busy agent may queue the change.";
const HISTORY_DESCRIPTION: &str =
    "A snapshot of retained conversation history. Live messages continue in chat.";
const NAME_DESCRIPTION: &str = "Shown on this device. Leave empty to use the agent’s own title.";
const END_CONFIRMATION: &str =
    "End this agent? Its current work stops. You can resume its conversation later.";
const HISTORY_TRIMMED: &str =
    "The server has trimmed earlier events; this starts at its oldest retained event.";
const SETUP_INFO: &str = "Checking a connection sends a small test request and may use your provider allowance. Git is required for reviewing changes.";

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
    /// Review's commit message; cleared once the commit lands.
    pub commit_message: Entity<InputState>,
    /// Sessions whose OS alert was clicked, sent from the alert's thread.
    pub alert_clicks: async_channel::Sender<String>,
    /// A live access switch was sent and its receipt has not come back.
    pub access_switching: bool,
    /// The approval card's "Deny with note" box, open while it is shown.
    pub deny_note: Entity<InputState>,
    pub deny_open: bool,
    /// The limit card the user closed: (session, window, reset). A different
    /// window or reset shows it again.
    pub limit_dismissed: Option<(String, &'static str, Option<i64>)>,
    _alert_clicks: Task<()>,
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
    /// Re-reads the Hosts view every 10s while it is open; ends itself
    /// (and the `host.*`/`queue.*` watch) once the view closes.
    pub hosts_poll: Option<Task<()>>,
    /// The local limits editor is open.
    pub limits_open: bool,
    /// One input per `hosts::LIMITS` key, in that order.
    pub limit_inputs: Vec<Entity<InputState>>,
    pub limits_error: String,
    /// A `queue.setConfig` is on its way; the editor closes when it lands.
    pub limits_sent: bool,
    /// The `host-action` number when it was sent; only a newer one settles it.
    pub limits_after: u64,
    /// Where the New Agent form launches (`Local` is today's direct spawn).
    pub launch_host: wks_native::hosts::HostChoice,
    /// "Queue if busy" for This machine: submit through the queue instead.
    pub launch_queue: bool,
    // r2-pages
    /// Design v2's New session: which chip's choices are open under the
    /// task box (one at a time; `None` keeps the form short).
    pub launch_panel: Option<super::launch::LaunchPanel>,
    /// The project folder chosen on a peer (from its target's `cwds`).
    pub launch_host_cwd: String,
    /// The user picked `launch_host_cwd` themselves: a later targets read
    /// does not replace it with the resolved folder.
    pub launch_host_cwd_picked: bool,
    /// The newest `queue-targets` read already applied to the form.
    pub targets_seen: u64,
    /// "Link & use" on the form: the `link-action` number before it was
    /// sent, the peer and its folder, selected once the link is saved.
    pub link_use: Option<(u64, String, String)>,
    /// The Hosts → Projects cell whose "Choose…" list is open, by (local
    /// folder, peer).
    pub link_choose: Option<(String, String)>,
    /// A `queue.submit` from the form is on its way.
    pub queue_sent: bool,
    pub queue_receipt: u64,
    /// The job whose Remove was clicked once; the second click removes it.
    pub job_confirm_remove: Option<String>,
    /// The Jobs page shows every job's runs instead of the jobs.
    pub job_runs_tab: bool,
    /// The run history shows failed runs only.
    pub job_failed_only: bool,
    /// The run open in the history, by (job id, start time).
    pub job_run_open: Option<(String, i64)>,
    /// What the Scratch page shows: a folder (the hub resolves a worktree to
    /// its main checkout) or a project as `group:<id>`. Fixed while open.
    pub scratch_project: Option<String>,
    /// A project a badge asked Scratch to open on, used once.
    pub scratch_preset: Option<String>,
    pub scratch_filter: wks_native::scratch::Filter,
    /// The scratch file open in the list, by (folder, relative path).
    pub scratch_open: Option<(String, String)>,
    pub scratch_purpose: Entity<InputState>,
    pub scratch_note: Entity<InputState>,
    /// The project editor's draft (folders live here; the text fields in
    /// the inputs below until Save).
    pub group_draft: Option<wks_native::project_groups::Draft>,
    pub group_confirm_delete: bool,
    pub group_error: String,
    /// A save or delete is on its way; the editor closes when it lands.
    pub group_save_sent: bool,
    pub group_name: Entity<InputState>,
    pub group_icon: Entity<InputState>,
    pub group_color: Entity<InputState>,
    pub group_folder: Entity<InputState>,
    pub confirm_end: Option<String>,
    pub resume: Option<String>,
    pub local_paths: bool,
    pub keep_running: std::rc::Rc<std::cell::Cell<bool>>,
    pub upload_receipt: u64,
    pub attachments: HashMap<String, Vec<(String, String)>>,
    /// Uploaded size by hub path, from each `files.upload` result: decides
    /// whether an image is small enough to send as real image input.
    pub attachment_sizes: HashMap<String, u64>,
    /// The composer's `/` menu: the highlighted row, and the draft the user
    /// dismissed it for with Esc (it stays shut until the draft changes),
    /// each keyed by (session, draft).
    pub slash_cursor: Option<(String, String, usize)>,
    pub slash_dismissed: Option<(String, String)>,
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
    /// The hub's `agents.jobFullAccess` as last read or saved; `None` until
    /// read. Never assumed from a pending toggle.
    pub job_access: Option<bool>,
    pub job_access_receipt: u64,
    /// The hub's `agents.ignoreExternalMcp` as last read or saved; `None`
    /// until read. Never assumed from a pending toggle.
    pub external_mcp: Option<bool>,
    pub external_mcp_receipt: u64,
    /// The hub's `agents.autoTitle` as last read or saved (see
    /// `features::title_settings`); `None` until read.
    pub titles: Option<Value>,
    pub titles_receipt: u64,
    /// The hub's default terminal shell and installable shells as last read
    /// or saved (see `terminal::shell_settings`); `None` until read.
    pub terminal_shell: Option<Value>,
    pub terminal_shell_receipt: u64,
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
    /// The successor is a fresh agent of the source's own provider ("Start
    /// fresh from a summary") rather than the other provider.
    pub handoff_fresh: bool,
    /// A summary handoff asked for a session not yet selected; the page opens
    /// when the selection lands (see `start_summary_handoff`).
    pub summary_pending: Option<String>,
    /// This window asked for a handoff and awaits a receipt newer than
    /// `handoff_seen` (the latest one when it asked).
    pub handoff_sent: bool,
    pub handoff_seen: u64,
    /// Session history's search, groups and keyboard cursor.
    pub recent: super::recent::RecentUi, // area-pages
    /// Design v2 page state: open overflow menus and expanded host cards.
    pub pages: PagesUi,
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
        let (alert_clicks, clicked) = async_channel::unbounded::<String>();
        let _alert_clicks = cx.spawn_in(window, async move |this, cx| {
            while let Ok(id) = clicked.recv().await {
                if this
                    .update_in(cx, |this, window, cx| {
                        this.open_alerted_session(id, window, cx)
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            alert_clicks,
            _alert_clicks,
            access_switching: false,
            deny_note: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Tell the agent what to do instead")
            }),
            deny_open: false,
            limit_dismissed: None,
            name: cx.new(|cx| InputState::new(window, cx).placeholder("Session name")),
            commit_message: cx.new(|cx| {
                InputState::new(window, cx)
                    .auto_grow(1, 4)
                    .placeholder("Commit message (Ctrl+Enter to commit)")
            }),
            return_launch: false,
            approval_details: false,
            selected_options: Vec::new(),
            notice: String::new(),
            dismissed_notices: Vec::new(),
            show_archived: false,
            job_expanded: None,
            hosts_poll: None,
            limits_open: false,
            limit_inputs: wks_native::hosts::LIMITS
                .iter()
                .map(|_| cx.new(|cx| InputState::new(window, cx)))
                .collect(),
            limits_error: String::new(),
            limits_sent: false,
            limits_after: 0,
            launch_host: Default::default(),
            launch_queue: false,
            launch_panel: None,
            launch_host_cwd: String::new(),
            launch_host_cwd_picked: false,
            targets_seen: 0,
            link_use: None,
            link_choose: None,
            queue_sent: false,
            queue_receipt: 0,
            job_confirm_remove: None,
            job_runs_tab: false,
            job_failed_only: false,
            job_run_open: None,
            scratch_project: None,
            scratch_preset: None,
            group_draft: None,
            group_confirm_delete: false,
            group_error: String::new(),
            group_save_sent: false,
            group_name: cx.new(|cx| InputState::new(window, cx).placeholder("Acme")),
            group_icon: cx.new(|cx| InputState::new(window, cx).placeholder("🚀")),
            group_color: cx.new(|cx| InputState::new(window, cx).placeholder("#5b8def")),
            group_folder: cx
                .new(|cx| InputState::new(window, cx).placeholder("/path/to/another/repository")),
            scratch_filter: Default::default(),
            scratch_open: None,
            scratch_purpose: cx
                .new(|cx| InputState::new(window, cx).placeholder("What this file is for")),
            scratch_note: cx.new(|cx| {
                InputState::new(window, cx)
                    .auto_grow(1, 4)
                    .placeholder("e.g. ran on prod 2026-11-01, 4,210 rows")
            }),
            confirm_end: None,
            resume: None,
            local_paths: false,
            keep_running: Default::default(),
            upload_receipt: 0,
            attachments: HashMap::new(),
            attachment_sizes: HashMap::new(),
            slash_cursor: None,
            slash_dismissed: None,
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
            job_access: None,
            job_access_receipt: 0,
            external_mcp: None,
            external_mcp_receipt: 0,
            titles: None,
            titles_receipt: 0,
            terminal_shell: None,
            terminal_shell_receipt: 0,
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
            handoff_fresh: false,
            summary_pending: None,
            handoff_sent: false,
            handoff_seen: 0,
            recent: super::recent::RecentUi::new(window, cx),
            pages: Default::default(),
        }
    }
}
/// An OS alert for one session; clicking it opens that session.
#[cfg_attr(all(test, feature = "ui-tests"), allow(dead_code))]
pub(super) struct Alert {
    pub title: String,
    pub body: String,
    pub session: String,
}

/// OS notifications for sessions that need attention while the window is in
/// the background. Shown off the UI thread, at most five per update. Where
/// the platform reports clicks (XDG on Linux/BSD, Windows toasts), each alert
/// waits on its own thread until it is clicked or closed, and a click opens
/// its session in this window.
#[cfg(not(all(test, feature = "ui-tests")))]
fn post_attention_alerts(
    alerts: Vec<Alert>,
    clicks: async_channel::Sender<String>,
    _: &mut Context<Workspace>,
) {
    for alert in alerts.into_iter().take(5) {
        let clicks = clicks.clone();
        let spawned = std::thread::Builder::new()
            .name("wks-alert".into())
            .spawn(move || {
                let mut notification = notify_rust::Notification::new();
                notification
                    .summary(&alert.title)
                    .body(&alert.body)
                    .appname("Workspacer Native");
                #[cfg(all(unix, not(target_os = "macos")))]
                notification.action("default", "Open");
                #[cfg(target_os = "windows")]
                notification.app_id(if cfg!(feature = "rust-hub") {
                    "Workspacer.Native.RustPreview"
                } else {
                    "Workspacer.Native"
                });
                let handle = match notification.show() {
                    Ok(handle) => handle,
                    Err(error) => {
                        eprintln!("Native notification unavailable: {error}");
                        return;
                    }
                };
                #[cfg(all(unix, not(target_os = "macos")))]
                handle.wait_for_action(|action| {
                    if action == "default" {
                        let _ = clicks.send_blocking(alert.session);
                    }
                });
                #[cfg(target_os = "windows")]
                let _ = handle.wait_for_response(|response: &notify_rust::NotificationResponse| {
                    if matches!(
                        response,
                        notify_rust::NotificationResponse::Default
                            | notify_rust::NotificationResponse::Action(_)
                    ) {
                        let _ = clicks.send_blocking(alert.session);
                    }
                });
                // macOS reports no clicks, so nothing opens the session.
                #[cfg(target_os = "macos")]
                let _ = (handle, clicks, alert.session);
            });
        if let Err(error) = spawned {
            eprintln!("Native notification unavailable: {error}");
        }
    }
}

/// The UI tests record alerts instead of showing them. A real one is
/// platform FFI on the GPUI test thread, which runs "background" tasks: a
/// WinRT toast in the COM apartment each test platform opens and closes (the
/// serial Windows suite died with an access violation starting the second
/// test to raise one, on a fresh thread), D-Bus on the developer's desktop.
#[cfg(all(test, feature = "ui-tests"))]
fn post_attention_alerts(
    alerts: Vec<Alert>,
    _: async_channel::Sender<String>,
    _: &mut Context<Workspace>,
) {
    POSTED_ALERTS.with_borrow_mut(|posted| {
        posted.extend(
            alerts
                .into_iter()
                .take(5)
                .map(|Alert { title, body, .. }| (title, body)),
        )
    });
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
        // area-panels: under design v2, Changes opens as a dock beside the
        // conversation when the window has room.
        if screen == Screen::Changes && self.open_changes_dock(window, cx) {
            return;
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
                self.request(Request::JobRuns, cx);
            }
            Screen::Hosts => self.open_hosts(cx),
            Screen::Scratch => self.open_scratch(cx),
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
            Screen::PullRequest => self.open_pull_request(cx),
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
        self.sync_project_groups(next);
        self.sync_hosts(next, window, cx);
        if self.settings.notifications
            && self.view.connected
            && next.connected
            && !window.is_window_active()
        {
            self.alert_attention(next, cx);
        }
        self.refresh_review_after_turn(next, cx);
        // The deny note belongs to one approval; a new or resolved one closes it.
        let approval_of = |view: &View| {
            view.selected
                .as_ref()
                .and_then(|id| view.sessions.iter().find(|s| &s.id == id))
                .map(|s| (s.approval.clone(), s.card()))
                .filter(|(approval, _)| approval.is_some())
        };
        if approval_of(&self.view) != approval_of(next) {
            self.extras.deny_open = false;
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
        if let Some(state) = next.requests.get("job-access")
            && !state.loading
            && state.number > self.extras.job_access_receipt
        {
            self.extras.job_access_receipt = state.number;
            if state.error.is_none() {
                self.extras.job_access = Some(state.value["jobFullAccess"] == true);
            }
        }
        if let Some(state) = next.requests.get("external-mcp")
            && !state.loading
            && state.number > self.extras.external_mcp_receipt
        {
            self.extras.external_mcp_receipt = state.number;
            if state.error.is_none() {
                self.extras.external_mcp = Some(state.value["ignoreExternalMcp"] == true);
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
        if let Some(state) = next.requests.get("terminal-shell")
            && !state.loading
            && state.number > self.extras.terminal_shell_receipt
        {
            self.extras.terminal_shell_receipt = state.number;
            if state.error.is_none() {
                self.extras.terminal_shell = Some((*state.value).clone());
            }
        }
        self.sync_attachments(next, window, cx);
        self.sync_questions(next, window, cx);
        self.sync_queue(next, window, cx);
        self.sync_async_questions(next);
        if self.view.selected != next.selected {
            self.extras.confirm_end = None;
            if matches!(
                self.screen,
                Screen::Changes
                    | Screen::History
                    | Screen::Session
                    | Screen::Model
                    | Screen::Handoff
                    | Screen::PullRequest
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
                Some(Alert {
                    title: title.to_owned(),
                    body: self.session_title(s),
                    session: s.id.clone(),
                })
            })
            .collect();
        if !alerts.is_empty() {
            post_attention_alerts(alerts, self.extras.alert_clicks.clone(), cx);
        }
    }

    /// A clicked alert: bring this window forward on its session. A window
    /// pinned to another session only comes forward.
    pub(super) fn open_alerted_session(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.activate_window();
        if self
            .requested_session
            .as_ref()
            .is_some_and(|pinned| pinned != &id)
            || !self.view.sessions.iter().any(|s| s.id == id)
        {
            return;
        }
        self.project_filter = None;
        self.show_screen(Screen::Conversation, window, cx);
        self.command(Command::Select(id), cx);
    }
    /// The request's loading or error line, toned; empty when settled.
    pub(super) fn feature_message(&self, key: &str) -> Div {
        let p = self.palette();
        match self.view.requests.get(key) {
            Some(s) if s.loading => chrome::notice_line("Loading…", chrome::Tone::Loading, p),
            Some(s) => match s.error.as_ref().filter(|e| !e.is_empty()) {
                Some(error) => chrome::notice_line(error.clone(), chrome::Tone::Error, p),
                None => div(),
            },
            None => div(),
        }
    }

    pub(super) fn render_feature(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let short = window.viewport_size().height < px(620.);
        let session_name = self.selected_session().map(|s| self.session_title(s));
        let handoff_title = self.handoff_title();
        let (title, description): (&str, Option<SharedString>) = match self.screen {
            // r3-pages (R3.5): no "connection".
            Screen::Recent if self.design().is_v2() => (
                "Session history",
                Some("Every session you have run, ended ones included.".into()),
            ),
            Screen::Recent => (
                "Session history",
                Some("Every session this connection knows about, including ended ones.".into()),
            ),
            // r2-pages: no "hub" in v2's default copy.
            Screen::Jobs if self.design().is_v2() => (
                "Jobs",
                Some("Work that runs on a schedule, even while this window is closed.".into()),
            ),
            Screen::Jobs => ("Jobs", Some(super::jobs::JOBS_DESCRIPTION.into())),
            // r2-pages: v2 says "Machines".
            Screen::Hosts if self.design().is_v2() => {
                ("Machines", Some(super::hosts::HOSTS_DESCRIPTION.into()))
            }
            Screen::Hosts => ("Hosts", Some(super::hosts::HOSTS_DESCRIPTION.into())),
            // r3-pages (R3.5): named as the palette names it, and explained.
            Screen::Scratch if self.design().is_v2() => (
                "Scratch files",
                Some(super::scratch::SCRATCH_DESCRIPTION_V2.into()),
            ),
            Screen::Scratch => ("Scratch", Some(super::scratch::SCRATCH_DESCRIPTION.into())),
            Screen::History => ("Conversation history", Some(HISTORY_DESCRIPTION.into())),
            // The session itself is the hero; its facts follow underneath.
            Screen::Session => (session_name.as_deref().unwrap_or("Session details"), None),
            Screen::Setup => (
                "Agent setup",
                Some("Connect your agents on the machine running this workspace.".into()),
            ),
            Screen::Handoff => (handoff_title.as_str(), Some(HANDOFF_DESCRIPTION.into())),
            Screen::PullRequest => (
                "Pull request",
                Some(super::pull_request::DESCRIPTION.into()),
            ),
            Screen::Model => ("Model, effort and access", Some(MODEL_DESCRIPTION.into())),
            _ => ("", None),
        };
        let trailing = match self.screen {
            Screen::Recent => Some(
                self.pquiet(
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
                self.pquiet(
                    "jobs-refresh",
                    "Refresh",
                    IconName::Redo,
                    self.view.connected,
                )
                .when(self.view.connected, |d| {
                    d.on_click(cx.listener(|this, _, _, cx| {
                        this.request(Request::Jobs, cx);
                        this.request(Request::JobRuns, cx);
                    }))
                })
                .into_any_element(),
            ),
            Screen::Hosts => Some(
                self.pquiet(
                    "hosts-refresh",
                    "Refresh",
                    IconName::Redo,
                    self.view.connected,
                )
                .debug_selector(|| "hosts-refresh".into())
                .when(self.view.connected, |d| {
                    d.on_click(cx.listener(|this, _, _, cx| this.request(Request::Hosts, cx)))
                })
                .into_any_element(),
            ),
            Screen::Scratch => Some(
                self.pquiet(
                    "scratch-refresh",
                    "Refresh",
                    IconName::Redo,
                    self.view.connected,
                )
                .when(self.view.connected, |d| {
                    d.on_click(cx.listener(|this, _, _, cx| this.refresh_scratch(cx)))
                })
                .into_any_element(),
            ),
            Screen::Setup => {
                let can_check = self.view.connected
                    && !self.view.requests.get("setup").is_some_and(|s| s.loading);
                Some(
                    self.pquiet("setup-refresh", "Recheck", IconName::Redo, can_check)
                        .debug_selector(|| "setup-refresh".into())
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
                        })
                        .into_any_element(),
                )
            }
            Screen::Session => self
                .selected_session()
                .map(|s| self.session_status_pill(s).into_any_element()),
            Screen::Handoff => self.handoff_continue(cx).map(IntoElement::into_any_element),
            _ => None,
        };
        let body = match self.screen {
            Screen::Jobs => self.render_jobs(cx),
            Screen::Hosts => self.render_hosts(cx),
            Screen::Scratch => self.render_scratch(window, cx),
            Screen::History => self.render_history(window, cx),
            Screen::Session => self.render_session(window.viewport_size().width < px(900.), cx),
            Screen::Setup => self.render_setup(cx),
            Screen::Model => self.render_model(cx),
            Screen::Handoff => self.render_handoff(cx),
            Screen::PullRequest => self.render_pull_request(window, cx),
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
            chrome::notice_line(self.extras.notice.clone(), tone, p)
                .debug_selector(|| "feature-notice".into())
        });
        if self.design().is_v2() {
            // Every feature page shares one frame under v2: the shared page
            // header (back link, title, description, actions) in the shared
            // page width (`docs/design-v2.md`, "Migrating a screen").
            return self.feature_page_v2(title.to_owned(), description, notice, body, cx);
        }
        let header = self.page_header(
            Some(back),
            (self.screen == Screen::Session && session_name.is_some()).then_some("SESSION DETAILS"),
            title.to_owned(),
            description,
            trailing,
            short,
        );
        let header = match self.selected_session() {
            Some(s) if self.screen == Screen::Session => div()
                .flex()
                .flex_col()
                .gap_2()
                .child(header)
                .child(self.session_summary_line(s)),
            _ => header,
        };
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
                names.insert(id, name.clone());
            }
            self.save_settings(cx);
            self.extras.notice = "Name saved".into();
            // The hub's own name too (every client, and a Codex thread),
            // where the hub offers it; the device name above stays either way.
            if !name.is_empty()
                && self.view.connected
                && self.view.may_call("agents.rename")
                && self.selected_session().is_some_and(|s| s.label != name)
            {
                self.act(Action::Rename(name), cx);
            }
        }
        cx.notify();
    }

    /// The session's status as a tinted pill, beside its name in the header.
    fn session_status_pill(&self, s: &Session) -> Div {
        let p = self.palette();
        let (status, color) = if self.view.connected {
            self.status_of(s, p)
        } else {
            ("Offline", p.muted)
        };
        div()
            .debug_selector(|| "session-status".into())
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(10.))
            .py_1()
            .rounded_full()
            .bg(gpui::Hsla::from(rgb(color)).opacity(0.12))
            .text_size(px(chrome::scale::META))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(color))
            .child(status_dot(color))
            .child(status.to_owned())
    }

    /// Under the session's name: its agent, effort and project at a glance.
    fn session_summary_line(&self, s: &Session) -> Div {
        let p = self.palette();
        let sep = || div().text_color(rgb(p.disabled)).child("·");
        let effort = (!s.effort.is_empty())
            .then(|| format!("{} effort", wks_native::launch::effort_label(&s.effort)));
        div()
            .debug_selector(|| "session-summary".into())
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .min_w_0()
            .text_size(px(chrome::scale::META))
            .text_color(rgb(p.muted))
            .child(self.page_model(s, 12.).font_weight(FontWeight::MEDIUM))
            .when_some(effort, |d, effort| d.child(sep()).child(effort))
            .when_some(s.context_window, |d, tokens| {
                d.child(sep())
                    .child(format!("{} context", super::launch::context_label(tokens)))
            })
            .child(sep())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .min_w_0()
                    .child(Icon::new(IconName::Folder).size(px(12.)))
                    .child(
                        div()
                            .truncate()
                            .child(chrome::project_label(&s.cwd).to_owned()),
                    ),
            )
    }

    /// `narrow`: labels sit above their values instead of in a column.
    fn render_session(&self, narrow: bool, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
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
        // One rhythm for every fact: a fixed label column, the value, and an
        // optional action at the row's end. Rows share a height and divider.
        let fact = |label: &'static str, value: AnyElement, action: Option<AnyElement>| {
            let value = div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap_4()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(chrome::scale::BODY))
                        .child(value),
                )
                .children(action.map(|a| div().flex_shrink_0().child(a)));
            div()
                .flex()
                .min_w_0()
                .min_h(px(44.))
                .border_t_1()
                .border_color(rgb(p.border))
                .map(|d| {
                    if narrow {
                        d.flex_col().gap_1().py_2()
                    } else {
                        d.items_center().gap_4().py_1()
                    }
                })
                .child(
                    div()
                        .when(!narrow, |d| d.w(px(112.)).h(px(32.)).flex().items_center())
                        .flex_shrink_0()
                        .text_size(px(chrome::scale::META))
                        .text_color(rgb(p.muted))
                        .child(label),
                )
                .child(value.when(narrow, |d| d.w_full()))
        };
        let name = fact(
            "Name",
            div()
                .flex()
                .flex_col()
                .gap_1()
                .pb_2()
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
                            self.pbutton("save-session-name", "Save", true)
                                .debug_selector(|| "save-session-name".into())
                                .flex_shrink_0()
                                .py(px(6.))
                                .on_click(cx.listener(|this, _, _, cx| this.save_session_name(cx))),
                        ),
                )
                .child(
                    div()
                        .text_size(px(chrome::scale::CAPTION))
                        .text_color(rgb(p.muted))
                        .child(NAME_DESCRIPTION),
                )
                .into_any_element(),
            None,
        )
        // The first row needs no divider under the card's heading, and its
        // label lines up with the field rather than the field and its hint.
        .border_t_0()
        .items_start()
        .when(!narrow, |d| d.pt_2());
        let model_enabled = self.supported_session() && self.view.connected;
        let agent_detail = [
            (!s.effort.is_empty())
                .then(|| format!("{} effort", wks_native::launch::effort_label(&s.effort))),
            s.context_window
                .map(|t| format!("{} context", super::launch::context_label(t))),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        let details = self
            .page_card()
            .debug_selector(|| "session-details-card".into())
            .px_4()
            .pt_4()
            .pb_1()
            .flex()
            .flex_col()
            .child(card_heading("Details", None, p).pb_2())
            .child(name)
            .child(fact(
                "Agent",
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .child(self.page_model(s, 12.).flex_shrink_0())
                    .when(!agent_detail.is_empty(), |d| {
                        d.child(
                            div()
                                .truncate()
                                .text_size(px(chrome::scale::META))
                                .text_color(rgb(p.muted))
                                .child(agent_detail.clone()),
                        )
                    })
                    .into_any_element(),
                model_enabled.then(|| {
                    self.pquiet("session-change-model", "Change…", IconName::Settings, true)
                        .debug_selector(|| "session-change-model".into())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_feature(Screen::Model, window, cx)
                        }))
                        .into_any_element()
                }),
            ))
            .child(fact(
                "Folder",
                div()
                    .truncate()
                    .font_family(mono_font())
                    .text_size(px(chrome::scale::META))
                    .child(s.cwd.clone())
                    .into_any_element(),
                None,
            ))
            .when(
                self.view.may_call("pr.threads") || self.view.may_call("pr.forBranch"),
                |d| {
                    let linked = s.pull_request.is_some();
                    let text = match &s.pull_request {
                        Some(link) => format!(
                            "{} {}{}",
                            link.provider_label(),
                            link.short(),
                            if link.title.is_empty() {
                                String::new()
                            } else {
                                format!(" · {}", link.title)
                            }
                        ),
                        None => "Not linked".to_owned(),
                    };
                    d.child(fact(
                        "Pull request",
                        div()
                            .truncate()
                            .when(!linked, |d| d.text_color(rgb(p.muted)))
                            .child(text)
                            .into_any_element(),
                        Some(
                            self.quiet_button(
                                "session-pull-request",
                                if linked { "Open" } else { "Link…" },
                                super::pull_request::icon(),
                                true,
                            )
                            .debug_selector(|| "session-pull-request".into())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_feature(Screen::PullRequest, window, cx)
                            }))
                            .into_any_element(),
                        ),
                    ))
                },
            );

        // Carry the work on: resume a stopped session, or hand it to a new
        // agent. Resuming is the obvious next step for a stopped session, so
        // it alone is the page's primary action.
        let mut carry_on: Vec<AnyElement> = Vec::new();
        if s.stopped() && self.supported_session() {
            carry_on.push(
                self.pprimary("resume-session", "Resume session…", !busy)
                    .debug_selector(|| "resume-session".into())
                    .when(!busy, |d| {
                        d.on_click(cx.listener(move |this, _, window, cx| {
                            this.resume_session(&resume, window, cx)
                        }))
                    })
                    .into_any_element(),
            );
        }
        if let Some(target) = wks_native::handoff::target(s)
            .ok()
            .filter(|_| self.view.connected)
        {
            carry_on.push(
                self.pbutton(
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
                })
                .into_any_element(),
            );
        }
        // A same-provider successor from a cheap summary: for a session too
        // long or too cold to resume cheaply.
        if wks_native::handoff::targets(s).is_ok() && self.view.connected {
            let fresh = s.clone();
            carry_on.push(
                self.pbutton("fresh-session", "Start fresh from a summary…", !busy)
                    .debug_selector(|| "fresh-session".into())
                    .when(!busy, |d| {
                        d.on_click(cx.listener(move |this, _, window, cx| {
                            this.start_summary_handoff(&fresh, window, cx)
                        }))
                    })
                    .into_any_element(),
            );
        }
        let carry_note = if s.stopped() {
            "Resume picks this conversation up again; a handoff starts a new agent with a brief of it."
        } else {
            "A handoff starts a new agent with a brief of this work; this one keeps running."
        };
        // Managing the row itself: archive (reversible) and end (destructive),
        // each explained on its own line rather than in one footnote.
        let manage_row = |title: &'static str, text: &'static str, control: AnyElement| {
            action_row(title, text, Some(control), p)
        };
        let has_carry_on = !carry_on.is_empty();
        let actions = self
            .page_card()
            .debug_selector(|| "session-actions-card".into())
            .px_4()
            .pt_4()
            .pb_1()
            .flex()
            .flex_col()
            .child(card_heading("Actions", None, p).pb_3())
            .when(has_carry_on, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .pb_3()
                        .child(div().flex().flex_wrap().gap_2().children(carry_on))
                        .child(
                            div()
                                .text_size(px(chrome::scale::META))
                                .text_color(rgb(p.muted))
                                .child(carry_note),
                        ),
                )
            })
            .child(
                manage_row(
                    if archived { "Archived" } else { "Archive" },
                    match (archived, s.stopped(), self.design().is_v2()) {
                        // r3-pages (R3.5): no "hub".
                        (true, _, true) => "Hidden from the sidebar everywhere you use Workspacer.",
                        (false, true, true) => {
                            "Hides it from the sidebar everywhere you use Workspacer."
                        }
                        (false, false, true) => {
                            "Hides it from the sidebar everywhere you use Workspacer. It keeps running."
                        }
                        (true, _, false) => "Hidden from the list in every client of this hub.",
                        (false, true, false) => {
                            "Hides it from the list in every client of this hub."
                        }
                        (false, false, false) => {
                            "Hides it from the list in every client of this hub. It keeps running."
                        }
                    },
                    self.pbutton(
                        "archive-session",
                        if archived {
                            "Restore from archive"
                        } else {
                            "Archive"
                        },
                        true,
                    )
                    .debug_selector(|| "archive-session".into())
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_archive(&archive, cx)))
                    .into_any_element(),
                )
                .when(!has_carry_on, |d| d.border_t_0()),
            )
            .when(!s.stopped(), |d| {
                d.child(manage_row(
                    "End session",
                    "Stops the agent and its current work. You can resume the conversation later.",
                    if confirming {
                        div().into_any_element()
                    } else {
                        self.pdanger("end-session", "End session…", !busy)
                            .debug_selector(|| "end-session".into())
                            .when(!busy, |d| {
                                d.on_click(cx.listener(move |this, _, _, cx| {
                                    this.extras.confirm_end = Some(end.clone());
                                    cx.notify();
                                }))
                            })
                            .into_any_element()
                    },
                ))
            })
            .when(confirming, |d| {
                d.child(
                    div()
                        .debug_selector(|| "confirm-end-panel".into())
                        .mb_3()
                        .p_3()
                        .rounded(px(p.control_radius))
                        .bg(gpui::Hsla::from(rgb(p.error)).opacity(0.08))
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(div().flex_1().min_w(px(220.)).child(chrome::notice_line(
                            END_CONFIRMATION,
                            chrome::Tone::Warning,
                            p,
                        )))
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(
                                    self.pbutton("cancel-end", "Keep running", true)
                                        .debug_selector(|| "cancel-end".into())
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.extras.confirm_end = None;
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    self.pdanger("confirm-end", "End session", !busy)
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
            });
        let notice = self.view.notice.clone();
        let context = self.render_context_card(s, cx);
        let mcp = self.render_mcp_card(s, cx);
        let codex = self.render_codex_card(s, cx);
        div()
            .flex()
            .flex_col()
            .gap_4()
            .when(!notice.is_empty(), |d| {
                d.child(chrome::notice_line(
                    notice.clone(),
                    chrome::notice_tone(&notice),
                    p,
                ))
            })
            .child(details)
            .child(actions)
            .children(context)
            .children(codex)
            .children(mcp)
    }
    fn render_history(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
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
                        self.pquiet("older-history", "Previous", IconName::ChevronLeft, page > 0)
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
                        self.pquiet(
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
                |d| d.child(chrome::notice_line(HISTORY_TRIMMED, chrome::Tone::Info, p)),
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
        let p = self.palette();
        // Recheck lives in the page header, beside the title it refreshes.
        div()
            .flex()
            .flex_col()
            .gap_4()
            .children(
                ["claude", "codex"]
                    .into_iter()
                    .map(|provider| self.render_setup_card(provider, cx)),
            )
            .child(
                div()
                    .px_1()
                    .child(chrome::notice_line(SETUP_INFO, chrome::Tone::Info, p)),
            )
    }

    /// One provider: whether it is installed, and its connection check.
    fn render_setup_card(&self, provider: &'static str, cx: &mut Context<Self>) -> Div {
        if self.design().is_v2() {
            return self.render_setup_card_v2(provider, cx);
        }
        let p = self.palette();
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
        } else if found == Some(false) {
            p.warning
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
            .when_some(self.v2_palette(), |d, v2| {
                // v2: the shared status vocabulary, no pill.
                d.child(
                    div()
                        .debug_selector(move || format!("setup-installed-{provider}"))
                        .flex_shrink_0()
                        .child({
                            // r2-systems: the one setup vocabulary and source.
                            let setup = self.agent_setup(provider);
                            let nothing_ready = ["claude", "codex"].iter().all(|id| {
                                self.agent_setup(id) != super::states::AgentSetup::Installed
                            });
                            StatusBadge::new(setup.kind(nothing_ready))
                                .label(setup.label())
                                .render(&v2)
                        }),
                )
            })
            .when(!self.design().is_v2(), |d| {
                d.child(
                    div()
                        .debug_selector(move || format!("setup-installed-{provider}"))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .px(px(10.))
                        .py_1()
                        .rounded_full()
                        .bg(gpui::Hsla::from(rgb(installed)).opacity(0.12))
                        .text_size(px(chrome::scale::CAPTION))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(installed))
                        .child(status_dot(installed))
                        .child(match found {
                            Some(true) => "Installed",
                            Some(false) => "Not found",
                            None => "Not checked",
                        }),
                )
            });
        // How to install and sign in, with the terminal command set apart.
        let (install, command) = if claude {
            ("Install Claude Code, then sign in by running", "claude")
        } else {
            ("Install Codex CLI, then sign in by running", "codex login")
        };
        let instructions = div()
            .pl(px(52.))
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x_1()
            .gap_y_1()
            .text_size(px(chrome::scale::META))
            .text_color(rgb(p.muted))
            .child(install)
            .child(
                div()
                    .px(px(6.))
                    .py(px(1.))
                    .rounded(px(4.))
                    .bg(chrome::tonal(p, 0.07))
                    .font_family(mono_font())
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.text))
                    .child(command),
            )
            .child("in a terminal.");
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
                        brand_spinner(12., p)
                    } else {
                        status_dot(color)
                    })
                    .child(description),
            )
            .child(
                self.pbutton(
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
        self.page_card()
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(identity)
                    .child(instructions),
            )
            .child(check)
            .when(current && !busy, |d| {
                d.when_some(state.and_then(|s| s.error.as_ref()), |d, error| {
                    d.child(chrome::notice_line(error.clone(), chrome::Tone::Error, p))
                })
                .when_some(value["readinessError"].as_str(), |d, error| {
                    d.child(chrome::notice_line(
                        error.to_owned(),
                        chrome::Tone::Warning,
                        p,
                    ))
                })
            })
    }

    // r2-pages: R2.3 one state set (Ready / Needs sign-in / Not installed),
    // copy matched to the state, and the command to run, copyable.
    fn render_setup_card_v2(&self, provider: &'static str, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let t = p.tokens;
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
        let readiness = if current && !busy {
            value["readiness"]["state"].as_str().unwrap_or("unchecked")
        } else {
            "unchecked"
        };
        let setup = SetupState::of(found, readiness);
        let (install, sign_in) = if claude {
            ("npm install -g @anthropic-ai/claude-code", "claude")
        } else {
            ("npm install -g @openai/codex", "codex login")
        };
        let name = if claude { "Claude" } else { "Codex" };
        let (sentence, command): (String, Option<&'static str>) = match setup {
            SetupState::NotInstalled => (
                format!(
                    "{name} isn't installed on this machine. Run this in a terminal, then check again:"
                ),
                Some(install),
            ),
            SetupState::NeedsSignIn => (
                format!("{name} is installed. Sign in by running this in a terminal:"),
                Some(sign_in),
            ),
            SetupState::Ready if readiness == "responding" => (
                format!("{name} is installed, signed in and answering."),
                None,
            ),
            SetupState::Ready => (
                format!("{name} is installed. Check the connection to confirm you're signed in."),
                None,
            ),
            SetupState::Problem(why) => (why.to_owned(), None),
            SetupState::Unknown => ("Looking for it on this machine…".to_owned(), None),
        };
        let badge = if checking {
            StatusBadge::new(StatusKind::Working).label("Checking…")
        } else {
            setup.badge()
        };
        let copy_id = SharedString::from(format!("setup-copy-{provider}"));
        let inset = command.map(|command| {
            div()
                .debug_selector(move || format!("setup-command-{provider}"))
                .flex()
                .items_center()
                .gap(px(t.space.md))
                .pl(px(t.space.lg))
                .pr(px(t.space.xs))
                .py(px(t.space.xs))
                .rounded(px(t.radius.control))
                .bg(primitives::tonal(&p, t.wash.rest))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(mono_font())
                        .text_size(px(t.text.mono))
                        .text_color(rgb(p.text))
                        .child(format!("$ {command}")),
                )
                .child(
                    primitives::button(
                        copy_id.clone(),
                        "Copy",
                        Some(IconName::Copy),
                        ButtonVariant::Tertiary,
                        ButtonSize::Small,
                        true,
                        &p,
                    )
                    .debug_selector(move || format!("setup-copy-{provider}"))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(command.to_owned()))
                    })),
                )
        });
        let check = primitives::button(
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
            None,
            ButtonVariant::Secondary,
            ButtonSize::Small,
            can_check && setup != SetupState::NotInstalled,
            &p,
        )
        .debug_selector(move || format!("setup-{provider}"))
        .when(can_check && setup != SetupState::NotInstalled, |d| {
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
        });
        primitives::card(&p)
            .debug_selector(move || format!("setup-card-{provider}"))
            .gap(px(t.space.lg))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(t.space.lg))
                    .child(chrome::provider_mark(provider, t.layout.avatar_lg, p))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(t.space.xxs))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(t.space.md))
                                    .child(
                                        div()
                                            .text_size(px(t.text.subheading))
                                            .font_weight(FontWeight(t.weight.semibold))
                                            .text_color(rgb(p.text))
                                            .child(name),
                                    )
                                    .child(
                                        div()
                                            .debug_selector(move || {
                                                format!("setup-installed-{provider}")
                                            })
                                            .child(badge.render(&p)),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(t.text.body))
                                    .line_height(gpui::relative(t.text.line_height))
                                    .text_color(rgb(p.roles.text_secondary))
                                    .child(sentence),
                            ),
                    )
                    // Nothing to check until it is installed.
                    .when(setup != SetupState::NotInstalled, |d| d.child(check)),
            )
            .children(inset.map(|inset| div().pl(px(t.layout.avatar_lg + t.space.lg)).child(inset)))
            .when(current && !busy, |d| {
                d.when_some(state.and_then(|s| s.error.as_ref()), |d, error| {
                    d.child(chrome::notice_line(error.clone(), chrome::Tone::Error, p))
                })
                .when_some(value["readinessError"].as_str(), |d, error| {
                    d.child(chrome::notice_line(
                        error.to_owned(),
                        chrome::Tone::Warning,
                        p,
                    ))
                })
            })
    }

    fn render_model(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        if !self.supported_session() {
            return empty_note(
                "Model switching is available for Claude and Codex sessions.",
                p,
            );
        }
        let busy = self.view.busy || !self.view.connected;
        let notice = self.view.notice.clone();
        // Same label treatment as the context window's, so the second row's
        // controls line up.
        let label = |text: &'static str| projects::section_label(text, p);
        let effort = div()
            .w(px(220.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(label("Effort"))
            .child(
                div()
                    .debug_selector(|| "launch-effort-picker".into())
                    .child(Select::new(&self.effort_picker).disabled(busy)),
            );
        // A model with a single context window offers no choice: say which
        // it is rather than draw a one-option switch.
        let only_window = self
            .catalog_model(&self.model_choice)
            .and_then(|m| match m.windows.as_slice() {
                [only] => Some(*only),
                _ => None,
            })
            .filter(|only| self.context_window == Some(*only));
        let context = match only_window {
            Some(tokens) => div()
                .debug_selector(|| "model-context-fixed".into())
                .flex()
                .flex_col()
                .gap_2()
                .child(label("Context window"))
                .child(
                    div()
                        .h(px(32.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(chrome::scale::BODY))
                        .child(format!("{} tokens", super::launch::context_label(tokens)))
                        .child(
                            div()
                                .text_size(px(chrome::scale::META))
                                .text_color(rgb(p.muted))
                                .child("· the only window this model offers"),
                        ),
                ),
            None => self.render_context_choice(busy, cx),
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            .when(!notice.is_empty(), |d| {
                d.child(chrome::notice_line(
                    notice.clone(),
                    chrome::notice_tone(&notice),
                    p,
                ))
            })
            .child(
                self.page_card()
                    .debug_selector(|| "model-card".into())
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_4()
                    // The model (its Refresh belongs to its catalog, so it
                    // sits on the model's own label row), then the settings
                    // that ride with it side by side.
                    .child(self.render_model_select(busy, false, cx))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_start()
                            .gap_x_6()
                            .gap_y_4()
                            .child(effort)
                            .child(div().min_w(px(180.)).child(context)),
                    )
                    .child(
                        div()
                            .pt_3()
                            .border_t_1()
                            .border_color(rgb(p.border))
                            .flex()
                            .items_center()
                            .gap_4()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(chrome::scale::META))
                                    .text_color(rgb(p.muted))
                                    .child(MODEL_APPLY_NOTE),
                            )
                            .child(
                                self.pprimary("apply-model", "Apply", !busy)
                                    .debug_selector(|| "apply-model".into())
                                    .flex_shrink_0()
                                    .when(!busy, |d| {
                                        d.on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.apply_model_change(cx)
                                            }),
                                        )
                                    }),
                            ),
                    ),
            )
            .child(self.render_live_access(busy, cx))
    }

    /// The running session's access mode, switched live on one click: the
    /// daemon drives and verifies it, and refuses (with its reason) what can
    /// only be chosen at launch, such as Claude full access for a session
    /// started with approvals on.
    fn render_live_access(&self, busy: bool, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let Some(session) = self.selected_session() else {
            return div();
        };
        let provider = session.provider_id().to_owned();
        let current = Permission::from_wire(&provider, session.access_mode());
        let v2 = self.design().is_v2();
        // R3.5 (v2): the effective mode is always shown selected, as in the
        // model popover; until the agent reports one, that is "Ask first".
        let shown = if v2 {
            super::model_popover::effective_access(current, Permission::choices(&provider))
        } else {
            current
        };
        let switching = self.view.busy && self.extras.access_switching;
        // One segmented control, the current mode chosen. The chosen mode is
        // not clickable, so it is never sent again.
        let choices = div()
            .debug_selector(|| "live-access".into())
            .flex()
            .flex_wrap()
            .gap_1()
            .p(px(3.))
            .rounded(px(18.))
            .bg(rgb(p.base))
            .border_1()
            .border_color(rgb(p.border))
            .children(
                Permission::choices(&provider)
                    .iter()
                    .copied()
                    .map(|permission| {
                        let id = SharedString::from(format!("live-access-{}", permission.label()));
                        let active = shown == Some(permission);
                        // Unreported (v2), the shown default can still be set.
                        let enabled = !busy && (!active || (v2 && current.is_none()));
                        let provider = provider.clone();
                        chrome::interactive_control(div().id(id.clone()), p, enabled)
                            .debug_selector(move || id.to_string())
                            .h(px(28.))
                            .px_3()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .text_size(px(chrome::scale::META))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(match (active, busy) {
                                (true, _) if permission == Permission::FullAccess => p.warning,
                                (true, _) => p.text,
                                (false, true) => p.disabled,
                                (false, false) => p.muted,
                            }))
                            .when(active, |d| d.bg(rgb(p.selected)))
                            .child(if v2 {
                                super::model_popover::access_word(permission)
                            } else {
                                permission.label()
                            })
                            .when(enabled, |d| {
                                d.hover(move |s| s.text_color(rgb(p.text)))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let Ok(mode) = permission.wire(&provider) else {
                                            return;
                                        };
                                        this.extras.access_switching = true;
                                        this.act(
                                            Action::SetPermission {
                                                mode: mode.to_owned(),
                                                label: permission.label().to_owned(),
                                            },
                                            cx,
                                        );
                                    }))
                            })
                    }),
            );
        let (status, tone) = match (switching, current) {
            (true, _) => ("Switching…".to_owned(), chrome::Tone::Loading),
            (false, Some(current)) => (
                format!(
                    "{} Changes apply at once; nothing restarts.",
                    current.description()
                ),
                if current == Permission::FullAccess {
                    chrome::Tone::Warning
                } else {
                    chrome::Tone::Info
                },
            ),
            (false, None) if v2 && session.permission_mode.is_empty() => (
                "Not reported yet, so it asks first, the agents' default. Choosing one sets it."
                    .to_owned(),
                chrome::Tone::Info,
            ),
            (false, None) if session.permission_mode.is_empty() => (
                "The provider has not reported this session’s mode yet. Choosing one sets it."
                    .to_owned(),
                chrome::Tone::Info,
            ),
            (false, None) => (
                format!(
                    "Running in “{}”, which this client does not offer.",
                    session.permission_mode
                ),
                chrome::Tone::Warning,
            ),
        };
        self.page_card()
            .debug_selector(|| "access-card".into())
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .h(px(24.))
                    .flex()
                    .items_center()
                    .child(projects::section_label("Access", p)),
            )
            .child(div().flex().child(choices))
            .child(div().pt_1().child(chrome::notice_line(status, tone, p)))
            // Codex plan mode rides beside approvals (`collaborationMode`),
            // offered only once the session reports it is available.
            .when(session.plan_mode_available(), |d| {
                let plan = session.in_plan_mode();
                let posture = match session.permission_mode.as_str() {
                    "yolo" => "yolo",
                    _ => "ask",
                }
                .to_owned();
                let enabled = !busy;
                d.child(
                    action_row(
                        "Plan mode",
                        if plan {
                            "Codex plans before it acts in its next turns; approvals stay as set above."
                        } else {
                            "Codex can plan before it acts in its next turns, keeping the approvals above."
                        },
                        Some(
                            self.pbutton(
                                "live-plan-mode",
                                if plan { "Leave plan mode" } else { "Plan mode" },
                                enabled,
                            )
                            .debug_selector(|| "live-plan-mode".into())
                            .when(plan, |d| d.bg(rgb(p.selected)).text_color(rgb(p.accent)))
                            .when(enabled, |d| {
                                d.on_click(cx.listener(move |this, _, _, cx| {
                                    this.act(
                                        Action::SetPlanMode {
                                            plan: !plan,
                                            posture: posture.clone(),
                                        },
                                        cx,
                                    )
                                }))
                            })
                            .into_any_element(),
                        ),
                        p,
                    )
                    .mt_2()
                    .pb_0(),
                )
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
        if matches!(receipt.action, Action::SetPermission { .. }) {
            self.extras.access_switching = false;
        }
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

/// One divided row of a page card: a title with its explanation, and the
/// control that acts on it at the row's end (Session details' actions and
/// tools). The caller drops the divider on a card's first row.
pub(super) fn action_row(
    title: &'static str,
    text: impl Into<SharedString>,
    control: Option<AnyElement>,
    p: Palette,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap_4()
        .min_w_0()
        .py_3()
        .border_t_1()
        .border_color(rgb(p.border))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(chrome::scale::BODY))
                        .font_weight(FontWeight::MEDIUM)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(px(chrome::scale::META))
                        .text_color(rgb(p.muted))
                        .child(text.into()),
                ),
        )
        .children(control.map(|c| div().flex_shrink_0().child(c)))
}

/// Heading and optional description at the top of a page card.
pub(super) fn card_heading(
    title: &'static str,
    description: Option<&'static str>,
    p: Palette,
) -> Div {
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

// area-pages: the shared v2 page frame and the small helpers every migrated
// page uses (`docs/design-v2.md`, "Migrating a screen").
impl Workspace {
    /// The palette when design v2 is on; `None` keeps a call site on v1.
    pub(super) fn v2_palette(&self) -> Option<Palette> {
        self.design().is_v2().then(|| self.palette())
    }

    /// The card surface for the design in use: `chrome::card` under v1; the
    /// v2 card's surface, edge and lift (no padding or layout, which the
    /// caller sets as it did for v1) under v2.
    pub(super) fn page_card(&self) -> Div {
        match self.v2_palette() {
            Some(p) => {
                let t = p.tokens;
                div()
                    .rounded(px(t.radius.panel))
                    .bg(rgb(p.surface))
                    .border(px(t.stroke.hairline))
                    .border_color(rgb(p.roles.card_border))
                    .shadow(primitives::elevation(&p, t.elevation.raised))
            }
            None => chrome::card(self.palette()),
        }
    }

    /// Every feature page under v2: back link, title, one-line description
    /// and right-aligned actions over the body, in the shared page width.
    pub(super) fn feature_page_v2(
        &mut self,
        title: String,
        description: Option<SharedString>,
        notice: Option<Div>,
        body: Div,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let back = primitives::back_link(
            "feature-back",
            if self.extras.return_launch {
                "Back to new session"
            } else {
                "Back to chat"
            },
            &p,
        )
        .debug_selector(|| "feature-back".into())
        .on_click(cx.listener(|this, _, window, cx| this.back_from_feature(window, cx)));
        let actions = self.feature_actions_v2(cx);
        let header = primitives::page_header(
            Some(back.into_any_element()),
            title,
            description,
            actions,
            &p,
        )
        .debug_selector(|| "page-header".into());
        let header = match self.selected_session() {
            Some(s) if self.screen == Screen::Session => div()
                .flex()
                .flex_col()
                .gap(px(t.space.md))
                .child(header)
                .child(self.session_summary_line(s)),
            _ => header,
        };
        // r3-pages: forms keep a readable measure, left-aligned under the
        // shared header, while tables use the page's width.
        let body = match self.screen {
            Screen::Model | Screen::Session | Screen::Handoff | Screen::PullRequest => {
                div().w_full().max_w(px(t.pages().form)).child(body)
            }
            _ => body,
        };
        if self.screen == Screen::Recent {
            // Its rows are the page's own children, so the keyboard cursor
            // can scroll one into view.
            let mut items = vec![div().pb(px(t.space.xl)).child(header).into_any_element()];
            items.extend(notice.map(|n| div().pb(px(t.space.md)).child(n).into_any_element()));
            items.extend(self.render_recent(cx));
            return self.page_list(
                "feature-view",
                t.pages().wide,
                false,
                &self.extras.recent.scroll,
                items,
            );
        }
        self.page_view(
            "feature-view",
            t.pages().wide,
            false,
            div()
                .flex()
                .flex_col()
                .gap(px(t.space.xxl))
                .child(header)
                .children(notice)
                .child(body),
        )
    }

    /// The page header's actions under v2: at most one primary, the rest
    /// secondary or ghost.
    fn feature_actions_v2(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = self.palette();
        let connected = self.view.connected;
        let refresh = |id: &'static str| {
            primitives::button(
                id,
                "Refresh",
                Some(IconName::Redo),
                ButtonVariant::Tertiary,
                ButtonSize::Small,
                connected,
                &p,
            )
            .debug_selector(move || id.into())
        };
        match self.screen {
            Screen::Recent => vec![
                refresh("history-refresh")
                    .when(connected, |d| {
                        d.on_click(cx.listener(|this, _, _, cx| this.request(Request::Recent, cx)))
                    })
                    .into_any_element(),
            ],
            Screen::Jobs => vec![
                refresh("jobs-refresh")
                    .when(connected, |d| {
                        d.on_click(cx.listener(|this, _, _, cx| {
                            this.request(Request::Jobs, cx);
                            this.request(Request::JobRuns, cx);
                        }))
                    })
                    .into_any_element(),
                self.new_job_button(cx).into_any_element(),
            ],
            // r2-pages: the page's one Refresh re-reads the project links too.
            Screen::Hosts => vec![
                refresh("hosts-refresh")
                    .when(connected, |d| {
                        d.on_click(cx.listener(|this, _, _, cx| {
                            this.request(Request::Hosts, cx);
                            this.request(Request::ProjectLinks, cx);
                        }))
                    })
                    .into_any_element(),
            ],
            Screen::Scratch => vec![
                refresh("scratch-refresh")
                    .when(connected, |d| {
                        d.on_click(cx.listener(|this, _, _, cx| this.refresh_scratch(cx)))
                    })
                    .into_any_element(),
            ],
            Screen::Setup => {
                let can_check =
                    connected && !self.view.requests.get("setup").is_some_and(|s| s.loading);
                vec![
                    // r2-pages: one style for every page's refresh.
                    primitives::button(
                        "setup-refresh",
                        "Check again",
                        Some(IconName::Redo),
                        ButtonVariant::Tertiary,
                        ButtonSize::Small,
                        can_check,
                        &p,
                    )
                    .debug_selector(|| "setup-refresh".into())
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
                    })
                    .into_any_element(),
                ]
            }
            Screen::Session => self
                .selected_session()
                .map(|s| {
                    let badge = self.session_status_v2(s);
                    div()
                        .debug_selector(|| "session-status".into())
                        .child(badge.render(&p))
                        .into_any_element()
                })
                .into_iter()
                .collect(),
            Screen::Handoff => self
                .handoff_continue(cx)
                .map(IntoElement::into_any_element)
                .into_iter()
                .collect(),
            _ => Vec::new(),
        }
    }
}

// r2-pages
/// One agent's setup, in the one state set Setup, the welcome and the
/// sidebar share: Ready / Needs sign-in / Not installed (plus a failed
/// check, and "not read yet").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SetupState {
    Ready,
    NeedsSignIn,
    NotInstalled,
    /// Installed, but the connection check failed for another reason.
    Problem(&'static str),
    Unknown,
}

impl SetupState {
    /// From `setup.installed[].found` and the last readiness check.
    pub(super) fn of(found: Option<bool>, readiness: &str) -> Self {
        match (found, readiness) {
            (Some(false), _) => Self::NotInstalled,
            (_, "unauthenticated") => Self::NeedsSignIn,
            (_, "limited") => {
                Self::Problem("Signed in, but the account has reached its usage limit.")
            }
            (_, "timeout") => Self::Problem("The connection check timed out. Try it again."),
            (_, "network-error") => {
                Self::Problem("The network is unavailable, so the check couldn't run.")
            }
            (Some(true), "responding" | "unchecked" | "unsupported") => Self::Ready,
            (Some(true), _) => Self::Problem("The connection check failed. Try it again."),
            (None, _) => Self::Unknown,
        }
    }

    pub(super) fn badge(self) -> StatusBadge {
        match self {
            Self::Ready => StatusBadge::new(StatusKind::Done).label("Ready"),
            Self::NeedsSignIn => StatusBadge::new(StatusKind::NeedsYou).label("Needs sign-in"),
            Self::NotInstalled => StatusBadge::new(StatusKind::Idle).label("Not installed"),
            Self::Problem(_) => StatusBadge::new(StatusKind::Failed).label("Check failed"),
            Self::Unknown => StatusBadge::new(StatusKind::Idle).label("Not checked"),
        }
    }
}

/// Design v2 page state (area-pages).
#[derive(Default)]
pub(super) struct PagesUi {
    /// The overflow ("…") menu open on a page, by its key.
    pub menu: Option<String>,
    /// Hosts whose Details (metrics and queue) are open, by host.
    pub hosts_open: std::collections::BTreeSet<String>,
}

impl Workspace {
    /// A "More actions" trigger with its menu below it while open. Each item
    /// is a [`primitives::menu_item`] whose click handler does its work and
    /// calls [`Self::close_page_menu`]; a click outside closes it too.
    pub(super) fn page_overflow(
        &self,
        key: impl Into<String>,
        items: Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let key: String = key.into();
        let trigger_id = SharedString::from(format!("{key}-more"));
        let selector = trigger_id.clone();
        let trigger = primitives::overflow_trigger(trigger_id, &p)
            .debug_selector(move || selector.to_string());
        self.page_menu(key, trigger, items, cx)
    }

    // r3-pages
    /// Any page control that opens a menu below it (the "…" trigger, the
    /// Projects page's "Add…"): the trigger toggles `pages.menu` under
    /// `key`; a click outside closes it.
    pub(super) fn page_menu(
        &self,
        key: impl Into<String>,
        trigger: Stateful<Div>,
        items: Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let key: String = key.into();
        let open = self.extras.pages.menu.as_deref() == Some(key.as_str());
        let toggle = key.clone();
        let trigger = trigger.on_click(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            let menu = &mut this.extras.pages.menu;
            *menu = if menu.as_deref() == Some(toggle.as_str()) {
                None
            } else {
                Some(toggle.clone())
            };
            cx.notify();
        }));
        let menu_selector = format!("{key}-menu");
        div()
            .relative()
            .flex_shrink_0()
            .child(trigger)
            .when(open, |d| {
                d.child(
                    gpui::deferred(
                        div()
                            .absolute()
                            .top(px(t.layout.icon_button + t.space.xs))
                            .right_0()
                            .child(
                                primitives::menu(&p)
                                    .debug_selector(move || menu_selector.clone())
                                    .on_mouse_down_out(
                                        cx.listener(|this, _, _, cx| this.close_page_menu(cx)),
                                    )
                                    .children(items),
                            ),
                    )
                    .with_priority(1),
                )
            })
    }

    pub(super) fn close_page_menu(&mut self, cx: &mut Context<Self>) {
        if self.extras.pages.menu.take().is_some() {
            cx.notify();
        }
    }
}

// area-pages: design-aware buttons for the migrated pages. Under v1 each one
// is exactly the shipped control; under v2 the shared primitive in the
// matching variant (one primary per region, destructive as ghost-danger).
impl Workspace {
    /// `self.button` / a v2 secondary button.
    pub(super) fn pbutton(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        enabled: bool,
    ) -> Stateful<Div> {
        let id: SharedString = id.into();
        match self.v2_palette() {
            Some(p) => primitives::button(
                id,
                label,
                None,
                ButtonVariant::Secondary,
                ButtonSize::Small,
                enabled,
                &p,
            ),
            None => self.button(id, label, enabled),
        }
    }

    /// `self.primary_button` / a v2 primary button.
    pub(super) fn pprimary(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        enabled: bool,
    ) -> Stateful<Div> {
        let id: SharedString = id.into();
        match self.v2_palette() {
            Some(p) => primitives::button(
                id,
                label,
                None,
                ButtonVariant::Primary,
                ButtonSize::Small,
                enabled,
                &p,
            ),
            None => self.primary_button(id, label, enabled),
        }
    }

    /// `self.danger_button` / a v2 ghost-danger button.
    pub(super) fn pdanger(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        enabled: bool,
    ) -> Stateful<Div> {
        let id: SharedString = id.into();
        match self.v2_palette() {
            Some(p) => primitives::button(
                id,
                label,
                None,
                ButtonVariant::Destructive,
                ButtonSize::Small,
                enabled,
                &p,
            ),
            None => self.danger_button(id, label, enabled),
        }
    }

    /// `self.quiet_button` / a v2 ghost button with its icon.
    pub(super) fn pquiet(
        &self,
        id: impl Into<SharedString>,
        label: &'static str,
        icon: IconName,
        enabled: bool,
    ) -> Stateful<Div> {
        let id: SharedString = id.into();
        match self.v2_palette() {
            Some(p) => primitives::button(
                id,
                label,
                Some(icon),
                ButtonVariant::Tertiary,
                ButtonSize::Small,
                enabled,
                &p,
            ),
            None => self.quiet_button(id, label, icon, enabled),
        }
    }

    /// `self.icon_button` / a v2 ghost icon button with a tooltip.
    pub(super) fn picon(
        &self,
        id: impl Into<SharedString>,
        label: &'static str,
        icon: IconName,
        enabled: bool,
    ) -> Stateful<Div> {
        let id: SharedString = id.into();
        match self.v2_palette() {
            Some(p) => {
                primitives::icon_button(id, icon, label, ButtonVariant::Tertiary, enabled, &p)
            }
            None => self.icon_button(id, label, icon, enabled),
        }
    }
}

// area-pages: neutral provider identity on the pages (v2 keeps brand colour
// for the composer's model chip only).
impl Workspace {
    /// A session's provider mark and model: `chrome::model_badge` under v1;
    /// tertiary grey under v2.
    pub(super) fn page_model(&self, s: &Session, size: f32) -> Div {
        let p = self.palette();
        let Some(v2) = self.v2_palette() else {
            return chrome::model_badge(s, p, size);
        };
        let t = v2.tokens;
        let glyph = match s.provider.as_str() {
            "claude" => Some("brand/claude.svg"),
            "codex" => Some("brand/openai.svg"),
            _ => None,
        };
        div()
            .flex()
            .items_center()
            .gap(px(t.space.xs))
            .text_color(rgb(v2.roles.provider))
            .when_some(glyph, |d, path| {
                d.child(
                    gpui::svg()
                        .path(path)
                        .size(px(t.icon.xs))
                        .flex_shrink_0()
                        .text_color(rgb(v2.roles.provider)),
                )
            })
            .child(s.display_model())
    }

    /// A session's status as the shared v2 badge: the one derivation
    /// (r4-truth), read from the live session when the fleet still holds
    /// it (a History row is the hub's stored copy).
    pub(super) fn session_status_v2(&self, s: &Session) -> StatusBadge {
        let live = self
            .view
            .sessions
            .iter()
            .find(|l| l.id == s.id)
            .unwrap_or(s);
        self.session_status(live, timing::now_ms()).badge()
    }
}
