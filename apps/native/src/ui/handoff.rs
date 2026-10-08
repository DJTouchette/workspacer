//! "Continue with Codex…" / "Continue with Claude…" and "Start fresh from a
//! summary": the successor's provider, launch settings and who writes the
//! brief, for the open session. The settings reuse the New Agent pickers
//! (catalog models, effort, access); the folder is the source's own and is
//! not offered as a choice. The controller writes the brief, then starts the
//! successor; see `wks_native::handoff`.
use super::*;
use gpui::AnyElement;
use wks_native::{
    features::Request,
    handoff::{self, Brief, Stage},
};

impl Workspace {
    /// Seed the shared launch pickers for the selected session's successor.
    /// Model and effort start on the target's defaults (the catalogs share no
    /// ids); access carries the source's, never wider.
    pub(super) fn open_handoff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.extras.handoff_seen = self
            .view
            .handoff_receipt
            .as_ref()
            .map_or(0, |receipt| receipt.number);
        self.extras.handoff_sent = false;
        let Some(source) = self.selected_session().cloned() else {
            return;
        };
        self.seed_handoff(&source, false, window, cx);
    }

    /// "Start fresh from a summary" for `session`: the handoff page with a
    /// new agent of the same provider and a cheap model's summary as the
    /// brief, so a session whose prompt cache has gone cold is continued
    /// without being resumed. The user reviews the settings and starts it.
    /// A session that is not open yet is selected first; the page follows
    /// once the selection lands.
    pub(super) fn start_summary_handoff(
        &mut self,
        session: &Session,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.spawn_pending || self.view.creating {
            return;
        }
        if self.view.selected.as_ref() != Some(&session.id) {
            self.extras.summary_pending = Some(session.id.clone());
            self.command(Command::Select(session.id.clone()), cx);
            return;
        }
        self.extras.summary_pending = None;
        self.show_screen(Screen::Handoff, window, cx);
        if self.screen != Screen::Handoff {
            return;
        }
        self.extras.notice.clear();
        self.extras.confirm_end = None;
        self.extras.handoff_seen = self
            .view
            .handoff_receipt
            .as_ref()
            .map_or(0, |receipt| receipt.number);
        self.extras.handoff_sent = false;
        self.seed_handoff(session, true, window, cx);
        cx.notify();
    }

    /// Open the page `start_summary_handoff` asked for once its session is
    /// the selected one; a selection that went elsewhere drops the request.
    pub(super) fn resume_summary_handoff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.extras.summary_pending.clone() else {
            return;
        };
        if self.view.selected.as_ref() != Some(&id) {
            if !self.view.sessions.iter().any(|s| s.id == id) {
                self.extras.summary_pending = None;
            }
            return;
        }
        if let Some(session) = self.selected_session().cloned() {
            self.start_summary_handoff(&session, window, cx);
        }
    }

    /// The successor `fresh` names for `source`: its own provider afresh, or
    /// the other one. `None` where the session cannot be handed off.
    pub(super) fn handoff_target(&self, source: &Session) -> Option<&'static str> {
        let [other, same] = handoff::targets(source).ok()?;
        Some(if self.extras.handoff_fresh {
            same
        } else {
            other
        })
    }

    /// Point the shared launch pickers at the chosen successor.
    fn seed_handoff(
        &mut self,
        source: &Session,
        fresh: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.extras.handoff_fresh = fresh;
        let Some(target) = self.handoff_target(source) else {
            return;
        };
        self.choose_provider(target, window, cx);
        // Codex lists models for the folder the successor will run in.
        self.projects.cwd = source.cwd.clone();
        self.model_choice.clear();
        self.context_window = None;
        self.effort.clear();
        self.model
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.reset_model_picker(window, cx);
        self.reconcile_effort(window, cx);
        self.permission = handoff::carry_permission(target, &source.permission_mode);
        // Starting fresh exists to leave the source's context alone, and an
        // ended agent cannot write its own brief: both default to the summary.
        self.extras.handoff_brief = if source.stopped() || fresh {
            Brief::Summary
        } else {
            Brief::Agent
        };
        self.load_models(false, cx);
        // Installed check only: no test request to the provider.
        self.request(
            Request::Setup {
                provider: target.into(),
                check: false,
            },
            cx,
        );
    }

    /// A handoff is in flight on this connection, or this window just asked
    /// for one and has not yet seen its outcome.
    pub(super) fn handoff_busy(&self) -> bool {
        self.view.handoff.is_some()
            || self.view.creating
            || (self.extras.handoff_sent
                && self
                    .view
                    .handoff_receipt
                    .as_ref()
                    .is_none_or(|receipt| receipt.number <= self.extras.handoff_seen))
    }

    pub(super) fn handoff_title(&self) -> String {
        match self.selected_session().and_then(|s| self.handoff_target(s)) {
            Some(_) if self.extras.handoff_fresh => "Start fresh from a summary".into(),
            Some(target) => format!("Continue with {}", handoff::provider_name(target)),
            None => "Continue with another agent".into(),
        }
    }

    pub(super) fn continue_handoff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.handoff_busy() || !self.view.connected || self.demo {
            return;
        }
        let Some(source) = self.selected_session().cloned() else {
            return;
        };
        let Some(target) = self.handoff_target(&source) else {
            return;
        };
        let model = if self.model_choice == "__custom" {
            self.model.read(cx).value().trim().to_owned()
        } else {
            self.model_choice.clone()
        };
        if self.model_choice == "__custom" && model.is_empty() {
            self.extras.notice = "Enter a custom model or choose Provider default.".into();
            self.model.update(cx, |input, cx| input.focus(window, cx));
            cx.notify();
            return;
        }
        let request = handoff::Request {
            source: source.id.clone(),
            brief: self.extras.handoff_brief,
            successor: wks_native::controller::NewSession {
                provider: target.into(),
                cwd: source.cwd.clone(),
                model,
                context_window: self.context_window,
                permission: self.permission,
                effort: self.effort.clone(),
                ..Default::default()
            },
        };
        // The controller checks again against its own session state; this
        // only keeps an invalid form from leaving the window.
        if let Err(error) = request.validate(&source) {
            self.extras.notice = error.to_string();
            cx.notify();
            return;
        }
        self.extras.notice.clear();
        self.extras.handoff_seen = self
            .view
            .handoff_receipt
            .as_ref()
            .map_or(0, |receipt| receipt.number);
        match self.controller.command(Command::Handoff(request)) {
            Ok(()) => self.extras.handoff_sent = true,
            Err(error) => self.extras.notice = error.to_string(),
        }
        cx.notify();
    }

    /// The installed check for `target`: (in flight, found). `None` found is
    /// unknown (not checked, failed, or an older hub), which never blocks.
    fn handoff_detection(&self, target: &str) -> (bool, Option<bool>) {
        let setup = self.view.requests.get("setup").filter(
            |state| matches!(&state.request, Request::Setup { provider, .. } if provider == target),
        );
        let found = setup.filter(|state| !state.loading).and_then(|state| {
            state.value["installed"]
                .as_array()?
                .iter()
                .find(|row| row["provider"] == target)?["found"]
                .as_bool()
        });
        (setup.is_some_and(|state| state.loading), found)
    }

    /// The page's primary action, in its header so it stays on screen at any
    /// window height; `None` where the session cannot be continued.
    pub(super) fn handoff_continue(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let target = self.handoff_target(self.selected_session()?)?;
        let enabled = !self.handoff_busy()
            && self.view.connected
            && !self.demo
            && self.handoff_detection(target).1 != Some(false);
        let label = SharedString::from(if self.extras.handoff_fresh {
            "Start fresh".into()
        } else {
            format!("Continue with {}", handoff::provider_name(target))
        });
        Some(
            self.primary_button("handoff-continue", label, enabled)
                .debug_selector(|| "handoff-continue".into())
                .flex_shrink_0()
                .when(enabled, |d| {
                    d.on_click(cx.listener(|this, _, window, cx| this.continue_handoff(window, cx)))
                }),
        )
    }

    pub(super) fn render_handoff(&self, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let Some(source) = self.selected_session().cloned() else {
            return div().child(chrome::notice_line(
                "Choose a session to continue.",
                chrome::Tone::Info,
                p,
                "handoff-none",
            ));
        };
        let target = match handoff::targets(&source) {
            Ok(_) => self.handoff_target(&source).unwrap_or("claude"),
            Err(error) => {
                return div().debug_selector(|| "handoff-unavailable".into()).child(
                    chrome::notice_line(
                        error.to_string(),
                        chrome::Tone::Warning,
                        p,
                        "handoff-unavailable",
                    ),
                );
            }
        };
        let name = handoff::provider_name(target);
        let source_name = handoff::provider_name(&source.provider);
        let fresh = self.extras.handoff_fresh;
        let busy = self.handoff_busy() || !self.view.connected;
        let (checking, found) = self.handoff_detection(target);
        let missing = found == Some(false);

        let fact = |label: &'static str, value: AnyElement| {
            div()
                .flex()
                .items_center()
                .gap_4()
                .min_w_0()
                .py_1()
                .child(
                    div()
                        .w(px(96.))
                        .flex_shrink_0()
                        .text_size(px(chrome::scale::META))
                        .text_color(rgb(p.muted))
                        .child(label),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .text_size(px(chrome::scale::BODY))
                        .child(value),
                )
        };
        let from = chrome::card(p)
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
            .child(projects::section_label("This session", p))
            .child(fact(
                "Agent",
                chrome::model_badge(&source, p, 12.).into_any_element(),
            ))
            .child(fact(
                "Folder",
                div()
                    .debug_selector(|| "handoff-folder".into())
                    .truncate()
                    .font_family(mono_font())
                    .text_size(px(chrome::scale::META))
                    .child(source.cwd.clone())
                    .into_any_element(),
            ))
            .child(
                div()
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.muted))
                    .child(if fresh {
                        format!(
                            "A new {name} agent starts in this same folder with an empty context \
                             and reads a written brief, so this session's long context is not \
                             read again. This session stays available with its history."
                        )
                    } else {
                        format!(
                            "{name} starts in this same folder and reads a written brief. This \
                             session stays available with its history; {name} does not receive \
                             the private context held by {source_name}."
                        )
                    }),
            );
        let other = handoff::target(&source).unwrap_or("codex");
        let way = |id: &'static str, label: String, value: bool| {
            self.button(id, label, !busy)
                .debug_selector(move || id.into())
                .when(fresh == value, |d| {
                    d.bg(rgb(p.selected)).text_color(rgb(p.accent))
                })
                .when(!busy && fresh != value, |d| {
                    d.on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(source) = this.selected_session().cloned() {
                            this.seed_handoff(&source, value, window, cx);
                        }
                        cx.notify();
                    }))
                })
        };
        let ways = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(way(
                "handoff-way-other",
                format!("Continue with {}", handoff::provider_name(other)),
                false,
            ))
            .child(way(
                "handoff-way-fresh",
                format!("Start a fresh {source_name}"),
                true,
            ));

        let detection = match (checking, found) {
            (true, _) => chrome::notice_line(
                format!("Checking that {name} is installed…"),
                chrome::Tone::Loading,
                p,
                "handoff-detect",
            ),
            (_, Some(true)) => chrome::notice_line(
                format!("{name} is installed on the hub’s machine."),
                chrome::Tone::Success,
                p,
                "handoff-detect",
            ),
            (_, Some(false)) => chrome::notice_line(
                format!("{name} was not found on the hub’s machine. Set it up before continuing."),
                chrome::Tone::Warning,
                p,
                "handoff-detect",
            ),
            (_, None) => chrome::notice_line(
                format!(
                    "Could not confirm {name} is installed; the hub refuses the launch if it is not."
                ),
                chrome::Tone::Info,
                p,
                "handoff-detect",
            ),
        };
        let successor = chrome::card(p)
            .p_4()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(chrome::provider_mark(target, 32., p))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(chrome::scale::HEADING))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(if fresh {
                                        format!("Fresh {name} agent")
                                    } else {
                                        format!("New {name} agent")
                                    }),
                            )
                            .child(
                                div()
                                    .debug_selector(|| "handoff-detect".into())
                                    .child(detection),
                            ),
                    )
                    .when(missing, |d| {
                        d.child(self.button("handoff-setup", "Agent setup", true).on_click(
                            cx.listener(|this, _, window, cx| {
                                this.open_feature(Screen::Setup, window, cx)
                            }),
                        ))
                    }),
            )
            .child(ways)
            .child(self.render_launch_options(busy, cx));

        let stopped = source.stopped();
        let brief = self.extras.handoff_brief;
        let choice = |id: &'static str, label: String, value: Brief, enabled: bool| {
            self.button(id, label, enabled)
                .debug_selector(move || id.into())
                .when(brief == value, |d| {
                    d.bg(rgb(p.selected)).text_color(rgb(p.accent))
                })
                .when(enabled, |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| {
                        this.extras.handoff_brief = value;
                        cx.notify();
                    }))
                })
        };
        let brief_card = chrome::card(p)
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(projects::section_label("Brief", p))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(choice(
                        "handoff-brief-agent",
                        format!("Written by {source_name}"),
                        Brief::Agent,
                        !busy && !stopped,
                    ))
                    .child(choice(
                        "handoff-brief-summary",
                        "Summary by a fast model".into(),
                        Brief::Summary,
                        !busy,
                    ))
                    .child(choice(
                        "handoff-brief-quick",
                        "Quick summary".into(),
                        Brief::Mechanical,
                        !busy,
                    )),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(p.muted))
                    .child(match (brief, stopped) {
                        (Brief::Agent, _) => format!(
                            "{source_name} stops its current work and writes the brief, which takes \
                             one turn and up to 2½ minutes. If it cannot, the hub uses a quick \
                             summary instead and says so."
                        ),
                        (Brief::Summary, _) => format!(
                            "A fast, inexpensive model (Haiku for Claude, unless Settings names \
                             another title model) reads the quick summary and the latest part of \
                             the conversation and writes the brief, in up to about 1½ minutes. \
                             {source_name} is not resumed or sent anything. If the model cannot, \
                             the hub uses the quick summary instead and says so."
                        ),
                        (Brief::Mechanical, true) => "This agent has ended, so the hub summarizes \
                            its retained conversation."
                            .into(),
                        (Brief::Mechanical, false) => "The hub summarizes the retained \
                            conversation now. Nothing is sent to this agent."
                            .into(),
                    }),
            );

        let progress = self
            .view
            .handoff
            .as_ref()
            .filter(|progress| progress.source == source.id);
        let receipt = self.view.handoff_receipt.as_ref().filter(|receipt| {
            receipt.number > self.extras.handoff_seen
                && receipt.source == source.id
                && receipt.successor.is_none()
        });
        let status = match (progress, receipt) {
            (Some(progress), _) => Some(chrome::notice_line(
                match progress.stage {
                    Stage::Brief(Brief::Agent) => {
                        format!("Asking {source_name} to write the brief…")
                    }
                    Stage::Brief(Brief::Summary) => "A fast model is writing the summary…".into(),
                    Stage::Brief(Brief::Mechanical) => "Summarizing the conversation…".into(),
                    Stage::Starting => format!("Starting {name}…"),
                },
                chrome::Tone::Loading,
                p,
                "handoff-progress",
            )),
            (None, Some(receipt)) => Some(chrome::notice_line(
                receipt.summary(),
                chrome::Tone::Error,
                p,
                "handoff-error",
            )),
            (None, None) if self.view.handoff.is_some() => Some(chrome::notice_line(
                "Another session’s handoff is in progress.",
                chrome::Tone::Info,
                p,
                "handoff-other",
            )),
            _ => None,
        };
        div()
            .debug_selector(|| "handoff-view".into())
            .flex()
            .flex_col()
            .gap_4()
            // Progress and failures sit beside the header's Continue, not
            // below the fold.
            .children(status.map(|s| s.debug_selector(|| "handoff-status".into())))
            .child(from)
            .child(successor)
            .child(brief_card)
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use gpui::{TestAppContext, VisualTestContext, size};
    use wks_native::{
        controller::SpawnReceipt,
        handoff::{Receipt, Written},
        launch::Permission,
        model::Session,
    };

    fn fixture(
        cx: &mut TestAppContext,
    ) -> (
        Entity<Workspace>,
        VisualTestContext,
        tokio::sync::mpsc::Receiver<Command>,
    ) {
        let (workspace, visual, commands, updates) = crate::ui::tests::fixture(cx);
        // These tests drive the view directly; the channel stays open.
        std::mem::forget(updates);
        // Tall enough that every card on the page can be clicked; the first
        // test checks where Continue itself sits in a short window.
        visual.simulate_resize(size(px(1200.), px(1600.)));
        (workspace, visual, commands)
    }

    fn source(provider: &str) -> Session {
        let mut s = Session {
            id: "a".into(),
            label: "Alpha".into(),
            state: "input".into(),
            cwd: "/work/repo-wt".into(),
            ..Default::default()
        };
        s.merge(&serde_json::json!({"provider":provider,"livePermissionMode":"default"}));
        s
    }

    fn view(sessions: Vec<Session>) -> View {
        View {
            connected: true,
            selected: Some("a".into()),
            sessions: Arc::new(sessions),
            ..Default::default()
        }
    }

    /// The next command that is not a read (project registry, catalog,
    /// installed agents) the window issues on its own.
    fn next_effect(commands: &mut tokio::sync::mpsc::Receiver<Command>) -> Option<Command> {
        std::iter::from_fn(|| commands.try_recv().ok()).find(|c| {
            !matches!(
                c,
                Command::LoadModels { .. }
                    | Command::Request(
                        Request::Setup { check: false, .. }
                            | Request::Projects
                            | Request::InspectProject { .. }
                            | Request::BrowseFolders { .. }
                            | Request::ProjectIcons { .. }
                    )
            )
        })
    }

    fn click(visual: &mut VisualTestContext, selector: &'static str) {
        visual.run_until_parked();
        let bounds = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not rendered"));
        visual.simulate_click(bounds.center(), gpui::Modifiers::default());
        visual.run_until_parked();
    }

    /// Rests the pointer on the title capsule, lets its secondary actions
    /// finish showing (the test platform draws no animation frames), then
    /// clicks one: hidden title actions ignore pointer clicks.
    fn click_title_action(
        workspace: &Entity<Workspace>,
        visual: &mut VisualTestContext,
        selector: &'static str,
    ) {
        visual.run_until_parked();
        let bar = visual.debug_bounds("title-bar").expect("title bar");
        visual.simulate_mouse_move(bar.center(), None, gpui::Modifiers::default());
        visual.run_until_parked();
        visual.update(|_, cx| {
            workspace.update(cx, |this, cx| {
                this.settle_title_reveal();
                cx.notify();
            })
        });
        click(visual, selector);
    }

    #[test]
    fn handoff_outcomes_take_honest_notice_tones() {
        let mut receipt = Receipt {
            number: 1,
            source: "a".into(),
            provider: "codex".into(),
            successor: Some("c".into()),
            brief: Some(Written {
                path: "/h/b.md".into(),
                fallback: None,
                kind: Brief::Agent,
            }),
            error: None,
        };
        let tone = |r: &Receipt| chrome::notice_tone(&r.summary());
        assert!(matches!(tone(&receipt), chrome::Tone::Success));
        receipt.brief.as_mut().unwrap().fallback = Some("deadline".into());
        assert!(matches!(tone(&receipt), chrome::Tone::Warning));
        receipt.successor = None;
        receipt.error = Some("codex is not installed".into());
        assert!(matches!(tone(&receipt), chrome::Tone::Error));
        receipt.brief = None;
        assert!(matches!(tone(&receipt), chrome::Tone::Error));
    }

    #[gpui::test]
    fn header_continue_with_codex_stages_the_takeover_for_review(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.demo = false;
                // A wider default must not widen the source's own access.
                this.settings.default_codex_access = Permission::FullAccess;
                this.update_view(Arc::new(view(vec![source("claude")])), window, cx);
                this.composer
                    .update(cx, |input, cx| input.set_value("source draft", window, cx));
            })
        });
        click_title_action(&workspace, &mut visual, "open-handoff");
        let seen: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
        assert!(seen.iter().any(|c| matches!(c,
            Command::Request(Request::Setup { provider, check: false }) if provider == "codex")));
        workspace.read_with(&visual, |this, cx| {
            assert_eq!(this.screen, Screen::Handoff);
            assert_eq!(this.provider, "codex");
            assert_eq!(this.permission, Permission::Ask);
            assert_eq!(this.extras.handoff_brief, Brief::Agent);
            assert_eq!(this.handoff_title(), "Continue with Codex");
            assert!(this.model_choice.is_empty(), "the target's default model");
            assert_eq!(this.composer.read(cx).value().as_ref(), "source draft");
        });
        assert!(visual.debug_bounds("handoff-folder").is_some());

        // Continue is the page header's action: on screen in a short window
        // without scrolling past the settings.
        visual.simulate_resize(size(px(1000.), px(520.)));
        visual.run_until_parked();
        let action = visual.debug_bounds("handoff-continue").unwrap();
        assert!(action.bottom() < px(130.), "{action:?}");
        click(&mut visual, "handoff-continue");
        let Some(Command::Handoff(request)) = next_effect(&mut commands) else {
            panic!("Continue must request one handoff");
        };
        assert_eq!(request.source, "a");
        assert_eq!(request.brief, Brief::Agent);
        assert_eq!(request.successor.provider, "codex");
        assert_eq!(request.successor.cwd, "/work/repo-wt");
        assert_eq!(request.successor.permission, Permission::Ask);
        assert!(request.successor.message.is_empty() && request.successor.model.is_empty());
        // A second click before the controller answers starts nothing.
        click(&mut visual, "handoff-continue");
        assert!(next_effect(&mut commands).is_none());

        // The controller's success: the successor is selected with its
        // takeover message staged, and the source keeps its own draft.
        let prompt = wks_native::handoff::successor_prompt("/h/.workspacer/handoffs/b.md");
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut next = view(vec![
                    source("claude"),
                    Session {
                        id: "c".into(),
                        provider: "codex".into(),
                        cwd: "/work/repo-wt".into(),
                        state: "idle".into(),
                        ..Default::default()
                    },
                ]);
                next.selected = Some("c".into());
                next.spawn_receipt = Some(SpawnReceipt {
                    number: 7,
                    session: Some("c".into()),
                    error: None,
                    unsent_message: Some(prompt.clone()),
                });
                next.handoff_receipt = Some(Receipt {
                    number: 7,
                    source: "a".into(),
                    provider: "codex".into(),
                    successor: Some("c".into()),
                    brief: Some(Written {
                        path: "/h/.workspacer/handoffs/b.md".into(),
                        fallback: None,
                        kind: Brief::Agent,
                    }),
                    error: None,
                });
                this.update_view(Arc::new(next), window, cx);
            })
        });
        visual.run_until_parked();
        workspace.read_with(&visual, |this, cx| {
            assert_eq!(this.screen, Screen::Conversation);
            assert_eq!(this.composer.read(cx).value().as_ref(), prompt);
            assert_eq!(
                this.drafts.get("a").map(String::as_str),
                Some("source draft")
            );
            assert!(!this.handoff_busy());
        });
        assert!(
            next_effect(&mut commands).is_none(),
            "nothing is sent for the user"
        );
    }

    #[gpui::test]
    fn failures_are_shown_and_a_retry_is_one_new_request(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.demo = false;
                let mut stopped = source("codex");
                stopped.state = "stopped".into();
                stopped.permission_mode = "yolo".into();
                this.update_view(Arc::new(view(vec![stopped])), window, cx);
                this.open_feature(Screen::Handoff, window, cx);
            })
        });
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.provider, "claude");
            assert_eq!(this.handoff_title(), "Continue with Claude");
            // Bypass intent carries across vocabularies; nothing wider exists.
            assert_eq!(this.permission, Permission::FullAccess);
            // An ended agent cannot take the turn to write its own brief; a
            // cheap model summarizes it instead.
            assert_eq!(this.extras.handoff_brief, Brief::Summary);
        });
        click(&mut visual, "handoff-brief-agent");
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.extras.handoff_brief, Brief::Summary)
        });
        click(&mut visual, "handoff-continue");
        let Some(Command::Handoff(first)) = next_effect(&mut commands) else {
            panic!("Continue must request a handoff");
        };
        assert_eq!(first.brief, Brief::Summary);
        assert_eq!(first.successor.provider, "claude");
        assert_eq!(first.successor.permission, Permission::FullAccess);

        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut next = (*this.view).clone();
                next.handoff_receipt = Some(Receipt {
                    number: 9,
                    source: "a".into(),
                    provider: "claude".into(),
                    successor: None,
                    brief: Some(Written {
                        path: "/h/.workspacer/handoffs/x.md".into(),
                        fallback: None,
                        kind: Brief::Summary,
                    }),
                    error: Some("profile unavailable".into()),
                });
                this.update_view(Arc::new(next), window, cx);
            })
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("handoff-status").is_some());
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.screen, Screen::Handoff);
            assert!(!this.handoff_busy());
        });
        click(&mut visual, "handoff-continue");
        assert!(matches!(
            next_effect(&mut commands),
            Some(Command::Handoff(_))
        ));
        assert!(next_effect(&mut commands).is_none());
    }

    #[gpui::test]
    fn managers_are_gated_and_a_missing_provider_cannot_be_chosen(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.demo = false;
                let mut manager = source("claude");
                manager.wake_target = true;
                this.update_view(Arc::new(view(vec![manager])), window, cx);
            })
        });
        click_title_action(&workspace, &mut visual, "open-handoff");
        assert!(visual.debug_bounds("handoff-unavailable").is_some());
        assert!(visual.debug_bounds("handoff-continue").is_none());
        assert!(next_effect(&mut commands).is_none());

        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut next = view(vec![source("claude")]);
                next.requests.insert(
                    "setup",
                    wks_native::features::RequestState {
                        number: 1,
                        request: Request::Setup {
                            provider: "codex".into(),
                            check: false,
                        },
                        loading: false,
                        value: Arc::new(serde_json::json!({
                            "installed":[{"provider":"codex","found":false}]
                        })),
                        error: None,
                    },
                );
                this.update_view(Arc::new(next), window, cx);
            })
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("handoff-continue").is_some());
        click(&mut visual, "handoff-continue");
        assert!(
            next_effect(&mut commands).is_none(),
            "missing Codex is not launched"
        );
    }

    fn stopped(id: &str) -> Session {
        let mut s = source("claude");
        s.id = id.into();
        s.state = "stopped".into();
        s
    }

    #[gpui::test]
    fn start_summary_handoff_starts_a_fresh_same_provider_agent(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.demo = false;
                this.update_view(Arc::new(view(vec![stopped("a")])), window, cx);
                let session = this.selected_session().cloned().unwrap();
                this.start_summary_handoff(&session, window, cx);
            })
        });
        visual.run_until_parked();
        let seen: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
        assert!(seen.iter().any(|c| matches!(c,
            Command::Request(Request::Setup { provider, check: false }) if provider == "claude")));
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.screen, Screen::Handoff);
            assert_eq!(this.provider, "claude");
            assert!(this.extras.handoff_fresh);
            assert_eq!(this.extras.handoff_brief, Brief::Summary);
            assert_eq!(this.handoff_title(), "Start fresh from a summary");
        });
        // The cross-provider way stays one click away, and back.
        click(&mut visual, "handoff-way-other");
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.provider, "codex");
            assert!(!this.extras.handoff_fresh);
            assert_eq!(this.handoff_title(), "Continue with Codex");
            assert_eq!(this.extras.handoff_brief, Brief::Summary, "an ended agent");
        });
        click(&mut visual, "handoff-way-fresh");
        workspace.read_with(&visual, |this, _| assert_eq!(this.provider, "claude"));
        click(&mut visual, "handoff-continue");
        let Some(Command::Handoff(request)) = next_effect(&mut commands) else {
            panic!("Start fresh must request one handoff");
        };
        assert_eq!(request.source, "a");
        assert_eq!(request.brief, Brief::Summary);
        assert_eq!(request.successor.provider, "claude");
        assert_eq!(request.successor.cwd, "/work/repo-wt");
        assert!(request.successor.message.is_empty());
        assert!(
            request.successor.resume_session_id.is_none(),
            "never a resume"
        );
        // The progress line names the cheap model, not the source.
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut next = (*this.view).clone();
                next.handoff = Some(wks_native::handoff::Progress {
                    number: 3,
                    source: "a".into(),
                    provider: "claude".into(),
                    stage: Stage::Brief(Brief::Summary),
                });
                this.update_view(Arc::new(next), window, cx);
            })
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("handoff-status").is_some());
        workspace.read_with(&visual, |this, _| assert!(this.handoff_busy()));
    }

    #[gpui::test]
    fn start_summary_handoff_selects_the_session_first_and_follows_it(cx: &mut TestAppContext) {
        let (workspace, mut visual, mut commands) = fixture(cx);
        let paused = stopped("b");
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.demo = false;
                this.update_view(
                    Arc::new(view(vec![source("claude"), paused.clone()])),
                    window,
                    cx,
                );
                this.start_summary_handoff(&paused, window, cx);
            })
        });
        visual.run_until_parked();
        assert!(matches!(
            next_effect(&mut commands),
            Some(Command::Select(id)) if id == "b"
        ));
        workspace.read_with(&visual, |this, _| {
            assert_ne!(
                this.screen,
                Screen::Handoff,
                "not before the selection lands"
            )
        });
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut next = view(vec![source("claude"), paused.clone()]);
                next.selected = Some("b".into());
                this.update_view(Arc::new(next), window, cx);
            })
        });
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.screen, Screen::Handoff);
            assert!(this.extras.handoff_fresh && this.extras.summary_pending.is_none());
            assert_eq!(this.extras.handoff_brief, Brief::Summary);
        });
    }

    #[gpui::test]
    fn session_details_offer_a_fresh_start_and_live_sessions_can_pick_the_summary(
        cx: &mut TestAppContext,
    ) {
        let (workspace, mut visual, _commands) = fixture(cx);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.demo = false;
                this.update_view(Arc::new(view(vec![source("codex")])), window, cx);
                this.open_feature(Screen::Session, window, cx);
            })
        });
        click(&mut visual, "fresh-session");
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.screen, Screen::Handoff);
            assert_eq!(this.provider, "codex");
            assert!(this.extras.handoff_fresh);
            // Starting fresh leaves even a live source's context alone.
            assert_eq!(this.extras.handoff_brief, Brief::Summary);
        });
        // The ordinary entry keeps its cross-provider, agent-written default,
        // and the summary is one of its brief choices.
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.open_feature(Screen::Handoff, window, cx)
            })
        });
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.provider, "claude");
            assert!(!this.extras.handoff_fresh);
            assert_eq!(this.extras.handoff_brief, Brief::Agent);
        });
        click(&mut visual, "handoff-brief-summary");
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.extras.handoff_brief, Brief::Summary)
        });
    }
}
