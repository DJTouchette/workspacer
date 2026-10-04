//! Where chat links lead: the system browser for web links, and a read-only
//! viewer for files and images. Files are read on the session's machine by
//! the connected hub (`file-preview` request), never from this client's disk.
//!
//! The viewer is one [`PreviewPane`] per window. In the main window it docks
//! beside the conversation when there is room and covers it as a modal sheet
//! when there is not; "Pop out" moves it into a window of its own. Markdown
//! documents open rendered, with their read-only source one toggle away.
use super::*;
use base64::Engine;
use gpui::{AnyElement, KeyDownEvent, Pixels, WeakEntity, WindowHandle, deferred};
use gpui_component::Root;
use gpui_component::input::{Input, InputState, Position};
use gpui_component::text::{LinkClickFn, TextViewHandle};
use wks_native::{
    features::{Request, RequestState},
    links::{self, DocumentLink, FileKind, FileTarget, Link},
};

const KEY: &str = "file-preview";
/// The conversation keeps at least this much width beside a docked viewer.
const MIN_CHAT_BESIDE: f32 = 400.;
/// Rendered documents keep a comfortable reading measure.
const READING_WIDTH: f32 = 760.;
/// How long the viewer takes to slide in from the right.
const SLIDE: std::time::Duration = std::time::Duration::from_millis(240);

/// Fast start, gentle stop, without ease-out-quint's front-loading (most of
/// its travel lands in the first frame or two, which reads as a pop).
fn ease_out_cubic(t: f32) -> f32 {
    1. - (1. - t).powi(3)
}

/// How a pane is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Presentation {
    /// Modal, over the conversation (narrow windows).
    Sheet,
    /// Beside the conversation, which stays usable.
    Docked,
    /// The root of its own window.
    Window,
}

/// What a Markdown file shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Preview,
    Source,
}

/// Viewer state the workspace owns: the main window's pane and the popped
/// out window, at most one of which is showing.
#[derive(Default)]
pub(super) struct FileViewer {
    pane: Option<Entity<PreviewPane>>,
    popout: Option<Popout>,
    /// Bumped each time the main window's pane opens, so it slides in once
    /// rather than on every file it navigates to.
    opened: u64,
}

struct Popout {
    window: WindowHandle<Root>,
    pane: Entity<PreviewPane>,
}

/// One window's viewer: a snapshot of the request it shows plus the widgets
/// built for that window (source editor, rendered document, image).
pub(super) struct PreviewPane {
    workspace: WeakEntity<Workspace>,
    pub(super) presentation: Presentation,
    /// Laid-out width, for the document's reading margins.
    width: Pixels,
    state: RequestState,
    /// Request number whose content the widgets below were built from.
    shown: u64,
    mode: Mode,
    /// Markdown small enough to render (larger documents show source).
    renderable: bool,
    contents: SharedString,
    editor: Option<Entity<InputState>>,
    image: Option<(Arc<gpui::Image>, u64, u64)>,
    document: Option<TextViewHandle>,
    /// A refused link or missing heading, shown under the header.
    notice: Option<SharedString>,
    /// Files this pane navigated away from, for Back.
    history: Vec<FileTarget>,
    /// The next request was issued from this pane (a link or Back).
    navigation: Option<Navigation>,
    /// New content arrived; the window should focus it.
    wants_focus: bool,
    focus: FocusHandle,
    /// Keep the discovery identity fixed until the platform watch attaches.
    title_ready: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Navigation {
    Forward,
    Back,
}

impl PreviewPane {
    fn new(
        workspace: WeakEntity<Workspace>,
        presentation: Presentation,
        state: &RequestState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut pane = Self {
            workspace,
            presentation,
            width: px(READING_WIDTH),
            state: state.clone(),
            shown: 0,
            mode: Mode::Source,
            renderable: false,
            contents: SharedString::default(),
            editor: None,
            image: None,
            document: None,
            notice: None,
            history: vec![],
            navigation: None,
            wants_focus: true,
            focus: cx.focus_handle(),
            title_ready: presentation != Presentation::Window,
        };
        pane.load(state, window, cx);
        pane
    }

    fn target(&self) -> Option<&FileTarget> {
        match &self.state.request {
            Request::FilePreview { target, .. } => Some(target),
            _ => None,
        }
    }

    fn session(&self) -> &str {
        match &self.state.request {
            Request::FilePreview { session, .. } => session,
            _ => "",
        }
    }

    /// A Markdown document whose rendered preview can be shown.
    pub(super) fn previewable(&self) -> bool {
        self.renderable && self.editor.is_some()
    }

    pub(super) fn has_focus(&self, window: &Window, cx: &App) -> bool {
        self.focus.contains_focused(window, cx)
            || self
                .editor
                .as_ref()
                .is_some_and(|e| e.read(cx).focus_handle(cx).contains_focused(window, cx))
    }

    /// Focus what the keyboard should drive: the source editor, or the pane
    /// itself (document scrolling, Esc) while loading or previewing.
    pub(super) fn focus_content(&self, window: &mut Window, cx: &App) {
        match (&self.editor, self.mode) {
            (Some(editor), Mode::Source) if self.shown == self.state.number => {
                editor.read(cx).focus_handle(cx).focus(window)
            }
            _ => window.focus(&self.focus),
        }
    }

