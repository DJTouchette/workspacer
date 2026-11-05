//! R5.3 The component gallery: every component of `docs/design-system.md`
//! in every state, in the current theme. `Screen::Gallery`, opened by
//! `wks-native --gallery` in dev builds (never in the nav); shot in every
//! built-in theme as `system-gallery-<theme>` by `shoot-all.py`.
//!
//! Interaction states (hover, pressed) are drawn as static specimens
//! (`primitives::button_specimen`) so one frame shows all of them.
use super::super::decision::{Choice, Decision, Details, Frame, Grant, Risk, SUBMIT_ANSWER};
use super::super::primitives::{
    self as prim, ButtonSize, ButtonVariant, Col, ControlState, CountTone, StatusKind,
};
use super::*;

const VARIANTS: [ButtonVariant; 5] = [
    ButtonVariant::Primary,
    ButtonVariant::Secondary,
    ButtonVariant::Tertiary,
    ButtonVariant::Destructive,
    // r8-decisions (R8.2): the armed confirm.
    ButtonVariant::Danger,
];

impl Workspace {
    /// The gallery page (R5.3): always the v2 design in the current theme.
    pub(in crate::ui) fn render_gallery(&mut self) -> Div {
        let p = self.appearance.palette_for(Design::V2);
        self.page_view(
            "gallery-scroll",
            p.tokens.pages().wide,
            false,
            div()
                .debug_selector(|| "system-gallery".into())
                .child(gallery(&p)),
        )
    }

    /// Open the gallery (dev builds: `--gallery`).
    pub(crate) fn open_gallery(&mut self) {
        self.screen = Screen::Gallery;
    }
}

/// A titled specimen block.
fn section(title: &'static str, p: &Palette) -> Div {
    let t = p.tokens;
    div()
        .flex()
        .flex_col()
        .gap(px(t.space.md))
        .child(prim::section_header(title, None, p))
}

/// A caption under or beside a specimen.
fn caption(text: impl Into<SharedString>, p: &Palette) -> Div {
    let t = p.tokens;
    div()
        .text_size(px(t.text.meta))
        .text_color(rgb(p.roles.text_tertiary))
        .child(text.into())
}

fn row(p: &Palette) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(p.tokens.space.md))
}

/// Every component in every state, laid out in two columns.
pub(crate) fn gallery(p: &Palette) -> AnyElement {
    let t = p.tokens;
    let column = || {
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(t.space.xxl))
    };
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(t.space.xxl))
        .child(page_specimen(p))
        .child(
            div()
                .w_full()
                .flex()
                .items_start()
                .gap(px(t.space.xxl))
                .child(
                    column()
                        .child(foundations(p))
                        .child(buttons(p))
                        .child(icon_buttons(p))
                        .child(navigation(p))
                        .child(statuses(p))
                        .child(toggles(p))
                        .child(meters(p)),
                )
                .child(
                    column()
                        .child(menus(p))
                        .child(overlays(p))
                        .child(cards(p))
                        .child(table(p))
                        .child(composers(p)),
                ),
        )
        .child(decision(p))
        .into_any_element()
}

/// PageShell: the header every page shares, at the page's width rule.
fn page_specimen(p: &Palette) -> Div {
    page_shell(
        prim::page_header(
            prim::back_link("gallery-back", "Back to chat", p).into_any_element(),
            "Component gallery",
            Some(
                "Every component in every state, in this theme. The spec is docs/design-system.md."
                    .into(),
            ),
            vec![
                prim::button(
                    "gallery-header-action",
                    "Primary action",
                    Some(IconName::Plus),
                    ButtonVariant::Primary,
                    ButtonSize::Large,
                    true,
                    p,
                )
                .into_any_element(),
            ],
            p,
        ),
        None,
        div(),
        PageWidth::Wide,
        p,
    )
}

fn swatch(name: &'static str, color: u32, p: &Palette) -> Div {
    let t = p.tokens;
    div()
        .flex()
        .items_center()
        .gap(px(t.space.sm))
        .w(px(t.pages().age + t.space.xxl))
        .child(
            div()
                .size(px(t.icon.lg))
                .flex_shrink_0()
                .rounded(px(t.radius.xs))
                .bg(rgb(color))
                .border(px(t.stroke.hairline))
                .border_color(rgb(p.roles.border_strong)),
        )
        .child(caption(name, p).truncate())
}

