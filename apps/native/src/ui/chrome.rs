//! Shared native chrome: quiet actions, readable sections, and preference rows.
use super::gauge::{self, Gauge, context_notice};
use super::island::Notice;
use super::*;
use gpui::AnyElement;
use gpui_component::tooltip::Tooltip;

/// GPUI 0.2.2 replaces the entire text refinement in interaction styles.
/// Seed it from the base style so a color change preserves size, weight and font.
pub(super) trait ControlTextStyle: Sized {
    fn hover_text_style(
        self,
        f: impl FnOnce(gpui::StyleRefinement) -> gpui::StyleRefinement,
    ) -> Self;
    fn active_text_style(
        self,
        f: impl FnOnce(gpui::StyleRefinement) -> gpui::StyleRefinement,
    ) -> Self;
}

impl ControlTextStyle for Stateful<Div> {
    fn hover_text_style(
        mut self,
        f: impl FnOnce(gpui::StyleRefinement) -> gpui::StyleRefinement,
    ) -> Self {
        let text = self.style().text.clone();
        self.hover(|mut style| {
            style.text = text;
            f(style)
        })
    }

    fn active_text_style(
        mut self,
        f: impl FnOnce(gpui::StyleRefinement) -> gpui::StyleRefinement,
    ) -> Self {
        let text = self.style().text.clone();
        self.active(|mut style| {
            style.text = text;
            f(style)
        })
    }
}

/// GPUI owns stable focus handles and Enter/Space activation for focusable divs.
/// Reserve the ring at rest so focus never changes control geometry. The ring
/// is keyboard-only (`focus_visible`): a press still focuses the control, but
/// an accent border left behind would read as a chip or toggle still "on".
pub(super) fn interactive_control(
    control: Stateful<Div>,
    p: Palette,
    enabled: bool,
) -> Stateful<Div> {
    control
        .border_2()
        .border_color(gpui::rgba(0))
        .when(enabled, |d| {
            d.focusable()
                .tab_stop(true)
                .key_context("NativeControl")
                .cursor_pointer()
                .focus_visible(|s| s.border_color(rgb(p.accent)))
        })
}

/// Shared outer measure and gutters for transcript, header and composer.
/// Anthropic brand clay, as the desktop's `CLAUDE_CLAY`.
pub(super) const CLAUDE_CLAY: u32 = 0xD97757;

/// Provider mark and brand color, following desktop `agentLogos.tsx`: Claude
/// keeps its clay; the OpenAI mark (Codex) takes the text color.
pub(super) fn model_badge(session: &Session, p: Palette, size: f32) -> Div {
    brand_badge(&session.provider, session.display_model(), p, size)
}

/// A provider's brand mark on a rounded tile, for choosing or describing an
/// agent (New Agent, Agent setup). Same marks and colors as [`brand_badge`].
pub(super) fn provider_mark(provider: &str, size: f32, p: Palette) -> Div {
    let (mark, color) = match provider {
        "claude" => (Some("brand/claude.svg"), CLAUDE_CLAY),
        "codex" => (Some("brand/openai.svg"), p.text),
        _ => (None, p.accent),
    };
    let glyph = size * 0.5;
    div()
        .size(px(size))
        .flex_shrink_0()
        .rounded(px((size * 0.3).min(p.control_radius)))
        .bg(rgb(p.selected))
        .flex()
        .items_center()
        .justify_center()
        .child(match mark {
            Some(path) => gpui::svg()
                .path(path)
                .size(px(glyph))
                .text_color(rgb(color))
                .into_any_element(),
            None => Icon::new(IconName::Bot)
                .size(px(glyph))
                .text_color(rgb(color))
                .into_any_element(),
        })
}

/// The badge for any provider and display name (sessions and child agents).
pub(super) fn brand_badge(provider: &str, name: String, p: Palette, size: f32) -> Div {
    let (mark, color) = match provider {
        "claude" => (Some("brand/claude.svg"), CLAUDE_CLAY),
        "codex" => (Some("brand/openai.svg"), p.text),
        _ => (None, p.accent),
    };
    let name = if name.is_empty() {
        match provider {
            "claude" => "Claude".to_owned(),
            "codex" => "Codex".to_owned(),
            "" => "Agent".to_owned(),
            other => other.to_owned(),
        }
    } else {
        name
    };
    div()
        .flex()
        .items_center()
        .gap(px(5.))
        .min_w_0()
        .child(match mark {
            Some(path) => gpui::svg()
                .path(path)
                .size(px(size))
                .flex_shrink_0()
                .text_color(rgb(color))
                .into_any_element(),
            None => Icon::new(IconName::Bot)
                .size(px(size))
                .flex_shrink_0()
                .text_color(rgb(color))
                .into_any_element(),
        })
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_color(rgb(color))
                .child(name),
        )
}

/// Native type scale for chrome outside the conversation. Chat prose follows
/// the reader's text-size setting instead; these keep pages consistent.
pub(super) mod scale {
    /// Page titles (Settings, Projects, Changes…); `TITLE_SHORT` in short windows.
    pub const TITLE: f32 = 22.;
    pub const TITLE_SHORT: f32 = 18.;
    /// Card and section headings.
    pub const HEADING: f32 = 15.;
    /// Body copy and control labels on pages.
    pub const BODY: f32 = 13.;
    /// Descriptions, metadata and notices.
    pub const META: f32 = 12.;
    /// Captions, chips and hints.
    pub const CAPTION: f32 = 11.;
    /// Uppercase overlines.
    pub const OVERLINE: f32 = 10.;
}

/// The shared raised surface for page content: settings groups, launch
/// sections and secondary screens all use this one card treatment.
pub(super) fn card(p: Palette) -> Div {
    div()
        .rounded(px(p.panel_radius))
        .bg(rgb(p.surface))
        .border_1()
        .border_color(rgb(p.border))
}

/// What a status line means; color and icon follow from it, so loading is
/// never shown as a warning and a success never reads as an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tone {
    Info,
    Loading,
    Success,
    Warning,
    Error,
}

