//! Configurable keybindings.
//!
//! `input.rs` used to match `KeyCode`s inline. Keys now resolve through a
//! [`Keymap`]: a chord (key + modifiers) maps to a semantic [`Action`] within a
//! [`Context`] (which handler is live). Defaults reproduce the original
//! vim-first bindings; the user's `tui.json` merges overrides on top.
//!
//! Text-entry modes (the composer, the spawn cwd field, the palette query)
//! capture characters literally and are deliberately **not** routed through the
//! keymap — only navigation/normal-mode keys are remappable. Numeric answer
//! keys (`1`–`9`) are positional and also stay literal.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A discrete thing the user can trigger. Context decides which are reachable;
/// the same chord can bind different actions in different contexts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    Back,
    Refresh,
    Help,
    Palette,
    // sidebar navigation
    SelectNext,
    SelectPrev,
    SelectFirst,
    SelectLast,
    JumpAttention,
    OpenAgent,
    OpenAgentTerminal,
    OpenReview,
    OpenNotes,
    RenameAgent,
    Respawn,
    // agent / tabs
    NewAgent,
    NewTerminal,
    CloseTab,
    TabNext,
    TabPrev,
    // chat
    ToggleTranscript,
    Attach,
    InsertMode,
    ScrollDown,
    ScrollUp,
    // control
    Approve,
    Deny,
    ApproveAlways,
    Interrupt,
    Stop,
    // window splits (panes)
    SplitRight,
    SplitDown,
    FocusNextPane,
    FocusPrevPane,
    ClosePane,
    OnlyPane,
    // harpoon (pinned agents) + jump history
    HarpoonToggle,
    AltAgent,
    JumpBack,
    JumpForward,
    // sidebar
    ToggleStopped,
    OpenFilter,
}

impl Action {
    /// Stable config/display name (snake_case).
    pub fn name(self) -> &'static str {
        use Action::*;
        match self {
            Quit => "quit",
            Back => "back",
            Refresh => "refresh",
            Help => "help",
            Palette => "palette",
            SelectNext => "select_next",
            SelectPrev => "select_prev",
            SelectFirst => "select_first",
            SelectLast => "select_last",
            JumpAttention => "jump_attention",
            OpenAgent => "open_agent",
            OpenAgentTerminal => "open_agent_terminal",
            OpenReview => "open_review",
            OpenNotes => "open_notes",
            RenameAgent => "rename_agent",
            Respawn => "respawn",
            NewAgent => "new_agent",
            NewTerminal => "new_terminal",
            CloseTab => "close_tab",
            TabNext => "tab_next",
            TabPrev => "tab_prev",
            ToggleTranscript => "toggle_transcript",
            Attach => "attach",
            InsertMode => "insert_mode",
            ScrollDown => "scroll_down",
            ScrollUp => "scroll_up",
            Approve => "approve",
            Deny => "deny",
            ApproveAlways => "approve_always",
            Interrupt => "interrupt",
            Stop => "stop",
            SplitRight => "split_right",
            SplitDown => "split_down",
            FocusNextPane => "focus_next_pane",
            FocusPrevPane => "focus_prev_pane",
            ClosePane => "close_pane",
            OnlyPane => "only_pane",
            HarpoonToggle => "harpoon_toggle",
            AltAgent => "alt_agent",
            JumpBack => "jump_back",
            JumpForward => "jump_forward",
            ToggleStopped => "toggle_stopped",
            OpenFilter => "filter",
        }
    }

    pub fn from_name(s: &str) -> Option<Action> {
        use Action::*;
        Some(match s {
            "quit" => Quit,
            "back" => Back,
            "refresh" => Refresh,
            "help" => Help,
            "palette" => Palette,
            "select_next" => SelectNext,
            "select_prev" => SelectPrev,
            "select_first" => SelectFirst,
            "select_last" => SelectLast,
            "jump_attention" => JumpAttention,
            "open_agent" => OpenAgent,
            "open_agent_terminal" => OpenAgentTerminal,
            "open_review" => OpenReview,
            "open_notes" => OpenNotes,
            "rename_agent" => RenameAgent,
            "respawn" => Respawn,
            "new_agent" => NewAgent,
            "new_terminal" => NewTerminal,
            "close_tab" => CloseTab,
            "tab_next" => TabNext,
            "tab_prev" => TabPrev,
            "toggle_transcript" => ToggleTranscript,
            "attach" => Attach,
            "insert_mode" => InsertMode,
            "scroll_down" => ScrollDown,
            "scroll_up" => ScrollUp,
            "approve" => Approve,
            "deny" => Deny,
            "approve_always" => ApproveAlways,
            "interrupt" => Interrupt,
            "stop" => Stop,
            "split_right" => SplitRight,
            "split_down" => SplitDown,
            "focus_next_pane" => FocusNextPane,
            "focus_prev_pane" => FocusPrevPane,
            "close_pane" => ClosePane,
            "only_pane" => OnlyPane,
            "harpoon_toggle" => HarpoonToggle,
            "alt_agent" => AltAgent,
            "jump_back" => JumpBack,
            "jump_forward" => JumpForward,
            "toggle_stopped" => ToggleStopped,
            "filter" => OpenFilter,
            _ => return None,
        })
    }
}

