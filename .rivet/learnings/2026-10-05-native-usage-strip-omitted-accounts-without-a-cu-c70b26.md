---
title: Native usage strip omitted accounts without a current reading
date: 2026-10-05
confidence: medium
suggested_doc: usage-accounting
related_paths:
  - apps/native/src/usage.rs
  - apps/native/src/ui/usage.rs
  - apps/native/src/controller.rs
promoted: false
---

# Native usage strip omitted accounts without a current reading

## Observation
usage::accounts skipped every account with no measured current window, so a Claude login whose windows were unknown (not polled yet / needs_reauth) disappeared and the sidebar showed only Codex. usage.report also carries a pseudo-account (label 'unattributed', source 'transcript') and providers whose windows are all structurally 'unavailable' (Copilot). Now: unattributed and all-unavailable accounts are left out; any other account without a reading is listed with an explicit state (Sign in again / Refresh failed / No reading yet + reason). A failed usage.report with nothing cached shows 'Usage unavailable' via View.usage_error.
