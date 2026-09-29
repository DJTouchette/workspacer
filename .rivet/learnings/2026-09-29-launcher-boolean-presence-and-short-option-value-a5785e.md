---
title: Launcher boolean presence and short option values are part of the ownership contract
date: 2026-09-29
suggested_doc: workspacer-serve-cli
promoted: false
---

# Launcher boolean presence and short option values are part of the ownership contract

## Observation
Go launcher booleans accept explicit true/false spellings, and an explicit --allow-new-token=false must override WORKSPACER_ALLOW_NEW_TOKEN=1. Rust uses a presence-aware option at this boundary. Context-aware normalization must also reserve short -f values, not merely long flags; joining the unchanged OsString value with an equals separator preserves dash-prefixed names without accidentally treating them as authority options. Retired child-bin overrides fail explicitly rather than finding Go on PATH.
