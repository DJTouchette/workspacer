//! Design v2 for the conversation's bottom dock: the composer, the approval
//! strip attached to it, and the plan and limit decision cards above it.
//! Only drawn when `Workspace::design()` is v2; v1 keeps `conversation.rs`.
//!
//! Rules (`docs/design-v2-direction.md` §4):
//! - One primary per region. The composer's Send/Steer is the primary until
//!   a decision is open; then the decision owns the primary and Send drops
//!   to secondary.
//! - The send button says what it does: "Send" when the agent is idle,
//!   "Steer" while it works. Queue and Send now live in its split menu, each
//!   with a one-line explanation. Shortcuts live in tooltips, not a hint row.
//! - A pending tool approval is a dense strip joined to the composer, with
//!   numbered actions (number keys pick them) and Details as a ghost.
use super::conversation::ChatLayout;
use super::primitives::{self, ButtonSize, ButtonVariant, CardHead, StatusBadge, StatusKind};
use super::*;
use gpui::{AnyElement, relative};
use serde_json::Value;

/// The v2 composer's own state.
#[derive(Default)]
pub(super) struct ComposerUi {
    /// The send button's split menu (Queue / Send now) is open.
    pub send_menu: bool,
}

/// A decision the selected session is waiting on, in dock order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Decision {
    /// A tool permission prompt (the strip on the composer).
    Approval,
    /// A finished plan waiting for approval (`ExitPlanMode`).
    Plan,
    /// The account is out of a usage window.
    Limit,
    /// A question set (`AskUserQuestion` or an MCP elicitation).
    Questions,
}

/// The plan an `ExitPlanMode` approval carries, when it has one.
pub(super) fn plan_text(approval: &Value) -> Option<&str> {
    approval
        .get("toolName")
        .and_then(Value::as_str)
        .filter(|tool| *tool == "ExitPlanMode")
        .and(approval.pointer("/toolInput/plan"))
        .and_then(Value::as_str)
        .filter(|plan| !plan.trim().is_empty())
}

