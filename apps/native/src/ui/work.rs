//! Work cards: a run of adjacent tool calls as one overview card, following the
//! desktop WorkCard. A summary header (steps, files, commands, +/−, duration)
//! sits above one-line steps; clicking a step opens its input/output in place.
use super::*;
use gpui::AnyElement;
use gpui_component::{Icon, IconName};
use std::ops::Range;
use wks_native::{model::Row, tool_preview, transcript::Tool};

/// Older steps beyond this stay behind an "earlier steps" toggle.
const VISIBLE_STEPS: usize = 6;

fn category_icon(tool: &Tool) -> IconName {
    match tool.category() {
        "Edit" => IconName::Replace,
        "Read" => IconName::File,
        "Command" => IconName::SquareTerminal,
        "Search" => IconName::Search,
        _ if tool.name.to_ascii_lowercase().contains("web") => IconName::Globe,
        _ => IconName::Settings,
    }
}

pub(super) fn spinner(id: String, size: f32) -> AnyElement {
    Icon::new(IconName::LoaderCircle)
        .size(px(size))
        .with_animation(
            SharedString::from(id),
            Animation::new(std::time::Duration::from_millis(800)).repeat(),
            |icon, progress| {
                icon.transform(gpui::Transformation::rotate(gpui::percentage(progress)))
            },
        )
        .into_any_element()
}

impl Workspace {
    fn toggle_tool(&mut self, identity: String, expanded: bool, cx: &mut Context<Self>) {
        if !self
            .view
            .transcript
            .rows
            .iter()
            .any(|r| tools::identity(r) == identity)
        {
            return;
        }
        self.pause_follow();
        let anchor = self.scroll_anchor();
        self.tool_expansion.insert(identity, !expanded);
        let count = self.view.transcript.rows.len();
        self.list.splice(0..count, count);
        self.list.scroll_to(anchor);
        cx.notify();
    }

    pub(super) fn toggle_open(&mut self, key: String, default: bool, cx: &mut Context<Self>) {
        if self.chat.open.len() > 2048 {
            self.chat.open.clear();
        }
        self.pause_follow();
        let anchor = self.scroll_anchor();
        let value = self.chat.open.entry(key).or_insert(default);
        *value = !*value;
        let count = self.view.transcript.rows.len();
        self.list.splice(0..count, count);
        self.list.scroll_to(anchor);
        cx.notify();
    }

