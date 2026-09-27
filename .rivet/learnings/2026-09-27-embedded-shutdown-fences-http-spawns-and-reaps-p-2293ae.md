---
title: Embedded shutdown fences HTTP spawns and reaps PTYs after runtime teardown
date: 2026-09-27
promoted: false
---

# Embedded shutdown fences HTTP spawns and reaps PTYs after runtime teardown

## Observation
Axum listener abort stops accepting but existing request tasks can continue; the embedded lifecycle fences spawn handlers with an RwLock before child cleanup. Managed adapter startup runs after the spawn response, so retain SessionStore until the dedicated runtime is dropped and perform a final PTY kill/reap sweep afterwards. portable-pty children have no kill-on-drop, unlike Tokio provider processes.
