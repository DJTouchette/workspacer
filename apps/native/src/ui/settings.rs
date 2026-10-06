//! Settings: a category rail beside grouped cards, with a search box that
//! filters every preference across categories by title, description and
//! keywords.
use super::*;
use gpui::AnyElement;
use gpui_component::{Disableable, switch::Switch};
use wks_native::features::Request;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum SettingsSection {
    #[default]
    Appearance,
    Typography,
    Workspace,
    Remote,
    Agents,
    Chat,
    Keyboard,
    About,
}

impl SettingsSection {
    pub(super) const ALL: [Self; 8] = [
        Self::Appearance,
        Self::Typography,
        Self::Workspace,
        Self::Remote,
        Self::Agents,
        Self::Chat,
        Self::Keyboard,
        Self::About,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Typography => "Typography",
            Self::Workspace => "Workspace",
            Self::Remote => "Remote",
            Self::Agents => "Agents",
            Self::Chat => "Chat",
            Self::Keyboard => "Keyboard",
            Self::About => "About",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Appearance => "Choose the palette for your workspace.",
            Self::Typography => "Fonts and reading size. Saved on this device.",
            Self::Workspace => "How the app fits into your day.",
            Self::Remote => "Reach this machine from your phone over Tailscale.",
            Self::Agents => "Defaults for new sessions. Existing sessions keep theirs.",
            Self::Chat => "How conversations and tool calls read.",
            Self::Keyboard => "Move around your workspace at your own pace.",
            Self::About => "Version and updates.",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Appearance => IconName::Palette,
            Self::Typography => IconName::CaseSensitive,
            Self::Workspace => IconName::LayoutDashboard,
            Self::Remote => IconName::Globe,
            Self::Agents => IconName::Bot,
            Self::Chat => IconName::Inbox,
            Self::Keyboard => IconName::SquareTerminal,
            Self::About => IconName::Info,
        }
    }
}

/// Every whitespace-separated term must appear in one of the haystacks,
/// case-insensitively; an empty query matches everything.
pub(super) fn settings_match(query: &str, haystacks: &[&str]) -> bool {
    let haystacks: Vec<String> = haystacks.iter().map(|h| h.to_lowercase()).collect();
    query
        .split_whitespace()
        .map(str::to_lowercase)
        .all(|term| haystacks.iter().any(|h| h.contains(&term)))
}

enum Layout {
    /// Title and description left, control right.
    Row,
    /// Title and description above a full-width control.
    Block,
}

struct Entry {
    section: SettingsSection,
    id: &'static str,
    title: &'static str,
    description: &'static str,
    keywords: &'static str,
    layout: Layout,
    control: AnyElement,
}

impl Entry {
    fn new(
        section: SettingsSection,
        id: &'static str,
        title: &'static str,
        description: &'static str,
        keywords: &'static str,
        layout: Layout,
        control: impl IntoElement,
    ) -> Self {
        Self {
            section,
            id,
            title,
            description,
            keywords,
            layout,
            control: control.into_any_element(),
        }
    }

    fn matches(&self, query: &str) -> bool {
        settings_match(
            query,
            &[
                self.title,
                self.description,
                self.keywords,
                self.section.label(),
            ],
        )
    }
}

