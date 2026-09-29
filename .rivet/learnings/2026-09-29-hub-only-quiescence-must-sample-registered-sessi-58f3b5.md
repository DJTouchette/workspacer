---
title: Hub-only quiescence must sample registered session providers through a private client
date: 2026-09-29
promoted: false
---

# Hub-only quiescence must sample registered session providers through a private client

## Observation
Actual watcher audit found NativeSources failed closed forever whenever options.engine was None, even if a central/hub-only host had a live registered sessions.snapshots provider. sessions::install returns early without an engine; Core selects explicit handlers before provider registrations, and the watcher never provides sessions.snapshots itself. The fallback now uses a private internal service Client for that literal method within the unchanged10s read bound, preserving no-provider and malformed-response blockers and all owned-engine launch/workflow/terminal blockers. Four real WS/provider/Watcher composites cover all eight original Go test responsibilities, including exact request-sequence exclusion, infrastructure filtering, source-read demand gating and15minute cutoff, stale wire shape, missing/unknown sessions, and shell schedule exemption. Full library306+jobs8+quiescence3 passed.
