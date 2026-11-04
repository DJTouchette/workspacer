//! Design v2 for the conversation's bottom dock: the composer, the approval
//! strip attached to it, and the plan and limit decision cards above it.
//! Only drawn when `Workspace::design()` is v2; v1 keeps `conversation.rs`.
//!
//! Rules (`docs/design-v2-direction.md` §4 and the Round 2 addendum):
//! - One primary per region. The composer's send is the primary until a
//!   decision is open; then the decision owns the primary, the composer
//!   dims, its placeholder names the block and its send drops to secondary.
//! - The send button says what it does: "Send" when the agent is idle;
//!   "Send now" and "Queue" while it works, with Steer (the default Enter)
//!   in the split menu with a line explaining it. Shortcuts live in
//!   tooltips, not a hint row.
//! - Every decision (approval, plan, usage limit, questions) is the one
//!   "waiting on you" container (`primitives::waiting_card`): a question
//!   headline, an always-visible where/why line, and actions in the four
//!   button roles with number keys.
//! - A plan and a spent usage limit are ONE card.
use super::conversation::ChatLayout;
use super::primitives::{self, ButtonSize, ButtonVariant, StatusBadge, StatusKind};
use super::*;
use gpui::{AnyElement, relative};
use serde_json::Value;

/// The v2 composer's own state.
#[derive(Default)]
pub(super) struct ComposerUi {
    /// The send button's split menu (Steer / Queue) is open.
    pub send_menu: bool,
    /// The question a multi-question set's stepper shows (0-based).
    pub question_step: usize,
    /// The placeholder last given to the composer, when v2 changed it.
    pub placeholder: Option<&'static str>,
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

/// The send button's label and tooltip: "Send now" (it interrupts) while
/// the agent works, "Send" otherwise.
pub(super) fn send_copy(steering: bool) -> (&'static str, &'static str) {
    if steering {
        (
            "Send now",
            "Interrupt: the agent sets its current work aside and reads this at once",
        )
    } else {
        ("Send", "Send this message to the agent")
    }
}

/// What Steer does, for its menu row and tooltip.
pub(super) const STEER_COPY: &str = "The agent reads this at its next step, without stopping its work. What Enter does while it works.";

/// What Queue does.
pub(super) const QUEUE_COPY: &str =
    "Hold it until this turn ends, then send it as the next message.";

/// The composer's placeholder while a decision waits on the user (the
/// default otherwise).
pub(super) fn composer_placeholder(decision: Option<Decision>) -> &'static str {
    match decision {
        None => DEFAULT_PLACEHOLDER,
        Some(Decision::Approval) => "Allow or deny the request above to continue…",
        Some(Decision::Plan) => "Approve the plan above, or reply to keep planning…",
        Some(Decision::Limit) => "Messages wait until the usage limit resets…",
        Some(Decision::Questions) => "Answer the questions above, or reply here instead…",
    }
}

/// The composer's resting placeholder (`ui.rs` sets it at startup).
pub(super) const DEFAULT_PLACEHOLDER: &str = "Ask anything, or describe a task…";

/// The decisions as the dock draws them: a plan absorbs a spent usage limit
/// into one card, so the limit is not a card of its own beside it.
pub(super) fn decision_cards(decisions: &[Decision]) -> Vec<Decision> {
    let plan = decisions.contains(&Decision::Plan);
    decisions
        .iter()
        .copied()
        .filter(|d| !(plan && *d == Decision::Limit))
        .collect()
}

/// The last folder or two of a path, for one-line context ("apps/native").
fn short_path(path: &str) -> String {
    let parts: Vec<&str> = path.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    match parts.as_slice() {
        [] => path.to_owned(),
        [one] => (*one).to_owned(),
        [.., a, b] => format!("{a}/{b}"),
    }
}

/// `path` relative to `cwd` when it lies inside it.
fn relative_to<'a>(path: &'a str, cwd: &str) -> &'a str {
    let cwd = cwd.trim_end_matches('/');
    if cwd.is_empty() {
        return path;
    }
    path.strip_prefix(cwd)
        .and_then(|rest| rest.strip_prefix('/'))
        .filter(|rest| !rest.is_empty())
        .unwrap_or(path)
}

