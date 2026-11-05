//! Design v2's conversation header (R3.3): a header strip with the title,
//! its status words and the red failure chip on the left and the labelled,
//! badged session toolbar on the right; the transcript scrolls beneath it.
//! The shipped capsule is `chrome::render_title_bar`.
use super::primitives::{self, StatusKind, tooltip_view};
use super::*;
use gpui::AnyElement;
use wks_native::background_tasks::{self as bg, Kind};

/// "3 subagents", "1 shell task", "2 background tasks": the running work
/// in words. `None` when nothing is running.
pub(super) fn running_words(tasks: &[bg::Task], fallback: u32) -> Option<String> {
    let running: Vec<&bg::Task> = tasks.iter().filter(|t| t.running()).collect();
    let n = if tasks.is_empty() {
        fallback as usize
    } else {
        running.len()
    };
    if n == 0 {
        return None;
    }
    let kind = running.first().map(|t| t.kind);
    let same = kind.is_some() && running.iter().all(|t| Some(t.kind) == kind);
    let (one, many) = match kind.filter(|_| same) {
        Some(Kind::Agent | Kind::Teammate | Kind::Remote) => ("subagent", "subagents"),
        Some(Kind::Shell) => ("shell task", "shell tasks"),
        Some(Kind::Workflow) => ("workflow", "workflows"),
        _ => ("background task", "background tasks"),
    };
    Some(format!("{n} {}", if n == 1 { one } else { many }))
}

/// The session's access mode in a word or two; "Unknown" when the provider
/// has not reported one this app offers.
pub(super) fn access_word(session: &Session) -> (&'static str, Option<Permission>) {
    match Permission::from_wire(session.provider_id(), session.access_mode()) {
        Some(Permission::Ask) => ("Ask", Some(Permission::Ask)),
        Some(Permission::AcceptEdits) => ("Edits", Some(Permission::AcceptEdits)),
        Some(Permission::Plan) => ("Plan", Some(Permission::Plan)),
        Some(Permission::FullAccess) => ("Full access", Some(Permission::FullAccess)),
        None => ("Unknown", None),
    }
}

impl Workspace {
    /// Where the header's fade turns from solid canvas to clear. v2 draws a
    /// solid header strip (no fade); v1 keeps its gentler 60%.
    pub(super) fn title_fade_stop(&self) -> f32 {
        if !self.design().is_v2() {
            return 0.6;
        }
        1.
    }

    /// The selected session's state for the header: the one derivation
    /// (r4-truth), Offline while the hub is away.
    pub(super) fn title_kind(&self, session: &Session) -> StatusKind {
        self.session_kind(session, timing::now_ms())
    }