impl Workspace {
    /// Pill-group choice used for small enumerations (text size, agent).
    pub(super) fn segmented<T: Copy + PartialEq + 'static>(
        &self,
        id: &'static str,
        options: Vec<(T, String)>,
        selected: T,
        on_pick: impl Fn(&mut Self, T, &mut Window, &mut Context<Self>) + Clone + 'static,
        cx: &mut Context<Self>,
    ) -> Div {
        self.segmented_enabled(id, options, selected, true, on_pick, cx)
    }

    /// `segmented` that can be disabled as a whole (while a request runs).
    /// Every option shares one height and centers its label.
    pub(super) fn segmented_enabled<T: Copy + PartialEq + 'static>(
        &self,
        id: &'static str,
        options: Vec<(T, String)>,
        selected: T,
        enabled: bool,
        on_pick: impl Fn(&mut Self, T, &mut Window, &mut Context<Self>) + Clone + 'static,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        div()
            .flex_shrink_0()
            .flex()
            .gap_1()
            .p(px(3.))
            .rounded_full()
            .bg(rgb(p.base))
            .border_1()
            .border_color(rgb(p.border))
            .children(options.into_iter().enumerate().map(|(ix, (value, label))| {
                let active = value == selected;
                let on_pick = on_pick.clone();
                chrome::interactive_control(div().id((id, ix)), p, enabled)
                    .debug_selector(move || format!("{id}-{ix}"))
                    .h(px(28.))
                    .min_w(px(44.))
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .text_size(px(12.))
                    .line_height(px(16.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(match (enabled, active) {
                        (false, _) => p.disabled,
                        (true, true) => p.text,
                        (true, false) => p.muted,
                    }))
                    .when(active, |d| d.bg(rgb(p.selected)))
                    .child(label)
                    .when(enabled, |d| {
                        d.cursor_pointer()
                            .hover(move |s| s.text_color(rgb(p.text)))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                on_pick(this, value, window, cx)
                            }))
                    })
            }))
    }

    fn settings_entries(&self, cx: &mut Context<Self>) -> Vec<Entry> {
        use SettingsSection as S;
        let p = self.appearance.palette();
        let mut entries = Vec::new();

        let themes = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                // Eight palettes: equal tiles that wrap rather than one row
                // squeezed to unreadable previews in a narrow window.
                div()
                    .debug_selector(|| "theme-picker".into())
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .children(Appearance::ALL.into_iter().map(|appearance| {
                        let colors = appearance.palette();
                        let active = self.appearance == appearance;
                        chrome::interactive_control(div().id(appearance.label()), p, true)
                            .debug_selector(move || format!("theme-{}", appearance.label()))
                            .w(px(148.))
                            .flex_shrink_0()
                            .rounded(px(p.panel_radius))
                            .overflow_hidden()
                            .cursor_pointer()
                            .border_color(if active { rgb(p.accent) } else { gpui::rgba(0) })
                            .bg(rgb(p.base))
                            .hover(|style| style.bg(rgb(p.selected)))
                            .child(
                                div()
                                    .m_2()
                                    .h(px(64.))
                                    .rounded(px(p.control_radius))
                                    .bg(rgb(colors.chat))
                                    .flex()
                                    .overflow_hidden()
                                    .child(
                                        div()
                                            .w(px(24.))
                                            .h_full()
                                            .bg(rgb(colors.base))
                                            .p_2()
                                            .child(status_dot(colors.accent)),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .p_2()
                                            .flex()
                                            .flex_col()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .h(px(6.))
                                                    .w(px(36.))
                                                    .rounded_full()
                                                    .bg(rgb(colors.text)),
                                            )
                                            .child(
                                                div()
                                                    .h(px(4.))
                                                    .w_full()
                                                    .rounded_full()
                                                    .bg(rgb(colors.border)),
                                            )
                                            .child(
                                                div()
                                                    .h(px(14.))
                                                    .w_full()
                                                    .rounded(px(4.))
                                                    .bg(rgb(colors.surface)),
                                            ),
                                    ),
                            )
                            .child(
                                div()
                                    .px_3()
                                    .pb_3()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(px(12.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(rgb(if active { p.accent } else { p.text }))
                                            .child(appearance.label()),
                                    )
                                    .when(active, |d| {
                                        d.child(
                                            Icon::new(IconName::Check)
                                                .size(px(14.))
                                                .text_color(rgb(p.accent)),
                                        )
                                    }),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.choose_theme(appearance, window, cx)
                            }))
                    })),
            )
            .when(!self.theme_error.is_empty(), |d| {
                d.child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(p.warning))
                        .child(self.theme_error.clone()),
                )
            });
        entries.push(Entry::new(
            S::Appearance,
            "theme",
            "Theme",
            "Dark, Light, Nord, or Omarchy's Tokyo Night, Catppuccin Mocha and Latte, Gruvbox and Everforest. Press t in Normal mode to cycle.",
            "palette color colour dark light nord mode omarchy tokyo night catppuccin mocha latte gruvbox everforest",
            Layout::Block,
            themes,
        ));

        entries.push(Entry::new(
            S::Appearance,
            "interface-size",
            "Interface size",
            "Scales the whole app on top of your display's scaling. Ctrl/Cmd + and − step it; Ctrl/Cmd 0 resets.",
            "zoom scale dpi size bigger smaller tiny huge display resolution",
            Layout::Block,
            self.segmented(
                "interface-size",
                wks_native::navigation::INTERFACE_SCALES
                    .into_iter()
                    .map(|pct| (pct, format!("{pct}%")))
                    .collect(),
                self.settings.interface_scale,
                |this, pct, window, cx| this.set_interface_scale(pct, window, cx),
                cx,
            ),
        ));
        entries.push(Entry::new(
            S::Appearance,
            "reduce-motion",
            "Reduce motion",
            "The title capsule reveals its actions and grows around notices at once, without springing into shape. Fades stay.",
            "motion animation animate spring bounce reduce reduced accessibility island",
            Layout::Row,
            Switch::new("settings-reduce-motion")
                .checked(self.settings.reduce_motion)
                .tooltip("Reduce motion")
                .on_click(cx.listener(|this, checked, _, cx| {
                    this.settings.reduce_motion = *checked;
                    this.save_settings(cx);
                })),
        ));

        entries.push(Entry::new(
            S::Typography,
            "interface-font",
            "Interface font",
            "Used for the app and conversation text.",
            "font family typeface ui sans",
            Layout::Block,
            Select::new(&self.fonts.interface).w_full(),
        ));
        entries.push(Entry::new(
            S::Typography,
            "code-font",
            "Code font",
            "Used for code, commands and paths.",
            "font family typeface monospace mono",
            Layout::Block,
            Select::new(&self.fonts.code).w_full(),
        ));
        entries.push(Entry::new(
            S::Typography,
            "text-size",
            "Conversation text size",
            "Body size for messages; headings scale with it.",
            "font size zoom scale bigger smaller reading",
            Layout::Row,
            self.segmented(
                "text-size",
                [13u8, 15, 17, 19]
                    .into_iter()
                    .map(|size| (size, format!("{size}")))
                    .collect(),
                self.settings.text_size,
                |this, size, window, cx| {
                    this.settings.text_size = size;
                    this.apply_typography(cx);
                    this.save_settings(cx);
                    window.refresh();
                },
                cx,
            ),
        ));
        entries.push(Entry::new(
            S::Typography,
            "preview",
            "Preview",
            "How your fonts read together.",
            "sample example",
            Layout::Block,
            div()
                .p_3()
                .rounded(px(p.control_radius))
                .bg(rgb(p.chat))
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .font_family(self.fonts.resolved(&self.settings.interface_font, "Inter"))
                        .text_size(px(self.settings.text_size as f32))
                        .child("The quick brown fox jumps over the lazy dog."),
                )
                .child(
                    div()
                        .font_family(
                            self.fonts
                                .resolved(&self.settings.code_font, "JetBrains Mono"),
                        )
                        .text_size(px((self.settings.text_size as f32 - 2.).max(12.)))
                        .text_color(rgb(p.accent))
                        .child("const workspace = await connect();"),
                ),
        ));
        entries.push(Entry::new(
            S::Typography,
            "reset-typography",
            "Reset typography",
            "Inter, JetBrains Mono and 15 px.",
            "default restore fonts",
            Layout::Row,
            self.quiet_button("reset-fonts", "Reset", IconName::Undo2, true)
                .on_click(cx.listener(|this, _, window, cx| {
                    let defaults = Settings::default();
                    this.settings.interface_font = defaults.interface_font;
                    this.settings.code_font = defaults.code_font;
                    this.settings.text_size = defaults.text_size;
                    this.fonts.sync(&this.settings, window, cx);
                    this.apply_typography(cx);
                    this.save_settings(cx);
                    window.refresh();
                })),
        ));

        entries.push(Entry::new(
            S::Workspace,
            "keep-running",
            "Keep running when closed",
            "Minimize to the taskbar or Dock. Use Quit to stop the local backend.",
            "background tray minimize close quit",
            Layout::Row,
            Switch::new("settings-background")
                .checked(self.settings.keep_running)
                .tooltip("Keep running when closed")
                .on_click(cx.listener(|this, checked, _, cx| {
                    this.settings.keep_running = *checked;
                    this.extras.keep_running.set(*checked);
                    this.save_settings(cx);
                })),
        ));
        entries.push(Entry::new(
            S::Workspace,
            "notifications",
            "Notifications",
            "Completion, approval and question alerts while this window is inactive.",
            "alerts notify desktop approval question done",
            Layout::Row,
            Switch::new("settings-notifications")
                .checked(self.settings.notifications)
                .tooltip("Notifications")
                .on_click(cx.listener(|this, checked, _, cx| {
                    this.settings.notifications = *checked;
                    this.save_settings(cx);
                })),
        ));
        entries.push(Entry::new(
            S::Workspace,
            "agent-setup",
            "Agent setup",
            "Install and connect the agents on your workspace host.",
            "install claude codex providers login connect",
            Layout::Row,
            self.quiet_button("settings-setup", "Set up", IconName::ArrowRight, true)
                .on_click(
                    cx.listener(|this, _, window, cx| this.open_feature(Screen::Setup, window, cx)),
                ),
        ));

        entries.push(Entry::new(
            S::Remote,
            "tailscale",
            "Tailscale HTTPS",
            "Serve Workspacer to devices on your tailnet with Tailscale Serve. Nothing is exposed to the public internet.",
            "remote phone mobile share tailscale serve https tailnet network",
            Layout::Block,
            self.render_remote_sharing(cx),
        ));
        entries.push(Entry::new(
            S::Remote,
            "pair-phone",
            "Pair a phone",
            "Each access level has its own link. Choose the least access you need.",
            "remote phone mobile pair qr code token scope link",
            Layout::Block,
            self.render_phone_pairing(cx),
        ));
        entries.push(Entry::new(
            S::Remote,
            "paired-devices",
            "Pairing links",
            "Links you've created on this machine.",
            "remote phone devices tokens revoke pairing",
            Layout::Block,
            self.render_paired_devices(cx),
        ));

        entries.push(Entry::new(
            S::Agents,
            "default-agent",
            "Default agent",
            "Provider for new sessions. Press a in Normal mode to switch.",
            "provider claude codex new session",
            Layout::Row,
            self.segmented(
                "default-agent",
                vec![
                    (Provider::Claude, "Claude".into()),
                    (Provider::Codex, "Codex".into()),
                ],
                self.settings.default_provider,
                |this, provider, _, cx| {
                    this.settings.default_provider = provider;
                    this.save_settings(cx);
                },
                cx,
            ),
        ));
        for (provider, label, title, id) in [
            ("claude", "Claude", "Claude access mode", "claude-access"),
            ("codex", "Codex", "Codex access mode", "codex-access"),
        ] {
            let selected = self.settings.default_access(provider);
            entries.push(Entry::new(
                S::Agents,
                id,
                title,
                if label == "Claude" {
                    "Used when starting a Claude agent. Override it per session."
                } else {
                    "Used when starting a Codex agent. Override it per session."
                },
                "permission approval access bypass full yolo ask plan",
                Layout::Block,
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        self.segmented(
                            id,
                            Permission::choices(provider)
                                .iter()
                                .copied()
                                .map(|access| (access, access.label().to_owned()))
                                .collect(),
                            selected,
                            move |this, access, _, cx| {
                                if provider == "claude" {
                                    this.settings.default_claude_access = access;
                                } else {
                                    this.settings.default_codex_access = access;
                                }
                                this.save_settings(cx);
                            },
                            cx,
                        ),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(if selected == Permission::FullAccess {
                                p.warning
                            } else {
                                p.muted
                            }))
                            .child(selected.description()),
                    ),
            ));
        }

        entries.push(Entry::new(
            S::Agents,
            "auto-title",
            "Name sessions automatically",
            "After a session's first answer, the hub names it from what you asked. A name you type is never replaced. Saved in the hub's shared settings.",
            "title name rename automatic auto session label haiku",
            Layout::Row,
            self.render_titles_switch(cx),
        ));
        entries.push(Entry::new(
            S::Agents,
            "title-model",
            "Title model",
            "Which provider and model write those names. Keep it cheap: a title is a few words.",
            "title name automatic provider model haiku mini cheap claude codex",
            Layout::Block,
            self.render_title_model(cx),
        ));
        entries.push(Entry::new(
            S::Agents,
            "child-access",
            "Child agents start with full access",
            "Agents your sessions start (through the spawn skill or Workspacer tools) skip the provider's own approval prompts: Claude runs in bypass-permissions mode, Codex with full access. Saved in the hub's shared settings, so every client of this hub sees it. Workspacer's own approval gate, tool access and your existing sessions are unchanged; a resumed session keeps its mode.",
            "child subagent spawn worker full access bypass permissions yolo approvals prompts",
            Layout::Block,
            self.render_child_access(cx),
        ));
        entries.push(Entry::new(
            S::Chat,
            "merge-turn",
            "One card per turn",
            "Group a turn's tool calls into a single card, with the agent's notes between them inside it.",
            "tools work card group merge collapse steps",
            Layout::Row,
            Switch::new("settings-merge-turn")
                .checked(self.settings.merge_turn_tools)
                .tooltip("One card per turn")
                .on_click(cx.listener(|this, checked, _, cx| {
                    this.settings.merge_turn_tools = *checked;
                    this.list.splice(0..this.view.transcript.rows.len(), this.view.transcript.rows.len());
                    this.save_settings(cx);
                })),
        ));
        entries.push(Entry::new(
            S::Chat,
            "clock",
            "12-hour clock",
            "Show AM / PM instead of a 24-hour clock.",
            "time timestamp am pm format",
            Layout::Row,
            Switch::new("settings-clock")
                .checked(self.settings.twelve_hour_clock)
                .tooltip("12-hour clock")
                .on_click(cx.listener(|this, checked, _, cx| {
                    this.settings.twelve_hour_clock = *checked;
                    this.list.splice(
                        0..this.view.transcript.rows.len(),
                        this.view.transcript.rows.len(),
                    );
                    this.save_settings(cx);
                })),
        ));

        entries.push(Entry::new(
            S::Keyboard,
            "vim",
            "Vim navigation",
            "Normal mode for navigation, Insert mode for typing. Press v to toggle.",
            "vim keys modal normal insert hjkl",
            Layout::Row,
            Switch::new("toggle-vim")
                .checked(self.settings.vim_navigation)
                .tooltip("Vim navigation")
                .on_click(cx.listener(|this, checked, window, cx| {
                    this.settings.vim_navigation = *checked;
                    window.focus(&this.focus);
                    this.save_settings(cx);
                })),
        ));
        entries.push(Entry::new(
            S::Keyboard,
            "send-key",
            "Send messages with",
            "The composer's send key. The other combination adds a new line.",
            "enter send submit newline shift ctrl cmd composer keyboard shortcut",
            Layout::Row,
            self.segmented(
                "send-key",
                vec![
                    (
                        false,
                        if cfg!(target_os = "macos") {
                            "⌘ Enter".into()
                        } else {
                            "Ctrl Enter".into()
                        },
                    ),
                    (true, "Enter".into()),
                ],
                self.settings.enter_sends,
                |this, enter_sends, _, cx| {
                    this.settings.enter_sends = enter_sends;
                    this.save_settings(cx);
                },
                cx,
            ),
        ));
        entries.push(Entry::new(
            S::Keyboard,
            "shortcuts",
            "Shortcuts",
            "Vim shortcuts apply in Normal mode. Text fields keep ordinary editing keys.",
            "keys keybindings hotkeys shortcuts reference",
            Layout::Block,
            div().flex().flex_col().gap_2().children(
                [
                    (
                        "Esc",
                        "Leave a text field; press again to return to the conversation",
                    ),
                    (
                        "j / k",
                        "Next / previous session, project or settings category",
                    ),
                    ("gg / G", "First / last session or project"),
                    (
                        "i",
                        "Compose, edit the new-session form, or add a project path",
                    ),
                    ("/", "Filter sessions and projects, or search settings"),
                    ("g p / h", "Projects"),
                    ("Enter / l", "Open the selected project"),
                    ("g h / g d", "Session history / changes"),
                    ("g a / g e / g m", "Agent setup / session / model"),
                    ("g s", "Settings"),
                    ("g c", "Conversation"),
                    ("n", "New session in the selected project"),
                    (
                        "t / v / a",
                        "Settings: cycle theme / toggle Vim / switch default agent",
                    ),
                    ("Ctrl U / D", "Scroll conversation up / down"),
                    ("Ctrl/Cmd ,", "Settings (also when Vim navigation is off)"),
                    ("Ctrl/Cmd P", "Projects (also when Vim navigation is off)"),
                    (
                        "Ctrl/Cmd Enter",
                        "Send message, create session, or save a project path",
                    ),
                    (
                        "Enter / Shift Enter",
                        "With Send messages with Enter: send / new line in the composer",
                    ),
                    ("Ctrl/Cmd L", "Focus the editor"),
                    (
                        "Ctrl/Cmd + / − / 0",
                        "Larger / smaller / default interface size",
                    ),
                ]
                .into_iter()
                .map(|(keys, label)| {
                    div()
                        .flex()
                        .gap_3()
                        .items_start()
                        .child(keycap(keys, p).w(px(120.)).flex_shrink_0())
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(p.muted))
                                .child(label),
                        )
                }),
            ),
        ));

        entries.push(Entry::new(
            S::About,
            "updates",
            "Updates",
            "Follows the channel this build came from: nightly builds track the rolling nightly, stable builds the latest release.",
            "version release upgrade check download changelog nightly stable install",
            Layout::Block,
            self.render_update_card(cx),
        ));
        entries
    }

    fn render_child_access(&self, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let state = self.view.requests.get("child-access");
        let pending = state.is_some_and(|s| s.loading);
        let can_write = self.view.connected && !self.demo && !pending;
        let current = self.extras.child_access;
        let error = state.filter(|s| !s.loading).and_then(|s| s.error.clone());
        let (status, tone) = match (current, &error) {
            (_, Some(error)) => (
                format!("Couldn't reach the hub's setting: {error}"),
                chrome::Tone::Error,
            ),
            (None, None) if pending || self.view.connected => {
                ("Reading the hub's setting…".to_owned(), chrome::Tone::Loading)
            }
            (None, None) => (
                "Connect to a hub to change this.".to_owned(),
                chrome::Tone::Info,
            ),
            (Some((true, _)), None) => (
                "On: new child agents skip provider approval prompts.".to_owned(),
                chrome::Tone::Warning,
            ),
            (Some((false, true)), None) => (
                "Off. Fleet Manager workers still start with full access (the hub's Fleet setting).".to_owned(),
                chrome::Tone::Info,
            ),
            (Some((false, false)), None) => (
                "Off: child agents ask before edits and commands, as their provider does.".to_owned(),
                chrome::Tone::Info,
            ),
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .debug_selector(|| "child-access-switch".into())
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        Switch::new("settings-child-access")
                            .checked(current.is_some_and(|(on, _)| on))
                            .disabled(!can_write || current.is_none())
                            .tooltip("Child agents start with full access")
                            .on_click(cx.listener(|this, checked, _, cx| {
                                this.request(
                                    Request::ChildAccess {
                                        set: Some(*checked),
                                    },
                                    cx,
                                );
                            })),
                    ),
            )
            .child(chrome::notice_line(status, tone, p, "child-access-status"))
    }

    fn render_entry(&self, entry: Entry, first: bool) -> Div {
        let p = self.appearance.palette();
        let id = entry.id;
        let text = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(entry.title),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(p.muted))
                    .child(entry.description),
            );
        div()
            .debug_selector(move || format!("setting-{id}"))
            .px_4()
            .py_3()
            .when(!first, |d| d.border_t_1().border_color(rgb(p.border)))
            .map(|d| match entry.layout {
                Layout::Row => d
                    .flex()
                    .items_center()
                    .gap_4()
                    .child(text)
                    .child(entry.control),
                Layout::Block => d.flex().flex_col().gap_3().child(text).child(entry.control),
            })
    }

    fn settings_card(&self, entries: Vec<Entry>) -> Div {
        let p = self.appearance.palette();
        div()
            .rounded(px(p.panel_radius))
            .border_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.surface))
            .overflow_hidden()
            .children(
                entries
                    .into_iter()
                    .enumerate()
                    .map(|(ix, e)| self.render_entry(e, ix == 0)),
            )
    }

    pub(super) fn render_settings(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let short = window.viewport_size().height < px(620.);
        // Beside a wide sidebar the category rail shrinks to icons, so the
        // preferences themselves keep a readable measure.
        let viewport = unzoom(window.viewport_size().width);
        let sidebar = if self.sidebar_collapsed {
            56.
        } else {
            wks_native::navigation::sidebar_width(self.settings.sidebar_width, viewport)
        };
        let tight = viewport - sidebar < 820.;
        let query = self.settings_search.read(cx).value().trim().to_owned();
        let searching = !query.is_empty();
        let entries = self.settings_entries(cx);
        let counts: Vec<usize> = SettingsSection::ALL
            .iter()
            .map(|section| {
                entries
                    .iter()
                    .filter(|e| e.section == *section && e.matches(&query))
                    .count()
            })
            .collect();
        let rail = div()
            .debug_selector(|| "settings-rail".into())
            .w(px(if tight { 40. } else { 200. }))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_1()
            .children(
                SettingsSection::ALL
                    .into_iter()
                    .zip(counts.iter().copied())
                    .map(|(section, count)| {
                        let active = !searching && self.settings_section == section;
                        let dimmed = searching && count == 0;
                        let label = section.label();
                        chrome::interactive_control(div().id(section.label()), p, true)
                            .debug_selector(move || format!("settings-nav-{}", section.label()))
                            .h(px(34.))
                            .map(|d| if tight { d.justify_center() } else { d.px_3() })
                            .when(tight, |d| {
                                d.tooltip(move |window, cx| {
                                    gpui_component::tooltip::Tooltip::new(label).build(window, cx)
                                })
                            })
                            .rounded(px(p.control_radius))
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(13.))
                            .font_weight(if active {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::MEDIUM
                            })
                            .text_color(rgb(if active {
                                p.text
                            } else if dimmed {
                                p.disabled
                            } else {
                                p.muted
                            }))
                            .when(active, |d| d.bg(rgb(p.selected)))
                            .hover(move |s| s.bg(rgb(p.selected)).text_color(rgb(p.text)))
                            .child(Icon::new(section.icon()).size(px(15.)).text_color(rgb(
                                if active {
                                    p.accent
                                } else if dimmed {
                                    p.disabled
                                } else {
                                    p.muted
                                },
                            )))
                            .when(!tight, |d| d.child(div().flex_1().child(section.label())))
                            .when(!tight && searching && count > 0, |d| {
                                d.child(
                                    div()
                                        .px(px(6.))
                                        .rounded_full()
                                        .bg(gpui::Hsla::from(rgb(p.accent)).opacity(0.15))
                                        .text_size(px(11.))
                                        .text_color(rgb(p.accent))
                                        .child(count.to_string()),
                                )
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.enter_settings_section(section, cx);
                                this.settings_search
                                    .update(cx, |input, cx| input.set_value("", window, cx));
                                // Keep Normal-mode keys (j / k, /) live after a click.
                                window.focus(&this.focus);
                                cx.notify();
                            }))
                    }),
            );
        let search = div()
            .debug_selector(|| "settings-search".into())
            .h(px(40.))
            .px_3()
            .rounded(px(p.control_radius))
            .border_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.surface))
            .flex()
            .items_center()
            .gap_2()
            .child(
                Icon::new(IconName::Search)
                    .size(px(15.))
                    .text_color(rgb(p.muted)),
            )
            .child(
                div()
                    .flex_1()
                    .child(Input::new(&self.settings_search).appearance(false)),
            )
            .child(keycap("/", p));
        let mut content = div().flex_1().min_w_0().flex().flex_col().gap_4();
        if searching {
            let mut any = false;
            let mut entries = entries;
            for section in SettingsSection::ALL {
                let (hits, rest): (Vec<Entry>, Vec<Entry>) = entries
                    .into_iter()
                    .partition(|e| e.section == section && e.matches(&query));
                entries = rest;
                if hits.is_empty() {
                    continue;
                }
                any = true;
                content = content.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(overline(section.label().to_uppercase(), p))
                        .child(self.settings_card(hits)),
                );
            }
            if !any {
                content = content.child(
                    div()
                        .debug_selector(|| "settings-no-results".into())
                        .py_10()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_2()
                        .child(
                            Icon::new(IconName::Search)
                                .size(px(22.))
                                .text_color(rgb(p.disabled)),
                        )
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .child(format!("No settings match “{query}”")),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(p.muted))
                                .child("Try a broader word, like “font”, “theme” or “keys”."),
                        ),
                );
            }
        } else {
            let section = self.settings_section;
            content = content
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(20.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(section.label()),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(p.muted))
                                .child(section.description()),
                        ),
                )
                .child(
                    self.settings_card(
                        entries
                            .into_iter()
                            .filter(|e| e.section == section)
                            .collect(),
                    ),
                );
        }
        let title = div()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_1()
            .child(overline("MAKE IT YOURS", p))
            .child(
                div()
                    .debug_selector(|| "page-title".into())
                    .text_size(px(if short {
                        chrome::scale::TITLE_SHORT
                    } else {
                        chrome::scale::TITLE
                    }))
                    .font_weight(FontWeight::BOLD)
                    .child("Settings"),
            );
        self.page_view(
            "settings-view",
            920.,
            short,
            div()
                .flex()
                .flex_col()
                .gap_5()
                .map(|d| {
                    if tight {
                        d.child(title).child(search)
                    } else {
                        d.child(
                            div()
                                .flex()
                                .items_end()
                                .gap_4()
                                .child(title.w(px(200.)))
                                .child(div().flex_1().min_w_0().child(search)),
                        )
                    }
                })
                .when(!self.settings_error.is_empty(), |d| {
                    d.child(chrome::notice_line(
                        self.settings_error.clone(),
                        chrome::Tone::Error,
                        p,
                        "settings-error",
                    ))
                })
                .child(
                    div()
                        .flex()
                        .items_start()
                        .gap(px(if tight { 12. } else { 16. }))
                        .child(rail)
                        .child(content),
                ),
        )
    }

    /// j / k in Normal mode step through categories.
    pub(super) fn step_settings_section(&mut self, step: isize, cx: &mut Context<Self>) {
        let all = SettingsSection::ALL;
        let ix = all
            .iter()
            .position(|s| *s == self.settings_section)
            .unwrap_or(0) as isize;
        self.enter_settings_section(all[(ix + step).rem_euclid(all.len() as isize) as usize], cx);
        cx.notify();
    }

    /// Remote reads live Tailscale state each time it is opened.
    pub(super) fn enter_settings_section(
        &mut self,
        section: SettingsSection,
        cx: &mut Context<Self>,
    ) {
        if section == SettingsSection::Remote && self.settings_section != section {
            self.refresh_remote(cx);
        }
        self.settings_section = section;
    }
}

#[cfg(test)]
mod tests {
    use super::settings_match;

    #[test]
    fn every_term_must_match_some_field() {
        let fields = [
            "Code font",
            "Used for code, commands and paths.",
            "monospace",
        ];
        assert!(settings_match("", &fields));
        assert!(settings_match("FONT", &fields));
        assert!(settings_match("mono font", &fields));
        assert!(!settings_match("mono theme", &fields));
        assert!(settings_match("  paths  ", &fields));
    }
}