    /// Adopt a newer state of the request, or a new request. Widgets are
    /// rebuilt only when a request finishes loading.
    fn load(&mut self, state: &RequestState, window: &mut Window, cx: &mut Context<Self>) {
        if state.number != self.state.number {
            self.notice = None;
            let previous = self.target().cloned().filter(|_| self.shown != 0);
            match self.navigation.take() {
                Some(Navigation::Forward) => self.history.extend(previous),
                Some(Navigation::Back) => {}
                None => self.history.clear(),
            }
            // The previous file's widgets go now, not when the next one
            // loads: focus must never rest on an editor no longer shown.
            let focused = self.has_focus(window, cx);
            self.clear_content();
            if focused {
                window.focus(&self.focus);
            }
        }
        self.state = state.clone();
        cx.notify();
        if state.loading || state.number == self.shown {
            return;
        }
        self.clear_content();
        self.shown = state.number;
        self.wants_focus = true;
        if state.error.is_some() {
            return;
        }
        let Some(target) = self.target().cloned() else {
            return;
        };
        if self.presentation == Presentation::Window && self.title_ready {
            window.set_window_title(&popout_title(&target));
        }
        match target.kind {
            FileKind::Text => {
                let contents = state.value["contents"].as_str().unwrap_or("").to_owned();
                let markdown = target.markdown();
                self.renderable = markdown && contents.len() <= links::MAX_RENDERED_MARKDOWN_BYTES;
                if markdown && !self.renderable {
                    self.notice = Some(
                        format!(
                            "This document is {}; Markdown renders up to {}, so it opens as source.",
                            links::size(contents.len() as u64),
                            links::size(links::MAX_RENDERED_MARKDOWN_BYTES as u64)
                        )
                        .into(),
                    );
                }
                // A line anchor is a request for that line: it opens the
                // source there. Otherwise documents open rendered.
                self.mode = if self.renderable && target.line.is_none() {
                    Mode::Preview
                } else {
                    Mode::Source
                };
                self.contents = contents.clone().into();
                let language = target.language();
                let editor = cx.new(|cx| {
                    InputState::new(window, cx)
                        .code_editor(language)
                        .line_number(true)
                        .searchable(true)
                        .soft_wrap(false)
                        .default_value(contents)
                });
                self.editor = Some(editor);
                self.place_cursor(target.line, target.column, window, cx);
            }
            FileKind::Image => {
                let image = state.value["png"]
                    .as_str()
                    .and_then(|png| base64::engine::general_purpose::STANDARD.decode(png).ok())
                    .map(|bytes| Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, bytes)));
                if let Some(image) = image {
                    self.image = Some((
                        image,
                        state.value["width"].as_u64().unwrap_or(0),
                        state.value["height"].as_u64().unwrap_or(0),
                    ));
                }
            }
        }
    }

    fn clear_content(&mut self) {
        self.shown = 0;
        self.editor = None;
        self.image = None;
        self.document = None;
        self.contents = SharedString::default();
        self.renderable = false;
        self.mode = Mode::Source;
    }

    /// The request this pane was waiting for will never arrive here.
    fn interrupt(&mut self, cx: &mut Context<Self>) {
        if self.state.loading {
            self.state.loading = false;
            self.state.error = Some(
                "Loading stopped before the file arrived (the session changed). Open the link again."
                    .into(),
            );
            self.navigation = None;
            cx.notify();
        }
    }

    /// Put the source cursor (and active-line band) on a 1-based line.
    fn place_cursor(
        &self,
        line: Option<u32>,
        column: Option<u32>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(editor) = &self.editor else {
            return;
        };
        let position = Position::new(
            line.unwrap_or(1).saturating_sub(1),
            column.unwrap_or(1).saturating_sub(1),
        );
        move_cursor(editor, position, window, cx);
        if line.is_some() {
            reveal(editor.downgrade(), position, 2, window);
        }
    }

    pub(super) fn set_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        if mode == self.mode || (mode == Mode::Preview && !self.previewable()) {
            return;
        }
        let focused = self.has_focus(window, cx);
        self.mode = mode;
        if focused {
            self.focus_content(window, cx);
        }
        cx.notify();
    }

    /// Show the source at a line of this same document.
    fn show_line(
        &mut self,
        line: u32,
        column: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_mode(Mode::Source, window, cx);
        self.place_cursor(Some(line), column, window, cx);
        self.focus_content(window, cx);
    }

    /// Ctrl+F from the document: search its source.
    fn find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.clone() else {
            return;
        };
        self.set_mode(Mode::Source, window, cx);
        // Opened directly, not by dispatching `Search`: the source editor is
        // not in the rendered dispatch tree until the next frame, and an
        // action dispatched before then never reaches it. Search focuses
        // its own query field.
        editor.update(cx, |editor, cx| editor.open_search(window, cx));
    }

    /// A link clicked inside the rendered document.
    pub(super) fn follow_link(&mut self, raw: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.target().cloned() else {
            return;
        };
        self.notice = None;
        match links::classify_in_document(&target.path, raw) {
            DocumentLink::Heading(slug) => self.scroll_to_heading(&slug, cx),
            DocumentLink::Line(line, column) => self.show_line(line, column, window, cx),
            DocumentLink::Other(Link::Web(url)) => cx.open_url(&url),
            DocumentLink::Other(Link::Anchor) => {}
            DocumentLink::Other(Link::Refused(message)) => self.notice = Some(message.into()),
            DocumentLink::Other(Link::File(next)) => {
                self.open(next, Navigation::Forward, cx);
            }
        }
        cx.notify();
    }

    /// Read another file from the same session through the workspace.
    fn open(&mut self, target: FileTarget, navigation: Navigation, cx: &mut Context<Self>) {
        let session = self.session().to_owned();
        let result = self
            .workspace
            .update(cx, |ws, cx| ws.request_preview(&session, target, cx))
            .unwrap_or_else(|_| Err("The workspace closed.".into()));
        match result {
            Ok(()) => self.navigation = Some(navigation),
            Err(message) => self.notice = Some(message.into()),
        }
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        if let Some(target) = self.history.pop() {
            self.open(target, Navigation::Back, cx);
            cx.notify();
        }
    }

    fn scroll_to_heading(&mut self, slug: &str, cx: &mut Context<Self>) {
        let Some(document) = self.document.clone() else {
            return;
        };
        if slug.is_empty() {
            document.scroll_to_block(0, cx);
            return;
        }
        let headings = document.headings(cx);
        let slugs = links::heading_slugs(headings.iter().map(|(_, _, text)| text.as_str()));
        match slugs.iter().position(|s| s == slug) {
            Some(i) => document.scroll_to_block(headings[i].0, cx),
            None => self.notice = Some(format!("No heading “#{slug}” in this document.").into()),
        }
    }

    /// Arrow, page and Home/End keys scroll the rendered document.
    fn scroll_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(document) = self.document.clone().filter(|_| self.mode == Mode::Preview) else {
            return false;
        };
        let m = &event.keystroke.modifiers;
        if m.control || m.alt || m.platform || m.function {
            return false;
        }
        let page = document.viewport_height(cx).max(px(120.)) * 0.85;
        let line = px(48.);
        match (event.keystroke.key.as_str(), m.shift) {
            ("down", _) => document.scroll_by(line, cx),
            ("up", _) => document.scroll_by(-line, cx),
            ("pagedown", _) | ("space", false) => document.scroll_by(page, cx),
            ("pageup", _) | ("space", true) => document.scroll_by(-page, cx),
            ("home", _) => document.scroll_to_block(0, cx),
            ("end", _) => document.scroll_to_block(usize::MAX, cx),
            _ => return false,
        }
        cx.notify();
        true
    }

    /// Run a workspace action after this pane's update ends (the action
    /// reads or replaces the pane). From a popped-out window it runs in the
    /// main window, where the workspace's focus lives.
    fn workspace_call(
        &self,
        window: &mut Window,
        cx: &mut App,
        f: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
    ) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        if self.presentation == Presentation::Window {
            let main = workspace.read(cx).main_window;
            cx.defer(move |cx| {
                if let Some(main) = main {
                    let _ = main.update(cx, |_, window, cx| {
                        workspace.update(cx, |ws, cx| f(ws, window, cx))
                    });
                }
            });
        } else {
            window.defer(cx, move |window, cx| {
                workspace.update(cx, |ws, cx| f(ws, window, cx))
            });
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.presentation == Presentation::Window {
            window.remove_window();
        }
        self.workspace_call(window, cx, |ws, window, cx| {
            ws.close_file_viewer(window, cx)
        });
    }

    fn render_body(
        &mut self,
        p: Palette,
        text_size: f32,
        style: Option<gpui_component::text::TextViewStyle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let target = self.target().cloned();
        let name = target
            .as_ref()
            .map(|t| t.name().to_owned())
            .unwrap_or_default();
        if let Some(error) = &self.state.error {
            return div()
                .debug_selector(|| "file-viewer-error".into())
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .p_6()
                .child(
                    Icon::new(IconName::TriangleAlert)
                        .size(px(20.))
                        .text_color(rgb(p.warning)),
                )
                .child(
                    div()
                        .max_w(px(480.))
                        .text_size(px(13.))
                        .text_color(rgb(p.text))
                        .child(error.clone()),
                )
                .into_any_element();
        }
        if self.state.loading || self.state.number != self.shown {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .gap_2()
                .text_size(px(12.))
                .text_color(rgb(p.muted))
                .child(brand_spinner(14., p, "file-viewer-loading"))
                .child(format!("Loading {name}…"))
                .into_any_element();
        }
        if let Some(style) = style {
            let view = TextView::markdown(
                SharedString::from(format!("file-preview-markdown-{}", self.shown)),
                self.contents.clone(),
                window,
                cx,
            )
            .style(style)
            .selectable(true)
            .scrollable(true);
            let document = view.handle();
            self.document = Some(document.clone());
            if self.mode == Mode::Source {
                // Not drawn; constructing it each frame keeps its state.
                return self.render_source(p, text_size, cx);
            }
            let margin = ((self.width - px(READING_WIDTH)) / 2.).max(px(20.));
            return div()
                .id("file-viewer-markdown")
                .debug_selector(|| "file-viewer-markdown".into())
                .flex_1()
                .min_h_0()
                .bg(rgb(p.chat))
                .text_color(rgb(p.prose))
                .text_size(px(text_size))
                .line_height(gpui::relative(1.65))
                // The side margins scroll the document too.
                .on_scroll_wheel(move |event, window, cx| {
                    let bounds = document.viewport_bounds(cx);
                    if !bounds.contains(&event.position) {
                        let delta = event.delta.pixel_delta(window.line_height());
                        document.scroll_by(-delta.y, cx);
                        window.refresh();
                    }
                })
                .child(view.px(margin).pt_4())
                .into_any_element();
        }
        if self.editor.is_some() {
            return self.render_source(p, text_size, cx);
        }
        if let Some((image, ..)) = &self.image {
            return div()
                .debug_selector(|| "file-viewer-image".into())
                .flex_1()
                .min_h_0()
                .p_4()
                .bg(rgb(p.code_block))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    gpui::img(image.clone())
                        .size_full()
                        .object_fit(gpui::ObjectFit::ScaleDown),
                )
                .into_any_element();
        }
        div()
            .flex_1()
            .p_6()
            .text_color(rgb(p.warning))
            .child("This preview could not be displayed.")
            .into_any_element()
    }

    fn render_source(&self, p: Palette, text_size: f32, cx: &App) -> AnyElement {
        let Some(editor) = &self.editor else {
            return div().into_any_element();
        };
        div()
            .id("file-viewer-text")
            .debug_selector(|| "file-viewer-text".into())
            .flex_1()
            .min_h_0()
            .bg(rgb(p.code_block))
            .text_size(px(text_size - 1.))
            .font_family(gpui_component::Theme::global(cx).mono_font_family.clone())
            .child(Input::new(editor).disabled(true).appearance(false).h_full())
            .into_any_element()
    }

    /// Preview / Source, a segmented control for Markdown documents.
    fn render_modes(&self, p: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let markdown = self.target().is_some_and(FileTarget::markdown)
            && self.editor.is_some()
            && self.state.error.is_none();
        if !markdown {
            return None;
        }
        let segment = |mode: Mode, label: &'static str, enabled: bool| {
            let active = self.mode == mode;
            div()
                .id(match mode {
                    Mode::Preview => "file-viewer-mode-preview",
                    Mode::Source => "file-viewer-mode-source",
                })
                .debug_selector(move || {
                    match mode {
                        Mode::Preview => "file-viewer-mode-preview",
                        Mode::Source => "file-viewer-mode-source",
                    }
                    .into()
                })
                .px_2()
                .py(px(3.))
                .rounded(px(p.control_radius))
                .text_size(px(12.))
                .when(active, |d| {
                    d.bg(rgb(p.surface))
                        .text_color(rgb(p.text))
                        .shadow(chrome::floating_shadow(p))
                })
                .when(!active, |d| {
                    d.text_color(rgb(if enabled { p.muted } else { p.disabled }))
                })
                .when(enabled && !active, |d| {
                    d.cursor_pointer()
                        .hover(|s| s.text_color(rgb(p.text)))
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.set_mode(mode, window, cx)),
                        )
                })
                .child(label)
        };
        let shortcut = if cfg!(target_os = "macos") {
            "⌘⇧V"
        } else {
            "Ctrl+Shift+V"
        };
        Some(
            div()
                .id("file-viewer-modes")
                .flex()
                .items_center()
                .gap(px(2.))
                .p(px(2.))
                .rounded(px(p.control_radius + 2.))
                .bg(rgb(p.selected))
                .flex_shrink_0()
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(format!(
                        "Rendered document or its source ({shortcut})"
                    ))
                    .build(window, cx)
                })
                .child(segment(Mode::Preview, "Preview", self.renderable))
                .child(segment(Mode::Source, "Source", true))
                .into_any_element(),
        )
    }
}

