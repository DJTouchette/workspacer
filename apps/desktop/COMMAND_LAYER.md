# The Command Layer — a tmux/neovim keyboard mode for Workspacer

**Status: ALL FIVE PHASES SHIPPED (e25b597f boundary, 5c1be001 engine,
b6d66a51 verbs, cc679432 toggle, c293be14 chrome + tmux preset, 16c100cf
cmdline + jumplist + conflict warnings + announcement + landing docs; design
harness at /command-layer-harness.html). Still open, each with its seam
prepared: hub-bus `command.run` (the command:action door exists; needs the
4-registry bus checklist) and TUI pin-store unification (TUI migrates off
tui-pins.json to read config.ui.pinnedAgentCwds). Companion to
DESIGN_LANGUAGE.md.**

This is the synthesis of a three-way design competition (tmux prefix layer vs. full
neovim modal app vs. Hyprland-style mod layer), judged on one killer criterion —
*the reliability of the boundary between "keys drive the app" and "keys go to the
focused terminal/composer"* — then hardened against a 22-point adversarial gap
review. Decisions below are settled unless marked OPEN.

## The verdict, and why

**Winner: the tmux-style transient command layer ("Prefix Deck"), hardened with
four grafts from the losing designs.**

- A **persistent Normal/Insert modal app** was rejected. It is the deepest neovim
  feel, but its failure mode is silent and catastrophic: any ungated autofocus
  path (App's 15-frame focus retry, ClaudePane's isActive refocus, the palette's
  Wayland focus net) re-focuses the composer while the chip still says NORMAL,
  and `jjjj` types into a live agent's chat. Maintaining that boundary means
  suppression treaties with three autofocus loops and a five-listener Escape
  ordering contract, forever. **Doctrine: no persistent app modes, no DOM focus
  parking. Write refusals like this down (this file) the way uiMode.ts does.**
- An **always-on Alt/Super mod layer** was rejected as the default vocabulary.
  It converts mode errors into permanent namespace confiscation inside the very
  terminals the doctrine protects (`alt+f/b/d` are readline; macOS Option types
  characters; AltGr reports ctrl+alt; the user's own Hyprland owns Super and
  much of Alt). Its best inventions are grafted instead.
- The prefix layer wins because its worst case is *visible* (an armed strip you
  can see and Esc out of), it reuses the ONE boundary mechanism already proven
  in production (the capture-phase leader in useKeyboardNav.ts), and with
  `enabled: false` the dispatcher is behaviorally today's.

**The four grafts:**
1. *(from Mod Layer)* All new verbs register with `scope: 'layer'` in
   ACTION_REGISTRY + RENDERER_ONLY_SHORTCUTS — the fleet-*/inbox-* pattern. The
   preset drift test filters scoped actions, so the three preset maps,
   config_defaults.json, both generated twins, and the Go writer stay untouched.
2. *(from Mod Layer, redefined per gap review)* **Dwell-HUD**: if armed and no
   key arrives within `hudDelayMs` (400ms default), the compact strip expands
   into the full grouped key grid (Focus / Move / Create / Layout / Act),
   rendered live from the merged chord tree so it cannot drift. (It is a dwell
   timer, NOT a "hold" — on Linux the leader arms on key-UP; there is no held key.)
3. *(from Normal Mode)* **FocusChip** — a persistent bottom-right chip showing
   INSERT (composer, green) / TERM (xterm, red, "keys go to the agent") /
   BROWSE (any webview, incl. plugin panes) / APP, derived purely from
   focusin/focusout. Pure telemetry, zero behavior, zero modes. It answers
   "where do my unprefixed keys go?" — TUI mode-chip parity.
4. *(from Normal Mode)* Extract `CommandPalette.commandActions` into
   `lib/commandRegistry.ts` and add a `prefix :` cmdline palette variant with the
   TUI's ex-verbs (`q vs sp on clo new term pin rename`). One verb table, two
   surfaces; ~15 palette-only commands become keyboard-reachable.

## Interaction model

- Press the leader (`ctrl+space`; Linux: lone-Alt tap via existing resolveLeader)
  from ANY focus — terminal, composer, webview. The layer arms: a bottom glass
  strip appears with the key groups, the workspace gets an accent ring.
- One more keystroke acts, then focus returns to the remembered element
  (`term.focus()` for xterm). No persistent state.
- `timeoutMs: 0` — armed until resolved (TUI no-timeout semantics). Esc, any
  mousedown, window blur, or an unknown key disarms.