/// Which handler is live; selects the binding table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Context {
    /// Always checked first, regardless of view.
    Global,
    /// Sidebar / dashboard.
    List,
    /// An agent's raw-terminal pane (Claude terminal mode or a shell tab).
    AgentTerminal,
    /// An agent's parsed-transcript pane.
    AgentTranscript,
}

impl Context {
    pub fn from_name(s: &str) -> Option<Context> {
        Some(match s {
            "global" => Context::Global,
            "list" => Context::List,
            "agent_terminal" => Context::AgentTerminal,
            "agent_transcript" => Context::AgentTranscript,
            _ => return None,
        })
    }
}

/// A key + its modifiers, normalized so it compares/hashes stably.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Chord {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

impl Chord {
    fn new(code: KeyCode, mods: KeyModifiers) -> Chord {
        Chord { code, mods }.normalized()
    }

    /// Strip noise so equivalent presses match: keep only CTRL/ALT/SHIFT, and
    /// drop SHIFT on character keys (the uppercase char already encodes it, so
    /// `G` and `shift+g` are the same chord — matching the old inline matches).
    fn normalized(mut self) -> Chord {
        self.mods &= KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT;
        if matches!(self.code, KeyCode::Char(_)) {
            self.mods.remove(KeyModifiers::SHIFT);
        }
        self
    }

    pub fn from_event(k: &KeyEvent) -> Chord {
        Chord::new(k.code, k.modifiers)
    }

    /// Parse a config chord like `ctrl+k`, `shift+tab`, `enter`, `G`, `space`.
    /// Returns `None` on an unrecognised key token.
    pub fn parse(s: &str) -> Option<Chord> {
        let s = s.trim();
        let mut mods = KeyModifiers::NONE;
        let mut parts: Vec<&str> = s.split('+').collect();
        // The final segment is the key; everything before it is a modifier.
        let key = parts.pop()?;
        for m in parts {
            match m.trim().to_lowercase().as_str() {
                "ctrl" | "control" | "c" => mods |= KeyModifiers::CONTROL,
                "alt" | "meta" | "a" => mods |= KeyModifiers::ALT,
                "shift" | "s" => mods |= KeyModifiers::SHIFT,
                _ => return None,
            }
        }
        let mut code = parse_key(key)?;
        // shift+tab is its own keycode; fold it in and clear the shift bit.
        if code == KeyCode::Tab && mods.contains(KeyModifiers::SHIFT) {
            code = KeyCode::BackTab;
            mods.remove(KeyModifiers::SHIFT);
        }
        Some(Chord::new(code, mods))
    }

