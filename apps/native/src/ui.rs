use gpui::{
    App, ClipboardItem, Context, Div, Entity, FocusHandle, KeyBinding, ListAlignment, ListOffset,
    ListScrollEvent, ListState, Render, SharedString, Stateful, Task, Window, actions, div, list,
    prelude::*, px, rgb, uniform_list,
};
use gpui_component::{
    input::{Input, InputState},
    text::TextView,
};
use std::{collections::HashMap, sync::Arc};
use wks_native::controller::{Action, Command, Controller, NewSession, View};

// Native equivalents of the desktop semantic tokens. No CSS/browser runtime.
const BASE: u32 = 0x14171c;
const SURFACE: u32 = 0x1b2028;
const SELECTED: u32 = 0x293448;
const TEXT: u32 = 0xe2e6ed;
const MUTED: u32 = 0x919cab;
const ACCENT: u32 = 0x8cb4ff;
const WARNING: u32 = 0xe4bd78;

actions!(
    native,
    [
        SendMessage,
        CreateSession,
        NextSession,
        PreviousSession,
        FocusComposer,
        Refresh,
        Quit
    ]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-n", CreateSession, Some("Workspace")),
        KeyBinding::new("cmd-n", CreateSession, Some("Workspace")),
        KeyBinding::new("ctrl-enter", SendMessage, Some("Workspace")),
        KeyBinding::new("cmd-enter", SendMessage, Some("Workspace")),
        // Input owns its own Enter bindings, so match the focused editor's
        // context as well; a root-only binding loses to its newline action.
        KeyBinding::new("ctrl-enter", SendMessage, Some("Workspace > Input")),
        KeyBinding::new("cmd-enter", SendMessage, Some("Workspace > Input")),
        KeyBinding::new("alt-down", NextSession, Some("Workspace")),
        KeyBinding::new("alt-up", PreviousSession, Some("Workspace")),
        KeyBinding::new("ctrl-l", FocusComposer, Some("Workspace")),
        KeyBinding::new("cmd-l", FocusComposer, Some("Workspace")),
        KeyBinding::new("ctrl-r", Refresh, Some("Workspace")),
        KeyBinding::new("cmd-r", Refresh, Some("Workspace")),
        KeyBinding::new("cmd-q", Quit, None),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
}

pub struct Workspace {
    controller: Controller,
    view: Arc<View>,
    composer: Entity<InputState>,
    new_session: bool,
    provider: &'static str,
    project: Entity<InputState>,
    label: Entity<InputState>,
    model: Entity<InputState>,
    prompt: Entity<InputState>,
    spawn_pending: bool,
    last_spawn_receipt: u64,
    spawn_error: String,
    focus: FocusHandle,
    list: ListState,
    drafts: HashMap<String, String>,
    last_receipt: u64,
    local_notice: String,
    follow: bool,
    demo: bool,
    requested_session: Option<String>,
    selection_requested: bool,
    _updates: Task<()>,
    #[cfg(feature = "ui-tests")]
    rendered_rows: std::rc::Rc<std::cell::RefCell<std::collections::BTreeSet<usize>>>,
}