/// One wrapped status line with its tone's icon; for notices on pages.
pub(super) fn notice_line(
    text: impl Into<SharedString>,
    tone: Tone,
    p: Palette,
    id: impl Into<gpui::ElementId>,
) -> Div {
    let (color, text_color) = match tone {
        Tone::Info | Tone::Loading => (p.muted, p.muted),
        Tone::Success => (p.success, p.text),
        Tone::Warning => (p.warning, p.warning),
        Tone::Error => (p.error, p.error),
    };
    div()
        .w_full()
        .flex()
        .items_start()
        .gap_2()
        .text_size(px(scale::META))
        .line_height(gpui::relative(1.45))
        .text_color(rgb(text_color))
        .child(
            div()
                .flex_shrink_0()
                .h(px(scale::META * 1.45))
                .flex()
                .items_center()
                .child(match tone {
                    Tone::Loading => brand_spinner(11., p, id).into_any_element(),
                    Tone::Success => Icon::new(IconName::CircleCheck)
                        .size(px(13.))
                        .text_color(rgb(color))
                        .into_any_element(),
                    Tone::Info => Icon::new(IconName::Info)
                        .size(px(13.))
                        .text_color(rgb(color))
                        .into_any_element(),
                    Tone::Warning | Tone::Error => Icon::new(IconName::TriangleAlert)
                        .size(px(13.))
                        .text_color(rgb(color))
                        .into_any_element(),
                }),
        )
        .child(div().flex_1().min_w_0().child(text.into()))
}

