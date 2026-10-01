//! Readable Markdown with session-owned file previews and full text selection.
use super::*;
use gpui_component::{ActiveTheme, text::TextViewStyle};
use wks_native::{features::Request, transcript::file_target};

impl Workspace {
    pub(super) fn render_markdown(
        &self,
        key: &str,
        source: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let workspace = cx.entity().downgrade();
        let owner = self.view.selected.clone().unwrap_or_default();
        let style = TextViewStyle {
            on_link_click: Some(Arc::new(move |url, _, cx| {
                if url.starts_with("https://")
                    || url.starts_with("http://")
                    || url.starts_with("mailto:")
                {
                    cx.open_url(url);
                    return;
                }
                let _ = workspace.update(cx, |this, cx| {
                    if !this.view.connected || this.view.selected.as_ref() != Some(&owner) {
                        return;
                    }
                    let cwd = this
                        .selected_session()
                        .map(|s| s.cwd.as_str())
                        .unwrap_or("");
                    if let Some(path) = file_target(cwd, url) {
                        this.request(
                            Request::FilePreview {
                                session: owner.clone(),
                                path,
                            },
                            cx,
                        );
                    }
                });
            })),
            unordered_list_marker: Some("• ".into()),
            paragraph_gap: gpui::rems(0.65),
            heading_base_font_size: px(self.settings.text_size as f32),
            heading_font_size: Some(Arc::new(|level, base| {
                base + px(match level {
                    1 => 9.,
                    2 => 5.,
                    3 => 2.,
                    _ => 0.,
                })
            })),
            highlight_theme: cx.theme().highlight_theme.clone(),
            is_dark: self.appearance != Appearance::Light,
            code_block: div()
                .rounded(px(self.appearance.palette().panel_radius))
                .p_3()
                .style()
                .clone(),
        };
        let key = key.to_owned();
        let debug_key = key.clone();
        div()
            .id(SharedString::from(key.clone()))
            .debug_selector(move || format!("markdown-inline-{debug_key}"))
            .min_w_0()
            .w_full()
            .line_height(gpui::relative(1.6))
            .child(
                TextView::markdown(
                    SharedString::from(format!("markdown-{key}")),
                    source.to_owned(),
                    window,
                    cx,
                )
                .style(style)
                .selectable(true),
            )
    }
}