    /// Human-readable form for the help overlay, e.g. `ctrl+k`, `shift+tab`,
    /// `g`. Reparses to the same chord via [`Chord::parse`].
    pub fn display(&self) -> String {
        // BackTab carries no SHIFT bit (it's its own keycode) but reads as
        // shift+tab to a human and must parse back to BackTab.
        let (key, implied_shift) = match self.code {
            KeyCode::BackTab => ("tab".to_string(), true),
            other => (key_name(other), false),
        };
        let mut out = String::new();
        if self.mods.contains(KeyModifiers::CONTROL) {
            out.push_str("ctrl+");
        }
        if self.mods.contains(KeyModifiers::ALT) {
            out.push_str("alt+");
        }
        if implied_shift || self.mods.contains(KeyModifiers::SHIFT) {
            out.push_str("shift+");
        }
        out.push_str(&key);
        out
    }
}

fn parse_key(s: &str) -> Option<KeyCode> {
    let lower = s.to_lowercase();
    Some(match lower.as_str() {
        "enter" | "return" | "cr" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "backtab" => KeyCode::BackTab,
        "space" => KeyCode::Char(' '),
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "backspace" | "bs" => KeyCode::Backspace,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" | "pgup" => KeyCode::PageUp,
        "pagedown" | "pgdn" => KeyCode::PageDown,
        "delete" | "del" => KeyCode::Delete,
        "insert" | "ins" => KeyCode::Insert,
        // A single character key — preserve case (so `G` ≠ `g`).
        _ => {
            let mut chars = s.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None; // multi-char and not a known name
            }
            KeyCode::Char(c)
        }
    })
}

fn key_name(code: KeyCode) -> String {
    match code {
        KeyCode::Enter => "enter".into(),
        KeyCode::Esc => "esc".into(),
        KeyCode::Tab => "tab".into(),
        KeyCode::BackTab => "tab".into(), // shown with the shift+ prefix
        KeyCode::Char(' ') => "space".into(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Up => "up".into(),
        KeyCode::Down => "down".into(),
        KeyCode::Left => "left".into(),
        KeyCode::Right => "right".into(),
        KeyCode::Backspace => "backspace".into(),
        other => format!("{other:?}").to_lowercase(),
    }
}

/// The outcome of feeding a key sequence to the keymap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyMatch {
    /// The sequence is bound — fire this action.
    Action(Action),
    /// The sequence is a prefix of one or more longer bindings: collect more
    /// keys. This is what pops the which-key menu.
    Pending,
    /// Not bound and not a prefix — a dead end.
    None,
}

/// One reachable continuation from a pending prefix, for the which-key popup.
#[derive(Debug, Clone, Copy)]
pub struct Continuation {
    /// The next chord to press.
    pub chord: Chord,
    /// `Some` when that chord completes a binding (a leaf); `None` when it only
    /// leads deeper (a group — reserved for nested menus).
    pub action: Option<Action>,
}

/// Render a chord sequence the way the user types it, e.g. `space f`.
pub fn display_seq(seq: &[Chord]) -> String {
    seq.iter().map(Chord::display).collect::<Vec<_>>().join(" ")
}

/// The full set of bindings: one (chord-sequence → action) table per context.
#[derive(Debug, Clone)]
pub struct Keymap {
    tables: HashMap<Context, HashMap<Vec<Chord>, Action>>,
    leader: Chord,
}

impl Default for Keymap {
    fn default() -> Self {
        Keymap::with_defaults()
    }
}

impl Keymap {
    /// The leader chord (`space` by default) — the prefix the which-key menu
    /// hangs off.
    pub fn leader(&self) -> Chord {
        self.leader
    }

    /// Resolve a single chord in a context to its action — sugar for the
    /// single-key path (and the tests). Only fires a length-1 binding that
    /// isn't also a prefix of something longer.
    #[allow(dead_code)]
    pub fn action(&self, ctx: Context, chord: Chord) -> Option<Action> {
        match self.resolve(&[ctx], &[chord.normalized()]) {
            KeyMatch::Action(a) => Some(a),
            _ => None,
        }
    }