impl Render for PreviewPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(workspace) = self.workspace.upgrade() else {
            return div().into_any_element();
        };
        if self.presentation == Presentation::Window {
            self.width = window.viewport_size().width;
        }
        // Built in Source mode too: the document's keyed state (parse,
        // scroll position) then survives a round trip through the source.
        let previewing = self.previewable()
            && !self.state.loading
            && self.state.error.is_none()
            && self.state.number == self.shown;
        let pane = cx.entity().downgrade();
        let (p, text_size, interface_size, style, back, place, copy, close) = {
            let ws = workspace.read(cx);
            let style = previewing.then(|| {
                let on_link: Arc<LinkClickFn> = Arc::new(move |url, window, cx| {
                    let _ = pane.update(cx, |pane, cx| pane.follow_link(url, window, cx));
                });
                ws.markdown_style(on_link, true, cx)
            });
            let place = if self.presentation == Presentation::Window {
                ws.icon_button(
                    "file-viewer-dock",
                    "Dock beside the chat",
                    IconName::PanelRight,
                    true,
                )
                .debug_selector(|| "file-viewer-dock".into())
            } else {
                ws.icon_button(
                    "file-viewer-popout",
                    "Open in a separate window",
                    IconName::ExternalLink,
                    true,
                )
                .debug_selector(|| "file-viewer-popout".into())
            };
            (
                ws.appearance.palette(),
                ws.settings.text_size as f32,
                ws.settings.text_size.clamp(12, 20) as f32,
                style,
                ws.icon_button("file-viewer-back", "Back", IconName::ArrowLeft, true)
                    .debug_selector(|| "file-viewer-back".into()),
                place,
                ws.icon_button("file-viewer-copy", "Copy path", IconName::Copy, true),
                ws.icon_button("file-viewer-close", "Close", IconName::Close, true)
                    .debug_selector(|| "file-viewer-close".into()),
            )
        };
        let body = self.render_body(p, text_size, style, window, cx);
        let Some(target) = self.target().cloned() else {
            return div().into_any_element();
        };
        let mut details = vec![];
        if let Some(line) = target.line {
            details.push(match target.column {
                Some(column) => format!("Line {line}, column {column}"),
                None => format!("Line {line}"),
            });
        }
        if let Some((_, width, height)) = &self.image
            && *width > 0
        {
            details.push(format!("{width} × {height}"));
        }
        if let Some(size) = self.state.value["size"]
            .as_u64()
            .filter(|_| !self.state.loading)
        {
            details.push(links::size(size));
        }
        details.push("Read-only".into());
        let path = target.path.clone();
        let window_pane = self.presentation == Presentation::Window;
        let modes = self.render_modes(p, cx);
        let back = (!self.history.is_empty())
            .then(|| back.on_click(cx.listener(|this, _, _, cx| this.back(cx))));
        let place = if window_pane {
            place.on_click(cx.listener(|this, _, window, cx| {
                this.workspace_call(window, cx, |ws, window, cx| ws.dock_popout(window, cx));
            }))
        } else {
            place.on_click(cx.listener(|this, _, window, cx| {
                this.workspace_call(window, cx, |ws, window, cx| ws.pop_out(window, cx));
            }))
        };
        let copy = copy.on_click(move |_, _, cx| {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(path.clone()))
        });
        let close = close.on_click(cx.listener(|this, _, window, cx| this.close(window, cx)));
        let header = div()
            .px_4()
            .py_3()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(rgb(p.border))
            .children(back)
            .child(
                Icon::new(match (&target.kind, target.markdown()) {
                    (FileKind::Image, _) => IconName::GalleryVerticalEnd,
                    (FileKind::Text, true) => IconName::BookOpen,
                    (FileKind::Text, false) => IconName::File,
                })
                .size(px(16.))
                .text_color(rgb(p.muted))
                .flex_shrink_0(),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .debug_selector(|| "file-viewer-title".into())
                            .truncate()
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(target.name().to_owned()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(p.muted))
                            .child(format!("{} · {}", target.path, details.join(" · "))),
                    ),
            )
            .children(modes)
            .child(place)
            .child(copy)
            .child(close);
        let notice = self.notice.clone().map(|notice| {
            div()
                .debug_selector(|| "file-viewer-notice".into())
                .px_4()
                .py_2()
                .flex()
                .items_center()
                .gap_2()
                .border_b_1()
                .border_color(rgb(p.border))
                .text_size(px(12.))
                .text_color(rgb(p.warning))
                .child(Icon::new(IconName::Info).size(px(12.)).flex_shrink_0())
                .child(div().flex_1().min_w_0().child(notice))
        });
        div()
            .id("file-viewer")
            .debug_selector(|| "file-viewer".into())
            .size_full()
            .when(!window_pane, |d| {
                d.rounded(px(p.panel_radius))
                    .border_1()
                    .border_color(rgb(p.border))
            })
            .when(self.presentation == Presentation::Sheet, |d| {
                d.shadow(chrome::floating_shadow(p))
            })
            .bg(rgb(p.surface))
            .text_color(rgb(p.text))
            .when(window_pane, |d| {
                d.font_family(gpui_component::Theme::global(cx).font_family.clone())
                    .text_size(px(interface_size))
            })
            .overflow_hidden()
            .flex()
            .flex_col()
            // Inside the main window the shell drops its workspace context
            // while this has focus (always, as a sheet); Tab must not walk
            // focus out to the composer either.
            .key_context("FileViewer")
            .track_focus(&self.focus)
            .on_action(cx.listener(|_, _: &ViewerTab, _, _| {}))
            .on_action(cx.listener(|this, _: &ViewerToggleSource, window, cx| {
                let next = match this.mode {
                    Mode::Preview => Mode::Source,
                    Mode::Source => Mode::Preview,
                };
                this.set_mode(next, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ViewerFind, window, cx| this.find(window, cx)))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                // A separate window closes like any window; Esc is for the
                // editor's search there.
                if event.keystroke.key == "escape" && this.presentation != Presentation::Window {
                    this.close(window, cx);
                    cx.stop_propagation();
                }
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if this.scroll_key(event, cx) {
                    cx.stop_propagation();
                }
            }))
            .child(header)
            .children(notice)
            .child(body)
            .into_any_element()
    }
}

