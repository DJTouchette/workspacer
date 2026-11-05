//! R5.4 The DecisionCard: the design system's one decision component
//! (`docs/design-system.md` → DecisionCard). A tool approval in the
//! conversation, a plan review, a question and a job proposal, in the
//! conversation, on a Fleet tile and in Jobs, all draw through
//! [`Decision::render`], so they look and read the same:
//!
//! - a question headline whose code spans wrap, never truncate;
//! - the agent's reason, the full request (a long or multi-line command,
//!   wrapped, in mono), where it would run (folder and host) and, when one
//!   is plain from the request, a risk cue ([`risk`]);
//! - Allow (Primary) and Deny (Destructive) side by side on one line,
//!   never stacked, even in a narrow Fleet tile;
//! - each "Always allow" as a short Tertiary button with the exact rule
//!   and scope it saves beside it ([`Grant`]), then Details, which expands
//!   *below* itself and shows only what the card does not already say;
//! - number keys as keycaps only while this decision takes them (the top
//!   decision in the dock, the focused Fleet tile).
//!
//! Callers describe the decision and hand in click handlers; this module
//! owns the layout, the button tiers and the keycaps. Design v2 only.
use super::primitives::{self, ButtonSize, ButtonVariant};
use super::system::actions;
use super::*;
use gpui::{AnyElement, ClickEvent, relative};
use std::rc::Rc;

/// The one submit label of every question card (the conversation's
/// question set, a Fleet tile's question).
pub(crate) const SUBMIT_ANSWER: &str = "Send answer";

/// A click handler, as `cx.listener(…)` builds one.
pub(crate) type OnClick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// A request value shows in full under the headline (rather than as a code
/// span in it) once it is longer than this or has several lines.
pub(crate) const HEADLINE_FITS: usize = 60;

/// Whether `value` is too long or too tall for the headline: it then shows
/// whole, wrapped, as the decision's request.
pub(crate) fn needs_request_line(value: &str) -> bool {
    let value = value.trim();
    value.contains('\n') || value.chars().count() > HEADLINE_FITS
}

/// The folded composer's tooltip (R4.6, R7.7): replying does not answer.
pub(crate) const REPLY_KEEPS_PENDING: &str =
    "Write to the agent instead. The request stays open until you answer it above.";

/// R7.1 The one key map. Every decision surface (the conversation's dock,
/// the sidebar, a Fleet tile, a Fleet table row, Jobs, a plan card, a
/// stalled subagent) numbers its answers the same way, whatever else the
/// card offers: 1 the positive answer (Allow, Approve, Nudge), 2 the "no"
/// (Deny, Keep planning, Stop), 3 Always allow (absent when not offered,
/// and then never reused), 4 Deny with note. Nothing else takes a number:
/// a variant of the positive answer (Allow all, Approve · run with Codex)
/// goes behind Shift+1 ([`VARIANT_KEY`]); other extras (View output) take
/// no key. A question's option list is the one exception: its options are
/// the answers, numbered as listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Slot {
    Positive,
    Deny,
    Always,
    DenyNote,
}

impl Slot {
    /// The slot's number key.
    pub(crate) fn key(self) -> usize {
        match self {
            Self::Positive => 1,
            Self::Deny => 2,
            Self::Always => 3,
            Self::DenyNote => 4,
        }
    }
}

/// R7.1: the keycap of the positive answer's variant (Allow all, Approve ·
/// run with Codex): Shift+1, never a number of its own.
pub(crate) const VARIANT_KEY: &str = "⇧1";

/// R7.1: whether a keystroke is the variant key (Shift+1; some layouts
/// report it as "!").
pub(crate) fn is_variant_key(k: &gpui::Keystroke) -> bool {
    let m = &k.modifiers;
    !(m.control || m.alt || m.platform || m.function) && ((m.shift && k.key == "1") || k.key == "!")
}

/// One action of a decision.
#[derive(Clone)]
pub(crate) struct Choice {
    /// Element id and debug selector.
    pub id: SharedString,
    pub label: SharedString,
    /// Its number key (1-based), drawn only while the decision takes keys.
    pub key: Option<usize>,
    /// R7.1: a non-number key (Shift+1 for a variant), drawn the same way.
    pub shortcut: Option<SharedString>,
    pub tooltip: Option<SharedString>,
    pub enabled: bool,
    pub on_click: Option<OnClick>,
}

impl Choice {
    pub(crate) fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            key: None,
            shortcut: None,
            tooltip: None,
            enabled: true,
            on_click: None,
        }
    }

    pub(crate) fn key(mut self, key: Option<usize>) -> Self {
        self.key = key;
        self
    }

    /// R7.1: the choice's fixed slot (and so its number key).
    pub(crate) fn slot(self, slot: Slot) -> Self {
        self.key(Some(slot.key()))
    }

    /// R7.1: the positive answer's variant: Shift+1, no number.
    pub(crate) fn variant_key(mut self) -> Self {
        self.key = None;
        self.shortcut = Some(VARIANT_KEY.into());
        self
    }

    /// The keycap it shows while its decision takes keys.
    fn cap(&self) -> Option<SharedString> {
        self.key
            .map(|k| SharedString::from(k.to_string()))
            .or_else(|| self.shortcut.clone())
    }

    pub(crate) fn tooltip(mut self, tip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tip.into());
        self
    }

    pub(crate) fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub(crate) fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }

    /// The button in `variant`, its keycap shown when `keys` is on: at
    /// full strength while the number keys go to this decision (`live`),
    /// dimmed while they go elsewhere (R6.3: keys show on every keyed
    /// decision, and where they go is visible).
    fn render(&self, variant: ButtonVariant, keys: bool, live: bool, p: &Palette) -> Stateful<Div> {
        let t = p.tokens;
        let size = ButtonSize::Small;
        let el = match self.cap().filter(|_| keys) {
            Some(key) if live => primitives::keyed_button(
                self.id.clone(),
                key,
                self.label.clone(),
                variant,
                size,
                self.enabled,
                p,
            ),
            Some(key) => primitives::button(
                self.id.clone(),
                self.label.clone(),
                None,
                variant,
                size,
                self.enabled,
                p,
            )
            .gap(px(t.space.md))
            .child(dim_keycap(
                key,
                variant == ButtonVariant::Primary && self.enabled,
                p,
            )),
            None => primitives::button(
                self.id.clone(),
                self.label.clone(),
                None,
                variant,
                size,
                self.enabled,
                p,
            ),
        };
        let id = self.id.clone();
        // A long label (an exact "Always allow" rule) wraps inside its
        // button rather than running out of a narrow tile.
        el.debug_selector(move || id.to_string())
            .h_auto()
            .min_h(px(size.height(p)))
            .py(px(t.space.xxs))
            .flex_shrink_0()
            .max_w_full()
            .text_align(gpui::TextAlign::Left)
            .line_height(relative(1.3))
            .when_some(self.tooltip.clone(), |d, tip| {
                d.tooltip(primitives::tooltip_view(tip, None, p))
            })
            .when_some(self.on_click.clone().filter(|_| self.enabled), |d, f| {
                d.on_click(move |e, w, cx| f(e, w, cx))
            })
    }
}

