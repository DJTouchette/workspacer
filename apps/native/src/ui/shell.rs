//! Design v2's shell: the attention-first sidebar (groups, two densities,
//! hover-only row actions, tree guides), its one-row footer, the collapsed
//! rail whose status ring leads, `g n` (next session that needs you) and the
//! keyboard cheat sheet. Everything here draws only under `Design::V2`; the
//! shipped sidebar lives in `sidebar.rs`.
use super::attention::{self, Bucket};
use super::primitives::{
    self, ButtonVariant, Intent, StatusBadge, StatusKind, kbd, status_indicator, tooltip_view,
};
use super::sidebar::SidebarRow;
use super::*;
use gpui::{AnyElement, AnyView, deferred};
use gpui_component::tooltip::Tooltip;
use wks_native::child_agents::ChildAgent;

/// Window-local state of the v2 shell.
pub(super) struct ShellUi {
    /// Folded groups, by `Bucket::index`. Ended starts folded.
    pub(super) folded: [bool; 4],
    /// The keyboard cheat sheet (`?`, Ctrl+/).
    pub(super) shortcuts: bool,
    /// The model, effort and access popover under the title's model chip.
    pub(super) model_popover: bool,
    /// Where the model chip was last drawn, so the popover hangs from it.
    pub(super) model_anchor: gpui::Bounds<gpui::Pixels>,
    /// "Finish setup" was dismissed for this run.
    pub(super) setup_dismissed: bool,
}

impl Default for ShellUi {
    fn default() -> Self {
        Self {
            folded: [false, false, false, true],
            shortcuts: false,
            model_popover: false,
            model_anchor: gpui::Bounds::default(),
            setup_dismissed: false,
        }
    }
}

/// One row of the v2 session list: a group's header, or a session/child row.
#[derive(Clone)]
pub(super) enum ShellRow {
    Group {
        bucket: Bucket,
        count: usize,
        folded: bool,
    },
    Item(SidebarRow),
}

/// What a session row shows as its status.
fn session_kind(this: &Workspace, session: &Session) -> StatusKind {
    if !this.view.connected {
        return StatusKind::Idle;
    }
    if this.paused(session) {
        return StatusKind::Paused;
    }
    match attention::bucket(session, false) {
        Bucket::NeedsYou => StatusKind::NeedsYou,
        Bucket::Working => StatusKind::Working,
        Bucket::Idle | Bucket::Ended => StatusKind::Idle,
    }
}

/// A provider-native child's status.
fn child_kind(child: &ChildAgent, connected: bool) -> StatusKind {
    if !connected {
        StatusKind::Idle
    } else if child.failed() {
        StatusKind::Failed
    } else if child.running() {
        StatusKind::Working
    } else if child.settled() {
        StatusKind::Done
    } else if wks_native::child_agents::provider_active(&child.status) {
        StatusKind::NeedsYou
    } else {
        StatusKind::Idle
    }
}

/// The plain words for what a session waiting on the user wants.
fn waiting_word(session: &Session) -> &'static str {
    if session.questions.is_some() || matches!(session.state.as_str(), "question" | "waiting_input")
    {
        "Question"
    } else {
        "Approval"
    }
}

/// A provider's mark in the neutral identity colour (v2 keeps brand colour
/// for the composer's model chip only).
pub(super) fn neutral_glyph(provider: &str, size: f32, p: Palette) -> AnyElement {
    let color = rgb(p.roles.provider);
    match provider {
        "claude" => gpui::svg()
            .path("brand/claude.svg")
            .size(px(size))
            .flex_shrink_0()
            .text_color(color)
            .into_any_element(),
        "codex" => gpui::svg()
            .path("brand/openai.svg")
            .size(px(size))
            .flex_shrink_0()
            .text_color(color)
            .into_any_element(),
        _ => Icon::new(IconName::Bot)
            .size(px(size))
            .flex_shrink_0()
            .text_color(color)
            .into_any_element(),
    }
}

/// The brand mark and the "Workspacer" wordmark beside it.
pub(super) fn wordmark(p: Palette) -> Div {
    let t = p.tokens;
    div()
        .flex()
        .items_center()
        .gap(px(t.space.md))
        .flex_shrink_0()
        .child(brand_mark(t.icon.lg, p))
        .child(
            div()
                .debug_selector(|| "sidebar-wordmark".into())
                .text_size(px(t.text.subheading))
                .font_weight(FontWeight(t.weight.semibold))
                .text_color(rgb(p.text))
                .child("Workspacer"),
        )
}

/// A small icon action inside a row, shown on hover.
fn row_action(id: SharedString, icon: IconName, tip: &'static str, p: &Palette) -> Stateful<Div> {
    let t = p.tokens;
    primitives::icon_button(id, icon, tip, ButtonVariant::Ghost, true, p)
        .size(px(t.icon.md + t.space.sm))
        .rounded(px(t.radius.sm))
}

/// A rail or hover card's rich tooltip: title, status, then the meta line.
fn hover_card(
    title: String,
    status: StatusBadge,
    meta: String,
    p: Palette,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |window, cx| {
        let (title, status, meta) = (title.clone(), status.clone(), meta.clone());
        Tooltip::element(move |_, _| {
            let t = p.tokens;
            div()
                .debug_selector(|| "rail-hover-card".into())
                .max_w(px(t.layout.menu_width))
                .flex()
                .flex_col()
                .gap(px(t.space.xs))
                .child(
                    div()
                        .text_size(px(t.text.body))
                        .font_weight(FontWeight(t.weight.semibold))
                        .text_color(rgb(p.text))
                        .child(title.clone()),
                )
                .child(status.clone().render(&p))
                .when(!meta.is_empty(), |d| {
                    d.child(
                        div()
                            .text_size(px(t.text.meta))
                            .text_color(rgb(p.roles.text_tertiary))
                            .child(meta.clone()),
                    )
                })
        })
        .build(window, cx)
    }
}

impl Workspace {
    /// The sidebar's session rows under v2: group headers, then each group's
    /// trees (folded groups show their header only; a search unfolds all).
    pub(super) fn shell_rows(&self, cx: &App) -> Vec<ShellRow> {
        let visible = self.visible_sessions(cx);
        let sessions = &self.view.sessions;
        let tree = wks_native::navigation::session_tree(sessions, &visible);
        let searching = !self.search.read(cx).value().is_empty();
        let bucket_of = |ix: usize| {
            let s = &sessions[ix];
            let own = attention::bucket(s, self.paused(s));
            // A provider-native child at work keeps its parent among the working.
            if own > Bucket::Working && self.native_children(s).iter().any(|c| c.running()) {
                Bucket::Working
            } else {
                own
            }
        };
        let groups = attention::group(&tree, bucket_of, |ix| {
            sessions[ix].telemetry.last_activity_ms
        });
        let mut rows = Vec::new();
        for (bucket, members) in groups {
            let folded = self.shell.folded[bucket.index()] && !searching;
            let count = members
                .iter()
                .filter(|&&(ix, _)| bucket_of(ix) == bucket)
                .count()
                .max(1);
            rows.push(ShellRow::Group {
                bucket,
                count,
                folded,
            });
            if folded {
                continue;
            }
            for (index, depth) in members {
                rows.push(ShellRow::Item(SidebarRow::Session { index, depth }));
                let parent = &sessions[index];
                for child in self.native_children(parent).iter().filter(|child| {
                    !self.provider_cleared(&parent.id, child)
                        || self
                            .view
                            .child
                            .as_ref()
                            .is_some_and(|t| t.parent == parent.id && t.agent == child.id)
                }) {
                    rows.push(ShellRow::Item(SidebarRow::Provider {
                        parent: parent.id.clone(),
                        child: Box::new(child.clone()),
                        depth: depth + 1,
                    }));
                }
            }
        }
        rows
    }

