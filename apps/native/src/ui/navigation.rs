use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Screen {
    Conversation,
    Projects,
    Settings,
    Recent,
    Changes,
    History,
    Session,
    Setup,
    Model,
}

impl Workspace {
    pub(crate) fn configure_settings(
        &mut self,
        settings: Settings,
        path: Option<std::path::PathBuf>,
        scope: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.provider = settings.default_provider.id();
        self.permission = settings.default_access(self.provider);
        self.settings = settings;
        self.settings_path = path;
        self.project_scope = scope;
        self.fonts.sync(&self.settings, window, cx);
        self.apply_typography(cx);
    }

    pub(super) fn save_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_error = match &self.settings_path {
            Some(path) => self
                .settings
                .save(path)
                .err()
                .map(|e| format!("Applied for this run, but could not save: {e}"))
                .unwrap_or_default(),
            None => "Applied for this run; no settings location is available.".into(),
        };
        cx.notify();
    }

    pub(super) fn project_rows(&self, cx: &App) -> Vec<Project> {
        let query = self.search.read(cx).value().to_lowercase();
        projects(
            &self.view.sessions,
            self.settings.bookmarks(&self.project_scope),
        )
        .into_iter()
        .filter(|p| p.path.to_lowercase().contains(&query))
        .collect()
    }

    pub(super) fn visible_sessions(&self, cx: &App) -> Vec<usize> {
        let query = self.search.read(cx).value().to_lowercase();
        let visible: Vec<_> = self
            .view
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                !self.archived(&s.id)
                    && self
                        .project_filter
                        .as_ref()
                        .is_none_or(|path| &s.cwd == path)
                    && (self.session_title(s).to_lowercase().contains(&query)
                        || s.cwd.to_lowercase().contains(&query))
            })
            .map(|(ix, _)| ix)
            .collect();
        wks_native::navigation::session_tree(&self.view.sessions, &visible)
            .into_iter()
            .map(|(ix, _)| ix)
            .collect()
    }

    pub(super) fn show_screen(
        &mut self,
        screen: Screen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.spawn_pending || self.view.creating {
            return;
        }
        if screen == Screen::Conversation
            && self.project_filter.as_ref().is_some_and(|path| {
                !self
                    .view
                    .sessions
                    .iter()
                    .any(|s| Some(&s.id) == self.view.selected.as_ref() && &s.cwd == path)
            })
        {
            self.project_filter = None;
        }
        self.screen = screen;
        self.new_session = false;
        window.focus(&self.focus);
        cx.notify();
    }

    fn normal_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus.is_focused(window) {
            self.back_from_feature(window, cx);
        } else {
            window.focus(&self.focus);
            cx.notify();
        }
    }

    fn focus_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.new_session {
            self.project.update(cx, |input, cx| input.focus(window, cx));
        } else if self.screen == Screen::Projects {
            self.project_path
                .update(cx, |input, cx| input.focus(window, cx));
        } else if self.screen == Screen::Conversation && self.view.selected.is_some() {
            self.composer
                .update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    fn open_project_path(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.requested_session.is_some() {
            self.settings_error =
                "This window is pinned to a session. Open another window to browse projects."
                    .into();
            cx.notify();
            return;
        }
        self.project_filter = Some(path.clone());
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        if let Some(session) = self.view.sessions.iter().find(|s| s.cwd == path) {
            let id = session.id.clone();
            self.show_screen(Screen::Conversation, window, cx);
            self.project_filter = Some(path);
            self.command(Command::Select(id), cx);
        } else if !self.demo {
            self.show_new_session(window, cx);
        }
        cx.notify();
    }

    fn open_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.screen != Screen::Projects || self.new_session {
            return;
        }
        let rows = self.project_rows(cx);
        if let Some(project) = rows.get(self.project_cursor.min(rows.len().saturating_sub(1))) {
            self.open_project_path(project.path.clone(), window, cx);
        }
    }

    pub(super) fn add_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.project_path.read(cx).value().to_string();
        match self.settings.add_project(&self.project_scope, &path) {
            Ok(()) => {
                self.save_settings(cx);
                self.project_path
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.search
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.project_cursor = self
                    .project_rows(cx)
                    .iter()
                    .position(|p| p.path == path.trim())
                    .unwrap_or(0);
                self.projects_scroll
                    .scroll_to_item(self.project_cursor, gpui::ScrollStrategy::Center);
                window.focus(&self.focus);
            }
            Err(e) => self.settings_error = e.to_string(),
        }
        cx.notify();
    }

    fn edge(&mut self, last: bool, cx: &mut Context<Self>) {
        if self.new_session || !matches!(self.screen, Screen::Conversation | Screen::Projects) {
            return;
        }
        if self.screen == Screen::Projects {
            self.project_cursor = if last {
                self.project_rows(cx).len().saturating_sub(1)
            } else {
                0
            };
            self.projects_scroll
                .scroll_to_item(self.project_cursor, gpui::ScrollStrategy::Center);
        } else {
            let rows = self.visible_sessions(cx);
            let index = if last {
                rows.len().saturating_sub(1)
            } else {
                0
            };
            if let Some(&ix) = rows.get(index) {
                self.sidebar_scroll.scroll_to_item(
                    self.sidebar_session_position(index, cx),
                    gpui::ScrollStrategy::Center,
                );
                self.command(Command::Select(self.view.sessions[ix].id.clone()), cx);
            }
        }
        cx.notify();
    }

    fn page(&mut self, direction: f32, window: &Window, cx: &mut Context<Self>) {
        if self.screen == Screen::Conversation && !self.new_session {
            self.pause_follow();
            self.list
                .scroll_by(self.list.viewport_bounds().size.height * (direction * 0.5));
            cx.defer_in(window, |this, window, cx| this.capture_reading(window, cx));
            cx.notify();
        }
    }

    pub(super) fn shell(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        div()
            .key_context(
                if self.settings.vim_navigation && self.focus.is_focused(window) {
                    "Workspace VimNormal"
                } else {
                    "Workspace"
                },
            )
            .track_focus(&self.focus)
            .capture_action(
                cx.listener(|this, _: &gpui_component::input::Paste, window, cx| {
                    if this.screen == Screen::Conversation
                        && !this.new_session
                        && this.composer.read(cx).focus_handle(cx).is_focused(window)
                        && this.view.connected
                        && !this.view.busy
                        && this.paste_image(cx)
                    {
                        cx.stop_propagation();
                    }
                }),
            )
            .size_full()
            .flex()
            .bg(rgb(p.base))
            .text_color(rgb(p.text))
            .font_family(gpui_component::Theme::global(cx).font_family.clone())
            .text_size(px(self.settings.text_size.clamp(12, 20) as f32))
            .on_action(cx.listener(Self::send))
            .on_action(cx.listener(|this, _: &CycleTheme, window, cx| {
                if this.screen == Screen::Settings {
                    let index = Appearance::ALL
                        .iter()
                        .position(|a| *a == this.appearance)
                        .unwrap_or(0);
                    this.choose_theme(
                        Appearance::ALL[(index + 1) % Appearance::ALL.len()],
                        window,
                        cx,
                    );
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleVim, window, cx| {
                if this.screen == Screen::Settings {
                    this.settings.vim_navigation = !this.settings.vim_navigation;
                    window.focus(&this.focus);
                    this.save_settings(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &CycleProvider, _, cx| {
                if this.screen == Screen::Settings {
                    this.settings.default_provider =
                        if this.settings.default_provider == Provider::Claude {
                            Provider::Codex
                        } else {
                            Provider::Claude
                        };
                    this.save_settings(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &NormalMode, window, cx| this.normal_mode(window, cx)))
            .on_action(cx.listener(|this, _: &ShowProjects, window, cx| {
                this.show_screen(Screen::Projects, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowSettings, window, cx| {
                this.show_screen(Screen::Settings, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowHistory, window, cx| {
                this.open_feature(Screen::Recent, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowChanges, window, cx| {
                this.open_feature(Screen::Changes, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowSetup, window, cx| {
                this.open_feature(Screen::Setup, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowSessionDetails, window, cx| {
                this.open_feature(Screen::Session, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowModel, window, cx| {
                this.open_feature(Screen::Model, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowConversation, window, cx| {
                this.show_screen(Screen::Conversation, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &CreateSession, window, cx| {
                    this.show_new_session(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &NextSession, _, cx| this.move_selection(1, cx)))
            .on_action(cx.listener(|this, _: &PreviousSession, _, cx| this.move_selection(-1, cx)))
            .on_action(
                cx.listener(|this, _: &FocusComposer, window, cx| this.focus_editor(window, cx)),
            )
            .on_action(cx.listener(|this, _: &Search, window, cx| {
                this.search.update(cx, |input, cx| input.focus(window, cx));
                cx.notify();
            }))
            .on_action(
                cx.listener(|this, _: &OpenProject, window, cx| this.open_project(window, cx)),
            )
            .on_action(cx.listener(|this, _: &FirstItem, _, cx| this.edge(false, cx)))
            .on_action(cx.listener(|this, _: &LastItem, _, cx| this.edge(true, cx)))
            .on_action(cx.listener(|this, _: &PageUp, window, cx| this.page(-1., window, cx)))
            .on_action(cx.listener(|this, _: &PageDown, window, cx| this.page(1., window, cx)))
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.command(Command::Refresh, cx)))
    }

    pub(super) fn render_projects(&self, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let rows = self.project_rows(cx);
        let count = rows.len();
        div().flex_1().min_w_0().h_full().flex().flex_col().bg(rgb(p.chat))
            .child(div().p_5().flex().flex_col().gap_2()
                .child(overline("WORKSPACE", p))
                .child(div().text_size(px(24.)).font_weight(FontWeight::BOLD).child("Projects"))
                .child(div().text_size(px(12.)).text_color(rgb(p.muted)).child("Your sessions, organized by directory. Save a path to start something new."))
                .child(div().flex().flex_wrap().gap_2().child(Input::new(&self.project_path))
                    .when(self.extras.local_paths, |d| d.child(self.button("browse-bookmark", "Browse…", true).on_click(cx.listener(|this, _, window, cx| this.pick_folder(true, window, cx)))))
                    .child(self.button("save-project", "Save project", true).flex_shrink_0()
                        .on_click(cx.listener(|this, _, window, cx| this.add_project(window, cx)))))
                .child(div().text_size(px(11.)).text_color(rgb(p.muted)).child("Paths belong to the connected hub. Saving a path does not create a directory or launch an agent."))
                .when(!self.settings_error.is_empty(), |d| d.child(div().text_color(rgb(p.warning)).child(self.settings_error.clone()))))
            .when(count == 0, |d| d.child(div().p_5().text_color(rgb(p.muted)).child("No matching projects. Save a directory above or clear the sidebar filter.")))
            .child(uniform_list("project-list", count, cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                range.map(|ix| {
                    let project = &rows[ix];
                    let path = project.path.clone();
                    let forget = path.clone();
                    div().h(px(100.)).px_5().pb_2().child(chrome::interactive_control(div().id(("project", ix)), p, true).h_full().p_3().rounded(px(p.panel_radius))
                        .bg(rgb(if ix == this.project_cursor { p.selected } else { p.surface }))
                        .cursor_pointer().hover(|style| style.bg(rgb(p.selected)))
                        .on_click(cx.listener(move |this, _, window, cx| this.open_project_path(path.clone(), window, cx)))
                        .child(div().flex().justify_between().items_center()
                            .child(div().min_w_0().truncate().font_weight(FontWeight::SEMIBOLD).child(project.title().to_owned()))
                            .when(project.saved, |d| d.child(this.button("forget-project", "Unsave", true)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    if let Some(paths) = this.settings.projects.get_mut(&this.project_scope) { paths.retain(|path| path != &forget); }
                                    this.save_settings(cx);
                                })))))
                        .child(div().truncate().text_size(px(12.)).text_color(rgb(p.muted)).child(project.path.clone()))
                        .child(div().text_size(px(11.)).text_color(rgb(p.accent)).child(format!("{} sessions · {}", project.sessions, if project.sessions == 0 { "Open to start a session" } else { "Open project" }))))
                }).collect::<Vec<_>>()
            })).track_scroll(self.projects_scroll.clone()).flex_1().min_h_0())
            .child(div().px_5().py_3().text_size(px(11.)).text_color(rgb(p.muted)).child(if self.settings.vim_navigation { "j / k navigate · Enter open · / filter · i add · Ctrl Enter save" } else { "Click a project to open it · Ctrl Enter to save a path" }))
    }

    pub(super) fn render_settings(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        use gpui_component::switch::Switch;
        let p = self.appearance.palette();
        let update_busy = self.view.requests.get("updates").is_some_and(|s| s.loading);
        div().id("settings-view").flex_1().min_w_0().h_full().overflow_y_scroll().bg(rgb(p.chat)).p_5()
            .child(div().max_w(px(700.)).mx_auto().flex().flex_col().gap_4()
                .child(div().py_3().flex().flex_col().gap_2()
                    .child(overline("MAKE IT YOURS", p))
                    .child(div().text_size(px(28.)).font_weight(FontWeight::BOLD).child("Settings"))
                    .child(div().text_color(rgb(p.muted)).text_size(px(12.)).child("A workspace that feels like yours. Preferences apply immediately on this device.")))
                .child(chrome::section("Appearance", "Choose the palette for your workspace.", p)
                    .child(div().flex().gap_2().children(Appearance::ALL.into_iter().map(|appearance| {
                        let colors = appearance.palette();
                        let active = self.appearance == appearance;
                        chrome::interactive_control(div().id(appearance.label()), p, true).flex_1().min_w_0().rounded(px(p.panel_radius)).overflow_hidden().cursor_pointer()
                            .bg(rgb(if active { p.selected } else { p.base }))
                            .hover(|style| style.bg(rgb(p.selected)))
                            .child(div().m_2().h(px(64.)).rounded(px(p.control_radius)).bg(rgb(colors.chat)).flex().overflow_hidden()
                                .child(div().w(px(24.)).h_full().bg(rgb(colors.base)).p_2().child(status_dot(colors.accent)))
                                .child(div().flex_1().p_2().flex().flex_col().gap_2()
                                    .child(div().h(px(6.)).w(px(36.)).rounded_full().bg(rgb(colors.text)))
                                    .child(div().h(px(4.)).w_full().rounded_full().bg(rgb(colors.border)))
                                    .child(div().h(px(14.)).w_full().rounded(px(4.)).bg(rgb(colors.surface)))))
                            .child(div().px_3().pb_3().flex().items_center().justify_between()
                                .child(div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(rgb(if active { p.accent } else { p.text })).child(appearance.label()))
                                .when(active, |d| d.child(Icon::new(IconName::Check).size(px(14.)).text_color(rgb(p.accent)))))
                            .on_click(cx.listener(move |this, _, window, cx| this.choose_theme(appearance, window, cx)))
                    })))
                    .when(!self.theme_error.is_empty(), |d| d.child(div().text_size(px(12.)).text_color(rgb(p.warning)).child(self.theme_error.clone()))))
                .child(self.render_typography(cx))
                .child(chrome::section("Workspace", "Control how your workspace fits into your day.", p)
                    .child(chrome::preference("Keep running when closed", "Minimize to the taskbar or Dock. Use Quit to stop the local backend.", p)
                        .child(Switch::new("settings-background").checked(self.settings.keep_running).tooltip("Keep running when closed")
                            .on_click(cx.listener(|this, checked, _, cx| { this.settings.keep_running = *checked; this.extras.keep_running.set(*checked); this.save_settings(cx); }))))
                    .child(chrome::preference("Notifications", "Completion, approval and question alerts while this window is inactive.", p)
                        .child(Switch::new("settings-notifications").checked(self.settings.notifications).tooltip("Notifications")
                            .on_click(cx.listener(|this, checked, _, cx| { this.settings.notifications = *checked; this.save_settings(cx); }))))
                    .child(chrome::preference("Agent setup", "Install and connect the agents on your workspace host.", p)
                        .child(self.quiet_button("settings-setup", "Set up", IconName::ArrowRight, true).on_click(cx.listener(|this, _, window, cx| this.open_feature(Screen::Setup, window, cx))))))
                .child(chrome::section("Default agent", "Used for new sessions. Existing sessions keep their provider.", p)
                    .child(div().flex().gap_2().children([(Provider::Claude, "Claude"), (Provider::Codex, "Codex")].into_iter().map(|(provider, label)| {
                        self.button(label, label, true).flex_1().flex().items_center().justify_between()
                            .when(self.settings.default_provider == provider, |d| d.bg(rgb(p.selected)).text_color(rgb(p.accent)).child(Icon::new(IconName::Check).size(px(14.))))
                            .on_click(cx.listener(move |this, _, _, cx| { this.settings.default_provider = provider; this.save_settings(cx); }))
                    }))))
                .child(chrome::section("Default access mode", "Used when starting a new agent or switching providers. You can override it for each session.", p)
                    .children([("claude", "Claude"), ("codex", "Codex")].into_iter().map(|(provider, label)| {
                        let selected = self.settings.default_access(provider);
                        div().flex().flex_col().gap_2()
                            .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(label))
                            .child(div().flex().flex_wrap().gap_2().children(Permission::choices(provider).iter().copied().map(|access| {
                                self.button("default-access", access.label(), true).id((provider, access as usize))
                                    .when(selected == access, |d| d.bg(rgb(p.selected)).text_color(rgb(p.accent)))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if provider == "claude" { this.settings.default_claude_access = access; }
                                        else { this.settings.default_codex_access = access; }
                                        this.save_settings(cx);
                                    }))
                            })))
                            .child(div().text_size(px(12.)).text_color(rgb(if selected == Permission::FullAccess { p.warning } else { p.muted })).child(selected.description()))
                    })))
                .child(chrome::section("Keyboard", "Move around your workspace at your own pace.", p)
                    .child(chrome::preference("Vim navigation", "Normal mode for navigation. Insert mode for typing.", p)
                        .child(Switch::new("toggle-vim").checked(self.settings.vim_navigation).tooltip("Vim navigation")
                            .on_click(cx.listener(|this, checked, window, cx| {
                                this.settings.vim_navigation = *checked;
                                window.focus(&this.focus);
                                this.save_settings(cx);
                            })))))
                .child(chrome::section("Updates", "Native preview updates are installed manually.", p)
                    .child(div().text_size(px(12.)).text_color(rgb(p.muted)).child(format!("Installed version {}", wks_native::features::installed_version())))
                    .child(div().flex().flex_wrap().gap_2()
                        .child(self.button("check-updates", if update_busy { "Checking…" } else { "Check for updates" }, !update_busy)
                            .when(!update_busy, |d| d.on_click(cx.listener(|this, _, _, cx| this.request(wks_native::features::Request::Updates, cx)))))
                        .child(self.quiet_button("open-releases", "Release notes", IconName::ExternalLink, true).on_click(|_, _, cx| cx.open_url(wks_native::features::RELEASES_URL))))
                    .child(self.feature_message("updates"))
                    .when_some(self.view.requests.get("updates").and_then(|s| s.value["version"].as_str()), |d, version| d.child(div().text_size(px(12.)).child(format!("Latest stable release: {version}")))))
                .when(!self.settings_error.is_empty(), |d| d.child(div().text_size(px(12.)).text_color(rgb(p.warning)).child(self.settings_error.clone())))
                .child(chrome::section("Shortcuts", "Vim shortcuts apply in Normal mode. Text fields keep ordinary editing keys.", p)
                .children([
                    ("Esc", "Leave a text field; press again to return to the conversation"),
                    ("j / k", "Next / previous session or project"),
                    ("gg / G", "First / last session or project"),
                    ("i", "Compose, edit the new-session form, or add a project path"),
                    ("/", "Filter sessions and projects"),
                    ("g p / h", "Projects"), ("Enter / l", "Open the selected project"),
                    ("g h / g d", "Session history / changes"), ("g a / g e / g m", "Agent setup / session / model"),
                    ("g s", "Settings"), ("g c", "Conversation"),
                    ("n", "New session in the selected project"),
                    ("t / v / a", "Settings: cycle theme / toggle Vim / switch default agent"),
                    ("Ctrl U / D", "Scroll conversation up / down"),
                    ("Ctrl/Cmd ,", "Settings (also when Vim navigation is off)"),
                    ("Ctrl/Cmd P", "Projects (also when Vim navigation is off)"),
                    ("Ctrl/Cmd Enter", "Send message, create session, or save a project path"),
                    ("Ctrl/Cmd L", "Focus the editor"),
                ].into_iter().map(|(keys, label)| div().flex().gap_3().items_start()
                    .child(keycap(keys, p).w(px(120.)).flex_shrink_0())
                    .child(div().text_size(px(12.)).text_color(rgb(p.muted)).child(label))))
                ))
    }
}
