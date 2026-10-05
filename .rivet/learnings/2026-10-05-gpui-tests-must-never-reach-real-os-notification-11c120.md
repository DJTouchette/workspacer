---
title: GPUI tests must never reach real OS notification FFI (Windows serial AV)
date: 2026-10-05
author: claude (native AV worker)
confidence: medium
related_paths:
  - apps/native/src/ui/features.rs
  - apps/native/src/ui.rs
  - vendor/gpui/src/platform/test/platform.rs
  - .github/workflows/native-client.yml
promoted: false
---

# GPUI tests must never reach real OS notification FFI (Windows serial AV)

## Observation
native-client CI 37343915868: wks-native unit binary died STATUS_ACCESS_VIOLATION starting finished_native_subagents_leave_the_sidebar_once_the_parent_turn_ends (proven by its flushed 'test NAME ... ' partial line, which GitHub joined to the next binary's 'running 5 tests'). sync_features posts notify_rust toasts when a session transitions (attention_transition) while the window is inactive; TestWindow::is_active is always false, so any UI test with a responding->input / new approval / new questions transition posted a REAL WinRT toast on the GPUI test thread (TestDispatcher runs background tasks there; Windows TestPlatform OleInitialize/OleUninitialize per test). In serial order that test is the 2nd toast-raising test (after enter_sends_setting_swaps_send_and_newline_in_the_composer_only, added 291ff96f); parallel runs passed. Stack not yet captured: cdb diagnostics added in da056af9.

## Impact
Real platform side effects from unit tests crash Windows serial runs and post notifications to developers' Linux desktops via D-Bus.

## Recommendation
Route platform side effects through a cfg(all(test, feature="ui-tests")) seam (see post_attention_alerts/POSTED_ALERTS in apps/native/src/ui/features.rs). Identify a crashed serial libtest test from its unfinished 'test NAME ... ' prefix, not from the last 'ok' line.