    // r3-shell
    /// The v2 conversation header (R3.3): a real strip at the top of the
    /// conversation, the transcript scrolling beneath it. The title and its
    /// status words on the left, the session toolbar on the right; below
    /// the strip, the notices and the pinned plan in the reading column.
    /// Everything it covers is padding (`header_bounds`), never content.
    pub(super) fn render_chat_header_v2(
        &mut self,
        layout: super::conversation::ChatLayout,
        selected: Option<&Session>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let sh = t.shell();
        let title = selected
            .map(|s| self.session_title(s))
            .unwrap_or_else(|| "Your sessions".into());
        let child_bar = self.render_child_title_bar(cx);
        let viewing_child = child_bar.is_some();
        let strip_shown = selected.is_some() || viewing_child;
        let left = match child_bar {
            Some(bar) => bar
                .bg(gpui::transparent_black())
                .border_0()
                .shadow(Vec::new())
                .flex_shrink()
                .min_w_0(),
            None => self.render_title_bar_v2(layout.narrow, layout.enabled, &title, selected, cx),
        };
        let toolbar = (selected.is_some() && !viewing_child)
            .then(|| self.session_toolbar_v2(layout.enabled, cx));
        let compact = unzoom(window.viewport_size().height) < 300.;
        let notice = self.title_notice();
        let notices: Vec<AnyElement> = self
            .title_notice_rows(notice, false, compact, cx)
            .into_iter()
            .map(|(_, row)| row)
            .collect();
        let plan = self.render_pinned_plan(layout.compact, window, cx);
        // A header that grows or shrinks while reading history keeps the
        // same message under the reader's eye.
        let header_size = chrome::measure(cx.entity().downgrade(), |this: &mut Self, bounds, _| {
            if this.header_bounds == bounds {
                return false;
            }
            let delta = bounds.size.height - this.header_bounds.size.height;
            if !this.follow && delta != px(0.) {
                let mut anchor = this.scroll_anchor();
                anchor.offset_in_item += delta;
                this.list.scroll_to(anchor);
            }
            this.header_bounds = bounds;
            true
        });
        let caption = chrome::custom_caption();
        let strip = div()
            .id("chat-header")
            .debug_selector(|| "chat-header".into())
            .relative()
            .w_full()
            .h(px(sh.header))
            .flex_shrink_0()
            .pl(px(t.space.lg))
            .pr(px(t.space.sm))
            .flex()
            .items_center()
            .gap(px(t.space.md))
            .border_b(px(t.stroke.hairline))
            .border_color(rgb(p.border))
            // The strip's empty room drags the window; its controls occlude.
            .when(caption, |d| {
                d.child(
                    chrome::drag_region(div())
                        .debug_selector(|| "chat-drag-region".into())
                        .absolute()
                        .inset_0(),
                )
                .pr(px(chrome::CAPTION_WIDTH + t.space.sm))
            })
            .child(left)
            .child(div().flex_1().min_w_0())
            .children(toolbar);
        let band = (!notices.is_empty() || plan.is_some()).then(|| {
            div()
                .w_full()
                .flex()
                .justify_center()
                .pt(px(t.space.sm))
                .pb(px(t.space.sm))
                .child(
                    self.reading_column()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(t.space.sm))
                        .when(!notices.is_empty(), |d| {
                            d.child(
                                div()
                                    .id("title-island-notices")
                                    .debug_selector(|| "title-island-notices".into())
                                    .occlude()
                                    .w_full()
                                    .max_h(window.viewport_size().height * 0.35)
                                    .overflow_y_scroll()
                                    .px(px(t.space.md))
                                    .py(px(t.space.xs))
                                    .flex()
                                    .flex_col()
                                    .gap(px(t.space.xs))
                                    .rounded(px(t.radius.panel))
                                    .bg(rgb(p.surface))
                                    .border(px(t.stroke.hairline))
                                    .border_color(rgb(p.roles.card_border))
                                    .children(notices),
                            )
                        })
                        .children(plan),
                )
        });
        div()
            .absolute()
            .top_0()
            .left_0()
            .w_full()
            .flex()
            .flex_col()
            .bg(rgb(p.chat))
            .child(header_size)
            .map(|d| {
                if strip_shown {
                    d.child(strip)
                } else if caption {
                    // The welcome keeps its own question; only the drag room.
                    d.child(
                        chrome::drag_region(div())
                            .debug_selector(|| "chat-drag-region".into())
                            .w_full()
                            .h(px(sh.header)),
                    )
                } else {
                    d
                }
            })
            .children(band)
    }

    /// The v2 title, the header strip's left side: the status (glyph and
    /// word; Idle shows none), the project crumb (it gives way first), the
    /// session title (truncated last), the access mode once it is known,
    /// the session's chips and the red chip naming a failure. The model
    /// lives in the composer's chip; running work in the toolbar's Tasks.
    pub(super) fn render_title_bar_v2(
        &self,
        narrow: bool,
        _enabled: bool,
        title: &str,
        session: Option<&Session>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.palette();
        let t = p.tokens;
        div()
            .id("title-bar")
            .debug_selector(|| "title-bar".into())
            .occlude()
            .min_w_0()
            .flex_shrink()
            .h_full()
            .flex()
            .items_center()
            .gap(px(t.space.sm))
            .when_some(session, |d, session| {
                let kind = self.title_kind(session);
                let label = kind.label();
                d.child(
                    div()
                        .id("title-status")
                        .debug_selector(|| "title-status".into())
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .gap(px(t.space.xs))
                        .text_size(px(t.text.meta))
                        .font_weight(FontWeight(t.weight.medium))
                        .text_color(rgb(kind.text_color(&p)))
                        .tooltip(tooltip_view(label, None, &p))
                        .children(primitives::status_glyph(kind, &p))
                        // R3.1: Idle shows no glyph and no word.
                        .when(kind.has_glyph() && !narrow, |d| d.child(label)),
                )
                .when(kind.has_glyph() && !narrow, |d| {
                    d.child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(t.text.body))
                            .text_color(rgb(p.roles.text_tertiary))
                            .child("·"),
                    )
                })
                // r4-calm (R4.4): the crumb is the first thing to go, and
                // its "/" goes with it (one run of text, never an orphan);
                // with a side panel narrowing the column it steps aside so
                // the title keeps its room.
                .when(!narrow && self.title_crumb_fits(), |d| {
                    let project = self.project_name(&session.cwd);
                    d.child(
                        div()
                            .id("title-crumb")
                            .debug_selector(|| "title-crumb".into())
                            .min_w_0()
                            .max_w(px(t.layout.menu_width / 2.))
                            // Gives way long before the title does.
                            .map(|mut d| {
                                d.style().flex_shrink = Some(1000.);
                                d
                            })
                            .truncate()
                            .text_size(px(t.text.body))
                            .text_color(rgb(p.roles.text_tertiary))
                            .tooltip(tooltip_view(project.clone(), None, &p))
                            .child(format!("{project} /")),
                    )
                })
            })
            .child(
                div()
                    .id("title-text")
                    .debug_selector(|| "title-text".into())
                    .min_w_0()
                    .truncate()
                    .text_size(px(t.text.subheading))
                    .font_weight(FontWeight(t.weight.semibold))
                    .text_color(rgb(p.text))
                    .tooltip(tooltip_view(title.to_owned(), None, &p))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.title_reveal.tap(std::time::Instant::now());
                        cx.notify();
                    }))
                    .child(title.to_owned()),
            )
            .when_some(session, |d, session| {
                let (access, mode) = access_word(session);
                let full = mode == Some(Permission::FullAccess);
                // Hidden until the agent reports a mode this app offers:
                // "unknown" is not something to show or choose.
                d.when_some(
                    mode.filter(|_| self.supported_session() && !narrow),
                    |d, mode| {
                        d.child(
                            chrome::interactive_control(
                                div().id("title-access"),
                                p,
                                self.supported_session(),
                            )
                            .debug_selector(|| "title-access".into())
                            .flex_shrink_0()
                            .h(px(t.layout.control_height_sm))
                            .px(px(t.space.sm))
                            .rounded(px(t.radius.control))
                            .flex()
                            .items_center()
                            .gap(px(t.space.xs))
                            .text_size(px(t.text.meta))
                            .font_weight(FontWeight(t.weight.medium))
                            // r4-calm: amber is only "needs you"; full
                            // access keeps its alert glyph, in neutral text.
                            .text_color(rgb(p.roles.text_secondary))
                            .hover(move |s| s.bg(rgb(p.roles.hover)))
                            .tooltip(tooltip_view(
                                format!(
                                    "Access: {} · {} · click to change",
                                    mode.label(),
                                    mode.description()
                                ),
                                None,
                                &p,
                            ))
                            .when(full, |d| {
                                d.child(Icon::new(IconName::TriangleAlert).size(px(t.icon.xs)))
                            })
                            .child(access)
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.open_model_popover(window, cx);
                                    this.shell.model_from_header = true;
                                },
                            )),
                        )
                    },
                )
            })
            .children(
                session
                    .map(|s| self.status_chips(s, narrow, cx))
                    .unwrap_or_default(),
            )
            .children(session.and_then(|s| {
                // r4-truth: only a failure still open (not since passed).
                let failed = self.open_failures(s);
                failed
                    .first()
                    .map(|task| self.render_failed_chip(task, failed.len(), s, cx))
            }))
            .children(session.and_then(|s| self.render_stall_recovery(s, narrow, cx)))
    }

    /// The red chip naming the newest unacknowledged failed task: the name
    /// opens it (its log, or an agent's conversation), the ✕ acknowledges it.
    fn render_failed_chip(
        &self,
        task: &bg::Task,
        count: usize,
        session: &Session,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let t = p.tokens;
        let name = task.title();
        // r4-truth: it names its task ("cargo test failed · Restore sessions").
        let owner = self.session_title(session);
        let mut label = super::status::failure_label(task, &owner, 28);
        if count > 1 {
            label.push_str(&format!(" · +{}", count - 1));
        }
        let error = task.error.trim();
        let tip = if error.is_empty() {
            format!("Background task “{name}” in {owner} failed · open to see why")
        } else {
            format!("Background task “{name}” in {owner} failed · {error}")
        };
        let open_id = task.id.clone();
        let ack_id = task.id.clone();
        chrome::interactive_control(div().id("title-failed"), p, true)
            .debug_selector(|| "title-failed".into())
            // r4-calm (R4.4): it gives way (its words truncate) before the
            // title does, after the crumb; the glyph and ✕ stay.
            .min_w(px(t.layout.control_height_sm * 2.))
            .map(|mut d| {
                d.style().flex_shrink = Some(100.);
                d
            })
            .h(px(t.layout.control_height_sm))
            .pl(px(t.space.md))
            .pr(px(t.space.xxs))
            .rounded_full()
            .flex()
            .items_center()
            .gap(px(t.space.xs))
            .text_size(px(t.text.meta))
            .font_weight(FontWeight(t.weight.medium))
            .text_color(rgb(p.roles.status.fault))
            .bg(primitives::wash(p.roles.status.fault, t.wash.tone_rest))
            .hover(move |s| s.bg(primitives::wash(p.roles.status.fault, t.wash.tone_hover)))
            .children(primitives::status_glyph(StatusKind::Failed, &p))
            .child(div().min_w_0().truncate().child(label))
            .tooltip(tooltip_view(tip, None, &p))
            .on_click(
                cx.listener(move |this, _, window, cx| this.open_task_by_id(&open_id, window, cx)),
            )
            .child(
                chrome::interactive_control(div().id("title-failed-ack"), p, true)
                    .debug_selector(|| "title-failed-ack".into())
                    .size(px(t.layout.control_height_sm - t.space.sm))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .hover(move |s| s.bg(primitives::wash(p.roles.status.fault, t.wash.tone_hover)))
                    .tooltip(tooltip_view("Dismiss · you have seen it", None, &p))
                    .child(Icon::new(IconName::Close).size(px(t.icon.xs)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.shell.acknowledged_failures.insert(ack_id.clone());
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    // r4-truth (R4.7)
    /// A session that stopped answering offers its way out right in the
    /// header: Nudge (the subagent rows' machinery: a line it reads at once)
    /// and Stop. Shown only while the one status reads Not responding.
    fn render_stall_recovery(
        &self,
        session: &Session,
        narrow: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.title_kind(session) != StatusKind::NotResponding {
            return None;
        }
        let p = self.palette();
        let t = p.tokens;
        let enabled = self.view.connected && !self.view.busy;
        let nudge_id = session.id.clone();
        let stop_id = session.id.clone();
        Some(
            div()
                .debug_selector(|| "title-stall-recovery".into())
                .occlude()
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap(px(t.space.xs))
                .child(
                    primitives::with_tooltip(
                        primitives::button(
                            "title-nudge",
                            "Nudge",
                            None,
                            primitives::ButtonVariant::Secondary,
                            primitives::ButtonSize::Small,
                            enabled,
                            &p,
                        ),
                        "Nudge · ask it to report back: sends “Still there? Reply with where you are.” now",
                        None,
                        &p,
                    )
                    .debug_selector(|| "title-nudge".into())
                    .when(enabled, |d| {
                        d.on_click(cx.listener(move |this, _, _, cx| {
                            this.command(
                                Command::Act {
                                    session: nudge_id.clone(),
                                    action: Action::SendNow(
                                        super::transcript_v2::NUDGE_TEXT.into(),
                                    ),
                                },
                                cx,
                            )
                        }))
                    }),
                )
                .when(!narrow, |d| {
                    d.child(
                        primitives::with_tooltip(
                            primitives::button(
                                "title-stop",
                                "Stop",
                                None,
                                primitives::ButtonVariant::Tertiary,
                                primitives::ButtonSize::Small,
                                enabled,
                                &p,
                            ),
                            "Stop this turn",
                            None,
                            &p,
                        )
                        .debug_selector(|| "title-stop".into())
                        .when(enabled, |d| {
                            d.on_click(cx.listener(move |this, _, _, cx| {
                                this.command(
                                    Command::Act {
                                        session: stop_id.clone(),
                                        action: Action::Stop,
                                    },
                                    cx,
                                )
                            }))
                        }),
                    )
                })
                .into_any_element(),
        )
    }

    // r4-calm (R4.4)
    /// Whether the header has room for the project crumb beside the title:
    /// not while a side panel or a narrow window squeezes the column.
    pub(super) fn title_crumb_fits(&self) -> bool {
        f32::from(self.chat_width) >= self.tokens().shell().toolbar_labels_at
    }

    // r4-truth (R4.1)
    /// The selected session's changes in the Changes panel's own scope and
    /// words: every uncommitted change on its branch, `(files, added,
    /// removed)`, once the panel has read them for this folder. The header
    /// shows exactly this, labelled "Branch", never a count of its own.
    pub(super) fn branch_line_changes(&self) -> Option<(usize, usize, usize)> {
        let cwd = &self.selected_session()?.cwd;
        self.review_stack_summary(cwd)
    }

    // r4-calm (R4.4)
    /// One toolbar tool: a 16px icon (never a label: the title keeps the
    /// room), a 28px hit area, an optional badge, and a tooltip naming it
    /// with its shortcut.
    #[allow(clippy::too_many_arguments)]
    fn toolbar_tool(
        &self,
        id: &'static str,
        icon: impl Into<Icon>,
        tip: SharedString,
        keys: Option<&'static str>,
        enabled: bool,
        on: bool,
        badge: Option<AnyElement>,
    ) -> Stateful<Div> {
        let p = self.palette();
        let t = p.tokens;
        chrome::interactive_control(div().id(id), p, enabled)
            .debug_selector(move || id.into())
            .flex_shrink_0()
            .h(px(t.layout.control_height_sm))
            .min_w(px(t.layout.control_height_sm))
            .px(px(if badge.is_some() { t.space.sm } else { 0. }))
            .rounded(px(t.radius.control))
            .flex()
            .items_center()
            .justify_center()
            .gap(px(t.space.xs))
            .text_size(px(t.text.meta))
            .font_weight(FontWeight(t.weight.medium))
            .map(super::shell::tabular)
            .text_color(rgb(if !enabled {
                p.disabled
            } else if on {
                p.text
            } else {
                p.roles.text_secondary
            }))
            .when(on, |d| d.bg(rgb(p.selected)))
            .when(enabled && !on, |d| {
                d.hover(move |s| s.bg(rgb(p.roles.hover)).text_color(rgb(p.text)))
                    .active(move |s| s.bg(rgb(p.roles.pressed)))
            })
            .tooltip(tooltip_view(tip, keys, &p))
            .child(Icon::new(icon).size(px(t.icon.md)))
            .children(badge)
    }

    // r3-shell, r4-calm (R4.4)
    /// The session toolbar on the header strip's right: icon buttons for
    /// Changes (Branch +N −M), Files, Terminal (● while its shell runs),
    /// Tasks (running count), then one "…" holding History, Details and
    /// Model. Each tooltip names the tool and its shortcut.
    pub(super) fn session_toolbar_v2(&self, enabled: bool, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let session = self.selected_session();
        let selected = session.is_some();
        let vim = self.vim();
        let mac = cfg!(target_os = "macos");
        let quiet = |text: String| {
            div()
                .flex_shrink_0()
                .text_color(rgb(p.roles.text_tertiary))
                .child(text)
                .into_any_element()
        };
        // r4-truth: the Changes panel's scope and label, "Branch +17 −3".
        let lines = self.branch_line_changes();
        let changes_tip: SharedString = match lines {
            Some((files, added, removed)) => format!(
                "Changes · Branch: every uncommitted change, {files} {}, +{added} −{removed}",
                if files == 1 { "file" } else { "files" }
            )
            .into(),
            None => "Changes".into(),
        };
        let changes = self
            .toolbar_tool(
                "open-changes",
                IconName::Replace,
                changes_tip,
                vim.then_some("g d"),
                selected,
                self.review_dock_showing(),
                lines
                    .filter(|(_, added, removed)| added + removed > 0)
                    .map(|(_, added, removed)| format!("Branch +{added} −{removed}"))
                    .map(quiet),
            )
            .when(selected, |d| {
                d.on_click(
                    cx.listener(|this, _, window, cx| {
                        this.open_feature(Screen::Changes, window, cx)
                    }),
                )
            });
        let files = self
            .toolbar_tool(
                "open-editor",
                IconName::FolderOpen,
                "Files and editor".into(),
                Some(if mac { "⌘ Shift E" } else { "Ctrl Shift E" }),
                selected,
                false,
                None,
            )
            .when(selected, |d| {
                d.on_click(cx.listener(|this, _, window, cx| this.open_selected_editor(window, cx)))
            });
        let shell_running = self
            .view
            .selected
            .as_ref()
            .is_some_and(|id| self.view.terminals.contains_key(id));
        let terminal = self
            .toolbar_tool(
                "open-terminal",
                IconName::SquareTerminal,
                if self.terminal.open {
                    "Hide terminal".into()
                } else if shell_running {
                    "Terminal · its shell is running".into()
                } else {
                    "Terminal in this session's folder".into()
                },
                Some(if mac { "⌘ `" } else { "Ctrl `" }),
                selected,
                self.terminal.open,
                shell_running.then(|| {
                    div()
                        .debug_selector(|| "terminal-running-dot".into())
                        .size(px(t.layout.status_dot))
                        .rounded(px(t.radius.full))
                        .bg(rgb(p.roles.status.working))
                        .into_any_element()
                }),
            )
            .when(selected, |d| {
                d.on_click(cx.listener(|this, _, window, cx| this.toggle_terminal(window, cx)))
            });
        let running = session.map_or(0, |s| {
            if s.tasks.is_empty() {
                s.background_tasks as usize
            } else {
                s.tasks.iter().filter(|t| t.running()).count()
            }
        });
        let tasks_tip: SharedString = session
            .and_then(|s| running_words(&s.tasks, s.background_tasks))
            .map(|words| format!("Tasks · {words} running"))
            .unwrap_or_else(|| "Tasks · background work".into())
            .into();
        let tasks = self
            .toolbar_tool(
                "title-tasks",
                primitives::Concept::Tasks,
                tasks_tip,
                None,
                selected,
                self.tasks.open,
                (running > 0).then(|| {
                    primitives::count_badge(
                        primitives::count_text(running),
                        primitives::CountTone::Neutral,
                        &p,
                    )
                    .into_any_element()
                }),
            )
            .when(selected, |d| {
                d.on_click(cx.listener(|this, _, window, cx| this.toggle_tasks(window, cx)))
            });
        div()
            .debug_selector(|| "session-toolbar".into())
            .occlude()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(t.space.xxs))
            .child(changes)
            .child(files)
            .child(terminal)
            .child(tasks)
            .child(self.session_more_menu(enabled, cx))
    }

    // r4-calm (R4.4)
    /// The toolbar's "…": History, Details and Model, each with its
    /// shortcut, in one menu that hangs under the strip at its right.
    fn session_more_menu(&self, enabled: bool, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let selected = self.selected_session().is_some();
        let model_enabled = enabled && self.supported_session();
        let vim = self.vim();
        let open = self.shell.session_menu && selected;
        let trigger = primitives::icon_button(
            "session-more",
            IconName::Ellipsis,
            "History, details and model",
            primitives::ButtonVariant::Tertiary,
            selected,
            &p,
        )
        .debug_selector(|| "session-more".into())
        .when(open, |d| d.bg(rgb(p.selected)).text_color(rgb(p.text)))
        .when(selected, |d| {
            d.on_click(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                this.shell.session_menu = !this.shell.session_menu;
                cx.notify();
            }))
        });
        let item = |id: &'static str,
                    label: &'static str,
                    icon: Icon,
                    keys: Option<&'static str>,
                    enabled: bool| {
            primitives::menu_item_icon(id, label, Some(icon), false, enabled, &p)
                .debug_selector(move || id.into())
                .when_some(keys, |d, keys| {
                    d.child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(t.text.meta))
                            .text_color(rgb(p.roles.text_tertiary))
                            .child(keys),
                    )
                })
        };
        let close = |this: &mut Self| this.shell.session_menu = false;
        let menu = primitives::menu(&p)
            .debug_selector(|| "session-more-menu".into())
            .occlude()
            .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
                close(this);
                cx.notify();
            }))
            .child(
                item(
                    "open-history",
                    "Conversation history",
                    Icon::new(primitives::Concept::History),
                    vim.then_some("g h"),
                    selected,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    close(this);
                    this.open_feature(Screen::History, window, cx)
                })),
            )
            .child(
                item(
                    "open-session",
                    "Session details",
                    Icon::new(primitives::Concept::Details),
                    vim.then_some("g e"),
                    selected,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    close(this);
                    this.open_feature(Screen::Session, window, cx)
                })),
            )
            .child(
                item(
                    "open-model",
                    "Model, effort and access",
                    Icon::new(IconName::Settings2),
                    vim.then_some("g m"),
                    model_enabled,
                )
                .when(model_enabled, |d| {
                    d.on_click(cx.listener(move |this, _, window, cx| {
                        close(this);
                        this.open_model_popover(window, cx);
                        this.shell.model_from_header = true;
                    }))
                }),
            );
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
                            .child(menu),
                    )
                    .with_priority(1),
                )
            })
    }

    /// The session's tools in labelled groups (review · session · agent),
    /// each tooltip naming its shortcut. v2 shows them on the header strip
    /// (`session_toolbar_v2`); this is what a v1 caller asking for the v2
    /// actions gets.
    pub(super) fn chat_actions_v2(
        &self,
        enabled: bool,
        _floating: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        self.session_toolbar_v2(enabled, cx)
    }

    /// v2 has no floating rail: the tools live on the header strip.
    pub(super) fn render_tool_rail_v2(&self, _enabled: bool, _cx: &mut Context<Self>) -> Div {
        div()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(kind: &str, status: &str) -> bg::Task {
        bg::Task {
            kind: Kind::of(kind),
            status: status.into(),
            ..Default::default()
        }
    }

    #[test]
    fn running_work_is_counted_in_words() {
        assert_eq!(running_words(&[], 0), None);
        assert_eq!(running_words(&[], 2).as_deref(), Some("2 background tasks"));
        let agents = [
            task("local_agent", "running"),
            task("local_agent", "running"),
        ];
        assert_eq!(running_words(&agents, 0).as_deref(), Some("2 subagents"));
        let one = [
            task("local_bash", "running"),
            task("local_agent", "completed"),
        ];
        assert_eq!(running_words(&one, 0).as_deref(), Some("1 shell task"));
        let mixed = [
            task("local_bash", "running"),
            task("local_agent", "running"),
        ];
        assert_eq!(
            running_words(&mixed, 0).as_deref(),
            Some("2 background tasks")
        );
        let done = [task("local_bash", "completed")];
        assert_eq!(running_words(&done, 0), None);
    }

    // r3-foundation
    #[test]
    fn the_red_chip_names_the_newest_unacknowledged_failure() {
        let failed = |id: &str, ended: i64| bg::Task {
            id: id.into(),
            ended_at: Some(ended),
            ..task("local_bash", "failed")
        };
        let tasks = [
            failed("old", 10),
            task("local_bash", "running"),
            failed("new", 20),
        ];
        let mut acked = std::collections::BTreeSet::new();
        let ids = |acked: &std::collections::BTreeSet<String>| {
            super::super::status::open_failures(&tasks, acked)
                .iter()
                .map(|t| t.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&acked), ["new", "old"]);
        acked.insert("new".to_owned());
        assert_eq!(ids(&acked), ["old"]);
        acked.insert("old".to_owned());
        assert!(ids(&acked).is_empty());
        // The neutral words never count a failure as running.
        assert_eq!(running_words(&tasks, 0).as_deref(), Some("1 shell task"));
    }
}