/// An "Always allow" grant (R5.4): a short Tertiary "Always allow" whose
/// exact rule and scope show beside it ("Saves `cargo test *` · this
/// session"), never a long Secondary button that outweighs Deny.
#[derive(Clone)]
pub(crate) struct Grant {
    pub choice: Choice,
    /// The rule as people read it (`cargo test *`).
    pub rule: String,
    /// The provider's exact rule (`Bash(cargo test:*)`), for Details.
    pub exact: String,
    /// How long it lasts ("this session").
    pub scope: String,
}

impl Grant {
    /// A grant from the provider's words ("Always allow Bash(cargo test:*)
    /// this session"); `command` names "this command" grants.
    pub(crate) fn new(choice: Choice, label: &str, command: &str) -> Self {
        let (rule, exact, scope) = grant_rule(label, command);
        Self {
            choice,
            rule,
            exact,
            scope,
        }
    }
}

/// What a grant's words save: (the rule as read, the exact rule, the
/// scope). Claude's `Bash(x:*)` allows every command starting with `x`
/// (read `x *`); a bare `Bash(x)` only `x`; other tools read "Tool
/// content". Every grant the hub offers lasts for this session.
pub(crate) fn grant_rule(label: &str, command: &str) -> (String, String, String) {
    const SESSION: &str = "this session";
    let label = label.trim();
    let body = label
        .strip_suffix(SESSION)
        .map(|b| b.trim_end())
        .map(|b| b.strip_suffix(" for").unwrap_or(b))
        .unwrap_or(label)
        .trim();
    let scope = SESSION.to_owned();
    if let Some(rest) = body.strip_prefix("Allow all ") {
        // The `setMode` grant: edits run without asking.
        return (format!("all {rest}"), body.to_owned(), scope);
    }
    let body = body.strip_prefix("Always allow ").unwrap_or(body).trim();
    if body.eq_ignore_ascii_case("this command") {
        let command = command.trim();
        let rule = if command.is_empty() { body } else { command };
        return (rule.to_owned(), rule.to_owned(), scope);
    }
    if let Some(dirs) = body.strip_prefix("access to ") {
        return (format!("files in {dirs}"), body.to_owned(), scope);
    }
    let read: Vec<String> = body
        .split(", ")
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(|rule| {
            match rule
                .strip_suffix(')')
                .and_then(|r| r.split_once('('))
                .filter(|(tool, content)| !tool.is_empty() && !content.is_empty())
            {
                Some(("Bash", content)) => match content.strip_suffix(":*") {
                    Some(prefix) => format!("{prefix} *"),
                    None => content.to_owned(),
                },
                Some((tool, content)) => format!("{tool} {content}"),
                None => rule.to_owned(),
            }
        })
        .collect();
    let rule = if read.is_empty() {
        body.to_owned()
    } else {
        read.join(", ")
    };
    (rule, body.to_owned(), scope)
}

/// A cheap risk cue (R5.4), derived from the request alone: what the
/// request would do that is hard to take back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Risk {
    Deletes,
    /// r6-qa: deletes git branches (`git branch -d`/`-D`), which a
    /// scheduled cleanup job does as often as a shell `rm`.
    DeletesBranches,
    OutsideProject,
    Network,
}

impl Risk {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Deletes => "Deletes files",
            Self::DeletesBranches => "Deletes branches",
            Self::OutsideProject => "Writes outside the project",
            Self::Network => "Uses the network",
        }
    }

    /// R6.3: a request that deletes files (or branches) is allowed only
    /// on a second press ("Press again to allow").
    pub(crate) fn confirms(self) -> bool {
        matches!(self, Self::Deletes | Self::DeletesBranches)
    }
}

/// Allow's label once a destructive request has been pressed once.
pub(crate) const PRESS_AGAIN: &str = "Press again to delete";

/// Allow's tooltip on a request that needs the second press.
pub(crate) const CONFIRM_TIP: &str = "It deletes something: press Allow (or 1) twice to allow it";

/// R6.3: the destructive request armed by a first Allow, by its key
/// ([`confirm_key`]). One at a time: arming another disarms it.
#[derive(Default)]
pub(crate) struct Arming {
    pub(crate) armed: Option<String>,
}

/// The arming key of `session`'s request `card` (or its session alone).
pub(crate) fn confirm_key(session: &str, card: Option<&str>) -> String {
    format!("{session}:{}", card.unwrap_or(""))
}

impl Workspace {
    /// Whether an Allow of the request `key` (with its `risk`) goes ahead
    /// now. A destructive request goes ahead on the second press; the
    /// first only arms it, so Allow says "Press again to allow".
    pub(crate) fn confirm_allow(
        &mut self,
        key: &str,
        risk: Option<Risk>,
        cx: &mut Context<Self>,
    ) -> bool {
        if !risk.is_some_and(Risk::confirms) {
            return true;
        }
        if self.arming.armed.as_deref() == Some(key) {
            self.arming.armed = None;
            return true;
        }
        self.arming.armed = Some(key.to_owned());
        cx.notify();
        false
    }