    /// Sessions in the order the list shows them (what j/k, Enter in the
    /// search and `g n` walk): v2's grouped order, else the tree's.
    pub(super) fn nav_sessions(&self, cx: &App) -> Vec<usize> {
        if !self.design().is_v2() {
            return self.visible_sessions(cx);
        }
        self.shell_rows(cx)
            .into_iter()
            .filter_map(|row| match row {
                ShellRow::Item(SidebarRow::Session { index, .. }) => Some(index),
                _ => None,
            })
            .collect()
    }

    /// Where session `index` (into [`Self::nav_sessions`]) sits in the v2 list.
    pub(super) fn shell_position(&self, index: usize, cx: &App) -> usize {
        let order = self.nav_sessions(cx);
        self.shell_rows(cx)
            .iter()
            .position(|row| {
                matches!(row, ShellRow::Item(SidebarRow::Session { index: ix, .. })
                    if Some(ix) == order.get(index))
            })
            .unwrap_or(index)
    }

    /// Sessions waiting on the user, among those listed.
    pub(super) fn needing_you(&self, cx: &App) -> Vec<usize> {
        self.visible_sessions(cx)
            .into_iter()
            .filter(|&ix| attention::needs_you(&self.view.sessions[ix]))
            .collect()
    }

    /// `g n` / Alt+N: open the next session that needs you. (v1 keeps `g n`
    /// for Scratch.)
    pub(super) fn next_needs_you(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.design().is_v2() {
            self.open_feature(Screen::Scratch, window, cx);
            return;
        }
        // A search could hide the one waiting; the jump looks at everything.
        if !self.search.read(cx).value().is_empty() {
            self.search
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        let needing = self.needing_you(cx);
        let order = self.nav_sessions(cx);
        let current = self
            .navigation_selected
            .as_ref()
            .or(self.view.selected.as_ref())
            .and_then(|id| self.view.sessions.iter().position(|s| &s.id == id));
        let Some(next) = attention::next_needing(&order, &needing, current) else {
            self.extras.notice = "Nothing needs you right now.".into();
            cx.notify();
            return;
        };
        // Its group may be folded (a paused tree, say): open it.
        self.shell.folded[Bucket::NeedsYou.index()] = false;
        if let Some(position) = self.nav_sessions(cx).iter().position(|&ix| ix == next) {
            self.sidebar_scroll.scroll_to_item(
                self.shell_position(position, cx),
                gpui::ScrollStrategy::Center,
            );
        }
        self.new_session = false;
        self.screen = Screen::Conversation;
        let id = self.view.sessions[next].id.clone();
        if Some(next) != current {
            self.command(Command::Select(id), cx);
        }
        cx.notify();
    }

    fn toggle_group(&mut self, bucket: Bucket, cx: &mut Context<Self>) {
        let folded = &mut self.shell.folded[bucket.index()];
        *folded = !*folded;
        cx.notify();
    }

    /// The model the workspace mostly runs (the provider default's most
    /// common display name): rows name a model only when it differs.
    fn usual_model(&self) -> (String, String) {
        let provider = self.settings.default_provider.id().to_owned();
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for s in self
            .view
            .sessions
            .iter()
            .filter(|s| s.provider_id() == provider)
        {
            *counts.entry(s.display_model()).or_default() += 1;
        }
        let model = counts
            .into_iter()
            .max_by_key(|(_, n)| *n)
            .map(|(m, _)| m)
            .unwrap_or_default();
        (provider, model)
    }

    fn row_height(&self) -> f32 {
        let t = self.tokens();
        if self.settings.sidebar_compact {
            t.layout.sidebar_row_compact
        } else {
            t.layout.sidebar_row
        }
    }

    // ── The sidebar ─────────────────────────────────────────────────────

    /// The v2 sidebar: wordmark and actions, search, grouped sessions and
    /// the one-row footer. Docked, or in the rail's flyout.
    pub(super) fn render_sidebar_v2(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let t = p.tokens;
        let rows = self.shell_rows(cx);
        let filtered = !self.search.read(cx).value().is_empty() || self.project_filter.is_some();
        let empty = rows.is_empty();
        let normal = self.settings.vim_navigation && self.focus.is_focused(window);
        let header =
            chrome::drag_region(div())
                .debug_selector(|| "sidebar-drag-region".into())
                .h(px(t.layout.footer_height))
                .px(px(t.space.lg))
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap(px(t.space.md))
                .child(wordmark(p))
                .child(div().flex_1())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(t.space.xxs))
                        .flex_shrink_0()
                        .occlude()
                        .child(
                            primitives::icon_button_with_shortcut(
                                "new-session",
                                IconName::Plus,
                                "New session",
                                Some("Ctrl N"),
                                ButtonVariant::Ghost,
                                !self.demo,
                                &p,
                            )
                            .debug_selector(|| "new-session-button".into())
                            .when(!self.demo, |d| {
                                d.on_click(cx.listener(|this, _, window, cx| {
                                    this.show_new_session(window, cx)
                                }))
                            }),
                        )
                        .child(self.nav_icon(
                            "nav-projects",
                            IconName::Folder,
                            "Projects",
                            Some("Ctrl P"),
                            Screen::Projects,
                            cx,
                        ))
                        .child(self.nav_icon(
                            "nav-history",
                            IconName::BookOpen,
                            "Session history",
                            self.settings.vim_navigation.then_some("g h"),
                            Screen::Recent,
                            cx,
                        ))
                        .child(self.sidebar_toggle_v2(cx)),
                );
        let search_empty = self.search.read(cx).value().is_empty();
        let search = div().px(px(t.space.md)).pb(px(t.space.md)).child(
            div()
                .key_context("SessionSearch")
                .debug_selector(|| "sidebar-search-field".into())
                .h(px(t.layout.control_height_sm + t.space.xs))
                .px(px(t.space.md))
                .flex()
                .items_center()
                .gap(px(t.space.sm))
                .rounded(px(t.radius.control))
                .bg(primitives::tonal(&p, t.wash.rest))
                .child(
                    Icon::new(IconName::Search)
                        .size(px(t.icon.sm))
                        .text_color(rgb(p.roles.text_tertiary)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(Input::new(&self.search).appearance(false)),
                )
                .when(search_empty, |d| d.child(kbd("Ctrl K", &p))),
        );
        let list = uniform_list(
            "sessions",
            rows.len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|ix| this.render_shell_row(&rows[ix], ix, cx))
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(self.sidebar_scroll.clone())
        .flex_1()
        .min_h_0()
        .px(px(t.space.sm))
        .pb(px(t.layout.control_height));
        let mode = normal.then(|| self.render_mode_chip(cx));
        div()
            .id("sidebar")
            .relative()
            .w(px(wks_native::navigation::sidebar_width(
                self.settings.sidebar_width,
                unzoom(window.viewport_size().width),
            )))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(p.base))
            .rounded(px(t.radius.panel))
            .border(px(t.stroke.hairline))
            .border_color(rgb(p.border))
            .overflow_hidden()
            .flex()
            .flex_col()
            .debug_selector(|| "session-sidebar".into())
            .child(header)
            .child(search)
            .children(self.render_cleanup_banner(cx))
            .when(self.view.power_paused, |d| {
                d.child(
                    div()
                        .px(px(t.space.lg))
                        .pb(px(t.space.md))
                        .flex()
                        .flex_col()
                        .gap(px(t.space.sm))
                        .text_size(px(t.text.meta))
                        .text_color(rgb(p.warning))
                        .child(self.view.notice.clone())
                        .when(self.view.can_resume_power_pause, |d| {
                            d.child(
                                primitives::button(
                                    "resume-power-pause",
                                    "Reconnect",
                                    Some(IconName::Redo),
                                    ButtonVariant::Secondary,
                                    primitives::ButtonSize::Small,
                                    true,
                                    &p,
                                )
                                .on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.command(Command::Refresh, cx)
                                    }),
                                ),
                            )
                        }),
                )
            })
            .when(!self.ui_bus.notice.is_empty(), |d| {
                d.child(self.render_ui_notice(cx))
            })
            .children(self.render_filter_v2(cx))
            .when(empty, |d| d.child(self.render_list_empty_v2(filtered, cx)))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(list)
                    .children(mode),
            )
            .children(self.render_finish_setup(cx))
            .child(self.render_footer_v2(window, cx))
            .child(self.sidebar_resize_handle(cx))
            .into_any_element()
    }

    /// A header or footer icon that opens a page, highlighted while it is open.
    fn nav_icon(
        &self,
        id: &'static str,
        icon: IconName,
        tip: &'static str,
        shortcut: Option<&'static str>,
        screen: Screen,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.palette();
        let open = !self.new_session && self.screen == screen;
        primitives::icon_button_with_shortcut(
            id,
            icon,
            tip,
            shortcut,
            ButtonVariant::Ghost,
            true,
            &p,
        )
        .debug_selector(move || id.into())
        .when(open, |d| d.bg(rgb(p.selected)).text_color(rgb(p.text)))
        .on_click(cx.listener(move |this, _, window, cx| {
            if matches!(screen, Screen::Projects | Screen::Settings) {
                this.show_screen(screen, window, cx)
            } else {
                this.open_feature(screen, window, cx)
            }
        }))
    }

    fn sidebar_toggle_v2(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let p = self.palette();
        let (icon, tip) = if self.sidebar_collapsed {
            (IconName::PanelLeftOpen, "Expand sidebar")
        } else {
            (IconName::PanelLeftClose, "Collapse sidebar")
        };
        primitives::icon_button("toggle-sidebar", icon, tip, ButtonVariant::Ghost, true, &p)
            .debug_selector(|| "sidebar-toggle".into())
            .on_click(cx.listener(|this, _, _, cx| {
                this.sidebar_collapsed = !this.sidebar_collapsed;
                cx.notify();
            }))
    }

    /// The filtered project, above the list: its mark and name, waiting
    /// scratch, a new session there, and the way back to every project.
    fn render_filter_v2(&self, cx: &mut Context<Self>) -> Option<Div> {
        let filter = self.project_filter.clone()?;
        let p = self.palette();
        let t = p.tokens;
        let group = self.filter_group();
        let folders = self.filter_folders();
        let (known, title) = match &group {
            Some(group) => (Some(group.as_known()), group.name.clone()),
            None => (
                self.known_projects()
                    .into_iter()
                    .find(|k| wks_native::projects::same_dir(&k.path, &filter)),
                self.project_name(&filter),
            ),
        };
        let mark_path = known.as_ref().map_or(filter.clone(), |k| k.path.clone());
        let pending = self.pending_in(&folders);
        let spawn_in = folders.first().cloned();
        let can_spawn = self.view.connected && !self.demo && spawn_in.is_some();
        Some(
            div()
                .debug_selector(|| "sidebar-project-filter".into())
                .mx(px(t.space.md))
                .mb(px(t.space.md))
                .px(px(t.space.md))
                .h(px(t.layout.control_height))
                .rounded(px(t.radius.control))
                .border(px(t.stroke.hairline))
                .border_color(rgb(p.border))
                .flex()
                .items_center()
                .gap(px(t.space.md))
                .child(self.project_mark(known.as_ref(), &mark_path, t.icon.lg))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(t.text.body))
                        .font_weight(FontWeight(t.weight.medium))
                        .text_color(rgb(p.text))
                        .child(title),
                )
                .when(pending > 0, |d| {
                    let target = filter.clone();
                    d.child(
                        div()
                            .id("sidebar-filter-pending")
                            .debug_selector(|| "sidebar-filter-pending".into())
                            .cursor_pointer()
                            .tooltip(tooltip_view(
                                "Scratch files waiting for you to run them",
                                None,
                                &p,
                            ))
                            .child(primitives::pill(
                                format!("{pending} waiting"),
                                Intent::Warning,
                                &p,
                            ))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.extras.scratch_preset = Some(target.clone());
                                this.open_feature(Screen::Scratch, window, cx)
                            })),
                    )
                })
                .child(
                    primitives::icon_button(
                        "new-agent-in-filter",
                        IconName::Plus,
                        "New session in this project",
                        ButtonVariant::Ghost,
                        can_spawn,
                        &p,
                    )
                    .debug_selector(|| "new-agent-in-filter".into())
                    .when_some(spawn_in.filter(|_| can_spawn), |d, path| {
                        d.on_click(cx.listener(move |this, _, window, cx| {
                            let filter = this.project_filter.clone();
                            this.new_agent_in_project(path.clone(), window, cx);
                            this.project_filter = filter;
                        }))
                    }),
                )
                .child(
                    primitives::icon_button(
                        "all-projects",
                        IconName::Close,
                        "Show every project",
                        ButtonVariant::Ghost,
                        true,
                        &p,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.project_filter = None;
                        cx.notify();
                    })),
                ),
        )
    }

    fn render_list_empty_v2(&self, filtered: bool, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let (title, body) = if !self.view.connected {
            (
                self.connection_copy().label,
                self.connection_copy().description,
            )
        } else if self.view.sessions_loading || self.archive_loading() {
            ("Loading sessions…", "Checking your workspace for sessions.")
        } else if filtered {
            (
                "No matching sessions",
                "Try another search or clear the filters.",
            )
        } else {
            ("No sessions yet", "Start a new session to begin.")
        };
        div()
            .px(px(t.space.xl))
            .py(px(t.space.lg))
            .flex()
            .flex_col()
            .gap(px(t.space.xs))
            .child(
                div()
                    .text_size(px(t.text.body))
                    .font_weight(FontWeight(t.weight.medium))
                    .text_color(rgb(p.text))
                    .child(title),
            )
            .child(
                div()
                    .text_size(px(t.text.meta))
                    .text_color(rgb(p.roles.text_secondary))
                    .child(body),
            )
            .when(filtered, |d| {
                d.child(
                    primitives::button(
                        "clear-session-filters",
                        "Clear filters",
                        None,
                        ButtonVariant::Secondary,
                        primitives::ButtonSize::Small,
                        true,
                        &p,
                    )
                    .debug_selector(|| "clear-session-filters".into())
                    .mt(px(t.space.md))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.project_filter = None;
                        this.search
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        cx.notify();
                    })),
                )
            })
    }

    fn render_shell_row(&self, row: &ShellRow, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        match row {
            ShellRow::Group {
                bucket,
                count,
                folded,
            } => self.render_group_header(*bucket, *count, *folded, cx),
            ShellRow::Item(SidebarRow::Session { index, depth }) => {
                self.render_session_v2(*index, *depth, ix, cx)
            }
            ShellRow::Item(SidebarRow::Provider {
                parent,
                child,
                depth,
            }) => self.render_child_v2(parent, child, *depth, ix, cx),
        }
    }

    fn render_group_header(
        &self,
        bucket: Bucket,
        count: usize,
        folded: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let t = p.tokens;
        let needs = bucket == Bucket::NeedsYou;
        let color = if needs {
            p.warning
        } else {
            p.roles.text_tertiary
        };
        let slug = bucket.slug();
        div()
            .h(px(self.row_height()))
            .px(px(t.space.xs))
            .pb(px(t.space.xxs))
            .flex()
            .items_end()
            .gap(px(t.space.sm))
            .child(
                chrome::interactive_control(
                    div().id(SharedString::from(format!("sidebar-group-{slug}"))),
                    p,
                    true,
                )
                .debug_selector(move || format!("sidebar-group-{slug}"))
                .h(px(t.layout.control_height_sm - t.space.xs))
                .px(px(t.space.xs))
                .rounded(px(t.radius.sm))
                .flex()
                .items_center()
                .gap(px(t.space.xs))
                .text_size(px(t.text.meta))
                .font_weight(FontWeight(t.weight.semibold))
                .text_color(rgb(color))
                .hover(move |s| s.bg(rgb(p.roles.hover)))
                .tooltip(tooltip_view(
                    if folded {
                        "Show this group"
                    } else {
                        "Fold this group"
                    },
                    None,
                    &p,
                ))
                .child(
                    Icon::new(if folded {
                        IconName::ChevronRight
                    } else {
                        IconName::ChevronDown
                    })
                    .size(px(t.icon.xs))
                    .text_color(rgb(p.roles.text_tertiary)),
                )
                .child(bucket.label())
                .when(!needs, |d| {
                    d.child(
                        div()
                            .font_weight(FontWeight(t.weight.regular))
                            .text_color(rgb(p.roles.text_tertiary))
                            .child(count.to_string()),
                    )
                })
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_group(bucket, cx))),
            )
            .when(needs, |d| {
                d.child(
                    chrome::interactive_control(div().id("sidebar-next-needs-you"), p, true)
                        .debug_selector(|| "sidebar-next-needs-you".into())
                        .mb(px(t.space.xs))
                        .rounded(px(t.radius.full))
                        .tooltip(tooltip_view(
                            "Open the next session that needs you",
                            Some(if self.settings.vim_navigation {
                                "g n"
                            } else {
                                "Alt N"
                            }),
                            &p,
                        ))
                        .child(primitives::badge(count.to_string(), Intent::Warning, &p))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.next_needs_you(window, cx)),
                        ),
                )
            })
            .into_any_element()
    }

    /// The left edge of a row at `depth`, and the 1px tree guide that ties a
    /// nested row to its parent's status mark.
    fn row_frame(&self, depth: usize) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let indent = t.space.xl;
        let depth = depth.min(6) as f32;
        div()
            .relative()
            .h(px(self.row_height()))
            .pl(px(depth * indent))
            .py(px(t.stroke.hairline))
            .when(depth > 0., |d| {
                // Under the parent's status box: its row's left padding,
                // the control's focus border and half the box.
                let x =
                    (depth - 1.) * indent + t.space.md + t.stroke.focus + t.layout.status_box / 2.;
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(px(x))
                        .w(px(t.stroke.hairline))
                        .bg(rgb(p.border)),
                )
            })
    }

    fn render_session_v2(
        &self,
        index: usize,
        depth: usize,
        ix: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let t = p.tokens;
        let compact = self.settings.sidebar_compact;
        let session = &self.view.sessions[index];
        let id = session.id.clone();
        let active = self
            .navigation_selected
            .as_ref()
            .or(self.view.selected.as_ref())
            == Some(&id)
            && (self.navigation_selected.is_some() || self.view.child.is_none());
        let title = self.session_title(session);
        let kind = session_kind(self, session);
        let project = self.project_name(&session.cwd);
        let (usual_provider, usual_model) = self.usual_model();
        let model = session.display_model();
        let model = (session.provider_id() != usual_provider || model != usual_model).then(|| {
            if model.is_empty() {
                "Default model".to_owned()
            } else {
                model
            }
        });
        let job = !session.job_id.is_empty();
        let now = super::cache::now();
        let cold =
            super::cache::cold(session, now).map(|cold| super::cache::marker_tooltip(cold, now));
        let age = session
            .telemetry
            .last_activity_ms
            .map(|at| wks_native::format::age(at, timing::now_ms()));
        let (status_label, _) = if self.view.connected {
            self.status_of(session, p)
        } else {
            ("Offline", p.muted)
        };
        let details = {
            let mut lines = vec![title.clone(), session.cwd.clone()];
            let provider = match session.provider_id() {
                "claude" => "Claude",
                "codex" => "Codex",
                "" => "Agent",
                other => other,
            };
            let model = session.display_model();
            lines.push(format!(
                "{provider} · {} · {status_label}",
                if model.is_empty() {
                    "Default model"
                } else {
                    &model
                }
            ));
            if job {
                lines.push("Started by a job · its runs are under Jobs → Runs".into());
            }
            if let Some(cold) = &cold {
                lines.push(cold.clone());
            }
            lines.join("\n") + super::sidebar::child_limit_note(session)
        };
        // Nested, finished children offer Clear (this device's view) in place
        // of Archive; a parent with finished children can clear them all.
        let finished_child = depth > 0 && wks_native::child_agents::session_finished(session);
        let clearable = self.clearable_children(session);
        let group: SharedString = format!("sidebar-row-{ix}").into();
        let mut actions: Vec<AnyElement> = Vec::new();
        if !clearable.is_empty() {
            actions.push(
                row_action(
                    format!("clear-finished-{}", session.id).into(),
                    IconName::EyeOff,
                    super::sidebar::CLEAR_FINISHED,
                    &p,
                )
                .debug_selector(move || format!("sidebar-clear-finished-{ix}"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.clear_children(clearable.clone(), cx);
                }))
                .into_any_element(),
            );
        }
        actions.push(if finished_child {
            let clear = super::sidebar::session_clear(session);
            row_action(
                format!("clear-child-{}", session.id).into(),
                IconName::EyeOff,
                super::sidebar::CLEAR_CHILD,
                &p,
            )
            .debug_selector(move || format!("sidebar-clear-{ix}"))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.clear_children(vec![clear.clone()], cx);
            }))
            .into_any_element()
        } else {
            let target = id.clone();
            row_action(
                format!("archive-sidebar-{}", session.id).into(),
                IconName::Inbox,
                "Archive · hides it in every client, keeps it running · restore from Session history → Archived",
                &p,
            )
            .debug_selector(move || format!("sidebar-archive-{ix}"))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.toggle_archive(&target, cx);
            }))
            .into_any_element()
        });
        let trailing = self.row_trailing(group.clone(), age, actions);
        // The waiting word (Approval/Question), a pause, or nothing: the
        // group already names the state.
        let note = match kind {
            StatusKind::NeedsYou => Some((waiting_word(session).to_owned(), p.warning)),
            StatusKind::Paused => Some(("Paused".to_owned(), p.roles.text_tertiary)),
            _ if !self.view.connected => Some(("Offline".to_owned(), p.roles.text_tertiary)),
            _ => None,
        };
        let meta = div()
            .flex()
            .items_center()
            .gap(px(t.space.xs))
            .min_w_0()
            .text_size(px(t.text.meta))
            .text_color(rgb(p.roles.text_tertiary))
            .child(div().min_w_0().flex_shrink().truncate().child(project))
            .when_some(model.filter(|_| !compact), |d, model| {
                d.child("·").child(
                    div()
                        .debug_selector(move || format!("sidebar-model-{ix}"))
                        .flex_shrink_0()
                        .max_w(gpui::relative(0.5))
                        .truncate()
                        .child(model),
                )
            })
            .when(job && !compact, |d| {
                d.child("·").child(
                    div()
                        .debug_selector(move || format!("sidebar-job-{ix}"))
                        .flex_shrink_0()
                        .child("Job"),
                )
            })
            .when(cold.is_some(), |d| {
                d.child(
                    super::cache::marker(p, t.icon.xs, false)
                        .debug_selector(move || format!("sidebar-cold-{ix}")),
                )
            })
            .when_some(note.clone().filter(|_| !compact), |d, (note, color)| {
                d.child("·")
                    .child(div().flex_shrink_0().text_color(rgb(color)).child(note))
            });
        let title_el = div()
            .flex_1()
            .min_w_0()
            .truncate()
            .text_size(px(t.text.body))
            .font_weight(FontWeight(if active {
                t.weight.semibold
            } else {
                t.weight.medium
            }))
            .text_color(rgb(if kind == StatusKind::Idle && session.stopped() {
                p.roles.text_secondary
            } else {
                p.text
            }))
            .child(title);
        let control = chrome::interactive_control(
            div().id(SharedString::from(format!("session-{}", session.id))),
            p,
            true,
        )
        .debug_selector(move || format!("sidebar-session-{ix}"))
        .group(group)
        .size_full()
        .px(px(t.space.md))
        .rounded(px(t.radius.control))
        .overflow_hidden()
        .flex()
        .items_center()
        .gap(px(t.space.sm))
        .when(active, |d| d.bg(rgb(p.selected)))
        .when(!active, |d| d.hover(move |s| s.bg(rgb(p.roles.hover))))
        .tooltip(tooltip_view(details, None, &p))
        .child(self.row_body(kind, title_el, meta, trailing, true))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.new_session = false;
            this.screen = Screen::Conversation;
            this.command(Command::Select(id.clone()), cx)
        }));
        self.row_frame(depth).child(control).into_any_element()
    }

    /// A row's content. Comfortable: status and title on the first line,
    /// the meta line under the title. Compact: one line, with the meta
    /// (project) after the title when `meta_inline`.
    fn row_body(
        &self,
        kind: StatusKind,
        title: Div,
        meta: Div,
        trailing: Div,
        meta_inline: bool,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let title = title.line_height(px(t.text.body + t.space.xs));
        if self.settings.sidebar_compact {
            return div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(t.space.sm))
                .child(status_indicator(kind, &p))
                .child(title)
                .when(meta_inline, |d| {
                    d.child(meta.flex_shrink().max_w(gpui::relative(0.4)))
                })
                .child(trailing);
        }
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
                    .gap(px(t.space.sm))
                    .child(status_indicator(kind, &p))
                    .child(title)
                    .child(trailing),
            )
            .child(
                meta.pl(px(t.layout.status_box + t.space.sm))
                    .line_height(px(t.text.meta + t.space.xxs)),
            )
    }

    /// A row's right edge: its age at rest, its actions on hover (hidden
    /// actions take no clicks).
    fn row_trailing(
        &self,
        group: SharedString,
        age: Option<String>,
        actions: Vec<AnyElement>,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let slot = (t.icon.md + t.space.sm) * actions.len() as f32;
        div()
            .relative()
            .flex_shrink_0()
            .min_w(px(slot))
            .h(px(t.icon.md + t.space.sm))
            .flex()
            .items_center()
            .justify_end()
            .when_some(age, |d, age| {
                d.child(
                    div()
                        .text_size(px(t.text.meta))
                        .text_color(rgb(p.roles.text_tertiary))
                        .group_hover(group.clone(), |s| s.invisible())
                        .child(age),
                )
            })
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .flex()
                    .items_center()
                    .invisible()
                    .group_hover(group, |s| s.visible())
                    .children(actions),
            )
    }

    fn render_child_v2(
        &self,
        parent: &str,
        child: &ChildAgent,
        depth: usize,
        ix: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let t = p.tokens;
        let connected = self.view.connected;
        let viewing = self.view.child.as_ref().is_some_and(|v| {
            v.parent == parent && v.agent == child.id && self.navigation_selected.is_none()
        });
        let title = [&child.description, &child.label]
            .into_iter()
            .find(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| "Subagent".into());
        let kind = child_kind(child, connected);
        let model = wks_native::model::model_display_name(&child.model);
        let group: SharedString = format!("sidebar-row-{ix}").into();
        let (parent, agent) = (parent.to_owned(), child.id.clone());
        let mut actions = Vec::new();
        if child.settled() {
            let (parent, child) = (parent.clone(), child.clone());
            actions.push(
                row_action(
                    format!("clear-native-{parent}-{}", child.id).into(),
                    IconName::EyeOff,
                    super::sidebar::CLEAR_SUBAGENT,
                    &p,
                )
                .debug_selector(move || format!("sidebar-clear-provider-{ix}"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.clear_children(vec![super::sidebar::provider_clear(&parent, &child)], cx);
                }))
                .into_any_element(),
            );
        }
        let meta = div()
            .debug_selector(move || format!("sidebar-child-model-{ix}"))
            .min_w_0()
            .truncate()
            .text_size(px(t.text.meta))
            .text_color(rgb(p.roles.text_tertiary))
            .child(if model.is_empty() {
                "Subagent".to_owned()
            } else {
                format!("Subagent · {model}")
            });
        let title_el = div()
            .flex_1()
            .min_w_0()
            .truncate()
            .text_size(px(t.text.body))
            .font_weight(FontWeight(t.weight.medium))
            .text_color(rgb(if child.settled() {
                p.roles.text_secondary
            } else {
                p.text
            }))
            .child(title);
        let trailing = self.row_trailing(group.clone(), None, actions);
        let control = chrome::interactive_control(
            div().id(SharedString::from(format!(
                "sidebar-native-{parent}-{agent}"
            ))),
            p,
            connected,
        )
        .debug_selector(move || format!("sidebar-provider-{ix}"))
        .group(group)
        .size_full()
        .px(px(t.space.md))
        .rounded(px(t.radius.control))
        .overflow_hidden()
        .flex()
        .items_center()
        .gap(px(t.space.sm))
        .when(viewing, |d| d.bg(rgb(p.selected)))
        .when(!viewing && connected, |d| {
            d.hover(move |s| s.bg(rgb(p.roles.hover)))
        })
        .tooltip(tooltip_view("Subagent · open its conversation", None, &p))
        .child(self.row_body(kind, title_el, meta, trailing, false))
        .when(connected, |d| {
            d.on_click(cx.listener(move |this, _, _, cx| {
                this.open_sidebar_child(&parent, &agent, cx);
            }))
        });
        self.row_frame(depth).child(control).into_any_element()
    }

    /// The vim mode, only while in Normal mode (typing is the default):
    /// a quiet chip over the list's corner that opens the cheat sheet.
    fn render_mode_chip(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let p = self.palette();
        let t = p.tokens;
        div()
            .id("sidebar-mode")
            .debug_selector(|| "sidebar-mode".into())
            .absolute()
            .right(px(t.space.md))
            .bottom(px(t.space.md))
            .occlude()
            .px(px(t.space.sm))
            .py(px(t.space.xxs))
            .rounded(px(t.radius.full))
            .bg(rgb(p.roles.raised))
            .border(px(t.stroke.hairline))
            .border_color(rgb(p.roles.border_strong))
            .shadow(primitives::elevation(&p, t.elevation.raised))
            .flex()
            .items_center()
            .gap(px(t.space.xs))
            .text_size(px(t.text.overline))
            .font_weight(FontWeight(t.weight.semibold))
            .text_color(rgb(p.roles.text_secondary))
            .cursor_pointer()
            .child("Normal")
            .child(kbd("?", &p))
            .tooltip(tooltip_view(
                "Keyboard navigation · press i to type, ? for every key",
                None,
                &p,
            ))
            .on_click(cx.listener(|this, _, _, cx| {
                this.shell.shortcuts = true;
                cx.notify();
            }))
    }

    /// Install and sign-in prompts folded into one dismissible line.
    fn render_finish_setup(&self, cx: &mut Context<Self>) -> Option<Div> {
        if self.shell.setup_dismissed {
            return None;
        }
        let all = self.usage_accounts().map(|(a, _)| a).unwrap_or_default();
        let notes = self.usage_notes(&all);
        if notes.is_empty() {
            return None;
        }
        let p = self.palette();
        let t = p.tokens;
        let tip = notes
            .iter()
            .map(|(_, text)| text.clone())
            .collect::<Vec<_>>()
            .join(" · ");
        Some(
            div()
                .px(px(t.space.md))
                .pb(px(t.space.xs))
                .flex()
                .items_center()
                .gap(px(t.space.xxs))
                .child(
                    primitives::list_row("sidebar-finish-setup", false, true, &p)
                        .debug_selector(|| "sidebar-finish-setup".into())
                        .flex_1()
                        .min_h(px(t.layout.control_height_sm))
                        .text_size(px(t.text.meta))
                        .text_color(rgb(p.roles.text_secondary))
                        .tooltip(tooltip_view(tip, None, &p))
                        .child(
                            Icon::new(IconName::CircleUser)
                                .size(px(t.icon.sm))
                                .text_color(rgb(p.accent)),
                        )
                        .child(format!("Finish setup ({})", notes.len()))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_feature(Screen::Setup, window, cx)
                        })),
                )
                .child(
                    primitives::icon_button(
                        "sidebar-finish-setup-dismiss",
                        IconName::Close,
                        "Hide until next launch",
                        ButtonVariant::Ghost,
                        true,
                        &p,
                    )
                    .debug_selector(|| "sidebar-finish-setup-dismiss".into())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.shell.setup_dismissed = true;
                        cx.notify();
                    })),
                ),
        )
    }

    /// One row: Jobs, Hosts, Scratch, compact usage, then Settings. Every
    /// item is an icon with a labelled tooltip; a lost connection is named.
    fn render_footer_v2(&self, _window: &Window, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let vim = self.settings.vim_navigation;
        let settings = self
            .nav_icon(
                "nav-settings",
                IconName::Settings,
                if self.update_available() {
                    "Settings · an update is available"
                } else {
                    "Settings"
                },
                Some("Ctrl ,"),
                Screen::Settings,
                cx,
            )
            .relative()
            .when(self.update_available(), |d| {
                d.debug_selector(|| "update-pill".into()).child(
                    div()
                        .absolute()
                        .top(px(t.space.xs))
                        .right(px(t.space.xs))
                        .size(px(t.layout.status_dot))
                        .rounded(px(t.radius.full))
                        .bg(rgb(p.accent)),
                )
            });
        let scratch = self
            .nav_icon(
                "nav-scratch",
                IconName::FolderClosed,
                "Scratch files",
                vim.then_some("g x"),
                Screen::Scratch,
                cx,
            )
            .relative()
            .when_some(Some(self.pending_total()).filter(|n| *n > 0), |d, total| {
                d.child(
                    div()
                        .absolute()
                        .top(-px(t.space.xs))
                        .right(-px(t.space.xs))
                        .debug_selector(|| "scratch-pending-badge".into())
                        .child(primitives::badge(
                            if total > 99 {
                                "99+".to_owned()
                            } else {
                                total.to_string()
                            },
                            Intent::Warning,
                            &p,
                        )),
                )
            });
        div()
            .debug_selector(|| "sidebar-footer".into())
            .h(px(t.layout.footer_height))
            .flex_shrink_0()
            .px(px(t.space.md))
            .border_t(px(t.stroke.hairline))
            .border_color(rgb(p.border))
            .flex()
            .items_center()
            .gap(px(t.space.xxs))
            .child(self.nav_icon(
                "nav-jobs",
                IconName::Calendar,
                "Jobs",
                vim.then_some("g j"),
                Screen::Jobs,
                cx,
            ))
            .child(self.nav_icon(
                "nav-hosts",
                IconName::LayoutDashboard,
                "Hosts",
                vim.then_some("g o"),
                Screen::Hosts,
                cx,
            ))
            .child(scratch)
            .child(div().flex_1().min_w_0())
            .map(|d| match self.footer_connection_label() {
                Some(label) => d.child(
                    div()
                        .debug_selector(|| "sidebar-connection-status".into())
                        .flex()
                        .items_center()
                        .gap(px(t.space.xs))
                        .text_size(px(t.text.meta))
                        .text_color(rgb(p.roles.text_secondary))
                        .when(!self.view.connected, |d| {
                            d.child(status_indicator(StatusKind::NeedsYou, &p))
                        })
                        .child(label),
                ),
                None => d.children(self.render_usage_compact(cx)),
            })
            .child(settings)
    }

    fn sidebar_resize_handle(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let p = self.palette();
        let t = p.tokens;
        div()
            .id("sidebar-resize")
            .debug_selector(|| "sidebar-resize".into())
            .absolute()
            .right_0()
            .top_0()
            .bottom_0()
            .w(px(t.space.sm))
            .cursor(gpui::CursorStyle::ResizeLeftRight)
            .hover(move |s| s.bg(rgb(p.border)))
            .tooltip(tooltip_view(
                "Drag to resize · double-click to reset",
                None,
                &p,
            ))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    if event.click_count == 2 {
                        this.settings.sidebar_width = this.tokens().layout.sidebar_width;
                        this.sidebar_drag = None;
                        this.save_settings(cx);
                    } else {
                        let width = wks_native::navigation::sidebar_width(
                            this.settings.sidebar_width,
                            unzoom(window.viewport_size().width),
                        );
                        this.sidebar_drag = Some((event.position.x, width));
                        cx.notify();
                    }
                    cx.stop_propagation();
                }),
            )
            .child(self.sidebar_drag_watch(cx))
    }

    // ── The rail ────────────────────────────────────────────────────────

    /// The collapsed sidebar: each session's status ring leads (amber needs
    /// you, a breathing blue ring is working), in the list's grouped order,
    /// with a hover card naming it.
    pub(super) fn render_rail_v2(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette();
        let t = p.tokens;
        let rows: Vec<SidebarRow> = self
            .shell_rows(cx)
            .into_iter()
            .filter_map(|row| match row {
                ShellRow::Item(item) => Some(item),
                ShellRow::Group { .. } => None,
            })
            .collect();
        let needing = self.needing_you(cx).len();
        let vim = self.settings.vim_navigation;
        div()
            .id("sidebar")
            .debug_selector(|| "session-sidebar".into())
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| this.rail_hover(*hovered, cx)))
            .w(px(t.layout.rail_width))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(p.base))
            .rounded(px(t.radius.panel))
            .border(px(t.stroke.hairline))
            .border_color(rgb(p.border))
            .overflow_hidden()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(t.space.xs))
            .py(px(t.space.md))
            .child(
                chrome::drag_region(div())
                    .debug_selector(|| "sidebar-drag-region".into())
                    .h(px(t.layout.control_height_sm))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(brand_mark(t.icon.lg, p)),
            )
            .child(self.sidebar_toggle_v2(cx))
            .child(
                primitives::icon_button_with_shortcut(
                    "new-session-rail",
                    IconName::Plus,
                    "New session",
                    Some("Ctrl N"),
                    ButtonVariant::Ghost,
                    !self.demo,
                    &p,
                )
                .debug_selector(|| "new-session-button".into())
                .when(!self.demo, |d| {
                    d.on_click(cx.listener(|this, _, window, cx| this.show_new_session(window, cx)))
                }),
            )
            .child(
                primitives::icon_button_with_shortcut(
                    "search-rail",
                    IconName::Search,
                    "Search sessions",
                    Some("Ctrl K"),
                    ButtonVariant::Ghost,
                    true,
                    &p,
                )
                .debug_selector(|| "sidebar-search".into())
                .on_click(cx.listener(|this, _, window, cx| this.open_switcher(window, cx))),
            )
            .when_some(self.rail_filter(), |d, mark| {
                d.child(mark.on_click(cx.listener(|this, _, _, cx| {
                    this.project_filter = None;
                    cx.notify();
                })))
            })
            .when(needing > 0, |d| {
                d.child(
                    chrome::interactive_control(div().id("rail-next-needs-you"), p, true)
                        .debug_selector(|| "rail-next-needs-you".into())
                        .rounded(px(t.radius.full))
                        .tooltip(tooltip_view(
                            format!(
                                "{needing} {} you · open the next",
                                if needing == 1 {
                                    "session needs"
                                } else {
                                    "sessions need"
                                }
                            ),
                            Some(if vim { "g n" } else { "Alt N" }),
                            &p,
                        ))
                        .child(primitives::badge(needing.to_string(), Intent::Warning, &p))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.next_needs_you(window, cx)),
                        ),
                )
            })
            .when(!self.ui_bus.notice.is_empty(), |d| {
                d.child(
                    primitives::icon_button(
                        "notice-rail",
                        IconName::Info,
                        "Show the workspace notice",
                        ButtonVariant::Ghost,
                        true,
                        &p,
                    )
                    .text_color(rgb(p.warning))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.open_peek(super::portrait::Peek::Pinned, cx)
                    })),
                )
            })
            .child(
                div()
                    .w(px(t.layout.rail_width - 2. * t.space.md))
                    .my(px(t.space.xs))
                    .h(px(t.stroke.hairline))
                    .bg(rgb(p.border)),
            )
            .child(
                uniform_list(
                    "session-rail",
                    rows.len(),
                    cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|ix| this.render_rail_mark(&rows[ix], ix, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .w_full()
                .flex_1()
                .min_h_0()
                .track_scroll(self.sidebar_scroll.clone()),
            )
            .when(self.view.power_paused, |d| {
                d.child(
                    primitives::icon_button(
                        "resume-rail",
                        IconName::Redo,
                        "Reconnect",
                        ButtonVariant::Ghost,
                        self.view.can_resume_power_pause,
                        &p,
                    )
                    .text_color(rgb(p.warning))
                    .when(self.view.can_resume_power_pause, |d| {
                        d.on_click(cx.listener(|this, _, _, cx| this.command(Command::Refresh, cx)))
                    }),
                )
            })
            .child(self.nav_icon(
                "jobs-rail",
                IconName::Calendar,
                "Jobs",
                vim.then_some("g j"),
                Screen::Jobs,
                cx,
            ))
            .child(self.nav_icon(
                "hosts-rail",
                IconName::LayoutDashboard,
                "Hosts",
                vim.then_some("g o"),
                Screen::Hosts,
                cx,
            ))
            .child(
                self.nav_icon(
                    "scratch-rail",
                    IconName::FolderClosed,
                    "Scratch files",
                    vim.then_some("g x"),
                    Screen::Scratch,
                    cx,
                )
                .relative()
                .when(self.pending_total() > 0, |d| {
                    d.child(
                        div()
                            .absolute()
                            .top(px(t.space.xxs))
                            .right(px(t.space.xxs))
                            .size(px(t.layout.status_dot))
                            .rounded(px(t.radius.full))
                            .bg(rgb(p.warning)),
                    )
                }),
            )
            .child(self.nav_icon(
                "settings-rail",
                IconName::Settings,
                "Settings",
                Some("Ctrl ,"),
                Screen::Settings,
                cx,
            ))
            .when(!self.view.connected, |d| {
                d.child(
                    div()
                        .id("rail-offline")
                        .tooltip(tooltip_view(self.connection_copy().label, None, &p))
                        .child(status_indicator(StatusKind::NeedsYou, &p)),
                )
            })
            .into_any_element()
    }

    /// One rail mark: initials inside a ring that carries the state.
    fn render_rail_mark(&self, row: &SidebarRow, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette();
        let t = p.tokens;
        let connected = self.view.connected;
        let (id, selector, kind, title, meta, depth, active, initials_text, glyph): (
            SharedString,
            String,
            StatusKind,
            String,
            String,
            usize,
            bool,
            String,
            bool,
        ) = match row {
            SidebarRow::Session { index, depth } => {
                let s = &self.view.sessions[*index];
                let active = self
                    .navigation_selected
                    .as_ref()
                    .or(self.view.selected.as_ref())
                    == Some(&s.id)
                    && (self.navigation_selected.is_some() || self.view.child.is_none());
                let title = self.session_title(s);
                let age = s
                    .telemetry
                    .last_activity_ms
                    .map(|at| format!(" · {}", wks_native::format::age(at, timing::now_ms())))
                    .unwrap_or_default();
                let mut mark = wks_native::format::monogram(&title);
                if *depth > 0 {
                    mark.truncate(mark.chars().next().map_or(0, char::len_utf8));
                }
                (
                    format!("session-{}", s.id).into(),
                    format!("sidebar-session-{ix}"),
                    session_kind(self, s),
                    title,
                    format!("{}{age}", self.project_name(&s.cwd)),
                    *depth,
                    active,
                    mark,
                    false,
                )
            }
            SidebarRow::Provider {
                parent,
                child,
                depth,
            } => {
                let viewing = self.view.child.as_ref().is_some_and(|v| {
                    &v.parent == parent && v.agent == child.id && self.navigation_selected.is_none()
                });
                let title = [&child.description, &child.label]
                    .into_iter()
                    .find(|s| !s.is_empty())
                    .cloned()
                    .unwrap_or_else(|| "Subagent".into());
                (
                    format!("sidebar-native-{parent}-{}", child.id).into(),
                    format!("sidebar-provider-{ix}"),
                    child_kind(child, connected),
                    title,
                    "Subagent".into(),
                    *depth,
                    viewing,
                    String::new(),
                    true,
                )
            }
        };
        let size = if depth == 0 {
            t.layout.rail_session
        } else {
            t.layout.rail_child
        };
        let ring = match kind {
            StatusKind::NeedsYou | StatusKind::Working | StatusKind::Failed => kind.color(&p),
            _ => p.border,
        };
        let quiet = matches!(
            kind,
            StatusKind::Idle | StatusKind::Paused | StatusKind::Done
        );
        // Working breathes: a halo widens and fades around the ring.
        let halo = (kind == StatusKind::Working).then(|| {
            super::motion::looping(
                std::time::Duration::from_millis(t.motion.pulse_ms as u64),
                1.,
            )
        });
        let card = hover_card(title, StatusBadge::new(kind), meta, p);
        let target = row.clone();
        let mark = div()
            .relative()
            .size(px(size))
            .flex_shrink_0()
            .when_some(halo, |d, beat| {
                let grow = t.space.xs * beat;
                d.child(
                    div()
                        .absolute()
                        .top(px(-grow))
                        .left(px(-grow))
                        .size(px(size + 2. * grow))
                        .rounded(px(t.radius.full))
                        .bg(primitives::wash(ring, t.elevation.glow_alpha * (1. - beat))),
                )
            })
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded(px(t.radius.full))
                    .border(px(t.stroke.focus))
                    .border_color(rgb(ring))
                    .bg(rgb(if active { p.selected } else { p.base }))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(t.text.overline))
                    .font_weight(FontWeight(t.weight.semibold))
                    .text_color(rgb(if active {
                        p.text
                    } else if quiet {
                        p.roles.text_tertiary
                    } else {
                        p.roles.text_secondary
                    }))
                    .map(|d| {
                        if glyph {
                            d.child(Icon::new(IconName::SquareTerminal).size(px(t.icon.xs)))
                        } else {
                            d.child(initials_text)
                        }
                    }),
            );
        div()
            .relative()
            .h(px(t.layout.rail_row))
            .w_full()
            .flex()
            .items_center()
            .justify_center()
            .when(active, |d| {
                d.child(
                    div()
                        .absolute()
                        .left_0()
                        .top(px((t.layout.rail_row - t.icon.lg) / 2.))
                        .w(px(t.stroke.focus + t.stroke.hairline))
                        .h(px(t.icon.lg))
                        .rounded_r(px(t.radius.xs))
                        .bg(rgb(p.accent)),
                )
            })
            .child(
                chrome::interactive_control(div().id(id), p, connected || depth == 0)
                    .debug_selector(move || selector.clone())
                    .rounded(px(t.radius.full))
                    .hover(move |s| s.bg(rgb(p.roles.hover)))
                    .tooltip(card)
                    .child(mark)
                    .on_click(cx.listener(move |this, _, _, cx| match &target {
                        SidebarRow::Session { index, .. } => {
                            let id = this.view.sessions[*index].id.clone();
                            this.new_session = false;
                            this.screen = Screen::Conversation;
                            this.command(Command::Select(id), cx);
                        }
                        SidebarRow::Provider { parent, child, .. } => {
                            let (parent, agent) = (parent.clone(), child.id.clone());
                            this.open_sidebar_child(&parent, &agent, cx);
                        }
                    })),
            )
            .into_any_element()
    }

    // ── The keyboard cheat sheet ───────────────────────────────────────

    pub(super) fn toggle_shortcuts(&mut self, cx: &mut Context<Self>) {
        self.shell.shortcuts = !self.shell.shortcuts;
        cx.notify();
    }

    /// Every key worth knowing, grouped; the modal keys only when they are on.
    pub(super) fn render_shortcuts(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.shell.shortcuts || !self.design().is_v2() {
            return None;
        }
        let p = self.palette();
        let t = p.tokens;
        let mac = cfg!(target_os = "macos");
        let m = |key: &'static str, mac_key: &'static str| if mac { mac_key } else { key };
        let mut groups: Vec<(&'static str, Vec<(&'static str, &'static str)>)> = vec![
            (
                "Sessions",
                vec![
                    ("Search sessions", m("Ctrl K", "⌘ K")),
                    ("Next session that needs you", "Alt N"),
                    ("Next / previous session", "Alt ↓, Alt ↑"),
                    ("New session", m("Ctrl N", "⌘ N")),
                    ("Projects", m("Ctrl P", "⌘ P")),
                    ("Settings", m("Ctrl ,", "⌘ ,")),
                ],
            ),
            (
                "Conversation",
                vec![
                    ("Type a message", m("Ctrl L", "⌘ L")),
                    ("Send", m("Ctrl Enter", "⌘ Enter")),
                    ("Files and editor", m("Ctrl Shift E", "⌘ Shift E")),
                    ("Terminal", m("Ctrl `", "⌘ `")),
                    ("Refresh", m("Ctrl R", "⌘ R")),
                    ("Zoom in / out", m("Ctrl +, Ctrl -", "⌘ +, ⌘ -")),
                ],
            ),
        ];
        if self.settings.vim_navigation {
            groups.push((
                "Normal mode (press Esc)",
                vec![
                    ("Next / previous session", "j, k"),
                    ("Start typing", "i"),
                    ("Search", "/"),
                    ("Next session that needs you", "g n"),
                    ("Projects · Settings · History", "g p, g s, g h"),
                    ("Changes · Details · Model", "g d, g e, g m"),
                    ("Jobs · Hosts · Scratch", "g j, g o, g x"),
                    ("First / last", "g g, G"),
                    ("This sheet", "?"),
                ],
            ));
        }
        let row = |label: &'static str, keys: &'static str| {
            div()
                .h(px(t.layout.control_height_sm))
                .flex()
                .items_center()
                .gap(px(t.space.lg))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(t.text.body))
                        .text_color(rgb(p.roles.text_secondary))
                        .child(label),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(t.space.sm))
                        // ", " separates alternatives; spaces separate the
                        // keys pressed together or in sequence.
                        .children(keys.split(", ").enumerate().map(|(ix, chord)| {
                            div()
                                .flex()
                                .items_center()
                                .gap(px(t.space.xxs))
                                .when(ix > 0, |d| {
                                    d.child(
                                        div()
                                            .pr(px(t.space.xs))
                                            .text_size(px(t.text.meta))
                                            .text_color(rgb(p.roles.text_tertiary))
                                            .child("/"),
                                    )
                                })
                                .children(chord.split(' ').map(|k| kbd(k, &p)))
                        })),
                )
        };
        let body = div()
            .flex()
            .flex_col()
            .gap(px(t.space.lg))
            .children(groups.into_iter().map(|(title, keys)| {
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .pb(px(t.space.xs))
                            .text_size(px(t.text.meta))
                            .font_weight(FontWeight(t.weight.semibold))
                            .text_color(rgb(p.roles.text_tertiary))
                            .child(title),
                    )
                    .children(keys.into_iter().map(|(l, k)| row(l, k)))
            }));
        let dialog = primitives::dialog(&p)
            .id("shortcuts-dialog")
            .debug_selector(|| "shortcuts-dialog".into())
            .occlude()
            .max_h(gpui::relative(0.85))
            .overflow_y_scroll()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_start()
                    .child(div().flex_1().child(primitives::dialog_header(
                        "Keyboard shortcuts",
                        Some(if self.settings.vim_navigation {
                            "Vim navigation is on: Esc leaves the composer for Normal mode.".into()
                        } else {
                            "Turn on Vim navigation in Settings → Keyboard for single-key moves."
                                .into()
                        }),
                        &p,
                    )))
                    .child(
                        primitives::icon_button(
                            "shortcuts-close",
                            IconName::Close,
                            "Close",
                            ButtonVariant::Ghost,
                            true,
                            &p,
                        )
                        .debug_selector(|| "shortcuts-close".into())
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_shortcuts(cx))),
                    ),
            )
            .child(body);
        Some(
            deferred(
                primitives::scrim(&p)
                    .id("shortcuts-scrim")
                    .debug_selector(|| "shortcuts-scrim".into())
                    .occlude()
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.toggle_shortcuts(cx)),
                    )
                    .child(dialog),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }
}

use std::collections::BTreeMap;