- **Repeat groups**: after `prefix h`, the group keys (h/j/k/l, n/p, J/K) re-arm
  for `repeatMs` (500ms) so `prefix h h l` walks panes. The strip stays visible
  for the WHOLE repeat window, and the window cancels on any focus change into
  an editable target (composer autofocus lands < 500ms) — no invisible stealing.
- `prefix prefix` sends a literal leader to the focused terminal (nested tmux),
  addressed to the pane id remembered at arm time, implemented in both
  terminal-hosting panes. Works on Linux too (second Alt tap while armed).

## Keymap (defaults; all steps are one key after the prefix)

Grammar note (gap fix): chord steps are matched on `e.key` **case- and
shift-aware** (`prefix shift+k` is distinct from `prefix k`) — this requires the
Phase 2 grammar extension; buildChordTree gets a duplicate-path assertion.

| Keys | Action | Status |
|---|---|---|
| `h j k l` | focus pane left/down/up/right (repeat) | exists |
| `] / [` | next / prev tab (repeat) — brackets navigate tabs, braces (`{ }`) move panes; `n` belongs to deny | exists |
| `< / >` | move tab | exists |
| `,` | rename tab (tmux-exact) | exists |
| `c` / `t` / `shift+b` | new Claude / terminal / browser | exists |
| `v` / `s` | quick split / split palette | exists |
| `x` | close pane | exists |
| `z` | zoom pane (tmux semantics: any structural/nav action unzooms) | build |
| `{ / }` | swap pane | build |
| `o` | cycle pane | build |
| `q` → digit | pane hints (numbered badges) | build |
| `Enter` | focus composer | build |
| `( / )` | prev / next agent | exists |
| `a` | jump to next agent needing attention | exists |
| `'` | alternate (last-focused) agent | build |
| `ctrl+o / ctrl+i` | session jumplist back / forward | build |
| `1-9` | jump to pinned agent (⚓ badges) | build |
| `m` | pin/unpin agent | build |
| `y` / `n` | approve / deny top attention item — matches Fleet Deck & sidebar-card conventions; the strip SHOWS the item summary; no-ops with a toast if nothing is pending or the top item changed in the last ~1s | build (backend exists) |
| `shift+n` | spawn agent | exists |
| `w` | Fleet Deck (its scoped keys already shipped) | exists |
| `i` | toggle inspector (freed by the Inbox deprecation below) | exists |
| `shift+k / shift+j` | chat scroll half-page (repeat) | build (chatScrollBus) |
| `g g` / `shift+g` | chat scroll top / bottom | build |
| `:` | cmdline palette (ex-verbs) | build |
| `/` | library picker | exists — dead today, fixed by Phase 1 |
| `e` / `-` | sidebar / bottom terminal | exists |
| `f` / `r` / `?` | open file / review / help | exists |

**Approve/deny: SETTLED — `y` / `n` everywhere** (user decision 2026-08-20),
matching Fleet Deck and the sidebar cards. Next/prev tab moved to `] / [` to
free `n`; the strip labels y/n explicitly beside the pending item's summary.

## Deprecation: the Inbox drawer

**The Triage Inbox drawer is deprecated** (user decision 2026-08-20): the
sidebar live-feed cards (spec-2a activity cards with inline Approve/Reply)
supersede it as the attention surface. Consequences for this plan:

- The command layer ships **no inbox binding**; `prefix i` goes to the
  inspector instead. `prefix y/n` + `prefix a` + the sidebar cards + Fleet Deck
  are the attention story.
- The drawer, its `toggle-inbox` action, and the `inbox-*` scoped bindings stay
  functional but get marked deprecated (Settings label + ShortcutOverlay note,
  Phase 4) and are candidates for removal once the sidebar cards reach full
  parity (bulk actions, snooze/queue review — audit during Phase 4).
- Nothing new (bindings, docs, chrome) references the inbox.

## Setting shape

```yaml
keybindings:
  commandLayer:
    enabled: false        # THE switch; Settings → Keybindings toggle
    timeoutMs: 0          # 0 = armed until resolved
    repeatMs: 500
    hudDelayMs: 400       # dwell before strip → full HUD
    passthrough: true     # prefix-prefix → literal to terminal
    indicator: strip      # strip | minimal; never 'none' — no invisible armed state
    leaderOverride: ''    # per-platform escape hatch (Hyprland Alt conflicts, web client)
```

