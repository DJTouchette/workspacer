//! The New Agent screen's project chooser: the hub's shared registry, this
//! device's older bookmarks and the fleet's directories as one list, plus a
//! summary of the chosen project and a hub-side folder browser.
use super::*;
use gpui_component::input::{MoveDown, MoveUp};
use wks_native::features::Request;
use wks_native::projects::{self, Identity, Inspection, KnownProject, Patch, Source};

/// Rows the picker offers, in keyboard order.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum PickRow {
    /// The query is an absolute path that is not already a project.
    Typed(String),
    /// The chosen folder, when it is not one of the listed projects, so
    /// reopening the list never hides what is selected.
    Current(String),
    Project(KnownProject),
}

impl PickRow {
    fn path(&self) -> &str {
        match self {
            Self::Typed(path) | Self::Current(path) => path,
            Self::Project(project) => &project.path,
        }
    }
}

#[derive(Default)]
pub(super) struct ProjectUi {
    /// The working directory the next launch uses. Empty = not chosen yet.
    pub cwd: String,
    pub picker_open: bool,
    pub cursor: usize,
    /// The newest registry the hub returned, from a read or a verified write.
    pub registry: Option<Arc<serde_json::Value>>,
    pub registry_revision: u64,
    pub read_receipt: u64,
    pub registry_error: Option<String>,
    pub save_receipt: u64,
    /// Outcome of the user's last pin/forget, shown beside the list.
    pub notice: String,
    /// A path the hub refused to pin, offered as a device-only bookmark.
    pub fallback: Option<String>,
    /// The hub folder browser replaces the list while open.
    pub browsing: bool,
    /// Name/icon editing for one project on the Projects screen.
    pub editor: Option<IdentityEditor>,
    /// Downloaded icons by `iconFile`: `None` = unavailable (keeps the
    /// emoji/initials mark); absent = not read yet.
    pub icons: HashMap<String, Option<Arc<gpui::Image>>>,
    pub icon_receipt: u64,
}

/// The project identity form: the registry's own fields, prefilled from the
/// hub's current entry so saving never drops what another client set.
pub(super) struct IdentityEditor {
    pub path: String,
    pub base: Identity,
    pub name: Entity<InputState>,
    pub icon: Entity<InputState>,
    pub favicon: Entity<InputState>,
}

/// Picker rows are this tall; the list shows at most this many before it
/// scrolls, so the rest of the form stays in view.
const ROW_HEIGHT: f32 = 52.;
const VISIBLE_ROWS: usize = 5;

impl Workspace {
    pub(super) fn known_projects(&self) -> Vec<KnownProject> {
        projects::list(
            self.projects.registry.as_deref(),
            &self.view.sessions,
            self.settings.bookmarks(&self.project_scope),
        )
    }

    pub(super) fn known_project(&self, path: &str) -> Option<KnownProject> {
        self.known_projects()
            .into_iter()
            .find(|p| projects::same_dir(&p.path, path))
    }

    pub(super) fn pick_rows(&self, cx: &App) -> Vec<PickRow> {
        let query = self.project_query.read(cx).value().trim().to_owned();
        let known = self.known_projects();
        let mut rows = Vec::new();
        if wks_native::launch::absolute_directory(&query)
            && !known.iter().any(|p| projects::same_dir(&p.path, &query))
        {
            rows.push(PickRow::Typed(projects::project_key(&query)));
        }
        if query.is_empty()
            && !self.projects.cwd.is_empty()
            && !known
                .iter()
                .any(|p| projects::same_dir(&p.path, &self.projects.cwd))
        {
            rows.push(PickRow::Current(self.projects.cwd.clone()));
        }
        let absolute = wks_native::launch::absolute_directory(&query);
        rows.extend(
            known
                .into_iter()
                .filter(|p| {
                    if absolute {
                        // A pasted path narrows to that folder and its children.
                        projects::project_key(&p.path)
                            .to_lowercase()
                            .starts_with(&projects::project_key(&query).to_lowercase())
                    } else {
                        p.matches(&query)
                    }
                })
                .map(PickRow::Project),
        );
        rows
    }

    pub(super) fn load_projects(&mut self, cx: &mut Context<Self>) {
        if self.demo || !self.view.connected {
            return;
        }
        if self
            .view
            .requests
            .get("projects")
            .is_some_and(|state| state.loading)
        {
            return;
        }
        self.request(Request::Projects, cx);
    }

    /// Ask the hub about the chosen folder again (fresh form, reconnect).
    pub(super) fn refresh_project_inspection(&mut self, cx: &mut Context<Self>) {
        if self.demo
            || !self.view.connected
            || !wks_native::launch::absolute_directory(&self.projects.cwd)
        {
            return;
        }
        let path = self.projects.cwd.clone();
        self.request(Request::InspectProject { path }, cx);
    }

    /// The hub's latest word on the chosen folder, if it is about that folder.
    pub(super) fn inspection(&self) -> Option<Result<Inspection, String>> {
        let state = self.view.requests.get("project-inspect")?;
        let Request::InspectProject { path } = &state.request else {
            return None;
        };
        if path != &self.projects.cwd || state.loading {
            return None;
        }
        if let Some(error) = &state.error {
            return Some(Err(error.clone()));
        }
        projects::parse_inspection(&state.value).map(Ok)
    }

