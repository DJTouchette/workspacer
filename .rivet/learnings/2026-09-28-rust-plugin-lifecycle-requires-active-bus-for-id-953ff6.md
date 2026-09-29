---
title: Rust plugin lifecycle requires active bus for identity registration
date: 2026-09-28
promoted: false
---

# Rust plugin lifecycle requires active bus for identity registration

## Observation
The Rust plugin Manager registers and revokes tokens by awaited Handle commands, so startup load must run after the bus command loop starts and stop must run before that loop exits. Sidecar replacements stop the old direct child before registering replacement identity. Plugin settings use the existing .settings.json overlay and redact all read/event paths; only WKS_SETTINGS delivers plaintext. Legacy Go plugin package is internal/plugin (singular), despite old context references.
