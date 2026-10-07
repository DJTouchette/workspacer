use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Screen {
    Conversation,
    Projects,
    Settings,
    Recent,
    /// The hub's scheduled jobs: read and approve, never author.
    Jobs,
    Changes,
    History,
    Session,
    Setup,
    Model,
    /// Continue the open session with the other provider.
    Handoff,
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

    pub(super) fn project_rows(&self, cx: &App) -> Vec<KnownProject> {
        let query = self.search.read(cx).value().to_owned();
        self.known_projects()
            .into_iter()
            .filter(|p| p.listed(&query))
            .collect()
    }

    pub(super) fn visible_sessions(&self, cx: &App) -> Vec<usize> {
        if self.archive_loading() {
            return Vec::new();
        }
        let query = self.search.read(cx).value().to_lowercase();
        let sessions = &self.view.sessions;
        // Finished children the user cleared leave this device's list, except
        // the one open now; a live descendant keeps its cleared parent.
        let open = self
            .navigation_selected
            .as_ref()
            .or(self.view.selected.as_ref());
        let cleared = wks_native::navigation::cleared_hidden(
            sessions,
            |ix| !self.archived(&sessions[ix].id),
            |ix| Some(&sessions[ix].id) != open && self.session_cleared(&sessions[ix]),
        );
        // Project and search each match a lineage, so workers in their own
        // worktrees stay nested under a manager the filter matched.
        let in_project = wks_native::navigation::lineage_filter(
            sessions,
            |ix| !self.archived(&sessions[ix].id) && !cleared[ix],
            |ix| {
                self.project_filter
                    .as_ref()
                    .is_none_or(|path| wks_native::projects::same_dir(&sessions[ix].cwd, path))
            },
        );
        let visible = wks_native::navigation::lineage_filter(
            sessions,
            |ix| in_project.binary_search(&ix).is_ok(),
            |ix| {
                let s = &sessions[ix];
                self.session_title(s).to_lowercase().contains(&query)
                    || s.cwd.to_lowercase().contains(&query)
            },
        );
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
                // A worker belongs to its manager's project for this check.
                let sessions = &self.view.sessions;
                !sessions
                    .iter()
                    .position(|s| Some(&s.id) == self.view.selected.as_ref())
                    .is_some_and(|ix| {
                        std::iter::once(ix)
                            .chain(wks_native::navigation::ancestors(sessions, ix))
                            .any(|ix| wks_native::projects::same_dir(&sessions[ix].cwd, path))
                    })
            })
        {
            self.project_filter = None;
        }
        self.screen = screen;
        self.new_session = false;
        if screen == Screen::Projects {
            self.load_projects(cx);
        }
        // The hub's child-agent access setting is shown fresh each visit.
        if screen == Screen::Settings
            && !self.demo
            && self.view.connected
            && !self
                .view
                .requests
                .get("child-access")
                .is_some_and(|s| s.loading)
        {
            self.request(wks_native::features::Request::ChildAccess { set: None }, cx);
        }
        if screen == Screen::Settings {
            if !self.extras.title_harness_seeded {
                self.extras.title_harness_seeded = true;
                self.extras.title_harness = self.settings.default_provider.id();
            }
            self.load_titles(cx);
        }
        // An outcome belongs to the screen it happened on; a pending
        // keep-on-device offer stays until it is answered.
        if self.projects.fallback.is_none() {
            self.projects.notice.clear();
        }
        window.focus(&self.focus);
        cx.notify();
    }

    fn normal_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.viewer_modal(window) {
            self.close_file_viewer(window, cx);
            return;
        }
        if self.usage_open {
            self.usage_open = false;
            cx.notify();
            return;
        }
        // Esc leaves an open project list for the project already chosen.
        if self.new_session && self.close_project_picker(window, cx) {
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

    pub(super) fn focus_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.new_session {
            if self.projects.picker_open {
                self.project_query
                    .update(cx, |input, cx| input.focus(window, cx));
            } else {
                self.prompt.update(cx, |input, cx| input.focus(window, cx));
            }
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
        if let Some(session) = self
            .view
            .sessions
            .iter()
            .find(|s| wks_native::projects::same_dir(&s.cwd, &path))
        {
            let id = session.id.clone();
            self.show_screen(Screen::Conversation, window, cx);
            self.project_filter = Some(path);
            self.command(Command::Select(id), cx);
        } else if !self.demo {
            self.show_new_session(window, cx);
        }
        cx.notify();
    }

    /// Start another agent in a project, whatever is already running there:
    /// the project filter keeps its sessions one click away in the sidebar,
    /// and the New Agent form opens on that folder. Nothing launches until
    /// the user confirms the form.
    pub(super) fn new_agent_in_project(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.demo || self.spawn_pending || self.view.creating {
            return;
        }
        self.project_filter = Some(path.clone());
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.show_new_session(window, cx);
        self.seed_project(&path, cx);
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

    /// Save a typed path as a pinned project in the hub's shared registry.
    /// Nothing is created on disk and no agent starts.
    pub(super) fn add_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.project_path.read(cx).value().trim().to_owned();
        if !wks_native::launch::absolute_directory(&path) {
            self.settings_error = "Enter an absolute project directory on the hub's machine".into();
            cx.notify();
            return;
        }
        self.settings_error.clear();
        self.set_project_pin(wks_native::projects::project_key(&path), true, cx);
        self.project_path
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.project_cursor = 0;
        self.projects_scroll
            .scroll_to_item(0, gpui::ScrollStrategy::Top);
        window.focus(&self.focus);
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
        let viewer = self.viewer_owns_keys(window, cx);
        div()
            .relative()
            // Deferred, so they paint over the sidebar and content added later.
            .children(self.render_caption(window))
            .children(self.render_usage_modal(cx))
            .children(self.render_file_viewer(window, cx))
            .children(self.render_tasks_sheet(window, cx))
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
            // While the file viewer is modal or has focus none of the
            // workspace's actions exist: no binding matches the composer,
            // sidebar or conversation, and only the viewer's own keys work.
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
            .on_action(cx.listener(Self::composer_enter))
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
            .on_action(cx.listener(|this, _: &ShowJobs, window, cx| {
                this.open_feature(Screen::Jobs, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowConversation, window, cx| {
                this.show_screen(Screen::Conversation, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CreateSession, window, cx| {
                // On Projects, `n` starts an agent in the highlighted
                // project rather than wherever the chat last was.
                if this.screen == Screen::Projects && !this.new_session {
                    let rows = this.project_rows(cx);
                    if let Some(project) =
                        rows.get(this.project_cursor.min(rows.len().saturating_sub(1)))
                    {
                        this.new_agent_in_project(project.path.clone(), window, cx);
                        return;
                    }
                }
                this.show_new_session(window, cx)
            }))
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
            .on_action(
                cx.listener(|this, _: &OpenEditor, window, cx| {
                    this.open_selected_editor(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &ToggleTerminal, window, cx| {
                    this.toggle_terminal(window, cx)
                }),
            )
    }

    /// The editor on the selected session's folder.
    pub(super) fn open_selected_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.selected_session().cloned() else {
            self.extras.notice = "Select a session to browse its files.".into();
            cx.notify();
            return;
        };
        self.open_editor(&session.id, &session.cwd, window, cx);
    }

    pub(super) fn render_projects(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        // Short windows give the list, not the explanations, the height.
        let short = window.viewport_size().height < px(620.);
        let p = self.appearance.palette();
        let rows = self.project_rows(cx);
        let count = rows.len();
        let can_write = self.can_write_projects();
        let header = self.page_header(
            None,
            Some("WORKSPACE"),
            "Projects",
            (!short).then(|| {
                "Pinned and recent projects from the connected hub, plus folders your sessions run in."
                    .into()
            }),
            None,
            short,
        );
        let top = if chrome::custom_caption() {
            chrome::PAGE_CAPTION_INSET
        } else if short {
            12.
        } else {
            24.
        };
        let empty = if self
            .view
            .requests
            .get("projects")
            .is_some_and(|s| s.loading)
            && self.projects.registry.is_none()
        {
            "Loading projects…"
        } else {
            "No matching projects. Pin a directory above or clear the sidebar filter."
        };
        let hint = if self.settings.vim_navigation {
            "j / k navigate · Enter open · n new agent · / filter · i add · Ctrl Enter pin"
        } else {
            "Click a project to open it · New agent starts another · Ctrl Enter to pin a path"
        };
        let list = uniform_list(
            "project-list",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|ix| this.render_project_row(ix, &rows[ix], cx))
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(self.projects_scroll.clone())
        .flex_1()
        .min_h_0();
        div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(p.chat))
            .children(chrome::page_drag_strip())
            .child(
                div()
                    .w_full()
                    .max_w(px(CHAT_WIDTH + 48.))
                    .mx_auto()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .px(px(if short { 16. } else { 24. }))
                            .pt(px(top))
                            .pb_3()
                            .flex()
                            .flex_col()
                            .gap(px(if short { 8. } else { 16. }))
                            .child(header)
                            .child(self.render_project_form(short, can_write, cx))
                            .children(self.render_project_notices(cx))
                            .children(self.render_identity_editor(cx))
                            .when_some(self.projects.registry_error.clone(), |d, error| {
                                d.child(chrome::notice_line(
                                    projects::registry_error_notice(&error),
                                    chrome::Tone::Warning,
                                    p,
                                    "projects-registry-error",
                                ))
                            }),
                    )
                    .when(count == 0, |d| {
                        d.child(
                            div()
                                .px_6()
                                .py_6()
                                .text_center()
                                .text_size(px(chrome::scale::META))
                                .text_color(rgb(p.muted))
                                .child(empty),
                        )
                    })
                    .child(list)
                    .child(
                        div()
                            .px_6()
                            .py_3()
                            .text_size(px(chrome::scale::CAPTION))
                            .text_color(rgb(p.muted))
                            .child(hint),
                    ),
            )
    }

    /// The hub registry takes writes now: connected, not a demo, and no save
    /// already in flight.
    fn can_write_projects(&self) -> bool {
        let saving = self
            .view
            .requests
            .get("project-save")
            .is_some_and(|s| s.loading);
        self.view.connected && !self.demo && !saving
    }

    /// The path field that pins a project on the hub.
    fn render_project_form(&self, short: bool, can_write: bool, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        chrome::card(p)
            .p(px(if short { 10. } else { 16. }))
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(220.))
                            .child(Input::new(&self.project_path)),
                    )
                    .when(self.extras.local_paths, |d| {
                        d.child(self.button("browse-bookmark", "Browse…", true).on_click(
                            cx.listener(|this, _, window, cx| this.pick_folder(true, window, cx)),
                        ))
                    })
                    .child(
                        self.primary_button("save-project", "Pin project", can_write)
                            .flex_shrink_0()
                            .when(can_write, |d| {
                                d.on_click(
                                    cx.listener(|this, _, window, cx| this.add_project(window, cx)),
                                )
                            }),
                    ),
            )
            .when(!short, |d| {
                d.child(
                    div()
                        .text_size(px(chrome::scale::CAPTION))
                        .text_color(rgb(p.muted))
                        .child(
                            "Paths belong to the connected hub. Pinning shares the project with Workspacer on that hub; it does not create a folder or launch an agent.",
                        ),
                )
            })
    }

    /// Settings and project notices, with the offer to keep projects on
    /// this device when the hub could not take them.
    fn render_project_notices(&self, cx: &mut Context<Self>) -> Vec<Div> {
        let p = self.appearance.palette();
        let mut notices = Vec::new();
        if !self.settings_error.is_empty() {
            notices.push(chrome::notice_line(
                self.settings_error.clone(),
                chrome::Tone::Error,
                p,
                "projects-settings-error",
            ));
        }
        if !self.projects.notice.is_empty() {
            let tone = if self.projects.fallback.is_some() {
                chrome::Tone::Warning
            } else {
                chrome::notice_tone(&self.projects.notice)
            };
            notices.push(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w(px(200.)).child(chrome::notice_line(
                        self.projects.notice.clone(),
                        tone,
                        p,
                        "projects-notice",
                    )))
                    .when(self.projects.fallback.is_some(), |d| {
                        d.child(
                            self.quiet_button(
                                "keep-on-device-projects",
                                "Keep on this device",
                                IconName::Check,
                                true,
                            )
                            .debug_selector(|| "keep-on-device-projects".into())
                            .on_click(
                                cx.listener(|this, _, _, cx| this.keep_project_on_device(cx)),
                            ),
                        )
                    }),
            );
        }
        notices
    }

    fn render_project_row(&self, ix: usize, project: &KnownProject, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let can_write = self.can_write_projects();
        let can_spawn = self.view.connected && !self.demo;
        let pinned = project.favourite;
        let on_device = project.source == wks_native::projects::Source::Device;
        let sessions = match (project.live_sessions, project.sessions) {
            (0, 0) => "No sessions yet · open to start one".to_owned(),
            (0, 1) => "1 ended · open to view it".to_owned(),
            (0, n) => format!("{n} ended · open to browse them"),
            (live, n) => format!("{live} running of {n} · open to browse them"),
        };
        let path = project.path.clone();
        let new_path = path.clone();
        let edit_path = path.clone();
        let pin_path = path.clone();
        let forget = project.clone();
        let details = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(project.title().to_owned()),
                    )
                    .when(on_device, |d| {
                        d.child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(10.))
                                .text_color(rgb(p.muted))
                                .child("this device"),
                        )
                    }),
            )
            .child(
                div()
                    .truncate()
                    .font_family(mono_font())
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .child(project.path.clone()),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(if project.live_sessions > 0 {
                        p.busy
                    } else {
                        p.accent
                    }))
                    .child(sessions),
            );
        let row = chrome::interactive_control(div().id(("project", ix)), p, true)
            .h_full()
            .p_3()
            .rounded(px(p.panel_radius))
            .bg(rgb(if ix == self.project_cursor {
                p.selected
            } else {
                p.surface
            }))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(p.selected)))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.open_project_path(path.clone(), window, cx)
            }))
            .flex()
            .items_center()
            .gap_3()
            .child(self.project_mark(Some(project), &project.path, 36.))
            .child(details)
            .child(
                self.quiet_button(
                    SharedString::from(format!("new-agent-in-project-{ix}")),
                    "New agent",
                    IconName::Plus,
                    can_spawn,
                )
                .debug_selector(move || format!("new-agent-in-project-{ix}"))
                .when(can_spawn, |d| {
                    d.on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.new_agent_in_project(new_path.clone(), window, cx);
                    }))
                }),
            )
            .child(
                self.icon_button(
                    SharedString::from(format!("edit-project-{ix}")),
                    "Edit name and icon",
                    IconName::Palette,
                    can_write,
                )
                .debug_selector(move || format!("edit-project-{ix}"))
                .when(can_write, |d| {
                    d.on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.open_identity_editor(edit_path.clone(), window, cx);
                    }))
                }),
            )
            .when(project.removable(), |d| {
                d.child(
                    self.danger_button(
                        SharedString::from(format!("forget-project-{ix}")),
                        "Forget",
                        on_device || can_write,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.forget_project(&forget, cx);
                    })),
                )
            })
            .child(
                self.icon_button(
                    SharedString::from(format!("pin-project-{ix}")),
                    if pinned {
                        "Unpin project"
                    } else {
                        "Pin project"
                    },
                    IconName::Star,
                    can_write,
                )
                .when(pinned, |d| d.text_color(rgb(p.accent)))
                .when(can_write, |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.set_project_pin(pin_path.clone(), !pinned, cx);
                    }))
                }),
            );
        div().h(px(84.)).px_6().pb_2().child(row)
    }
}