    fn inspecting(&self) -> bool {
        self.view.requests.get("project-inspect").is_some_and(|s| {
            s.loading
                && matches!(&s.request, Request::InspectProject { path } if path == &self.projects.cwd)
        })
    }

    /// Choose the launch directory. Prompt, provider, model and options are
    /// the user's own choices and stay; only cwd-scoped state follows.
    pub(super) fn select_project(
        &mut self,
        path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.spawn_pending || self.view.creating {
            return;
        }
        let path = projects::project_key(path);
        let changed = !projects::same_dir(&path, &self.projects.cwd) || path.is_empty();
        self.projects.cwd = path;
        self.projects.picker_open = self.projects.cwd.is_empty();
        self.projects.browsing = false;
        self.spawn_error.clear();
        if self.projects.fallback.is_none() {
            self.projects.notice.clear();
        }
        self.project_query
            .update(cx, |input, cx| input.set_value("", window, cx));
        if changed {
            self.refresh_project_inspection(cx);
            self.load_models(false, cx);
        }
        if self.projects.picker_open {
            self.project_query
                .update(cx, |input, cx| input.focus(window, cx));
        } else {
            self.prompt.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    /// Seed the launch directory without moving focus (fresh form, resume,
    /// bus prefill). Returns whether it changed (and was sent for checking).
    pub(super) fn seed_project(&mut self, path: &str, cx: &mut Context<Self>) -> bool {
        let path = projects::project_key(path);
        let changed = !projects::same_dir(&path, &self.projects.cwd);
        self.projects.cwd = path;
        self.projects.picker_open = self.projects.cwd.is_empty();
        self.projects.browsing = false;
        if changed {
            self.refresh_project_inspection(cx);
        }
        changed
    }

    pub(super) fn open_project_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.spawn_pending || self.view.creating {
            return;
        }
        self.projects.picker_open = true;
        self.projects.browsing = false;
        self.project_query
            .update(cx, |input, cx| input.set_value("", window, cx));
        let rows = self.pick_rows(cx);
        self.projects.cursor = rows
            .iter()
            .position(|row| projects::same_dir(row.path(), &self.projects.cwd))
            .unwrap_or(0);
        self.load_projects(cx);
        self.project_query
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    /// Close the list and keep the chosen project. No-op until one is chosen:
    /// there would be nothing to show in its place.
    pub(super) fn close_project_picker(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.projects.picker_open || self.projects.cwd.is_empty() {
            return false;
        }
        self.projects.picker_open = false;
        self.projects.browsing = false;
        self.project_query
            .update(cx, |input, cx| input.set_value("", window, cx));
        window.focus(&self.focus);
        cx.notify();
        true
    }

    pub(super) fn move_project_cursor(&mut self, step: isize, cx: &mut Context<Self>) {
        let len = self.pick_rows(cx).len();
        if len == 0 {
            return;
        }
        self.projects.cursor =
            (self.projects.cursor.min(len - 1) as isize + step).rem_euclid(len as isize) as usize;
        self.project_list_scroll
            .scroll_to_item(self.projects.cursor, gpui::ScrollStrategy::Center);
        cx.notify();
    }

    /// Enter in the search field: the highlighted row, else a typed path.
    pub(super) fn confirm_project_cursor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.pick_rows(cx);
        if let Some(row) = rows.get(self.projects.cursor.min(rows.len().saturating_sub(1))) {
            let path = row.path().to_owned();
            self.select_project(&path, window, cx);
        }
    }

    /// Ctrl Enter with the list open launches into a pasted absolute path, the
    /// same as choosing its "Use folder" row first.
    pub(super) fn adopt_typed_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.projects.picker_open {
            return;
        }
        let query = self.project_query.read(cx).value().trim().to_owned();
        if wks_native::launch::absolute_directory(&query) {
            self.select_project(&query, window, cx);
        }
    }

    pub(super) fn set_project_pin(&mut self, path: String, pinned: bool, cx: &mut Context<Self>) {
        if self.demo || !self.view.connected {
            self.projects.notice = "Connect to the hub to change its projects.".into();
            cx.notify();
            return;
        }
        if self
            .view
            .requests
            .get("project-save")
            .is_some_and(|s| s.loading)
        {
            return;
        }
        self.projects.notice.clear();
        self.projects.fallback = None;
        self.request(
            Request::SaveProject {
                path,
                change: Patch::Pin(pinned),
            },
            cx,
        );
    }

    pub(super) fn forget_project(&mut self, project: &KnownProject, cx: &mut Context<Self>) {
        match project.source {
            Source::Device => {
                if let Some(paths) = self.settings.projects.get_mut(&self.project_scope) {
                    paths.retain(|p| !projects::same_dir(p, &project.path));
                }
                self.save_settings(cx);
            }
            Source::Hub if project.removable() => {
                if self
                    .view
                    .requests
                    .get("project-save")
                    .is_some_and(|s| s.loading)
                {
                    return;
                }
                self.projects.notice.clear();
                self.request(
                    Request::SaveProject {
                        path: project.path.clone(),
                        change: Patch::Remove,
                    },
                    cx,
                );
            }
            _ => {}
        }
        cx.notify();
    }

