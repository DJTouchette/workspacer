//! Design v2's command palette (Ctrl+K): sessions, actions and settings
//! pages in one fuzzy list. `>` narrows to actions, `@` to sessions; with no
//! query, recent sessions come first. Up/Down move, Enter opens, Esc closes.
//! v1 keeps Ctrl+K as the sidebar's session search.
use super::primitives::{self, StatusKind, kbd, status_indicator};
use super::settings::SettingsSection;
use super::*;
use gpui::{AnyElement, HighlightStyle, StyledText, deferred};
use std::ops::Range;

gpui::actions!(native, [PaletteUp, PaletteDown, PaletteEnter, PaletteClose]);

/// One action entry: what it does, title, key hint, icon, search aliases,
/// and whether it is offered right now.
type ActionRow = (
    Act,
    &'static str,
    Option<&'static str>,
    IconName,
    &'static str,
    bool,
);

/// Window-local palette state.
#[derive(Default)]
pub(super) struct PaletteUi {
    pub(super) open: bool,
    input: Option<Entity<InputState>>,
    _watch: Option<gpui::Subscription>,
    cursor: usize,
    /// Sessions opened from the palette (or open when it opened), most
    /// recent first: they lead an empty query.
    recents: Vec<String>,
    scroll: gpui::ScrollHandle,
}

/// What a query is limited to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Scope {
    Sessions,
    Actions,
    Settings,
}

impl Scope {
    fn label(self) -> &'static str {
        match self {
            Self::Sessions => "Sessions",
            Self::Actions => "Actions",
            Self::Settings => "Settings",
        }
    }
}

/// `>` narrows to actions (and settings pages), `@` to sessions.
pub(super) fn parse_scope(input: &str) -> (Option<Scope>, &str) {
    let trimmed = input.trim_start();
    if let Some(rest) = trimmed.strip_prefix('>') {
        (Some(Scope::Actions), rest.trim())
    } else if let Some(rest) = trimmed.strip_prefix('@') {
        (Some(Scope::Sessions), rest.trim())
    } else {
        (None, trimmed.trim())
    }
}

/// A fuzzy match of `query` in `text`: every query character in order,
/// ignoring case and spaces. Scores favour a whole substring, a prefix,
/// word starts and runs; returns the matched byte ranges for highlights.
pub(super) fn fuzzy(query: &str, text: &str) -> Option<(i32, Vec<Range<usize>>)> {
    let needle: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    if needle.is_empty() {
        return Some((0, Vec::new()));
    }
    let lower_text = text.to_lowercase();
    let lower_query = query.trim().to_lowercase();
    // A plain substring wins outright (and highlights as one run).
    if lower_text.len() == text.len()
        && let Some(at) = lower_text.find(&lower_query)
    {
        let word_start = at == 0
            || !text[..at]
                .chars()
                .next_back()
                .is_some_and(char::is_alphanumeric);
        let score =
            1000 - at as i32 + if word_start { 200 } else { 0 } + if at == 0 { 300 } else { 0 };
        return Some((score, std::iter::once(at..at + lower_query.len()).collect()));
    }
    let mut ranges: Vec<Range<usize>> = Vec::new();
    let mut score = 0;
    let mut next = 0;
    let mut prev_matched = false;
    let mut prev_char: Option<char> = None;
    for (at, c) in text.char_indices() {
        if next == needle.len() {
            break;
        }
        let matched = c.to_lowercase().eq(std::iter::once(needle[next]));
        if matched {
            let start = prev_char.is_none_or(|p| !p.is_alphanumeric());
            score += 10 + if prev_matched { 15 } else { 0 } + if start { 20 } else { 0 };
            match ranges.last_mut() {
                Some(last) if prev_matched && last.end == at => last.end = at + c.len_utf8(),
                _ => ranges.push(at..at + c.len_utf8()),
            }
            next += 1;
        } else {
            score -= 1;
        }
        prev_matched = matched;
        prev_char = Some(c);
    }
    (next == needle.len()).then_some((score, ranges))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    NewSession,
    NextNeedsYou,
    Changes,
    Files,
    Terminal,
    Details,
    Model,
    Tasks,
    History,
    Projects,
    Jobs,
    Machines,
    Scratch,
    Setup,
    Theme(Appearance),
    NextTheme,
    Sidebar,
    Shortcuts,
    Refresh,
}

