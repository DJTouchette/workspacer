---
title: Facade cleanup must be tested against the persistent credential store
date: 2026-09-29
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - services/hub-rs/tests/session_facade.rs
promoted: false
---

# Facade cleanup must be tested against the persistent credential store

## Observation
Legacy facade tests bind spawn failure and repeated stopped observations to credential deletion. Testing a fake preparation callback alone does not prove persistent token cleanup. The new session_facade integration exercises both provider routes, actual credential minting, a token-path filesystem failure, successful retry, and duplicate-stop avoidance; the owning session_facade target passed all 11 tests in /tmp/workspacer-facade-parity-final.log.

## Recommendation
Retain both generation fencing tests and real SessionFacade persistence tests when changing launch cleanup.
