---
title: Native projects now read and write the hub's shared config.projects registry
date: 2026-10-02
promoted: false
---

# Native projects now read and write the hub's shared config.projects registry

## Observation
apps/native/src/projects.rs is the native projects model: the canonical store is the hub's config.yaml projects map (desktop projectRegistry.ts semantics: favourite, lastOpened, legacy directories.favourites/recent read-only), folded with native-settings.json hub-scoped bookmarks (device-only, legacy) and session cwds. config.save replaces /projects WHOLESALE and answers a skipped save (config lock held, write failed) with the unchanged config rather than an error, so every write rebuilds the map from a fresh config.get and verifies the readback (features.rs save_project). config.save needs operator scope; a refused pin offers an explicit keep-on-device fallback. Removal is refused for entries carrying anything beyond favourite/lastOpened or present in scripts/widgets. Acknowledged launches TouchProject (lastOpened) like the desktop's recordRecentDir. Spawn metadata records projectCwd but sessions.snapshots does not expose it, so native project-session linkage is still exact cwd (same_dir: forward-slash key, case-insensitive only for drive-letter/UNC paths). tests/projects_hub.rs proves this against an isolated NativeHost Rust hub (no providers).