fn foundations(p: &Palette) -> Div {
    let t = p.tokens;
    let r = p.roles;
    let swatches: [(&'static str, u32); 19] = [
        ("base", p.base),
        ("chat", p.chat),
        ("surface", p.surface),
        ("raised", r.raised),
        ("selected", p.selected),
        ("hover", r.hover),
        ("border", p.border),
        ("border strong", r.border_strong),
        ("text", p.text),
        ("secondary", r.text_secondary),
        ("tertiary", r.text_tertiary),
        ("accent", p.accent),
        ("primary", p.primary),
        ("on primary", p.on_primary),
        ("attention", r.status.attention),
        ("fault", r.status.fault),
        ("warning", r.caution),
        ("done", r.status.done),
        ("idle", r.status.idle),
    ];
    let type_scale: [(&'static str, f32, f32); 7] = [
        ("Title 24", t.text.title, t.weight.title),
        ("Title short 20", t.text.title_short, t.weight.semibold),
        ("Heading 17", t.text.heading, t.weight.semibold),
        ("Lead 15", t.text.lead, t.weight.semibold),
        ("Subheading 14", t.text.subheading, t.weight.semibold),
        ("Body 13", t.text.body, t.weight.regular),
        ("Meta 12", t.text.meta, t.weight.regular),
    ];
    section("Foundations", p)
        .debug_selector(|| "gallery-foundations".into())
        .child(
            row(p).children(
                swatches
                    .into_iter()
                    .map(|(name, color)| swatch(name, color, p)),
            ),
        )
        .child(
            div()
                .flex()
                .items_end()
                .gap(px(t.space.xl))
                .child(div().flex().flex_col().gap(px(t.space.xxs)).children(
                    type_scale.into_iter().map(|(name, size, weight)| {
                        div()
                            .text_size(px(size))
                            .font_weight(FontWeight(weight))
                            .text_color(rgb(p.text))
                            .child(name)
                    }),
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(t.space.xs))
                        .child(div().text_color(rgb(p.text)).child("Text: primary"))
                        .child(
                            div()
                                .text_color(rgb(r.text_secondary))
                                .child("Text: secondary"),
                        )
                        .child(
                            div()
                                .text_color(rgb(r.text_tertiary))
                                .child("Text: tertiary"),
                        )
                        .child(
                            div()
                                .text_color(rgb(p.accent))
                                .underline()
                                .child("A link in accent"),
                        ),
                ),
        )
        .child(
            row(p)
                .debug_selector(|| "gallery-spacing".into())
                .items_end()
                .children(
                    [
                        t.space.xs,
                        t.space.md,
                        t.space.lg,
                        t.space.xl,
                        t.space.xxl,
                        t.space.xxxl,
                    ]
                    .into_iter()
                    .map(|s| {
                        div()
                            .w(px(s))
                            .h(px(s))
                            .bg(prim::wash(p.accent, t.wash.tone_hover))
                    }),
                )
                .children(
                    [t.radius.xs, t.radius.control, t.radius.panel]
                        .into_iter()
                        .map(|radius| {
                            div()
                                .size(px(t.space.xxxl))
                                .rounded(px(radius))
                                .border(px(t.stroke.hairline))
                                .border_color(rgb(r.border_strong))
                        }),
                ),
        )
        .child(
            row(p)
                .debug_selector(|| "gallery-elevation".into())
                .children(Level::ALL.into_iter().map(|level| {
                    elevated(level, p)
                        .w(px(t.pages().when - t.space.xxxl))
                        .p(px(t.space.lg))
                        .text_size(px(t.text.meta))
                        .text_color(rgb(r.text_secondary))
                        .child(format!("{level:?}"))
                })),
        )
}

fn buttons(p: &Palette) -> Div {
    let t = p.tokens;
    let states = ControlState::ALL;
    // The label column, then one column per state.
    let cell = move |ix: usize| {
        if ix == 0 {
            t.pages().action_slot
        } else {
            t.pages().age
        }
    };
    section("Button", p)
        .debug_selector(|| "gallery-buttons".into())
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(t.space.sm))
                .child(
                    row(p).children(
                        ["", "Rest", "Hover", "Pressed", "Disabled"]
                            .into_iter()
                            .enumerate()
                            .map(|(ix, h)| caption(h, p).w(px(cell(ix)))),
                    ),
                )
                .children(VARIANTS.into_iter().map(|v| {
                    row(p)
                        .child(caption(format!("{v:?}"), p).w(px(cell(0))))
                        .children(states.into_iter().map(move |state| {
                            div().w(px(cell(1))).child(prim::button_specimen(
                                format!("{v:?}"),
                                v,
                                ButtonSize::Small,
                                state,
                                p,
                            ))
                        }))
                })),
        )
        .child(
            row(p)
                .children(VARIANTS.into_iter().enumerate().map(|(ix, v)| {
                    prim::button(
                        ("gallery-large", ix),
                        format!("{v:?}"),
                        (ix == 0).then_some(IconName::Plus),
                        v,
                        ButtonSize::Large,
                        true,
                        p,
                    )
                }))
                .child(focus_specimen(p)),
        )
        .child({
            // R5.2: Send fills only when it can send; Stop is Destructive.
            let (send, can) = send_state(true);
            let (empty, cannot) = send_state(false);
            let (main, menu) = prim::split_button_parts(
                "gallery-split",
                "Queue",
                Some(IconName::ArrowUp),
                ButtonVariant::Primary,
                ButtonSize::Small,
                true,
                p,
            );
            row(p)
                .child(prim::button(
                    "gallery-send",
                    "Send",
                    Some(IconName::ArrowUp),
                    send,
                    ButtonSize::Small,
                    can,
                    p,
                ))
                .child(prim::button(
                    "gallery-send-empty",
                    "Send",
                    Some(IconName::ArrowUp),
                    empty,
                    ButtonSize::Small,
                    cannot,
                    p,
                ))
                .child(prim::button(
                    "gallery-stop",
                    "Stop",
                    None,
                    stop_variant(),
                    ButtonSize::Small,
                    true,
                    p,
                ))
                .child(prim::split_button(main, menu, p))
                .child(prim::keyed_button(
                    "gallery-keyed",
                    "1",
                    "Allow",
                    ButtonVariant::Primary,
                    ButtonSize::Small,
                    true,
                    p,
                ))
                .child(caption("Send · Send (empty) · Stop · Queue · keyed", p))
        })
}

