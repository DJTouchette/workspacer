---
title: Native Codex model catalog needs discovery errors and default metadata
date: 2026-10-01
promoted: false
---

# Native Codex model catalog needs discovery errors and default metadata

## Observation
Verified the installed Codex app-server model/list returns model as the launch ID, displayName as presentation, and isDefault as the default marker (eight visible account models on 2026-10-01). Claudemon maps these into id/label/default correctly, and the Rust hub exposes an array that native consumes. The native picker formerly discarded default metadata and hid IDs behind labels; Rust hub formerly swallowed upstream discovery errors into an empty array. Preserve exact launch IDs, display the live default, and propagate discovery errors so native can distinguish failure from a genuinely empty catalog. Witness currently reports these native and hub model files unmapped, requiring the full native suite plus hub models tests.