/// The tool an approval asks for ("Bash"), and the one line it would run or
/// touch ("cargo test"), both empty-safe.
pub(super) fn approval_summary(approval: &Value) -> (String, String) {
    let tool = approval
        .get("toolName")
        .or_else(|| approval.get("tool"))
        .and_then(Value::as_str)
        .unwrap_or("Tool")
        .to_owned();
    let summary = approval
        .pointer("/toolInput/command")
        .or_else(|| approval.pointer("/toolInput/file_path"))
        .or_else(|| approval.pointer("/toolInput/path"))
        .or_else(|| approval.pointer("/toolInput/url"))
        .or_else(|| approval.pointer("/toolInput/pattern"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_owned();
    (tool, summary)
}

/// What Details shows for an approval, as (label, value, monospace) rows:
/// the request itself, then where and for whom it would run, then why. No
/// raw JSON: remaining scalar inputs become their own rows.
pub(super) fn approval_facts(
    approval: &Value,
    cwd: &str,
    host: &str,
    agent: &str,
) -> Vec<(String, String, bool)> {
    let input = approval.get("toolInput").unwrap_or(&Value::Null);
    let mut rows = Vec::new();
    let named = [
        ("command", "Command"),
        ("file_path", "File"),
        ("path", "Path"),
        ("url", "URL"),
        ("pattern", "Pattern"),
    ];
    for (key, label) in named {
        if let Some(value) = input
            .get(key)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
        {
            rows.push((label.to_owned(), value.trim_end().to_owned(), true));
        }
    }
    if !cwd.is_empty() {
        rows.push(("Folder".into(), cwd.to_owned(), true));
    }
    if !host.is_empty() {
        rows.push(("Host".into(), host.to_owned(), false));
    }
    if !agent.is_empty() {
        rows.push(("Agent".into(), agent.to_owned(), false));
    }
    let reason = input
        .get("description")
        .or_else(|| approval.get("reason"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|r| !r.is_empty());
    if let Some(reason) = reason {
        rows.push(("Reason".into(), reason.to_owned(), false));
    }
    // Anything else the tool was given, as a short line each.
    if let Some(object) = input.as_object() {
        for (key, value) in object {
            if named.iter().any(|(k, _)| k == key) || key == "description" {
                continue;
            }
            let text = match value {
                Value::String(s) => s.clone(),
                Value::Number(_) | Value::Bool(_) => value.to_string(),
                Value::Null => continue,
                other => other.to_string(),
            };
            let text = wks_native::format::truncate_end(&text, 160).unwrap_or(text);
            rows.push((sentence_case(key), text, true));
        }
    }
    rows
}

/// `old_string` / `oldString` → "Old string".
fn sentence_case(key: &str) -> String {
    let mut words = String::new();
    for (ix, ch) in key.chars().enumerate() {
        if ch == '_' || ch == '-' {
            words.push(' ');
        } else if ch.is_uppercase() && ix > 0 {
            words.push(' ');
            words.extend(ch.to_lowercase());
        } else if ix == 0 {
            words.extend(ch.to_uppercase());
        } else {
            words.push(ch);
        }
    }
    words
}

/// What the composer says it is waiting on while a decision is open.
pub(super) fn waiting_label(decision: Decision) -> &'static str {
    match decision {
        Decision::Approval => "Waiting for your approval",
        Decision::Plan => "Waiting for you to approve the plan",
        Decision::Limit => "Waiting for the usage limit to reset",
        Decision::Questions => "Waiting for your answers",
    }
}

/// The send button's label and tooltip: Steer while the agent works.
pub(super) fn send_copy(steering: bool) -> (&'static str, &'static str) {
    if steering {
        (
            "Steer",
            "Steer: the agent reads this at its next step, without stopping its work",
        )
    } else {
        ("Send", "Send this message to the agent")
    }
}

/// "1 of 2" when more than one decision is open.
fn position_label(decisions: &[Decision], decision: Decision) -> Option<String> {
    let ix = decisions.iter().position(|d| *d == decision)?;
    (decisions.len() > 1).then(|| format!("{} of {}", ix + 1, decisions.len()))
}

/// Number keys on the approval strip, in button order: Allow, each "Always
/// allow" grant, Deny, then Deny with note where it is offered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum StripKey {
    Allow,
    Always(usize),
    Deny,
    DenyWithNote,
}

pub(super) fn strip_keys(always: usize, note: bool) -> Vec<StripKey> {
    let mut keys = vec![StripKey::Allow];
    keys.extend((0..always).map(StripKey::Always));
    keys.push(StripKey::Deny);
    if note {
        keys.push(StripKey::DenyWithNote);
    }
    keys
}

impl Workspace {
    /// The decisions the session waits on, in the order the dock shows them
    /// (the approval first: it sits on the composer).
    pub(super) fn decisions(&self, session: &Session) -> Vec<Decision> {
        let mut out = Vec::new();
        if let Some(approval) = &session.approval {
            out.push(if plan_text(approval).is_some() {
                Decision::Plan
            } else {
                Decision::Approval
            });
        }
        if let Some((window, resets_at)) = session.limits.exhausted()
            && !session.working()
            && self.extras.limit_dismissed.as_ref()
                != Some(&(session.id.clone(), window, resets_at))
        {
            out.push(Decision::Limit);
        }
        if session.questions.is_some() {
            out.push(Decision::Questions);
        }
        out
    }

    /// Whether Deny with note is offered: Claude reads a deny's note, Codex
    /// gets it as a user message; other providers have no field for it.
    fn deny_note_offered(&self) -> bool {
        self.selected_session()
            .is_some_and(|s| matches!(s.provider_id(), "claude" | "codex"))
    }

    fn host_label(&self) -> &'static str {
        if self.extras.local_paths {
            "This machine"
        } else {
            "Workspace hub"
        }
    }

    /// The v2 composer: one surface holding the approval strip (when one is
    /// pending), draft attachments, the slash menu, the input and a toolbar
    /// of chips, status and the send control.
    pub(super) fn render_composer_v2(
        &mut self,
        layout: ChatLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let ChatLayout {
            narrow,
            compact,
            enabled,
            working,
        } = layout;
        let uploading = self.uploading();
        let can_send = enabled && !uploading;
        let selected = self.selected_session().cloned();
        let draft_cwd = selected.as_ref().map(|s| s.cwd.clone()).unwrap_or_default();
        let draft_files = self.draft_files(cx);
        let viewport = window.viewport_size().height;
        let attachments_share = if unzoom(viewport) < 300. { 0.05 } else { 0.10 };
        let focused = self.composer.read(cx).focus_handle(cx).is_focused(window);
        let steering = self.composer_busy();
        let decisions = selected
            .as_ref()
            .map(|s| self.decisions(s))
            .unwrap_or_default();
        let deciding = !decisions.is_empty();
        let slash_rows = self.slash_menu(cx);
        let strip = selected
            .as_ref()
            .and_then(|s| s.approval.as_ref())
            .filter(|a| plan_text(a).is_none())
            .map(|approval| self.render_approval_strip(approval, layout, true, &decisions, cx));
        let edge = if focused {
            p.roles.focus
        } else {
            p.roles.border_strong
        };
        let pad = if compact { t.space.md } else { t.space.lg };

        // Toolbar, left: attach, then the session's context as quiet chips.
        let attach = primitives::icon_button(
            "attach-file",
            IconName::Plus,
            if uploading {
                "Attaching file…"
            } else {
                "Attach a file (or paste an image)"
            },
            ButtonVariant::Ghost,
            can_send,
            &p,
        )
        .debug_selector(|| "attach-file".into())
        .when(can_send, |d| {
            d.on_click(cx.listener(|this, _, window, cx| this.pick_attachment(window, cx)))
        });
        let folder = selected
            .as_ref()
            .map(|s| s.cwd.clone())
            .filter(|cwd| !cwd.is_empty() && !narrow)
            .map(|cwd| {
                let name = std::path::Path::new(&cwd)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| cwd.clone());
                primitives::toolbar_chip(
                    "composer-folder",
                    Some(
                        Icon::new(IconName::Folder)
                            .size(px(t.icon.xs))
                            .into_any_element(),
                    ),
                    name,
                    false,
                    true,
                    &p,
                )
                .debug_selector(|| "composer-folder".into())
                .tooltip(primitives::tooltip_view(cwd, None, &p))
            });
        let model = selected.as_ref().map(|s| {
            let supported = self.supported_session() && self.view.connected;
            let name = {
                let model = s.display_model();
                if model.is_empty() {
                    match s.provider_id() {
                        "codex" => "Codex".to_owned(),
                        _ => "Claude".to_owned(),
                    }
                } else {
                    model
                }
            };
            primitives::toolbar_chip(
                "composer-model",
                Some(chrome::provider_glyph(&s.provider, t.icon.xs, p)),
                name,
                supported,
                true,
                &p,
            )
            .debug_selector(|| "composer-model".into())
            .tooltip(primitives::tooltip_view(
                "Model, effort and access",
                Some("g m"),
                &p,
            ))
            .when(supported, |d| {
                d.on_click(
                    cx.listener(|this, _, window, cx| this.open_feature(Screen::Model, window, cx)),
                )
            })
        });
        let meter = selected
            .as_ref()
            .filter(|_| !self.title_carries_context())
            .and_then(|s| gauge::context_meter(s, p));

        // Toolbar, right: what the session is doing, Stop, and the send control.
        let status = self.composer_status_v2(layout, &decisions, narrow);
        let stop = working.then(|| {
            chrome::interactive_control(div().id("stop"), p, enabled)
                .debug_selector(|| "composer-stop".into())
                .size(px(t.layout.icon_button))
                .flex()
                .items_center()
                .justify_center()
                .flex_shrink_0()
                .rounded(px(t.radius.control))
                .bg(primitives::tonal(&p, t.wash.rest))
                .tooltip(primitives::tooltip_view("Stop the agent", None, &p))
                .when(enabled, |d| {
                    d.hover(move |s| s.bg(primitives::tonal(&p, t.wash.hover)))
                        .on_click(cx.listener(|this, _, _, cx| this.act(Action::Stop, cx)))
                })
                .child(
                    div()
                        .size(px(t.icon.xs - t.space.xxs))
                        .rounded(px(t.space.xxs))
                        .bg(rgb(if enabled { p.text } else { p.disabled })),
                )
        });
        let send = self.render_send_control(steering, deciding, can_send, cx);

        let toolbar = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(t.space.md))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .gap(px(t.space.xxs))
                    .child(attach)
                    .children(folder)
                    .children(model)
                    .children(meter),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(t.space.md))
                    .children(status)
                    .children(stop)
                    .child(send),
            );

        let body = div()
            .flex()
            .flex_col()
            .gap(px(t.space.sm))
            .px(px(pad))
            .pt(px(pad))
            .pb(px(t.space.md))
            .when(!draft_files.is_empty() || uploading, |d| {
                d.child(
                    div()
                        .id("composer-attachments-scroll")
                        .debug_selector(|| "composer-attachments-scroll".into())
                        .max_h((viewport * attachments_share).min(px(t.layout.row_height * 3.)))
                        .overflow_y_scroll()
                        .child(self.render_draft_attachments(draft_files, &draft_cwd, cx)),
                )
            })
            .when(!slash_rows.is_empty(), |d| {
                d.child(self.render_slash_menu(slash_rows, cx))
            })
            .child(
                div().px(px(t.space.xs)).child(
                    Input::new(&self.composer)
                        .appearance(false)
                        .disabled(self.view.busy),
                ),
            )
            .child(toolbar);

        let composer = div()
            .id("floating-composer")
            .debug_selector(|| "chat-composer".into())
            .key_context(
                match (self.settings.enter_sends, self.slash_menu(cx).is_empty()) {
                    (true, true) => "Composer ComposerEnter",
                    (false, true) => "Composer",
                    (true, false) => "Composer ComposerEnter SlashMenu",
                    (false, false) => "Composer SlashMenu",
                },
            )
            .occlude()
            .relative()
            .flex()
            .flex_col()
            .bg(rgb(p.surface))
            .border(px(t.stroke.hairline))
            .border_color(rgb(edge))
            .rounded(px(t.radius.composer))
            .shadow(primitives::elevation(&p, t.elevation.floating))
            .children(strip)
            .child(body);
        let menu =
            (self.composer_ui.send_menu && steering).then(|| self.render_send_menu(can_send, cx));
        div()
            .debug_selector(|| "chat-composer-group".into())
            .w_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap(px(t.space.md))
            .children(self.render_cold_note(cx))
            .child(div().relative().child(composer).children(menu))
    }

    /// The live status in the composer's toolbar: a decision it waits on
    /// (Needs you), the agent working (with its clock), or a connection
    /// state. A settled turn's summary shows as quiet text; Ready shows
    /// nothing (idle is the default).
    fn composer_status_v2(
        &self,
        layout: ChatLayout,
        decisions: &[Decision],
        narrow: bool,
    ) -> Option<Div> {
        let p = self.palette();
        let t = p.tokens;
        let activity = self.composer_activity(layout);
        let badge = |kind: StatusKind, label: String| {
            let badge = if narrow {
                primitives::status_indicator(kind, &p)
            } else {
                StatusBadge::new(kind).label(label).render(&p)
            };
            badge
                .debug_selector(|| "composer-status".into())
                .max_w(px(t.layout.menu_width))
        };
        let connected = self.view.connected;
        if connected
            && !self.view.loading
            && !self.view.busy
            && let Some(first) = decisions.first()
        {
            return Some(badge(StatusKind::NeedsYou, waiting_label(*first).into()));
        }
        if activity.live {
            let kind = if !connected {
                match self.connection_copy().label {
                    "Paused" => StatusKind::Paused,
                    "Unavailable" => StatusKind::Failed,
                    _ => StatusKind::Working,
                }
            } else {
                StatusKind::Working
            };
            let label = if activity.label == "Waiting…" {
                "Waiting for the agent to respond".to_owned()
            } else {
                activity.label
            };
            return Some(badge(kind, label));
        }
        if narrow || activity.label == "Ready" {
            return None;
        }
        Some(
            div()
                .debug_selector(|| "composer-status".into())
                .min_w_0()
                .max_w(px(t.layout.menu_width))
                .truncate()
                .text_size(px(t.text.meta))
                .text_color(rgb(p.roles.text_tertiary))
                .child(activity.label),
        )
    }

    /// Send (idle) or Steer (working) as the composer's one primary; while
    /// working it is a split button whose menu holds Queue and Send now.
    /// While a decision is open it demotes to secondary.
    fn render_send_control(
        &self,
        steering: bool,
        deciding: bool,
        can_send: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let variant = if deciding {
            ButtonVariant::Secondary
        } else {
            ButtonVariant::Primary
        };
        let (label, tip) = send_copy(steering);
        let label = if self.view.busy { "Sending…" } else { label };
        let key = self.send_key();
        if !steering {
            return primitives::button(
                "send",
                label,
                Some(IconName::ArrowUp),
                variant,
                ButtonSize::Small,
                can_send,
                &p,
            )
            .debug_selector(|| "composer-send".into())
            .tooltip(primitives::tooltip_view(tip, Some(key), &p))
            .when(can_send, |d| {
                d.on_click(cx.listener(|this, _, window, cx| this.send(&SendMessage, window, cx)))
            })
            .into_any_element();
        }
        let (main, chevron) = primitives::split_button_parts(
            "send",
            label,
            Some(IconName::ArrowUp),
            variant,
            ButtonSize::Small,
            can_send,
            &p,
        );
        let main = main
            .debug_selector(|| "composer-send".into())
            .tooltip(primitives::tooltip_view(tip, Some(key), &p))
            .when(can_send, |d| {
                d.on_click(cx.listener(|this, _, window, cx| this.send(&SendMessage, window, cx)))
            });
        let chevron = chevron
            .debug_selector(|| "composer-send-menu".into())
            .when(can_send, |d| {
                d.on_click(cx.listener(|this, _, _, cx| {
                    this.composer_ui.send_menu = !this.composer_ui.send_menu;
                    cx.notify();
                }))
            });
        primitives::split_button(main, chevron, &p).into_any_element()
    }

    /// The keys that send, as tooltip keycaps.
    fn send_key(&self) -> &'static str {
        match (self.settings.enter_sends, cfg!(target_os = "macos")) {
            (true, _) => "Enter",
            (false, true) => "⌘ Enter",
            (false, false) => "Ctrl Enter",
        }
    }

    fn send_now_key(&self) -> &'static str {
        match (self.settings.enter_sends, cfg!(target_os = "macos")) {
            (true, true) => "⌘ Enter",
            (true, false) => "Ctrl Enter",
            (false, true) => "⌘ ⇧ Enter",
            (false, false) => "Ctrl Shift Enter",
        }
    }

    /// The split menu over the send button: the two other ways to deliver a
    /// draft mid-turn, each with a line saying what it does.
    fn render_send_menu(&self, can_send: bool, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let item = |id: &'static str,
                    icon: IconName,
                    title: &'static str,
                    body: &'static str,
                    key: Option<&'static str>| {
            primitives::list_row(id, false, can_send, &p)
                .debug_selector(move || id.into())
                .items_start()
                .py(px(t.space.sm))
                .child(
                    Icon::new(icon)
                        .size(px(t.icon.md))
                        .text_color(rgb(p.roles.text_secondary)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(t.space.xxs))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(t.space.md))
                                .child(
                                    div()
                                        .flex_1()
                                        .font_weight(FontWeight(t.weight.medium))
                                        .child(title),
                                )
                                .when_some(key, |d, key| {
                                    d.child(
                                        div().flex().gap(px(t.space.xxs)).children(
                                            key.split_whitespace()
                                                .map(|k| primitives::kbd(k.to_owned(), &p)),
                                        ),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .text_size(px(t.text.meta))
                                .line_height(relative(t.text.line_height))
                                .text_color(rgb(p.roles.text_secondary))
                                .child(body),
                        ),
                )
        };
        div()
            .absolute()
            .right_0()
            .bottom(relative(1.))
            .pb(px(t.space.sm))
            .child(
                primitives::menu(&p)
                    .id("composer-send-menu-list")
                    .debug_selector(|| "composer-send-menu-list".into())
                    .w(px(t.layout.dialog_width - t.space.huge * 2.))
                    .occlude()
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.composer_ui.send_menu = false;
                        cx.notify();
                    }))
                    .child(
                        item(
                            "composer-queue",
                            IconName::Inbox,
                            "Queue",
                            "Hold it until this turn ends, then send it as the next message.",
                            None,
                        )
                        .when(can_send, |d| {
                            d.on_click(cx.listener(|this, _, window, cx| {
                                this.composer_ui.send_menu = false;
                                this.submit(Submit::Queue, window, cx)
                            }))
                        }),
                    )
                    .child(
                        item(
                            "composer-send-now",
                            IconName::ArrowUp,
                            "Send now",
                            "Interrupt: the agent sets its current work aside and reads this at once.",
                            Some(self.send_now_key()),
                        )
                        .when(can_send, |d| {
                            d.on_click(cx.listener(|this, _, window, cx| {
                                this.composer_ui.send_menu = false;
                                this.submit(Submit::Now, window, cx)
                            }))
                        }),
                    ),
            )
    }

    /// A pending tool approval as one dense strip: what it wants to run,
    /// numbered actions (number keys pick them), Details as a ghost. Joined
    /// to the composer's top edge when `joined`, a card of its own otherwise
    /// (while a subagent's conversation replaces the composer).
    pub(super) fn render_approval_strip(
        &self,
        approval: &Value,
        layout: ChatLayout,
        joined: bool,
        decisions: &[Decision],
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let (compact, enabled) = (layout.compact, layout.enabled);
        let (tool, summary) = approval_summary(approval);
        let open = self.extras.approval_details;
        let card = self.selected_session().and_then(Session::card);
        let always = wks_native::session_controls::always_options(approval);
        let note = self.deny_note_offered() && !self.extras.deny_open;
        let keys = strip_keys(always.len(), note);
        let number = |key: &StripKey| {
            keys.iter()
                .position(|k| k == key)
                .map(|ix| (ix + 1).to_string())
                .unwrap_or_default()
        };
        let pad = if compact { t.space.md } else { t.space.lg };
        let header = div()
            .flex()
            .items_center()
            .gap(px(t.space.sm))
            .min_w_0()
            .child(primitives::status_indicator(StatusKind::NeedsYou, &p))
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(t.text.body))
                    .font_weight(FontWeight(t.weight.semibold))
                    .text_color(rgb(p.text))
                    .child(tool),
            )
            .when(!summary.is_empty(), |d| {
                d.child(
                    div()
                        .flex_shrink_0()
                        .text_color(rgb(p.roles.text_tertiary))
                        .child("·"),
                )
                .child(
                    primitives::truncated("approval-summary", summary.clone(), &p)
                        .debug_selector(|| "approval-summary".into())
                        .flex_1()
                        .font_family(mono_font())
                        .text_size(px(t.text.body))
                        .text_color(rgb(p.text)),
                )
            })
            .when(summary.is_empty(), |d| {
                d.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(t.text.body))
                        .text_color(rgb(p.roles.text_secondary))
                        .child("wants permission to run"),
                )
            })
            .children(position_label(decisions, Decision::Approval).map(|label| {
                div()
                    .debug_selector(|| "approval-position".into())
                    .flex_shrink_0()
                    .text_size(px(t.text.meta))
                    .text_color(rgb(p.roles.text_tertiary))
                    .child(label)
            }))
            .child(
                primitives::button(
                    if compact {
                        "approval-toggle-compact"
                    } else {
                        "approval-toggle"
                    },
                    if open { "Hide details" } else { "Details" },
                    None,
                    ButtonVariant::Ghost,
                    ButtonSize::Small,
                    true,
                    &p,
                )
                .debug_selector(|| "approval-toggle".into())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.extras.approval_details = !this.extras.approval_details;
                    cx.notify();
                })),
            );
        let details = open.then(|| {
            let session = self.selected_session();
            let cwd = session.map(|s| s.cwd.clone()).unwrap_or_default();
            let agent = session
                .map(|s| {
                    let model = s.display_model();
                    let title = self.session_title(s);
                    if model.is_empty() {
                        title
                    } else {
                        format!("{title} · {model}")
                    }
                })
                .unwrap_or_default();
            let facts = approval_facts(approval, &cwd, self.host_label(), &agent);
            div()
                .id("approval-details")
                .debug_selector(|| "approval-details".into())
                .max_h(px(if compact {
                    t.layout.row_height * 2.
                } else {
                    t.layout.row_height * 6.
                }))
                .overflow_y_scroll()
                .px(px(t.space.lg))
                .py(px(t.space.md))
                .rounded(px(t.radius.control))
                .bg(rgb(p.chat))
                .border(px(t.stroke.hairline))
                .border_color(rgb(p.roles.card_border))
                .flex()
                .flex_col()
                .gap(px(t.space.xs))
                .children(
                    facts
                        .into_iter()
                        .enumerate()
                        .map(|(ix, (label, value, mono))| {
                            div()
                                .debug_selector(move || format!("approval-fact-{ix}"))
                                .flex()
                                .items_start()
                                .gap(px(t.space.lg))
                                .text_size(px(t.text.meta))
                                .line_height(relative(t.text.line_height))
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .w(px(t.layout.avatar_lg * 2.))
                                        .text_color(rgb(p.roles.text_tertiary))
                                        .child(label),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_color(rgb(p.text))
                                        .when(mono, |d| d.font_family(mono_font()))
                                        .child(value),
                                )
                        }),
                )
        });
        let deny_note = self
            .extras
            .deny_open
            .then(|| self.render_deny_note_v2(enabled, "Deny and tell it", cx));
        let actions = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(t.space.sm))
            .child({
                let card = card.clone();
                primitives::keyed_button(
                    "approve",
                    number(&StripKey::Allow),
                    "Allow",
                    // While a note is being written, sending it is the primary.
                    if self.extras.deny_open {
                        ButtonVariant::Secondary
                    } else {
                        ButtonVariant::Primary
                    },
                    ButtonSize::Small,
                    enabled,
                    &p,
                )
                .debug_selector(|| "approve".into())
                .tooltip(primitives::tooltip_view("Allow this once", None, &p))
                .when(enabled, |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| {
                        this.act(Action::Approve(true, card.clone()), cx)
                    }))
                })
            })
            // One button per grant the card offers, in the hub's own words:
            // the user sees exactly what "always" would allow.
            .children(always.into_iter().enumerate().map(|(ix, option)| {
                let card = card.clone();
                let enabled = enabled && card.is_some();
                primitives::keyed_button(
                    SharedString::from(format!("always-option-{ix}")),
                    number(&StripKey::Always(ix)),
                    option.label.clone(),
                    ButtonVariant::Secondary,
                    ButtonSize::Small,
                    enabled,
                    &p,
                )
                .debug_selector(move || format!("always-option-{ix}"))
                .max_w(px(t.layout.dialog_width))
                .when(enabled, |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| {
                        let Some(card) = card.clone() else {
                            return;
                        };
                        this.act(
                            Action::AlwaysAllow {
                                option: option.id.clone(),
                                label: option.label.clone(),
                                card,
                            },
                            cx,
                        )
                    }))
                })
            }))
            .child({
                let card = card.clone();
                primitives::keyed_button(
                    "deny",
                    number(&StripKey::Deny),
                    "Deny",
                    ButtonVariant::Secondary,
                    ButtonSize::Small,
                    enabled,
                    &p,
                )
                .debug_selector(|| "deny".into())
                .when(enabled, |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| {
                        this.act(Action::Approve(false, card.clone()), cx)
                    }))
                })
            })
            .when(note, |d| {
                let card = card.clone();
                d.child(
                    primitives::keyed_button(
                        "deny-with-note",
                        number(&StripKey::DenyWithNote),
                        "Deny with note…",
                        ButtonVariant::Ghost,
                        ButtonSize::Small,
                        enabled,
                        &p,
                    )
                    .debug_selector(|| "deny-with-note".into())
                    .tooltip(primitives::tooltip_view(
                        "Deny, and tell the agent what to do instead",
                        None,
                        &p,
                    ))
                    .when(enabled, |d| {
                        d.on_click(cx.listener(move |this, _, window, cx| {
                            this.open_deny_note(card.clone(), window, cx)
                        }))
                    }),
                )
            });
        let strip = div()
            .debug_selector(|| "approval-card".into())
            .occlude()
            .w_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap(px(t.space.md))
            .px(px(pad))
            .py(px(if compact {
                t.space.sm
            } else {
                t.space.md + t.space.xxs
            }))
            .bg(primitives::wash(p.warning, t.wash.rest))
            .child(header)
            .children(details)
            .children(deny_note)
            .child(actions);
        if joined {
            // Inside the composer's 1px edge: its own top corners, and a
            // hairline where the draft begins.
            strip
                .rounded_t(px(t.radius.composer - t.stroke.hairline))
                .border_b(px(t.stroke.hairline))
                .border_color(rgb(p.roles.card_border))
        } else {
            div()
                .w_full()
                .flex_shrink_0()
                .rounded(px(t.radius.panel))
                .bg(rgb(p.surface))
                .border(px(t.stroke.hairline))
                .border_color(rgb(p.roles.card_border))
                .shadow(primitives::elevation(&p, t.elevation.floating))
                .child(strip.rounded(px(t.radius.panel - t.stroke.hairline)))
        }
    }

    /// Open the note box under a Deny with note / Keep planning.
    pub(super) fn open_deny_note(
        &mut self,
        card: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tools.deny_card = card;
        self.extras.deny_open = true;
        self.extras
            .deny_note
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    /// The note box: the field, the send, and a way back.
    fn render_deny_note_v2(
        &self,
        enabled: bool,
        send: &'static str,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        div()
            .flex()
            .items_center()
            .gap(px(t.space.sm))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .debug_selector(|| "deny-note".into())
                    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                        let k = &event.keystroke;
                        if k.key == "enter" && !k.modifiers.shift {
                            cx.stop_propagation();
                            this.send_deny_note(window, cx);
                        } else if k.key == "escape" {
                            cx.stop_propagation();
                            this.extras.deny_open = false;
                            cx.notify();
                        }
                    }))
                    .child(Input::new(&self.extras.deny_note)),
            )
            .child(
                primitives::button(
                    "deny-note-cancel",
                    "Cancel",
                    None,
                    ButtonVariant::Ghost,
                    ButtonSize::Small,
                    true,
                    &p,
                )
                .debug_selector(|| "deny-note-cancel".into())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.extras.deny_open = false;
                    cx.notify();
                })),
            )
            .child(
                primitives::button(
                    "deny-note-send",
                    send,
                    None,
                    ButtonVariant::Primary,
                    ButtonSize::Small,
                    enabled,
                    &p,
                )
                .debug_selector(|| "deny-note-send".into())
                .when(enabled, |d| {
                    d.on_click(cx.listener(|this, _, window, cx| this.send_deny_note(window, cx)))
                }),
            )
    }

    /// Number keys on a pending approval: 1 Allow, then each "Always" grant,
    /// Deny, Deny with note. They act only when no text field would take the
    /// digit: nothing is focused, or the composer is focused and empty.
    pub(super) fn decision_keys(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.design().is_v2()
            || self.screen != Screen::Conversation
            || self.new_session
            || self.view.child.is_some()
            || self.extras.deny_open
            || !self.view.connected
            || self.view.busy
        {
            return;
        }
        let k = &event.keystroke;
        if k.modifiers.modified() {
            return;
        }
        let Ok(n) = k.key.parse::<usize>() else {
            return;
        };
        let composer = self.composer.read(cx);
        let free = self.focus.is_focused(window)
            || (composer.focus_handle(cx).is_focused(window) && composer.value().is_empty());
        if !free {
            return;
        }
        let Some(session) = self.selected_session().cloned() else {
            return;
        };
        let Some(approval) = session.approval.as_ref().filter(|a| plan_text(a).is_none()) else {
            return;
        };
        let always = wks_native::session_controls::always_options(approval);
        let keys = strip_keys(always.len(), self.deny_note_offered());
        let Some(key) = n.checked_sub(1).and_then(|ix| keys.get(ix)) else {
            return;
        };
        cx.stop_propagation();
        let card = session.card();
        match key {
            StripKey::Allow => self.act(Action::Approve(true, card), cx),
            StripKey::Deny => self.act(Action::Approve(false, card), cx),
            StripKey::DenyWithNote => self.open_deny_note(card, window, cx),
            StripKey::Always(ix) => {
                if let (Some(option), Some(card)) = (always.get(*ix), card) {
                    self.act(
                        Action::AlwaysAllow {
                            option: option.id.clone(),
                            label: option.label.clone(),
                            card,
                        },
                        cx,
                    )
                }
            }
        }
    }

    /// Claude finished planning: the plan in a v2 card. With the account out
    /// of its usage window too, the primary approves and hands the work to
    /// the other agent in one step.
    pub(super) fn render_plan_card_v2(
        &self,
        plan: &str,
        layout: ChatLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let (compact, enabled) = (layout.compact, layout.enabled);
        let session = self.selected_session();
        let decisions = session.map(|s| self.decisions(s)).unwrap_or_default();
        let card = session.and_then(Session::card);
        let handoff_to = decisions
            .contains(&Decision::Limit)
            .then(|| self.other_agent())
            .flatten();
        let can_hand_off = self.supported_session() && self.view.connected && !self.demo;
        let deny_open = self.extras.deny_open;
        let agent = session
            .map(|s| match s.provider_id() {
                "codex" => "Codex",
                _ => "Claude",
            })
            .unwrap_or("the agent");
        let mut head = CardHead::new("Review the plan")
            .icon(IconName::BookOpen)
            .status(StatusBadge::new(StatusKind::NeedsYou).label("Plan ready"))
            .meta(format!(
                "Nothing runs until you approve it · {agent} will start on approval"
            ));
        if let Some(label) = position_label(&decisions, Decision::Plan) {
            head = head.action(
                div()
                    .debug_selector(|| "plan-position".into())
                    .text_size(px(t.text.meta))
                    .text_color(rgb(p.roles.text_tertiary))
                    .child(label),
            );
        }
        let approve = |id: &'static str,
                       label: SharedString,
                       variant: ButtonVariant,
                       card: Option<String>,
                       then_handoff: bool| {
            // A feedback note being written owns the primary.
            let variant = if deny_open && variant == ButtonVariant::Primary {
                ButtonVariant::Secondary
            } else {
                variant
            };
            primitives::button(
                id,
                label,
                None,
                variant,
                ButtonSize::Small,
                enabled && (!then_handoff || can_hand_off),
                &p,
            )
            .when(enabled && (!then_handoff || can_hand_off), |d| {
                d.on_click(cx.listener(move |this, _, window, cx| {
                    this.act(Action::Approve(true, card.clone()), cx);
                    if then_handoff {
                        this.open_feature(Screen::Handoff, window, cx);
                    }
                }))
            })
        };
        let actions = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(t.space.sm))
            .map(|d| match handoff_to {
                Some(other) => d
                    .child(
                        approve(
                            "approve-handoff",
                            format!("Approve and continue with {other}…").into(),
                            ButtonVariant::Primary,
                            card.clone(),
                            true,
                        )
                        .debug_selector(|| "plan-approve-handoff".into()),
                    )
                    .child(
                        approve(
                            "approve",
                            "Approve plan".into(),
                            ButtonVariant::Secondary,
                            card.clone(),
                            false,
                        )
                        .debug_selector(|| "plan-approve".into()),
                    ),
                None => d.child(
                    approve(
                        "approve",
                        "Approve plan".into(),
                        ButtonVariant::Primary,
                        card.clone(),
                        false,
                    )
                    .debug_selector(|| "plan-approve".into()),
                ),
            })
            .when(!self.extras.deny_open, |d| {
                let card = card.clone();
                d.child(
                    primitives::button(
                        "plan-revise",
                        "Keep planning…",
                        None,
                        ButtonVariant::Ghost,
                        ButtonSize::Small,
                        enabled,
                        &p,
                    )
                    .debug_selector(|| "plan-revise".into())
                    .tooltip(primitives::tooltip_view(
                        "Send feedback and let it keep planning",
                        None,
                        &p,
                    ))
                    .when(enabled, |d| {
                        d.on_click(cx.listener(move |this, _, window, cx| {
                            this.open_deny_note(card.clone(), window, cx)
                        }))
                    }),
                )
            });
        primitives::card(&p)
            .debug_selector(|| "plan-card".into())
            .occlude()
            .w_full()
            .flex_shrink_0()
            .when(compact, |d| d.p(px(t.space.md)))
            .shadow(primitives::elevation(&p, t.elevation.floating))
            .child(head.render(&p))
            .child(
                div()
                    .id("plan-body")
                    .debug_selector(|| "plan-body".into())
                    .max_h(px(if compact {
                        t.layout.row_height * 4.
                    } else {
                        t.layout.row_height * 9.
                    }))
                    .overflow_y_scroll()
                    .px(px(t.space.xl))
                    .py(px(t.space.md))
                    .rounded(px(t.radius.control))
                    .bg(rgb(p.chat))
                    .border(px(t.stroke.hairline))
                    .border_color(rgb(p.roles.card_border))
                    .child(self.render_markdown("approval-plan", plan, window, cx)),
            )
            .when(self.extras.deny_open, |d| {
                d.child(self.render_deny_note_v2(enabled, "Keep planning", cx))
            })
            .child(actions)
    }

    /// The other provider the limit card would hand this session to.
    fn other_agent(&self) -> Option<&'static str> {
        let session = self.selected_session()?;
        Some(match session.provider_id() {
            "codex" => "Claude",
            _ => "Codex",
        })
    }

    /// The account is out of a usage window: which, when it comes back, and
    /// the handoff. Its "Continue with" is the primary unless the plan card
    /// above already owns it (then it offers the same thing in one step).
    pub(super) fn render_limit_card_v2(
        &self,
        session: &Session,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let p = self.palette();
        let t = p.tokens;
        let limits = &session.limits;
        let Some((window, resets_at)) = limits.exhausted() else {
            let warning = limits.warning.clone()?;
            return Some(
                primitives::card(&p)
                    .debug_selector(|| "limit-card".into())
                    .occlude()
                    .w_full()
                    .flex_shrink_0()
                    .p(px(if compact { t.space.md } else { t.space.lg }))
                    .shadow(primitives::elevation(&p, t.elevation.floating))
                    .child(
                        StatusBadge::new(StatusKind::NeedsYou)
                            .label(warning)
                            .render(&p),
                    ),
            );
        };
        let decisions = self.decisions(session);
        if !decisions.contains(&Decision::Limit) {
            return None;
        }
        let provider = match session.provider_id() {
            "codex" => "Codex",
            _ => "Claude",
        };
        let other = self.other_agent().unwrap_or("Codex");
        let when = resets_at
            .map(|at| {
                let now = wks_native::timing::now_ms() / 1000;
                format!("Resets in {}", wks_native::usage::resets_in(at, now))
            })
            .unwrap_or_else(|| "Resets later".into());
        let can_hand_off = self.supported_session() && self.view.connected && !self.demo;
        let plan_owns_primary = decisions.contains(&Decision::Plan);
        let key = (session.id.clone(), window, resets_at);
        let mut head = CardHead::new(format!("{provider}'s {window} limit is reached"))
            .icon(IconName::TriangleAlert)
            .status(StatusBadge::new(StatusKind::NeedsYou).label("Usage limit"))
            .meta(format!(
                "{when} · messages wait until then, or {other} can carry on now"
            ));
        if let Some(label) = position_label(&decisions, Decision::Limit) {
            head = head.action(
                div()
                    .debug_selector(|| "limit-position".into())
                    .text_size(px(t.text.meta))
                    .text_color(rgb(p.roles.text_tertiary))
                    .child(label),
            );
        }
        Some(
            primitives::card(&p)
                .debug_selector(|| "limit-card".into())
                .occlude()
                .w_full()
                .flex_shrink_0()
                .when(compact, |d| d.p(px(t.space.md)))
                .shadow(primitives::elevation(&p, t.elevation.floating))
                .child(head.render(&p))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(t.space.sm))
                        .child(
                            primitives::button(
                                "limit-handoff",
                                format!("Continue with {other}…"),
                                None,
                                if plan_owns_primary {
                                    ButtonVariant::Secondary
                                } else {
                                    ButtonVariant::Primary
                                },
                                ButtonSize::Small,
                                can_hand_off,
                                &p,
                            )
                            .debug_selector(|| "limit-handoff".into())
                            .when(can_hand_off, |d| {
                                d.on_click(cx.listener(|this, _, window, cx| {
                                    this.open_feature(Screen::Handoff, window, cx)
                                }))
                            }),
                        )
                        .child(
                            primitives::button(
                                "limit-dismiss",
                                "Wait for the reset",
                                None,
                                ButtonVariant::Ghost,
                                ButtonSize::Small,
                                true,
                                &p,
                            )
                            .debug_selector(|| "limit-dismiss".into())
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.extras.limit_dismissed = Some(key.clone());
                                    cx.notify();
                                },
                            )),
                        ),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plans_are_told_apart_from_tool_approvals() {
        let plan = json!({"toolName":"ExitPlanMode","toolInput":{"plan":"## Do it"}});
        assert_eq!(plan_text(&plan), Some("## Do it"));
        let empty = json!({"toolName":"ExitPlanMode","toolInput":{"plan":"  "}});
        assert_eq!(plan_text(&empty), None);
        let bash = json!({"toolName":"Bash","toolInput":{"command":"cargo test\n--all"}});
        assert_eq!(plan_text(&bash), None);
        assert_eq!(
            approval_summary(&bash),
            ("Bash".into(), "cargo test".into())
        );
        assert_eq!(approval_summary(&json!({})), ("Tool".into(), String::new()));
    }

    #[test]
    fn details_read_as_facts_not_json() {
        let approval = json!({"toolName":"Bash","toolInput":{
            "command":"cargo test",
            "description":"Run the workspace tests",
            "timeout": 120000,
            "run_in_background": false,
        }});
        let facts = approval_facts(
            &approval,
            "/home/dev/workspacer",
            "This machine",
            "Tests · Sonnet",
        );
        let labels: Vec<&str> = facts.iter().map(|(l, _, _)| l.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Command",
                "Folder",
                "Host",
                "Agent",
                "Reason",
                "Timeout",
                "Run in background"
            ]
        );
        assert_eq!(facts[4].1, "Run the workspace tests");
        assert!(facts.iter().all(|(_, v, _)| !v.contains('{')));
        // Nothing known about where or why: only the request itself.
        let bare = approval_facts(&json!({"toolInput":{"file_path":"a.rs"}}), "", "", "");
        assert_eq!(bare, vec![("File".into(), "a.rs".into(), true)]);
    }

    #[test]
    fn strip_numbers_follow_the_buttons() {
        assert_eq!(
            strip_keys(0, true),
            [StripKey::Allow, StripKey::Deny, StripKey::DenyWithNote]
        );
        assert_eq!(
            strip_keys(2, false),
            [
                StripKey::Allow,
                StripKey::Always(0),
                StripKey::Always(1),
                StripKey::Deny
            ]
        );
    }

    #[test]
    fn stacked_decisions_count_from_one() {
        let both = [Decision::Plan, Decision::Limit];
        assert_eq!(
            position_label(&both, Decision::Limit).as_deref(),
            Some("2 of 2")
        );
        assert_eq!(position_label(&both[..1], Decision::Plan), None);
        assert_eq!(position_label(&both, Decision::Questions), None);
    }

    #[test]
    fn the_send_button_names_what_it_does() {
        assert_eq!(send_copy(false).0, "Send");
        assert_eq!(send_copy(true).0, "Steer");
        assert!(send_copy(true).1.contains("without stopping"));
        assert_eq!(
            waiting_label(Decision::Approval),
            "Waiting for your approval"
        );
        assert_eq!(sentence_case("run_in_background"), "Run in background");
        assert_eq!(sentence_case("oldString"), "Old string");
    }
}