    /// Resolve a (possibly multi-chord) sequence against an ordered list of
    /// contexts. Earlier contexts win an exact match; a longer binding reachable
    /// in *any* context makes the sequence [`KeyMatch::Pending`] — we never fire
    /// a short binding while a longer one is still reachable, which is how we
    /// avoid needing vim's `timeoutlen` timer.
    pub fn resolve(&self, ctxs: &[Context], seq: &[Chord]) -> KeyMatch {
        let mut exact: Option<Action> = None;
        let mut is_prefix = false;
        for &ctx in ctxs {
            let Some(table) = self.tables.get(&ctx) else { continue };
            for (binding, action) in table {
                if binding.as_slice() == seq {
                    exact.get_or_insert(*action);
                } else if binding.len() > seq.len() && binding.starts_with(seq) {
                    is_prefix = true;
                }
            }
        }
        if is_prefix {
            KeyMatch::Pending
        } else if let Some(a) = exact {
            KeyMatch::Action(a)
        } else {
            KeyMatch::None
        }
    }

    /// The chords that can follow `prefix`, for the which-key popup. Deduped by
    /// next chord and sorted by display.
    pub fn continuations(&self, ctxs: &[Context], prefix: &[Chord]) -> Vec<Continuation> {
        let mut by_chord: HashMap<Chord, Option<Action>> = HashMap::new();
        for &ctx in ctxs {
            let Some(table) = self.tables.get(&ctx) else { continue };
            for (binding, action) in table {
                if binding.len() > prefix.len() && binding.starts_with(prefix) {
                    let next = binding[prefix.len()];
                    let leaf = if binding.len() == prefix.len() + 1 { Some(*action) } else { None };
                    // A concrete leaf shouldn't be shadowed by a deeper group on
                    // the same chord; first writer wins and we only fill leaves.
                    by_chord.entry(next).or_insert(leaf);
                }
            }
        }
        let mut out: Vec<Continuation> = by_chord
            .into_iter()
            .map(|(chord, action)| Continuation { chord, action })
            .collect();
        out.sort_by(|a, b| a.chord.display().cmp(&b.chord.display()));
        out
    }