/// An approval as a question (`docs/design-v2-direction.md` R2.2): runs of
/// plain words and code (`true`), e.g. ("Run", `cargo test`, "in native?").
pub(super) fn approval_headline(approval: &Value, cwd: &str) -> Vec<(String, bool)> {
    let (tool, summary) = approval_summary(approval);
    let input = approval.get("toolInput").unwrap_or(&Value::Null);
    let text = |s: &str| (s.to_owned(), false);
    let code = |s: &str| (s.to_owned(), true);
    let file = input
        .get("file_path")
        .or_else(|| input.get("path"))
        .or_else(|| input.get("notebook_path"))
        .and_then(Value::as_str)
        .map(|f| relative_to(f, cwd).to_owned());
    let lower = tool.to_ascii_lowercase();
    let folder = (!cwd.is_empty()).then(|| short_path(cwd));
    match lower.as_str() {
        "bash" | "shell" | "exec_command" | "local_shell" if !summary.is_empty() => {
            let mut parts = vec![text("Run"), code(&summary)];
            match folder {
                Some(folder) => parts.push(text(&format!("in {folder}?"))),
                None => parts.push(text("?")),
            }
            parts
        }
        "edit" | "multiedit" | "notebookedit" | "apply_patch" if file.is_some() => {
            vec![text("Edit"), code(&file.unwrap_or_default()), text("?")]
        }
        "write" if file.is_some() => {
            vec![text("Write"), code(&file.unwrap_or_default()), text("?")]
        }
        "read" if file.is_some() => vec![text("Read"), code(&file.unwrap_or_default()), text("?")],
        "webfetch" | "web_fetch" if !summary.is_empty() => {
            vec![text("Open"), code(&summary), text("?")]
        }
        "grep" | "glob" if !summary.is_empty() => {
            vec![text("Search for"), code(&summary), text("?")]
        }
        _ if !summary.is_empty() => {
            vec![text(&format!("Let {tool} use")), code(&summary), text("?")]
        }
        _ => vec![text(&format!("Allow {tool} to run?"))],
    }
}

