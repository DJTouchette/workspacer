---
title: Native per-agent terminal: hub terminals.* contract and key routing
date: 2026-10-05
confidence: high
suggested_doc: native-embedded-backend
related_paths:
  - apps/native/src/terminal.rs
  - apps/native/src/controller.rs
  - apps/native/src/ui/terminal.rs
  - services/hub-rs/src/services/terminals.rs
promoted: false
---

# Native per-agent terminal: hub terminals.* contract and key routing

## Observation
Hub terminals.create spawns the host login shell as an engine session (mode 'unknown', appears in sessions.snapshots); sessions.attachTerminal replays the screen as pty.bytes.<id> events (subscribe BEFORE attaching or the replay is lost), leases expire after 20s without sessions.terminalKeepalive (false = re-attach), pty.exit/pty.desync are broadcast topics. Input must be serialized per shell (one in-flight terminalInput, coalesce the rest) or FuturesUnordered reorders keystrokes. GPUI runs keystroke interceptors before any binding, so a focused terminal takes every key there (Ctrl+N, Esc, Alt+arrows) instead of fighting key contexts. Interactive shells ignore SIGTERM; restart uses claude.signal SIGKILL.

## Impact
Shell rows leak into the session list and keys get eaten by workspace bindings unless handled this way.

## Recommendation
Keep shell ids (current + retired + remembered per hub) out of View.sessions; route terminal keys via intercept_keystrokes; output goes through the shared terminal Feed, not the per-frame View snapshot.
