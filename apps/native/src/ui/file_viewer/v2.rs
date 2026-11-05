//! The file viewer's header under design v2: a path breadcrumb (folders in
//! tertiary, the file name in front), details beside it, and every action
//! as an icon button with a tooltip; Save is the one primary, shown only
//! while there is something to save.
use super::*;
use crate::ui::primitives::{self, ButtonSize, ButtonVariant};

/// The folders shown before a file's name: its path under the explorer
/// root when it is inside it, else its last few folders, with "…" for
/// what is left out.
pub(super) fn crumbs(path: &str, root: &str) -> Vec<String> {
    const MOST: usize = 3;
    let path = path.replace('\\', "/");
    let root = root.replace('\\', "/");
    let root = root.trim_end_matches('/');
    let relative = path
        .strip_prefix(root)
        .filter(|rest| !root.is_empty() && rest.starts_with('/'))
        .map(|rest| rest.trim_start_matches('/'));
    let inside = relative.is_some();
    let rel = relative.unwrap_or(&path);
    let mut parts: Vec<String> = rel
        .split('/')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    parts.pop(); // the file name
    if inside && let Some(project) = root.rsplit('/').next().filter(|s| !s.is_empty()) {
        parts.insert(0, project.to_owned());
    }
    if parts.len() > MOST {
        let keep = parts.split_off(parts.len() - MOST);
        parts = std::iter::once("…".to_owned()).chain(keep).collect();
    }
    parts
}

/// What the header needs from the render pass.
pub(super) struct HeaderV2 {
    pub title: String,
    pub path: String,
    pub details: Vec<String>,
    pub icon: IconName,
    pub dirty: bool,
    pub saving: bool,
    pub can_save: bool,
    pub editable: bool,
    pub window_pane: bool,
}

