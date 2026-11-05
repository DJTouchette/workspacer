//! Design v2 for the conversation's bottom dock: the composer, the approval
//! strip attached to it, and the plan and limit decision cards above it.
//! Only drawn when `Workspace::design()` is v2; v1 keeps `conversation.rs`.
//!
//! Rules (`docs/design-v2-direction.md` §4 and the Round 2 and 3 addenda):
//! - One primary per region. The composer's send is the primary until a
//!   decision is open; then the decision card owns the primary and the
//!   composer stays a visible, bordered field (R5.4, `system::composer_field`)
//!   whose placeholder says "Reply — the request stays open" and
//!   whose Send is Secondary (`system::composer_send_state`).
//! - One primary button in one place whose label says what it does:
//!   "Send" when idle; "Queue" while the agent works and there is a draft
//!   (Enter: it runs after the current step), with Send now and After this
//!   turn in its split menu; "Stop · Esc" while it works with nothing
//!   drafted. Shortcuts live in tooltips and faint keycap hints.
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
    /// Queue's split menu (Send now / After this turn) is open.
    pub send_menu: bool,
    /// R6.4: the access chip's menu is open.
    pub access_menu: bool,
    /// The question a multi-question set's stepper shows (0-based).
    pub question_step: usize,
    /// The placeholder last given to the composer, when v2 changed it.
    pub placeholder: Option<&'static str>,
    /// R6.3: the digit keys answer the keyed decision (no text field has
    /// focus). Set once per frame by the dock, before its cards draw, so
    /// keycaps and the hint show only where digits actually go.
    pub digits_to_card: bool,
    /// R6.3: the decision (session and card) whose one-line composer was
    /// opened. It stays open for that decision, so a click elsewhere (which
    /// moves the focus on press) never folds it under the pointer.
    pub reply_open: Option<String>,
    /// R8.4: the draft's attached files, counted once per frame for the
    /// card's reply line.
    pub reply_files: usize,
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

/// The full request an approval carries, as (label, value, monospace) rows
/// for its Details (R4.3: Details expands the whole request in place): the
/// tool, every input it was given (commands and paths whole), the folder
/// and the host. No raw JSON.
pub(super) fn approval_facts(
    approval: &Value,
    cwd: &str,
    host: &str,
) -> Vec<(String, String, bool)> {
    const MAX: usize = 4000;
    let input = approval.get("toolInput").unwrap_or(&Value::Null);
    let (tool, _) = approval_summary(approval);
    let mut rows = vec![("Tool".to_owned(), tool, false)];
    let named = [
        ("command", "Command"),
        ("file_path", "File"),
        ("path", "Path"),
        ("url", "URL"),
        ("pattern", "Pattern"),
        ("description", "Reason"),
    ];
    for (key, label) in named {
        if let Some(value) = input
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim_end)
            .filter(|v| !v.is_empty())
        {
            rows.push((label.to_owned(), value.to_owned(), key != "description"));
        }
    }
    // Anything else the tool was given, each in full.
    if let Some(object) = input.as_object() {
        for (key, value) in object {
            if named.iter().any(|(k, _)| k == key) {
                continue;
            }
            let text = match value {
                Value::String(s) => s.clone(),
                Value::Number(_) | Value::Bool(_) => value.to_string(),
                Value::Null => continue,
                other => serde_json::to_string_pretty(other).unwrap_or_default(),
            };
            let text = wks_native::format::truncate_end(&text, MAX).unwrap_or(text);
            rows.push((sentence_case(key), text, true));
        }
    }
    if !cwd.trim().is_empty() {
        rows.push(("Folder".to_owned(), cwd.trim().to_owned(), true));
    }
    if !host.trim().is_empty() {
        rows.push(("Host".to_owned(), host.trim().to_owned(), false));
    }
    rows
}