/// The keyboard focus ring (`roles.focus` in the reserved 2px border).
fn focus_specimen(p: &Palette) -> Div {
    let t = p.tokens;
    div()
        .debug_selector(|| "gallery-focus".into())
        .h(px(t.layout.control_height))
        .px(px(t.space.lg))
        .flex()
        .items_center()
        .rounded(px(t.radius.control))
        .border(px(t.stroke.focus))
        .border_color(rgb(p.roles.focus))
        .text_size(px(t.text.subheading))
        .font_weight(FontWeight(t.weight.medium))
        .text_color(rgb(p.text))
        .child("Focused")
}

fn icon_buttons(p: &Palette) -> Div {
    section("IconButton", p)
        .debug_selector(|| "gallery-icon-buttons".into())
        .child(
            row(p)
                .child(
                    IconButton::new("gallery-icon-rest", IconName::Search, "Search")
                        .shortcut("Ctrl K")
                        .render(p),
                )
                .child(
                    IconButton::new(
                        "gallery-icon-selected",
                        IconName::PanelLeftClose,
                        "Collapse sidebar",
                    )
                    .shortcut("Ctrl B")
                    .selected(true)
                    .render(p),
                )
                .child(
                    IconButton::new("gallery-icon-secondary", IconName::Plus, "Add")
                        .variant(ButtonVariant::Secondary)
                        .render(p),
                )
                .child(
                    IconButton::new("gallery-icon-danger", IconName::Delete, "Delete")
                        .variant(ButtonVariant::Destructive)
                        .render(p),
                )
                .child(
                    IconButton::new("gallery-icon-disabled", IconName::Redo, "Retry")
                        .enabled(false)
                        .render(p),
                )
                .child(prim::overflow_trigger("gallery-overflow", p))
                .child(caption(
                    "rest · open · secondary · destructive · disabled · overflow",
                    p,
                )),
        )
        // r5-shell: the header strip's tools: a badge beside the icon, a
        // corner dot (accent: something new; a status role: a running shell).
        .child(
            row(p)
                .child(
                    IconButton::new("gallery-icon-badge", prim::Concept::Tasks, "Tasks")
                        .icon_size(p.tokens.icon.md)
                        .badge(Some(prim::count_badge("3", CountTone::Neutral, p)))
                        .render(p),
                )
                .child(
                    IconButton::new("gallery-icon-dot", IconName::Settings, "Settings")
                        .shortcut("Ctrl ,")
                        .dot(Some("gallery-icon-dot-mark"))
                        .render(p),
                )
                .child(
                    IconButton::new("gallery-icon-run", IconName::SquareTerminal, "Terminal")
                        .dot_color(Some("gallery-icon-run-mark"), p.roles.status.working)
                        .render(p),
                )
                .child(caption("badge · update dot · running dot", p)),
        )
}

/// r5-shell: a capacity meter's three tiers: neutral, neutral-strong
/// (`caution`) from 80%, red at the limit. Never amber.
fn meters(p: &Palette) -> Div {
    let t = p.tokens;
    let specimen = |share: f32, intent: prim::Intent, label: &'static str| {
        div()
            .w(px(t.layout.menu_width))
            .flex()
            .flex_col()
            .gap(px(t.space.xs))
            .child(caption(label, p))
            .child(prim::meter(share, intent, p))
    };
    section("Meter", p)
        .debug_selector(|| "gallery-meters".into())
        .child(
            row(p)
                .child(specimen(0.42, prim::Intent::Neutral, "42% · neutral"))
                .child(specimen(0.93, prim::Intent::Caution, "93% · warning"))
                .child(specimen(
                    1.,
                    prim::Intent::Caution,
                    "100% · at the limit (still the warning hue)",
                )),
        )
}