impl PreviewPane {
    pub(super) fn render_header_v2(&self, h: HeaderV2, p: Palette, cx: &mut Context<Self>) -> Div {
        let t = p.tokens;
        let ghost = |id: &'static str, icon: IconName, tip: &'static str| {
            primitives::icon_button(id, icon, tip, ButtonVariant::Tertiary, true, &p)
                .debug_selector(move || id.into())
        };
        let back = (!self.history.is_empty()).then(|| {
            ghost("file-viewer-back", IconName::ArrowLeft, "Back")
                .on_click(cx.listener(|this, _, _, cx| this.back_guarded(cx)))
        });
        let files = ghost(
            "file-viewer-files",
            if self.explorer.open {
                IconName::PanelLeftClose
            } else {
                IconName::PanelLeftOpen
            },
            if self.explorer.open {
                "Hide files"
            } else {
                "Show files"
            },
        )
        .on_click(cx.listener(|this, _, _, cx| this.toggle_explorer(cx)));
        let crumbs = if self.target().is_some() {
            crumbs(&h.path, &self.explorer.root)
        } else {
            Vec::new()
        };
        let breadcrumb = div()
            .id("file-viewer-breadcrumb")
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(t.space.xs))
            .overflow_hidden()
            // The full path and the file's details live in the tooltip, so
            // a narrow dock still shows the folders and the name.
            .tooltip(primitives::tooltip_view(
                wks_native::format::meta(
                    std::iter::once(h.path.clone()).chain(h.details.iter().cloned()),
                ),
                None,
                &p,
            ))
            .child(
                Icon::new(h.icon)
                    .size(px(t.icon.sm))
                    .flex_shrink_0()
                    .text_color(rgb(p.roles.text_tertiary)),
            )
            // The folders shrink as one run, so the name always shows whole.
            .when(!crumbs.is_empty(), |d| {
                d.child(
                    div()
                        .flex_shrink()
                        .min_w_0()
                        .truncate()
                        .text_size(px(t.text.meta))
                        .text_color(rgb(p.roles.text_tertiary))
                        .child(format!("{} /", crumbs.join(" / "))),
                )
            })
            .child(
                div()
                    .debug_selector(|| "file-viewer-title".into())
                    .flex_shrink_0()
                    .text_size(px(t.text.subheading))
                    .font_weight(FontWeight(t.weight.semibold))
                    .text_color(rgb(p.text))
                    .child(h.title),
            )
            .when(h.dirty, |d| {
                d.child(
                    div()
                        .debug_selector(|| "file-viewer-dirty".into())
                        .size(px(t.layout.status_dot))
                        .flex_shrink_0()
                        .rounded(px(t.radius.full))
                        .bg(rgb(p.warning)),
                )
            });
        let modes = self.render_modes_v2(&p, cx);
        let shortcut = if cfg!(target_os = "macos") {
            "⌘ S"
        } else {
            "Ctrl S"
        };
        let save = h.editable.then(|| {
            primitives::with_tooltip(
                primitives::button(
                    "file-viewer-save",
                    if h.saving { "Saving…" } else { "Save" },
                    None,
                    if h.dirty {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Tertiary
                    },
                    ButtonSize::Small,
                    h.can_save,
                    &p,
                ),
                "Save",
                Some(shortcut),
                &p,
            )
            .debug_selector(|| "file-viewer-save".into())
            .when(h.can_save, |d| {
                d.on_click(cx.listener(|this, _, _, cx| {
                    let force = this.conflict.is_some() && !this.dirty();
                    this.save(force, cx)
                }))
            })
        });
        let place = if h.window_pane {
            ghost(
                "file-viewer-dock",
                IconName::PanelRight,
                "Dock beside the chat",
            )
            .on_click(cx.listener(|this, _, window, cx| {
                this.workspace_call(window, cx, |ws, window, cx| ws.dock_popout(window, cx));
            }))
        } else {
            ghost(
                "file-viewer-popout",
                IconName::ExternalLink,
                "Open in a separate window",
            )
            .on_click(cx.listener(|this, _, window, cx| {
                this.workspace_call(window, cx, |ws, window, cx| ws.pop_out(window, cx));
            }))
        };
        let path = h.path.clone();
        let copy =
            ghost("file-viewer-copy", IconName::Copy, "Copy path").on_click(move |_, _, cx| {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(path.clone()))
            });
        let close = primitives::icon_button_with_shortcut(
            "file-viewer-close",
            IconName::Close,
            "Close",
            Some("Esc"),
            ButtonVariant::Tertiary,
            true,
            &p,
        )
        .debug_selector(|| "file-viewer-close".into())
        .on_click(cx.listener(|this, _, window, cx| this.close(window, cx)));
        div()
            .debug_selector(|| "file-viewer-header".into())
            .flex_shrink_0()
            .h(px(t.panels.header_height))
            .pl(px(t.space.sm))
            .pr(px(t.space.sm))
            .flex()
            .items_center()
            .gap(px(t.space.xs))
            .map(|d| system::edge(d, system::Side::Bottom, system::EdgeTone::Divider, &p))
            .children(back)
            .child(files)
            .child(
                div()
                    .pl(px(t.space.xs))
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .child(breadcrumb),
            )
            .children(modes)
            .children(save)
            .child(place)
            .child(copy)
            .child(close)
    }

    /// Preview / Source for Markdown, as the shared segmented control.
    fn render_modes_v2(&self, p: &Palette, cx: &mut Context<Self>) -> Option<Div> {
        let markdown = self.target().is_some_and(FileTarget::markdown)
            && self.editor.is_some()
            && self.state.error.is_none();
        if !markdown {
            return None;
        }
        let previewable = self.renderable;
        let segment = |mode: Mode, id: &'static str, label: &'static str, enabled: bool| {
            primitives::segment(id, label, self.mode == mode, p)
                .debug_selector(move || id.into())
                .when(enabled, |d| {
                    d.on_click(
                        cx.listener(move |this, _, window, cx| this.set_mode(mode, window, cx)),
                    )
                })
        };
        Some(
            div().pr(px(p.tokens.space.xs)).child(
                primitives::segmented(p)
                    .child(segment(
                        Mode::Preview,
                        "file-viewer-mode-preview",
                        "Preview",
                        previewable,
                    ))
                    .child(segment(
                        Mode::Source,
                        "file-viewer-mode-source",
                        "Source",
                        true,
                    )),
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breadcrumbs_start_at_the_project() {
        let root = "/home/dev/src/workspacer/apps/native";
        assert_eq!(
            crumbs("/home/dev/src/workspacer/apps/native/README.md", root),
            ["native"]
        );
        assert_eq!(
            crumbs(
                "/home/dev/src/workspacer/apps/native/src/ui/review.rs",
                root
            ),
            ["native", "src", "ui"]
        );
        assert_eq!(
            crumbs(
                "/home/dev/src/workspacer/apps/native/src/ui/review/v2/page.rs",
                root
            ),
            ["…", "ui", "review", "v2"]
        );
        // Outside the root: its last folders.
        assert_eq!(
            crumbs("/etc/nginx/sites/default", root),
            ["etc", "nginx", "sites"]
        );
        assert_eq!(crumbs("C:\\work\\app\\main.rs", ""), ["C:", "work", "app"]);
    }
}
