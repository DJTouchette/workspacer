---
title: Rust launch integration authority is an in-memory broker permit rather than JSON grants
date: 2026-09-28
promoted: false
---

# Rust launch integration authority is an in-memory broker permit rather than JSON grants

## Observation
The all-in-process Rust preparation adapter wraps SessionFacade and requires a typed broker LaunchPermit bound to pending call id, live host connection, selected plugin, session identity and nonce. JSON launchIntegrationGranted cannot populate its registry. It checks the proof before and after preparation, consumes it once, retains only selection metadata in the lifecycle journal, and delegates facade sweep/revoke. Codex config is read through an owned bounded app-server stdio probe; plugins receive only provider id/base URL and whitelisted context. Patch shape follows TS, with additional known identity/model/permission/facade override rejection so appended plugin arguments cannot contradict routing receipts; Headroom base URL overrides remain allowed.