    /// R7.2: the tier of a side panel's main action (Commit, Ask agent to
    /// fix). While a decision waits on the selected session, the decision
    /// owns the one Filled button, so the panel's is Bordered.
    pub(crate) fn side_panel_variant(&self) -> ButtonVariant {
        let deciding = self
            .selected_session()
            .is_some_and(|s| self.decision_takes_composer(s));
        actions::region_variant(actions::Verb::Allow, !deciding)
    }

    /// Whether the request `key` waits for its second press.
    pub(crate) fn armed(&self, key: &str) -> bool {
        self.arming.armed.as_deref() == Some(key)
    }

    /// R6.3: number keys on the keyed decisions outside the conversation's
    /// dock and the Fleet: Jobs' first proposal (1 Approve, 2 Reject) and
    /// a stalled subagent's view (1 Nudge, 2 Stop). Only
    /// while nothing else is focused, so a digit never leaves a text field.
    pub(crate) fn page_decision_keys(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let k = &event.keystroke;
        if !self.design().is_v2()
            || self.new_session
            || k.modifiers.modified()
            || !self.focus.is_focused(window)
        {
            return;
        }
        let Ok(n) = k.key.parse::<usize>() else {
            return;
        };
        let taken = match self.screen {
            Screen::Jobs if !self.extras.job_runs_tab => {
                let all = self.job_rows().unwrap_or_default();
                match all.iter().find(|j| j.proposal()) {
                    Some(job) if n <= 2 => {
                        let job = job.clone();
                        self.job_proposal_key(&job, &all, n, cx);
                        true
                    }
                    _ => false,
                }
            }
            Screen::Conversation if self.view.child.is_some() => self.stall_key(n, cx),
            _ => false,
        };
        if taken {
            cx.stop_propagation();
        }
    }
}

/// The risk an approval carries, if one is plain from its tool and input:
/// a delete (`rm`, `git clean`, `git reset --hard`), a write outside the
/// session's folder, or network use (`curl`, `git push`, installs, web
/// tools). The first that applies, in that order.
pub(crate) fn risk(approval: &serde_json::Value, cwd: &str) -> Option<Risk> {
    let tool = approval
        .get("toolName")
        .or_else(|| approval.get("tool"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let input = approval
        .get("toolInput")
        .unwrap_or(&serde_json::Value::Null);
    let cwd = cwd.trim().trim_end_matches('/');
    let outside = |path: &str| {
        let path = path.trim();
        !cwd.is_empty()
            && (path.starts_with('/') || path.starts_with('~'))
            && !(path == cwd || path.starts_with(&format!("{cwd}/")))
    };
    match tool {
        "WebFetch" | "WebSearch" | "web_fetch" | "web_search" => return Some(Risk::Network),
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" | "apply_patch" => {
            let path = ["file_path", "path", "notebook_path"]
                .into_iter()
                .find_map(|k| input.get(k).and_then(serde_json::Value::as_str));
            return path.filter(|p| outside(p)).map(|_| Risk::OutsideProject);
        }
        _ => {}
    }
    let command = input.get("command").and_then(serde_json::Value::as_str)?;
    // Words, with shell separators as words of their own.
    let spaced = command
        .replace("&&", " ; ")
        .replace("||", " ; ")
        .replace(['|', ';', '\n', '(', ')'], " ; ");
    let words: Vec<&str> = spaced.split_whitespace().collect();
    // Each simple command's program and its first argument.
    let commands: Vec<(&str, Option<&str>)> = words
        .split(|w| *w == ";")
        .filter_map(|c| {
            let mut c = c
                .iter()
                .copied()
                .skip_while(|w| *w == "sudo" || w.contains('='));
            let program = c.next()?.rsplit('/').next()?;
            Some((program, c.find(|w| !w.starts_with('-'))))
        })
        .collect();
    let has = |program: &str, sub: Option<&str>| {
        commands
            .iter()
            .any(|(p, s)| *p == program && sub.is_none_or(|sub| *s == Some(sub)))
    };
    if ["rm", "rmdir", "unlink", "shred"]
        .iter()
        .any(|p| has(p, None))
        || has("git", Some("clean"))
        || (has("git", Some("reset")) && words.contains(&"--hard"))
        || (has("find", None) && words.contains(&"-delete"))
    {
        return Some(Risk::Deletes);
    }
    // Also behind `xargs` (`… | xargs git branch -d`).
    if words.windows(2).any(|w| w == ["git", "branch"])
        && words.iter().any(|w| matches!(*w, "-d" | "-D" | "--delete"))
    {
        return Some(Risk::DeletesBranches);
    }
    let redirected = words
        .windows(2)
        .any(|w| matches!(w[0], ">" | ">>" | "tee") && outside(w[1]))
        || words.iter().any(|w| {
            w.strip_prefix(">>")
                .or(w.strip_prefix('>'))
                .is_some_and(outside)
        });
    if redirected {
        return Some(Risk::OutsideProject);
    }
    let network = ["curl", "wget", "ssh", "scp", "rsync", "nc", "ftp", "fly"]
        .iter()
        .any(|p| has(p, None))
        || ["push", "pull", "fetch", "clone"]
            .iter()
            .any(|s| has("git", Some(s)))
        || [
            ("npm", "install"),
            ("npm", "i"),
            ("pnpm", "add"),
            ("pnpm", "install"),
            ("yarn", "add"),
            ("pip", "install"),
            ("pip3", "install"),
            ("cargo", "install"),
            ("gem", "install"),
            ("brew", "install"),
            ("apt", "install"),
            ("apt-get", "install"),
        ]
        .iter()
        .any(|(p, s)| has(p, Some(s)));
    network.then_some(Risk::Network)
}

/// The Details toggle and, while open, what it expands to. It opens
/// *below* its toggle and shows only what the card does not already say.
pub(crate) struct Details {
    pub toggle: Choice,
    pub open: bool,
    /// Free content (a job's spec).
    pub content: Option<AnyElement>,
    /// Or the request as fact rows, filtered against the card.
    pub facts: Option<Facts>,
}

/// Fact rows for [`Details`]: (label, value, monospace), drawn by
/// [`facts`] under `selector`, one `prefix-{ix}` per row.
pub(crate) struct Facts {
    pub selector: &'static str,
    pub prefix: &'static str,
    pub rows: Vec<(String, String, bool)>,
}

impl Details {
    /// Details that expand to `content` (built only while open).
    pub(crate) fn element(toggle: Choice, open: bool, content: Option<AnyElement>) -> Self {
        Self {
            toggle,
            open,
            content,
            facts: None,
        }
    }

    /// Details that expand to the request's facts, less what the card shows.
    pub(crate) fn facts(
        toggle: Choice,
        open: bool,
        selector: &'static str,
        prefix: &'static str,
        rows: Vec<(String, String, bool)>,
    ) -> Self {
        Self {
            toggle,
            open,
            content: None,
            facts: Some(Facts {
                selector,
                prefix,
                rows,
            }),
        }
    }
}

/// Where the decision is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Frame {
    /// The "waiting on you" card (conversation, Jobs).
    Card { compact: bool },
    /// Inside a container that already frames it (a Fleet tile).
    Inline,
    /// R6.3: a table row's actions alone (the Fleet table): the answer
    /// row, right-aligned, in the same tiers and with the same keys. The
    /// row's other cells say what is asked.
    Row,
}