#[derive(Clone)]
enum Target {
    Session(usize),
    Action(Act),
    Setting(SettingsSection),
}

struct Entry {
    scope: Scope,
    title: String,
    meta: String,
    keys: Option<&'static str>,
    icon: Option<IconName>,
    status: Option<StatusKind>,
    target: Target,
    /// Words that also find it ("diff" finds Changes).
    aliases: &'static str,
    /// Order with no query (recents first).
    rank: usize,
}

struct Hit {
    entry: Entry,
    score: i32,
    ranges: Vec<Range<usize>>,
}

impl Workspace {
    /// Ctrl+K under v2: open the palette (again: close it).
    pub(super) fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.shell.palette.open {
            self.close_palette(window, cx);
            return;
        }
        let input = match self.shell.palette.input.clone() {
            Some(input) => input,
            None => {
                let input = cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder("Search sessions, actions and settings…")
                });
                self.shell.palette._watch = Some(cx.subscribe(
                    &input,
                    |this, _, event: &gpui_component::input::InputEvent, cx| {
                        if matches!(event, gpui_component::input::InputEvent::Change) {
                            this.shell.palette.cursor = 0;
                            this.shell.palette.scroll.scroll_to_item(0);
                            cx.notify();
                        }
                    },
                ));
                self.shell.palette.input = Some(input.clone());
                input
            }
        };
        if let Some(id) = self.view.selected.clone() {
            self.remember_recent(id);
        }
        self.shell.model_popover = false;
        self.shell.shortcuts = false;
        self.shell.palette.open = true;
        self.shell.palette.cursor = 0;
        input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.shell.palette.open {
            return;
        }
        self.shell.palette.open = false;
        window.focus(&self.focus);
        cx.notify();
    }

    fn remember_recent(&mut self, id: String) {
        let recents = &mut self.shell.palette.recents;
        recents.retain(|r| *r != id);
        recents.insert(0, id);
        recents.truncate(20);
    }

    fn palette_query(&self, cx: &App) -> String {
        self.shell
            .palette
            .input
            .as_ref()
            .map(|i| i.read(cx).value().to_string())
            .unwrap_or_default()
    }

    /// Everything the palette can reach, before matching.
    fn palette_entries(&self, cx: &App) -> Vec<Entry> {
        let mac = cfg!(target_os = "macos");
        let m = |key: &'static str, mac_key: &'static str| if mac { mac_key } else { key };
        let vim = self.vim();
        let now = timing::now_ms();
        let mut out = Vec::new();
        // Sessions: recents, then the list's own order (needs you first).
        let order = self.nav_sessions(cx);
        let recents = &self.shell.palette.recents;
        let mut sessions: Vec<usize> = self
            .visible_sessions(cx)
            .into_iter()
            .filter(|ix| !order.contains(ix))
            .collect();
        sessions.splice(0..0, order.iter().copied());
        for (n, ix) in sessions.into_iter().enumerate() {
            let s = &self.view.sessions[ix];
            let recent = recents.iter().position(|r| *r == s.id);
            let reason = self
                .inbox_reason(s, now)
                .filter(|_| attention::needs_you(s) || self.stalled_for(s, now).is_some());
            let age = s
                .telemetry
                .last_activity_ms
                .map(|at| wks_native::format::age(at, now));
            let meta = [Some(self.project_name(&s.cwd)), reason.map(|r| r.text), age]
                .into_iter()
                .flatten()
                .filter(|m| !m.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            let status = if attention::needs_you(s) {
                StatusKind::NeedsYou
            } else if self.stalled_for(s, now).is_some() {
                StatusKind::Failed
            } else if s.working() {
                StatusKind::Working
            } else if self.paused(s) {
                StatusKind::Paused
            } else {
                StatusKind::Idle
            };
            out.push(Entry {
                scope: Scope::Sessions,
                title: self.session_title(s),
                meta,
                keys: None,
                icon: None,
                status: Some(status),
                target: Target::Session(ix),
                aliases: "",
                rank: recent.map_or(1000 + n, |r| r),
            });
        }
        let selected = self.view.selected.is_some();
        let theme_light = matches!(
            self.appearance,
            Appearance::Light | Appearance::CatppuccinLatte
        );
        let mut actions: Vec<ActionRow> = vec![
            (
                Act::NewSession,
                "New session",
                Some(m("Ctrl N", "⌘ N")),
                IconName::Plus,
                "start create agent chat",
                !self.demo,
            ),
            (
                Act::NextNeedsYou,
                "Go to the next session that needs you",
                Some("Alt N"),
                IconName::ArrowRight,
                "inbox approval question waiting jump",
                true,
            ),
            (
                Act::Changes,
                "Open changes",
                vim.then_some("g d"),
                IconName::Replace,
                "diff review git",
                selected,
            ),
            (
                Act::Files,
                "Open files and editor",
                Some(m("Ctrl Shift E", "⌘ Shift E")),
                IconName::FolderOpen,
                "explorer tree",
                selected,
            ),
            (
                Act::Terminal,
                if self.terminal.open {
                    "Hide terminal"
                } else {
                    "Open terminal"
                },
                Some(m("Ctrl `", "⌘ `")),
                IconName::SquareTerminal,
                "shell console",
                selected,
            ),
            (
                Act::Tasks,
                "Show background tasks",
                None,
                IconName::LoaderCircle,
                "jobs running processes logs",
                selected,
            ),
            (
                Act::Details,
                "Session details",
                vim.then_some("g e"),
                IconName::Info,
                "rename end stop",
                selected,
            ),
            (
                Act::Model,
                "Change model, effort or access",
                vim.then_some("g m"),
                IconName::Settings2,
                "permission mode",
                selected && self.supported_session(),
            ),
            (
                Act::History,
                "Session history",
                vim.then_some("g h"),
                IconName::BookOpen,
                "archived recent past",
                true,
            ),
            (
                Act::Projects,
                "Projects",
                Some(m("Ctrl P", "⌘ P")),
                IconName::Folder,
                "folders",
                true,
            ),
            (
                Act::Jobs,
                "Jobs",
                vim.then_some("g j"),
                IconName::Calendar,
                "scheduled automation cron",
                true,
            ),
            (
                Act::Machines,
                "Machines",
                vim.then_some("g o"),
                IconName::LayoutDashboard,
                "hosts computers queue",
                true,
            ),
            (
                Act::Scratch,
                "Scratch files",
                vim.then_some("g x"),
                IconName::FolderClosed,
                "notes",
                true,
            ),
            (
                Act::Setup,
                "Agent setup",
                None,
                IconName::CircleUser,
                "install sign in login claude codex",
                true,
            ),
            (
                Act::Theme(if theme_light {
                    Appearance::Dark
                } else {
                    Appearance::Light
                }),
                if theme_light {
                    "Switch to the dark theme"
                } else {
                    "Switch to the light theme"
                },
                None,
                IconName::Palette,
                "theme appearance mode colour color",
                true,
            ),
            (
                Act::NextTheme,
                "Next theme",
                None,
                IconName::Palette,
                "theme cycle nord catppuccin",
                true,
            ),
            (
                Act::Sidebar,
                if self.sidebar_collapsed {
                    "Expand the sidebar"
                } else {
                    "Collapse the sidebar"
                },
                None,
                IconName::PanelLeftClose,
                "rail panel",
                true,
            ),
            (
                Act::Shortcuts,
                "Keyboard shortcuts",
                Some(m("Ctrl /", "⌘ /")),
                IconName::Info,
                "keys help cheat sheet",
                true,
            ),
            (
                Act::Refresh,
                "Refresh",
                Some(m("Ctrl R", "⌘ R")),
                IconName::Redo,
                "reload reconnect",
                true,
            ),
        ];
        actions.retain(|a| a.5);
        for (n, (act, title, keys, icon, aliases, _)) in actions.into_iter().enumerate() {
            out.push(Entry {
                scope: Scope::Actions,
                title: title.to_owned(),
                meta: String::new(),
                keys,
                icon: Some(icon),
                status: None,
                target: Target::Action(act),
                aliases,
                rank: n,
            });
        }
        let pages: [(SettingsSection, &str, &'static str); 7] = [
            (
                SettingsSection::Appearance,
                "Appearance",
                "theme density reading width conversation chat",
            ),
            (SettingsSection::Typography, "Typography", "font size text"),
            (
                SettingsSection::Workspace,
                "Workspace",
                "layout notifications",
            ),
            (SettingsSection::Remote, "Remote", "phone tailscale"),
            (
                SettingsSection::Agents,
                "Agents",
                "defaults provider access",
            ),
            (
                SettingsSection::Keyboard,
                "Keyboard",
                "vim modal keys send enter",
            ),
            (SettingsSection::About, "About", "version update"),
        ];
        for (n, (section, title, aliases)) in pages.into_iter().enumerate() {
            out.push(Entry {
                scope: Scope::Settings,
                title: format!("Settings: {title}"),
                meta: String::new(),
                keys: (section == SettingsSection::Appearance).then_some(m("Ctrl ,", "⌘ ,")),
                icon: Some(IconName::Settings),
                status: None,
                target: Target::Setting(section),
                aliases,
                rank: n,
            });
        }
        out
    }

    /// The palette's rows for its current query, grouped by scope.
    fn palette_hits(&self, cx: &App) -> Vec<Hit> {
        let raw = self.palette_query(cx);
        let (scope, query) = parse_scope(&raw);
        let limit = self.tokens().shell().palette_limit;
        let mut hits: Vec<Hit> = self
            .palette_entries(cx)
            .into_iter()
            .filter(|e| match scope {
                Some(Scope::Sessions) => e.scope == Scope::Sessions,
                Some(Scope::Actions) => e.scope != Scope::Sessions,
                _ => true,
            })
            .filter_map(|entry| {
                if query.is_empty() {
                    return Some(Hit {
                        entry,
                        score: 0,
                        ranges: Vec::new(),
                    });
                }
                let (score, ranges) = fuzzy(query, &entry.title)
                    .or_else(|| {
                        fuzzy(query, entry.aliases)
                            .filter(|_| query.len() > 2)
                            .map(|(s, _)| (s / 4, Vec::new()))
                    })
                    .or_else(|| {
                        (entry.scope == Scope::Sessions)
                            .then(|| fuzzy(query, &entry.meta))
                            .flatten()
                            .map(|(s, _)| (s / 4, Vec::new()))
                    })?;
                Some(Hit {
                    entry,
                    score,
                    ranges,
                })
            })
            .collect();
        if query.is_empty() {
            // No query: a few recent sessions, then every action.
            let sessions = if scope == Some(Scope::Sessions) {
                limit
            } else {
                5
            };
            hits.sort_by_key(|h| (h.entry.scope, h.entry.rank));
            let mut seen = 0;
            hits.retain(|h| {
                if h.entry.scope != Scope::Sessions {
                    return true;
                }
                seen += 1;
                seen <= sessions
            });
        } else {
            hits.sort_by(|a, b| {
                a.entry
                    .scope
                    .cmp(&b.entry.scope)
                    .then(b.score.cmp(&a.score))
                    .then(a.entry.rank.cmp(&b.entry.rank))
            });
            // The best scope leads.
            let best = hits.iter().max_by_key(|h| h.score).map(|h| h.entry.scope);
            if let Some(best) = best {
                hits.sort_by_key(|h| h.entry.scope != best);
            }
            let mut per: BTreeMap<Scope, usize> = BTreeMap::new();
            hits.retain(|h| {
                let n = per.entry(h.entry.scope).or_default();
                *n += 1;
                *n <= limit
            });
        }
        hits
    }

    fn palette_move(&mut self, step: isize, cx: &mut Context<Self>) {
        let count = self.palette_hits(cx).len();
        if count == 0 {
            return;
        }
        let cursor = self.shell.palette.cursor as isize + step;
        self.shell.palette.cursor = cursor.rem_euclid(count as isize) as usize;
        // Rows are interleaved with scope headers: reveal the row's slot.
        let slot = self.palette_slot(self.shell.palette.cursor, cx);
        self.shell.palette.scroll.scroll_to_item(slot);
        cx.notify();
    }

    /// The list child that shows hit `cursor` (headers take slots too).
    fn palette_slot(&self, cursor: usize, cx: &App) -> usize {
        let hits = self.palette_hits(cx);
        let mut slot = 0;
        let mut last = None;
        for (n, hit) in hits.iter().enumerate() {
            if last != Some(hit.entry.scope) {
                slot += 1;
                last = Some(hit.entry.scope);
            }
            if n == cursor {
                return slot;
            }
            slot += 1;
        }
        slot
    }

    fn palette_run(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hit) = self.palette_hits(cx).into_iter().nth(index) else {
            return;
        };
        self.close_palette(window, cx);
        match hit.entry.target {
            Target::Session(ix) => {
                if let Some(s) = self.view.sessions.get(ix) {
                    self.remember_recent(s.id.clone());
                    self.open_listed_session(ix, cx);
                }
            }
            Target::Setting(section) => {
                self.show_screen(Screen::Settings, window, cx);
                self.settings_section = section;
            }
            Target::Action(act) => match act {
                Act::NewSession => self.show_new_session(window, cx),
                Act::NextNeedsYou => self.next_needs_you(window, cx),
                Act::Changes => self.open_feature(Screen::Changes, window, cx),
                Act::Files => self.open_selected_editor(window, cx),
                Act::Terminal => self.toggle_terminal(window, cx),
                Act::Tasks => self.toggle_tasks(window, cx),
                Act::Details => self.open_feature(Screen::Session, window, cx),
                Act::Model => {
                    self.screen = Screen::Conversation;
                    self.new_session = false;
                    self.open_model_popover(window, cx)
                }
                Act::History => self.open_feature(Screen::Recent, window, cx),
                Act::Projects => self.show_screen(Screen::Projects, window, cx),
                Act::Jobs => self.open_feature(Screen::Jobs, window, cx),
                Act::Machines => self.open_feature(Screen::Hosts, window, cx),
                Act::Scratch => self.open_feature(Screen::Scratch, window, cx),
                Act::Setup => self.open_feature(Screen::Setup, window, cx),
                Act::Theme(appearance) => self.choose_theme(appearance, window, cx),
                Act::NextTheme => {
                    let at = Appearance::ALL
                        .iter()
                        .position(|a| *a == self.appearance)
                        .unwrap_or(0);
                    self.choose_theme(
                        Appearance::ALL[(at + 1) % Appearance::ALL.len()],
                        window,
                        cx,
                    )
                }
                Act::Sidebar => {
                    self.sidebar_collapsed = !self.sidebar_collapsed;
                }
                Act::Shortcuts => self.shell.shortcuts = true,
                Act::Refresh => self.command(Command::Refresh, cx),
            },
        }
        cx.notify();
    }

    /// The palette over everything, when open.
    pub(super) fn render_palette(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.shell.palette.open || !self.design().is_v2() {
            return None;
        }
        let input = self.shell.palette.input.clone()?;
        let p = self.palette();
        let t = p.tokens;
        let s = t.shell();
        let raw = self.palette_query(cx);
        let (scope, query) = parse_scope(&raw);
        let hits = self.palette_hits(cx);
        let cursor = self.shell.palette.cursor.min(hits.len().saturating_sub(1));
        let viewport = window.viewport_size();
        let width = px(s.palette_width).min(viewport.width - px(2. * t.space.xl));
        let mut list: Vec<AnyElement> = Vec::new();
        let mut last = None;
        for (n, hit) in hits.into_iter().enumerate() {
            if last != Some(hit.entry.scope) {
                last = Some(hit.entry.scope);
                list.push(
                    div()
                        .h(px(t.layout.control_height_sm))
                        .px(px(t.space.md))
                        .pt(px(t.space.sm))
                        .flex()
                        .items_center()
                        .text_size(px(t.text.meta))
                        .font_weight(FontWeight(t.weight.semibold))
                        .text_color(rgb(p.roles.text_tertiary))
                        .child(if query.is_empty() && hit.entry.scope == Scope::Sessions {
                            "Recent sessions"
                        } else {
                            hit.entry.scope.label()
                        })
                        .into_any_element(),
                );
            }
            list.push(self.palette_row(hit, n, n == cursor, cx));
        }
        let empty = list.is_empty();
        let card = div()
            .id("command-palette")
            .debug_selector(|| "command-palette".into())
            .key_context("CommandPalette")
            .occlude()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_action(cx.listener(|this, _: &PaletteUp, _, cx| this.palette_move(-1, cx)))
            .on_action(cx.listener(|this, _: &PaletteDown, _, cx| this.palette_move(1, cx)))
            .on_action(cx.listener(|this, _: &PaletteEnter, window, cx| {
                let cursor = this.shell.palette.cursor;
                this.palette_run(cursor, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &PaletteClose, window, cx| this.close_palette(window, cx)),
            )
            .absolute()
            .top(px(s.palette_top))
            .left((viewport.width - width) / 2.)
            .w(width)
            .flex()
            .flex_col()
            .rounded(px(t.radius.panel))
            .bg(rgb(p.roles.raised))
            .border(px(t.stroke.hairline))
            .border_color(rgb(p.roles.border_strong))
            .shadow(primitives::elevation(&p, t.elevation.modal))
            .overflow_hidden()
            .child(
                div()
                    .h(px(t.layout.control_height + t.space.md))
                    .px(px(t.space.lg))
                    .flex()
                    .items_center()
                    .gap(px(t.space.md))
                    .border_b(px(t.stroke.hairline))
                    .border_color(rgb(p.border))
                    .child(
                        Icon::new(IconName::Search)
                            .size(px(t.icon.md))
                            .text_color(rgb(p.roles.text_tertiary)),
                    )
                    .when_some(scope, |d, scope| {
                        d.child(
                            div()
                                .debug_selector(|| "command-palette-scope".into())
                                .flex_shrink_0()
                                .px(px(t.space.sm))
                                .py(px(t.space.xxs))
                                .rounded(px(t.radius.full))
                                .bg(primitives::tonal(&p, t.wash.hover))
                                .text_size(px(t.text.meta))
                                .font_weight(FontWeight(t.weight.medium))
                                .text_color(rgb(p.roles.text_secondary))
                                .child(scope.label()),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(t.text.ui))
                            .child(Input::new(&input).appearance(false)),
                    ),
            )
            .child(
                div()
                    .id("command-palette-list")
                    .debug_selector(|| "command-palette-list".into())
                    .track_scroll(&self.shell.palette.scroll)
                    .overflow_y_scroll()
                    .max_h(px(s.palette_list).min(viewport.height - px(s.palette_top * 2.)))
                    .px(px(t.space.sm))
                    .pb(px(t.space.sm))
                    .children(list)
                    .when(empty, |d| {
                        d.child(
                            div()
                                .px(px(t.space.md))
                                .py(px(t.space.xl))
                                .text_size(px(t.text.body))
                                .text_color(rgb(p.roles.text_tertiary))
                                .child(format!("Nothing matches “{query}”")),
                        )
                    }),
            )
            .child(
                div()
                    .h(px(t.layout.control_height_sm + t.space.xs))
                    .px(px(t.space.lg))
                    .flex()
                    .items_center()
                    .gap(px(t.space.lg))
                    .border_t(px(t.stroke.hairline))
                    .border_color(rgb(p.border))
                    .text_size(px(t.text.meta))
                    .text_color(rgb(p.roles.text_tertiary))
                    .child(hint(&["↑", "↓"], "move", &p))
                    .child(hint(&["Enter"], "open", &p))
                    .child(hint(&[">"], "actions", &p))
                    .child(hint(&["@"], "sessions", &p))
                    .child(div().flex_1())
                    .child(hint(&["Esc"], "close", &p)),
            );
        Some(
            deferred(
                div()
                    .id("command-palette-layer")
                    .debug_selector(|| "command-palette-layer".into())
                    .absolute()
                    .inset_0()
                    .occlude()
                    .bg(primitives::wash(p.shadow, t.elevation.scrim_alpha / 2.))
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, window, cx| this.close_palette(window, cx)),
                    )
                    .child(card),
            )
            .with_priority(3)
            .into_any_element(),
        )
    }

    fn palette_row(
        &self,
        hit: Hit,
        n: usize,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let t = p.tokens;
        let entry = hit.entry;
        let style = HighlightStyle {
            color: Some(rgb(p.text).into()),
            font_weight: Some(FontWeight(t.weight.semibold)),
            underline: Some(gpui::UnderlineStyle {
                thickness: px(t.stroke.hairline),
                color: Some(rgb(p.roles.text_tertiary).into()),
                wavy: false,
            }),
            ..Default::default()
        };
        let title = StyledText::new(entry.title.clone())
            .with_highlights(hit.ranges.into_iter().map(|r| (r, style)));
        div()
            .id(SharedString::from(format!("command-palette-row-{n}")))
            .debug_selector(move || format!("command-palette-row-{n}"))
            .h(px(t.layout.control_height))
            .px(px(t.space.md))
            .rounded(px(t.radius.control))
            .flex()
            .items_center()
            .gap(px(t.space.md))
            .cursor_pointer()
            .when(selected, |d| d.bg(rgb(p.selected)))
            .when(!selected, |d| d.hover(move |s| s.bg(rgb(p.roles.hover))))
            .child(
                div()
                    .size(px(t.icon.md))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .map(|d| match (entry.status, entry.icon) {
                        (Some(kind), _) => d.child(status_indicator(kind, &p)),
                        (None, Some(icon)) => d.child(
                            Icon::new(icon)
                                .size(px(t.icon.sm))
                                .text_color(rgb(p.roles.text_tertiary)),
                        ),
                        _ => d,
                    }),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_shrink()
                    .truncate()
                    .text_size(px(t.text.body))
                    .font_weight(FontWeight(t.weight.medium))
                    .text_color(rgb(if selected {
                        p.text
                    } else {
                        p.roles.text_secondary
                    }))
                    .child(title),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(t.text.meta))
                    .text_color(rgb(p.roles.text_tertiary))
                    .child(entry.meta),
            )
            .when_some(entry.keys, |d, keys| {
                d.child(
                    div()
                        .flex_shrink_0()
                        .flex()
                        .gap(px(t.space.xxs))
                        .children(keys.split(' ').map(|k| kbd(k, &p))),
                )
            })
            .on_mouse_move(cx.listener(move |this, _: &gpui::MouseMoveEvent, _, cx| {
                if this.shell.palette.cursor != n {
                    this.shell.palette.cursor = n;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| this.palette_run(n, window, cx)))
            .into_any_element()
    }
}

fn hint(keys: &[&'static str], label: &'static str, p: &Palette) -> Div {
    let t = p.tokens;
    div()
        .flex()
        .items_center()
        .gap(px(t.space.xs))
        .children(keys.iter().map(|k| kbd(*k, p)))
        .child(label)
}

use super::attention;
use std::collections::BTreeMap;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_pick_a_scope() {
        assert_eq!(parse_scope("> theme"), (Some(Scope::Actions), "theme"));
        assert_eq!(parse_scope("@bus"), (Some(Scope::Sessions), "bus"));
        assert_eq!(parse_scope(" review "), (None, "review"));
    }

    #[test]
    fn fuzzy_prefers_substrings_and_word_starts() {
        let (whole, ranges) = fuzzy("review", "Session creation review").unwrap();
        assert_eq!(ranges, vec![17..23]);
        let (scattered, ranges) = fuzzy("scr", "Session creation review").unwrap();
        assert!(whole > scattered);
        assert_eq!(ranges.len(), 2, "{ranges:?}");
        assert!(fuzzy("xyz", "Session").is_none());
        let (prefix, _) = fuzzy("open", "Open changes").unwrap();
        let (inner, _) = fuzzy("open", "Reopen").unwrap();
        assert!(prefix > inner);
        assert_eq!(fuzzy("", "anything").unwrap().0, 0);
        // Multi-byte text keeps valid ranges.
        let (_, ranges) = fuzzy("cafe", "Café notes").unwrap_or((0, vec![]));
        for r in ranges {
            assert!("Café notes".is_char_boundary(r.start));
        }
    }
}
