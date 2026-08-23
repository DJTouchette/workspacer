---
title: claudemon watch Embedded Terminal UI
tags: [claudemon, rust, tui, ratatui, sse, monitoring, rendering]
related_paths:
  - "services/claudemon/src/tui/mod.rs"
  - "services/claudemon/src/tui/app.rs"
  - "services/claudemon/src/tui/sse.rs"
  - "services/claudemon/src/tui/view/*.rs"
  - "services/claudemon/src/tui/editor.rs"
  - "services/claudemon/src/tui/preview.rs"
  - "services/claudemon/src/tui/syntax.rs"
owner: Damien Touchette
last_reviewed: 2026-07-11
---

# claudemon watch Embedded Terminal UI

## Overview
This is the small terminal UI shipped **inside the claudemon Rust binary**, invoked via `claudemon watch [--api <url>]` (`services/claudemon/src/cli.rs` `Command::Watch`, default `http://127.0.0.1:7891`, dispatched in `services/claudemon/src/cli.rs`'s `dispatch` function to `crate::tui::run(api)`). Do NOT confuse this with `apps/tui` (the standalone `wks-tui` client crate documented in memory `wks-tui-client.md`) — that is a separate, larger, vim-first Rust TUI project living outside `services/claudemon`. This module talks only to claudemon's own public REST + SSE API (`services/claudemon/src/daemon/api.rs`), so it has zero daemon-internal coupling and works against any compatible/remote daemon, including future re-implementations in other languages (per the doc comment in `services/claudemon/src/tui/mod.rs` (L10-12)).

## Key modules
- `services/claudemon/src/tui/mod.rs` — `run(api_url)` entrypoint: ratatui terminal init (`ratatui::init()` + `TerminalGuard` for panic-safe restore), `App::refresh_initial()` best-effort seed fetch, spawns the SSE task, then a `tokio::select!` loop merging `crossterm::EventStream` keys with `AppEvent`s from an `mpsc::unbounded_channel`, plus a 1s ticker for relative-time refresh and (in chat) transcript polling. Key dispatch splits into `handle_dashboard_key` and `handle_chat_key`.
- `services/claudemon/src/tui/app.rs` — `App` state (sessions map, display `order`, `selected` index, `connected` flag, per-session `gates`, current `View`), `AppEvent` enum (`Update(Box<SessionUpdate>)`, `SseConnected`, `SseDisconnected`, `Toast`), and all daemon-calling actions: `refresh_initial` (GET `/sessions`), `fetch_transcript_for_chat` (GET `/sessions/:id/transcript`), `act_send_message` (POST `/sessions/:id/message` when mode=Input else `/sessions/:id/input`), `act_approve` (POST `/sessions/:id/approve`), `act_answer` (POST `/sessions/:id/answer`), `act_toggle_gate` (POST `/sessions/:id/gate`). `apply_event` is the single mutation point for SSE-derived state and returns `true` when the caller should trigger a transcript refetch (assistant just returned to Input mode).
- `services/claudemon/src/tui/sse.rs` — minimal SSE client: GETs `{api}/events` with `accept: text/event-stream`, reconnect loop with fixed backoff (2s on connect failure/non-2xx, 1s after mid-stream drop), byte-buffer framing via `drain_sse_frames` (splits on `\n\n`), `parse_sse_frame` extracts `data:` lines and calls `app::parse_sse_data` to deserialize into `SessionUpdate`.
- `services/claudemon/src/tui/view/mod.rs` — pure `render(frame, &App)` dispatcher to `dashboard::render_dashboard` or `chat::render_chat`; also hosts shared rendering helpers: markdown-to-ratatui-`Line` conversion (`render_markdown_text`, `inline_markdown_spans`), word-wrap (`wrap_spans`/`wrap_str`), mode badges (`mode_badge`, `badge_token`), and `cat -n` gutter splitting (`split_cat_n_line`) used for Read-tool output.
- `services/claudemon/src/tui/view/dashboard.rs` — single-screen layout: 1-row header (connection dot + session count), session list, fixed 11-row details panel (mode/cwd/last event/pending payload/gate), toast row, 1-row hint footer.
- `services/claudemon/src/tui/view/chat.rs` — focused per-session chat: transcript pane (with pending-approval/question banners, tool-use/tool-result rendering, syntax-highlighted code fences) + multi-line input box sized off `Editor::visual_rows`.
- `services/claudemon/src/tui/editor.rs` — UTF-8-safe line-editor for the chat composer: byte-offset cursor, `HISTORY_CAP = 100` ring buffer for sent messages, `history_prev/next` browsing with a `composing` stash.
- `services/claudemon/src/tui/syntax.rs` — `syntect`-backed `Highlighter`, grammars from `two_face::syntax::extra_newlines` (adds TypeScript/TSX/JSX over syntect defaults), fixed `base16-ocean.dark` theme, lazily built via `once_cell::Lazy`.
- `services/claudemon/src/tui/preview.rs` — NOT the chat rendering path; it is a `ScenarioBuilder` + `snapshot_dashboard`/`snapshot_chat` test-harness module that renders `App` states into a `TestBackend` string buffer, consumed by `services/claudemon/tests/ui_preview.rs` (`cargo test --test ui_preview -- --nocapture`) for eyeballing UI states without a real terminal.

## Failure modes
- Initial fetch (`App::refresh_initial`, GET `/sessions`) is best-effort: failure just shows a toast (`services/claudemon/src/tui/mod.rs` (L35-37)) and the UI still comes up with an empty session list; user must press `r` to retry.
- SSE reconnects forever with a fixed backoff (no cap/jitter) and never surfaces a fatal error to the user beyond the header's connection dot going red and a one-off toast (`sse.rs:26-35`); a daemon that is down long-term just spins this loop silently in the background.
- `BroadcastStream` lag (daemon side, `daemon/api.rs` `event_stream`) drops events and only logs a `tracing::warn!` server-side — the TUI has no way to detect it missed updates; it will look "stuck" on stale state until the next successful `session.update` or a manual `r` refresh.
- Approve/answer/gate/send actions all fail soft: non-2xx or transport errors become a toast (`app.rs` `act_approve`/`act_answer`/`act_toggle_gate`/`act_send_message`), never a panic or blocking error — but `act_send_message` also restores the typed text into the editor on failure so it isn't lost.
- Chat auto-refresh is edge-triggered only on a mode transition away-from-then-back-to `Input` (`apply_event`, `app.rs:298-308`); if an SSE update is lost during that transition (see lag above) the transcript can silently stop auto-updating until the periodic 1s poll (`mod.rs:113-117`) catches it, or the user hits `r`.
- `PushKeyboardEnhancementFlags` (kitty keyboard protocol) is requested with `let _ = ...` — silently ignored on terminals that don't support it, meaning Alt/Shift/Ctrl+Enter disambiguation degrades to Ctrl+J-only newline entry with no user-visible signal.

## Gotchas
- Name collision risk: "claudemon watch" TUI vs `apps/tui` (`wks-tui`) are unrelated crates with similar purposes (both are session-monitoring TUIs). When asked to "modify the TUI" always confirm which one is meant — this doc covers only `services/claudemon/src/tui/`.
- This module must track the daemon's wire API by hand: routes are wired in `services/claudemon/src/daemon/api.rs` (`/sessions`, `/sessions/:id/{input,message,approve,answer,gate,transcript}`, `/events`). There is no shared client crate/OpenAPI codegen — `app.rs` builds URLs and JSON bodies as raw strings/`serde_json::json!` literals, so a route rename or payload shape change in `api.rs` silently breaks this TUI at runtime (compiles fine, fails with a toast).
- `AppEvent::Update` deserializes into `SessionUpdate { session_id, event, state: SessionState }` via `parse_sse_data`; `SessionState` is the same daemon-side session model — any field/shape change to `SessionState` (in `crate::session`) must stay in sync since the TUI has its own `#[derive(Deserialize)]` copy of the wrapper (`app.rs:28-33`), not a re-exported daemon type.
- `App::gates` is a client-side mirror only, reset on `refresh_initial` — if two `claudemon watch` instances (or the watch TUI + another daemon client) toggle the same session's gate, this TUI's local gate display can drift from the daemon's true state until the next full refresh.
- Quick-action keys (`a`/`d`/digit) are intentionally suppressed while the chat editor has text (`input_empty` checks in `handle_chat_key`, `mod.rs:222-243`) to avoid accidental approve/deny while typing — a resurrected/duplicated hotkey must preserve that guard or typed text like "deny" would trigger an action.
- `render_cache` on `ChatState` (`app.rs:61-66`, `TranscriptRenderKey`) caches syntax-highlighted transcript lines keyed on message count + last-message signature + wrap width + tool-expand flag; any new mutable rendering input (e.g. a new toggle) that isn't added to `TranscriptRenderKey` will silently render stale cached output.
- Only `/events` (session.update stream) is actually consumed by this TUI (`sse.rs`, hard-coded to `{api}/events`) — `/hooks/stream` and `/statusline/stream` exist on the daemon and are consumed by other clients (desktop, wks-tui) but this embedded TUI does not subscribe to them; don't assume feature parity with those streams here.
- `services/claudemon/src/tui/preview.rs` is a test-only rendering harness (used from `services/claudemon/tests/ui_preview.rs`) despite being a `pub mod`, not gated behind `#[cfg(test)]` — it's fine to read for understanding rendering scenarios but is not part of the live `claudemon watch` runtime path.