fn navigation(p: &Palette) -> Div {
    let t = p.tokens;
    section("NavItem", p)
        .debug_selector(|| "gallery-nav".into())
        .child(
            div()
                .flex()
                .gap(px(t.space.xl))
                .child(
                    div()
                        .w(px(t.layout.menu_width))
                        .p(px(t.space.xs))
                        .rounded(px(t.radius.panel))
                        .bg(rgb(p.base))
                        .flex()
                        .flex_col()
                        .gap(px(t.space.xxs))
                        .child(
                            NavItem::new("gallery-nav-fleet", prim::Concept::Fleet, "Fleet")
                                .shortcut("Ctrl Shift F")
                                .selected(true)
                                .render(p),
                        )
                        .child(
                            NavItem::new("gallery-nav-projects", IconName::Folder, "Projects")
                                .shortcut("g p")
                                .count(4, CountTone::Neutral)
                                .render(p),
                        )
                        .child(
                            NavItem::new("gallery-nav-jobs", IconName::Calendar, "Jobs")
                                .shortcut("g j")
                                .count(1, CountTone::NeedsYou)
                                .render(p),
                        )
                        .child(
                            NavItem::new(
                                "gallery-nav-machines",
                                prim::Concept::Machines,
                                "Machines",
                            )
                            .shortcut("g o")
                            .render(p),
                        )
                        // r5-shell: Scratch's own glyph, a definition in its
                        // tooltip, and a disabled destination.
                        .child(
                            NavItem::new("gallery-nav-scratch", prim::Concept::Scratch, "Scratch")
                                .hint("Scratch · files agents wrote for you to review and run")
                                .count(4, CountTone::Neutral)
                                .render(p),
                        )
                        .child(
                            NavItem::new("gallery-nav-new", IconName::Plus, "New session")
                                .enabled(false)
                                .render(p),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(t.space.md))
                        .child(
                            div()
                                .p(px(t.space.md))
                                .rounded(px(t.radius.panel))
                                .bg(rgb(p.base))
                                .flex()
                                .gap(px(t.space.md))
                                .child(
                                    NavItem::new(
                                        "gallery-tile-fleet",
                                        prim::Concept::Fleet,
                                        "Fleet",
                                    )
                                    .selected(true)
                                    .render_tile(p),
                                )
                                .child(
                                    NavItem::new(
                                        "gallery-tile-projects",
                                        IconName::FolderClosed,
                                        "Projects",
                                    )
                                    .count(4, CountTone::Neutral)
                                    .render_tile(p),
                                )
                                .child(
                                    NavItem::new("gallery-tile-jobs", IconName::Calendar, "Jobs")
                                        .count(1, CountTone::NeedsYou)
                                        .render_tile(p),
                                )
                                .child(
                                    NavItem::new(
                                        "gallery-tile-settings",
                                        IconName::Settings,
                                        "Settings",
                                    )
                                    .render_tile(p),
                                ),
                        )
                        .child(caption("Tiles: counts sit on the corner", p))
                        .child(
                            row(p)
                                .debug_selector(|| "gallery-counts".into())
                                .child(prim::count_badge("4", CountTone::Neutral, p))
                                .child(prim::count_badge("2", CountTone::NeedsYou, p))
                                .child(prim::count_badge("1", CountTone::Failed, p))
                                .child(prim::count_badge(
                                    prim::count_text(120),
                                    CountTone::Neutral,
                                    p,
                                ))
                                .child(caption("CountBadge", p)),
                        )
                        .child(
                            row(p)
                                .debug_selector(|| "gallery-keycaps".into())
                                .child(prim::kbd("Ctrl", p))
                                .child(prim::kbd("K", p))
                                .child(prim::keycap_hint("1", false, p))
                                .child(caption("Keycap", p)),
                        ),
                ),
        )
}

// r6-surfaces (R6.2): the one status table, every row in its cell.
fn statuses(p: &Palette) -> Div {
    let t = p.tokens;
    section("StatusCell · the status table", p)
        .debug_selector(|| "gallery-status".into())
        .child(
            div()
                .w_full()
                .grid()
                .grid_cols(3)
                .gap(px(t.space.sm))
                .children(prim::STATUS_TABLE.iter().map(|row| {
                    // The state's word and glyph, then the other words that
                    // take its glyph.
                    div()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(t.space.xxs))
                        .child(StatusCell::new(row.kind).render(p))
                        .when(!row.also.is_empty(), |d| {
                            d.child(
                                caption(row.also, p)
                                    .pl(px(t.system().glyph_slot + t.space.xs))
                                    .truncate(),
                            )
                        })
                })),
        )
        // r7-status (R7.3): the verification line, failed and contradicted.
        .child(super::verification_line(
            &super::super::status::Verification {
                outcome: super::super::status::TestOutcome::Failed,
                command: "cargo test".into(),
                at: Some(0),
                contradicted: true,
            },
            4 * 60_000,
            p,
        ))
        // r7-qa: a meta line of elements, one join; a hint footer.
        .child(
            super::meta_row(
                [
                    StatusCell::new(StatusKind::NeedsYou)
                        .label("2 needs you")
                        .render(p)
                        .into_any_element(),
                    div().child("2 folders · 22 sessions").into_any_element(),
                ],
                p,
            )
            .text_size(px(p.tokens.text.meta))
            .text_color(rgb(p.roles.text_secondary)),
        )
        .child(super::key_hints(
            [
                super::key_hint(&["j", "k"], "Move", p),
                super::key_hint(&["↵"], "Open", p),
            ],
            p,
        ))
}

