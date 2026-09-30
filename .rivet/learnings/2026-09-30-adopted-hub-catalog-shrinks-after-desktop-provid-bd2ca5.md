---
title: Adopted hub catalog shrinks after desktop provider quits
date: 2026-09-30
confidence: high
suggested_doc: auto-update-release-channel
related_paths:
  - apps/desktop/scripts/smoke-electron-ownership.mjs
promoted: false
---

# Adopted hub catalog shrinks after desktop provider quits

## Observation
Release36668076976 passed prequit operator catalog then failed a repeated100-tool assertion after Electron quit. Rust MCP tools/list filters current methodNames, so external hub-only correctly loses desktop tools while local help remains. Ownership smoke must retain the100-tool prequit floor and prove modern metadata, desktop-tool removal, actual retained help call, and external owner EOF shutdown afterward.

Validation: helper tests 7/7 passed. An isolated already-built actual Rust hub
returned 20 retained tools, modern cache metadata, no desktop get_host_cwd, and
successful local help, then exited0 on owner stdin EOF. The first actual probe
caught a missing modern Mcp-Name header on tools/call; the helper and HTTP test
now require that header. This local control is not a packaged Electron receipt;
the full release smoke still must execute the real provider disconnect path.