/// A notice row on its way out: the same words, tone icon and dismiss
/// glyph, no longer interactive, fading in place before the island closes
/// over it.
pub(super) fn island_ghost(
    slot: &'static str,
    text: &str,
    tone: Tone,
    dismissible: bool,
    p: Palette,
) -> AnyElement {
    div()
        .w_full()
        .flex()
        .items_start()
        .gap_1()
        .child(div().flex_1().min_w_0().py(px(5.)).child(notice_line(
            text.to_owned(),
            tone,
            p,
            SharedString::from(format!("island-ghost-{slot}-icon")),
        )))
        .when(dismissible, |d| {
            d.child(
                div()
                    .size(px(24.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(p.muted))
                    .child(Icon::new(IconName::Close).size(px(13.))),
            )
        })
        .into_any_element()
}

/// Free text from the hub or a local action, classified by how it reads.
/// Used where one notice slot carries both confirmations and failures.
pub(super) fn notice_tone(text: &str) -> Tone {
    let lower = text.to_lowercase();
    if lower.ends_with('…') && !lower.contains("fail") && !lower.contains("could not") {
        Tone::Loading
    } else if lower.starts_with("change queued") {
        // Accepted for later, not a problem.
        Tone::Info
    } else if lower.contains("saved")
        || lower == "session created"
        || lower.starts_with("handoff ready:")
        || lower.starts_with("pinned")
        || lower.starts_with("unpinned")
        || lower.ends_with(" applied")
        || lower == "committed."
        || lower == "pushed."
        || lower == "pulled."
        || lower.starts_with("discarded changes to ")
        || lower.contains(" change accepted:")
    {
        Tone::Success
    } else if lower.contains("fail")
        || lower.contains("error")
        || lower.contains("could not")
        || lower.contains("couldn't")
        || lower.contains("cannot")
        || lower.contains("unavailable")
        || lower.contains("refused")
    {
        Tone::Error
    } else {
        Tone::Warning
    }
}

pub(super) fn chat_column() -> Div {
    chat_frame().px_5()
}

/// The chat column's outer measure without its gutters, for a frame whose
/// child carries them (and so keeps them inside its own clip).
pub(super) fn chat_frame() -> Div {
    div().w_full().max_w(px(CHAT_WIDTH + 40.)).mx_auto()
}

/// A glow around the island in a notice's tone; `strength` 1 is a new
/// notice's greeting at its peak.
pub(super) fn tone_glow(tone: Tone, strength: f32, p: Palette) -> gpui::BoxShadow {
    let color = match tone {
        Tone::Info | Tone::Loading => p.accent,
        Tone::Success => p.success,
        Tone::Warning => p.warning,
        Tone::Error => p.error,
    };
    gpui::BoxShadow {
        color: gpui::Hsla::from(rgb(color)).opacity(0.45 * strength),
        offset: gpui::point(px(0.), px(0.)),
        blur_radius: px(22.),
        spread_radius: px(2. * strength),
    }
}

pub(super) fn floating_shadow(p: Palette) -> Vec<gpui::BoxShadow> {
    vec![gpui::BoxShadow {
        color: gpui::Hsla::from(rgb(p.shadow)).opacity(p.shadow_opacity),
        offset: gpui::point(px(0.), px(6.)),
        blur_radius: px(20.),
        spread_radius: px(-4.),
    }]
}

/// An invisible overlay filling its parent that reports the parent's laid-out
/// bounds. `measured` runs on the entity once the frame is done (never inside
/// its own render) and returns whether anything changed worth a re-render.
pub(super) fn measure<T: 'static>(
    entity: gpui::WeakEntity<T>,
    measured: impl FnOnce(&mut T, gpui::Bounds<gpui::Pixels>, &mut Context<T>) -> bool + 'static,
) -> impl IntoElement {
    canvas(
        move |bounds, _, cx| {
            cx.defer(move |cx| {
                let _ = entity.update(cx, |this, cx| {
                    if measured(this, bounds, cx) {
                        cx.notify();
                    }
                });
            });
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

pub(super) fn project_label(path: &str) -> &str {
    path.rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or("Workspace")
}

impl Workspace {
    /// Compact floating title pill: status, project / title, model chip and
    /// icon actions, sized to its content rather than the whole chat width.
    /// The title capsule and its transient notices as one surface: notices
    /// grow the capsule downward, like an island, instead of floating loose
    /// beneath it. With nothing to say it stays the plain capsule. `status`
    /// is the conversation notice (local first, then the hub's).
    /// `actions` are the capsule's secondary actions, shown on hover or
    /// focus (see `island.rs`); a subagent's capsule has none.
    pub(super) fn render_title_island(
        &mut self,
        bar: Stateful<Div>,
        actions: Option<Div>,
        status: String,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.appearance.palette();
        let compact = unzoom(window.viewport_size().height) < 300.;
        let omitted = if self.view.transcript.omitted && !self.view.transcript.has_older {
            OMITTED_NOTICE
        } else {
            ""
        };
        let feature = self.extras.notice.clone();
        // The context gauge rides the main capsule unless it is too narrow
        // (then the composer keeps its meter); a subagent's capsule has none.
        let carried = actions.is_some() && self.title_carries_context();
        let gauge = self
            .selected_session()
            .filter(|_| carried)
            .and_then(Gauge::of);
        let context = self
            .selected_session()
            .filter(|_| carried)
            .and_then(context_notice);
        // Every chat's current context band, so a dismissal survives
        // switching away and back but not a change of band.
        let bands: Vec<String> = self
            .view
            .sessions
            .iter()
            .filter_map(|s| context_notice(s).map(|n| n.key))
            .collect();
        // A dismissal hides one text in one slot; once the slot moves on,
        // the same words later are news again.
        self.extras
            .dismissed_notices
            .retain(|(slot, text)| match *slot {
                "status" => *text == status,
                "feature" => *text == feature,
                "omitted" => text == omitted,
                "context" => bands.contains(text),
                _ => false,
            });
        let shown = |slot: &str, text: &str| {
            !text.is_empty()
                && !self
                    .extras
                    .dismissed_notices
                    .iter()
                    .any(|(s, t)| *s == slot && t == text)
        };
        // The retry belongs to its error: that row stays until it is used.
        let retry = self.view.connected
            && !self.view.loading
            && self.view.notice.starts_with("Conversation unavailable:");
        let (show_status, show_feature, show_omitted) = (
            shown("status", &status) || (retry && !status.is_empty()),
            shown("feature", &feature),
            shown("omitted", omitted),
        );
        let context = context.filter(|n| shown("context", &n.key));
        // Each row with what it says, so the island can move it and, once
        // it goes, fade its words out in place.
        let mut rows: Vec<(Notice, AnyElement)> = Vec::new();
        if !self.view.connected && !self.view.transcript.rows.is_empty() {
            let text: String = self.connection_copy().title.into();
            let notice = Notice {
                slot: "connection",
                key: text.clone(),
                text,
                tone: Tone::Warning,
                dismissible: false,
            };
            rows.push((notice, self.render_connection_banner(cx).into_any_element()));
        }
        if show_status {
            let action = retry.then(|| {
                self.quiet_button("retry-conversation", "Retry", IconName::Redo, true)
                    .debug_selector(|| "retry-conversation".into())
                    .py_1()
                    .on_click(cx.listener(|this, _, _, cx| this.command(Command::Refresh, cx)))
                    .into_any_element()
            });
            let tone = notice_tone(&status);
            let dismiss = (!retry).then(|| status.clone());
            let row =
                self.island_notice("status", status.clone(), tone, action, dismiss, compact, cx);
            let notice = Notice {
                slot: "status",
                key: status.clone(),
                text: status,
                tone,
                dismissible: !retry,
            };
            rows.push((notice, row));
        }
        if show_feature {
            let tone = notice_tone(&feature);
            let dismiss = Some(feature.clone());
            let row =
                self.island_notice("feature", feature.clone(), tone, None, dismiss, compact, cx);
            let notice = Notice {
                slot: "feature",
                key: feature.clone(),
                text: feature,
                tone,
                dismissible: true,
            };
            rows.push((notice, row));
        }
        // A nearly full context. Its words keep the live figures, but the
        // row is named (and dismissed) by its band, so a token tick neither
        // greets again nor undoes a dismissal.
        if let Some(context) = context {
            let row = self.island_notice(
                "context",
                context.text.clone(),
                context.tone,
                None,
                Some(context.key.clone()),
                compact,
                cx,
            );
            let notice = Notice {
                slot: "context",
                key: context.key,
                text: context.text,
                tone: context.tone,
                dismissible: true,
            };
            rows.push((notice, row));
        }
        if self.view.loading && !self.view.transcript.rows.is_empty() {
            let text = "Refreshing conversation…";
            let row = self.island_notice(
                "refresh",
                text.into(),
                Tone::Loading,
                None,
                None,
                compact,
                cx,
            );
            rows.push((
                Notice {
                    slot: "refresh",
                    key: text.into(),
                    text: text.into(),
                    tone: Tone::Loading,
                    dismissible: false,
                },
                row,
            ));
        }
        if show_omitted {
            let action =
                self.quiet_button("omitted-history", "Open History", IconName::BookOpen, true)
                    .py_1()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_feature(Screen::History, window, cx)
                    }))
                    .into_any_element();
            let row = self.island_notice(
                "omitted",
                omitted.into(),
                Tone::Info,
                Some(action),
                Some(omitted.into()),
                compact,
                cx,
            );
            rows.push((
                Notice {
                    slot: "omitted",
                    key: omitted.into(),
                    text: omitted.into(),
                    tone: Tone::Info,
                    dismissible: true,
                },
                row,
            ));
        }
        let now = std::time::Instant::now();
        let motion = !self.settings.reduce_motion;
        let (notices, mut elements): (Vec<Notice>, Vec<AnyElement>) = rows.into_iter().unzip();
        // A chat switch shows the other chat's notices where they belong.
        let chat = {
            use std::hash::{Hash, Hasher};
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            self.view.selected.hash(&mut hash);
            if let Some(child) = &self.view.child {
                (&child.parent, &child.agent).hash(&mut hash);
            }
            hash.finish()
        };
        self.island_motion.sync(&notices, chat, motion, now);
        self.gauge_motion.sync(gauge, chat, motion, now);
        let open = self.island_motion.open(now);
        // The exact figures sit at the head of the revealed actions.
        let now_ms = super::cache::now();
        let cold = self
            .selected_session()
            .and_then(|s| super::cache::cold(s, now_ms))
            .map(|cold| super::cache::marker_tooltip(cold, now_ms));
        let actions = actions.map(|actions| match gauge {
            Some(gauge) => div()
                .flex()
                .items_center()
                .gap_2()
                .child(gauge::detail(gauge, cold, p))
                .child(actions),
            None => actions,
        });
        // The island's width once grown around notices: rows lay out at it
        // throughout, so words never rewrap while the outline moves.
        let grown_width = self.title_base_width
            + if actions.is_some() {
                self.title_reveal.width
            } else {
                gpui::px(0.)
            };
        let (bar, extra) = match actions {
            Some(actions) => self.reveal_title_actions(bar, actions, open, now, window, cx),
            None => (bar, gpui::px(0.)),
        };
        if self.island_motion.moving(now) || self.gauge_motion.moving(now) {
            window.request_animation_frame();
        }
        // Measures the capsule's width inside its border, less whatever its
        // actions add this frame: notices wrap at the resting width plus
        // the actions' current share, so the tray follows the outline in the
        // same frame as it grows or shrinks. (The bar, not the island: the
        // tray's own width must not feed back into it.)
        let base = measure(
            cx.entity().downgrade(),
            move |this: &mut Self, bounds, _| {
                let base = bounds.size.width - extra;
                let changed = (this.title_base_width - base).abs() > gpui::px(0.01);
                if changed {
                    this.title_base_width = base;
                }
                changed
            },
        );
        let bar = bar
            .child(base)
            .children(gauge.map(|gauge| gauge::hairline(gauge, self.gauge_motion.fill(now), p)));
        // From 70% the capsule holds a quiet glow in the warning tone: the
        // notice greeting's glow, held low and steady.
        let warmth = self.gauge_motion.glow(now);
        if !self.island_motion.shows(now) {
            return match warmth {
                Some((tone, strength)) => {
                    let mut shadow = floating_shadow(p);
                    shadow.push(tone_glow(tone, strength, p));
                    bar.shadow(shadow)
                }
                None => bar,
            }
            .into_any_element();
        }
        // The island's 1px border replaces the bar's own, so the capsule's
        // outline stays where it was and only grows downward.
        let border = gpui::px(1.);
        let width = self.title_base_width + extra;
        // New words fade in rather than pop. With motion, each row grows
        // into place and its words follow; without, the tray just fades.
        let signature = {
            use std::hash::{Hash, Hasher};
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            (show_status, show_feature, show_omitted, notices.len()).hash(&mut hash);
            self.extras.notice.hash(&mut hash);
            self.view.notice.hash(&mut hash);
            self.local_notice.hash(&mut hash);
            hash.finish() as usize
        };
        let mut tray_rows: Vec<AnyElement> = Vec::new();
        let mut above: Option<f32> = None;
        for row in self.island_motion.rows() {
            let content = if row.leaving {
                island_ghost(row.slot, &row.text, row.tone, row.dismissible, p)
            } else {
                match notices.iter().position(|n| n.slot == row.slot) {
                    Some(ix) => std::mem::replace(&mut elements[ix], div().into_any_element()),
                    None => continue,
                }
            };
            let presence = row.presence(now);
            // The gap between two rows (`gap_1`) belongs to both: it opens
            // and closes with whichever of them is arriving or leaving.
            let gap = above.map_or(gpui::px(0.), |above| {
                window.rem_size() * 0.25 * above.min(presence)
            });
            above = Some(presence);
            let (shown, drift) = row.content(now);
            let height = row.height(now);
            let slot = row.slot;
            let row_view = cx.entity().downgrade();
            tray_rows.push(
                div()
                    .flex_shrink_0()
                    .w_full()
                    .mt(gap)
                    .flex()
                    .flex_col()
                    .when_some(height, |d, height| d.h(height).overflow_hidden())
                    .child(
                        div()
                            .flex_shrink_0()
                            // Moving, at the grown width (less the tray's
                            // `px_3`), clipped by the row; at rest, its own.
                            .map(|d| match height {
                                Some(_) if self.title_base_width > px(0.) => {
                                    d.w(grown_width - window.rem_size() * 1.5)
                                }
                                _ => d.w_full(),
                            })
                            .relative()
                            .top(drift)
                            .opacity(shown)
                            .child(content)
                            .child(measure(row_view, move |this: &mut Self, bounds, _| {
                                let now = std::time::Instant::now();
                                this.island_motion.measured(slot, bounds.size.height, now)
                            })),
                    )
                    .into_any_element(),
            );
        }
        let grown = open.max(0.);
        let tray = div()
            .id("title-island-notices")
            .debug_selector(|| "title-island-notices".into())
            // Wraps at the capsule's width instead of widening it
            // (until the first measurement, at a modest cap).
            .map(|d| {
                if self.title_base_width > px(0.) {
                    d.w(width)
                } else {
                    d.w_full().max_w(px(420.))
                }
            })
            // Keep the full message scrollable without letting an
            // arbitrary host error cover the transcript and composer.
            .max_h(
                (window.viewport_size().height
                    - self.composer_dock_bounds.size.height
                    // pt_3 + pb_5 use two rems at the user's font size.
                    - px(40.) - window.rem_size() * 2.
                    - if unzoom(window.viewport_size().height) < 300. { gpui::px(0.) } else { gpui::px(24.) })
                .min(window.viewport_size().height * 0.35)
                .max(gpui::px(1.)),
            )
            .overflow_y_scroll()
            .px_3()
            // The seam and padding (`pt_1`, `pb_2`) grow with the island.
            .when(!compact, |d| {
                d.pt(window.rem_size() * 0.25 * grown)
                    .pb(window.rem_size() * 0.5 * grown)
            })
            .flex()
            .flex_col()
            .border_t(border * grown.min(1.))
            .border_color(gpui::Hsla::from(rgb(p.border)).opacity(0.6 * grown.min(1.)))
            .children(tray_rows);
        let tray = if motion {
            tray.into_any_element()
        } else {
            tray.with_animation(
                ("title-island-reveal", signature),
                Animation::new(std::time::Duration::from_millis(180))
                    .with_easing(gpui::ease_out_quint()),
                |tray, progress| tray.opacity(progress),
            )
            .into_any_element()
        };
        // A new notice greets with a short glow in its tone, then rests.
        let mut shadow = floating_shadow(p);
        for (tone, strength) in warmth.into_iter().chain(self.island_motion.glow(now)) {
            shadow.push(tone_glow(tone, strength, p));
        }
        div()
            .id("title-island")
            .debug_selector(|| "title-island".into())
            .occlude()
            .max_w_full()
            .min_w_0()
            .flex()
            .flex_col()
            // Sized by the bar alone: the tray follows it, never widens it.
            .items_start()
            // Half the capsule's height: the top keeps the pill's curve,
            // and the lower corners soften while the island stretches.
            .rounded_t(px(ISLAND_RADIUS))
            .rounded_b(px(ISLAND_RADIUS + self.island_motion.bulge(now)))
            .bg(rgb(p.surface))
            .border_1()
            .border_color(rgb(p.border))
            .shadow(shadow)
            .child(
                bar.bg(gpui::transparent_black())
                    .border_0()
                    .h(px(40.) - border * 2.)
                    .shadow(Vec::new()),
            )
            .child(tray)
            .into_any_element()
    }

    /// One island notice: tone icon and wrapped text, then its action and a
    /// dismiss (keyboard reachable, like every native control). `dismiss` is
    /// what a dismissal records for the slot, `None` for a row that stays.
    #[allow(clippy::too_many_arguments)]
    fn island_notice(
        &self,
        slot: &'static str,
        text: String,
        tone: Tone,
        action: Option<AnyElement>,
        dismiss: Option<String>,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.appearance.palette();
        div()
            .debug_selector(move || format!("island-notice-{slot}"))
            .flex_shrink_0()
            .w_full()
            .flex()
            .items_start()
            .gap_1()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .py(px(if compact { 0. } else { 5. }))
                    .child(notice_line(
                        text.clone(),
                        tone,
                        p,
                        SharedString::from(format!("island-notice-{slot}-icon")),
                    )),
            )
            .children(action)
            .when_some(dismiss, |d, dismiss| {
                d.child(
                    self.icon_button(
                        SharedString::from(format!("dismiss-{slot}-notice")),
                        "Dismiss",
                        IconName::Close,
                        true,
                    )
                    .debug_selector(move || format!("dismiss-{slot}-notice"))
                    .size(px(24.))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.extras.dismissed_notices.push((slot, dismiss.clone()));
                        cx.notify();
                    })),
                )
            })
            .into_any_element()
    }

    pub(super) fn render_title_bar(
        &self,
        narrow: bool,
        enabled: bool,
        title: &str,
        session: Option<&Session>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        div()
            .id("title-bar")
            .debug_selector(|| "title-bar".into())
            .occlude()
            .max_w_full()
            .min_w_0()
            .h(px(40.))
            .pl_4()
            .pr_1()
            .flex()
            .items_center()
            .gap_2()
            .rounded_full()
            .bg(rgb(p.surface))
            .border_1()
            .border_color(rgb(p.border))
            .shadow(floating_shadow(p))
            .when_some(session, |d, session| {
                let (label, color) = if self.view.connected {
                    self.status_of(session, p)
                } else {
                    ("Offline", p.muted)
                };
                d.child(
                    div()
                        .id("title-status")
                        .flex_shrink_0()
                        .tooltip({
                            let label = SharedString::from(label.to_owned());
                            move |window, cx| Tooltip::new(label.clone()).build(window, cx)
                        })
                        // Static: the composer status line owns the animated
                        // loader and elapsed time, so the pill only signals state.
                        .child(status_dot(color)),
                )
                .when(!narrow, |d| {
                    d.child(
                        div()
                            .flex_shrink_0()
                            .max_w(px(140.))
                            .truncate()
                            .text_size(px(12.))
                            .text_color(rgb(p.muted))
                            .child(self.project_name(&session.cwd)),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(p.disabled))
                            .child("/"),
                    )
                })
            })
            .child(
                div()
                    .id("title-text")
                    .debug_selector(|| "title-text".into())
                    .min_w_0()
                    .max_w(px(if narrow { 220. } else { 360. }))
                    .truncate()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    // A tap shows the actions where there is no hover.
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.title_reveal.tap(std::time::Instant::now());
                        cx.notify();
                    }))
                    .child(title.to_owned()),
            )
            .when_some(session.filter(|_| !narrow), |d, session| {
                // The model chip is the model/effort control: it opens Change
                // model on the session's current model and effort.
                let model_enabled = enabled && self.supported_session();
                let effort = (!session.effort.is_empty())
                    .then(|| wks_native::launch::effort_label(&session.effort));
                // Only a mode that differs from asking is worth the room.
                let access = wks_native::launch::Permission::from_wire(
                    session.provider_id(),
                    &session.permission_mode,
                )
                .filter(|mode| *mode != wks_native::launch::Permission::Ask);
                d.child(
                    interactive_control(div().id("title-model"), p, model_enabled)
                        .debug_selector(|| "title-model".into())
                        .flex_shrink_0()
                        .max_w(px(240.))
                        .pl(px(6.))
                        .pr_2()
                        .py(px(3.))
                        .rounded_full()
                        .bg(rgb(p.selected))
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(model_badge(session, p, 12.))
                        .when_some(effort, |d, effort| {
                            d.child(
                                div()
                                    .flex_shrink_0()
                                    .text_color(rgb(p.muted))
                                    .child(format!("· {effort}")),
                            )
                        })
                        .when_some(access, |d, access| {
                            d.child(
                                div()
                                    .debug_selector(|| "title-access".into())
                                    .flex_shrink_0()
                                    .text_color(rgb(
                                        if access == wks_native::launch::Permission::FullAccess {
                                            p.warning
                                        } else {
                                            p.muted
                                        },
                                    ))
                                    .child(format!("· {}", access.label())),
                            )
                        })
                        .when(model_enabled, |d| {
                            d.child(
                                Icon::new(IconName::ChevronDown)
                                    .size(px(10.))
                                    .text_color(rgb(p.muted)),
                            )
                            .hover(|s| s.bg(rgb(p.border)))
                            .tooltip(|window, cx| {
                                Tooltip::new("Change model, effort or access").build(window, cx)
                            })
                            .on_click(cx.listener(
                                |this, _, window, cx| this.open_feature(Screen::Model, window, cx),
                            ))
                        }),
                )
            }) // Background work is status, not a hidden action: the chip
            // stays in the capsule, narrow windows included.
            .children(session.and_then(|session| self.render_tasks_chip(session, cx)))
    }

    pub(super) fn chat_actions(&self, enabled: bool, cx: &mut Context<Self>) -> Div {
        let selected = self.view.selected.is_some();
        let model_enabled = enabled && self.supported_session();
        div()
            .flex()
            .items_center()
            .gap(px(2.))
            .flex_shrink_0()
            .child(
                self.icon_button("open-changes", "Changes", IconName::Replace, selected)
                    .debug_selector(|| "open-changes".into())
                    .when(selected, |d| {
                        d.on_click(cx.listener(|this, event, window, cx| {
                            if !this.title_action_allowed(event) {
                                return;
                            }
                            this.open_feature(Screen::Changes, window, cx)
                        }))
                    }),
            )
            .child(
                self.icon_button(
                    "open-editor",
                    "Files and editor (Ctrl+Shift+E)",
                    IconName::FolderOpen,
                    selected,
                )
                .debug_selector(|| "open-editor".into())
                .when(selected, |d| {
                    d.on_click(cx.listener(|this, event, window, cx| {
                        if this.title_action_allowed(event) {
                            this.open_selected_editor(window, cx)
                        }
                    }))
                }),
            )
            .child(
                self.icon_button(
                    "open-terminal",
                    if self.terminal.open {
                        "Hide terminal (Ctrl+`)"
                    } else {
                        "Terminal in this agent’s folder (Ctrl+`)"
                    },
                    IconName::SquareTerminal,
                    selected,
                )
                .debug_selector(|| "open-terminal".into())
                .when(self.terminal.open, |d| {
                    d.text_color(rgb(self.appearance.palette().accent))
                })
                .when(selected, |d| {
                    d.on_click(cx.listener(|this, event, window, cx| {
                        if this.title_action_allowed(event) {
                            this.toggle_terminal(window, cx)
                        }
                    }))
                }),
            )
            .child(
                self.icon_button(
                    "open-history",
                    "Conversation history",
                    IconName::BookOpen,
                    selected,
                )
                .debug_selector(|| "open-history".into())
                .when(selected, |d| {
                    d.on_click(cx.listener(|this, event, window, cx| {
                        if !this.title_action_allowed(event) {
                            return;
                        }
                        this.open_feature(Screen::History, window, cx)
                    }))
                }),
            )
            .child(
                self.icon_button("open-session", "Session details", IconName::Info, selected)
                    .debug_selector(|| "open-session".into())
                    .when(selected, |d| {
                        d.on_click(cx.listener(|this, event, window, cx| {
                            if !this.title_action_allowed(event) {
                                return;
                            }
                            this.open_feature(Screen::Session, window, cx)
                        }))
                    }),
            )
            .child(
                self.icon_button(
                    "open-model",
                    "Change model or effort",
                    IconName::Settings2,
                    model_enabled,
                )
                .debug_selector(|| "open-model".into())
                .when(model_enabled, |d| {
                    d.on_click(cx.listener(|this, event, window, cx| {
                        if !this.title_action_allowed(event) {
                            return;
                        }
                        this.open_feature(Screen::Model, window, cx)
                    }))
                }),
            )
            .child(self.handoff_action(cx))
    }

    /// "Continue with…": the other provider's successor (see `handoff.rs`).
    /// Shown for Claude and Codex sessions; a Fleet Manager's page explains
    /// why ordinary handoff is not offered.
    fn handoff_action(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let label = match self.selected_session().map(|s| s.provider.as_str()) {
            Some("codex") => "Continue with Claude…",
            Some("claude" | "") => "Continue with Codex…",
            _ => "Continue with another agent…",
        };
        let enabled = self.supported_session() && self.view.connected && !self.demo;
        self.icon_button("open-handoff", label, IconName::ArrowRight, enabled)
            .debug_selector(|| "open-handoff".into())
            .when(enabled, |d| {
                d.on_click(cx.listener(|this, event, window, cx| {
                    if !this.title_action_allowed(event) {
                        return;
                    }
                    this.open_feature(Screen::Handoff, window, cx)
                }))
            })
    }

    pub(super) fn quiet_button(
        &self,
        id: impl Into<SharedString>,
        label: &'static str,
        icon: IconName,
        enabled: bool,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        interactive_control(div().id(id.into()), p, enabled)
            .px_2()
            .py_2()
            .rounded(px(p.control_radius))
            .flex()
            .items_center()
            .gap_2()
            .flex_shrink_0()
            .text_size(px(12.))
            .text_color(rgb(if enabled { p.muted } else { p.disabled }))
            .when(enabled, |d| {
                d.cursor_pointer()
                    .hover_text_style(|s| s.bg(rgb(p.selected)).text_color(rgb(p.text)))
                    .active_text_style(|s| s.bg(rgb(p.border)).text_color(rgb(p.text)))
            })
            .child(Icon::new(icon).size(px(12.)))
            .child(label)
    }

    pub(super) fn icon_button(
        &self,
        id: impl Into<SharedString>,
        label: &'static str,
        icon: IconName,
        enabled: bool,
    ) -> Stateful<Div> {
        self.icon_button_style(id, label, icon, enabled, false)
    }

    pub(super) fn primary_icon_button(
        &self,
        id: impl Into<SharedString>,
        label: &'static str,
        icon: IconName,
        enabled: bool,
    ) -> Stateful<Div> {
        self.icon_button_style(id, label, icon, enabled, true)
    }

    fn icon_button_style(
        &self,
        id: impl Into<SharedString>,
        label: &'static str,
        icon: IconName,
        enabled: bool,
        primary: bool,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        interactive_control(div().id(id.into()), p, enabled)
            .size(px(28.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .text_color(rgb(if enabled { p.muted } else { p.disabled }))
            .when(primary, |d| {
                d.bg(rgb(if enabled { p.primary } else { p.selected }))
                    .text_color(rgb(if enabled { p.on_primary } else { p.disabled }))
            })
            .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
            .when(enabled, |d| {
                d.cursor_pointer()
                    .hover_text_style(|s| {
                        s.bg(rgb(if primary { p.primary_hover } else { p.selected }))
                            .text_color(rgb(if primary { p.on_primary } else { p.text }))
                    })
                    .active_text_style(|s| {
                        s.bg(rgb(if primary { p.primary_pressed } else { p.border }))
                            .text_color(rgb(if primary { p.on_primary } else { p.text }))
                    })
            })
            .child(Icon::new(icon).size(px(13.)))
    }

    /// A text button for actions that stop or remove something: quiet at
    /// rest, error-toned, never mistaken for the page's primary action.
    pub(super) fn danger_button(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        enabled: bool,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        let tint = |alpha: f32| gpui::Hsla::from(rgb(p.error)).opacity(alpha);
        interactive_control(div().id(id.into()), p, enabled)
            .px_3()
            .py_2()
            .font_weight(FontWeight::MEDIUM)
            .rounded(px(p.control_radius))
            .text_size(px(scale::META))
            .text_color(rgb(if enabled { p.error } else { p.disabled }))
            .when(enabled, |d| {
                d.hover_text_style(move |s| s.bg(tint(0.12)).text_color(rgb(p.error)))
                    .active_text_style(move |s| s.bg(tint(0.2)).text_color(rgb(p.error)))
            })
            .child(label.into())
    }

    /// Scrolling page body for every non-chat screen. Where the app draws its
    /// own caption the top strip is window chrome: it drags the window and
    /// keeps page actions clear of the minimize / maximize / close buttons.
    pub(super) fn page_view(
        &self,
        id: &'static str,
        max_width: f32,
        short: bool,
        content: impl IntoElement,
    ) -> Div {
        self.page_frame(
            Self::page_scroller(id, short)
                .child(div().w_full().max_w(px(max_width)).mx_auto().child(content)),
        )
    }

    /// [`Self::page_view`] whose items are the scroll container's own
    /// children, so `scroll` can bring one into view by its index.
    pub(super) fn page_list(
        &self,
        id: &'static str,
        max_width: f32,
        short: bool,
        scroll: &gpui::ScrollHandle,
        items: Vec<gpui::AnyElement>,
    ) -> Div {
        self.page_frame(
            Self::page_scroller(id, short)
                .track_scroll(scroll)
                .flex()
                .flex_col()
                .children(
                    items
                        .into_iter()
                        .map(|item| div().w_full().max_w(px(max_width)).mx_auto().child(item)),
                ),
        )
    }

    fn page_scroller(id: &'static str, short: bool) -> Stateful<Div> {
        div()
            .id(id)
            .size_full()
            .overflow_y_scroll()
            .px(px(if short { 16. } else { 24. }))
            .pt(px(if custom_caption() {
                PAGE_CAPTION_INSET
            } else if short {
                12.
            } else {
                24.
            }))
            .pb(px(if short { 16. } else { 32. }))
    }

    fn page_frame(&self, scroller: Stateful<Div>) -> Div {
        let p = self.appearance.palette();
        div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(rgb(p.chat))
            .child(scroller)
            .children(page_drag_strip())
    }

    /// The one page header: optional back action, overline, title and a
    /// wrapped description, with trailing actions on the right.
    pub(super) fn page_header(
        &self,
        back: Option<Stateful<Div>>,
        overline_text: Option<&'static str>,
        title: impl Into<SharedString>,
        description: Option<SharedString>,
        trailing: Option<AnyElement>,
        short: bool,
    ) -> Div {
        let p = self.appearance.palette();
        div()
            .debug_selector(|| "page-header".into())
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .children(back.map(|back| div().flex().child(back.ml(px(-10.)))))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(overline_text.map(|text| overline(text, p)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_4()
                            .child(
                                div()
                                    .debug_selector(|| "page-title".into())
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(if short {
                                        scale::TITLE_SHORT
                                    } else {
                                        scale::TITLE
                                    }))
                                    .font_weight(FontWeight::BOLD)
                                    .child(title.into()),
                            )
                            .children(trailing.map(|t| {
                                div()
                                    .debug_selector(|| "page-actions".into())
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(t)
                            })),
                    )
                    .children(description.map(|text| {
                        div()
                            .text_size(px(scale::META))
                            .line_height(gpui::relative(1.45))
                            .text_color(rgb(p.muted))
                            .child(text)
                    })),
            )
    }
}