fn toggles(p: &Palette) -> Div {
    let surface = |child: Stateful<Div>, label: &'static str| {
        div()
            .flex()
            .items_center()
            .gap(px(p.tokens.space.sm))
            .child(child)
            .child(caption(label, p))
    };
    section("Toggle", p)
        .debug_selector(|| "gallery-toggles".into())
        .child(
            row(p)
                .gap(px(p.tokens.space.xl))
                .child(surface(toggle("gallery-toggle-on", true, true, p), "On"))
                .child(surface(toggle("gallery-toggle-off", false, true, p), "Off"))
                .child(surface(
                    toggle("gallery-toggle-on-disabled", true, false, p),
                    "On, disabled",
                ))
                .child(surface(
                    toggle("gallery-toggle-off-disabled", false, false, p),
                    "Off, disabled",
                )),
        )
        .child(
            row(p)
                .gap(px(p.tokens.space.xl))
                .child(gpui_component::switch::Switch::new("gallery-switch-on").checked(true))
                .child(gpui_component::switch::Switch::new("gallery-switch-off").checked(false))
                .child(caption("Settings switches (same colours)", p)),
        )
}

fn menus(p: &Palette) -> Div {
    let t = p.tokens;
    let list = prim::menu(p)
        .debug_selector(|| "gallery-menu".into())
        .child(prim::menu_item(
            "gallery-menu-open",
            "Open in editor",
            Some(IconName::ExternalLink),
            false,
            true,
            p,
        ))
        .child(
            prim::list_row("gallery-menu-hover", false, true, p)
                .bg(rgb(p.roles.hover))
                .child(div().flex_1().child("Hovered item"))
                .child(prim::keycap_hint("2", false, p)),
        )
        .child(prim::menu_item(
            "gallery-menu-disabled",
            "Unavailable item",
            None,
            false,
            false,
            p,
        ))
        .child(prim::menu_separator(p))
        .child(prim::menu_item(
            "gallery-menu-delete",
            "Discard changes…",
            Some(IconName::Delete),
            true,
            true,
            p,
        ));
    section("Menu", p)
        .debug_selector(|| "gallery-menus".into())
        .child(
            div()
                .h(px(t.layout.dialog_width / 2. + t.space.xxl))
                .child(anchored_menu(
                    prim::toolbar_chip("gallery-menu-trigger", None, "Sonnet 4.6", true, true, p),
                    Some(list),
                    MenuAnchor::BelowStart,
                    p,
                )),
        )
        .child(caption(
            "Anchored to its trigger (BelowStart); rest, hover, disabled, destructive",
            p,
        ))
}

fn overlays(p: &Palette) -> Div {
    let t = p.tokens;
    let dialog = prim::dialog(p)
        .debug_selector(|| "gallery-dialog".into())
        .child(prim::dialog_header(
            "End this session?",
            Some("The agent stops and the conversation moves to History.".into()),
            p,
        ))
        .child(
            prim::dialog_footer(p)
                .child(prim::button(
                    "gallery-dialog-cancel",
                    "Cancel",
                    None,
                    ButtonVariant::Secondary,
                    ButtonSize::Small,
                    true,
                    p,
                ))
                .child(prim::button(
                    "gallery-dialog-end",
                    "End session",
                    None,
                    ButtonVariant::Destructive,
                    ButtonSize::Small,
                    true,
                    p,
                )),
        );
    section("Overlay / Dialog", p)
        .debug_selector(|| "gallery-overlay".into())
        .child(
            div()
                .relative()
                .h(px(t.layout.dialog_width / 2. + t.space.huge))
                .rounded(px(t.radius.panel))
                .overflow_hidden()
                .bg(rgb(p.chat))
                .p(px(t.space.lg))
                .child(
                    div()
                        .text_size(px(t.text.lead))
                        .text_color(rgb(p.prose))
                        .child("Page content under the scrim stays visible but quiet."),
                )
                .child(overlay(Placement::Center, dialog, p)),
        )
}

fn cards(p: &Palette) -> Div {
    let tile_body = |title: &'static str, kind: StatusKind, lines: usize| {
        tile(p)
            .child(
                prim::CardHead::new(title)
                    .status(prim::StatusBadge::new(kind))
                    .meta("Weekdays 07:00")
                    .render(p),
            )
            .children((0..lines).map(|ix| caption(format!("Event line {}", ix + 1), p)))
    };
    section("Card / Tile", p)
        .debug_selector(|| "gallery-cards".into())
        .child(
            tile_grid(3, p)
                .child(tile_body("Nightly", StatusKind::Working, 3))
                .child(tile_body("Release", StatusKind::NeedsYou, 0))
                .child(tile_body("Docs", StatusKind::Done, 1)),
        )
        .child(caption("Natural height: tiles never stretch to the row", p))
}