/// A decision waiting on the user.
pub(crate) struct Decision {
    pub selector: SharedString,
    /// Runs of words and code spans (`true`).
    pub headline: Vec<(String, bool)>,
    pub headline_selector: Option<SharedString>,
    /// The agent's reason (or the proposal's summary).
    pub reason: Option<String>,
    /// The full request, wrapped in mono, when the headline cannot show it.
    pub request: Option<String>,
    pub request_selector: SharedString,
    /// R7.7: the whole command, for Details (the card may show it cut, or
    /// as a code span in the headline).
    pub command: Option<String>,
    /// R7.7: the session's access mode in words, for Details.
    pub access: Option<String>,
    pub folder: Option<String>,
    pub host: Option<String>,
    /// What the request would do that is hard to take back.
    pub risk: Option<Risk>,
    /// R7.3: the session's latest test result
    /// (`system::verification_line`), under the meta line.
    pub verification: Option<AnyElement>,
    /// "2/3" while more than one waits.
    pub pager: Option<String>,
    /// A body between the head and the actions (a plan, a question's
    /// options or answer field).
    pub body: Option<AnyElement>,
    pub details: Option<Details>,
    /// A note being written (Deny with note, Keep planning).
    pub note: Option<AnyElement>,
    pub allow: Choice,
    /// Allow is the Primary unless a note being written owns it.
    pub allow_primary: bool,
    /// R7.2: whether this decision owns its region's one Filled button
    /// (the focused Fleet card or row, the dock's decision). Off, its
    /// positive answer is Bordered.
    pub filled: bool,
    /// Whether Allow shows at all: off when the body answers (a
    /// question's options send on a click).
    pub show_allow: bool,
    /// Variants of the positive answer ("Approve · run with Codex", "Allow
    /// all 3"): Bordered, on Shift+1 (R7.1), never a number.
    pub alternates: Vec<Choice>,
    /// "Always allow" grants, each with its exact rule and scope. Only the
    /// first takes slot 3.
    pub grants: Vec<Grant>,
    pub deny: Option<Choice>,
    /// Deny's tier: Destructive (Deny, Stop), or Secondary for a "no"
    /// that removes nothing (Keep planning).
    pub deny_variant: ButtonVariant,
    /// R7.1: "Deny with note", a visible Text button on slot 4.
    pub deny_note: Option<Choice>,
    /// Whether this decision shows its keycaps: only the focused decision
    /// (R7.1: unfocused tiles show none).
    pub keys: bool,
    /// Whether the number keys go to it now (R6.3): its keycaps at full
    /// strength. Off, they show dimmed: it is the focused decision, but the
    /// keys type into the composer for now.
    pub keys_live: bool,
    /// R6.3: a destructive request ([`Risk::confirms`]) pressed once:
    /// Allow reads "Press again to delete", in red (R7.2).
    pub armed: bool,
    pub frame: Frame,
}

impl Decision {
    pub(crate) fn new(selector: impl Into<SharedString>, allow: Choice, frame: Frame) -> Self {
        Self {
            selector: selector.into(),
            headline: Vec::new(),
            headline_selector: None,
            reason: None,
            request: None,
            request_selector: "decision-request".into(),
            command: None,
            access: None,
            folder: None,
            host: None,
            risk: None,
            verification: None,
            pager: None,
            body: None,
            details: None,
            note: None,
            allow,
            allow_primary: true,
            filled: true,
            show_allow: true,
            alternates: Vec::new(),
            grants: Vec::new(),
            deny: None,
            deny_variant: ButtonVariant::Destructive,
            deny_note: None,
            keys: false,
            keys_live: true,
            armed: false,
            frame,
        }
    }

