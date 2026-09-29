---
title: Native Rust owner must explicitly retarget manual Claude hooks to its bound port
date: 2026-09-28
suggested_doc: native-embedded-backend
related_paths:
  - Keep backend/config preview isolation distinct from shared provider account/manual-hook integration. Tests must opt into a temp settings path and verify its actual owned listener
  -  not rewrite ambient HOME.
promoted: false
---

# Native Rust owner must explicitly retarget manual Claude hooks to its bound port

## Observation
Old native Local starts embedded claudemon then Go serve --external-claudemon with the actual ready hook port; that launcher runs claudemon init against BaseDirs.home/.claude/settings.json (it does not use CLAUDE_CONFIG_DIR). New Rust Backend.prepare/initialize previously omitted init, leaving PTY/manual Claude hooks on an old or default7890 listener even when stream launch fixtures passed. Native now explicitly opts into Options.claude_hook_settings at chosenhome/.claude/settings.json; Backend uses engine.ready().hook_addr.port() with a path-explicit quiet helper. Default library construction never writes provider settings or mutates env. Overlay init was not a write-free substitute: existing run_overlay strips tagged hooks fromglobal settings, and additive overlay alone can double-fire oldhooks.