impl Workspace {
    /// Width of the docked viewer, or `None` when the window is too narrow
    /// to keep the conversation usable beside it (the viewer is a sheet).
    pub(super) fn viewer_dock_width(&self, window: &Window) -> Option<Pixels> {
        let viewport = unzoom(window.viewport_size().width);
        let sidebar = if self.sidebar_collapsed {
            56.
        } else {
            wks_native::navigation::sidebar_width(self.settings.sidebar_width, viewport)
        } + 8.;
        let available = viewport - sidebar;
        let panel = (available * 0.5).clamp(360., 760.);
        (available - panel >= MIN_CHAT_BESIDE).then(|| px(panel))
    }

    /// The main window's viewer is open as a modal sheet.
    pub(super) fn viewer_modal(&self, window: &Window) -> bool {
        self.chat.viewer.pane.is_some() && self.viewer_dock_width(window).is_none()
    }

    /// Keys go to the viewer, not the workspace: it is modal, or focused.
    pub(super) fn viewer_owns_keys(&self, window: &Window, cx: &App) -> bool {
        self.viewer_modal(window) || self.viewer_has_focus(window, cx)
    }

    pub(super) fn close_file_viewer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(popout) = self.chat.viewer.popout.take() {
            self.chat
                .dismissed
                .insert(KEY.into(), popout.pane.read(cx).state.number);
            // Already removed when its own close button ran; harmless twice.
            let _ = popout
                .window
                .update(cx, |_, window, _| window.remove_window());
        }
        if let Some(pane) = self.chat.viewer.pane.take() {
            let (number, inside) = {
                let pane = pane.read(cx);
                (pane.state.number, pane.has_focus(window, cx))
            };
            self.chat.dismissed.insert(KEY.into(), number);
            if inside || self.viewer_return.is_some() {
                self.release_viewer_focus(window, cx);
            }
        }
        cx.notify();
    }

    /// Keep keyboard focus where the viewer needs it. As a sheet it is
    /// modal: focus stays inside so nothing typed or bound reaches the
    /// composer, sidebar or conversation behind it. Docked, it takes focus
    /// when content arrives and otherwise leaves focus where the user put
    /// it. Runs after every view update.
    pub(super) fn hold_viewer_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = self.chat.viewer.pane.clone() else {
            return;
        };
        let inside = pane.read(cx).has_focus(window, cx);
        let modal = self.viewer_modal(window);
        let wants = pane.update(cx, |pane, _| std::mem::take(&mut pane.wants_focus));
        // Docked, arriving content never takes the keyboard from a draft
        // being typed beside it.
        let typing = !modal && self.composer.read(cx).focus_handle(cx).is_focused(window);
        let take = if modal {
            true
        } else {
            (wants && !typing) || pane.read(cx).focus.is_focused(window)
        };
        if !take {
            return;
        }
        if !inside && self.viewer_return.is_none() {
            self.viewer_return = window.focused(cx);
        }
        // Text takes focus once loaded (scrolling, selection, Ctrl+F);
        // focus already in its search field stays there.
        if !inside || pane.read(cx).focus.is_focused(window) {
            pane.read(cx).focus_content(window, cx);
        }
    }

    /// Focus is on the viewer or inside it. A just-built editor is not in
    /// the rendered tree yet, so it is checked by identity too.
    pub(super) fn viewer_has_focus(&self, window: &Window, cx: &App) -> bool {
        self.chat
            .viewer
            .pane
            .as_ref()
            .is_some_and(|pane| pane.read(cx).has_focus(window, cx))
    }

    /// Return focus to the composer if it had it, otherwise the workspace.
    fn release_viewer_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let composer = self.composer.read(cx).focus_handle(cx);
        if self.viewer_return.take().as_ref() == Some(&composer) {
            window.focus(&composer);
        } else {
            window.focus(&self.focus);
        }
    }

    /// Route one link from `owner`'s conversation. Web links need no hub;
    /// files are only requested for the selected, connected session.
    pub(super) fn open_link(&mut self, owner: &str, link: Link, cx: &mut Context<Self>) {
        match link {
            Link::Web(url) => cx.open_url(&url),
            Link::Anchor => {}
            Link::Refused(message) => {
                self.extras.notice = message;
                cx.notify();
            }
            Link::File(target) => {
                if self.view.selected.as_deref() != Some(owner) {
                    return;
                }
                if let Err(message) = self.request_preview(owner, target, cx) {
                    self.extras.notice = message;
                    cx.notify();
                }
            }
        }
    }

    /// Ask the connected hub to read a file of `owner`'s session. The
    /// result arrives as a `file-preview` request state.
    pub(super) fn request_preview(
        &mut self,
        owner: &str,
        target: FileTarget,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if !self.view.connected {
            return Err("Reconnect to the hub to open files.".into());
        }
        self.request(
            Request::FilePreview {
                session: owner.into(),
                target,
            },
            cx,
        );
        Ok(())
    }

    /// Link callback for Markdown/HTML views of the selected conversation.
    /// Relative paths resolve against the session cwd at click time.
    pub(super) fn link_handler(&self, cx: &mut Context<Self>) -> Arc<LinkClickFn> {
        let workspace = cx.entity().downgrade();
        let owner = self.view.selected.clone().unwrap_or_default();
        Arc::new(move |url, _, cx| {
            let _ = workspace.update(cx, |this, cx| {
                let cwd = this
                    .selected_session()
                    .map(|s| s.cwd.clone())
                    .unwrap_or_default();
                this.open_link(&owner, links::classify(&cwd, url), cx);
            });
        })
    }

    /// Hand a newer `file-preview` state to the showing pane, opening the
    /// main window's pane for a new request. Each pane keeps its own
    /// snapshot, so a dismissed or superseded request never comes back, and
    /// a pane waiting on a request that disappeared says so.
    pub(super) fn sync_file_viewer(
        &mut self,
        next: &View,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dismissed = self.chat.dismissed.get(KEY).copied().unwrap_or(0);
        let incoming = next
            .requests
            .get(KEY)
            .filter(|s| s.number > dismissed && matches!(s.request, Request::FilePreview { .. }));
        // A popped-out window destroyed without a close request (killed by
        // the window system) docks its content back instead of swallowing
        // every later file.
        if let Some(popout) = &self.chat.viewer.popout {
            let id = popout.window.window_id();
            if !cx.windows().iter().any(|w| w.window_id() == id) {
                self.dock_closing_popout(window, cx);
            }
        }
        let showing = match &self.chat.viewer.popout {
            Some(popout) => Some((popout.pane.clone(), Some(popout.window))),
            None => self.chat.viewer.pane.clone().map(|pane| (pane, None)),
        };
        let Some((pane, popout)) = showing else {
            if let Some(state) = incoming {
                let presentation = if self.viewer_dock_width(window).is_some() {
                    Presentation::Docked
                } else {
                    Presentation::Sheet
                };
                let workspace = cx.entity().downgrade();
                self.chat.viewer.pane =
                    Some(cx.new(|cx| PreviewPane::new(workspace, presentation, state, window, cx)));
                self.chat.viewer.opened += 1;
            }
            return;
        };
        let current = pane.read(cx).state.number;
        match incoming {
            Some(state) if state.number >= current => {
                let state = state.clone();
                match popout {
                    // The popped-out window builds its own widgets.
                    Some(handle) => {
                        let fresh = state.number != current;
                        let _ = handle.update(cx, |_, window, cx| {
                            pane.update(cx, |pane, cx| {
                                pane.load(&state, window, cx);
                                if std::mem::take(&mut pane.wants_focus)
                                    && (!pane.has_focus(window, cx)
                                        || pane.focus.is_focused(window))
                                {
                                    pane.focus_content(window, cx);
                                }
                            });
                            if fresh {
                                window.activate_window();
                            }
                        });
                    }
                    None => pane.update(cx, |pane, cx| pane.load(&state, window, cx)),
                }
            }
            _ => pane.update(cx, |pane, cx| pane.interrupt(cx)),
        }
    }

    /// Move the main window's viewer into a window of its own. The window
    /// opens after this update (its first render reads the workspace); on
    /// failure the viewer stays where it was and says why.
    pub(super) fn pop_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = self.chat.viewer.pane.clone() else {
            return;
        };
        let snapshot = {
            let pane = pane.read(cx);
            (pane.state.clone(), pane.mode, pane.history.clone())
        };
        let workspace = cx.entity();
        let main = window.window_handle();
        cx.defer(move |cx| {
            let opened = open_popout(workspace.downgrade(), snapshot, cx);
            let _ = main.update(cx, |_, window, cx| {
                workspace.update(cx, |ws, cx| ws.popped_out(pane, opened, window, cx))
            });
        });
    }

    fn popped_out(
        &mut self,
        pane: Entity<PreviewPane>,
        opened: anyhow::Result<OpenedPopout>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match opened {
            Ok(OpenedPopout {
                window: handle,
                pane: popped,
                watch,
            }) => {
                // Superseded meanwhile (closed, or another window): the
                // newest state wins and this window goes.
                if self.chat.viewer.pane.as_ref() != Some(&pane) {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                    return;
                }
                let inside = pane.read(cx).has_focus(window, cx);
                self.chat.viewer.pane = None;
                self.chat.viewer.popout = Some(Popout {
                    window: handle,
                    pane: popped,
                });
                if inside || self.viewer_return.is_some() {
                    self.release_viewer_focus(window, cx);
                }
                self.watch_popout(handle.window_id(), watch, window, cx);
            }
            Err(error) => pane.update(cx, |pane, cx| {
                pane.notice = Some(format!("Couldn't open a separate window: {error}").into());
                cx.notify();
            }),
        }
        cx.notify();
    }

    /// Consume the actual watch protocol. Closed channels are terminal too:
    /// worker startup/panic/connection failures must never strand the viewer.
    pub(super) fn watch_popout(
        &mut self,
        id: gpui::WindowId,
        watch: super::window_destroy::Watch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use super::window_destroy::{Event, Watch};
        let Watch::Started(events) = watch else {
            self.popout_attached(id, cx);
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            loop {
                let event = events.recv().await;
                let attached = matches!(event, Ok(Event::Attached));
                if !attached && !matches!(event, Ok(Event::Destroyed)) {
                    eprintln!("File preview watch ended; docking viewer: {event:?}");
                }
                let result = this.update_in(cx, |ws, window, cx| {
                    if attached {
                        ws.popout_attached(id, cx);
                    } else {
                        ws.popout_destroyed(id, window, cx);
                    }
                });
                if !attached || result.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn popout_attached(&mut self, id: gpui::WindowId, cx: &mut App) {
        let Some(popout) = self
            .chat
            .viewer
            .popout
            .as_ref()
            .filter(|popout| popout.window.window_id() == id)
        else {
            return;
        };
        let _ = popout.window.update(cx, |_, window, cx| {
            popout.pane.update(cx, |pane, _| {
                pane.title_ready = true;
                if let Some(target) = pane.target() {
                    window.set_window_title(&popout_title(target));
                }
            });
        });
    }

    /// Bring a popped-out viewer back beside the conversation (or as a
    /// sheet when the window is narrow), keeping its file, mode and history.
    pub(super) fn dock_popout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(popout) = &self.chat.viewer.popout {
            let handle = popout.window;
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
        self.dock_closing_popout(window, cx);
    }

    /// The window system destroyed the popped-out window without a close
    /// request (see `window_destroy`). GPUI still holds it, so updates to it
    /// keep succeeding invisibly: dock its viewer back and drop the dead
    /// window. Its normal closes (Dock, its close button, the app quitting)
    /// have already replaced or cleared `popout` by then and are ignored.
    pub(super) fn popout_destroyed(
        &mut self,
        id: gpui::WindowId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(handle) = self
            .chat
            .viewer
            .popout
            .as_ref()
            .map(|popout| popout.window)
            .filter(|handle| handle.window_id() == id)
        else {
            return;
        };
        self.dock_closing_popout(window, cx);
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    }

    /// Dock back from a window the platform is already closing.
    fn dock_closing_popout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(popout) = self.chat.viewer.popout.take() else {
            return;
        };
        let (state, mode, history) = {
            let pane = popout.pane.read(cx);
            (pane.state.clone(), pane.mode, pane.history.clone())
        };
        let presentation = if self.viewer_dock_width(window).is_some() {
            Presentation::Docked
        } else {
            Presentation::Sheet
        };
        let workspace = cx.entity().downgrade();
        let pane = cx.new(|cx| {
            let mut pane = PreviewPane::new(workspace, presentation, &state, window, cx);
            pane.history = history;
            pane.set_mode(mode, window, cx);
            pane
        });
        self.chat.viewer.pane = Some(pane);
        self.chat.viewer.opened += 1;
        self.hold_viewer_focus(window, cx);
        cx.notify();
    }

    /// Redraw a popped-out viewer after appearance or text settings change.
    pub(super) fn refresh_popout(&self, cx: &mut App) {
        if let Some(popout) = &self.chat.viewer.popout {
            let _ = popout.window.update(cx, |_, window, _| window.refresh());
        }
    }

    /// Close a popped-out window with its owner (main window closing).
    pub(super) fn close_popout_window(&mut self, cx: &mut App) {
        if let Some(popout) = self.chat.viewer.popout.take() {
            let _ = popout
                .window
                .update(cx, |_, window, _| window.remove_window());
        }
    }

    /// The docked viewer, a flex sibling right of the main content.
    pub(super) fn render_docked_viewer(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let pane = self.chat.viewer.pane.clone()?;
        let width = self.viewer_dock_width(window)?;
        pane.update(cx, |pane, _| {
            pane.presentation = Presentation::Docked;
            pane.width = width - px(8.);
        });
        // The slot grows from the window's right edge while the pane keeps
        // its full width, pinned to the slot's left edge: the viewer slides
        // in from the right and the conversation narrows with it.
        Some(
            div()
                .id("file-viewer-panel")
                .debug_selector(|| "file-viewer-panel".into())
                .relative()
                .h_full()
                .w(width)
                .flex_shrink_0()
                .overflow_hidden()
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .h_full()
                        .w(width)
                        .py_2()
                        .pr_2()
                        // Under an app-drawn caption the header starts below
                        // the window buttons instead of under them.
                        .when(chrome::custom_caption(), |d| {
                            d.pt(px(chrome::CAPTION_HEIGHT + 4.))
                        })
                        .child(pane),
                )
                .with_animation(
                    ("file-viewer-slide", self.chat.viewer.opened),
                    Animation::new(SLIDE).with_easing(ease_out_cubic),
                    move |panel, progress| panel.w(width * progress),
                )
                .into_any_element(),
        )
    }

    /// Narrow windows: the viewer slides over the conversation from the
    /// right as a modal sheet. Backdrop, ✕ and Esc close it.
    pub(super) fn render_file_viewer(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        if self.viewer_dock_width(window).is_some() {
            return None;
        }
        let pane = self.chat.viewer.pane.clone()?;
        let p = self.appearance.palette();
        let viewport = window.viewport_size();
        let width = (viewport.width - px(24.)).min(px(820.)).max(px(280.));
        pane.update(cx, |pane, _| {
            pane.presentation = Presentation::Sheet;
            pane.width = width;
        });
        Some(
            deferred(
                div()
                    .id("file-viewer-backdrop")
                    .debug_selector(|| "file-viewer-backdrop".into())
                    .absolute()
                    .inset_0()
                    // The sheet paints above the caption; leave its strip
                    // free so the window buttons stay visible and usable.
                    .when(chrome::custom_caption(), |d| {
                        d.top(px(chrome::CAPTION_HEIGHT))
                    })
                    .occlude()
                    .bg(gpui::Hsla::from(rgb(p.shadow)).opacity(0.45))
                    .flex()
                    .justify_end()
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, window, cx| this.close_file_viewer(window, cx)),
                    )
                    .child(
                        div()
                            .id("file-viewer-sheet")
                            .occlude()
                            .relative()
                            .w(width)
                            .h_full()
                            .p_2()
                            .child(pane)
                            .with_animation(
                                ("file-viewer-sheet-slide", self.chat.viewer.opened),
                                Animation::new(SLIDE).with_easing(ease_out_cubic),
                                move |sheet, progress| sheet.left(width * (1. - progress)),
                            ),
                    ),
            )
            .with_priority(2),
        )
    }
}

