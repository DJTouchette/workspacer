---
title: Native New session must explicitly release a --session pin
date: 2026-09-27
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/ui.rs
  - apps/native/src/main.rs
  - apps/native/README.md
promoted: false
---

# Native New session must explicitly release a --session pin

## Observation
The native app had repeatedly been launched with --session to focus the user's conversation. That flag disabled the New session button and show_new_session returned immediately, so clicks appeared to do nothing. New session now explicitly clears requested_session/selection_requested, refreshes any pin-suppressed state, opens the form, and permits a successful creation to select its new session. Automatic startup/refresh still cannot silently send to an unrelated session while pinned. A GPUI regression clicks the real New session button from a pinned view, asserts opening emits no Create, submits once, and confirms the creation receipt selects the new session. Launch ordinary interactive use without --session.
