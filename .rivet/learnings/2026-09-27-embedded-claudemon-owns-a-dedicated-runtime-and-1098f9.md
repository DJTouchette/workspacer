---
title: Embedded claudemon owns a dedicated runtime and retains hub launch setup
date: 2026-09-27
promoted: false
---

# Embedded claudemon owns a dedicated runtime and retains hub launch setup

## Observation
Native embedded mode must hold a single-process daemon lease through runtime teardown. Provider MCP callbacks use resettable API_BASE; binding both listeners before workers prevents partial-start leaks. Typed commands reuse the Axum router in process while agents.spawn stays on hub/brain for worktree and MCP setup. Embedded lifecycle avoids process signals, parent stdin monitoring, and Windows process job confinement.
