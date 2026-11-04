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
        let p = self.palette();
        let mut style = self.markdown_style(self.link_handler(cx), false, cx);
        let (hover, menu) = super::link_menu::link_events(cx.entity().downgrade(), None);
        style.on_link_hover = Some(hover);
        style.on_link_menu = Some(menu);
        let key = key.to_owned();
        let debug_key = key.clone();
        // area-transcript: v2 left-aligns text columns (numbers stay right).
        let aligned = if self.design().is_v2() {
            left_align_text_columns(source)
        } else {
            None
        };
        let source = aligned.as_deref().unwrap_or(source);
        div()
            .id(SharedString::from(key.clone()))
            .debug_selector(move || format!("markdown-inline-{debug_key}"))
            .min_w_0()
            .w_full()
            .text_color(rgb(p.prose))
            .line_height(gpui::relative(1.6))
            .when(self.design().is_v2(), |d| {
                d.line_height(gpui::relative(p.tokens.transcript().prose_line_height))
            })
            .child(
                TextView::markdown(
                    SharedString::from(format!("markdown-{key}")),
                    self.markdown_source(&key, source),
                    window,
                    cx,
                )
                .style(style)
                .selectable(true),
            )
    }
}

impl Workspace {
    /// `source` as shared text, reusing the copy this key last drew when it
    /// is unchanged: a frame then compares each message instead of copying it.
    fn markdown_source(&self, key: &str, source: &str) -> SharedString {
        let mut held = self.markdown_sources.borrow_mut();
        if let Some(text) = held.get(key)
            && text.as_ref() == source
        {
            return text.clone();
        }
        if held.len() >= 2048 {
            held.clear();
        }
        let text = SharedString::from(source.to_owned());
        held.insert(key.to_owned(), text.clone());
        text
    }