Defaults land in `services/hub/cmd/brain/config_defaults.json` →
`npm run gen:config-defaults` (both generated twins). A fourth keybinding preset
`tmux` re-arranges EXISTING persisted actions into flat `prefix <key>` chords and
its `presetConfigPatch` also sets `commandLayer.enabled: true`. Switching away
from the tmux preset leaves the layer enabled (the strip re-renders from the new
merged map — defined behavior, not an accident); the Go `migrateKeybindings`
mirror is extended in the same commit, pinned by a `contracts/` fixture.

**Merge policy (gap fix):** layer-scoped chords merge into the global chord tree
ONLY while `commandLayer.enabled`, and persisted user/preset chords always beat
layer defaults. The preset shadow-collision test runs over the MERGED map.

## Phases

**Phase 1 — Terminal-boundary hardening (standalone value, ships alone).**
Derive both xterm `attachCustomKeyEventHandler` allowlists from the merged
keybinding config (kill the drifted hardcoded twins in TerminalPane/ClaudePane);
unify every combo matcher on shortcuts.ts/comboMatcher — this fixes a LIVE bug
(library-picker `mod+shift+l` can never match; move it and toggle-inspector into
executeAction/bus dispatch); extract BrowserPane's key-forwarding into a shared
helper and apply it to ALL webview-hosting panes (plugin panes, mdpreview);
forward the leader itself out of guests. Enumerate every key whose PTY behavior
flips (diff old vs derived allowlists) in the changelog; manual xterm matrix
(Ctrl+C, Ctrl+D, readline alt+f/b) before commit.

**Phase 2 — Grammar + engine.** Shift-aware chord steps + duplicate-path
assertion FIRST (the keymap's case pairs are unimplementable without it). Then
the layer engine as ref-backed stages inside the existing capture handler (never
a sibling listener): arm/disarm, repeat groups, remembered-focus restore,
prefix-prefix passthrough, dwell timer, `prefix 1-9` digit-range chord step
(e.code DigitN; numpad excluded — documented). Disarm + suppress arming while
any modal dialog is mounted (`data-leader-capture` on dialog roots; spawn-dialog
regression test). A MINIMAL strip (bare PREFIX chip reusing ChordHint) ships in
this phase — an enabled layer must never be invisible.

**Phase 3 — New primitives (`scope: 'layer'`, renderer-only).** zoom-pane (write
the event × state table first; tmux defaults; zoomedPaneId persists in layout
save, cleared on dead pane), swap/cycle pane, focus-composer extraction,
chatScrollBus + ClaudePane handler (sticky-bottom must survive), pins
(`ui.pinnedAgentCwds` — added to WHOLESALE_PATHS in BOTH writers with a
round-trip test; **shared store with the TUI: the TUI migrates from
tui-pins.json to read this key** — decide before this ships), alternate-agent +
jumplist, approve/deny via AttentionContext, `command.*` hub-bus cases for web
parity. Each primitive lands with a web-parity checklist line (/m is explicitly
out of scope).

**Phase 4 — tmux preset + keyboard chrome.** CommandStrip (bottom glass strip +
inverted PREFIX chip + accent ring, rendered from buildChordTree — drift-proof),
dwell-HUD, FocusChip, PaneHints, ChordHint gated to `indicator !== 'strip'`,
Settings: preset row + layer toggle + **multi-step ShortcutEditor capture** (the
editor must be able to express `prefix g g` and shifted steps) + conflict
validation at save (duplicates, chord-prefix shadowing, merged-map aware).
Accessibility: `role="status" aria-live="polite"` on strip/chip; all timings are
config for motor-impairment reasons (say so in Settings); StickyKeys note —
combo leader recommended, tap-arming disableable. Discoverability for EXISTING
users: a palette command ("Enable command layer") + a one-time notification-
center post linking to Settings.

**Phase 5 — Cmdline + docs.** commandRegistry extraction + `prefix :` ex-verbs;
landing/*.html keyboard docs (Linux Alt-tap asymmetry, Hyprland Alt interplay,
nested-tmux passthrough, numpad + AZERTY/QWERTZ notes — punctuation steps match
on `e.key`, letter aliases documented for EU layouts); rivet context doc +
rivet.learn entries (armed-window flag, scope:'layer' preset-test exemption,
0x00 passthrough).

## Non-goals / rejections (doctrine)

- No persistent Normal/Insert modes; no DOM focus parking; no Esc smart-unwind
  (Esc in a terminal belongs to the PTY, full stop).
- No default held-modifier combos (readline/Option/AltGr/Hyprland costs); power
  users can rebind toward that themselves.
- Mouse-first experience unchanged when `enabled: false`; mousedown always
  disarms — the mouse never fights the layer.
- /m mobile client: out of scope.
