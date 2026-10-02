use super::*;
use gpui::{AnyElement, WeakEntity};
use gpui_component::tooltip::Tooltip;
use gpui_component::{Icon, IconName};
use wks_native::{model::Row, tool_preview};

pub(super) fn identity(row: &Row) -> String {
    row.tool
        .as_ref()
        .filter(|tool| !tool.id.is_empty())
        .map(|tool| format!("call:{}", tool.id))
        .unwrap_or_else(|| format!("row:{}", row.key))
}

pub(super) fn card(
    row: &Row,
    session: &str,
    expansion: Option<bool>,
    workspace: WeakEntity<Workspace>,
    appearance: (Palette, bool),
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let (p, twelve_hour) = appearance;
    let tool = row.tool.as_ref().expect("tool row");
    let origin = wks_native::child_agents::tool_kind(tool);
    let mut preview = tool_preview::parse(
        &tool.name,
        &tool.input,
        tool.complete.then_some(tool.output.as_str()),
    );
    if matches!(tool.category(), "Skill" | "Subagent" | "Workflow") {
        preview.target = tool.target();
    }
    if tool.category() == "Subagent" && preview.description.is_empty() {
        preview.title = "Spawn agent".into();
    }
    let expanded = expansion.unwrap_or(tool.is_error);
    let key = row.key;
    let stable_identity = identity(row);
    let element_key = format!("{session}-{stable_identity}");
    let title = preview.title.clone();
    let subtitle = if preview.target == title {
        ""
    } else {
        &preview.target
    };
    let (status, color, icon) = if tool.is_error {
        ("Failed", p.error, IconName::TriangleAlert)
    } else if tool.complete && tool.category() == "Subagent" {
        ("Dispatched", p.success, IconName::Check)
    } else if tool.complete {
        ("Done", p.success, IconName::Check)
    } else {
        ("Running", p.busy, IconName::LoaderCircle)
    };
    let status = match (row.timestamp_ms, tool.completed_at_ms) {
        (Some(start), Some(end)) if end >= start => {
            format!("{status} · {}", timing::duration_label(end - start))
        }
        _ => status.into(),
    };
    let status_icon = Icon::new(icon).size(px(12.));
    let status_icon = if !tool.complete && !tool.is_error {
        status_icon
            .with_animation(
                SharedString::from(format!("{element_key}-spinner")),
                Animation::new(std::time::Duration::from_millis(800)).repeat(),
                |icon, progress| {
                    icon.transform(gpui::Transformation::rotate(gpui::percentage(progress)))
                },
            )
            .into_any_element()
    } else {
        status_icon.into_any_element()
    };
    let details =
        if expanded {
            let mut body = div()
                .id(SharedString::from(format!("{element_key}-details")))
                .max_h(px(520.))
                .overflow_y_scroll()
                .px_3()
                .pb_3()
                .pt_2()
                .border_t_1()
                .border_color(rgb(p.border))
                .flex()
                .flex_col()
                .gap_3()
                .when(!preview.working_directory.is_empty(), |d| {
                    d.child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.muted))
                            .child(preview.working_directory.clone()),
                    )
                })
                .when(!preview.description.is_empty(), |d| {
                    d.child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(p.text))
                            .child(preview.description.clone()),
                    )
                });
            for (index, block) in preview.blocks.iter().take(16).enumerate() {
                let text = block.text.lines().take(160).collect::<Vec<_>>().join("\n");
                let text: String = text.chars().take(12_000).collect();
                let clipped = text.len() < block.text.trim_end_matches('\n').len();
                body = body.child(
                    div()
                        .flex_shrink_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_size(px(11.))
                                .text_color(rgb(p.muted))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .child(block.label.clone()),
                                )
                                .when(block.language != "text", |d| d.child(block.language)),
                        )
                        .child(
                            TextView::markdown(
                                SharedString::from(format!("{element_key}-{index}")),
                                tool_preview::fenced(block.language, &text),
                                window,
                                cx,
                            )
                            .style(gpui_component::text::TextViewStyle {
                                highlight_theme: gpui_component::Theme::global(cx)
                                    .highlight_theme
                                    .clone(),
                                is_dark: gpui_component::Theme::global(cx).mode.is_dark(),
                                ..Default::default()
                            })
                            .selectable(true),
                        )
                        .when(
                            block.label == "Output" && tool.completed_at_ms.is_some(),
                            |d| {
                                d.child(div().text_size(px(11.)).text_color(rgb(p.muted)).child(
                                    timing::timestamp_label(
                                        tool.completed_at_ms,
                                        timing::now_ms(),
                                        twelve_hour,
                                    ),
                                ))
                            },
                        )
                        .when(clipped, |d| {
                            d.child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(p.muted))
                                    .child("Preview shortened"),
                            )
                        }),
                );
            }
            if preview.blocks.len() > 16 {
                body = body.child(div().text_size(px(11.)).text_color(rgb(p.muted)).child(
                    format!(
                        "{} additional sections omitted from preview",
                        preview.blocks.len() - 16
                    ),
                ));
            }
            Some(body)
        } else {
            None
        };
    div()
        .id(SharedString::from(format!("{element_key}-card")))
        .rounded(px(p.panel_radius))
        .overflow_hidden()
        .child(
            chrome::interactive_control(
                div().id(SharedString::from(format!("{element_key}-toggle"))),
                p,
                true,
            )
            .rounded(px(p.panel_radius))
            .focus(|s| s.border_color(rgb(p.accent)).bg(rgb(p.selected)))
            .debug_selector(|| format!("tool-toggle-{key}"))
            .cursor_pointer()
            .px_3()
            .py_2()
            .flex()
            .items_start()
            .gap_2()
            .hover(|s| s.bg(rgb(p.selected)))
            .child(
                Icon::new(if expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .size(px(14.))
                .mt(px(2.))
                .text_color(rgb(p.muted)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_3()
                            .pr_6()
                            .when_some(origin, |d, origin| {
                                let managed =
                                    origin == wks_native::child_agents::ChildKind::Workspacer;
                                d.child(
                                    div()
                                        .id(SharedString::from(format!("{element_key}-origin")))
                                        .debug_selector(move || {
                                            if managed {
                                                "workspacer-spawn-icon".into()
                                            } else {
                                                "native-spawn-icon".into()
                                            }
                                        })
                                        .tooltip(move |window, cx| {
                                            Tooltip::new(if managed {
                                                "Workspacer spawn"
                                            } else {
                                                "Provider-native subagent"
                                            })
                                            .build(window, cx)
                                        })
                                        .child(
                                            Icon::new(if managed {
                                                IconName::Bot
                                            } else {
                                                IconName::SquareTerminal
                                            })
                                            .size(px(14.))
                                            .text_color(rgb(p.accent)),
                                        ),
                                )
                            })
                            .child(
                                div()
                                    .min_w_0()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(title),
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
                            .child(
                                div()
                                    .flex()
                                    .text_size(px(11.))
                                    .items_center()
                                    .gap_1()
                                    .text_color(rgb(color))
                                    .child(status_icon)
                                    .child(status),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(p.muted))
                                    .child(tool.category()),
                            ),
                    )
                    .when(!subtitle.is_empty(), |d| {
                        d.child(
                            div()
                                .truncate()
                                .text_size(px(11.))
                                .text_color(rgb(p.muted))
                                .font_family(
                                    gpui_component::Theme::global(cx).mono_font_family.clone(),
                                )
                                .child(subtitle.to_owned()),
                        )
                    }),
            )
            .on_click(move |_, _, cx| {
                let _ = workspace.update(cx, |this, cx| {
                    if this
                        .view
                        .transcript
                        .rows
                        .iter()
                        .any(|r| identity(r) == stable_identity)
                    {
                        this.pause_follow();
                        let anchor = this.scroll_anchor();
                        this.tool_expansion
                            .insert(stable_identity.clone(), !expanded);
                        this.list.splice(
                            0..this.view.transcript.rows.len(),
                            this.view.transcript.rows.len(),
                        );
                        this.list.scroll_to(anchor);
                    }
                    cx.notify();
                });
            }),
        )
        .when(row.truncated, |d| {
            d.child(
                div()
                    .px_3()
                    .pb_2()
                    .text_size(px(11.))
                    .text_color(rgb(p.warning))
                    .child("Tool content was clipped to the retained-history limit."),
            )
        })
        .children(details)
        .into_any_element()
}