    /// R7.7: what Details shows: the whole command, the agent's stated
    /// reason, the access mode, and in words what each Always allow would
    /// let through (with the provider's exact rule), then whatever else
    /// the request carries that the card does not say (the tool, other
    /// arguments). The folder and host are on the card and stay out.
    pub(crate) fn detail_rows(
        &self,
        rows: Vec<(String, String, bool)>,
    ) -> Vec<(String, String, bool)> {
        let mut out: Vec<(String, String, bool)> = Vec::new();
        let command = self
            .command
            .clone()
            .or_else(|| self.request.clone())
            .or_else(|| {
                rows.iter()
                    .find(|(label, _, _)| label == "Command")
                    .map(|(_, v, _)| v.clone())
            })
            .filter(|c| !c.trim().is_empty());
        if let Some(command) = &command {
            out.push(("Command".into(), command.trim_end().to_owned(), true));
        }
        let reason = self
            .reason
            .clone()
            .or_else(|| {
                rows.iter()
                    .find(|(label, _, _)| label == "Reason")
                    .map(|(_, v, _)| v.clone())
            })
            .filter(|r| !r.trim().is_empty());
        if let Some(reason) = &reason {
            out.push(("Reason".into(), reason.trim().to_owned(), false));
        }
        if let Some(access) = self.access.clone().filter(|a| !a.trim().is_empty()) {
            out.push(("Access".into(), access, false));
        }
        for grant in &self.grants {
            let words = grant
                .choice
                .tooltip
                .as_ref()
                .map(|t| t.to_string())
                .unwrap_or_else(|| format!("{} runs without asking · {}", grant.rule, grant.scope));
            out.push(("Always".into(), words, false));
            if !grant.exact.is_empty() && grant.exact != grant.rule {
                out.push((
                    "Rule".to_owned(),
                    format!("{} · {}", grant.exact, grant.scope),
                    true,
                ));
            }
        }
        let shown: Vec<&str> = [self.folder.as_deref(), self.host.as_deref()]
            .into_iter()
            .flatten()
            .chain(command.as_deref())
            .chain(reason.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        out.extend(rows.into_iter().filter(|(label, value, _)| {
            !matches!(label.as_str(), "Command" | "Reason" | "Folder" | "Host")
                && !shown.contains(&value.trim())
        }));
        out
    }

    /// R7.1: the keys this decision draws (while `keys`), each with what it
    /// answers: the slot map a test holds every decision to.
    #[cfg(test)]
    pub(crate) fn key_map(&self) -> Vec<(String, Answer)> {
        if !self.keys {
            return Vec::new();
        }
        let noting = self.note.is_some();
        let mut out = Vec::new();
        let mut push = |choice: &Choice, answer: Answer| {
            if let Some(cap) = choice.cap() {
                out.push((cap.to_string(), answer));
            }
        };
        if self.show_allow && !noting {
            push(&self.allow, Answer::Positive);
        }
        if !noting {
            if let Some(deny) = &self.deny {
                push(deny, Answer::Deny);
            }
            for alternate in &self.alternates {
                push(alternate, Answer::Variant);
            }
            for (ix, grant) in self.grants.iter().enumerate() {
                push(
                    &grant.choice,
                    if ix == 0 {
                        Answer::Always
                    } else {
                        Answer::OtherGrant
                    },
                );
            }
            if let Some(note) = &self.deny_note {
                push(note, Answer::DenyNote);
            }
        }
        out
    }

    /// The tier of the positive answer as drawn (R7.2).
    fn allow_variant(&self) -> ButtonVariant {
        if self.armed && self.risk.is_some_and(Risk::confirms) {
            // Armed: red, "Press again to delete".
            ButtonVariant::Destructive
        } else if self.allow_primary && self.filled {
            ButtonVariant::Primary
        } else {
            ButtonVariant::Secondary
        }
    }

    pub(crate) fn render(mut self, p: &Palette) -> Div {
        let t = p.tokens;
        let keys = self.keys;
        let selector = self.selector.clone();
        #[cfg(test)]
        audit::record(&self);
        let headline = {
            let el = headline(&self.headline, p);
            match self.headline_selector.clone() {
                // (A wrapper: the headline keeps its own "headline-code".)
                Some(s) => div()
                    .w_full()
                    .min_w_0()
                    .debug_selector(move || s.to_string())
                    .child(el),
                None => el,
            }
        };
        let request = self.request.clone().map(|request| {
            let s = self.request_selector.clone();
            div()
                .debug_selector(move || s.to_string())
                .w_full()
                .min_w_0()
                .px(px(t.space.md))
                .py(px(t.space.xs))
                .rounded(px(t.radius.xs))
                .bg(rgb(p.code_inline))
                .font_family(mono_font())
                .text_size(px(t.text.mono))
                .line_height(relative(t.text.line_height))
                .text_color(rgb(p.text))
                .child(request.trim_end().to_owned())
        });
        // R7.7: three lines of metadata at most: the headline, the reason,
        // then one line with the risk chip and where it runs.
        let place = where_line(self.folder.as_deref(), self.host.as_deref());
        let place = (!place.is_empty()).then(|| {
            div()
                .debug_selector(|| "decision-where".into())
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(t.text.meta))
                .text_color(rgb(p.roles.text_tertiary))
                .child(place)
        });
        let risk = self.risk.map(|risk| risk_chip(risk, p));
        let meta = (place.is_some() || risk.is_some()).then(|| {
            div()
                .debug_selector(|| "decision-meta".into())
                .w_full()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(t.space.sm))
                .children(risk)
                .children(place)
        });
        let live = self.keys_live;
        let confirms = self.risk.is_some_and(Risk::confirms);
        if confirms {
            if self.armed {
                self.allow.label = PRESS_AGAIN.into();
            }
            self.allow.tooltip = Some(CONFIRM_TIP.into());
        }
        // Details: the full request in words (R7.7). Nothing to add, no
        // toggle.
        let details = self.details.take().and_then(|d| {
            let Details {
                toggle,
                open,
                content,
                facts: rows,
            } = d;
            let content = match rows {
                Some(Facts {
                    selector,
                    prefix,
                    rows,
                }) => {
                    let rows = self.detail_rows(rows);
                    if rows.is_empty() {
                        return None;
                    }
                    open.then(|| facts(selector, prefix, rows, p).into_any_element())
                }
                None => content.filter(|_| open),
            };
            Some((toggle, open, content))
        });
        let details_toggle = details.as_ref().map(|(toggle, open, _)| {
            let tip = if *open {
                "Hide the details"
            } else {
                "The whole command, why it asks, the access mode, and what Always allow would let through"
            };
            let id = toggle.id.clone();
            let control = primitives::button(
                toggle.id.clone(),
                toggle.label.clone(),
                Some(if *open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                }),
                actions::tier(actions::Verb::Details),
                ButtonSize::Small,
                toggle.enabled,
                p,
            )
            .debug_selector(move || id.to_string())
            .tooltip(primitives::tooltip_view(tip, None, p));
            match toggle.on_click.clone().filter(|_| toggle.enabled) {
                Some(f) => control
                    .on_click(move |e, w, cx| f(e, w, cx))
                    .into_any_element(),
                None => control.into_any_element(),
            }
        });
        let details = details.and_then(|(_, _, content)| content);
        let head = primitives::waiting_head(
            headline,
            self.reason
                .clone()
                .filter(|r| !r.trim().is_empty())
                .map(Into::into),
            self.pager.clone().map(Into::into),
            Vec::new(),
            p,
        );

        // R6.3: a note being written (Deny with note, Keep planning)
        // *replaces* the answer: its own send and Cancel are the only
        // buttons, never a second Allow / Deny row under it.
        let noting = self.note.is_some();
        // The answer: Allow and Deny side by side, never stacked; any
        // variant after them.
        let allow_variant = self.allow_variant();
        let allow =
            (self.show_allow && !noting).then(|| self.allow.render(allow_variant, keys, live, p));
        let deny = self.deny.as_ref().filter(|_| !noting).map(|deny| {
            div()
                .flex_shrink_0()
                .child(deny.render(self.deny_variant, keys, live, p))
        });
        let alternates: Vec<Stateful<Div>> = if noting {
            Vec::new()
        } else {
            self.alternates
                .iter()
                .map(|c| c.render(actions::tier(actions::Verb::AllowVariant), keys, live, p))
                .collect()
        };
        // R7.1: Deny with note, a visible Text button (slot 4).
        let deny_note = self
            .deny_note
            .as_ref()
            .filter(|_| !noting)
            .map(|c| c.render(actions::tier(actions::Verb::DenyWithNote), keys, live, p));
        let pair = (allow.is_some() || deny.is_some()).then(|| {
            div()
                .debug_selector(|| "decision-answer".into())
                .flex()
                .flex_nowrap()
                .items_center()
                .flex_shrink_0()
                .gap(px(t.space.sm))
                .children(allow)
                .children(deny)
        });
        if self.frame == Frame::Row {
            // The table row's slot: the answer alone, right-aligned.
            return div()
                .debug_selector(move || selector.to_string())
                .w_full()
                .flex()
                .flex_nowrap()
                .items_center()
                .justify_end()
                .gap(px(t.space.sm))
                .children(pair)
                .children(alternates)
                .children(deny_note);
        }
        let answer = (pair.is_some() || !alternates.is_empty()).then(|| {
            primitives::waiting_actions(p)
                .children(pair)
                .children(alternates)
        });
        // Then the quiet row: each grant with its exact rule and scope,
        // Deny with note, and Details (it decides nothing) at the end.
        let grants: Vec<Div> = if noting {
            Vec::new()
        } else {
            self.grants
                .iter()
                .map(|g| grant_row(g, keys, live, p))
                .collect()
        };
        let details_toggle = details_toggle.filter(|_| !noting);
        let quiet =
            (!grants.is_empty() || details_toggle.is_some() || deny_note.is_some()).then(|| {
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(t.space.sm))
                    .when(!grants.is_empty(), |d| {
                        d.child(
                            div()
                                .flex_1()
                                // Narrower than this, Details wraps below.
                                .min_w(px(t.system().tile_min * 0.75))
                                .flex()
                                .flex_col()
                                .gap(px(t.space.xxs))
                                .children(grants),
                        )
                    })
                    .children(deny_note.map(|d| div().flex_shrink_0().child(d)))
                    .children(details_toggle.map(|d| div().flex_shrink_0().child(d)))
            });

        let content = |d: Div| {
            d.child(head)
                .children(request)
                .children(meta)
                .children(self.verification)
                .children(self.body)
                .children(self.note)
                .children(answer)
                .children(quiet)
                .children(details.filter(|_| !noting))
        };
        match self.frame {
            Frame::Card { compact } => content(primitives::waiting_card(compact, p))
                .debug_selector(move || selector.to_string())
                .occlude(),
            Frame::Inline | Frame::Row => content(
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(t.space.sm)),
            )
            .debug_selector(move || selector.to_string()),
        }
    }
}