    /// `round_top`/`round_bottom` give the hover fill the card's corners;
    /// overflow clipping is rectangular, so a square fill would poke out.
    fn render_step(
        &mut self,
        row: &Row,
        (round_top, round_bottom): (bool, bool),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let tool = row.tool.as_ref().expect("tool row");
        let session = self.view.selected.clone().unwrap_or_default();
        let identity = tools::identity(row);
        let element_key = format!("{session}-{identity}");
        let expanded = self
            .tool_expansion
            .get(&identity)
            .copied()
            .unwrap_or(tool.is_error);
        let preview = tool_preview::parse(
            &tool.name,
            &tool.input,
            tool.complete.then_some(tool.output.as_str()),
        );
        let cwd = self
            .selected_session()
            .map(|s| s.cwd.trim_end_matches('/').to_owned())
            .unwrap_or_default();
        let target = if preview.target == preview.title {
            String::new()
        } else if !cwd.is_empty()
            && let Some(relative) = preview.target.strip_prefix(&format!("{cwd}/"))
        {
            relative.to_owned()
        } else {
            preview.target.clone()
        };
        let duration = match (row.timestamp_ms, tool.completed_at_ms) {
            (Some(start), Some(end)) if end >= start => Some(timing::duration_label(end - start)),
            _ => None,
        };
        let mono = gpui_component::Theme::global(cx).mono_font_family.clone();
        let key = row.key;
        let radius = px((p.panel_radius - 1.).max(0.));
        let header = div()
            .id(SharedString::from(format!("{element_key}-step")))
            .when(round_top, |d| d.rounded_t(radius))
            .when(round_bottom && !expanded, |d| d.rounded_b(radius))
            .debug_selector(move || format!("tool-toggle-{key}"))
            .px_3()
            .py(px(6.))
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .hover(|s| s.bg(rgb(p.selected)))
            .child(
                Icon::new(category_icon(tool))
                    .size(px(13.))
                    .flex_shrink_0()
                    .text_color(rgb(if tool.is_error { p.error } else { p.muted })),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .max_w(px(320.))
                    .truncate()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(p.text))
                    .child(preview.title.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(mono)
                    .text_size(px(11.5))
                    .text_color(rgb(p.muted))
                    .child(target),
            )
            .when(preview.added > 0, |d| {
                d.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(p.success))
                        .child(format!("+{}", preview.added)),
                )
            })
            .when(preview.removed > 0, |d| {
                d.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(p.error))
                        .child(format!("−{}", preview.removed)),
                )
            })
            .when(tool.is_error, |d| {
                d.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(p.error))
                        .child("Failed"),
                )
            })
            .when_some(duration, |d, label| {
                d.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(p.disabled))
                        .child(label),
                )
            })
            .when(!tool.complete && !tool.is_error, |d| {
                d.child(
                    div()
                        .text_color(rgb(p.busy))
                        .child(spinner(format!("{element_key}-spin"), 12.)),
                )
            })
            .child(
                Icon::new(if expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .size(px(12.))
                .flex_shrink_0()
                .text_color(rgb(p.disabled)),
            )
            .on_click(
                cx.listener(move |this, _, _, cx| this.toggle_tool(identity.clone(), expanded, cx)),
            );
        let mut step = div().flex().flex_col().child(header);
        if expanded {
            step = step.child(tools::details(
                &preview,
                tool,
                &element_key,
                (p, self.settings.twelve_hour_clock),
                window,
                cx,
            ));
            if row.truncated {
                step = step.child(
                    div()
                        .px_3()
                        .pb_2()
                        .text_size(px(11.))
                        .text_color(rgb(p.warning))
                        .child("Tool content was clipped to the retained-history limit."),
                );
            }
            let input = tool.value();
            if let Some(path) = input["file_path"]
                .as_str()
                .or_else(|| input["path"].as_str())
            {
                let cwd = self
                    .selected_session()
                    .map(|s| s.cwd.as_str())
                    .unwrap_or("");
                let link = super::file_viewer::tool_link(cwd, &input, path);
                step = step.child(div().px_3().pb_2().child(self.file_button(
                    &format!("live:{session}:{key}-file"),
                    link,
                    path,
                    cx,
                )));
            }
        }
        step
    }

    /// An assistant note between calls in a merged turn card, aligned with
    /// the step titles so the card reads as one narrated piece of work.
    fn render_note(
        &mut self,
        row: &Row,
        session: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let key = row.key;
        div()
            .debug_selector(move || format!("work-note-{key}"))
            .pl(px(33.))
            .pr_3()
            .py(px(6.))
            // A note introduces the calls after it, so it opens a new section.
            .border_t_1()
            .border_color(gpui::Hsla::from(rgb(p.border)).opacity(0.5))
            .text_size(px((self.settings.text_size as f32 - 1.).max(11.)))
            .child(self.render_markdown(
                &format!("work-note:{session}:{key}"),
                &row.text,
                window,
                cx,
            ))
    }

    pub(super) fn render_work_card(
        &mut self,
        span: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let view = self.view.clone();
        let rows = &view.transcript.rows;
        let session = self.view.selected.clone().unwrap_or_default();
        let card_key = format!("work:{session}:{}", tools::identity(&rows[span.start]));
        let summary = tool_preview::summarize_work(rows.range(span.clone()).map(AsRef::as_ref));
        // Merged turns interleave assistant notes; only calls count as steps.
        let steps = rows
            .range(span.clone())
            .filter(|r| r.tool.is_some())
            .count();
        let open = summary.failed > 0
            || steps == 1
            || self.chat.open.get(&card_key).copied().unwrap_or(true);
        let earlier_key = format!("{card_key}:earlier");
        let show_earlier = self.chat.open.get(&earlier_key).copied().unwrap_or(false);
        let hidden = if show_earlier {
            0
        } else {
            span.len().saturating_sub(VISIBLE_STEPS)
        };
        let hidden_steps = rows
            .range(span.start..span.start + hidden)
            .filter(|r| r.tool.is_some())
            .count();
        let radius = px((p.panel_radius - 1.).max(0.));
        let mut card = div()
            .debug_selector(|| "tool-activity-group".into())
            .w_full()
            .rounded(px(p.panel_radius))
            .border_1()
            .border_color(rgb(p.border))
            .bg(gpui::Hsla::from(rgb(p.surface)).opacity(0.6))
            .overflow_hidden()
            .flex()
            .flex_col();
        // A single call needs no header: its step row is the whole overview.
        if steps > 1 {
            let status = if summary.running > 0 {
                div()
                    .text_color(rgb(p.busy))
                    .child(spinner(format!("{card_key}-spin"), 13.))
            } else if summary.failed > 0 {
                div()
                    .text_color(rgb(p.error))
                    .child(Icon::new(IconName::TriangleAlert).size(px(13.)))
            } else {
                div()
                    .text_color(rgb(p.success))
                    .child(Icon::new(IconName::Check).size(px(13.)))
            };
            let toggle_key = card_key.clone();
            card = card.child(
                div()
                    .id(SharedString::from(format!("{card_key}-header")))
                    .debug_selector(|| "work-card-toggle".into())
                    .rounded_t(radius)
                    .when(!open, |d| d.rounded_b(radius))
                    .px_3()
                    .py_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(p.selected)))
                    .child(status)
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(p.text))
                            .child(format!("{steps} steps")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(rgb(p.muted))
                            .child(summary.text.clone()),
                    )
                    .when(summary.added > 0, |d| {
                        d.child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(p.success))
                                .child(format!("+{}", summary.added)),
                        )
                    })
                    .when(summary.removed > 0, |d| {
                        d.child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(p.error))
                                .child(format!("−{}", summary.removed)),
                        )
                    })
                    .when(summary.running > 0, |d| {
                        d.child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(p.busy))
                                .child(format!("{} running", summary.running)),
                        )
                    })
                    .when(summary.failed > 0, |d| {
                        d.child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(p.error))
                                .child(format!("{} failed", summary.failed)),
                        )
                    })
                    .when_some(summary.duration_ms, |d, ms| {
                        d.child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(p.disabled))
                                .child(timing::duration_label(ms)),
                        )
                    })
                    .child(
                        Icon::new(if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size(px(12.))
                        .text_color(rgb(p.disabled)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_open(toggle_key.clone(), true, cx)
                    })),
            );
        }
        if open {
            let mut list = div()
                .flex()
                .flex_col()
                .when(steps > 1, |d| d.border_t_1().border_color(rgb(p.border)));
            if hidden > 0 {
                list = list.child(
                    div()
                        .id(SharedString::from(format!("{card_key}-earlier")))
                        .px_3()
                        .py(px(5.))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(p.selected)))
                        .text_size(px(11.))
                        .text_color(rgb(p.muted))
                        .child(match hidden_steps {
                            0 => "Earlier notes".to_owned(),
                            1 => "1 earlier step".to_owned(),
                            n => format!("{n} earlier steps"),
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_open(earlier_key.clone(), false, cx)
                        })),
                );
            }
            let shown = rows.range(span.start + hidden..span.end);
            let last = shown.len().saturating_sub(1);
            for (n, row) in shown.enumerate() {
                list = list.child(if row.tool.is_some() {
                    self.render_step(row, (steps == 1, n == last), window, cx)
                } else {
                    self.render_note(row, &session, window, cx)
                });
            }
            card = card.child(list);
        }
        card
    }
}