/// The always-visible where/why line under an approval's headline: the
/// agent's reason, then where it would run.
pub(super) fn approval_context(approval: &Value, cwd: &str, host: &str) -> String {
    let input = approval.get("toolInput").unwrap_or(&Value::Null);
    let reason = input
        .get("description")
        .or_else(|| approval.get("reason"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|r| !r.is_empty());
    let place = match (cwd.is_empty(), host.is_empty()) {
        (false, false) => Some(format!("{} on {host}", short_path(cwd))),
        (false, true) => Some(short_path(cwd)),
        (true, false) => Some(host.to_owned()),
        (true, true) => None,
    };
    [reason.map(str::to_owned), place]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The pager on a decision card: "2/3" while more than one waits.
pub(super) fn pager(index: usize, total: usize) -> Option<String> {
    (total > 1).then(|| format!("{}/{total}", index + 1))
}

/// "1/2" when more than one decision card is open (a plan and its usage
/// limit are one card).
pub(super) fn position_label(decisions: &[Decision], decision: Decision) -> Option<String> {
    let cards = decision_cards(decisions);
    let ix = cards.iter().position(|d| *d == decision)?;
    pager(ix, cards.len())
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
        // While something waits on the user the composer yields: its
        // placeholder names the block, and it dims until it is used.
        let placeholder = composer_placeholder(decisions.first().copied());
        if self.composer_ui.placeholder.unwrap_or(DEFAULT_PLACEHOLDER) != placeholder {
            self.composer_ui.placeholder = Some(placeholder);
            self.composer.update(cx, |input, cx| {
                input.set_placeholder(placeholder, window, cx)
            });
        }
        let yielded = deciding && !focused && self.composer.read(cx).value().is_empty();
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
            // r2-systems: the human name; the raw id only on hover.
            let raw = s.model.split('[').next().unwrap_or("").trim().to_owned();
            let tip: SharedString = if raw.is_empty() || raw == name {
                "Model, effort and access".into()
            } else {
                format!("{raw} · Model, effort and access").into()
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
            .tooltip(primitives::tooltip_view(tip, Some("g m"), &p))
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
        let send = self.render_send_control(steering, deciding, can_send, narrow, cx);

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
            .child(
                div().px(px(t.space.xs)).child(
                    Input::new(&self.composer)
                        .appearance(false)
                        .disabled(self.view.busy),
                ),
            )
            .child(toolbar)
            .when(yielded, |d| d.opacity(t.wash.yielded));
        // The slash menu floats above the composer, over everything else in
        // the dock, never between a decision and the input.
        let slash = (!slash_rows.is_empty()).then(|| {
            div()
                .debug_selector(|| "slash-menu-layer".into())
                .absolute()
                .left_0()
                .right_0()
                .bottom(relative(1.))
                .pb(px(t.space.sm))
                .child(
                    primitives::popover(&p)
                        .occlude()
                        .p(px(t.space.xs))
                        .child(self.render_slash_menu(slash_rows, cx)),
                )
        });

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
            .child(
                div()
                    .relative()
                    .child(composer)
                    .children(menu)
                    .children(slash.map(gpui::deferred)),
            )
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
        if connected && !self.view.loading && !self.view.busy && !decisions.is_empty() {
            // r2-systems: the decision card and the placeholder already say
            // what is waited on; the toolbar does not repeat it.
            return None;
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

    /// "Send" (idle) as the composer's one primary. While the agent works:
    /// a secondary "Queue" and a split "Send now" whose menu holds Steer,
    /// each with its explanation. While a decision is open, nothing here is
    /// primary.
    fn render_send_control(
        &self,
        steering: bool,
        deciding: bool,
        can_send: bool,
        narrow: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let t = p.tokens;
        let variant = if deciding {
            ButtonVariant::SECONDARY
        } else {
            ButtonVariant::PRIMARY
        };
        let (label, tip) = send_copy(steering);
        let label = if self.view.busy { "Sending…" } else { label };
        if !steering {
            let key = self.send_key();
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
        let queue = (!narrow).then(|| {
            primitives::button(
                "queue",
                "Queue",
                None,
                ButtonVariant::SECONDARY,
                ButtonSize::Small,
                can_send,
                &p,
            )
            .debug_selector(|| "composer-queue-button".into())
            .tooltip(primitives::tooltip_view(QUEUE_COPY, Some("Tab"), &p))
            .when(can_send, |d| {
                d.on_click(
                    cx.listener(|this, _, window, cx| this.submit(Submit::Queue, window, cx)),
                )
            })
        });
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
            .tooltip(primitives::tooltip_view(tip, Some(self.send_now_key()), &p))
            .when(can_send, |d| {
                d.on_click(cx.listener(|this, _, window, cx| this.submit(Submit::Now, window, cx)))
            });
        let chevron = chevron
            .debug_selector(|| "composer-send-menu".into())
            .when(can_send, |d| {
                d.on_click(cx.listener(|this, _, _, cx| {
                    this.composer_ui.send_menu = !this.composer_ui.send_menu;
                    cx.notify();
                }))
            });
        div()
            .flex()
            .items_center()
            .gap(px(t.space.sm))
            .children(queue)
            .child(primitives::split_button(main, chevron, &p))
            .into_any_element()
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

    /// The split menu over Send now: the other ways to deliver a draft
    /// mid-turn, each with a line saying what it does. Steer is what the
    /// send key does while the agent works.
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
                            "composer-steer",
                            IconName::ArrowUp,
                            "Steer",
                            STEER_COPY,
                            Some(self.send_key()),
                        )
                        .when(can_send, |d| {
                            d.on_click(cx.listener(|this, _, window, cx| {
                                this.composer_ui.send_menu = false;
                                this.submit(Submit::Send, window, cx)
                            }))
                        }),
                    )
                    .child(
                        item(
                            "composer-queue",
                            IconName::Inbox,
                            "Queue",
                            QUEUE_COPY,
                            Some("Tab"),
                        )
                        .when(can_send, |d| {
                            d.on_click(cx.listener(|this, _, window, cx| {
                                this.composer_ui.send_menu = false;
                                this.submit(Submit::Queue, window, cx)
                            }))
                        }),
                    ),
            )
    }

    /// A pending tool approval as the "waiting on you" container: the
    /// request as a question ("Run `cargo test` in native?"), where and why
    /// under it, Details as a tertiary toggle, and the four numbered actions
    /// in their roles (1 Allow primary, 2 Always allow secondary, 3 Deny
    /// destructive, 4 Deny with note tertiary).
    pub(super) fn render_approval_strip(
        &self,
        approval: &Value,
        layout: ChatLayout,
        decisions: &[Decision],
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let (compact, enabled) = (layout.compact, layout.enabled);
        let open = self.extras.approval_details;
        let session = self.selected_session();
        let cwd = session.map(|s| s.cwd.clone()).unwrap_or_default();
        let card = session.and_then(Session::card);
        let always = wks_native::session_controls::always_options(approval);
        let note = self.deny_note_offered() && !self.extras.deny_open;
        let keys = strip_keys(always.len(), note);
        let number = |key: &StripKey| {
            keys.iter()
                .position(|k| k == key)
                .map(|ix| (ix + 1).to_string())
                .unwrap_or_default()
        };
        let headline = primitives::headline(
            approval_headline(approval, &cwd)
                .into_iter()
                .map(|(part, code)| {
                    if code {
                        primitives::Headline::Code(part.into())
                    } else {
                        primitives::Headline::Text(part.into())
                    }
                })
                .collect(),
            &p,
        )
        .debug_selector(|| "approval-summary".into());
        let context = approval_context(approval, &cwd, self.host_label());
        let toggle = primitives::button(
            if compact {
                "approval-toggle-compact"
            } else {
                "approval-toggle"
            },
            if open { "Hide details" } else { "Details" },
            None,
            ButtonVariant::TERTIARY,
            ButtonSize::Small,
            true,
            &p,
        )
        .debug_selector(|| "approval-toggle".into())
        .on_click(cx.listener(|this, _, _, cx| {
            this.extras.approval_details = !this.extras.approval_details;
            cx.notify();
        }))
        .into_any_element();
        let head = primitives::waiting_head(
            headline,
            (!context.is_empty()).then(|| context.into()),
            position_label(decisions, Decision::Approval).map(Into::into),
            vec![toggle],
            &p,
        );
        let details = open.then(|| {
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
            primitives::inset(&p)
                .id("approval-details")
                .debug_selector(|| "approval-details".into())
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
        let actions = primitives::waiting_actions(&p)
            .child({
                let card = card.clone();
                primitives::keyed_button(
                    "approve",
                    number(&StripKey::Allow),
                    "Allow",
                    // While a note is being written, sending it is the primary.
                    if self.extras.deny_open {
                        ButtonVariant::SECONDARY
                    } else {
                        ButtonVariant::PRIMARY
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
                    ButtonVariant::SECONDARY,
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
                    ButtonVariant::DESTRUCTIVE,
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
                        ButtonVariant::TERTIARY,
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
        primitives::waiting_card(compact, &p)
            .debug_selector(|| "approval-card".into())
            .occlude()
            .child(head)
            .children(details)
            .children(deny_note)
            .child(actions)
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
            // r2-systems: with no approval, number keys answer the question
            // the stepper shows.
            if session.questions.is_some() && self.question_number_key(n, window, cx) {
                cx.stop_propagation();
            }
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

    /// The usage window this session's account has spent, as the plan or
    /// limit card's where/why line: "Claude's 5-hour limit is reached ·
    /// resets in 2h 13m".
    fn limit_context(&self, session: &Session) -> Option<String> {
        let (window, resets_at) = session.limits.exhausted()?;
        let provider = match session.provider_id() {
            "codex" => "Codex",
            _ => "Claude",
        };
        let when = resets_at
            .map(|at| {
                let now = wks_native::timing::now_ms() / 1000;
                format!("resets in {}", wks_native::usage::resets_in(at, now))
            })
            .unwrap_or_else(|| "resets later".into());
        Some(format!("{provider}'s {window} limit is reached · {when}"))
    }

    /// Claude finished planning: the plan as a "waiting on you" container.
    /// A spent usage limit is part of the same card: Approve plan stays the
    /// primary (it waits for the reset), "Approve · run with Codex" is the
    /// secondary, Keep planning the tertiary.
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
        let limited = decisions.contains(&Decision::Limit);
        let handoff_to = limited.then(|| self.other_agent()).flatten();
        let can_hand_off = self.supported_session() && self.view.connected && !self.demo;
        let deny_open = self.extras.deny_open;
        let agent = session
            .map(|s| match s.provider_id() {
                "codex" => "Codex",
                _ => "Claude",
            })
            .unwrap_or("the agent");
        let context = match session
            .and_then(|s| self.limit_context(s))
            .filter(|_| limited)
        {
            Some(limit) => format!("{limit} · approving waits for it"),
            None => format!("Nothing runs until you approve · {agent} starts on approval"),
        };
        let head = primitives::waiting_head(
            primitives::headline(
                vec![primitives::Headline::Text("Start on this plan?".into())],
                &p,
            )
            .debug_selector(|| "plan-headline".into()),
            Some(context.into()),
            position_label(&decisions, Decision::Plan).map(Into::into),
            Vec::new(),
            &p,
        )
        .debug_selector(|| "plan-head".into());
        let approve = |id: &'static str,
                       label: SharedString,
                       variant: ButtonVariant,
                       card: Option<String>,
                       then_handoff: bool| {
            // A feedback note being written owns the primary.
            let variant = if deny_open && variant == ButtonVariant::PRIMARY {
                ButtonVariant::SECONDARY
            } else {
                variant
            };
            let enabled = enabled && (!then_handoff || can_hand_off);
            primitives::button(id, label, None, variant, ButtonSize::Small, enabled, &p).when(
                enabled,
                |d| {
                    d.on_click(cx.listener(move |this, _, window, cx| {
                        this.act(Action::Approve(true, card.clone()), cx);
                        if then_handoff {
                            this.open_feature(Screen::Handoff, window, cx);
                        }
                    }))
                },
            )
        };
        let actions = primitives::waiting_actions(&p)
            .child(
                approve(
                    "approve",
                    "Approve plan".into(),
                    ButtonVariant::PRIMARY,
                    card.clone(),
                    false,
                )
                .debug_selector(|| "plan-approve".into()),
            )
            .children(handoff_to.map(|other| {
                approve(
                    "approve-handoff",
                    format!("Approve · run with {other}").into(),
                    ButtonVariant::SECONDARY,
                    card.clone(),
                    true,
                )
                .debug_selector(|| "plan-approve-handoff".into())
                .tooltip(primitives::tooltip_view(
                    format!("Approve, then hand this session to {other} instead of waiting"),
                    None,
                    &p,
                ))
            }))
            .when(!self.extras.deny_open, |d| {
                let card = card.clone();
                d.child(
                    primitives::button(
                        "plan-revise",
                        "Keep planning",
                        None,
                        ButtonVariant::TERTIARY,
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
        primitives::waiting_card(compact, &p)
            .debug_selector(|| "plan-card".into())
            .occlude()
            .child(head)
            .child(
                primitives::inset(&p)
                    // Markdown lays out as a block, not a flex column.
                    .block()
                    .id("plan-body")
                    .debug_selector(|| "plan-body".into())
                    .max_h(px(if compact {
                        t.layout.row_height * 4.
                    } else {
                        t.layout.row_height * 9.
                    }))
                    .overflow_y_scroll()
                    .px(px(t.space.xl))
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

    /// The account is out of a usage window (and no plan card has absorbed
    /// it): the "waiting on you" container asking whether to carry on with
    /// the other agent. A warning before the limit is quiet, not attention.
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
            // Nothing to decide yet: a neutral note, not attention amber.
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
                        div()
                            .flex()
                            .items_center()
                            .gap(px(t.space.sm))
                            .text_size(px(t.text.meta))
                            .text_color(rgb(p.roles.text_secondary))
                            .child(Icon::new(IconName::Info).size(px(t.icon.sm)))
                            .child(warning),
                    ),
            );
        };
        let decisions = self.decisions(session);
        if !decision_cards(&decisions).contains(&Decision::Limit) {
            return None;
        }
        let provider = match session.provider_id() {
            "codex" => "Codex",
            _ => "Claude",
        };
        let other = self.other_agent().unwrap_or("Codex");
        let can_hand_off = self.supported_session() && self.view.connected && !self.demo;
        let key = (session.id.clone(), window, resets_at);
        let context = self
            .limit_context(session)
            .map(|line| format!("{line} · messages wait until then"));
        let head = primitives::waiting_head(
            primitives::headline(
                vec![primitives::Headline::Text(
                    format!("Continue with {other} while {provider} is paused?").into(),
                )],
                &p,
            ),
            context.map(Into::into),
            position_label(&decisions, Decision::Limit).map(Into::into),
            Vec::new(),
            &p,
        )
        .debug_selector(|| "limit-head".into());
        Some(
            primitives::waiting_card(compact, &p)
                .debug_selector(|| "limit-card".into())
                .occlude()
                .child(head)
                .child(
                    primitives::waiting_actions(&p)
                        .child(
                            primitives::button(
                                "limit-handoff",
                                format!("Continue with {other}"),
                                None,
                                ButtonVariant::PRIMARY,
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
                                ButtonVariant::TERTIARY,
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
        // A plan absorbs a spent limit: one card, no pager.
        let both = [Decision::Plan, Decision::Limit];
        assert_eq!(decision_cards(&both), [Decision::Plan]);
        assert_eq!(position_label(&both, Decision::Plan), None);
        assert_eq!(position_label(&both, Decision::Limit), None);
        let three = [Decision::Approval, Decision::Limit, Decision::Questions];
        assert_eq!(
            position_label(&three, Decision::Questions).as_deref(),
            Some("3/3")
        );
        assert_eq!(position_label(&three[..1], Decision::Approval), None);
        assert_eq!(pager(1, 3).as_deref(), Some("2/3"));
        assert_eq!(pager(0, 1), None);
    }

    #[test]
    fn the_send_button_names_what_it_does() {
        assert_eq!(send_copy(false).0, "Send");
        assert_eq!(send_copy(true).0, "Send now");
        assert!(STEER_COPY.contains("without stopping"));
        assert_eq!(composer_placeholder(None), DEFAULT_PLACEHOLDER);
        assert!(composer_placeholder(Some(Decision::Approval)).contains("Allow or deny"));
        assert_eq!(sentence_case("run_in_background"), "Run in background");
        assert_eq!(sentence_case("oldString"), "Old string");
    }

    #[test]
    fn approvals_read_as_questions_with_where_and_why() {
        let bash = json!({"toolName":"Bash","toolInput":{
            "command":"cargo test","description":"Run the tests before committing"}});
        let cwd = "/home/dev/src/workspacer/apps/native";
        assert_eq!(
            approval_headline(&bash, cwd),
            [
                ("Run".to_owned(), false),
                ("cargo test".to_owned(), true),
                ("in apps/native?".to_owned(), false)
            ]
        );
        assert_eq!(
            approval_context(&bash, cwd, "This machine"),
            "Run the tests before committing · apps/native on This machine"
        );
        let edit = json!({"toolName":"Edit","toolInput":{"file_path":"/home/dev/src/workspacer/apps/native/src/ui.rs"}});
        assert_eq!(
            approval_headline(&edit, cwd)[1],
            ("src/ui.rs".to_owned(), true)
        );
        assert_eq!(
            approval_headline(&json!({"toolName":"Mystery"}), ""),
            [("Allow Mystery to run?".to_owned(), false)]
        );
        assert_eq!(approval_context(&json!({}), "", ""), "");
    }
}