/// What a drawn key answers (R7.1's slot map, as [`Decision::key_map`]
/// reports it).
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    Positive,
    Deny,
    Always,
    /// A second or later "Always allow" (no key of its own).
    OtherGrant,
    DenyNote,
    /// A variant of the positive answer (Shift+1).
    Variant,
}

/// R7.1: whether a decision's drawn keys follow the one key map. The
/// problems, empty when it does.
#[cfg(test)]
pub(crate) fn slot_map_problems(map: &[(String, Answer)]) -> Vec<String> {
    let mut problems = Vec::new();
    for (cap, answer) in map {
        let want = match answer {
            Answer::Positive => Some(Slot::Positive.key().to_string()),
            Answer::Deny => Some(Slot::Deny.key().to_string()),
            Answer::Always => Some(Slot::Always.key().to_string()),
            Answer::DenyNote => Some(Slot::DenyNote.key().to_string()),
            Answer::Variant => Some(VARIANT_KEY.to_owned()),
            Answer::OtherGrant => None,
        };
        match want {
            Some(want) if *cap != want => {
                problems.push(format!("{answer:?} shows {cap}, the map says {want}"))
            }
            None => problems.push(format!("{answer:?} shows {cap}, it takes no key")),
            _ => {}
        }
    }
    problems
}

/// Tests: every decision drawn on this thread, so a test can hold all of
/// them to the key map and the one-Filled rule (R7.1, R7.2).
#[cfg(test)]
pub(crate) mod audit {
    use super::*;
    use std::cell::RefCell;

    #[derive(Clone, Debug)]
    pub(crate) struct Drawn {
        pub selector: String,
        pub frame: Frame,
        pub keys: Vec<(String, Answer)>,
        /// Filled (Primary) buttons it drew.
        pub filled: usize,
    }

    thread_local! {
        static DRAWN: RefCell<Vec<Drawn>> = const { RefCell::new(Vec::new()) };
    }

    pub(super) fn record(d: &Decision) {
        let noting = d.note.is_some();
        let filled =
            usize::from(d.show_allow && !noting && d.allow_variant() == ButtonVariant::Primary)
                + usize::from(
                    !noting && d.deny.is_some() && d.deny_variant == ButtonVariant::Primary,
                );
        DRAWN.with(|v| {
            v.borrow_mut().push(Drawn {
                selector: d.selector.to_string(),
                frame: d.frame,
                keys: d.key_map(),
                filled,
            })
        });
    }

    /// Forget what was drawn so far.
    pub(crate) fn clear() {
        DRAWN.with(|v| v.borrow_mut().clear());
    }