struct OpenedPopout {
    window: WindowHandle<Root>,
    pane: Entity<PreviewPane>,
    watch: super::window_destroy::Watch,
}

/// Open the viewer's own window with a pane built from `snapshot`.
fn open_popout(
    workspace: WeakEntity<Workspace>,
    (state, mode, history): (RequestState, Mode, Vec<FileTarget>),
    cx: &mut App,
) -> anyhow::Result<OpenedPopout> {
    let Request::FilePreview { target, .. } = &state.request else {
        anyhow::bail!("nothing to show");
    };
    let title = super::window_destroy::discovery_title(&popout_title(target), cx);
    let mut created = None;
    let bounds = gpui::Bounds::centered(None, gpui::size(px(880.), px(760.)), cx);
    let handle = cx.open_window(
        gpui::WindowOptions {
            window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
            window_min_size: Some(gpui::size(px(420.), px(320.))),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some(title.clone().into()),
                ..Default::default()
            }),
            app_id: Some("workspacer-native".into()),
            ..Default::default()
        },
        |window, cx| {
            window.set_window_title(&title);
            window.set_app_id("workspacer-native");
            let popped = cx.new(|cx| {
                let mut pane =
                    PreviewPane::new(workspace.clone(), Presentation::Window, &state, window, cx);
                pane.history = history;
                pane.set_mode(mode, window, cx);
                pane.wants_focus = false;
                pane
            });
            popped.read(cx).focus_content(window, cx);
            // The window's own close button docks the viewer back rather
            // than losing it.
            window.on_window_should_close(cx, move |_, cx| {
                let workspace = workspace.clone();
                cx.defer(move |cx| {
                    let Some(workspace) = workspace.upgrade() else {
                        return;
                    };
                    let Some(main) = workspace.read(cx).main_window else {
                        return;
                    };
                    let _ = main.update(cx, |_, window, cx| {
                        workspace.update(cx, |ws, cx| ws.dock_closing_popout(window, cx))
                    });
                });
                true
            });
            created = Some(popped.clone());
            cx.new(|cx| Root::new(gpui::AnyView::from(popped), window, cx))
        },
    )?;
    let popped = created.ok_or_else(|| anyhow::anyhow!("no viewer was created"))?;
    let watch = super::window_destroy::watch(&title, cx);
    Ok(OpenedPopout {
        window: handle,
        pane: popped,
        watch,
    })
}