/// The window-chrome strip across the top of a page, when the app draws its
/// own caption: drags the window, stops short of the caption buttons.
pub(super) fn page_drag_strip() -> Option<Div> {
    custom_caption().then(|| {
        drag_region(div())
            .debug_selector(|| "page-drag-region".into())
            .absolute()
            .top_0()
            .left_0()
            .right(px(CAPTION_WIDTH))
            .h(px(PAGE_CAPTION_INSET - 8.))
    })
}

/// The title capsule's corner radius (half its 40px height); the notice
/// island under it keeps the same curve.
pub(super) const ISLAND_RADIUS: f32 = 20.;
/// Shown while older retained messages are left out of the transcript.
const OMITTED_NOTICE: &str =
    "Showing recent messages. Open History to browse older retained messages.";

/// Top inset of pages under an app-drawn caption: the caption's height plus
/// breathing room, all of it drag surface except the caption buttons.
pub(super) const PAGE_CAPTION_INSET: f32 = CAPTION_HEIGHT + 8.;

/// Windows draws no system title bar; the app owns the caption buttons and the
/// drag regions. `WKS_NATIVE_CAPTION=1` previews the same chrome elsewhere.
pub(crate) fn custom_caption() -> bool {
    #[cfg(feature = "ui-tests")]
    if FORCE_CAPTION.get() {
        return true;
    }
    cfg!(target_os = "windows") || std::env::var_os("WKS_NATIVE_CAPTION").is_some()
}

