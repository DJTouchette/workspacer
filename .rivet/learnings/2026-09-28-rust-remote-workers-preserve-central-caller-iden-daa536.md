---
title: Rust remote workers preserve central caller identity and separate node liveness
date: 2026-09-28
promoted: false
---

# Rust remote workers preserve central caller identity and separate node liveness

## Observation
The Go brain --hub topology is retained through an owned Rust provider relay. Local brain.info handlers must not count as remote-node readiness: node probes use actual provider connection IDs and CAS eviction after two silent strikes. Negotiated providerCaller metadata binds delegated local calls to immutable central scope, fingerprint and connection ID; callerClosed revokes cached per-caller contexts. Full worker MCP requires a distinct scoped operator facade credential, never its provider token or local host token. The worker disables automatic central jobs/plugin/push/peer/node owners and reads upstream layouts from a bounded readonly cache. Plugin Electron runtime substitution is shared by installation/build and sidecar execution, only for bare node/node.exe, preserving exact install consent and explicit executable paths.
