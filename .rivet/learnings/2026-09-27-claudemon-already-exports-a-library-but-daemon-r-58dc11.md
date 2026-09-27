---
title: Claudemon already exports a library but daemon run is process-owned
date: 2026-09-27
promoted: false
---

# Claudemon already exports a library but daemon run is process-owned

## Observation
services/claudemon/src/lib.rs already exports daemon/providers/session/store and the binary is a Tokio entry point. daemon::run still owns SIGTERM/Ctrl-C and parent-death handling, sets process-global API_BASE OnceCell, starts detached persistence/maintenance/tailer/poller tasks, and relies on runtime teardown for managed-provider cleanup. Embedding in apps/native requires an explicit readiness/shutdown handle and tracked task ownership; invoking run in the existing UI runtime is not yet a clean reusable lifecycle. The native client also depends on hub agents.spawn capability services, so embedding claudemon alone does not eliminate the hub/launch-service stack.
