---
title: Native bus UI events need queued display intents and explicit unsupported panes
date: 2026-09-28
confidence: high
suggested_doc: native-embedded-backend
related_paths:
  - apps/native/src/ui_requests.rs
  - apps/native/src/ui/bus_commands.rs
promoted: false
---

# Native bus UI events need queued display intents and explicit unsupported panes

## Observation
Native controller previously subscribed only to agent snapshots/conversations, so facade.openTerminal and command navigation publications had no consumer. The native UI has no terminal pane; launching terminals.create upon receiving that event would create invisible work. Added bounded display intents, actual existing-screen navigation, and persistent unsupported notices with retained request metadata. UI consumption is not an external visible-pane acknowledgement. The socket sends an initial empty subscription at hello before the controller supplies its full topic set; protocol fixtures must wait for that later set.

## Recommendation
Keep terminal creation out of the headless controller and unsupported native panes. Test foreign-hub isolation, bounded queue, pinned sessions, prefilled spawn dialog without launch, and refusal of decision actions. Terminal widget work is a separate UI feature, while Rust PTY service remains owned by claudemon.