// Per thread, like the test zoom: concurrent GPUI tests must not see another
// test's caption preview.
#[cfg(feature = "ui-tests")]
thread_local! {
    pub(super) static FORCE_CAPTION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Width reserved at the top-right for the caption buttons.
pub(super) const CAPTION_WIDTH: f32 = 46. * 3.;
/// Height of the caption buttons' strip.
pub(super) const CAPTION_HEIGHT: f32 = 32.;

impl Workspace {
    /// Minimize / maximize / close at the window's top-right, painted over
    /// everything. On Windows the OS handles them through hit-test areas, which
    /// keeps native behavior (snap layouts on maximize, close confirmation via
    /// `on_window_should_close`); elsewhere they act on click.
    pub(super) fn render_caption(&self, window: &Window) -> Option<impl IntoElement> {
        if !custom_caption() {
            return None;
        }
        let p = self.appearance.palette();
        let native = cfg!(target_os = "windows");
        let maximized = window.is_maximized();
        let button = |id: &'static str, icon: IconName, area: gpui::WindowControlArea| {
            let close = matches!(area, gpui::WindowControlArea::Close);
            div()
                .id(id)
                .debug_selector(move || format!("caption-{id}"))
                .w(px(46.))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(p.muted))
                .when(close, |d| d.rounded_tr(px(p.panel_radius)))
                .hover(move |s| {
                    if close {
                        s.bg(rgb(0xc42b1c)).text_color(rgb(0xffffff))
                    } else {
                        s.bg(rgb(p.selected)).text_color(rgb(p.text))
                    }
                })
                .when(native, |d| d.window_control_area(area))
                .when(!native, |d| {
                    d.on_click(move |_, window, _| match area {
                        gpui::WindowControlArea::Min => window.minimize_window(),
                        gpui::WindowControlArea::Max => window.zoom_window(),
                        _ => window.remove_window(),
                    })
                })
                .child(Icon::new(icon).size(px(14.)))
        };
        Some(
            gpui::deferred(
                div()
                    .id("window-caption")
                    .debug_selector(|| "window-caption".into())
                    .absolute()
                    .top_0()
                    .right_0()
                    .h(px(CAPTION_HEIGHT))
                    .flex()
                    .occlude()
                    .child(button(
                        "minimize",
                        IconName::WindowMinimize,
                        gpui::WindowControlArea::Min,
                    ))
                    .child(button(
                        "maximize",
                        if maximized {
                            IconName::WindowRestore
                        } else {
                            IconName::WindowMaximize
                        },
                        gpui::WindowControlArea::Max,
                    ))
                    .child(button(
                        "close",
                        IconName::WindowClose,
                        gpui::WindowControlArea::Close,
                    )),
            )
            .with_priority(1),
        )
    }
}