    /// All bindings for a context, for the help overlay (sorted by display).
    pub fn bindings(&self, ctx: Context) -> Vec<(String, Action)> {
        let mut v: Vec<(String, Action)> = self
            .tables
            .get(&ctx)
            .map(|m| m.iter().map(|(seq, a)| (display_seq(seq), *a)).collect())
            .unwrap_or_default();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    /// Apply a single override. The chord string may be a multi-key sequence:
    /// whitespace-separated chords, with `<leader>` (or `leader`) standing in
    /// for the leader chord — e.g. `"<leader> f"` or `"g g"`. `action_name ==
    /// "none"`/`"unbind"`/`""` removes the binding. Returns `false` if the
    /// sequence or action couldn't be parsed — an unparseable entry is skipped,
    /// never fatal.
    pub fn set(&mut self, ctx: Context, seq_str: &str, action_name: &str) -> bool {
        let Some(seq) = self.parse_seq(seq_str) else { return false };
        let table = self.tables.entry(ctx).or_default();
        if matches!(action_name, "none" | "unbind" | "") {
            table.remove(&seq);
            return true;
        }
        match Action::from_name(action_name) {
            Some(a) => {
                table.insert(seq, a);
                true
            }
            None => false,
        }
    }

    /// Parse a binding string into a chord sequence. Tokens are whitespace-
    /// separated; `<leader>`/`leader` expands to the configured leader chord.
    fn parse_seq(&self, s: &str) -> Option<Vec<Chord>> {
        let mut out = Vec::new();
        for tok in s.split_whitespace() {
            let chord = if tok.eq_ignore_ascii_case("<leader>") || tok.eq_ignore_ascii_case("leader") {
                self.leader
            } else {
                Chord::parse(tok)?
            };
            out.push(chord);
        }
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }

    fn with_defaults() -> Keymap {
        Keymap::with_leader(Chord::parse("space").expect("space is a valid chord"))
    }

    /// Build the default keymap with a specific leader chord.
    pub fn with_leader(leader: Chord) -> Keymap {
        use Action::*;
        let mut tables: HashMap<Context, HashMap<Vec<Chord>, Action>> = HashMap::new();

        // Helper: build a table from (chord-string, action) pairs as length-1
        // sequences. Default chord strings are all known-good, so a parse miss
        // is a programmer bug.
        let build = |pairs: &[(&str, Action)]| -> HashMap<Vec<Chord>, Action> {
            pairs
                .iter()
                .map(|(s, a)| (vec![Chord::parse(s).expect("valid default chord")], *a))
                .collect()
        };

        tables.insert(
            Context::Global,
            build(&[
                ("ctrl+k", Palette),
                ("ctrl+c", Quit),
                ("?", Help),
                // Jump history + alternate agent, on their vim chords.
                ("ctrl+o", JumpBack),
                ("ctrl+6", AltAgent),
            ]),
        );

        tables.insert(
            Context::List,
            build(&[
                ("q", Quit),
                ("j", SelectNext),
                ("down", SelectNext),
                ("k", SelectPrev),
                ("up", SelectPrev),
                ("g", SelectFirst),
                ("G", SelectLast),
                ("m", JumpAttention),
                ("/", OpenFilter),
                ("c", NewAgent),
                ("r", Refresh),
                ("enter", OpenAgent),
                ("l", OpenAgent),
                ("T", OpenAgentTerminal),
                ("R", OpenReview),
                ("N", OpenNotes),
                ("e", RenameAgent),
                ("S", Respawn),
                ("y", Approve),
                ("n", Deny),
                ("a", ApproveAlways),
            ]),
        );

        // Shared agent keys, duplicated into both agent contexts so each lookup
        // is a single-table hit (no fallback chain to reason about).
        let agent_shared: &[(&str, Action)] = &[
            ("q", Quit),
            ("esc", Back),
            ("h", Back),
            ("c", NewAgent),
            ("]", TabNext),
            ("tab", TabNext),
            ("[", TabPrev),
            ("shift+tab", TabPrev),
            ("T", NewTerminal),
            ("w", CloseTab),
            ("R", OpenReview),
            ("N", OpenNotes),
            ("e", RenameAgent),
            ("S", Respawn),
        ];

        let mut terminal = build(agent_shared);
        terminal.extend(build(&[
            ("t", ToggleTranscript),
            ("i", Attach),
            ("enter", Attach),
            ("x", Interrupt),
            ("X", Stop),
        ]));
        tables.insert(Context::AgentTerminal, terminal);

        let mut transcript = build(agent_shared);
        transcript.extend(build(&[
            ("t", ToggleTranscript),
            ("i", InsertMode),
            ("j", ScrollDown),
            ("down", ScrollDown),
            ("k", ScrollUp),
            ("up", ScrollUp),
            ("r", Refresh),
            ("y", Approve),
            ("n", Deny),
            ("a", ApproveAlways),
            ("x", Interrupt),
            ("X", Stop),
        ]));
        tables.insert(Context::AgentTranscript, transcript);

        // Window commands — vim's `Ctrl-w` prefix, in both agent contexts. Lets
        // you tile agents side by side and move focus between them. These are
        // multi-key sequences, so they also surface in the which-key popup.
        let window_cmds: &[(&str, Action)] = &[
            ("v", SplitRight),
            ("s", SplitDown),
            ("w", FocusNextPane),
            ("W", FocusPrevPane),
            ("l", FocusNextPane),
            ("h", FocusPrevPane),
            ("q", ClosePane),
            ("c", ClosePane),
            ("o", OnlyPane),
        ];
        let ctrl_w = Chord::parse("ctrl+w").expect("valid ctrl+w chord");
        for ctx in [Context::AgentTerminal, Context::AgentTranscript] {
            let table = tables.entry(ctx).or_default();
            for (s, a) in window_cmds {
                let chord = Chord::parse(s).expect("valid window-command chord");
                table.insert(vec![ctrl_w, chord], *a);
            }
        }

        // Leader menu — the which-key popup. `<leader>` then one key. Lives in
        // Global so it's reachable from every non-text view; the actions no-op
        // where they don't apply. These are the discoverable, mnemonic verbs;
        // the single-key bindings above stay for muscle memory.
        let leader_menu: &[(&str, Action)] = &[
            ("p", Palette),
            ("a", NewAgent),
            ("t", NewTerminal),
            ("n", OpenNotes),
            ("r", RenameAgent),
            ("g", OpenReview),
            ("m", JumpAttention),
            ("S", Respawn),
            // harpoon: pin the agent + jump history (1-9 jump to a pinned slot,
            // handled positionally in input.rs).
            ("h", HarpoonToggle),
            ("o", JumpBack),
            ("i", JumpForward),
            // Toggle showing stopped/history sessions in the sidebar.
            ("x", ToggleStopped),
            ("?", Help),
            ("q", Quit),
        ];
        let global = tables.entry(Context::Global).or_default();
        for (s, a) in leader_menu {
            let chord = Chord::parse(s).expect("valid leader-menu chord");
            global.insert(vec![leader, chord], *a);
        }

        Keymap { tables, leader }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(c: char, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), mods)
    }

