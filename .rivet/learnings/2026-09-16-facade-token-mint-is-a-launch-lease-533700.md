---
title: Facade token mint is a launch lease
date: 2026-09-16
promoted: false
---

# Facade token mint is a launch lease

## Observation
A per-session facade token is minted before integration preparation, MCP config creation, metadata registration, and the daemon launch. Treat that token as a launch lease: every error after mint must revoke it, ownership transfers to session teardown only after the daemon acknowledges launch, and headless live-to-ended transitions must revoke it independently of explicit close.