fn table(p: &Palette) -> Div {
    let t = p.tokens;
    let pg = t.pages();
    let cols = [
        ("Machine", Col::Flex),
        ("Status", Col::Keep(pg.status)),
        ("", Col::Keep(prim::action_slots_width(1, p))),
    ];
    let rows: [(&str, StatusCell, Option<&str>); 4] = [
        (
            "studio (this machine)",
            StatusCell::new(StatusKind::Ready).label("Accepting work"),
            Some("Details"),
        ),
        (
            "build-01",
            StatusCell::new(StatusKind::Working).label("Running 2"),
            Some("Details"),
        ),
        ("laptop", StatusCell::new(StatusKind::Offline), None),
        (
            "ci-runner",
            StatusCell::new(StatusKind::Paused),
            Some("Resume"),
        ),
    ];
    section("Table", p)
        .debug_selector(|| "gallery-table".into())
        .child(
            // r7-pages (R7.6): the one table style, a group row in its frame.
            super::table(p)
                .child(super::table_head(&cols, p))
                .child(super::table_band(p).child("This workspace"))
                .children(
                    rows.into_iter()
                        .enumerate()
                        .map(|(ix, (name, cell, action))| {
                            prim::page_table_row(p)
                                .child(prim::page_cell(cols[0].1).child(name))
                                .child(prim::page_cell(cols[1].1).child(cell.render(p)))
                                .child(prim::page_cell(cols[2].1).child(prim::action_slots(
                                    vec![prim::action_slot(
                                        pg.action_slot,
                                        action.map(|label| {
                                            prim::slot_button(
                                                prim::button(
                                                    ("gallery-table-action", ix),
                                                    label,
                                                    None,
                                                    ButtonVariant::Tertiary,
                                                    ButtonSize::Small,
                                                    true,
                                                    p,
                                                ),
                                                p,
                                            )
                                            .into_any_element()
                                        }),
                                    )],
                                    p,
                                )))
                        }),
                ),
        )
}