/// The command an approval would run, whole, when the headline cannot show
/// it (several lines, or long): it then shows wrapped under the headline.
pub(super) fn approval_request(approval: &Value) -> Option<String> {
    approval
        .pointer("/toolInput/command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|c| super::decision::needs_request_line(c))
        .map(str::to_owned)
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

/// An "Always allow" grant as its button says it and its tooltip explains
/// it (R3.2): the exact scope, derived from what the request carries.
/// Claude's grants are session rules ("Always allow Bash(cargo test:*) this
/// session"): a `Bash(x:*)` rule allows every command starting with `x`, a
/// bare `Bash(x)` only that command. Codex says "this command", so the
/// command is named. Words we cannot read stay as given, plus their scope:
/// every grant offered here lasts for this session only (`always_options`).
pub(super) fn always_copy(label: &str, approval: &Value) -> (String, String) {
    const SESSION: &str = " this session";
    const UNTIL: &str = "Until this session ends";
    let label = label.trim();
    let (_, command) = approval_summary(approval);
    let quoted = |text: &str| format!("\u{201c}{text}\u{201d}");
    if label.eq_ignore_ascii_case("Always allow this command for this session")
        && !command.is_empty()
    {
        return (
            format!("Always allow {}{SESSION}", quoted(&command)),
            format!("{UNTIL}, exactly {} runs without asking", quoted(&command)),
        );
    }
    let Some(body) = label
        .strip_suffix(SESSION)
        .map(|b| b.strip_suffix(" for").unwrap_or(b))
    else {
        // The words carry no scope: add the one it has.
        return (
            format!("{label}{SESSION}"),
            format!("{UNTIL}, matching requests run without asking"),
        );
    };
    if let Some(rules) = body.strip_prefix("Always allow ") {
        let mut said = Vec::new();
        let mut explained = Vec::new();
        for rule in rules.split(", ").map(str::trim).filter(|r| !r.is_empty()) {
            let parsed = rule
                .strip_suffix(')')
                .and_then(|r| r.split_once('('))
                .filter(|(tool, content)| !tool.is_empty() && !content.is_empty());
            match parsed {
                Some(("Bash", content)) => match content.strip_suffix(":*") {
                    Some(prefix) => {
                        said.push(format!("{prefix} commands"));
                        explained.push(format!("any command starting with {}", quoted(prefix)));
                    }
                    None => {
                        said.push(quoted(content));
                        explained.push(format!("exactly {}", quoted(content)));
                    }
                },
                Some((tool, content)) => {
                    said.push(format!("{tool} {content}"));
                    explained.push(format!("{tool} on {content}"));
                }
                None => {
                    said.push(rule.to_owned());
                    explained.push(format!("every {rule} request"));
                }
            }
        }
        if !said.is_empty() {
            return (
                format!("Always allow {}{SESSION}", said.join(", ")),
                format!("{UNTIL}, {} runs without asking", explained.join(", ")),
            );
        }
    }
    (
        label.to_owned(),
        format!("{UNTIL}, matching requests run without asking"),
    )
}

/// What the composer's one primary button does now (R3.2). It stays in one
/// place; its label changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SendSlot {
    /// Idle, or a reply written while a decision waits: send it.
    Send,
    /// The agent works and there is a draft: it runs after the current step
    /// (the send key's steer). Send now and After this turn are in its menu.
    Queue,
    /// The agent works and nothing is drafted: stop it (Esc).
    Stop,
}

/// The slot for the session's state: `working` = mid-turn, `steering` = a
/// message now would steer it (mid-turn, or one still in flight),
/// `deciding` = a decision waits, `draft` = there is something to send.
pub(super) fn send_slot(working: bool, steering: bool, deciding: bool, draft: bool) -> SendSlot {
    if deciding {
        SendSlot::Send
    } else if working && !draft {
        SendSlot::Stop
    } else if steering {
        SendSlot::Queue
    } else {
        SendSlot::Send
    }
}

/// The slot's label and tooltip.
pub(super) fn send_copy(slot: SendSlot) -> (&'static str, &'static str) {
    match slot {
        SendSlot::Send => ("Send", "Send this message to the agent"),
        SendSlot::Queue => ("Queue", QUEUE_COPY),
        SendSlot::Stop => ("Stop", "Stop the agent"),
    }
}

/// What Queue (the send key while the agent works) does.
pub(super) const QUEUE_COPY: &str = "Runs after the current step";

/// What Send now does, in Queue's menu.
pub(super) const SEND_NOW_COPY: &str =
    "Interrupt: the agent sets its current work aside and reads this at once.";

/// What After this turn does, in Queue's menu.
pub(super) const AFTER_TURN_COPY: &str =
    "Hold it until this turn ends, then send it as the next message.";

/// R8.4: the card's Text action that opens its one reply input.
pub(super) const REPLY_ACTION: &str = "Reply instead";

/// R8.4: what the open reply says beside its send (R7.7: the consequence).
pub(super) const REPLY_NOTE: &str = "The request stays open until you answer it";

/// The composer's placeholder while a decision waits (R5.4: the field
/// stays, only its words change).
pub(super) const REPLY_INSTEAD: &str = "Reply — the request stays open";

/// The composer's placeholder while a decision waits on the user (the
/// default otherwise). It never repeats the card.
pub(super) fn composer_placeholder(decision: Option<Decision>) -> &'static str {
    match decision {
        None => DEFAULT_PLACEHOLDER,
        Some(Decision::Limit) => "Messages wait until the usage limit resets…",
        Some(Decision::Approval | Decision::Plan | Decision::Questions) => REPLY_INSTEAD,
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
/// R8.4: a headline's runs as one plain line ("Run cargo test in
/// native?"), for the folded card: one space between runs, none before
/// closing punctuation.
pub(super) fn headline_words(runs: &[(String, bool)]) -> String {
    let mut out = String::new();
    for (text, _) in runs {
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let glued = text.starts_with(['?', '!', '.', ',', ':', ';', ')']);
        if !out.is_empty() && !glued {
            out.push(' ');
        }
        out.push_str(text);
    }
    out
}

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
            // R4.3: a command the headline cannot show whole shows wrapped
            // under it (`approval_request`), never cut beside Allow.
            let mut parts = if approval_request(approval).is_some() {
                vec![text("Run this command")]
            } else {
                vec![text("Run"), code(&summary)]
            };
            match folder {
                Some(folder) => parts.push(text(&format!("in {folder}?"))),
                None if parts.len() == 1 => parts[0].0.push('?'),
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

/// Why the agent asks (its stated reason), for the line under an
/// approval's headline. Where it would run (folder and host) is the
/// decision's own line (`decision::where_line`).
pub(super) fn approval_context(approval: &Value) -> Option<String> {
    let input = approval.get("toolInput").unwrap_or(&Value::Null);
    input
        .get("description")
        .or_else(|| approval.get("reason"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_owned)
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

/// The keys of a tool approval (R7.1, the one key map): 1 Allow, 2 Deny,
/// 3 the first "Always allow" (a second grant has no key), 4 Deny with
/// note. A key never moves to fill a gap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum StripKey {
    Allow,
    Always(usize),
    Deny,
    DenyWithNote,
}

impl StripKey {
    /// Its number key, from the slot map.
    pub(super) fn number(&self) -> Option<usize> {
        use super::decision::Slot;
        match self {
            Self::Allow => Some(Slot::Positive.key()),
            Self::Deny => Some(Slot::Deny.key()),
            Self::Always(0) => Some(Slot::Always.key()),
            Self::Always(_) => None,
            Self::DenyWithNote => Some(Slot::DenyNote.key()),
        }
    }
}

/// The keyed actions of an approval with `always` grants and, when `note`,
/// Deny with note, in key order.
pub(super) fn strip_keys(always: usize, note: bool) -> Vec<StripKey> {
    let mut keys = vec![StripKey::Allow, StripKey::Deny];
    if always > 0 {
        keys.push(StripKey::Always(0));
    }
    if note {
        keys.push(StripKey::DenyWithNote);
    }
    keys
}

/// What number key `n` does on such an approval.
pub(super) fn strip_key_at(n: usize, always: usize, note: bool) -> Option<StripKey> {
    strip_keys(always, note)
        .into_iter()
        .find(|k| k.number() == Some(n))
}

/// The keys of a plan review (R7.1): 1 Approve plan, 2 Keep planning (the
/// plan's "no", with its note), Shift+1 "Approve · run with Codex" while
/// a limit applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PlanKey {
    Approve,
    Handoff,
    KeepPlanning,
}

impl PlanKey {
    /// Its number key; the hand-off is Shift+1 and has none.
    pub(super) fn number(self) -> Option<usize> {
        use super::decision::Slot;
        match self {
            Self::Approve => Some(Slot::Positive.key()),
            Self::KeepPlanning => Some(Slot::Deny.key()),
            Self::Handoff => None,
        }
    }
}

pub(super) fn plan_keys(handoff: bool, keep_planning: bool) -> Vec<PlanKey> {
    let mut keys = vec![PlanKey::Approve];
    if keep_planning {
        keys.push(PlanKey::KeepPlanning);
    }
    if handoff {
        keys.push(PlanKey::Handoff);
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

    /// R6.3: whether the digit keys answer the keyed decision now: no text
    /// field has focus (the workspace root, or nothing, does) and the
    /// composer holds no draft.
    pub(super) fn digits_free(&self, window: &Window, cx: &App) -> bool {
        (self.focus.is_focused(window) || window.focused(cx).is_none())
            && self.composer.read(cx).value().is_empty()
    }

    /// Whether Deny with note is offered: Claude reads a deny's note, Codex
    /// gets it as a user message; other providers have no field for it.
    fn deny_note_offered(&self) -> bool {
        self.selected_session()
            .is_some_and(|s| matches!(s.provider_id(), "claude" | "codex"))
    }

    /// Whether a decision the session waits on takes the composer's slot
    /// (R3.2): an approval, a plan or a structured question set. A usage
    /// limit alone does not, and neither does a question the card cannot
    /// structure (it is answered from the composer).
    pub(super) fn decision_takes_composer(&self, session: &Session) -> bool {
        self.decisions(session).iter().any(|d| match d {
            Decision::Approval | Decision::Plan => true,
            Decision::Questions => session
                .questions
                .as_ref()
                .is_some_and(|q| !super::questions::question_list(q).is_empty()),
            Decision::Limit => false,
        })
    }

    // ----- r8-conversation (R8.4): one decision-and-reply area -----------

    /// The decision (session and card) a reply is open for.
    pub(super) fn reply_key(&self) -> Option<String> {
        self.selected_session()
            .map(|s| format!("{}\0{}", s.id, s.card().unwrap_or_default()))
    }

    /// R8.4: whether the composer is hidden: a decision takes its slot (an
    /// approval, a plan or a structured question set) and the card holds
    /// the one input instead.
    pub(super) fn composer_hidden(&self) -> bool {
        self.design().is_v2()
            && self.view.child.is_none()
            && self
                .selected_session()
                .is_some_and(|s| self.decision_takes_composer(s))
    }

    /// R8.4: whether the waiting card's reply is open (its input in the
    /// card, "Reply instead" pressed, or a draft already there).
    pub(super) fn reply_active(&self) -> bool {
        self.composer_hidden()
            && self.composer_ui.reply_open.is_some()
            && self.composer_ui.reply_open == self.reply_key()
    }

    /// R8.4: whether the waiting card folds to one line: the reader has
    /// scrolled up through the transcript and is not writing in it.
    pub(super) fn decision_collapsed(&self) -> bool {
        self.composer_hidden() && !self.follow && !self.reply_active() && !self.extras.deny_open
    }

    /// Once per frame, before the dock draws: a composer with focus or a
    /// draft opens the card's reply (so Ctrl+L, or a draft from before the
    /// request, lands in the card), and the input says the request stays
    /// open.
    pub(super) fn settle_reply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.composer_hidden() {
            return;
        }
        let composer = self.composer.read(cx);
        let focused = composer.focus_handle(cx).is_focused(window);
        let text = !composer.value().is_empty();
        let files = self.draft_files(cx).len();
        self.composer_ui.reply_files = files;
        if focused || text || files > 0 || self.uploading() {
            self.composer_ui.reply_open = self.reply_key();
        }
        if self.composer_ui.placeholder != Some(REPLY_INSTEAD) {
            self.composer_ui.placeholder = Some(REPLY_INSTEAD);
            self.composer.update(cx, |input, cx| {
                input.set_placeholder(REPLY_INSTEAD, window, cx)
            });
        }
    }

    /// Opens the card's reply and puts the cursor in it.
    pub(super) fn open_reply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.extras.deny_open = false;
        self.composer_ui.reply_open = self.reply_key();
        self.composer
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    /// Cancel: the reply closes, its text goes, and the digits answer the
    /// card again.
    pub(super) fn close_reply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer_ui.reply_open = None;
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.focus.focus(window);
        cx.notify();
    }

    /// R8.4: "Reply instead", the card's Text action that opens its one
    /// input. None while the reply or a note is already open.
    pub(super) fn reply_instead(
        &self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> Option<super::decision::Choice> {
        (self.composer_hidden() && !self.reply_active() && !self.extras.deny_open).then(|| {
            super::decision::Choice::new("reply-instead", REPLY_ACTION)
                .tooltip(super::decision::REPLY_KEEPS_PENDING)
                .enabled(enabled)
                .on_click(cx.listener(|this, _, window, cx| this.open_reply(window, cx)))
        })
    }

    /// "Reply instead" as a bare button, for a card that is not a
    /// `Decision` (the question set's footer).
    pub(super) fn reply_instead_button(
        &self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        let p = self.palette();
        self.reply_instead(enabled, cx).map(|_| {
            primitives::button(
                "reply-instead",
                REPLY_ACTION,
                None,
                system::actions::tier(system::actions::Verb::Details),
                ButtonSize::Small,
                enabled,
                &p,
            )
            .debug_selector(|| "reply-instead".into())
            .tooltip(primitives::tooltip_view(
                super::decision::REPLY_KEEPS_PENDING,
                None,
                &p,
            ))
            .when(enabled, |d| {
                d.on_click(cx.listener(|this, _, window, cx| this.open_reply(window, cx)))
            })
        })
    }

    /// The agent the reply goes to, by its one name (R8.6: "Claude Code",
    /// "Codex CLI").
    pub(super) fn reply_target(&self) -> &'static str {
        match self.selected_session().map(Session::provider_id) {
            Some("codex") => wks_native::format::agent_name("codex"),
            _ => wks_native::format::agent_name("claude"),
        }
    }

    /// R8.4: the open reply, last in the card: the composer's own input
    /// (Enter sends, as in the composer; Esc cancels), what happens to the
    /// request, Cancel, and the send that names where the text goes ("Send
    /// to Claude"). Secondary: the card's answer keeps the one Primary.
    pub(super) fn render_card_reply(
        &self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.reply_active() || self.extras.deny_open {
            return None;
        }
        let p = self.palette();
        let files = self.composer_ui.reply_files;
        let draft = !self.composer.read(cx).value().trim().is_empty() || files > 0;
        let can_send = enabled && !self.uploading() && draft && !self.view.busy;
        let slash = self.slash_menu(cx);
        let input = div()
            .w_full()
            .min_w_0()
            .debug_selector(|| "card-reply-input".into())
            .key_context(match (self.settings.enter_sends, slash.is_empty()) {
                (true, true) => "Composer ComposerEnter",
                (false, true) => "Composer",
                (true, false) => "Composer ComposerEnter SlashMenu",
                (false, false) => "Composer SlashMenu",
            })
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && this.slash_menu(cx).is_empty() {
                    cx.stop_propagation();
                    this.close_reply(window, cx);
                }
            }))
            .child(Input::new(&self.composer).disabled(self.view.busy));
        let menu = (!slash.is_empty()).then(|| {
            primitives::menu(&p)
                .debug_selector(|| "slash-menu-layer".into())
                .occlude()
                .child(self.render_slash_menu(slash, cx))
        });
        let input = system::anchored_menu(input, menu, system::MenuAnchor::AboveStart, &p).w_full();
        let note = match files {
            0 => REPLY_NOTE.to_owned(),
            1 => format!("1 file attached · {REPLY_NOTE}"),
            n => format!("{n} files attached · {REPLY_NOTE}"),
        };
        let cancel = primitives::button(
            "card-reply-cancel",
            "Cancel",
            None,
            ButtonVariant::Tertiary,
            ButtonSize::Small,
            true,
            &p,
        )
        .debug_selector(|| "card-reply-cancel".into())
        .tooltip(primitives::tooltip_view("Close the reply", Some("Esc"), &p))
        .on_click(cx.listener(|this, _, window, cx| this.close_reply(window, cx)))
        .into_any_element();
        let label = if self.view.busy {
            "Sending…".to_owned()
        } else {
            format!("Send to {}", self.reply_target())
        };
        let send = primitives::button(
            "card-reply-send",
            label,
            Some(IconName::ArrowUp),
            ButtonVariant::Secondary,
            ButtonSize::Small,
            can_send,
            &p,
        )
        .debug_selector(|| "card-reply-send".into())
        .tooltip(primitives::tooltip_view(
            super::decision::REPLY_KEEPS_PENDING,
            Some(self.send_key()),
            &p,
        ))
        .when(can_send, |d| {
            d.on_click(cx.listener(|this, _, window, cx| this.send(&SendMessage, window, cx)))
        })
        .into_any_element();
        Some(
            system::card_reply(input, note, [cancel, send], &p)
                .debug_selector(|| "card-reply".into())
                .into_any_element(),
        )
    }

    /// R8.4: the waiting decision in a few words, for its folded line
    /// ("Run `cargo test` in native?", "Approve the plan?", the question).
    pub(super) fn decision_summary(&self, session: &Session) -> String {
        match self.decisions(session).first() {
            Some(Decision::Approval) => session
                .approval
                .as_ref()
                .map(|a| headline_words(&approval_headline(a, &session.cwd)))
                .unwrap_or_default(),
            Some(Decision::Plan) => "Start on this plan?".to_owned(),
            Some(Decision::Questions) => session
                .questions
                .as_ref()
                .map(super::questions::question_list)
                .and_then(|list| {
                    list.first()
                        .and_then(|q| q["question"].as_str().or(q["header"].as_str()))
                        .map(|q| q.trim().to_owned())
                })
                .filter(|q| !q.is_empty())
                .unwrap_or_else(|| "The agent asked for input".to_owned()),
            _ => String::new(),
        }
    }

    /// R8.4: the folded card, in the dock's place for it: "Needs you ·
    /// Run `cargo test`? · Open". Open jumps to the latest and unfolds it.
    /// It replaces "Jump to latest" while it shows.
    pub(super) fn render_decision_line(&self, session: &Session, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let open = primitives::button(
            "decision-line-open",
            "Open",
            None,
            ButtonVariant::Secondary,
            ButtonSize::Small,
            true,
            &p,
        )
        .debug_selector(|| "decision-line-open".into())
        .tooltip(primitives::tooltip_view(
            "Back to the latest, with the request open",
            None,
            &p,
        ))
        .on_click(cx.listener(|this, _, _, cx| this.jump_to_latest(cx)));
        system::decision_line(self.decision_summary(session), open, &p)
            .debug_selector(|| "decision-line".into())
            .occlude()
    }

    /// Back to the newest message, following it.
    pub(super) fn jump_to_latest(&mut self, cx: &mut Context<Self>) {
        self.follow = true;
        self.list.scroll_to(gpui::ListOffset {
            item_ix: self.view.transcript.rows.len(),
            offset_in_item: gpui::Pixels::ZERO,
        });
        cx.notify();
    }

    /// Whether Esc stops a working agent (v2). Not with modal keys on: there
    /// Esc is how the composer is left for Normal mode, and stopping an
    /// agent must never be a side effect of that.
    fn escape_stops(&self) -> bool {
        self.design().is_v2() && !self.settings.vim(self.design())
    }

    /// Esc while the composer's primary reads "Stop · Esc" (v2): the agent
    /// is mid-turn and nothing is drafted. Returns whether it stopped it.
    pub(super) fn stop_on_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.escape_stops()
            || self.screen != Screen::Conversation
            || self.new_session
            || self.view.child.is_some()
            || !self.view.connected
            || self.view.busy
            || self.extras.deny_open
        {
            return false;
        }
        let working = self.selected_session().is_some_and(Session::working);
        let composer = self.composer.read(cx);
        let at_composer =
            composer.focus_handle(cx).is_focused(window) || self.focus.is_focused(window);
        if !working || !at_composer || !composer.value().trim().is_empty() || self.uploading() {
            return false;
        }
        let draft = self.draft_files(cx);
        if !draft.is_empty() {
            return false;
        }
        self.act(Action::Stop, cx);
        true
    }

    /// The v2 composer: the one composer component (`system::composer_field`)
    /// holding draft attachments, the input and a toolbar of chips, status
    /// and the send control, with the slash menu anchored above it. It is
    /// always a visible, bordered field; while a decision waits only its
    /// placeholder and its Send's tier change (R5.4).
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
        // R3.2: an approval, plan or question takes the composer's slot.
        let takes_slot = selected
            .as_ref()
            .is_some_and(|s| self.decision_takes_composer(s));
        let empty = self.composer.read(cx).value().is_empty();
        let placeholder = if takes_slot {
            REPLY_INSTEAD
        } else {
            composer_placeholder(decisions.first().copied())
        };
        if self.composer_ui.placeholder.unwrap_or(DEFAULT_PLACEHOLDER) != placeholder {
            self.composer_ui.placeholder = Some(placeholder);
            self.composer.update(cx, |input, cx| {
                input.set_placeholder(placeholder, window, cx)
            });
        }
        let slash_rows = self.slash_menu(cx);
        let draft = !empty || !draft_files.is_empty();
        // r8-conversation (R8.4): while a decision takes the slot the
        // composer is hidden (`composer_hidden`); its input is the card's
        // reply. This field draws only when nothing waits there.
        let state = if !enabled {
            system::ComposerState::Disabled
        } else if focused {
            system::ComposerState::Focus
        } else if deciding {
            system::ComposerState::Yielded
        } else {
            system::ComposerState::Rest
        };

        // Toolbar, left: attach, then the session's context as quiet chips.
        let attach = primitives::icon_button(
            "attach-file",
            IconName::Plus,
            if uploading {
                "Attaching file…"
            } else {
                "Attach a file (or paste an image)"
            },
            ButtonVariant::Tertiary,
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
                // r7-qa (R7.4): one name per project, the breadcrumb's.
                let name = self.project_name(&cwd);
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
                        "codex" => wks_native::format::agent_name("codex").to_owned(),
                        _ => wks_native::format::agent_name("claude").to_owned(),
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
                // r2-shell: the model popover hangs from this chip.
                d.on_click(cx.listener(|this, _, window, cx| this.open_model_popover(window, cx)))
                    .child(chrome::measure(
                        cx.entity().downgrade(),
                        |this: &mut Self, bounds, _| {
                            let changed = this.shell.model_anchor != bounds;
                            this.shell.model_anchor = bounds;
                            changed
                        },
                    ))
            })
        });
        let access = selected
            .as_ref()
            .and_then(|s| self.render_access_chip_v2(s, narrow, cx));
        let meter = selected
            .as_ref()
            .filter(|_| !self.title_carries_context())
            .and_then(|s| gauge::context_text_v2(s, p));

        // Toolbar, right: what the session is doing, then the one primary
        // in its fixed place (Send / Queue / Stop · Esc).
        let status = self.composer_status_v2(layout, &decisions, narrow);
        let slot = send_slot(
            working,
            steering,
            deciding,
            !empty || !draft_files.is_empty(),
        );
        if slot != SendSlot::Queue {
            self.composer_ui.send_menu = false;
        }
        let send = self.render_send_control(slot, enabled, can_send && draft, deciding, cx);
        // r7-pages (R7.8): typing in a running session never hides Stop;
        // Queue appears beside it, in the primary's place at the end.
        let stop_beside = (slot == SendSlot::Queue && working)
            .then(|| self.render_send_control(SendSlot::Stop, enabled, false, false, cx));

        let mut leading = vec![attach.into_any_element()];
        leading.extend(folder.map(IntoElement::into_any_element));
        leading.extend(model.map(IntoElement::into_any_element));
        leading.extend(access.map(IntoElement::into_any_element));
        leading.extend(meter.map(IntoElement::into_any_element));
        let mut trailing: Vec<AnyElement> = status
            .into_iter()
            .map(IntoElement::into_any_element)
            .collect();
        trailing.extend(stop_beside);
        trailing.push(send);
        let toolbar = system::composer_toolbar(leading, trailing, &p);

        let body = system::composer_body(state, compact, &p)
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
                system::composer_text(1., &p).child(
                    Input::new(&self.composer)
                        .appearance(false)
                        .disabled(self.view.busy),
                ),
            )
            .child(toolbar);
        // The slash menu (R5.4) never covers a pending decision: with one
        // waiting it sits in the dock's flow above the composer, pushing
        // the decision up (the dock's cards shrink and scroll); otherwise it
        // is anchored above the composer (`system::anchored_menu`).
        let slash = (!slash_rows.is_empty()).then(|| {
            primitives::menu(&p)
                .debug_selector(|| "slash-menu-layer".into())
                .occlude()
                .map(|d| {
                    if deciding {
                        d.w_full()
                    } else {
                        d.w(self
                            .composer_dock_bounds
                            .size
                            .width
                            .min(px(t.system().overlay_width)))
                    }
                })
                .child(self.render_slash_menu(slash_rows, cx))
        });
        let composer = system::composer_field(state, &p)
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
            .child(body);
        let (pushed, anchored) = if deciding {
            (slash, None)
        } else {
            (None, slash)
        };
        div()
            .debug_selector(|| "chat-composer-group".into())
            .w_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap(px(t.space.md))
            .children(self.render_cold_note(cx))
            .children(pushed)
            .child(
                system::anchored_menu(
                    div().relative().w_full().child(composer),
                    anchored,
                    system::MenuAnchor::AboveStart,
                    &p,
                )
                .w_full(),
            )
    }

    /// R6.4: the access chip beside the model chip ("Sonnet · Ask first"):
    /// the mode in force (its default until the agent reports one), Full
    /// access in the caution tone. It opens the access choices; a choice
    /// applies at once, with no Apply. Hidden on a narrow composer unless
    /// the session runs with full access.
    fn render_access_chip_v2(
        &mut self,
        session: &Session,
        narrow: bool,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        if !self.supported_session() {
            self.composer_ui.access_menu = false;
            return None;
        }
        let p = self.palette();
        let provider = session.provider_id().to_owned();
        let choices = Permission::choices(&provider);
        let current = Permission::from_wire(&provider, session.access_mode());
        let shown = super::model_popover::effective_access(current, choices)?;
        let full = shown == Permission::FullAccess;
        if narrow && !full {
            self.composer_ui.access_menu = false;
            return None;
        }
        let enabled = self.view.connected && !self.view.busy;
        let open = self.composer_ui.access_menu && enabled;
        let chip = system::access_chip("composer-access", shown.word(), full, enabled, &p)
            .debug_selector(|| "composer-access".into())
            .tooltip(primitives::tooltip_view(
                format!(
                    "Access · {}",
                    super::model_popover::access_consequence(shown)
                ),
                None,
                &p,
            ))
            .when(enabled, |d| {
                d.on_click(cx.listener(|this, _, _, cx| {
                    this.composer_ui.access_menu = !this.composer_ui.access_menu;
                    cx.notify();
                }))
            });
        let menu = open.then(|| {
            primitives::menu(&p)
                .debug_selector(|| "composer-access-menu".into())
                .occlude()
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.composer_ui.access_menu = false;
                    cx.notify();
                }))
                .children(choices.iter().copied().map(|permission| {
                    let id = SharedString::from(format!("composer-access-{}", permission.word()));
                    let active = permission == shown;
                    // The shown default can still be set while unreported.
                    let settable = !active || current.is_none();
                    let provider = provider.clone();
                    system::access_option(
                        id.clone(),
                        permission.word(),
                        super::model_popover::access_consequence(permission),
                        active,
                        permission == Permission::FullAccess,
                        true,
                        &p,
                    )
                    .debug_selector(move || id.to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.composer_ui.access_menu = false;
                        if settable && let Ok(mode) = permission.wire(&provider) {
                            this.extras.access_switching = true;
                            this.act(
                                Action::SetPermission {
                                    mode: mode.to_owned(),
                                    label: permission.word().to_owned(),
                                },
                                cx,
                            );
                        }
                        cx.notify();
                    }))
                }))
        });
        Some(system::anchored_menu(
            chip,
            menu,
            system::MenuAnchor::AboveStart,
            &p,
        ))
    }

    // r4-truth (R4.1)
    /// What the composer says from the selected session's one status, when
    /// the turn's own clock does not: a stall (as the header says it; a
    /// streaming reply never is one), or a turn that ended while its
    /// subagents still work ("2 subagents working"). `live` is the turn
    /// clock running.
    pub(super) fn composer_state(&self, live: bool) -> Option<(StatusKind, String)> {
        let now = timing::now_ms();
        let session = self
            .selected_session()
            .filter(|_| self.view.connected && self.view.child.is_none())?;
        match self.session_status(session, now).kind {
            StatusKind::NotResponding => {
                let label = self
                    .stalled_for(session, now)
                    .map(|ms| {
                        format!(
                            "{} · {}",
                            wks_native::format::NOT_RESPONDING,
                            super::attention::quiet_for(ms)
                        )
                    })
                    .unwrap_or_else(|| wks_native::format::NOT_RESPONDING.into());
                Some((StatusKind::NotResponding, label))
            }
            StatusKind::Working if !live && !session.working() && !session.starting() => {
                // The sidebar's own words for this tree ("4 subagents · 3
                // working"), never a count of its own.
                let index = self.view.sessions.iter().position(|s| s.id == session.id)?;
                self.subagents_line(&self.subtree(index), now)
                    .filter(|(line, _)| line.ends_with("working"))
                    .map(|(line, _)| (StatusKind::Working, line))
            }
            _ => None,
        }
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
        if let Some((kind, label)) = self.composer_state(activity.live) {
            return Some(badge(kind, label));
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

    /// The composer's one primary, in one place (R3.2): "Send" when idle;
    /// "Queue" while the agent works and there is a draft (it runs after
    /// the current step), with Send now and After this turn in its menu;
    /// "Stop · Esc" while it works and nothing is drafted. A reply written
    /// while a decision waits sends as a secondary: the card owns the
    /// primary.
    fn render_send_control(
        &self,
        slot: SendSlot,
        enabled: bool,
        can_send: bool,
        deciding: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let (label, tip) = send_copy(slot);
        // A reply while a decision waits does not answer it.
        let tip = if deciding && slot == SendSlot::Send {
            super::decision::REPLY_KEEPS_PENDING
        } else {
            tip
        };
        let label = if self.view.busy { "Sending…" } else { label };
        match slot {
            SendSlot::Stop => {
                // "Stop · Esc", the key as a faint hint; with modal keys on
                // Esc means Normal mode, so the hint is not offered.
                let key = self.escape_stops().then_some("Esc");
                match key {
                    // r5-system (R5.2): Stop interrupts, so it is
                    // Destructive, never the brand-blue Primary.
                    Some(key) => primitives::keyed_button(
                        "stop",
                        key,
                        label,
                        system::stop_variant(),
                        ButtonSize::Small,
                        enabled,
                        &p,
                    ),
                    None => primitives::button(
                        "stop",
                        label,
                        None,
                        system::stop_variant(),
                        ButtonSize::Small,
                        enabled,
                        &p,
                    ),
                }
                .debug_selector(|| "composer-stop".into())
                .tooltip(primitives::tooltip_view(tip, key, &p))
                .when(enabled, |d| {
                    d.on_click(cx.listener(|this, _, _, cx| this.act(Action::Stop, cx)))
                })
                .into_any_element()
            }
            SendSlot::Send => {
                // R5.2/R5.4: Send is the Primary exactly when it can send,
                // and the disabled Primary otherwise; while a decision waits
                // the card owns the Primary and Send is Secondary.
                let (variant, enabled) = system::composer_send_state(can_send, deciding);
                primitives::button(
                    "send",
                    label,
                    Some(IconName::ArrowUp),
                    variant,
                    ButtonSize::Small,
                    enabled,
                    &p,
                )
                .debug_selector(|| "composer-send".into())
                .tooltip(primitives::tooltip_view(tip, Some(self.send_key()), &p))
                .when(can_send, |d| {
                    d.on_click(
                        cx.listener(|this, _, window, cx| this.send(&SendMessage, window, cx)),
                    )
                })
                .into_any_element()
            }
            SendSlot::Queue => {
                let (main, chevron) = primitives::split_button_parts(
                    "send",
                    label,
                    Some(IconName::ArrowUp),
                    ButtonVariant::Primary,
                    ButtonSize::Small,
                    can_send,
                    &p,
                );
                let main =
                    main.debug_selector(|| "composer-send".into())
                        .tooltip(primitives::tooltip_view(tip, Some(self.send_key()), &p))
                        .when(can_send, |d| {
                            d.on_click(cx.listener(|this, _, window, cx| {
                                this.submit(Submit::Send, window, cx)
                            }))
                        });
                let chevron =
                    chevron
                        .debug_selector(|| "composer-send-menu".into())
                        .when(can_send, |d| {
                            d.on_click(cx.listener(|this, _, _, cx| {
                                this.composer_ui.send_menu = !this.composer_ui.send_menu;
                                cx.notify();
                            }))
                        });
                // r6-surfaces (R6.1): the menu hangs from Queue itself
                // (above it, right edges aligned), not from the composer's
                // top, so it stays clear of the transcript's last message
                // and its timestamp.
                let menu = self
                    .composer_ui
                    .send_menu
                    .then(|| self.render_send_menu(can_send, cx));
                primitives::split_button_with_menu_at(
                    main,
                    chevron,
                    menu,
                    system::MenuAnchor::AboveEnd,
                    &p,
                )
                .into_any_element()
            }
        }
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

    /// Queue's menu: the other ways to deliver a draft mid-turn (Send now
    /// interrupts, After this turn holds it), each with a line saying what
    /// it does. Queue itself is what the send key does while the agent works.
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
                                    d.child(div().flex().gap(px(t.space.xxs)).children(
                                        key.split_whitespace().map(|k| {
                                            primitives::keycap_hint(k.to_owned(), false, &p)
                                        }),
                                    ))
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
        div().child(
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
                        "composer-send-now",
                        IconName::ArrowUp,
                        "Send now",
                        SEND_NOW_COPY,
                        Some(self.send_now_key()),
                    )
                    .when(can_send, |d| {
                        d.on_click(cx.listener(|this, _, window, cx| {
                            this.composer_ui.send_menu = false;
                            this.submit(Submit::Now, window, cx)
                        }))
                    }),
                )
                .child(
                    // (v2 leaves Tab to the text: no key here.)
                    item(
                        "composer-queue",
                        IconName::Inbox,
                        "After this turn",
                        AFTER_TURN_COPY,
                        None,
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

    /// The host sessions run on, for a decision's where line: the hub's
    /// machine by name once the Machines snapshot named it, else "this
    /// machine".
    pub(super) fn decision_host(&self) -> String {
        self.host_snapshot()
            .and_then(|snapshot| {
                wks_native::hosts::cards(&snapshot, &self.view.hosts_live)
                    .into_iter()
                    .find(|card| card.local)
                    .and_then(|card| {
                        card.name
                            .strip_prefix("This machine · ")
                            .map(|name| format!("{name} (this machine)"))
                    })
            })
            // r4-integration: before the Machines snapshot loads, the hub has
            // often named itself already: a live `host.resources` sample or
            // the New-session `queue.targets` read.
            .or_else(|| {
                self.view
                    .hosts_live
                    .resources
                    .get(wks_native::hosts::LOCAL)
                    .map(|r| r.host.clone())
                    .or_else(|| {
                        self.launch_targets().and_then(|t| {
                            t.targets
                                .into_iter()
                                .find(|t| t.local)
                                .and_then(|t| t.resources)
                                .map(|r| r.host)
                        })
                    })
                    .filter(|name| !name.trim().is_empty())
                    .map(|name| format!("{} (this machine)", name.trim()))
            })
            .unwrap_or_else(|| "this machine".to_owned())
    }

    /// A pending tool approval as the R4.3 decision, in the composer's slot:
    /// the request as a question ("Run `cargo test` in native?"), why, where
    /// (folder and host), a long command whole, and the numbered actions:
    /// 1 Allow primary and 2 Deny (Destructive) side by side, 3 Always allow
    /// (its exact rule, Tertiary) below, and Deny with note on 4, a
    /// visible Text button (R7.1). Details expands the full request. Keys show while this is the decision they answer.
    pub(super) fn render_approval_strip(
        &self,
        approval: &Value,
        layout: ChatLayout,
        decisions: &[Decision],
        cx: &mut Context<Self>,
    ) -> Div {
        use super::decision::{Choice, Details, Frame};
        let p = self.palette();
        let (compact, enabled) = (layout.compact, layout.enabled);
        let session = self.selected_session();
        let cwd = session.map(|s| s.cwd.clone()).unwrap_or_default();
        let host = self.decision_host();
        let card = session.and_then(Session::card);
        let always = wks_native::session_controls::always_options(approval);
        let note = self.deny_note_offered() && !self.extras.deny_open;
        let number = |key: &StripKey| key.number();
        // r6-decisions (R6.3): a request that deletes files takes a
        // second press.
        let risk = super::decision::risk(approval, &cwd);
        let confirm = super::decision::confirm_key(
            session.map(|s| s.id.as_str()).unwrap_or(""),
            card.as_deref(),
        );
        let allow = {
            let card = card.clone();
            let confirm = confirm.clone();
            Choice::new("approve", "Allow")
                .key(number(&StripKey::Allow))
                .tooltip("Allow this once")
                .enabled(enabled)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.confirm_allow(&confirm, risk, cx) {
                        this.act(Action::Approve(true, card.clone()), cx)
                    }
                }))
        };
        // One grant per option the card offers, each with its exact rule
        // and scope beside it (r5-fleet: the DecisionCard's Grant).
        let command = approval_summary(approval).1;
        let grants = always
            .into_iter()
            .enumerate()
            .map(|(ix, option)| {
                let card = card.clone();
                let (_, scope) = always_copy(&option.label, approval);
                let words = option.label.clone();
                let choice = Choice::new(format!("always-option-{ix}"), "Always allow")
                    .key(number(&StripKey::Always(ix)))
                    .tooltip(scope)
                    .enabled(enabled && card.is_some())
                    .on_click(cx.listener(move |this, _, _, cx| {
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
                    }));
                super::decision::Grant::new(choice, &words, &command)
            })
            .collect();
        let deny = {
            let card = card.clone();
            Choice::new("deny", "Deny")
                .key(number(&StripKey::Deny))
                .tooltip("Deny this request")
                .enabled(enabled)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.act(Action::Approve(false, card.clone()), cx)
                }))
        };
        // R7.1: Deny with note is a visible Text button on slot 4.
        let deny_note = note.then(|| {
            let card = card.clone();
            Choice::new("deny-with-note", "Deny with note")
                .key(number(&StripKey::DenyWithNote))
                .tooltip("Deny, and tell the agent what to do instead")
                .enabled(enabled)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_deny_note(card.clone(), window, cx)
                }))
        });
        let details_open = self.extras.approval_details;
        let details = Details::facts(
            Choice::new("approval-toggle", "Details").on_click(cx.listener(|this, _, _, cx| {
                this.extras.approval_details = !this.extras.approval_details;
                cx.notify();
            })),
            details_open,
            "approval-details",
            "approval-fact",
            approval_facts(approval, &cwd, &host),
        );
        let mut decision =
            super::decision::Decision::new("approval-card", allow, Frame::Card { compact });
        decision.headline = approval_headline(approval, &cwd);
        decision.headline_selector = Some("approval-summary".into());
        decision.reason = approval_context(approval);
        decision.request = approval_request(approval);
        decision.request_selector = "approval-request".into();
        decision.folder = Some(cwd.clone()).filter(|c| !c.is_empty());
        decision.host = Some(host);
        decision.pager = position_label(decisions, Decision::Approval);
        decision.details = Some(details);
        decision.note = self.extras.deny_open.then(|| {
            self.render_deny_note_v2(
                enabled,
                "Deny and tell it",
                // r8-qa (R8.2): a denial is the neutral Bordered tier, the
                // same as the card's own Deny; never red, never blue.
                system::actions::tier(system::actions::Verb::Deny),
                cx,
            )
            .into_any_element()
        });
        // While a note is being written, sending it is the primary.
        decision.allow_primary = !self.extras.deny_open;
        decision.grants = grants;
        decision.risk = risk;
        decision.armed = self.armed(&confirm);
        decision.deny = Some(deny);
        decision.deny_note = deny_note;
        decision.command = approval
            .pointer("/toolInput/command")
            .and_then(Value::as_str)
            .map(|c| c.trim().to_owned())
            .filter(|c| !c.is_empty());
        decision.access = session.map(super::decision::access_line);
        decision.verification = self.decision_verification(session);
        // r8-decisions (R8.2): keycaps only after keyboard use.
        decision.keys = self
            .keycaps(session.is_some_and(|s| self.keyed_decision(s) == Some(Decision::Approval)));
        // R6.3: the keycaps stay (no layout shift) but dim while the digits
        // type into the composer instead of answering.
        decision.keys_live = self.composer_ui.digits_to_card;
        // R8.4: the card and the reply are one component.
        decision.reply = self.reply_instead(enabled, cx);
        decision.reply_field = self.render_card_reply(enabled, cx);
        decision.render(&p)
    }

    /// R7.3: the session's latest test result, for its decision card.
    fn decision_verification(&self, session: Option<&Session>) -> Option<AnyElement> {
        let v = self.session_verification(session?)?;
        let p = self.palette();
        Some(system::verification_line(&v, timing::now_ms(), &p).into_any_element())
    }

    /// The decision the number keys answer (R4.3: their keycaps show only
    /// there): the approval or plan, else a question set.
    pub(super) fn keyed_decision(&self, session: &Session) -> Option<Decision> {
        match &session.approval {
            Some(approval) if plan_text(approval).is_some() => Some(Decision::Plan),
            Some(_) => Some(Decision::Approval),
            None => session.questions.is_some().then_some(Decision::Questions),
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
        variant: ButtonVariant,
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
                    ButtonVariant::Tertiary,
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
                // r5-system (R5.2): a denial is never blue (R8.2: it is
                // neutral Bordered); "Keep planning" stays the Primary.
                primitives::button(
                    "deny-note-send",
                    send,
                    None,
                    variant,
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

    /// Number keys on a pending approval: 1 Allow, 2 Deny, then each
    /// "Always" grant, then Deny with note. They act only when no text field would take the
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
            // R8.4: a folded card answers nothing until it is open again.
            || self.decision_collapsed()
        {
            return;
        }
        let k = &event.keystroke;
        // R7.1: Shift+1 is the positive answer's variant (a plan's
        // "Approve · run with Codex"); other modified keys are not ours.
        let variant = super::decision::is_variant_key(k);
        if k.modifiers.modified() && !variant {
            return;
        }
        let n = if variant {
            0
        } else {
            let Ok(n) = k.key.parse::<usize>() else {
                return;
            };
            n
        };
        // R6.3: digits answer only while no text field has focus; in the
        // composer they type (the hint and keycaps say which).
        if !self.digits_free(window, cx) {
            return;
        }
        let Some(session) = self.selected_session().cloned() else {
            return;
        };
        if session
            .approval
            .as_ref()
            .is_some_and(|a| plan_text(a).is_some())
        {
            let (handoff, can_hand_off) = self.plan_handoff(Some(&session));
            let keys = plan_keys(handoff.is_some(), true);
            let wanted = |k: &&PlanKey| {
                if variant {
                    **k == PlanKey::Handoff
                } else {
                    k.number() == Some(n)
                }
            };
            let Some(key) = keys.iter().find(wanted) else {
                return;
            };
            cx.stop_propagation();
            let card = session.card();
            match key {
                PlanKey::Approve => self.act(Action::Approve(true, card), cx),
                PlanKey::Handoff if can_hand_off => {
                    self.act(Action::Approve(true, card), cx);
                    self.open_feature(Screen::Handoff, window, cx);
                }
                PlanKey::Handoff => {}
                PlanKey::KeepPlanning => self.open_deny_note(card, window, cx),
            }
            return;
        }
        let Some(approval) = session.approval.as_ref() else {
            // r2-systems: with no approval, number keys answer the question
            // the stepper shows.
            if session.questions.is_some() && self.question_number_key(n, window, cx) {
                cx.stop_propagation();
            }
            return;
        };
        let always = wks_native::session_controls::always_options(approval);
        let Some(key) = strip_key_at(n, always.len(), self.deny_note_offered()) else {
            return;
        };
        cx.stop_propagation();
        let card = session.card();
        match key {
            StripKey::Allow => {
                let confirm = super::decision::confirm_key(&session.id, card.as_deref());
                let risk = super::decision::risk(approval, &session.cwd);
                if self.confirm_allow(&confirm, risk, cx) {
                    self.act(Action::Approve(true, card), cx)
                }
            }
            StripKey::Deny => self.act(Action::Approve(false, card), cx),
            StripKey::DenyWithNote => self.open_deny_note(card, window, cx),
            StripKey::Always(ix) => {
                if let (Some(option), Some(card)) = (always.get(ix), card) {
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
            "codex" => wks_native::format::agent_name("codex"),
            _ => wks_native::format::agent_name("claude"),
        };
        let when = resets_at
            .map(|at| {
                let now = wks_native::timing::now_ms() / 1000;
                format!("resets in {}", wks_native::usage::resets_in(at, now))
            })
            .unwrap_or_else(|| "resets later".into());
        Some(format!("{provider}'s {window} limit is reached · {when}"))
    }

    /// Who a plan waiting under a spent limit could be handed to, and
    /// whether that is possible now.
    fn plan_handoff(&self, session: Option<&Session>) -> (Option<&'static str>, bool) {
        let limited = session.is_some_and(|s| self.decisions(s).contains(&Decision::Limit));
        let other = limited.then(|| self.other_agent()).flatten();
        let can = self.supported_session() && self.view.connected && !self.demo;
        (other, can)
    }

    /// Claude finished planning: the plan as the R4.3 decision. A spent
    /// usage limit is part of the same card: Approve plan stays the primary
    /// (it waits for the reset), "Approve · run with Codex" is the
    /// secondary, Keep planning (feedback, the plan's Deny with note) is
    /// secondary too. Numbered like an approval.
    pub(super) fn render_plan_card_v2(
        &self,
        plan: &str,
        layout: ChatLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        use super::decision::{Choice, Frame};
        let p = self.palette();
        let t = p.tokens;
        let (compact, enabled) = (layout.compact, layout.enabled);
        let session = self.selected_session();
        let decisions = session.map(|s| self.decisions(s)).unwrap_or_default();
        let card = session.and_then(Session::card);
        let limited = decisions.contains(&Decision::Limit);
        let (handoff_to, can_hand_off) = self.plan_handoff(session);
        let deny_open = self.extras.deny_open;
        let number = |key: PlanKey| key.number();
        let agent = session
            .map(|s| match s.provider_id() {
                "codex" => wks_native::format::agent_name("codex"),
                _ => wks_native::format::agent_name("claude"),
            })
            .unwrap_or("the agent");
        let context = match session
            .and_then(|s| self.limit_context(s))
            .filter(|_| limited)
        {
            Some(limit) => format!("{limit} · approving waits for it"),
            None => format!("Nothing runs until you approve · {agent} starts on approval"),
        };
        let approve = |id: &'static str, label: String, card: Option<String>, handoff: bool| {
            Choice::new(id, label)
                .enabled(enabled && (!handoff || can_hand_off))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.act(Action::Approve(true, card.clone()), cx);
                    if handoff {
                        this.open_feature(Screen::Handoff, window, cx);
                    }
                }))
        };
        let allow = approve("plan-approve", "Approve plan".into(), card.clone(), false)
            .key(number(PlanKey::Approve));
        let mut decision =
            super::decision::Decision::new("plan-card", allow, Frame::Card { compact });
        decision.headline = vec![("Start on this plan?".to_owned(), false)];
        decision.headline_selector = Some("plan-headline".into());
        decision.reason = Some(context);
        decision.folder = session.map(|s| s.cwd.clone()).filter(|c| !c.is_empty());
        decision.host = Some(self.decision_host());
        decision.pager = position_label(&decisions, Decision::Plan);
        // A feedback note being written owns the primary.
        decision.allow_primary = !deny_open;
        // r8-decisions (R8.3): at the limit, the hand-off runs now, so it
        // is the primary; Approve plan only waits.
        decision.runnable_first = limited && can_hand_off;
        decision.alternates = handoff_to
            .map(|other| {
                approve(
                    "plan-approve-handoff",
                    format!("Approve · run with {other}"),
                    card.clone(),
                    true,
                )
                .variant_key()
                .tooltip(format!(
                    "Approve, then hand this session to {other} instead of waiting"
                ))
            })
            .into_iter()
            .collect();
        // Keep planning removes nothing: Secondary, not Destructive.
        decision.deny_variant = ButtonVariant::Secondary;
        decision.deny = (!deny_open).then(|| {
            let card = card.clone();
            Choice::new("plan-revise", "Keep planning")
                .key(number(PlanKey::KeepPlanning))
                .tooltip("Send feedback and let it keep planning")
                .enabled(enabled)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_deny_note(card.clone(), window, cx)
                }))
        });
        decision.body = Some(
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
                .child(self.render_markdown_nested("approval-plan", plan, window, cx))
                .into_any_element(),
        );
        decision.note = deny_open.then(|| {
            self.render_deny_note_v2(enabled, "Keep planning", ButtonVariant::Primary, cx)
                .into_any_element()
        });
        decision.access = session.map(super::decision::access_line);
        decision.verification = self.decision_verification(session);
        // r8-decisions (R8.2): keycaps only after keyboard use.
        decision.keys =
            self.keycaps(session.is_some_and(|s| self.keyed_decision(s) == Some(Decision::Plan)));
        decision.keys_live = self.composer_ui.digits_to_card;
        // R8.4: the card and the reply are one component.
        decision.reply = self.reply_instead(enabled, cx);
        decision.reply_field = self.render_card_reply(enabled, cx);
        decision.render(&p)
    }

    /// The other provider the limit card would hand this session to.
    fn other_agent(&self) -> Option<&'static str> {
        let session = self.selected_session()?;
        Some(match session.provider_id() {
            "codex" => wks_native::format::agent_name("claude"),
            _ => wks_native::format::agent_name("codex"),
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
                    .map(|d| system::floating(d, &p))
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
            "codex" => wks_native::format::agent_name("codex"),
            _ => wks_native::format::agent_name("claude"),
        };
        let other = self
            .other_agent()
            .unwrap_or(wks_native::format::agent_name("codex"));
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
                                ButtonVariant::Primary,
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
                                ButtonVariant::Tertiary,
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
        // R4.3: Details is the whole request: the tool, every input in
        // full, the folder and the host, as words.
        let facts = approval_facts(&approval, "/home/dev/app", "studio");
        let labels: Vec<&str> = facts.iter().map(|(l, _, _)| l.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Tool",
                "Command",
                "Reason",
                "Timeout",
                "Run in background",
                "Folder",
                "Host"
            ]
        );
        assert!(facts.iter().all(|(_, v, _)| !v.contains('{')));
        // A command the headline cannot show whole shows in full under it,
        // and the headline names it without a cut code span.
        let long = json!({"toolName":"Bash","toolInput":{"command":"cargo test\n--all"}});
        assert_eq!(
            approval_request(&long).as_deref(),
            Some("cargo test\n--all")
        );
        assert_eq!(
            approval_headline(&long, "/home/dev/app"),
            [
                ("Run this command".to_owned(), false),
                ("in dev/app?".to_owned(), false)
            ]
        );
        assert_eq!(approval_request(&approval), None);
        assert_eq!(
            approval_facts(&json!({"toolInput":{"file_path":"a.rs"}}), "", ""),
            vec![
                ("Tool".into(), "Tool".into(), false),
                ("File".into(), "a.rs".into(), true)
            ]
        );
    }

    #[test]
    fn plan_reviews_are_numbered_like_approvals() {
        assert_eq!(
            plan_keys(true, true),
            [PlanKey::Approve, PlanKey::KeepPlanning, PlanKey::Handoff]
        );
        assert_eq!(plan_keys(true, false), [PlanKey::Approve, PlanKey::Handoff]);
        assert_eq!(plan_keys(false, false), [PlanKey::Approve]);
        // R7.1: the plan's slots are the approval's; the hand-off is Shift+1.
        assert_eq!(PlanKey::Approve.number(), Some(1));
        assert_eq!(PlanKey::KeepPlanning.number(), Some(2));
        assert_eq!(PlanKey::Handoff.number(), None);
    }

    #[test]
    fn always_allow_says_its_exact_scope() {
        let bash = json!({"toolName":"Bash","toolInput":{"command":"cargo test --locked"}});
        // Claude's prefix rule: every command starting with it.
        let (label, tip) = always_copy("Always allow Bash(cargo test:*) this session", &bash);
        assert_eq!(label, "Always allow cargo test commands this session");
        assert_eq!(
            tip,
            "Until this session ends, any command starting with \u{201c}cargo test\u{201d} runs without asking"
        );
        // An exact rule names the command.
        assert_eq!(
            always_copy("Always allow Bash(npm test) this session", &bash).0,
            "Always allow \u{201c}npm test\u{201d} this session"
        );
        // Codex's "this command" is the request's command.
        assert_eq!(
            always_copy("Always allow this command for this session", &bash).0,
            "Always allow \u{201c}cargo test --locked\u{201d} this session"
        );
        // Words without a scope get the one every offered grant has.
        assert_eq!(
            always_copy("Always allow cargo test", &bash).0,
            "Always allow cargo test this session"
        );
        // Other grants keep their words.
        assert_eq!(
            always_copy("Allow all edits this session", &bash).0,
            "Allow all edits this session"
        );
    }

    #[test]
    fn one_primary_changes_its_label() {
        // (working, steering, deciding, draft)
        assert_eq!(send_slot(false, false, false, false), SendSlot::Send);
        assert_eq!(send_slot(false, false, false, true), SendSlot::Send);
        assert_eq!(send_slot(true, true, false, false), SendSlot::Stop);
        assert_eq!(send_slot(true, true, false, true), SendSlot::Queue);
        // A message still in flight: nothing to stop yet.
        assert_eq!(send_slot(false, true, false, false), SendSlot::Queue);
        // A reply written while a decision waits.
        assert_eq!(send_slot(false, true, true, true), SendSlot::Send);
        assert_eq!(
            send_copy(SendSlot::Queue),
            ("Queue", "Runs after the current step")
        );
        assert_eq!(send_copy(SendSlot::Stop).0, "Stop");
    }

    #[test]
    fn strip_keys_keep_their_slots() {
        // R7.1: a missing Always allow leaves 3 empty; Deny with note is
        // still 4.
        assert_eq!(
            strip_keys(0, true),
            [StripKey::Allow, StripKey::Deny, StripKey::DenyWithNote]
        );
        assert_eq!(strip_key_at(3, 0, true), None);
        assert_eq!(strip_key_at(4, 0, true), Some(StripKey::DenyWithNote));
        // Only the first grant takes 3; a second has no key.
        assert_eq!(
            strip_keys(2, false),
            [StripKey::Allow, StripKey::Deny, StripKey::Always(0)]
        );
        assert_eq!(StripKey::Always(1).number(), None);
        assert_eq!(strip_key_at(4, 2, false), None);
        for (n, key) in [
            (1, StripKey::Allow),
            (2, StripKey::Deny),
            (3, StripKey::Always(0)),
            (4, StripKey::DenyWithNote),
        ] {
            assert_eq!(key.number(), Some(n));
            assert_eq!(strip_key_at(n, 1, true), Some(key));
        }
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
        assert_eq!(send_copy(SendSlot::Send).0, "Send");
        assert!(SEND_NOW_COPY.starts_with("Interrupt"));
        assert_eq!(composer_placeholder(None), DEFAULT_PLACEHOLDER);
        // The composer never repeats the card: it is the reply line.
        assert_eq!(
            composer_placeholder(Some(Decision::Approval)),
            REPLY_INSTEAD
        );
        assert_eq!(sentence_case("run_in_background"), "Run in background");
        assert_eq!(sentence_case("oldString"), "Old string");
    }

    // R8.4: the folded card reads its headline as one plain line.
    #[test]
    fn a_folded_card_reads_its_headline_as_words() {
        let runs = vec![
            ("Run".to_owned(), false),
            ("cargo test".to_owned(), true),
            ("in native".to_owned(), false),
            ("?".to_owned(), false),
        ];
        assert_eq!(headline_words(&runs), "Run cargo test in native?");
        assert_eq!(headline_words(&[]), "");
        assert_eq!(REPLY_ACTION, "Reply instead");
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
        // The reason; where it runs is the decision's own line (R4.3).
        assert_eq!(
            approval_context(&bash).as_deref(),
            Some("Run the tests before committing")
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
        assert_eq!(approval_context(&json!({})), None);
    }
}
