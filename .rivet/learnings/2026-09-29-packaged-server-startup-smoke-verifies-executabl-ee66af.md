---
title: Packaged server startup smoke verifies executable archive but not MCP protocol negotiation
date: 2026-09-29
confidence: high
related_paths:
  - scripts/smoke-server-bundle.py
  - .github/workflows/release.yml
promoted: false
---

# Packaged server startup smoke verifies executable archive but not MCP protocol negotiation

## Observation
The published 1bf2f53a Linux server archive passes extracted alias startup with empty PATH, isolated state and parent pipe, authenticated brain.info-backed status, MCP catalog health, API/hook/web probes and joined four-listener shutdown. scripts/smoke-server-bundle.py now runs this after server archive creation on all release platforms. MCP health/catalog-ready does not execute tools/list or negotiate newer protocol revisions. Windows listener closure is checked by process join and refused TCP connections, without exclusive rebind that can reject TIME_WAIT sockets.

## Recommendation
Run the same smoke on Windows/macOS CI and add explicit negotiated MCP tools/list contract coverage for client protocol failures.