    #[test]
    fn defaults_match_legacy_bindings() {
        let km = Keymap::default();
        assert_eq!(km.action(Context::List, Chord::parse("j").unwrap()), Some(Action::SelectNext));
        assert_eq!(km.action(Context::List, Chord::parse("G").unwrap()), Some(Action::SelectLast));
        assert_eq!(km.action(Context::List, Chord::parse("enter").unwrap()), Some(Action::OpenAgent));
        // enter means different things per context.
        assert_eq!(km.action(Context::AgentTerminal, Chord::parse("enter").unwrap()), Some(Action::Attach));
        assert_eq!(km.action(Context::AgentTranscript, Chord::parse("i").unwrap()), Some(Action::InsertMode));
        assert_eq!(km.action(Context::AgentTerminal, Chord::parse("i").unwrap()), Some(Action::Attach));
        assert_eq!(km.action(Context::Global, Chord::parse("ctrl+k").unwrap()), Some(Action::Palette));
    }

    #[test]
    fn shift_char_normalizes_to_uppercase_chord() {
        // Shift+t arrives as Char('T') possibly with SHIFT set; both must hit T.
        let with_shift = Chord::from_event(&ev('T', KeyModifiers::SHIFT));
        let without = Chord::from_event(&ev('T', KeyModifiers::NONE));
        assert_eq!(with_shift, without);
        let km = Keymap::default();
        assert_eq!(km.action(Context::List, with_shift), Some(Action::OpenAgentTerminal));
    }

    #[test]
    fn ctrl_modifier_is_significant() {
        let km = Keymap::default();
        let plain_k = Chord::from_event(&ev('k', KeyModifiers::NONE));
        let ctrl_k = Chord::from_event(&ev('k', KeyModifiers::CONTROL));
        assert_eq!(km.action(Context::List, plain_k), Some(Action::SelectPrev));
        // ctrl+k isn't bound in List, but is Global.
        assert_eq!(km.action(Context::List, ctrl_k), None);
        assert_eq!(km.action(Context::Global, ctrl_k), Some(Action::Palette));
    }

