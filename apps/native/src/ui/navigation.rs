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
        set_zoom(self.settings.interface_scale as f32 / 100.);
        self.fonts.sync(&self.settings, window, cx);
        self.apply_typography(cx);
    }

    /// Resize the whole interface; layout caches are rebuilt at the new size.
    pub(super) fn set_interface_scale(
        &mut self,
        percent: u16,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let percent = percent.clamp(
            wks_native::navigation::INTERFACE_SCALES[0],
            *wks_native::navigation::INTERFACE_SCALES.last().unwrap(),
        );
        if percent == self.settings.interface_scale {
            return;
        }
        self.settings.interface_scale = percent;
        set_zoom(percent as f32 / 100.);
        self.apply_typography(cx);
        self.save_settings(cx);
        window.refresh();
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
        if self.file_viewer().is_some() {
            self.close_file_viewer(window, cx);
            return;
        }
        if self.usage_open {
            self.usage_open = false;
            cx.notify();
            return;
        }
        if self.focus.is_focused(window) {
            if self.view.child.is_some() && self.screen == Screen::Conversation && !self.new_session
            {
                self.command(Command::ViewChild(None), cx);
                return;
            }
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

    fn page(&mut self, direction: f32, _window: &Window, cx: &mut Context<Self>) {
        if self.screen == Screen::Conversation && !self.new_session {
            self.pause_follow();
            self.list
                .scroll_by(self.list.viewport_bounds().size.height * (direction * 0.5));
            cx.notify();
        }
    }

    pub(super) fn shell(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let viewer = self.file_viewer().is_some();
        div()
            .relative()
            // Deferred, so they paint over the sidebar and content added later.
            .children(self.render_caption(window))
            .children(self.render_usage_modal(cx))
            .children(self.render_file_viewer(window, cx))
            .key_context(if viewer {
                "FileViewer"
            } else if self.settings.vim_navigation && self.focus.is_focused(window) {
                "Workspace VimNormal"
            } else {
                "Workspace"
            })
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
            .bg(rgb(p.chat))
            .text_color(rgb(p.text))
            .font_family(gpui_component::Theme::global(cx).font_family.clone())
            .text_size(px(self.settings.text_size.clamp(12, 20) as f32))
            // While the file viewer is open none of the workspace's actions
            // exist: no binding matches the covered composer, sidebar or
            // conversation, and only the viewer's own keys work.
            .map(|shell| {
                if viewer {
                    shell
                } else {
                    self.workspace_actions(shell, cx)
                }
            })
    }

    fn workspace_actions(&self, shell: Div, cx: &mut Context<Self>) -> Div {
        shell
            .on_action(cx.listener(Self::send))
            .on_action(cx.listener(|this, _: &ZoomIn, window, cx| {
                let next =
                    wks_native::navigation::step_interface_scale(this.settings.interface_scale, 1);
                this.set_interface_scale(next, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ZoomOut, window, cx| {
                let next =
                    wks_native::navigation::step_interface_scale(this.settings.interface_scale, -1);
                this.set_interface_scale(next, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ZoomReset, window, cx| {
                this.set_interface_scale(100, window, cx)
            }))
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
                if this.screen == Screen::Settings && !this.new_session {
                    this.settings_search
                        .update(cx, |input, cx| input.focus(window, cx));
                } else {
                    this.search.update(cx, |input, cx| input.focus(window, cx));
                }
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
}