/// r5-fleet (R5.4): the DecisionCard, one specimen per kind, each at a
/// Fleet tile's width so the answer row is seen not to stack: a tool
/// approval (keys on, a risk cue, an "Always allow" grant with its rule),
/// a question answered on the card, a plan, and a job proposal.
fn decision(p: &Palette) -> Div {
    let t = p.tokens;
    let specimen = |label: &'static str, card: Div| {
        div()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(t.space.sm))
            .child(caption(label, p))
            .child(card)
    };
    // A tool approval, as the conversation's dock shows it.
    let mut tool = Decision::new(
        "gallery-decision",
        Choice::new("gallery-allow", "Allow").key(Some(1)),
        Frame::Card { compact: false },
    );
    tool.headline = vec![
        ("Run".into(), false),
        ("git push origin main".into(), true),
        ("?".into(), false),
    ];
    tool.reason = Some("Publish the release branch".into());
    tool.folder = Some("/home/dev/src/workspacer".into());
    tool.host = Some("studio (this machine)".into());
    tool.risk = Some(Risk::Network);
    tool.grants = vec![Grant::new(
        Choice::new("gallery-always", "Always allow")
            .key(Some(3))
            .tooltip("Until this session ends, any command starting with “git push” runs"),
        "Always allow Bash(git push:*) this session",
        "git push origin main",
    )];
    tool.deny = Some(Choice::new("gallery-deny", "Deny").key(Some(2)));
    // R7.1: Deny with note, a visible Text button on 4.
    tool.deny_note = Some(Choice::new("gallery-deny-note", "Deny with note").key(Some(4)));
    tool.details = Some(Details::facts(
        Choice::new("gallery-details", "Details"),
        false,
        "gallery-details",
        "gallery-fact",
        vec![("Tool".into(), "Bash".into(), false)],
    ));
    tool.keys = true;

    // A question, answered on the card: its options send at once.
    let mut question = Decision::new(
        "gallery-question",
        Choice::new("gallery-question-send", SUBMIT_ANSWER),
        Frame::Inline,
    );
    question.headline = vec![("What should I call the new release branch?".into(), false)];
    question.show_allow = false;
    question.body = Some(
        div()
            .flex()
            .flex_col()
            .gap(px(t.space.xxs))
            .children(
                [("release/0.9", "Matches the milestone"), ("rc-0.9.0", "")]
                    .into_iter()
                    .enumerate()
                    .map(|(ix, (label, about))| {
                        prim::list_row(("gallery-option", ix), false, true, p)
                            .h_auto()
                            .py(px(t.space.xs))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .child(label)
                                    .when(!about.is_empty(), |d| {
                                        d.child(
                                            div()
                                                .text_size(px(t.text.meta))
                                                .text_color(rgb(p.roles.text_tertiary))
                                                .child(about),
                                        )
                                    }),
                            )
                            .child(prim::keycap_hint((ix + 1).to_string(), false, p))
                    }),
            )
            .into_any_element(),
    );

    // A plan: its "no" removes nothing, so it is Secondary.
    let mut plan = Decision::new(
        "gallery-plan",
        Choice::new("gallery-plan-approve", "Approve plan"),
        Frame::Card { compact: false },
    );
    plan.headline = vec![("Start on this plan?".into(), false)];
    plan.reason = Some("Nothing runs until you approve".into());
    plan.body = Some(
        prim::inset(p)
            .text_size(px(t.text.body))
            .text_color(rgb(p.roles.text_secondary))
            .child("1. Split the session store")
            .child("2. Move restore into the hub")
            .into_any_element(),
    );
    plan.deny = Some(Choice::new("gallery-plan-revise", "Keep planning"));
    plan.deny_variant = ButtonVariant::Secondary;

    // A job proposal, as a Fleet card shows it.
    let mut job = Decision::new(
        "gallery-job",
        Choice::new("gallery-job-approve", "Allow"),
        Frame::Inline,
    );
    job.headline = vec![("Schedule the job “Nightly tests”?".into(), false)];
    job.reason = Some("Runs the workspace tests · every day at 02:00".into());
    job.request = Some("$ cargo test --workspace".into());
    job.host = Some("studio (this machine)".into());
    job.deny = Some(Choice::new("gallery-job-reject", "Deny"));
    job.details = Some(Details::element(
        Choice::new("gallery-job-details", "Details"),
        false,
        None,
    ));

    // r6-decisions (R6.3): a request that deletes files, pressed once: its
    // risk chip, and Allow asking for the second press.
    let mut destructive = Decision::new(
        "gallery-destructive",
        Choice::new("gallery-destructive-allow", "Allow").key(Some(1)),
        Frame::Inline,
    );
    destructive.headline = vec![
        ("Run".into(), false),
        ("rm -rf target/debug".into(), true),
        ("?".into(), false),
    ];
    destructive.folder = Some("/home/dev/src/workspacer".into());
    destructive.risk = Some(Risk::Deletes);
    destructive.armed = true;
    destructive.deny = Some(Choice::new("gallery-destructive-deny", "Deny").key(Some(2)));
    destructive.keys = true;

    // R7.1: the focused decision while its keys type into the composer:
    // keycaps dimmed. The batch is the positive answer's variant, Shift+1.
    let mut elsewhere = Decision::new(
        "gallery-keys-elsewhere",
        Choice::new("gallery-elsewhere-allow", "Allow").key(Some(1)),
        Frame::Inline,
    );
    elsewhere.headline = vec![("Run".into(), false), ("cargo test".into(), true)];
    elsewhere.deny = Some(Choice::new("gallery-elsewhere-deny", "Deny").key(Some(2)));
    elsewhere.alternates =
        vec![Choice::new("gallery-batch", "Allow all 3 · cargo test").variant_key()];
    elsewhere.keys = true;
    elsewhere.keys_live = false;

    // Deny with note: the note replaces the answer row.
    let mut noting = Decision::new(
        "gallery-noting",
        Choice::new("gallery-noting-allow", "Allow").key(Some(1)),
        Frame::Inline,
    );
    noting.headline = vec![("Run".into(), false), ("cargo test".into(), true)];
    noting.deny = Some(Choice::new("gallery-noting-deny", "Deny").key(Some(2)));
    noting.note = Some(
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(t.space.sm))
            .child(div().w_full().min_w_0().child(
                super::composer_field(ComposerState::Focus, p).child("Run only the UI tests"),
            ))
            .child(prim::button(
                "gallery-noting-cancel",
                "Cancel",
                None,
                ButtonVariant::Tertiary,
                ButtonSize::Small,
                true,
                p,
            ))
            .child(prim::button(
                "gallery-noting-send",
                "Deny and tell it",
                None,
                ButtonVariant::Destructive,
                ButtonSize::Small,
                true,
                p,
            ))
            .into_any_element(),
    );
    noting.allow_primary = false;
    noting.keys = true;

    // A table row's answer (the Fleet table).
    let mut row_frame = Decision::new(
        "gallery-row",
        Choice::new("gallery-row-allow", "Allow").key(Some(1)),
        Frame::Row,
    );
    row_frame.deny = Some(Choice::new("gallery-row-deny", "Deny").key(Some(2)));
    row_frame.keys = true;

    section("DecisionCard", p)
        .debug_selector(|| "gallery-decisions".into())
        .child(
            super::tile_grid(4, p)
                .child(specimen("Tool approval · keys on", tool.render(p)))
                .child(specimen(
                    "Question · answered on the card",
                    super::waiting_tile(p).child(question.render(p)),
                ))
                .child(specimen("Plan", plan.render(p)))
                .child(specimen(
                    "Job proposal · Fleet card",
                    super::waiting_tile(p).child(job.render(p)),
                ))
                .child(specimen(
                    "Deletes files · pressed once",
                    super::waiting_tile(p).child(destructive.render(p)),
                ))
                .child(specimen(
                    "Keys type elsewhere · dimmed; batch on ⇧1",
                    super::waiting_tile(p).child(elsewhere.render(p)),
                ))
                .child(specimen(
                    "Deny with note · replaces the answer",
                    super::waiting_tile(p).child(noting.render(p)),
                ))
                .child(specimen(
                    "Table row · Frame::Row; SegmentedControl",
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(t.space.md))
                        .child(row_frame.render(p))
                        .child(
                            super::segmented_control(p)
                                .child(
                                    super::Segment::new("gallery-seg-needs", "Needs you")
                                        .count(3)
                                        .selected(true)
                                        .render(p),
                                )
                                .child(
                                    super::Segment::new("gallery-seg-active", "Active")
                                        .count(5)
                                        .render(p),
                                )
                                .child(
                                    super::Segment::new("gallery-seg-all", "All")
                                        .count(30)
                                        .render(p),
                                ),
                        ),
                )),
        )
}

