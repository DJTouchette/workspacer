//! R4.3: the one approval / decision component. A tool approval in the
//! conversation, a plan review, a Fleet tile's ask and a job proposal all
//! draw through [`Decision::render`], so they look and read the same:
//!
//! - a question headline whose code spans wrap, never truncate;
//! - the agent's reason, then the full request (a long or multi-line
//!   command, wrapped, in mono) and where it would run: folder and host;
//! - Allow as the one Primary, any "Always allow …" grant with its exact
//!   scoped rule, then Deny (and Deny with note) as Secondary, never blue;
//! - Details, a tertiary toggle that expands the full request in place;
//! - number keys as faint keycaps only while this decision takes them
//!   (the top decision in the dock, the focused Fleet tile).
//!
//! Callers describe the decision and hand in click handlers; this module
//! owns the layout, the button roles and the keycaps. Design v2 only.
use super::primitives::{self, ButtonSize, ButtonVariant};
use super::*;
use gpui::{AnyElement, ClickEvent, relative};
use std::rc::Rc;

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

    /// The button in `variant`, its keycap shown when `keys` is on.
    fn render(&self, variant: ButtonVariant, keys: bool, p: &Palette) -> Stateful<Div> {
        let t = p.tokens;
        let size = ButtonSize::Small;
        let el = match self.key.filter(|_| keys) {
            Some(key) => primitives::keyed_button(
                self.id.clone(),
                key.to_string(),
                self.label.clone(),
                variant,
                size,
                self.enabled,
                p,
            ),
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

/// The Details toggle and, while open, what it expands to.
pub(crate) struct Details {
    pub toggle: Choice,
    pub open: bool,
    pub content: Option<AnyElement>,
}

/// Where the decision is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Frame {
    /// The "waiting on you" card (conversation, Jobs).
    Card { compact: bool },
    /// Inside a container that already frames it (a Fleet tile).
    Inline,
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
    /// "2/3" while more than one waits.
    pub pager: Option<String>,
    /// A body between the head and the actions (a plan).
    pub body: Option<AnyElement>,
    pub details: Option<Details>,
    /// A note being written (Deny with note, Keep planning).
    pub note: Option<AnyElement>,
    pub allow: Choice,
    /// Allow is the Primary unless a note being written owns it.
    pub allow_primary: bool,
    /// Secondary alternatives to Allow: "Always allow …", "Approve · run
    /// with Codex".
    pub alternates: Vec<Choice>,
    pub deny: Option<Choice>,
    pub deny_note: Option<NoteMenu>,
    /// Whether this decision shows its number keys.
    pub keys: bool,
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
            pager: None,
            body: None,
            details: None,
            note: None,
            allow,
            allow_primary: true,
            alternates: Vec::new(),
            deny: None,
            deny_note: None,
            keys: false,
            frame,
        }
    }

    pub(crate) fn render(self, p: &Palette) -> Div {
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
        let details_toggle = self.details.as_ref().map(|d| {
            let tip = if d.open {
                "Hide the full request"
            } else {
                "Show the full request: every argument, the folder and the host"
            };
            let id = d.toggle.id.clone();
            let control = primitives::button(
                d.toggle.id.clone(),
                d.toggle.label.clone(),
                Some(if d.open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                }),
                ButtonVariant::Tertiary,
                ButtonSize::Small,
                d.toggle.enabled,
                p,
            )
            .debug_selector(move || id.to_string())
            .tooltip(primitives::tooltip_view(tip, None, p));
            match d.toggle.on_click.clone().filter(|_| d.toggle.enabled) {
                Some(f) => control
                    .on_click(move |e, w, cx| f(e, w, cx))
                    .into_any_element(),
                None => control.into_any_element(),
            }
        });
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
        let details = self
            .details
            .and_then(|d| d.open.then_some(d.content).flatten());

        // Actions: Allow (Primary), the alternatives (Secondary), Deny and
        // its note (Secondary). Never a second fill, never blue on Deny.
        let mut actions = primitives::waiting_actions(p).child(self.allow.render(
            if self.allow_primary {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Secondary
            },
            keys,
            p,
        ));
        for choice in &self.alternates {
            actions = actions.child(choice.render(ButtonVariant::Secondary, keys, p));
        }
        if let Some(deny) = &self.deny {
            actions = actions.child(deny_control(deny, self.deny_note, keys, p));
        }
        // Details, tertiary, last in the row: it expands the request in
        // place and decides nothing.
        actions = actions.children(details_toggle);

        let content = |d: Div| {
            d.child(head)
                .children(request)
                .children(place)
                .children(self.body)
                .children(details)
                .children(self.note)
                .child(actions)
        };
        match self.frame {
            Frame::Card { compact } => content(primitives::waiting_card(compact, p))
                .debug_selector(move || selector.to_string())
                .occlude(),
            Frame::Inline => content(
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

/// Deny as a Secondary button, or a Secondary split whose menu holds
/// "Deny with note…".
fn deny_control(deny: &Choice, note: Option<NoteMenu>, keys: bool, p: &Palette) -> Div {
    let Some(note) = note else {
        return div()
            .flex_shrink_0()
            .child(deny.render(ButtonVariant::Secondary, keys, p));
    };
    let (main, chevron) = primitives::split_button_parts(
        "deny",
        deny.label.clone(),
        None,
        ButtonVariant::Secondary,
        ButtonSize::Small,
        deny.enabled,
        p,
    );
    let main = {
        let id = deny.id.clone();
        main.debug_selector(move || id.to_string())
            .when_some(deny.key.filter(|_| keys), |d, key| {
                d.child(primitives::keycap_hint(key.to_string(), false, p))
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
                        d.child(primitives::keycap_hint(key.to_string(), false, p))
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
}