fn popout_title(target: &FileTarget) -> String {
    format!("{} — Workspacer", target.name())
}

/// Place the editor's cursor without moving keyboard focus: the editor's
/// own `set_cursor_position` also focuses it, which would take the keyboard
/// from a draft beside a docked viewer.
fn move_cursor(editor: &Entity<InputState>, position: Position, window: &mut Window, cx: &mut App) {
    let before = window.focused(cx);
    editor.update(cx, |editor, cx| {
        editor.set_cursor_position(position, window, cx)
    });
    match before {
        Some(handle) => window.focus(&handle),
        None => window.blur(),
    }
}

/// The editor scrolls to a position only once it has a layout, which the
/// first frames after creation provide; until then only the cursor moves.
fn reveal(editor: WeakEntity<InputState>, position: Position, frames: u8, window: &mut Window) {
    window.on_next_frame(move |window, cx| {
        if frames > 1 {
            reveal(editor, position, frames - 1, window);
        } else if let Some(editor) = editor.upgrade() {
            move_cursor(&editor, position, window, cx);
        }
    });
}

pub(super) fn tool_link(cwd: &str, input: &serde_json::Value, path: &str) -> Link {
    // A Read tool's `offset` is the first line it read.
    let line = input["offset"].as_u64().and_then(|l| u32::try_from(l).ok());
    links::tool_file(cwd, path, line)
}