    /// Everything drawn since the last [`clear`].
    pub(crate) fn drawn() -> Vec<Drawn> {
        DRAWN.with(|v| v.borrow().clone())
    }
}

/// A keycap at the strength of keys that go elsewhere just now (R6.3).
fn dim_keycap(key: SharedString, on_fill: bool, p: &Palette) -> Div {
    primitives::keycap_hint(key, on_fill, p).opacity(p.tokens.wash.yielded)
}

/// R6.3: the risk cue as a chip: its glyph and words on a quiet wash.
/// "Deletes files" (the one that asks for a second press) in the
/// neutral-strong `caution`; the others in secondary text.
pub(crate) fn risk_chip(risk: Risk, p: &Palette) -> Div {
    let t = p.tokens;
    let tone = if risk.confirms() {
        p.roles.caution
    } else {
        p.roles.text_secondary
    };
    div()
        .debug_selector(|| "decision-risk".into())
        .flex()
        .flex_none()
        .items_center()
        .gap(px(t.space.xs))
        .px(px(t.space.md))
        .py(px(t.space.xxs))
        .rounded(px(t.radius.full))
        .bg(primitives::tonal(p, t.wash.rest))
        .text_size(px(t.text.meta))
        .font_weight(FontWeight(t.weight.medium))
        .text_color(rgb(tone))
        .child(Icon::new(IconName::TriangleAlert).size(px(t.icon.sm)))
        .child(risk.label())
}

/// "Always allow" (Tertiary, with its key) and, beside it, what it saves.
fn grant_row(grant: &Grant, keys: bool, live: bool, p: &Palette) -> Div {
    let t = p.tokens;
    let id = grant.choice.id.clone();
    // One text, so the rule wraps as prose in a narrow tile.
    let lead = "Saves ";
    let text = format!("{lead}\u{2009}{}\u{2009} · {}", grant.rule, grant.scope);
    let start = lead.len();
    let end = start + '\u{2009}'.len_utf8() * 2 + grant.rule.len();
    let code = gpui::HighlightStyle {
        color: Some(rgb(p.text).into()),
        background_color: Some(super::system::inline_code_bg(p)),
        ..Default::default()
    };
    div()
        .debug_selector(move || format!("{id}-row"))
        .w_full()
        .min_w_0()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(t.space.xs))
        .child(div().flex_shrink_0().child(grant.choice.render(
            actions::tier(actions::Verb::AlwaysAllow),
            keys,
            live,
            p,
        )))
        .child(
            div()
                .flex_1()
                // Narrower than this, the rule wraps under the button.
                .min_w(px(t.system().tile_min / 2.))
                .text_size(px(t.text.meta))
                .line_height(relative(t.text.line_height))
                .text_color(rgb(p.roles.text_tertiary))
                .child(gpui::StyledText::new(text).with_highlights([(start..end, code)])),
        )
}

/// R7.7: the session's access mode for Details, in words: "Ask first ·
/// Asks before it runs a command or edits a file."
pub(crate) fn access_line(session: &Session) -> String {
    // The mode the composer's access chip shows (a provider that has not
    // reported one runs at its default, Ask first, where it offers it).
    let provider = session.provider_id();
    let current = wks_native::launch::Permission::from_wire(provider, session.access_mode());
    let choices = wks_native::launch::Permission::choices(provider);
    match super::model_popover::effective_access(current, choices) {
        Some(mode) => format!(
            "{} · {}",
            mode.word(),
            super::model_popover::access_consequence(mode)
        ),
        None => super::capsule::access_word(session).0.to_owned(),
    }
}

/// "Folder /home/dev/src/app · on studio" (either part may be missing).
pub(crate) fn where_line(folder: Option<&str>, host: Option<&str>) -> String {
    let folder = folder.map(str::trim).filter(|f| !f.is_empty());
    let host = host.map(str::trim).filter(|h| !h.is_empty());
    match (folder, host) {
        (Some(folder), Some(host)) => format!("In {folder} · on {host}"),
        (Some(folder), None) => format!("In {folder}"),
        (None, Some(host)) => format!("On {host}"),
        (None, None) => String::new(),
    }
}

/// The decision's headline at `text.lead` 600: words and code spans that
/// wrap onto the next line instead of truncating (R4.3: a command is never
/// cut next to Allow).
pub(crate) fn headline(parts: &[(String, bool)], p: &Palette) -> Div {
    let t = p.tokens;
    let (text, ranges) = headline_text(parts);
    let has_code = !ranges.is_empty();
    let code = gpui::HighlightStyle {
        font_weight: Some(FontWeight(t.weight.medium)),
        background_color: Some(super::system::inline_code_bg(p)),
        ..Default::default()
    };
    // One text, so words and code wrap as prose (a flex row of pieces
    // collapses under intrinsic sizing).
    div()
        .w_full()
        .min_w_0()
        .text_size(px(t.text.lead))
        .line_height(relative(t.text.line_height))
        .font_weight(FontWeight(t.weight.semibold))
        .text_color(rgb(p.text))
        .when(has_code, |d| d.debug_selector(|| "headline-code".into()))
        .child(gpui::StyledText::new(text).with_highlights(ranges.into_iter().map(|r| (r, code))))
}

/// The headline's text and the byte ranges of its code spans (each padded
/// by a thin space inside its highlight), runs joined by spaces, with no
/// space before closing punctuation ("in native?").
pub(crate) fn headline_text(parts: &[(String, bool)]) -> (String, Vec<std::ops::Range<usize>>) {
    const PAD: char = '\u{2009}';
    let mut text = String::new();
    let mut ranges = Vec::new();
    for (part, code) in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let glued = !*code && part.starts_with(['?', '.', ',', '!', ':']);
        if !text.is_empty() && !glued {
            text.push(' ');
        }
        if *code {
            let start = text.len();
            text.push(PAD);
            text.push_str(part);
            text.push(PAD);
            ranges.push(start..text.len());
        } else {
            text.push_str(part);
        }
    }
    (text, ranges)
}

