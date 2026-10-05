//! Readable Markdown with session-routed links and full text selection.
use super::*;
use gpui_component::{
    ActiveTheme,
    text::{ProseColors, TextViewStyle},
};

impl Workspace {
    pub(super) fn render_markdown(
        &self,
        key: &str,
        source: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        let style = self.markdown_style(self.link_handler(cx), false, cx);
        let key = key.to_owned();
        let debug_key = key.clone();
        div()
            .id(SharedString::from(key.clone()))
            .debug_selector(move || format!("markdown-inline-{debug_key}"))
            .min_w_0()
            .w_full()
            .text_color(rgb(p.prose))
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

impl Workspace {
    /// The chat's Markdown look, shared by the file viewer's document
    /// preview. `document` scales headings up for long-form reading.
    pub(super) fn markdown_style(
        &self,
        on_link_click: Arc<gpui_component::text::LinkClickFn>,
        document: bool,
        cx: &App,
    ) -> TextViewStyle {
        let p = self.appearance.palette();
        TextViewStyle {
            on_link_click: Some(on_link_click),
            unordered_list_marker: Some("• ".into()),
            // Desktop chat parity (components/markdown.tsx): bright emphasis,
            // accent inline code, accent bullets, labeled bordered fences.
            prose: Some(ProseColors {
                strong: rgb(p.text).into(),
                code: rgb(p.accent).into(),
                code_background: rgb(p.code_inline).into(),
                marker: rgb(p.accent).into(),
                muted: rgb(p.muted).into(),
                rule: rgb(p.border).into(),
                border: rgb(p.border).into(),
                code_header: rgb(p.code_header).into(),
            }),
            paragraph_gap: gpui::rems(if document { 0.75 } else { 0.5 }),
            heading_base_font_size: px(self.settings.text_size as f32),
            heading_font_size: Some(if document {
                Arc::new(|level, base| {
                    base + px(match level {
                        1 => 14.,
                        2 => 8.,
                        3 => 4.,
                        4 => 2.,
                        _ => 0.,
                    })
                })
            } else {
                Arc::new(|level, base| {
                    base + px(match level {
                        1 => 9.,
                        2 => 5.,
                        3 => 2.,
                        _ => 0.,
                    })
                })
            }),
            highlight_theme: cx.theme().highlight_theme.clone(),
            is_dark: self.appearance.is_dark(),
            inline_code_family: super::typography::inline_code_family(&cx.theme().mono_font_family),
            code_block: div()
                .rounded(px(p.control_radius))
                .bg(rgb(p.code_block))
                .style()
                .clone(),
        }
    }
}

impl Workspace {
    pub(super) fn render_result(
        &self,
        key: &str,
        title: &str,
        raw: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        let copy = raw.to_owned();
        let mut card = div()
            .id(SharedString::from(format!("result-{key}")))
            .debug_selector(|| "structured-result-card".into())
            .p_3()
            .rounded_md()
            .bg(rgb(p.surface))
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().child(title.to_owned()))
                    .child(
                        self.icon_button(
                            SharedString::from(format!("copy-result-{key}")),
                            "Copy result JSON",
                            IconName::Copy,
                            true,
                        )
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                        })),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .child("Worker-reported · checks and outcomes have no host verification"),
            );
        match serde_json::from_str::<serde_json::Value>(raw) {
            Ok(serde_json::Value::Object(fields)) => {
                let mut summary = div().flex().flex_wrap().gap_2();
                for (name, value) in &fields {
                    if value.is_boolean() || value.is_number() {
                        summary = summary.child(
                            div()
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .bg(rgb(p.selected))
                                .text_size(px(12.))
                                .child(format!(
                                    "{}: {}",
                                    result_label(name),
                                    if let Some(yes) = value.as_bool() {
                                        if yes { "Yes".into() } else { "No".into() }
                                    } else {
                                        value.to_string()
                                    }
                                )),
                        );
                    }
                }
                card = card.child(summary);
                // Caveats are always visible; arbitrary schema fields remain readable.
                for (name, value) in fields
                    .iter()
                    .filter(|(k, _)| matches!(k.as_str(), "caveat" | "caveats"))
                    .chain(fields.iter().filter(|(k, v)| {
                        !matches!(k.as_str(), "caveat" | "caveats")
                            && !v.is_boolean()
                            && !v.is_number()
                    }))
                {
                    let text = result_value(value);
                    card = card.child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(
                                        if matches!(name.as_str(), "caveat" | "caveats") {
                                            p.warning
                                        } else {
                                            p.muted
                                        },
                                    ))
                                    .child(result_label(name)),
                            )
                            .child(result_literal(&format!("{key}-{name}"), &text, window, cx)),
                    );
                }
                if fields.is_empty() {
                    card = card.child("No fields reported");
                }
            }
            _ => {
                card = card
                    .child(
                        div()
                            .text_color(rgb(p.warning))
                            .child("Result is not a complete JSON object · showing original text"),
                    )
                    .child(result_literal(key, raw, window, cx));
            }
        }
        card
    }
}

fn result_literal(key: &str, text: &str, window: &mut Window, cx: &mut App) -> TextView {
    TextView::html(
        SharedString::from(format!("result-text-{key}")),
        format!(
            "<div>{}</div>",
            wks_native::transcript::escape_native_html_text(text).replace('\n', "<br>")
        ),
        window,
        cx,
    )
    .selectable(true)
}

fn result_label(key: &str) -> String {
    let mut label = String::new();
    for c in key.chars() {
        if c == '_' || c == '-' {
            label.push(' ');
        } else {
            if c.is_uppercase() && !label.is_empty() {
                label.push(' ');
            }
            label.push(c);
        }
    }
    label
}

fn result_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "None reported".into(),
        serde_json::Value::String(text) if text.is_empty() => "None reported".into(),
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Array(items) if items.is_empty() => "None reported".into(),
        serde_json::Value::Array(items) => items
            .iter()
            .map(|v| format!("• {}", result_value(v)))
            .collect::<Vec<_>>()
            .join("\n"),
        serde_json::Value::Object(fields) if fields.is_empty() => "None reported".into(),
        serde_json::Value::Object(_) => serde_json::to_string_pretty(value).unwrap_or_default(),
        _ => value.to_string(),
    }
}