/// Marks an otherwise-empty area as the window's title bar (drag to move,
/// double-click to maximize) when the app draws its own caption.
pub(super) fn drag_region<E: InteractiveElement>(element: E) -> E {
    if custom_caption() {
        // GPUI's focusable shell prevents default on mouse-down. Windows sends
        // WM_NCLBUTTONDOWN through those listeners before DefWindowProc starts
        // the move, so the drag surface must exclude the shell's hitbox too.
        // BlockMouse also excludes underlying transcript selection; later
        // occluding title pills/buttons still exclude this drag hitbox.
        let element = element.occlude();
        let element = if cfg!(target_os = "windows") {
            element.window_control_area(gpui::WindowControlArea::Drag)
        } else {
            element
        };
        // Observe the real Div hitbox in the cross-platform harness. This
        // listener changes neither default handling nor event propagation.
        #[cfg(all(test, feature = "ui-tests"))]
        let element = element.on_mouse_down(gpui::MouseButton::Left, |_, _, _| {
            DRAG_HIT.set(true);
        });
        element
    } else {
        element
    }
}

#[cfg(all(test, feature = "ui-tests"))]
thread_local! {
    pub(super) static DRAG_HIT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
mod tests {
    use super::{Tone, notice_tone};

    #[test]
    fn notices_read_by_what_they_say() {
        assert_eq!(notice_tone("Name saved"), Tone::Success);
        assert_eq!(notice_tone("Pinned /work/api"), Tone::Success);
        assert_eq!(notice_tone("Saving project…"), Tone::Loading);
        assert_eq!(notice_tone("Attachment failed: too large"), Tone::Error);
        assert_eq!(
            notice_tone("Could not open the attachment picker."),
            Tone::Error
        );
        assert_eq!(
            notice_tone("This provider cannot be resumed by the native client yet."),
            Tone::Error
        );
        assert_eq!(
            notice_tone("Use a name of at most 200 characters."),
            Tone::Warning
        );
        // Failure wins over a trailing ellipsis.
        assert_eq!(notice_tone("Could not reconnect…"), Tone::Error);
        // Controller receipts for model/effort changes and new sessions.
        assert_eq!(
            notice_tone("Change queued; the provider will apply it when ready"),
            Tone::Info
        );
        assert_eq!(notice_tone("Model change accepted: opus"), Tone::Success);
        assert_eq!(notice_tone("Session created"), Tone::Success);
        assert_eq!(
            notice_tone(
                "Session created. Initial message delivery was not confirmed; check the conversation before sending the retained draft."
            ),
            Tone::Warning
        );
        assert_eq!(
            notice_tone("Model change refused: unsupported"),
            Tone::Error
        );
    }
}