impl Workspace {
    pub fn new(
        controller: Controller,
        demo: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let composer = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .rows(3)
                .placeholder("Message this session…")
        });
        let project =
            cx.new(|cx| InputState::new(window, cx).placeholder("Absolute project directory"));
        let label = cx.new(|cx| InputState::new(window, cx).placeholder("Optional session name"));
        let model = cx.new(|cx| InputState::new(window, cx).placeholder("Provider default"));
        let prompt = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .rows(3)
                .placeholder("What would you like to work on? (optional)")
        });
        let incoming = controller.views.clone();
        let updates = cx.spawn_in(window, async move |this, cx| {
            while let Ok(view) = incoming.recv().await {
                if this
                    .update_in(cx, |this, window, cx| this.update_view(view, window, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let list = ListState::new(0, ListAlignment::Bottom, px(250.));
        list.set_scroll_handler(cx.listener(|this, event: &ListScrollEvent, _, cx| {
            this.follow = !event.is_scrolled;
            cx.notify();
        }));
        let focus = cx.focus_handle();
        window.focus(&focus);
        Self {
            controller,
            view: Arc::new(View::default()),
            composer,
            new_session: false,
            provider: "claude",
            project,
            label,
            model,
            prompt,
            spawn_pending: false,
            last_spawn_receipt: 0,
            spawn_error: String::new(),
            focus,
            list,
            drafts: HashMap::new(),
            last_receipt: 0,
            local_notice: String::new(),
            follow: true,
            demo,
            requested_session: None,
            selection_requested: false,
            _updates: updates,
            #[cfg(feature = "ui-tests")]
            rendered_rows: Default::default(),
        }
    }

    pub fn open_session(&mut self, session: Option<String>) {
        self.requested_session = session;
    }

    fn update_view(&mut self, view: Arc<View>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(receipt) = &view.spawn_receipt
            && receipt.number > self.last_spawn_receipt
        {
            self.last_spawn_receipt = receipt.number;
            self.spawn_pending = false;
            self.spawn_error = receipt.error.clone().unwrap_or_default();
            if let Some(id) = &receipt.session {
                self.new_session = false;
                if let Some(message) = &receipt.unsent_message {
                    self.drafts.insert(id.clone(), message.clone());
                    if self.view.selected.as_ref() == Some(id) {
                        self.composer
                            .update(cx, |input, cx| input.set_value(message.clone(), window, cx));
                    }
                }
                self.prompt
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.composer
                    .update(cx, |input, cx| input.focus(window, cx));
            }
        }
        if let Some(id) = &self.requested_session {
            if view.selected.as_ref() == Some(id) {
                self.selection_requested = false;
            } else {
                let available = view.sessions.iter().any(|session| &session.id == id);
                if !available {
                    self.selection_requested = false;
                }
                if !self.selection_requested && available {
                    self.selection_requested =
                        self.controller.command(Command::Select(id.clone())).is_ok();
                }
                // A requested target must never silently turn into a different
                // session, especially when an external harness drives input.
                self.local_notice = if view.connected {
                    if available {
                        format!("Opening requested session {id}…")
                    } else {
                        format!("Requested session {id} is unavailable; controls are disabled.")
                    }
                } else {
                    view.notice.clone()
                };
                self.view = Arc::new(View {
                    connected: false,
                    ..(*self.view).clone()
                });
                cx.notify();
                return;
            }
        }
        self.local_notice.clear();
        if self.view.selected != view.selected {
            if let Some(id) = &self.view.selected {
                self.drafts
                    .insert(id.clone(), self.composer.read(cx).value().to_string());
            }
            let draft = view
                .selected
                .as_ref()
                .and_then(|id| self.drafts.get(id))
                .cloned()
                .unwrap_or_default();
            self.composer
                .update(cx, |input, cx| input.set_value(draft, window, cx));
            self.list.reset(view.transcript.rows.len());
            self.follow = true;
        } else {
            let old = &self.view.transcript.rows;
            let new = &view.transcript.rows;
            let first = old
                .iter()
                .zip(new)
                .position(|(a, b)| !Arc::ptr_eq(a, b))
                .unwrap_or(old.len().min(new.len()));
            // Invalidate only changed measurements. Retain the scroll anchor
            // while reading history; bottom alignment follows when at the tail.
            // Revisions restart after a reconnect/reselection. Row identities
            // and count remain authoritative even if two revisions collide.
            let changed = first < old.len() || first < new.len();
            if changed {
                self.list.splice(first..old.len(), new.len() - first);
            }
            if changed && self.follow {
                self.list.scroll_to(ListOffset {
                    item_ix: new.len(),
                    offset_in_item: px(0.),
                });
            }
        }
        if let Some(receipt) = &view.receipt
            && receipt.number > self.last_receipt
        {
            self.last_receipt = receipt.number;
            if receipt.error.is_none()
                && let Action::Send(sent) | Action::Answer(sent) = &receipt.action
            {
                if view.selected.as_ref() == Some(&receipt.session) {
                    if self.composer.read(cx).value().as_ref() == sent {
                        self.composer
                            .update(cx, |input, cx| input.set_value("", window, cx));
                    }
                } else if self.drafts.get(&receipt.session) == Some(sent) {
                    self.drafts.remove(&receipt.session);
                }
            }
        }
        self.view = view;
        cx.notify();
    }

    fn command(&mut self, command: Command, cx: &mut Context<Self>) {
        if let Command::Select(id) = &command
            && self
                .requested_session
                .as_ref()
                .is_some_and(|target| target != id)
        {
            self.local_notice =
                "This window is pinned by --session. Open another window to browse sessions."
                    .into();
            cx.notify();
            return;
        }
        self.local_notice = self
            .controller
            .command(command)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        cx.notify();
    }

    fn send(&mut self, _: &SendMessage, _: &mut Window, cx: &mut Context<Self>) {
        if self.new_session {
            self.create(cx);
            return;
        }
        let text = self.composer.read(cx).value().to_string();
        if text.trim().is_empty() || self.view.busy || !self.view.connected {
            return;
        }
        self.act(Action::Send(text), cx);
    }

    fn act(&mut self, action: Action, cx: &mut Context<Self>) {
        if let Some(session) = self.view.selected.clone() {
            self.command(Command::Act { session, action }, cx);
        }
    }

    fn show_new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.demo || self.requested_session.is_some() {
            return;
        }
        self.new_session = true;
        if self.project.read(cx).value().is_empty() {
            let cwd = self
                .view
                .sessions
                .iter()
                .find(|s| Some(&s.id) == self.view.selected.as_ref())
                .map(|s| s.cwd.clone())
                .unwrap_or_default();
            self.project
                .update(cx, |input, cx| input.set_value(cwd, window, cx));
        }
        self.project.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn create(&mut self, cx: &mut Context<Self>) {
        if self.demo
            || self.requested_session.is_some()
            || self.spawn_pending
            || self.view.creating
            || !self.view.connected
        {
            return;
        }
        let request = NewSession {
            provider: self.provider.into(),
            cwd: self.project.read(cx).value().to_string(),
            label: self.label.read(cx).value().to_string(),
            model: self.model.read(cx).value().to_string(),
            message: self.prompt.read(cx).value().to_string(),
        };
        self.spawn_error.clear();
        match request
            .params()
            .and_then(|_| self.controller.command(Command::Create(request)))
        {
            Ok(()) => self.spawn_pending = true,
            Err(error) => self.spawn_error = error.to_string(),
        }
        cx.notify();
    }

    fn move_selection(&mut self, step: isize, cx: &mut Context<Self>) {
        let sessions = &self.view.sessions;
        if sessions.is_empty() {
            return;
        }
        let index = sessions
            .iter()
            .position(|s| Some(&s.id) == self.view.selected.as_ref())
            .unwrap_or(0);
        let index = (index as isize + step).rem_euclid(sessions.len() as isize) as usize;
        self.command(Command::Select(sessions[index].id.clone()), cx);
    }

    fn button(&self, id: &'static str, label: &'static str, enabled: bool) -> Stateful<Div> {
        div()
            .id(id)
            .px_3()
            .py_2()
            .rounded_md()
            .bg(rgb(SURFACE))
            .text_color(rgb(if enabled { ACCENT } else { MUTED }))
            .text_size(px(12.))
            .when(enabled, |d| {
                d.cursor_pointer().hover(|s| s.bg(rgb(SELECTED)))
            })
            .child(label)
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self
            .view
            .sessions
            .iter()
            .find(|s| Some(&s.id) == self.view.selected.as_ref())
            .cloned();
        let enabled = self.view.connected
            && !self.view.busy
            && selected.as_ref().is_some_and(|s| !s.stopped());
        let snapshot = self.view.clone();
        let session_key = self.view.selected.clone().unwrap_or_default();
        #[cfg(feature = "ui-tests")]
        let rendered_rows = self.rendered_rows.clone();
        let title = selected
            .as_ref()
            .map(|s| s.title().to_owned())
            .unwrap_or_else(|| "Your sessions".into());
        let notice = if !self.local_notice.is_empty() {
            self.local_notice.clone()
        } else {
            self.view.notice.clone()
        };
        let transcript = list(self.list.clone(), move |ix, window, cx| {
            #[cfg(feature = "ui-tests")]
            rendered_rows.borrow_mut().insert(ix);
            let Some(row) = snapshot.transcript.rows.get(ix) else {
                return div().into_any_element();
            };
            let copy = row.text.clone();
            let content = if matches!(row.role, "Assistant" | "You") {
                row.text.clone()
            } else {
                row.text
                    .lines()
                    .map(|line| format!("    {line}\n"))
                    .collect::<String>()
            };
            div()
                .w_full()
                .px_5()
                .py_3()
                .child(
                    div()
                        .w_full()
                        .max_w(px(900.))
                        .mx_auto()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .flex()
                                .justify_between()
                                .text_size(px(12.))
                                .text_color(rgb(MUTED))
                                .child(row.role)
                                .child(
                                    div()
                                        .id(("copy", row.key))
                                        .cursor_pointer()
                                        .child("Copy")
                                        .on_click(move |_, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                copy.clone(),
                                            ))
                                        }),
                                ),
                        )
                        .when(row.truncated, |d| {
                            d.child(
                                div()
                                    .text_color(rgb(WARNING))
                                    .child("Earlier text in this message was clipped."),
                            )
                        })
                        .child(
                            TextView::markdown(
                                SharedString::from(format!("{session_key}-{}", row.key)),
                                content,
                                window,
                                cx,
                            )
                            .selectable(true),
                        ),
                )
                .into_any_element()
        })
        .flex_1()
        .min_h_0();

        let sidebar = div()
            .w(px(250.))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(SURFACE))
            .flex()
            .flex_col()
            .child(
                div().p_4().text_size(px(16.)).child("Workspacer").child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(MUTED))
                        .child(if self.demo {
                            "NATIVE EXPERIMENT · DEMO"
                        } else {
                            "NATIVE"
                        }),
                ),
            )
            .child(
                self.button(
                    "new-session",
                    "New session",
                    !self.demo && self.requested_session.is_none(),
                )
                .on_click(cx.listener(|this, _, window, cx| this.show_new_session(window, cx))),
            )
            .child(
                uniform_list(
                    "sessions",
                    self.view.sessions.len(),
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|ix| {
                                let session = &this.view.sessions[ix];
                                let id = session.id.clone();
                                let active = this.view.selected.as_ref() == Some(&id);
                                div()
                                    .id(ix)
                                    .h(px(68.))
                                    .px_4()
                                    .py_2()
                                    .cursor_pointer()
                                    .overflow_hidden()
                                    .when(active, |d| d.bg(rgb(SELECTED)))
                                    .hover(|s| s.bg(rgb(SELECTED)))
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .truncate()
                                            .child(session.title().to_owned()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(rgb(MUTED))
                                            .truncate()
                                            .child(session.cwd.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(rgb(if session.approval.is_some() {
                                                WARNING
                                            } else {
                                                MUTED
                                            }))
                                            .child(if session.approval.is_some() {
                                                "Needs approval".into()
                                            } else {
                                                session.state.clone()
                                            }),
                                    )
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.new_session = false;
                                        this.command(Command::Select(id.clone()), cx)
                                    }))
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .flex_1()
                .min_h_0(),
            )
            .child(
                div()
                    .p_4()
                    .text_size(px(11.))
                    .text_color(rgb(MUTED))
                    .child(format!(
                        "{} · {} sessions",
                        if self.view.connected {
                            "Connected"
                        } else {
                            "Reconnecting"
                        },
                        self.view.sessions.len()
                    )),
            );

        if self.new_session {
            let busy = self.spawn_pending || self.view.creating;
            let can_create = self.view.connected && !busy;
            return div()
                .key_context("Workspace")
                .track_focus(&self.focus)
                .size_full()
                .flex()
                .bg(rgb(BASE))
                .text_color(rgb(TEXT))
                .text_size(px(14.))
                .on_action(cx.listener(Self::send))
                .child(sidebar)
                .child(
                    div()
                        .id("new-session-form")
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .overflow_y_scroll()
                        .p_5()
                        .child(
                            div()
                                .max_w(px(640.))
                                .mx_auto()
                                .flex()
                                .flex_col()
                                .gap_3()
                                .child(div().text_size(px(20.)).child("New session"))
                                .child(
                                    div()
                                        .text_color(rgb(MUTED))
                                        .text_size(px(12.))
                                        .child("Start an agent on your connected Workspacer hub."),
                                )
                                .child("Provider")
                                .child(div().flex().gap_2().children(
                                    [("claude", "Claude"), ("codex", "Codex")].into_iter().map(
                                        |(provider, label)| {
                                            self.button(provider, label, !busy)
                                                .when(self.provider == provider, |d| {
                                                    d.bg(rgb(SELECTED))
                                                })
                                                .when(!busy, |d| {
                                                    d.on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            this.provider = provider;
                                                            cx.notify();
                                                        },
                                                    ))
                                                })
                                        },
                                    ),
                                ))
                                .child("Project directory")
                                .child(Input::new(&self.project).disabled(busy))
                                .child(
                                    div().text_size(px(12.)).text_color(rgb(MUTED)).child(
                                        "Use an existing absolute path on the hub's machine.",
                                    ),
                                )
                                .child("Session name")
                                .child(Input::new(&self.label).disabled(busy))
                                .child("Model (optional)")
                                .child(Input::new(&self.model).disabled(busy))
                                .child("First message")
                                .child(Input::new(&self.prompt).h(px(110.)).disabled(busy))
                                .when(!self.spawn_error.is_empty(), |d| {
                                    d.child(
                                        div()
                                            .text_color(rgb(WARNING))
                                            .child(self.spawn_error.clone()),
                                    )
                                })
                                .when(!self.view.connected, |d| {
                                    d.child(
                                        div()
                                            .text_color(rgb(WARNING))
                                            .child("Waiting for the hub connection…"),
                                    )
                                })
                                .child(
                                    div()
                                        .flex()
                                        .justify_between()
                                        .child(self.button("cancel-create", "Back", !busy).when(
                                            !busy,
                                            |d| {
                                                d.on_click(cx.listener(|this, _, _, cx| {
                                                    this.new_session = false;
                                                    cx.notify();
                                                }))
                                            },
                                        ))
                                        .child(
                                            self.button(
                                                "create-session",
                                                if busy {
                                                    "Creating session…"
                                                } else {
                                                    "Create session"
                                                },
                                                can_create,
                                            )
                                            .when(
                                                can_create,
                                                |d| {
                                                    d.on_click(
                                                        cx.listener(|this, _, _, cx| {
                                                            this.create(cx)
                                                        }),
                                                    )
                                                },
                                            ),
                                        ),
                                ),
                        ),
                )
                .into_any_element();
        }

        div().key_context("Workspace").track_focus(&self.focus).size_full().flex().bg(rgb(BASE)).text_color(rgb(TEXT)).text_size(px(14.))
            .on_action(cx.listener(Self::send))
            .on_action(cx.listener(|this, _: &CreateSession, window, cx| this.show_new_session(window, cx)))
            .on_action(cx.listener(|this, _: &NextSession, _, cx| this.move_selection(1, cx)))
            .on_action(cx.listener(|this, _: &PreviousSession, _, cx| this.move_selection(-1, cx)))
            .on_action(cx.listener(|this, _: &FocusComposer, window, cx| this.composer.update(cx, |input, cx| input.focus(window, cx))))
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.command(Command::Refresh, cx)))
            .child(sidebar)
            .child(div().flex_1().min_w_0().h_full().flex().flex_col()
                .child(div().px_5().py_3().flex().items_center().justify_between()
                    .child(div().child(title).child(div().text_size(px(11.)).text_color(rgb(MUTED)).child("Alt+↑/↓ sessions · Ctrl/Cmd+L compose")))
                    .child(self.button("refresh", "Refresh", true).on_click(cx.listener(|this, _, _, cx| this.command(Command::Refresh, cx)))))
                .when(!notice.is_empty(), |d| d.child(div().px_5().py_2().text_size(px(12.)).text_color(rgb(WARNING)).child(notice)))
                .when(self.view.transcript.omitted, |d| d.child(div().px_5().text_size(px(11.)).text_color(rgb(MUTED)).child("Showing recent history; older content is retained by the server.")))
                .when(self.view.loading, |d| d.child(div().px_5().text_color(rgb(MUTED)).child("Loading conversation…")))
                .when(!self.view.loading && self.view.transcript.rows.is_empty(), |d| d.child(div().p_5().text_color(rgb(MUTED)).child(if self.view.sessions.is_empty() {if self.view.connected {"No sessions yet. Choose New session to get started."} else {"Connecting to Workspacer. Start the desktop app or workspacer serve, or specify --bus."}} else {"No conversation yet."})))
                .child(transcript)
                .when(!self.follow, |d| d.child(self.button("latest", "Jump to latest", true).on_click(cx.listener(|this, _, _, cx| {
                    this.follow = true;
                    this.list.scroll_to(ListOffset { item_ix: this.view.transcript.rows.len(), offset_in_item: px(0.) }); cx.notify();
                }))))
                .when_some(selected.as_ref().and_then(|s| s.approval.as_ref()), |d, approval| {
                    let label = approval.get("toolName").or_else(|| approval.get("tool")).and_then(serde_json::Value::as_str).unwrap_or("Tool");
                    let details = approval.get("toolInput").or_else(|| approval.get("raw")).unwrap_or(approval).to_string();
                    d.child(div().px_5().py_2().flex().flex_col().gap_2().text_size(px(12.)).text_color(rgb(WARNING))
                        .child(format!("Approval requested: {label}"))
                        .child(div().id("approval-details").max_h(px(150.)).overflow_y_scroll().child(details))
                        .child(div().flex().gap_2()
                            .child(self.button("approve", "Allow once", enabled).when(enabled, |d| d.on_click(cx.listener(|this, _, _, cx| this.act(Action::Approve(true), cx)))))
                            .child(self.button("deny", "Deny", enabled).when(enabled, |d| d.on_click(cx.listener(|this, _, _, cx| this.act(Action::Approve(false), cx)))))))
                })
                .when_some(selected.as_ref().and_then(|s| s.questions.as_ref()), |d, questions| d.child(div().px_5().py_2().text_size(px(12.)).text_color(rgb(WARNING))
                    .child(format!("Question: {questions}"))
                    .child(self.button("answer", "Answer with composer", enabled).when(enabled, |d| d.on_click(cx.listener(|this, _, _, cx| this.act(Action::Answer(this.composer.read(cx).value().to_string()), cx)))))))
                .child(div().px_5().py_3().flex().flex_col().gap_2()
                    .child(Input::new(&self.composer).h(px(88.)).disabled(self.view.busy))
                    .child(div().flex().items_center().justify_between()
                        .child(div().text_size(px(11.)).text_color(rgb(MUTED)).child("Ctrl/Cmd+Enter send · Enter newline"))
                        .child(div().flex().gap_2()
                            .child(self.button("stop", "Interrupt", enabled).when(enabled, |d| d.on_click(cx.listener(|this, _, _, cx| this.act(Action::Stop, cx)))))
                            .child(self.button("send", if self.view.busy {"Sending…"} else {"Send"}, enabled).when(enabled, |d| d.on_click(cx.listener(|this, _, window, cx| this.send(&SendMessage, window, cx)))))))))
            .into_any_element()
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use gpui::{TestAppContext, VisualTestContext, size};
    use gpui_component::Root;
    use wks_native::{
        controller::Receipt,
        model::{ConversationSnapshot, Item, Session, Transcript},
    };

    fn fixture(
        cx: &mut TestAppContext,
    ) -> (
        Entity<Workspace>,
        VisualTestContext,
        async_channel::Receiver<Command>,
        async_channel::Sender<Arc<View>>,
    ) {
        cx.update(|cx| {
            gpui_component::init(cx);
            bind_keys(cx);
        });
        let (controller, commands, updates) = Controller::test_channels();
        let mut workspace = None;
        let window = cx.add_window(|window, cx| {
            let view = cx.new(|cx| Workspace::new(controller, true, window, cx));
            workspace = Some(view.clone());
            Root::new(view, window, cx)
        });
        let visual = VisualTestContext::from_window(window.into(), cx);
        visual.simulate_resize(size(px(1000.), px(700.)));
        (workspace.unwrap(), visual, commands, updates)
    }

    fn state(id: &str) -> View {
        View {
            connected: true,
            selected: Some(id.into()),
            sessions: Arc::new(vec![
                Session {
                    id: "a".into(),
                    label: "Alpha".into(),
                    state: "input".into(),
                    ..Default::default()
                },
                Session {
                    id: "b".into(),
                    label: "Beta".into(),
                    state: "input".into(),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        }
    }

    #[gpui::test]
    fn new_session_form_creates_once_and_keeps_failed_input(cx: &mut TestAppContext) {
        let (workspace, mut visual, commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.demo = false;
                this.update_view(Arc::new(state("a")), window, cx);
            })
        });
        visual.simulate_keystrokes("ctrl-n");
        visual.simulate_input("/work/project");
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                assert!(this.new_session);
                this.provider = "codex";
                this.prompt
                    .update(cx, |input, cx| input.set_value("Hello", window, cx));
            })
        });
        visual.simulate_keystrokes("ctrl-enter");
        visual.simulate_keystrokes("ctrl-enter");
        let Command::Create(request) = commands.try_recv().expect("create command") else {
            panic!("wrong command")
        };
        assert_eq!(request.cwd, "/work/project");
        assert_eq!(request.provider, "codex");
        assert_eq!(request.message, "Hello");
        assert!(commands.try_recv().is_err());
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut failed = state("a");
                failed.spawn_receipt = Some(wks_native::controller::SpawnReceipt {
                    number: 1,
                    session: None,
                    error: Some("Launch failed".into()),
                    unsent_message: None,
                });
                this.update_view(Arc::new(failed), window, cx);
                assert!(this.new_session);
                assert!(!this.spawn_pending);
                assert_eq!(this.prompt.read(cx).value().as_ref(), "Hello");
                assert_eq!(this.project.read(cx).value().as_ref(), "/work/project");
                assert_eq!(this.spawn_error, "Launch failed");
                let mut success = state("b");
                success.spawn_receipt = Some(wks_native::controller::SpawnReceipt {
                    number: 2,
                    session: Some("b".into()),
                    error: None,
                    unsent_message: Some("Hello".into()),
                });
                this.update_view(Arc::new(success), window, cx);
                assert!(!this.new_session);
                assert_eq!(this.composer.read(cx).value().as_ref(), "Hello");
            })
        });
    }

    #[gpui::test]
    fn keyboard_send_preserves_unicode_and_failed_drafts(cx: &mut TestAppContext) {
        let (workspace, mut visual, commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx)
            })
        });
        visual.simulate_keystrokes("ctrl-l");
        visual.simulate_input("résumé 🦀");
        visual.simulate_keystrokes("ctrl-enter");
        let Command::Act {
            session,
            action: Action::Send(text),
        } = commands
            .try_recv()
            .expect("send shortcut reaches controller")
        else {
            panic!("wrong action");
        };
        assert_eq!(session, "a");
        assert_eq!(text, "résumé 🦀");
        let mut failed = state("a");
        failed.receipt = Some(Receipt {
            number: 1,
            session: "a".into(),
            action: Action::Send(text.clone()),
            error: Some("Disconnected; outcome unknown".into()),
        });
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(failed), window, cx);
                assert_eq!(this.composer.read(cx).value().as_ref(), text);
            })
        });
    }

    #[gpui::test]
    fn late_send_completion_does_not_clear_another_sessions_draft(cx: &mut TestAppContext) {
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("a")), window, cx);
                this.composer
                    .update(cx, |input, cx| input.set_value("Alpha draft", window, cx));
                this.update_view(Arc::new(state("b")), window, cx);
                this.composer
                    .update(cx, |input, cx| input.set_value("Beta draft", window, cx));
                let mut acknowledged = state("b");
                acknowledged.receipt = Some(Receipt {
                    number: 1,
                    session: "a".into(),
                    action: Action::Send("Alpha draft".into()),
                    error: None,
                });
                this.update_view(Arc::new(acknowledged), window, cx);
                assert_eq!(this.composer.read(cx).value().as_ref(), "Beta draft");
                assert!(!this.drafts.contains_key("a"));
            })
        });
    }

    #[gpui::test]
    fn long_transcript_only_builds_visible_rows(cx: &mut TestAppContext) {
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        let mut view = state("a");
        let mut transcript = Transcript::default();
        transcript.snapshot(ConversationSnapshot {
            seq: 2000,
            first_seq: 1,
            items: (0..2000)
                .map(|i| Item {
                    kind: "assistant_text".into(),
                    text: format!("Message {i}\n\nA paragraph in a long transcript."),
                    ..Default::default()
                })
                .collect(),
        });
        view.transcript = transcript;
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
        });
        visual.run_until_parked();
        visual.simulate_resize(size(px(900.), px(650.)));
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| {
            let rendered = this.rendered_rows.borrow().len();
            assert!(
                rendered > 0 && rendered < 100,
                "constructed {rendered} of 2000 rows"
            );
        });
    }

    #[gpui::test]
    fn reselecting_the_same_session_updates_list_even_when_revisions_collide(
        cx: &mut TestAppContext,
    ) {
        let (workspace, mut visual, _commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                for count in [1, 4, 2] {
                    let mut view = state("a");
                    view.transcript.snapshot(ConversationSnapshot {
                        seq: count,
                        first_seq: 1,
                        items: (0..count)
                            .map(|_| Item {
                                kind: "assistant_text".into(),
                                text: "Fresh snapshot".into(),
                                ..Default::default()
                            })
                            .collect(),
                    });
                    assert_eq!(view.transcript.revision, 1);
                    this.update_view(Arc::new(view), window, cx);
                    assert_eq!(this.list.item_count(), count as usize);
                }
            })
        });
    }

    #[gpui::test]
    fn an_explicit_target_never_sends_to_the_default_session(cx: &mut TestAppContext) {
        let (workspace, mut visual, commands, _updates) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.open_session(Some("b".into()));
                this.composer.update(cx, |input, cx| {
                    input.set_value("targeted message", window, cx)
                });
                this.update_view(Arc::new(state("a")), window, cx);
                this.send(&SendMessage, window, cx);
                assert!(this.view.selected.is_none());
            })
        });
        assert!(matches!(commands.try_recv().unwrap(),Command::Select(id) if id == "b"));
        assert!(commands.try_recv().is_err());
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(state("b")), window, cx);
                this.composer.update(cx, |input, cx| {
                    input.set_value("still targeted", window, cx)
                });
                let mut disappeared = state("a");
                disappeared.sessions = Arc::new(vec![disappeared.sessions[0].clone()]);
                this.update_view(Arc::new(disappeared), window, cx);
                this.send(&SendMessage, window, cx);
                assert!(!this.view.connected);
                assert_eq!(this.view.selected.as_deref(), Some("b"));
            })
        });
        assert!(commands.try_recv().is_err());
    }
}
