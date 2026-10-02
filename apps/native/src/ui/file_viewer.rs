//! Where chat links lead: the system browser for web links, and a read-only
//! viewer for files and images. Files are read on the session's machine by
//! the connected hub (`file-preview` request), never from this client's disk.
use super::*;
use base64::Engine;
use gpui::{AnyElement, WeakEntity, deferred};
use gpui_component::input::{Input, InputState, Position};
use gpui_component::text::LinkClickFn;
use wks_native::{
    features::Request,
    links::{self, FileKind, Link},
};

#[derive(Default)]
pub(super) struct FileViewer {
    /// Request number whose content is loaded below.
    shown: u64,
    editor: Option<Entity<InputState>>,
    /// Display image and the source's own dimensions.
    image: Option<(Arc<gpui::Image>, u64, u64)>,
}

impl Workspace {
    /// The open viewer's request, if one belongs to the selected session.
    pub(super) fn file_viewer(&self) -> Option<&wks_native::features::RequestState> {
        self.view
            .requests
            .get("file-preview")
            .filter(|s| self.chat.dismissed.get("file-preview") != Some(&s.number))
            .filter(|s| {
                matches!(&s.request, Request::FilePreview { session, .. }
                    if Some(session) == self.view.selected.as_ref())
            })
    }

    pub(super) fn close_file_viewer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(number) = self.file_viewer().map(|s| s.number) {
            self.chat.dismissed.insert("file-preview".into(), number);
            self.chat.viewer = FileViewer::default();
            window.focus(&self.focus);
            cx.notify();
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
                if !self.view.connected {
                    self.extras.notice = "Reconnect to the hub to open files.".into();
                    cx.notify();
                    return;
                }
                self.request(
                    Request::FilePreview {
                        session: owner.into(),
                        target,
                    },
                    cx,
                );
            }
        }
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

    /// Build viewer content when a preview finishes loading.
    pub(super) fn sync_file_viewer(
        &mut self,
        next: &View,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = next.requests.get("file-preview") else {
            self.chat.viewer = FileViewer::default();
            return;
        };
        if state.loading || state.number == self.chat.viewer.shown {
            return;
        }
        let Request::FilePreview { target, .. } = &state.request else {
            return;
        };
        self.chat.viewer = FileViewer {
            shown: state.number,
            ..Default::default()
        };
        if state.error.is_some() {
            return;
        }
        match target.kind {
            FileKind::Text => {
                let contents = state.value["contents"].as_str().unwrap_or("").to_owned();
                let language = target.language();
                let editor = cx.new(|cx| {
                    InputState::new(window, cx)
                        .code_editor(language)
                        .line_number(true)
                        .searchable(true)
                        .soft_wrap(false)
                        .default_value(contents)
                });
                // Focus the text for keyboard scrolling, selection and Ctrl+F;
                // a line anchor places the cursor (and active-line band) there.
                let position = Position::new(
                    target.line.unwrap_or(1).saturating_sub(1),
                    target.column.unwrap_or(1).saturating_sub(1),
                );
                editor.update(cx, |editor, cx| {
                    editor.set_cursor_position(position, window, cx)
                });
                if target.line.is_some() {
                    reveal(editor.downgrade(), position, 2, window);
                }
                self.chat.viewer.editor = Some(editor);
            }
            FileKind::Image => {
                let image = state.value["png"]
                    .as_str()
                    .and_then(|png| base64::engine::general_purpose::STANDARD.decode(png).ok())
                    .map(|bytes| Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, bytes)));
                if let Some(image) = image {
                    self.chat.viewer.image = Some((
                        image,
                        state.value["width"].as_u64().unwrap_or(0),
                        state.value["height"].as_u64().unwrap_or(0),
                    ));
                }
            }
        }
    }

    /// Full-window sheet over the conversation. Backdrop, ✕ and Esc close it.
    pub(super) fn render_file_viewer(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let state = self.file_viewer()?;
        let Request::FilePreview { target, .. } = &state.request else {
            return None;
        };
        let p = self.appearance.palette();
        let viewport = window.viewport_size();
        let body: AnyElement = if let Some(error) = &state.error {
            div()
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
                        .text_size(px(13.))
                        .text_color(rgb(p.text))
                        .child(error.clone()),
                )
                .into_any_element()
        } else if state.loading || state.number != self.chat.viewer.shown {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .gap_2()
                .text_size(px(12.))
                .text_color(rgb(p.muted))
                .child(brand_spinner(14., p, "file-viewer-loading"))
                .child(format!("Loading {}…", target.name()))
                .into_any_element()
        } else if let Some(editor) = &self.chat.viewer.editor {
            div()
                .id("file-viewer-text")
                .debug_selector(|| "file-viewer-text".into())
                .flex_1()
                .min_h_0()
                .bg(rgb(p.code_block))
                .text_size(px(self.settings.text_size as f32 - 1.))
                .font_family(gpui_component::Theme::global(cx).mono_font_family.clone())
                .child(
                    Input::new(editor)
                        .disabled(true)
                        .appearance(false)
                        .h_full(),
                )
                .into_any_element()
        } else if let Some((image, ..)) = &self.chat.viewer.image {
            div()
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
                .into_any_element()
        } else {
            div()
                .flex_1()
                .p_6()
                .text_color(rgb(p.warning))
                .child("This preview could not be displayed.")
                .into_any_element()
        };
        let mut details = vec![];
        if let Some(line) = target.line {
            details.push(match target.column {
                Some(column) => format!("Line {line}, column {column}"),
                None => format!("Line {line}"),
            });
        }
        if let Some((_, width, height)) = &self.chat.viewer.image
            && *width > 0
        {
            details.push(format!("{width} × {height}"));
        }
        if let Some(size) = state.value["size"].as_u64() {
            details.push(links::size(size));
        }
        details.push("Read-only".into());
        let path = target.path.clone();
        let card = div()
            .id("file-viewer")
            .debug_selector(|| "file-viewer".into())
            .occlude()
            .w(px((f32::from(viewport.width) * 0.92).min(1120.)))
            .h(px((f32::from(viewport.height) * 0.86).max(240.)))
            .rounded(px(p.panel_radius))
            .border_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.surface))
            .shadow(chrome::floating_shadow(p))
            .text_color(rgb(p.text))
            .overflow_hidden()
            .flex()
            .flex_col()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.close_file_viewer(window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .px_4()
                    .py_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .border_b_1()
                    .border_color(rgb(p.border))
                    .child(
                        Icon::new(match target.kind {
                            FileKind::Image => IconName::GalleryVerticalEnd,
                            FileKind::Text => IconName::File,
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
                    .child(
                        self.icon_button("file-viewer-copy", "Copy path", IconName::Copy, true)
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(path.clone()))
                            }),
                    )
                    .child(
                        self.icon_button("file-viewer-close", "Close", IconName::Close, true)
                            .debug_selector(|| "file-viewer-close".into())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_file_viewer(window, cx)
                            })),
                    ),
            )
            .child(body);
        Some(
            deferred(
                div()
                    .id("file-viewer-backdrop")
                    .debug_selector(|| "file-viewer-backdrop".into())
                    .absolute()
                    .inset_0()
                    .occlude()
                    .bg(gpui::Hsla::from(rgb(p.shadow)).opacity(0.45))
                    .flex()
                    .items_center()
                    .justify_center()
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, window, cx| this.close_file_viewer(window, cx)),
                    )
                    .child(card),
            )
            .with_priority(2),
        )
    }
}

/// The editor scrolls to a position only once it has a layout, which the
/// first frames after creation provide; until then only the cursor moves.
fn reveal(editor: WeakEntity<InputState>, position: Position, frames: u8, window: &mut Window) {
    window.on_next_frame(move |window, cx| {
        if frames > 1 {
            reveal(editor, position, frames - 1, window);
        } else if let Some(editor) = editor.upgrade() {
            editor.update(cx, |editor: &mut InputState, cx| {
                editor.set_cursor_position(position, window, cx)
            });
        }
    });
}

pub(super) fn tool_link(cwd: &str, input: &serde_json::Value, path: &str) -> Link {
    // A Read tool's `offset` is the first line it read.
    let line = input["offset"]
        .as_u64()
        .and_then(|l| u32::try_from(l).ok());
    links::tool_file(cwd, path, line)
}

#[cfg(all(test, feature = "ui-tests"))]
impl FileViewer {
    pub(super) fn editor(&self) -> Option<&Entity<InputState>> {
        self.editor.as_ref()
    }
}
