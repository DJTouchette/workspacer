# Native client handoff — chat polish pass (2026-10-02)

The native GPUI client (`apps/native`) is meant to feel like the Electron app,
only simpler. This pass brought chat rendering and chrome up to that bar. All
work is on `main` (6eb3707b → 826da7ba); per-change evidence is in
`VALIDATION.md`, user-facing behavior in `README.md`.

## What changed

| Area | Commit | Where it lives |
|---|---|---|
| Desktop-parity Markdown: bright bold/italic over dimmer `prose` body, accent mono inline code, accent bullets, underlined h1/h2, bordered code blocks with a language header | 6eb3707b | `src/ui/markdown.rs` (`ProseColors`), vendored `src/text/{style,node,inline}.rs` |
| Syntax colors follow the theme: GitHub Dark/Light Default (desktop's shiki themes), Nord palette under Nord | 6eb3707b | `src/ui/syntax.rs`, set in `configure_theme` |
| Real bold: GPUI renders variable fonts at the default weight only, so static Inter Medium/SemiBold/Bold + JetBrains Mono Bold are embedded | 6eb3707b | `assets/fonts/`, `scripts/prepare-fonts.sh` (needs `FONT_PYTHON` with fonttools) |
| Work cards: every run of adjacent regular tool calls is one card (summary header + one-line steps, click a step for details) modeled on desktop `WorkCard.tsx` | 319429ad | `src/ui/work.rs`, `tool_preview::summarize_work`/`group_span` |
| Quiet right-aligned timestamps (disabled tone) | 319429ad | `timestamp_footer` in `src/ui/transcript.rs` |
| Chat always lands on the latest message (reading-position memory removed in **both** clients; desktop reverts db0dc488) | 9548c7f0, cfd263f8 | `land_on_latest` in `src/ui/transcript.rs` |
| Floating content-sized title pill over a fade; inset rounded sidebar/rail on a chat-colored shell; tighter message spacing (copy button moved to the timestamp line) | fe37c08b | `render_title_bar` in `src/ui/chrome.rs`, `src/ui/sidebar.rs` |
| Brand model badges (Claude clay / OpenAI marks from desktop `agentLogos.tsx`) with product names ("Opus 5.5"); rounded padded inline code; one animated loader (composer line) | 826da7ba | `chrome::model_badge`, `model_display_name` + `Session::display_model`/`runtime_model` in `src/model.rs`, `assets/icons/brand/`, `ui::Assets` |

## Things that are not obvious

- **Vendored gpui-component carries real patches.** Everything is opt-in via
  `TextViewStyle.prose` (`None` = upstream look) plus two upstream bug fixes:
  root blocks all got `is_last` (no paragraph gaps), and style updates kept the
  creation-time highlight theme. `vendor/gpui-component/WORKSPACER-PATCHES.md`
  lists them; reapply on upgrade.
- **GPUI text limits drove some designs.** `HighlightStyle` has no font family
  (inline-code mono is set per text run in `inline.rs`) and text-run backgrounds
  are square (inline code is painted as rounded quads; U+2009 thin spaces give
  margin and are stripped from copied selections). `overflow_hidden` clips
  rectangles, so hover fills inside rounded cards round their own corners.
- **Model names.** The session's `model` is the selected alias (`opus`); the
  resolved id comes from `statusLine.modelDisplay`/`usage.model` into
  `runtime_model`, which null/alias-only snapshots never clear. `display_model`
  prefers the runtime id only when it matches the selected family, so a fresh
  `/model` switch shows the new pick until the runtime reports it.
- **Every regular tool call is a work card now** (threshold 1, cap 12, latest 6
  visible). Skill/Subagent/Workflow calls still render with the older
  `tools::card` and keep their extra detail.
- **Settings compatibility.** `Settings.reading` was removed; serde ignores the
  field in old `native-settings.json` files and drops it on the next save.

## Verifying

```sh
cd apps/native
cargo test --locked --features ui-tests -- --test-threads=1   # 186 pass at 826da7ba
cargo clippy --locked --all-targets --features ui-tests -- -D warnings
make -C ../.. build-native   # release binary: apps/native/target/release/wks-native
```

Visual fixture: `native-harness serve --bind 127.0.0.1:17996 --sessions 3 --turns 0 --rich-transcript`,
then `wks-native --bus ws://127.0.0.1:17996/bus` with a throwaway `XDG_CONFIG_HOME`.
On the Hyprland dev box, a headless output for screenshots steals the next
workspace number — if a real window is on it, it moves too. Check the headless
workspace is empty before launching or closing anything there. Desktop checks
for the revert: renderer `tsc`, `npx vitest run` (1936 pass), and
`npx playwright test tests/e2e/chatTailPin.test.ts --project=renderer`.

## Follow-up pass (same day, uncommitted at time of writing)

- Tables and blockquotes use desktop styling (vendored `node.rs`, prose mode).
- Skill/subagent/workflow calls use the work-card shell; their detail sits
  inside the card and long dispatch briefs fold.
- **Settings → Chat → One card per turn** merges a turn's calls and the notes
  between them into one card (`Settings.merge_turn_tools`, default off).
- Nord timestamps brightened (`disabled` 0x616e88); a test pins quiet-text
  contrast for every theme.

## Open follow-ups

- Chat paging and account usage shipped (see VALIDATION.md). Paging re-reads a
  widening window rather than prepending a `before` page; fine up to the 2,000
  row client budget, but an O(n²) re-read if that budget ever grows a lot.
- Visual check of the above was not done (see VALIDATION.md); run the rich
  harness and look at tables, quotes, the Skill/Agent cards and the merged
  turn card in all three themes.
- Inline code renders at full size (desktop uses 0.9em); GPUI can't vary size
  within a line.
- No Windows/macOS rendering checks or live-provider runs were done for this
  pass; the bundled fonts and SVG marks are compiled in, so they should carry over.