    /// The chat's Markdown look, shared by the file viewer's document
    /// preview. `document` scales headings up for long-form reading.
    pub(super) fn markdown_style(
        &self,
        on_link_click: Arc<gpui_component::text::LinkClickFn>,
        document: bool,
        cx: &App,
    ) -> TextViewStyle {
        if self.design().is_v2() {
            return self.markdown_style_v2(on_link_click, document, cx);
        }
        let p = self.palette();
        TextViewStyle {
            on_link_click: Some(on_link_click),
            // Chat paths resolve against the session's cwd on click. A
            // previewed document's own relative paths mean something else.
            linkify: (!document).then(linkify),
            // Set by the caller that knows how its links resolve.
            on_link_hover: None,
            on_link_menu: None,
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
            layout: None,
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
        let p = self.palette();
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

/// File path, commit and reference detection for chat and tool output.
pub(super) fn linkify() -> Arc<gpui_component::text::LinkifyFn> {
    use gpui_component::text::LinkifySource;
    use wks_native::path_detect::{Source, links};
    Arc::new(|text, source| {
        links(
            text,
            match source {
                LinkifySource::Text => Source::Text,
                LinkifySource::InlineCode => Source::InlineCode,
                LinkifySource::CodeBlock => Source::Block,
            },
        )
    })
}

// area-transcript
impl Workspace {
    /// The v2 prose look: one accent (links only), neutral inline code and
    /// bullets, headings on the v2 rhythm, and code blocks on the card
    /// surface with a hairline edge rather than a hole in the page.
    fn markdown_style_v2(
        &self,
        on_link_click: Arc<gpui_component::text::LinkClickFn>,
        document: bool,
        cx: &App,
    ) -> TextViewStyle {
        let p = self.palette();
        let t = p.tokens;
        let tt = t.transcript();
        let steps = tt.heading_steps;
        TextViewStyle {
            on_link_click: Some(on_link_click),
            linkify: (!document).then(linkify),
            on_link_hover: None,
            on_link_menu: None,
            unordered_list_marker: Some("• ".into()),
            prose: Some(ProseColors {
                strong: rgb(p.text).into(),
                code: rgb(p.text).into(),
                code_background: rgb(p.code_inline).into(),
                marker: rgb(p.roles.text_tertiary).into(),
                muted: rgb(p.roles.text_secondary).into(),
                rule: rgb(p.border).into(),
                border: rgb(p.roles.card_border).into(),
                code_header: rgb(p.code_header).into(),
            }),
            paragraph_gap: gpui::rems(if document {
                tt.paragraph_gap * 1.25
            } else {
                tt.paragraph_gap
            }),
            heading_base_font_size: px(self.settings.text_size as f32),
            heading_font_size: Some(if document {
                Arc::new(move |level, base| {
                    base + px(match level {
                        1 => steps[0] * 2.,
                        2 => steps[1] * 2.,
                        3 => steps[2] * 2.,
                        _ => steps[3],
                    })
                })
            } else {
                Arc::new(move |level, base| {
                    base + px(match level {
                        1 => steps[0],
                        2 => steps[1],
                        3 => steps[2],
                        _ => steps[3],
                    })
                })
            }),
            highlight_theme: cx.theme().highlight_theme.clone(),
            is_dark: self.appearance.is_dark(),
            inline_code_family: super::typography::inline_code_family(&cx.theme().mono_font_family),
            code_block: div()
                .rounded(px(t.radius.control))
                .bg(rgb(p.code_block))
                .border(px(t.stroke.hairline))
                .border_color(rgb(p.roles.card_border))
                .style()
                .clone(),
            // r2-transcript: prose stops at ~80ch while code and tables span
            // the column; headings get room above and lose the underline.
            layout: Some(gpui_component::text::ProseLayout {
                measure: (!document)
                    .then(|| px(self.settings.text_size as f32 * tt.prose_measure_em)),
                heading_space: px(tt.heading_space),
                heading_gap: px(tt.heading_gap),
                heading_rule: false,
            }),
        }
    }
}

/// Splits one Markdown table row into its cells (outer pipes dropped,
/// escaped pipes kept).
fn table_cells(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let inner = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let inner = match inner.strip_suffix('|') {
        Some(rest) if !rest.ends_with('\\') => rest,
        _ => inner,
    };
    let mut cells = vec![String::new()];
    let mut escaped = false;
    for c in inner.chars() {
        match c {
            '|' if !escaped => cells.push(String::new()),
            _ => cells.last_mut().expect("one cell").push(c),
        }
        escaped = c == '\\' && !escaped;
    }
    cells.into_iter().map(|c| c.trim().to_owned()).collect()
}

/// A delimiter row's cells (`:--`, `:-:`, `--:`), or `None` if `line` is
/// not one.
fn delimiter_row(line: &str) -> Option<Vec<String>> {
    if !line.contains('-') || !line.contains('|') {
        return None;
    }
    let cells = table_cells(line);
    cells
        .iter()
        .all(|c| {
            let c = c.trim_start_matches(':').trim_end_matches(':');
            !c.is_empty() && c.chars().all(|ch| ch == '-')
        })
        .then_some(cells)
}

/// Whether a cell reads as a number: digits with the usual punctuation,
/// units and signs ("1,024", "-3.5%", "$0.04", "12 ms", "+2").
fn numeric(cell: &str) -> bool {
    let cell = cell
        .trim_matches(|c| c == '*' || c == '`' || c == '_')
        .trim();
    let digits = cell.chars().filter(char::is_ascii_digit).count();
    let visible = cell.chars().filter(|c| !c.is_whitespace()).count();
    digits > 0
        && digits * 2 >= visible
        && cell
            .chars()
            .all(|c| c.is_ascii_digit() || c.is_whitespace() || ",.%$€£+-−×:/kKMGBmsh".contains(c))
}

/// Markdown with every right- or centre-aligned table column whose cells are
/// not numbers re-marked as left-aligned: prose reads from a shared left
/// edge, and only figures line up on the right. `None` when nothing changes.
pub(super) fn left_align_text_columns(source: &str) -> Option<String> {
    let lines: Vec<&str> = source.split('\n').collect();
    let mut out: Vec<String> = lines.iter().map(|l| (*l).to_owned()).collect();
    let mut changed = false;
    let mut fence: Option<&str> = None;
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        if let Some(open) = fence {
            if trimmed.starts_with(open) {
                fence = None;
            }
            i += 1;
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = Some(&trimmed[..3]);
            i += 1;
            continue;
        }
        let Some(aligns) = (line.contains('|') && i + 1 < lines.len())
            .then(|| delimiter_row(lines[i + 1]))
            .flatten()
        else {
            i += 1;
            continue;
        };
        let mut end = i + 2;
        while end < lines.len() && lines[end].contains('|') && !lines[end].trim().is_empty() {
            end += 1;
        }
        let body: Vec<Vec<String>> = lines[i + 2..end].iter().map(|l| table_cells(l)).collect();
        let mut aligned = aligns.clone();
        for (column, align) in aligns.iter().enumerate() {
            if !align.ends_with(':') {
                continue;
            }
            let all_numbers = body
                .iter()
                .filter_map(|row| row.get(column))
                .filter(|cell| !cell.is_empty())
                .all(|cell| numeric(cell));
            if !all_numbers {
                aligned[column] = ":--".into();
            }
        }
        if aligned != aligns {
            out[i + 1] = format!("| {} |", aligned.join(" | "));
            changed = true;
        }
        i = end;
    }
    changed.then(|| out.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::left_align_text_columns as align;

    #[test]
    fn text_columns_read_from_the_left_and_numbers_stay_right() {
        let source = "| Area | Status | Notes | Lines |\n|:--|:-:|--:|--:|\n| Tables | Done | Header, stripes | 1,204 |\n| Quotes | Done | Accent rail | 12 |";
        let aligned = align(source).expect("text columns change");
        let delimiter = aligned.lines().nth(1).unwrap();
        assert_eq!(delimiter, "| :-- | :-- | :-- | --: |");
        assert!(aligned.ends_with("| Quotes | Done | Accent rail | 12 |"));
    }

    #[test]
    fn left_aligned_tables_and_code_fences_are_untouched() {
        assert_eq!(align("| A | B |\n|---|:--|\n| x | y |"), None);
        assert_eq!(align("```\n| A | B |\n|--:|--:|\n| x | y |\n```"), None);
        assert_eq!(align("no tables here"), None);
        assert_eq!(align("| Cost |\n|--:|\n| $0.04 |\n| 12 ms |"), None);
    }
}
