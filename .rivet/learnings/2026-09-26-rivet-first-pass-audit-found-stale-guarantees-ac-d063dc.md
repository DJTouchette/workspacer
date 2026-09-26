---
title: Rivet first-pass audit found stale guarantees across routing, UI, and launch paths
date: 2026-09-26
confidence: high
related_paths:
  - .rivet/context/domains/config.md
  - .rivet/context/domains/usage-accounting.md
  - .rivet/context/domains/desktop-remote-client-mode.md
  - .rivet/context/modules/webview-security-hardening.md
promoted: false
---

# Rivet first-pass audit found stale guarantees across routing, UI, and launch paths

## Observation
Current source has TS and Go pricing overrides and TTL-aware cache-write costing; workers-only pairing does not switch the renderer backend; backend installation no longer has the documented retry helper; the webview admits checked local files and has navigation/popup guards; TUI handoff sends after spawn; workflowAgentIds is derived only from the emitted run slice. Config write/lock failures can return the prior value, and persist-blocked saves may be memory-only. These findings were corrected in the corresponding context guides; deeper remaining coverage is tracked separately in docs/reviews/workspacer-rivet-audit.md.

## Recommendation
Continue the source-review ledger; clean metadata and references do not prove all behavioral guidance is current.