    /// The hub refused a pin (read-only token, locked config): keep the path
    /// on this device instead, saying so, rather than pretending it saved.
    pub(super) fn keep_project_on_device(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.projects.fallback.take() else {
            return;
        };
        match self.settings.add_project(&self.project_scope, &path) {
            Ok(()) => {
                self.save_settings(cx);
                self.projects.notice = if self.settings_error.is_empty() {
                    "Saved on this device only.".into()
                } else {
                    self.settings_error.clone()
                };
            }
            Err(error) => self.projects.notice = error.to_string(),
        }
        cx.notify();
    }

    /// Browse for a folder: the OS dialog when the hub shares this machine's
    /// filesystem, otherwise the hub's own directory listing.
    pub(super) fn browse_projects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.extras.local_paths {
            self.pick_folder(false, window, cx);
            return;
        }
        let start = if wks_native::launch::absolute_directory(&self.projects.cwd) {
            self.projects.cwd.clone()
        } else {
            String::new()
        };
        self.browse_to(start, cx);
    }

    pub(super) fn browse_to(&mut self, path: String, cx: &mut Context<Self>) {
        if self.demo || !self.view.connected {
            return;
        }
        self.projects.picker_open = true;
        self.projects.browsing = true;
        self.request(Request::BrowseFolders { path }, cx);
        cx.notify();
    }

    fn listing(&self) -> (bool, Option<projects::Listing>, Option<String>) {
        match self.view.requests.get("project-browse") {
            None => (false, None, None),
            Some(state) => (
                state.loading,
                projects::parse_listing(&state.value),
                state.error.clone(),
            ),
        }
    }

    /// Fold registry reads and verified writes into the newest registry, and
    /// surface the outcome of the user's own saves.
    pub(super) fn sync_projects(&mut self, next: &View) {
        if let Some(state) = next.requests.get("projects")
            && !state.loading
            && state.number > self.projects.read_receipt
        {
            self.projects.read_receipt = state.number;
            self.projects.registry_error = state.error.clone();
        }
        // Request numbers describe start order, not snapshot freshness. Reads
        // and writes carry revisions assigned inside the same per-hub barrier.
        // The controller's retained snapshot also survives queued touch slots.
        let snapshots = next.project_registry.iter().chain(
            ["projects", "project-save", "project-touch"]
                .into_iter()
                .filter_map(|key| next.requests.get(key))
                .filter(|s| !s.loading && s.error.is_none())
                .map(|s| &s.value),
        );
        for value in snapshots {
            let revision = value["revision"].as_u64().unwrap_or(0);
            if revision > self.projects.registry_revision {
                self.projects.registry = Some(value.clone());
                self.projects.registry_revision = revision;
                self.projects.registry_error = None;
            }
        }
        if let Some(state) = next.requests.get("project-icons")
            && !state.loading
            && state.number > self.projects.icon_receipt
        {
            self.projects.icon_receipt = state.number;
            if let Request::ProjectIcons { files } = &state.request {
                for file in files {
                    let image = state.value[file]["png"]
                        .as_str()
                        .and_then(|data| {
                            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data)
                                .ok()
                        })
                        .map(|bytes| {
                            Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, bytes))
                        });
                    self.projects.icons.insert(file.clone(), image);
                }
            }
        }
        if let Some(state) = next.requests.get("project-save")
            && !state.loading
            && state.number > self.projects.save_receipt
        {
            self.projects.save_receipt = state.number;
            if let Request::SaveProject { path, change } = &state.request {
                match (&state.error, change) {
                    (Some(error), Patch::Pin(true)) => {
                        self.projects.notice = format!("Couldn't pin on the hub: {error}");
                        self.projects.fallback = Some(path.clone());
                    }
                    (Some(error), _) => {
                        self.projects.notice =
                            format!("Couldn't update the hub's projects: {error}")
                    }
                    (None, Patch::Pin(true)) => self.projects.notice = "Pinned.".into(),
                    (None, Patch::Pin(false)) => self.projects.notice = "Unpinned.".into(),
                    (None, Patch::Remove) => self.projects.notice = "Removed from projects.".into(),
                    (None, Patch::Identity(identity)) => {
                        if self
                            .projects
                            .editor
                            .as_ref()
                            .is_some_and(|e| projects::same_dir(&e.path, path))
                        {
                            self.projects.editor = None;
                        }
                        self.projects.notice = if *identity == Identity::default() {
                            "Name and icon reset."
                        } else {
                            "Name and icon saved."
                        }
                        .into();
                    }
                    (None, Patch::Touch(_)) => {}
                }
            }
        }
    }

    /// A square mark in the project's colour: its configured one, else a hue
    /// derived from the path so it is stable everywhere.
    pub(super) fn project_mark(
        &self,
        project: Option<&KnownProject>,
        path: &str,
        size: f32,
    ) -> Div {
        let p = self.appearance.palette();
        let light = !self.appearance.is_dark();
        let (bg, fg): (gpui::Hsla, gpui::Hsla) = match project.and_then(|p| p.color) {
            Some(color) => {
                let solid: gpui::Hsla = rgb(color).into();
                (solid.opacity(0.22), solid)
            }
            None => {
                let h = projects::hue(path) / 360.;
                let fg = gpui::hsla(h, 0.55, if light { 0.38 } else { 0.68 }, 1.);
                (gpui::hsla(h, 0.5, if light { 0.9 } else { 0.24 }, 1.), fg)
            }
        };
        let mark = project
            .map(KnownProject::mark)
            .unwrap_or_else(|| projects::initials(projects::basename(path)));
        // A downloaded icon wins over the emoji/initials once it has loaded,
        // as on desktop; until then (or if it cannot be read) the mark stays.
        let icon = project
            .and_then(|p| p.icon_file.as_ref())
            .and_then(|file| self.projects.icons.get(file).cloned().flatten());
        if let Some(image) = icon {
            return div()
                .size(px(size))
                .flex_shrink_0()
                .rounded(px(p.control_radius.max(6.)))
                .bg(bg)
                .overflow_hidden()
                .flex()
                .items_center()
                .justify_center()
                .debug_selector(|| "project-icon-image".into())
                .child(
                    gpui::img(image)
                        .size(px((size * 0.72).round()))
                        .object_fit(gpui::ObjectFit::Contain),
                );
        }
        div()
            .size(px(size))
            .flex_shrink_0()
            .rounded(px(p.control_radius.max(6.)))
            .bg(bg)
            .text_color(fg)
            .flex()
            .items_center()
            .justify_center()
            .text_size(px((size * 0.38).round()))
            .font_weight(FontWeight::BOLD)
            .child(mark)
    }

    /// Read the downloaded icons the listed projects name and the client has
    /// not read yet (bounded batches; a failure is cached as unavailable).
    pub(super) fn ensure_project_icons(&mut self, cx: &mut Context<Self>) {
        if self.demo
            || !self.view.connected
            || self
                .view
                .requests
                .get("project-icons")
                .is_some_and(|s| s.loading)
        {
            return;
        }
        let files: Vec<String> = self
            .known_projects()
            .into_iter()
            .filter_map(|p| p.icon_file)
            .filter(|file| !self.projects.icons.contains_key(file))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .take(16)
            .collect();
        if !files.is_empty() {
            self.request(Request::ProjectIcons { files }, cx);
        }
    }

    /// The project's display name wherever a session's folder is shown: the
    /// registry's label, else the folder name.
    pub(super) fn project_name(&self, cwd: &str) -> String {
        self.projects
            .registry
            .as_deref()
            .and_then(|registry| projects::registry_label(registry, cwd))
            .unwrap_or_else(|| chrome::project_label(cwd).to_owned())
    }

    pub(super) fn open_identity_editor(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let base = self
            .known_project(&path)
            .map(|p| p.identity())
            .unwrap_or_default();
        let input =
            |value: &str, placeholder: &str, window: &mut Window, cx: &mut Context<Self>| {
                let value = value.to_owned();
                let placeholder = placeholder.to_owned();
                cx.new(|cx| {
                    let mut input = InputState::new(window, cx).placeholder(placeholder);
                    input.set_value(value, window, cx);
                    input
                })
            };
        let name = input(&base.label, projects::basename(&path), window, cx);
        let icon = input(&base.icon, "Emoji or two letters", window, cx);
        let favicon = input(
            &base.favicon,
            "https://example.com/favicon.ico (optional)",
            window,
            cx,
        );
        name.update(cx, |input, cx| input.focus(window, cx));
        self.projects.notice.clear();
        self.projects.fallback = None;
        self.projects.editor = Some(IdentityEditor {
            path,
            base,
            name,
            icon,
            favicon,
        });
        cx.notify();
    }

    /// Save the form (or reset every identity field). The favicon URL is
    /// fetched by the hub only when it changed; an unchanged URL keeps its
    /// downloaded file. Nothing is sent when nothing changed.
    pub(super) fn save_identity(&mut self, reset: bool, cx: &mut Context<Self>) {
        let Some(editor) = &self.projects.editor else {
            return;
        };
        if self.demo || !self.view.connected {
            self.projects.notice = "Connect to the hub to change its projects.".into();
            cx.notify();
            return;
        }
        if self
            .view
            .requests
            .get("project-save")
            .is_some_and(|s| s.loading)
        {
            return;
        }
        let identity = if reset {
            Identity::default()
        } else {
            let favicon = editor.favicon.read(cx).value().trim().to_owned();
            Identity {
                label: editor.name.read(cx).value().to_string(),
                icon: editor.icon.read(cx).value().to_string(),
                icon_file: if favicon == editor.base.favicon {
                    editor.base.icon_file.clone()
                } else {
                    String::new()
                },
                favicon,
            }
        };
        let identity = match identity.normalized() {
            Ok(identity) => identity,
            Err(error) => {
                self.projects.notice = error.to_string();
                cx.notify();
                return;
            }
        };
        if identity == editor.base {
            self.projects.notice = "No changes to save.".into();
            cx.notify();
            return;
        }
        let path = editor.path.clone();
        self.projects.notice = if identity.icon_file.is_empty() && !identity.favicon.is_empty() {
            "Downloading the icon on the hub…".into()
        } else {
            "Saving…".into()
        };
        self.request(
            Request::SaveProject {
                path,
                change: Patch::Identity(identity),
            },
            cx,
        );
        cx.notify();
    }

    pub(super) fn render_identity_editor(&self, cx: &mut Context<Self>) -> Option<Div> {
        let editor = self.projects.editor.as_ref()?;
        let p = self.appearance.palette();
        let saving = self
            .view
            .requests
            .get("project-save")
            .is_some_and(|s| s.loading);
        let can_write = self.view.connected && !self.demo && !saving;
        // The preview follows the form as typed.
        let mut preview = self.known_project(&editor.path).unwrap_or(KnownProject {
            path: editor.path.clone(),
            label: None,
            color: None,
            icon: None,
            favicon: None,
            icon_file: None,
            favourite: false,
            last_opened: None,
            sessions: 0,
            live_sessions: 0,
            source: Source::Hub,
            configured: false,
        });
        let name = editor.name.read(cx).value().trim().to_owned();
        let icon = editor.icon.read(cx).value().trim().to_owned();
        preview.label = (!name.is_empty()).then_some(name);
        preview.icon = (!icon.is_empty()).then_some(icon);
        if editor.favicon.read(cx).value().trim() != editor.base.favicon {
            preview.icon_file = None;
        }
        let field = |label: &'static str, input: &Entity<InputState>, selector: &'static str| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .flex_1()
                .min_w(px(160.))
                .child(section_label(label, p))
                .child(
                    div()
                        .debug_selector(move || selector.into())
                        .child(Input::new(input)),
                )
        };
        Some(
            chrome::card(p)
                .debug_selector(|| "project-identity-editor".into())
                .p_4()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(self.project_mark(Some(&preview), &editor.path, 40.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .truncate()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(preview.title().to_owned()),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .font_family(mono_font())
                                        .text_size(px(11.))
                                        .text_color(rgb(p.muted))
                                        .child(editor.path.clone()),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_3()
                        .child(field("Name", &editor.name, "project-identity-name"))
                        .child(field("Icon", &editor.icon, "project-identity-icon")),
                )
                .child(field("Icon URL", &editor.favicon, "project-identity-favicon"))
                .child(
                    div()
                        .text_size(px(chrome::scale::CAPTION))
                        .text_color(rgb(p.muted))
                        .child("Shared with Workspacer on this hub (desktop, phone and terminal clients). The hub downloads the icon URL once and keeps a copy; it wins over the emoji when it loads."),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .justify_end()
                        .gap_2()
                        .child(
                            self.danger_button("project-identity-reset", "Reset", can_write)
                                .when(can_write, |d| {
                                    d.on_click(cx.listener(|this, _, _, cx| {
                                        this.save_identity(true, cx)
                                    }))
                                }),
                        )
                        .child(self.button("project-identity-cancel", "Cancel", true).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.projects.editor = None;
                                cx.notify();
                            }),
                        ))
                        .child(
                            self.primary_button("project-identity-save", "Save", can_write)
                                .debug_selector(|| "project-identity-save".into())
                                .when(can_write, |d| {
                                    d.on_click(cx.listener(|this, _, _, cx| {
                                        this.save_identity(false, cx)
                                    }))
                                }),
                        ),
                ),
        )
    }

    fn project_facts(&self, project: Option<&KnownProject>) -> Vec<String> {
        let mut facts = Vec::new();
        if let Some(project) = project {
            if project.live_sessions > 0 {
                facts.push(format!("{} running", project.live_sessions));
            }
            let ended = project.sessions - project.live_sessions;
            if ended > 0 {
                facts.push(format!("{ended} ended"));
            }
            match project.source {
                Source::Device => facts.push("Saved on this device".into()),
                Source::Sessions if project.sessions == 0 => {}
                _ => {}
            }
        }
        facts
    }

    /// The chosen project at a glance: identity, where it lives on the hub,
    /// and what the hub says about it.
    pub(super) fn render_project_summary(
        &self,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        let cwd = self.projects.cwd.clone();
        let project = self.known_project(&cwd);
        let title = project
            .as_ref()
            .map(|p| p.title().to_owned())
            .unwrap_or_else(|| projects::basename(&cwd).to_owned());
        let pinned = project.as_ref().is_some_and(|p| p.favourite);
        let saving = self
            .view
            .requests
            .get("project-save")
            .is_some_and(|s| s.loading);
        let can_pin = !busy && !saving && self.view.connected && !self.demo;
        let (status_icon, status_color, status): (IconName, u32, String) = match self.inspection() {
            _ if self.inspecting() => (IconName::LoaderCircle, p.muted, "Checking folder…".into()),
            Some(Ok(Inspection::Repository { branch, changes })) => (
                IconName::CircleCheck,
                p.success,
                match (branch, changes) {
                    (Some(b), 0) => format!("{b} · clean"),
                    (Some(b), 1) => format!("{b} · 1 uncommitted change"),
                    (Some(b), n) => format!("{b} · {n} uncommitted changes"),
                    // Detached, or a repository with no commits yet.
                    (None, 0) => "Git repository · clean".into(),
                    (None, 1) => "Git repository · 1 uncommitted change".into(),
                    (None, n) => format!("Git repository · {n} uncommitted changes"),
                },
            ),
            Some(Ok(Inspection::NotRepository)) => (
                IconName::Folder,
                p.muted,
                "Folder · not a git repository".into(),
            ),
            Some(Ok(Inspection::GitUnknown(error))) => (
                IconName::Info,
                p.muted,
                format!("Folder found · git unavailable ({error})"),
            ),
            Some(Ok(Inspection::Missing(_))) => (
                IconName::TriangleAlert,
                p.warning,
                "Not found on the hub. Choose another folder, or create it first.".into(),
            ),
            Some(Err(error)) => (
                IconName::Info,
                p.muted,
                format!("Couldn't check this folder: {error}"),
            ),
            None => (IconName::Folder, p.muted, "On the connected hub".into()),
        };
        let facts = self.project_facts(project.as_ref());
        let pin_path = cwd.clone();
        div()
            .id("launch-project-summary")
            .debug_selector(|| "launch-project-summary".into())
            .flex()
            .items_center()
            .gap_3()
            .min_w_0()
            .child(self.project_mark(project.as_ref(), &cwd, 40.))
            .child(
                div()
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
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(title),
                            )
                            .children(facts.into_iter().map(|fact| {
                                div()
                                    .flex_shrink_0()
                                    .px(px(6.))
                                    .rounded_full()
                                    .bg(rgb(p.selected))
                                    .text_size(px(10.))
                                    .text_color(rgb(p.muted))
                                    .child(fact)
                            })),
                    )
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .font_family(mono_font())
                            .text_size(px(11.))
                            .text_color(rgb(p.muted))
                            .child(cwd.clone()),
                    )
                    .child(
                        div()
                            .debug_selector(|| "launch-project-status".into())
                            .w_full()
                            .flex()
                            .items_start()
                            .gap_1()
                            .text_size(px(11.))
                            .line_height(gpui::relative(1.4))
                            .text_color(rgb(status_color))
                            // Wraps: a missing-folder warning carries its
                            // recovery step, which must not be clipped.
                            .child(
                                div()
                                    .h(px(11. * 1.4))
                                    .flex()
                                    .items_center()
                                    .flex_shrink_0()
                                    .child(Icon::new(status_icon).size(px(11.))),
                            )
                            .child(div().flex_1().min_w_0().child(status)),
                    ),
            )
            .child(
                self.icon_button(
                    "launch-project-pin",
                    if pinned {
                        "Unpin project"
                    } else {
                        "Pin to projects"
                    },
                    IconName::Star,
                    can_pin,
                )
                .debug_selector(|| "launch-project-pin".into())
                .when(pinned, |d| d.text_color(rgb(p.accent)))
                .when(can_pin, |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| {
                        this.set_project_pin(pin_path.clone(), !pinned, cx)
                    }))
                }),
            )
            .child(
                self.button("launch-project-change", "Change", !busy)
                    .debug_selector(|| "launch-project-change".into())
                    .flex_shrink_0()
                    .when(!busy, |d| {
                        d.on_click(
                            cx.listener(|this, _, window, cx| this.open_project_picker(window, cx)),
                        )
                    }),
            )
    }

    /// Search, recent/pinned projects, a typed path, and browse.
    pub(super) fn render_project_picker(&self, busy: bool, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let rows = self.pick_rows(cx);
        let has_rows = !rows.is_empty();
        let query = self.project_query.read(cx).value().trim().to_owned();
        let registry_loading = self
            .view
            .requests
            .get("projects")
            .is_some_and(|s| s.loading)
            && self.projects.registry.is_none();
        let cursor = self.projects.cursor.min(rows.len().saturating_sub(1));
        let can_cancel = !self.projects.cwd.is_empty();
        let browse_label = if self.extras.local_paths {
            "Choose folder…"
        } else {
            "Browse hub…"
        };
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .debug_selector(|| "launch-project-search".into())
                    .child(
                        Input::new(&self.project_query)
                            .prefix(
                                Icon::new(IconName::Search)
                                    .size(px(14.))
                                    .text_color(rgb(p.muted)),
                            )
                            .disabled(busy),
                    ),
            )
            .child(
                self.quiet_button(
                    "launch-project-browse",
                    browse_label,
                    IconName::FolderOpen,
                    !busy && (self.extras.local_paths || self.view.connected),
                )
                .debug_selector(|| "launch-project-browse".into())
                .when(!busy, |d| {
                    d.on_click(cx.listener(|this, _, window, cx| this.browse_projects(window, cx)))
                }),
            )
            .when(can_cancel, |d| {
                d.child(
                    self.quiet_button("launch-project-cancel", "Cancel", IconName::Close, !busy)
                        .debug_selector(|| "launch-project-cancel".into())
                        .when(!busy, |d| {
                            d.on_click(cx.listener(|this, _, window, cx| {
                                this.close_project_picker(window, cx);
                            }))
                        }),
                )
            });
        let body = if self.projects.browsing {
            self.render_folder_browser(busy, cx).into_any_element()
        } else if rows.is_empty() {
            div()
                .debug_selector(|| "launch-project-empty".into())
                .py_5()
                .flex()
                .flex_col()
                .items_center()
                .gap_2()
                .text_center()
                .child(
                    Icon::new(if registry_loading {
                        IconName::LoaderCircle
                    } else {
                        IconName::FolderOpen
                    })
                    .size(px(20.))
                    .text_color(rgb(p.muted)),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(if registry_loading {
                            "Loading projects…".to_owned()
                        } else if query.is_empty() {
                            "No projects yet".to_owned()
                        } else {
                            format!("No projects match “{query}”")
                        }),
                )
                .when(!registry_loading, |d| {
                    d.child(div().text_size(px(12.)).text_color(rgb(p.muted)).child(
                        "Paste an absolute folder path above, or browse for one on the hub.",
                    ))
                })
                .into_any_element()
        } else {
            let list_height = rows.len().min(VISIBLE_ROWS) as f32 * ROW_HEIGHT;
            let current = self.projects.cwd.clone();
            div()
                .h(px(list_height))
                .child(
                    uniform_list(
                        "launch-project-list",
                        rows.len(),
                        cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                            range
                                .map(|ix| {
                                    this.render_pick_row(
                                        ix,
                                        &rows[ix],
                                        ix == cursor,
                                        projects::same_dir(rows[ix].path(), &current),
                                        busy,
                                        cx,
                                    )
                                })
                                .collect::<Vec<_>>()
                        }),
                    )
                    .track_scroll(self.project_list_scroll.clone())
                    .h_full(),
                )
                .into_any_element()
        };
        div()
            .debug_selector(|| "launch-project-picker".into())
            .flex()
            .flex_col()
            .gap_2()
            // Arrow keys move through the list while the search field keeps
            // focus, like a command palette.
            .capture_action(cx.listener(|this, _: &MoveUp, window, cx| {
                if this.project_query.read(cx).focus_handle(cx).is_focused(window) && !this.projects.browsing {
                    this.move_project_cursor(-1, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, window, cx| {
                if this.project_query.read(cx).focus_handle(cx).is_focused(window) && !this.projects.browsing {
                    this.move_project_cursor(1, cx);
                    cx.stop_propagation();
                }
            }))
            .child(header)
            .when_some(self.projects.registry_error.clone(), |d, error| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(11.))
                        .text_color(rgb(p.warning))
                        .child(Icon::new(IconName::TriangleAlert).size(px(12.)).flex_shrink_0())
                        .child(div().flex_1().min_w_0().child(format!(
                            "Couldn't read the hub's projects ({error}). Showing this device's projects and active folders."
                        )))
                        .child(
                            self.quiet_button("launch-project-retry", "Retry", IconName::Redo, self.view.connected)
                                .when(self.view.connected, |d| {
                                    d.on_click(cx.listener(|this, _, _, cx| this.load_projects(cx)))
                                }),
                        ),
                )
            })
            .child(body)
            .when(!self.projects.browsing && has_rows, |d| {
                d.child(div().text_size(px(11.)).text_color(rgb(p.muted)).child(
                    "↑ ↓ to move · Enter to choose · paths belong to the connected hub",
                ))
            })
    }

    fn render_pick_row(
        &self,
        ix: usize,
        row: &PickRow,
        highlighted: bool,
        current: bool,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        let path = row.path().to_owned();
        let choose = path.clone();
        let saving = self
            .view
            .requests
            .get("project-save")
            .is_some_and(|s| s.loading);
        let enabled = !busy;
        let base =
            chrome::interactive_control(div().id(("launch-project-row", ix)), p, enabled)
                .debug_selector(move || format!("launch-project-row-{ix}"))
                .w_full()
                .h(px(ROW_HEIGHT - 4.))
                .mb(px(4.))
                .px_2()
                .rounded(px(p.control_radius))
                .flex()
                .items_center()
                .gap_3()
                .bg(rgb(if highlighted { p.selected } else { p.surface }))
                .when(enabled, |d| {
                    d.hover(|s| s.bg(rgb(p.selected))).on_click(cx.listener(
                        move |this, _, window, cx| this.select_project(&choose, window, cx),
                    ))
                });
        match row {
            PickRow::Typed(path) | PickRow::Current(path) => base
                .child(
                    div()
                        .size(px(32.))
                        .flex_shrink_0()
                        .rounded(px(p.control_radius.max(6.)))
                        .bg(rgb(p.selected))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            Icon::new(IconName::FolderOpen)
                                .size(px(16.))
                                .text_color(rgb(p.accent)),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(if matches!(row, PickRow::Current(_)) {
                                    "Current folder"
                                } else {
                                    "Use this folder"
                                }),
                        )
                        .child(
                            div()
                                .truncate()
                                .font_family(mono_font())
                                .text_size(px(11.))
                                .text_color(rgb(p.muted))
                                .child(path.clone()),
                        ),
                ),
            PickRow::Project(project) => {
                let pinned = project.favourite;
                let pin_path = project.path.clone();
                let can_pin = enabled && !saving && self.view.connected && !self.demo;
                let detail = match (project.live_sessions, project.sessions) {
                    (0, 0) => None,
                    (0, n) => Some(format!("{n} ended")),
                    (live, _) => Some(format!("{live} running")),
                };
                base.child(self.project_mark(Some(project), &project.path, 32.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
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
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(project.title().to_owned()),
                                    )
                                    .when(project.source == Source::Device, |d| {
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
                            ),
                    )
                    .when_some(detail, |d, detail| {
                        d.child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(11.))
                                .text_color(rgb(if project.live_sessions > 0 {
                                    p.busy
                                } else {
                                    p.muted
                                }))
                                .child(detail),
                        )
                    })
                    .when(current, |d| {
                        d.child(
                            Icon::new(IconName::Check)
                                .size(px(14.))
                                .text_color(rgb(p.accent)),
                        )
                    })
                    .child(
                        self.icon_button(
                            SharedString::from(format!("launch-project-pin-{ix}")),
                            if pinned {
                                "Unpin project"
                            } else {
                                "Pin to projects"
                            },
                            IconName::Star,
                            can_pin,
                        )
                        .debug_selector(move || format!("launch-project-pin-{ix}"))
                        .when(pinned, |d| d.text_color(rgb(p.accent)))
                        .when(!pinned && !highlighted, |d| d.opacity(0.45))
                        .when(can_pin, |d| {
                            d.on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.set_project_pin(pin_path.clone(), !pinned, cx)
                            }))
                        }),
                    )
            }
        }
    }

    /// One level of the hub's filesystem. Choosing never creates anything.
    fn render_folder_browser(&self, busy: bool, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let (loading, listing, error) = self.listing();
        let path = listing.as_ref().map(|l| l.path.clone()).unwrap_or_default();
        let parent = listing
            .as_ref()
            .map(|l| l.parent.clone())
            .unwrap_or_default();
        let home = listing.as_ref().map(|l| l.home.clone()).unwrap_or_default();
        let can_up = !loading && !parent.is_empty() && parent != path;
        let can_use = !busy && !loading && !path.is_empty() && error.is_none();
        let dirs = listing.map(|l| l.dirs).unwrap_or_default();
        let use_path = path.clone();
        div()
            .debug_selector(|| "launch-project-browser".into())
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .rounded(px(p.control_radius))
            .border_1()
            .border_color(rgb(p.border))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        self.icon_button("browse-up", "Parent folder", IconName::ArrowUp, can_up)
                            .when(can_up, |d| {
                                d.on_click(cx.listener(move |this, _, _, cx| {
                                    this.browse_to(parent.clone(), cx)
                                }))
                            }),
                    )
                    .child(
                        self.icon_button(
                            "browse-home",
                            "Home folder",
                            IconName::Inbox,
                            !loading && !home.is_empty(),
                        )
                        .when(!loading && !home.is_empty(), |d| {
                            d.on_click(
                                cx.listener(move |this, _, _, cx| this.browse_to(home.clone(), cx)),
                            )
                        }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(mono_font())
                            .text_size(px(12.))
                            .child(if path.is_empty() {
                                "…".to_owned()
                            } else {
                                path.clone()
                            }),
                    ),
            )
            .child(
                div()
                    .id("launch-browser-dirs")
                    .h(px(4. * 32.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .when(loading, |d| {
                        d.child(
                            div()
                                .p_2()
                                .text_size(px(12.))
                                .text_color(rgb(p.muted))
                                .child("Loading folders…"),
                        )
                    })
                    .when_some(error.clone(), |d, error| {
                        d.child(
                            div()
                                .p_2()
                                .text_size(px(12.))
                                .text_color(rgb(p.warning))
                                .child(format!("Couldn't open this folder: {error}")),
                        )
                    })
                    .when(!loading && error.is_none() && dirs.is_empty(), |d| {
                        d.child(
                            div()
                                .p_2()
                                .text_size(px(12.))
                                .text_color(rgb(p.muted))
                                .child("No subfolders"),
                        )
                    })
                    .when(!loading, |d| {
                        d.children(dirs.into_iter().enumerate().map(|(ix, name)| {
                            let target = projects::child(&path, &name);
                            chrome::interactive_control(div().id(("browse-dir", ix)), p, true)
                                .debug_selector(move || format!("launch-browse-dir-{ix}"))
                                .h(px(30.))
                                .px_2()
                                .flex_shrink_0()
                                .rounded(px(p.control_radius))
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_size(px(12.))
                                .hover(|s| s.bg(rgb(p.selected)))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.browse_to(target.clone(), cx)
                                }))
                                .child(
                                    Icon::new(IconName::Folder)
                                        .size(px(13.))
                                        .text_color(rgb(p.muted)),
                                )
                                .child(div().min_w_0().truncate().child(name))
                        }))
                    }),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        self.button("browse-cancel", "Back to projects", true)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.projects.browsing = false;
                                this.project_query
                                    .update(cx, |input, cx| input.focus(window, cx));
                                cx.notify();
                            })),
                    )
                    .child(
                        self.primary_button("browse-use", "Use this folder", can_use)
                            .debug_selector(|| "launch-browse-use".into())
                            .when(can_use, |d| {
                                d.on_click(cx.listener(move |this, _, window, cx| {
                                    this.select_project(&use_path, window, cx)
                                }))
                            }),
                    ),
            )
    }

    /// The Project card: summary when chosen, the chooser otherwise, and the
    /// result of the user's last pin/forget.
    pub(super) fn render_project_section(&self, busy: bool, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(section_label("Project", p))
            .child(
                if self.projects.picker_open || self.projects.cwd.is_empty() {
                    self.render_project_picker(busy, cx).into_any_element()
                } else {
                    self.render_project_summary(busy, cx).into_any_element()
                },
            )
            .when(!self.projects.notice.is_empty(), |d| {
                d.child(
                    div()
                        .debug_selector(|| "launch-project-notice".into())
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(11.))
                        .text_color(rgb(if self.projects.fallback.is_some() {
                            p.warning
                        } else {
                            p.muted
                        }))
                        .child(self.projects.notice.clone())
                        .when(self.projects.fallback.is_some(), |d| {
                            d.child(
                                self.quiet_button(
                                    "keep-on-device",
                                    "Keep on this device",
                                    IconName::Check,
                                    true,
                                )
                                .debug_selector(|| "launch-project-keep-device".into())
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.keep_project_on_device(cx)),
                                ),
                            )
                        }),
                )
            })
    }
}

pub(super) fn section_label(label: &'static str, p: Palette) -> Div {
    div()
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(p.muted))
        .child(label)
}