/// TextField / Composer (R5.4): the one composer, in its states. Static
/// specimens: the live one needs the app's `InputState`.
fn composers(p: &Palette) -> Div {
    let t = p.tokens;
    let specimen = |id: &'static str,
                    state: ComposerState,
                    text: &'static str,
                    placeholder: bool,
                    deciding: bool,
                    can_send: bool,
                    note: &'static str| {
        div()
            .flex()
            .flex_col()
            .gap(px(t.space.xs))
            .child(composer_specimen(
                id,
                state,
                text,
                placeholder,
                deciding,
                can_send,
                p,
            ))
            .child(caption(note, p))
    };
    section("Composer", p)
        .debug_selector(|| "gallery-composers".into())
        .child(specimen(
            "gallery-composer-rest",
            ComposerState::Rest,
            "Ask anything, or describe a task…",
            true,
            false,
            false,
            "Rest, empty: Send is the disabled Primary",
        ))
        .child(specimen(
            "gallery-composer-focus",
            ComposerState::Focus,
            "Run only the UI tests first",
            false,
            false,
            true,
            "Focus with a draft: Send is Primary",
        ))
        .child(specimen(
            "gallery-composer-yielded",
            ComposerState::Yielded,
            "Reply — the request stays open",
            true,
            true,
            true,
            "A decision waits: still a bordered field; Send is Secondary",
        ))
        // r8-conversation (R8.4): the decision-and-reply area's parts.
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(t.space.xs))
                .child(super::card_reply(
                    super::edge(
                        div()
                            .w_full()
                            .px(px(t.space.md))
                            .py(px(t.space.xs))
                            .rounded(px(t.radius.control))
                            .text_color(rgb(p.text))
                            .child("Run only the UI tests first"),
                        super::Side::All,
                        super::EdgeTone::Strong,
                        p,
                    ),
                    "The request stays open until you answer it",
                    [
                        prim::button_specimen(
                            "Cancel",
                            ButtonVariant::Tertiary,
                            ButtonSize::Small,
                            ControlState::Rest,
                            p,
                        )
                        .into_any_element(),
                        prim::button_specimen(
                            "Send to Claude",
                            ButtonVariant::Secondary,
                            ButtonSize::Small,
                            ControlState::Rest,
                            p,
                        )
                        .into_any_element(),
                    ],
                    p,
                ))
                .child(caption(
                    "R8.4: the card's reply (last in the card; the composer is hidden). Its send names where the text goes",
                    p,
                )),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(t.space.xs))
                .child(super::decision_line(
                    "Run cargo test in native?",
                    prim::button_specimen(
                        "Open",
                        ButtonVariant::Secondary,
                        ButtonSize::Small,
                        ControlState::Rest,
                        p,
                    ),
                    p,
                ))
                .child(caption(
                    "R8.4: the card folded to one line while reading back; it replaces Jump to latest",
                    p,
                )),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(t.space.xs))
                .child(
                    div().flex().child(super::updates_chip(
                        "gallery-updates-chip",
                        Some(prim::StatusKind::NeedsYou),
                        "2 updates",
                        p,
                    )),
                )
                .child(super::updates_popover(
                    [
                        super::update_row(
                            "gallery-update-0",
                            Some(prim::StatusKind::Failed),
                            "Queue priority ordering failed: credit balance is too low",
                            None,
                            p,
                        )
                        .into_any_element(),
                        super::update_row(
                            "gallery-update-1",
                            Some(prim::StatusKind::NeedsYou),
                            "Release checklist for 0.9 needs you",
                            Some(
                                prim::button_specimen(
                                    "Open",
                                    ButtonVariant::Tertiary,
                                    ButtonSize::Small,
                                    ControlState::Rest,
                                    p,
                                )
                                .into_any_element(),
                            ),
                            p,
                        )
                        .into_any_element(),
                    ],
                    p,
                ))
                .child(caption(
                    "R8.4: other sessions' news, a header chip; its popover is one row per event, failures first",
                    p,
                )),
        )
        .child(specimen(
            "gallery-composer-disabled",
            ComposerState::Disabled,
            "Reconnecting…",
            true,
            false,
            false,
            "Disabled: offline or busy",
        ))
}
