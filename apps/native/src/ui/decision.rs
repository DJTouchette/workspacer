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
use super::*;
use gpui::{AnyElement, ClickEvent, relative};
use std::rc::Rc;

/// The one submit label of every question card (the conversation's
/// question set, a Fleet tile's question).
pub(crate) const SUBMIT_ANSWER: &str = "Send answer";

/// A click handler, as `cx.listener(…)` builds one.
pub(crate) type OnClick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// A menu-dismiss handler (a click outside it).
pub(crate) type OnDismiss = Rc<dyn Fn(&mut Window, &mut App)>;

/// A request value shows in full under the headline (rather than as a code
/// span in it) once it is longer than this or has several lines.
pub(crate) const HEADLINE_FITS: usize = 60;

/// Whether `value` is too long or too tall for the headline: it then shows
/// whole, wrapped, as the decision's request.
pub(crate) fn needs_request_line(value: &str) -> bool {
    let value = value.trim();
    value.contains('\n') || value.chars().count() > HEADLINE_FITS
}

/// The "Reply instead…" line's tooltip (R4.6): replying does not answer.
pub(crate) const REPLY_KEEPS_PENDING: &str =
    "Write to the agent instead. The request stays pending until you choose above.";

/// One action of a decision.
#[derive(Clone)]
pub(crate) struct Choice {
    /// Element id and debug selector.
    pub id: SharedString,
    pub label: SharedString,
    /// Its number key (1-based), drawn only while the decision takes keys.
    pub key: Option<usize>,
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
            tooltip: None,
            enabled: true,
            on_click: None,
        }
    }

    pub(crate) fn key(mut self, key: Option<usize>) -> Self {
        self.key = key;
        self
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
        let el = match self.key.filter(|_| keys) {
            Some(key) if live => primitives::keyed_button(
                self.id.clone(),
                key.to_string(),
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

/// Deny's "Deny with note…" menu, hung from a chevron on Deny.
pub(crate) struct NoteMenu {
    pub chevron: Choice,
    pub open: bool,
    pub item: Choice,
    pub on_dismiss: Option<OnDismiss>,
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
    OutsideProject,
    Network,
}

impl Risk {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Deletes => "Deletes files",
            Self::OutsideProject => "Writes outside the project",
            Self::Network => "Uses the network",
        }
    }

    /// R6.3: a request that deletes files is allowed only on a second
    /// press ("Press again to allow").
    pub(crate) fn confirms(self) -> bool {
        self == Self::Deletes
    }
}

/// Allow's label once a destructive request has been pressed once.
pub(crate) const PRESS_AGAIN: &str = "Press again to allow";

/// Allow's tooltip on a request that needs the second press.
pub(crate) const CONFIRM_TIP: &str = "Deletes files: press Allow (or its key) twice to allow it";

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

    /// Whether the request `key` waits for its second press.
    pub(crate) fn armed(&self, key: &str) -> bool {
        self.arming.armed.as_deref() == Some(key)
    }

    /// R6.3: number keys on the keyed decisions outside the conversation's
    /// dock and the Fleet: Jobs' first proposal (1 Approve, 2 Reject) and
    /// a stalled subagent's view (1 Nudge, 2 Stop, 3 View output). Only
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
    pub folder: Option<String>,
    pub host: Option<String>,
    /// What the request would do that is hard to take back.
    pub risk: Option<Risk>,
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
    /// Whether Allow shows at all: off when the body answers (a
    /// question's options send on a click).
    pub show_allow: bool,
    /// Secondary alternatives to Allow ("Approve · run with Codex").
    pub alternates: Vec<Choice>,
    /// "Always allow" grants, each with its exact rule and scope.
    pub grants: Vec<Grant>,
    pub deny: Option<Choice>,
    /// Deny's tier: Destructive (Deny, Reject), or Secondary for a "no"
    /// that removes nothing (Keep planning).
    pub deny_variant: ButtonVariant,
    pub deny_note: Option<NoteMenu>,
    /// Whether this decision shows its number keys.
    pub keys: bool,
    /// Whether the number keys go to it now (R6.3): its keycaps at full
    /// strength. Off, they show dimmed: keyed, but the keys go elsewhere
    /// (another focused card, text in the composer).
    pub keys_live: bool,
    /// R6.3: a destructive request ([`Risk::confirms`]) pressed once:
    /// Allow reads "Press again to allow".
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
            folder: None,
            host: None,
            risk: None,
            pager: None,
            body: None,
            details: None,
            note: None,
            allow,
            allow_primary: true,
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

    /// `rows` less what the card already says (the reason, the request,
    /// the folder, the host, a code span of the headline), plus each
    /// grant's exact rule: Details adds, it never repeats.
    pub(crate) fn detail_rows(
        &self,
        rows: Vec<(String, String, bool)>,
    ) -> Vec<(String, String, bool)> {
        let shown: Vec<&str> = [
            self.reason.as_deref(),
            self.request.as_deref(),
            self.folder.as_deref(),
            self.host.as_deref(),
        ]
        .into_iter()
        .flatten()
        .chain(
            self.headline
                .iter()
                .filter(|(_, code)| *code)
                .map(|(t, _)| t.as_str()),
        )
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
        let mut rows: Vec<_> = rows
            .into_iter()
            .filter(|(_, value, _)| !shown.contains(&value.trim()))
            .collect();
        for grant in &self.grants {
            if !grant.exact.is_empty() && grant.exact != grant.rule {
                rows.push((
                    "Rule".to_owned(),
                    format!("{} · {}", grant.exact, grant.scope),
                    true,
                ));
            }
        }
        rows
    }

    pub(crate) fn render(mut self, p: &Palette) -> Div {
        let t = p.tokens;
        let keys = self.keys;
        let selector = self.selector.clone();
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
        let place = where_line(self.folder.as_deref(), self.host.as_deref());
        let place = (!place.is_empty()).then(|| {
            div()
                .debug_selector(|| "decision-where".into())
                .min_w_0()
                .text_size(px(t.text.meta))
                .line_height(relative(t.text.line_height))
                .text_color(rgb(p.roles.text_tertiary))
                .child(place)
        });
        // R6.3: the risk is a visible chip, a fact about the request and
        // not a status: never amber or red. One that deletes files reads
        // neutral-strong (`caution`) and asks for a second press.
        let risk = self.risk.map(|risk| div().flex().child(risk_chip(risk, p)));
        let live = self.keys_live;
        let confirms = self.risk.is_some_and(Risk::confirms);
        if confirms {
            if self.armed {
                self.allow.label = PRESS_AGAIN.into();
            }
            self.allow.tooltip = Some(CONFIRM_TIP.into());
        }
        // Details: only what the card does not already say. Nothing left
        // to add, no toggle.
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
                "Show what the card leaves out: the tool, every argument, the exact rule"
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
                ButtonVariant::Tertiary,
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
        // The answer: Allow (Primary) and Deny (Destructive) side by side,
        // never stacked; any Secondary alternative after them.
        let allow = (self.show_allow && !noting).then(|| {
            self.allow.render(
                if self.allow_primary {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Secondary
                },
                keys,
                live,
                p,
            )
        });
        let deny = self.deny.as_ref().filter(|_| !noting).map(|deny| {
            deny_control(
                deny,
                self.deny_variant,
                self.deny_note.take(),
                keys,
                live,
                p,
            )
        });
        let alternates: Vec<Stateful<Div>> = if noting {
            Vec::new()
        } else {
            self.alternates
                .iter()
                .map(|c| c.render(ButtonVariant::Secondary, keys, live, p))
                .collect()
        };
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
                .children(alternates);
        }
        let answer = (pair.is_some() || !alternates.is_empty()).then(|| {
            primitives::waiting_actions(p)
                .children(pair)
                .children(alternates)
        });
        // Then the quiet row: each grant with its exact rule and scope,
        // and Details (it decides nothing) at the end.
        let grants: Vec<Div> = if noting {
            Vec::new()
        } else {
            self.grants
                .iter()
                .map(|g| grant_row(g, keys, live, p))
                .collect()
        };
        let details_toggle = details_toggle.filter(|_| !noting);
        let quiet = (!grants.is_empty() || details_toggle.is_some()).then(|| {
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
                .children(details_toggle.map(|d| div().flex_shrink_0().child(d)))
        });

        let content = |d: Div| {
            d.child(head)
                .children(request)
                .children(place)
                .children(risk)
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

/// A keycap at the strength of keys that go elsewhere just now (R6.3).
fn dim_keycap(key: usize, on_fill: bool, p: &Palette) -> Div {
    primitives::keycap_hint(key.to_string(), on_fill, p).opacity(p.tokens.wash.yielded)
}

/// The keycap after a split's label or a menu item: full strength while
/// the keys go here, dimmed otherwise.
fn keycap(key: usize, live: bool, p: &Palette) -> Div {
    if live {
        primitives::keycap_hint(key.to_string(), false, p)
    } else {
        dim_keycap(key, false, p)
    }
}

/// R6.3: the risk cue as a chip: its glyph and words on a quiet wash.
/// "Deletes files" (the one that asks for a second press) in the
/// neutral-strong `caution`; the others in secondary text.
fn risk_chip(risk: Risk, p: &Palette) -> Div {
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
        background_color: Some(rgb(p.code_inline).into()),
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
            ButtonVariant::Tertiary,
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

/// Deny in its tier, or a split whose menu holds "Deny with note…".
fn deny_control(
    deny: &Choice,
    variant: ButtonVariant,
    note: Option<NoteMenu>,
    keys: bool,
    live: bool,
    p: &Palette,
) -> Div {
    let Some(note) = note else {
        return div()
            .flex_shrink_0()
            .child(deny.render(variant, keys, live, p));
    };
    let (main, chevron) = primitives::split_button_parts(
        "deny",
        deny.label.clone(),
        None,
        variant,
        ButtonSize::Small,
        deny.enabled,
        p,
    );
    let main = {
        let id = deny.id.clone();
        main.debug_selector(move || id.to_string())
            .when_some(deny.key.filter(|_| keys), |d, key| {
                d.child(keycap(key, live, p))
            })
            .when_some(deny.tooltip.clone(), |d, tip| {
                d.tooltip(primitives::tooltip_view(tip, None, p))
            })
            .when_some(deny.on_click.clone().filter(|_| deny.enabled), |d, f| {
                d.on_click(move |e, w, cx| f(e, w, cx))
            })
    };
    let chevron = {
        let id = note.chevron.id.clone();
        chevron.debug_selector(move || id.to_string()).when_some(
            note.chevron
                .on_click
                .clone()
                .filter(|_| note.chevron.enabled),
            |d, f| d.on_click(move |e, w, cx| f(e, w, cx)),
        )
    };
    let menu = (note.open && note.chevron.enabled).then(|| {
        let item = &note.item;
        let id = item.id.clone();
        primitives::menu(p)
            .debug_selector(|| "deny-menu-list".into())
            .occlude()
            .when_some(note.on_dismiss.clone(), |d, f| {
                d.on_mouse_down_out(move |_, w, cx| f(w, cx))
            })
            .child(
                primitives::menu_item(item.id.clone(), item.label.clone(), None, false, true, p)
                    .debug_selector(move || id.to_string())
                    .when_some(item.key.filter(|_| keys), |d, key| {
                        d.child(keycap(key, live, p))
                    })
                    .when_some(item.tooltip.clone(), |d, tip| {
                        d.tooltip(primitives::tooltip_view(tip, None, p))
                    })
                    .when_some(item.on_click.clone(), |d, f| {
                        d.on_click(move |e, w, cx| f(e, w, cx))
                    }),
            )
    });
    primitives::split_button_with_menu(main, chevron, menu, p)
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
        background_color: Some(rgb(p.code_inline).into()),
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
    fn replying_says_the_request_stays_pending() {
        assert!(REPLY_KEEPS_PENDING.contains("stays pending"));
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