    #[test]
    fn parse_chord_variants() {
        assert_eq!(Chord::parse("ctrl+k").unwrap(), Chord::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert_eq!(Chord::parse("shift+tab").unwrap(), Chord::new(KeyCode::BackTab, KeyModifiers::NONE));
        assert_eq!(Chord::parse("enter").unwrap(), Chord::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(Chord::parse("space").unwrap(), Chord::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert!(Chord::parse("boguskey").is_none());
        assert!(Chord::parse("hyper+x").is_none());
    }

    #[test]
    fn override_and_unbind() {
        let mut km = Keymap::default();
        assert!(km.set(Context::List, "x", "quit"));
        assert_eq!(km.action(Context::List, Chord::parse("x").unwrap()), Some(Action::Quit));
        // Unbind the default q.
        assert!(km.set(Context::List, "q", "none"));
        assert_eq!(km.action(Context::List, Chord::parse("q").unwrap()), None);
        // Bad action / chord are reported, not fatal.
        assert!(!km.set(Context::List, "z", "frobnicate"));
        assert!(!km.set(Context::List, "nonsense-key", "quit"));
    }

    #[test]
    fn display_roundtrips_through_parse() {
        for s in ["ctrl+k", "g", "G", "enter", "esc", "space", "x"] {
            let c = Chord::parse(s).unwrap();
            // display() may canonicalize (e.g. lowercase names) but must reparse
            // to the same chord.
            assert_eq!(Chord::parse(&c.display()).unwrap(), c, "roundtrip {s}");
        }
        // shift+tab canonicalizes to a BackTab display that reparses equal.
        let bt = Chord::parse("shift+tab").unwrap();
        assert_eq!(Chord::parse(&bt.display()).unwrap(), bt);
    }

    #[test]
    fn leader_sequence_resolves() {
        let km = Keymap::default();
        let leader = km.leader();
        let ctxs = [Context::Global, Context::List];
        // The leader alone is a pending prefix (pops which-key), not an action.
        assert_eq!(km.resolve(&ctxs, &[leader]), KeyMatch::Pending);
        // <leader> p fires the palette.
        let p = Chord::parse("p").unwrap();
        assert_eq!(km.resolve(&ctxs, &[leader, p]), KeyMatch::Action(Action::Palette));
        // An unbound continuation is a dead end.
        let z = Chord::parse("z").unwrap();
        assert_eq!(km.resolve(&ctxs, &[leader, z]), KeyMatch::None);
        // A plain bound key still fires immediately — it isn't a prefix.
        let j = Chord::parse("j").unwrap();
        assert_eq!(km.resolve(&[Context::List], &[j]), KeyMatch::Action(Action::SelectNext));
    }

    #[test]
    fn leader_menu_lists_continuations() {
        let km = Keymap::default();
        let conts = km.continuations(&[Context::Global, Context::List], &[km.leader()]);
        // Every entry is a leaf (flat menu for now) and the palette is reachable.
        assert!(conts.iter().all(|c| c.action.is_some()));
        assert!(conts.iter().any(|c| c.action == Some(Action::Palette)));
    }

    #[test]
    fn multi_key_override_parses_and_unbinds() {
        let mut km = Keymap::default();
        // A user-defined `g g`. Because `g` alone is the List default, a longer
        // binding becomes reachable, so a lone `g` now waits (the documented
        // no-timeout tradeoff) and the full sequence fires.
        assert!(km.set(Context::List, "g g", "select_first"));
        let g = Chord::parse("g").unwrap();
        assert_eq!(km.resolve(&[Context::List], &[g]), KeyMatch::Pending);
        assert_eq!(km.resolve(&[Context::List], &[g, g]), KeyMatch::Action(Action::SelectFirst));
        // `<leader>` expands in override strings, and unbinding the sequence
        // restores the lone-key behavior.
        assert!(km.set(Context::List, "g g", "none"));
        assert_eq!(km.resolve(&[Context::List], &[g]), KeyMatch::Action(Action::SelectFirst));
    }
}