/// The expanded request: one row per fact (label, value, monospace) in the
/// card's inset, `prefix-{ix}` per row.
pub(crate) fn facts(
    selector: &'static str,
    prefix: &'static str,
    rows: Vec<(String, String, bool)>,
    p: &Palette,
) -> Div {
    let t = p.tokens;
    primitives::inset(p)
        .debug_selector(move || selector.into())
        .children(
            rows.into_iter()
                .enumerate()
                .map(|(ix, (label, value, mono))| {
                    div()
                        .debug_selector(move || format!("{prefix}-{ix}"))
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_says_where_it_runs() {
        assert_eq!(
            where_line(Some("/home/dev/app"), Some("studio")),
            "In /home/dev/app · on studio"
        );
        assert_eq!(where_line(Some(" "), Some("studio")), "On studio");
        assert_eq!(where_line(Some("/a"), None), "In /a");
        assert_eq!(where_line(None, None), "");
    }

    #[test]
    fn headlines_read_as_one_sentence() {
        let (text, ranges) = headline_text(&[
            ("Run".into(), false),
            ("cargo test".into(), true),
            ("in apps/native?".into(), false),
        ]);
        assert_eq!(text, "Run \u{2009}cargo test\u{2009} in apps/native?");
        assert_eq!(&text[ranges[0].clone()], "\u{2009}cargo test\u{2009}");
        let (text, _) = headline_text(&[
            ("Edit".into(), false),
            ("a.rs".into(), true),
            ("?".into(), false),
        ]);
        assert_eq!(text, "Edit \u{2009}a.rs\u{2009}?");
    }

    #[test]
    fn long_or_tall_requests_show_whole() {
        assert!(!needs_request_line("cargo test"));
        assert!(needs_request_line("cargo test\n--all"));
        assert!(needs_request_line(&"x".repeat(HEADLINE_FITS + 1)));
    }

    #[test]
    fn details_say_the_command_reason_access_and_what_always_lets_through() {
        let mut d = Decision::new("d", Choice::new("a", "Allow"), Frame::Inline);
        d.reason = Some("Run the tests".into());
        d.command = Some("cargo test --locked".into());
        d.access = Some("Ask first · Asks before it runs a command".into());
        d.folder = Some("/home/dev/app".into());
        d.grants = vec![Grant::new(
            Choice::new("g", "Always allow").tooltip(
                "Until this session ends, any command starting with “cargo test” runs without asking",
            ),
            "Always allow Bash(cargo test:*) this session",
            "",
        )];
        let rows = d.detail_rows(vec![
            ("Tool".into(), "Bash".into(), false),
            ("Command".into(), "cargo test --locked".into(), true),
            ("Folder".into(), "/home/dev/app".into(), true),
        ]);
        let labels: Vec<&str> = rows.iter().map(|(l, _, _)| l.as_str()).collect();
        assert_eq!(
            labels,
            ["Command", "Reason", "Access", "Always", "Rule", "Tool"]
        );
        assert!(rows[3].1.contains("any command starting with"));
    }

    #[test]
    fn the_slots_are_fixed() {
        assert_eq!(
            [Slot::Positive, Slot::Deny, Slot::Always, Slot::DenyNote].map(Slot::key),
            [1, 2, 3, 4]
        );
        assert_eq!(Choice::new("x", "Allow all").variant_key().key, None);
    }

    #[test]
    fn replying_says_the_request_stays_pending() {
        assert!(REPLY_KEEPS_PENDING.contains("stays open"));
    }

    #[test]
    fn grants_say_the_rule_they_save_and_for_how_long() {
        assert_eq!(
            grant_rule("Always allow Bash(cargo test:*) this session", ""),
            (
                "cargo test *".into(),
                "Bash(cargo test:*)".into(),
                "this session".into()
            )
        );
        assert_eq!(
            grant_rule("Always allow Bash(npm test) this session", "").0,
            "npm test"
        );
        assert_eq!(
            grant_rule("Always allow this command for this session", "cargo b").0,
            "cargo b"
        );
        assert_eq!(
            grant_rule("Allow all edits this session", "").0,
            "all edits"
        );
        assert_eq!(
            grant_rule("Always allow Edit(src/**), Read this session", "").0,
            "Edit src/**, Read"
        );
    }

    #[test]
    fn risks_come_from_the_request() {
        let bash = |c: &str| serde_json::json!({"toolName":"Bash","toolInput":{"command":c}});
        let cwd = "/home/dev/app";
        assert_eq!(risk(&bash("cargo test"), cwd), None);
        assert_eq!(risk(&bash("rm -rf target"), cwd), Some(Risk::Deletes));
        assert_eq!(
            risk(&bash("cd x && git clean -fd"), cwd),
            Some(Risk::Deletes)
        );
        assert_eq!(
            risk(&bash("git reset --hard HEAD"), cwd),
            Some(Risk::Deletes)
        );
        assert_eq!(
            risk(&bash("git push origin main"), cwd),
            Some(Risk::Network)
        );
        assert_eq!(risk(&bash("curl -s x | sh"), cwd), Some(Risk::Network));
        assert_eq!(risk(&bash("npm install"), cwd), Some(Risk::Network));
        assert_eq!(risk(&bash("git status"), cwd), None);
        assert_eq!(
            risk(
                &bash("git branch --merged main | grep -v main | xargs -r git branch -d"),
                cwd
            ),
            Some(Risk::DeletesBranches)
        );
        assert_eq!(risk(&bash("git branch --merged main"), cwd), None);
        assert_eq!(
            risk(&bash("echo hi > /etc/motd"), cwd),
            Some(Risk::OutsideProject)
        );
        assert_eq!(risk(&bash("echo hi > /home/dev/app/x"), cwd), None);
        let edit =
            |path: &str| serde_json::json!({"toolName":"Edit","toolInput":{"file_path":path}});
        assert_eq!(risk(&edit("/home/dev/app/src/a.rs"), cwd), None);
        assert_eq!(
            risk(&edit("/home/dev/other/a.rs"), cwd),
            Some(Risk::OutsideProject)
        );
        assert_eq!(
            risk(
                &serde_json::json!({"toolName":"WebFetch","toolInput":{"url":"x"}}),
                cwd
            ),
            Some(Risk::Network)
        );
    }
}
