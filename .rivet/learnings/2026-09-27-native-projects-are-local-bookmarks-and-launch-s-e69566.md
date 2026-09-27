---
title: Native projects are local bookmarks and launch settings are a limited subset
date: 2026-09-27
promoted: false
---

# Native projects are local bookmarks and launch settings are a limited subset

## Observation
apps/native/src/ui/navigation.rs implements Projects as exact-directory session groups plus hub-scoped local bookmarks, not shared hub project configuration. apps/native/src/ui.rs exposes Claude/Codex, directory, label, a free-text model override, and first message. apps/native/src/controller.rs NewSession::params always requests stream transport and skipPermissions=false; there is no permission-mode picker or provider model catalog in this launch flow. Runtime approve/deny and answer actions are implemented separately.