#[cfg(all(test, feature = "ui-tests"))]
impl Workspace {
    /// The main window's open viewer, docked or as a sheet.
    pub(super) fn file_viewer(&self) -> Option<&Entity<PreviewPane>> {
        self.chat.viewer.pane.as_ref()
    }
    pub(super) fn viewer_popout(&self) -> Option<(WindowHandle<Root>, Entity<PreviewPane>)> {
        self.chat
            .viewer
            .popout
            .as_ref()
            .map(|p| (p.window, p.pane.clone()))
    }
}

#[cfg(all(test, feature = "ui-tests"))]
impl PreviewPane {
    pub(super) fn state(&self) -> &RequestState {
        &self.state
    }
    pub(super) fn mode(&self) -> Mode {
        self.mode
    }
    pub(super) fn editor(&self) -> Option<&Entity<InputState>> {
        self.editor.as_ref()
    }
    pub(super) fn document(&self) -> Option<&TextViewHandle> {
        self.document.as_ref()
    }
    pub(super) fn notice(&self) -> Option<&str> {
        self.notice.as_ref().map(|n| n.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_targets_keep_the_read_offset_as_their_line() {
        let input = serde_json::json!({"file_path":"src/lib.rs","offset":40});
        let Link::File(target) = tool_link("/repo", &input, "src/lib.rs") else {
            panic!("tool path is a file");
        };
        assert_eq!(
            (target.path.as_str(), target.line),
            ("/repo/src/lib.rs", Some(40))
        );
        let Link::File(target) = tool_link("/repo", &serde_json::json!({}), "/tmp/a.png") else {
            panic!("absolute image path is a file");
        };
        assert_eq!((target.line, target.kind), (None, FileKind::Image));
    }
}
