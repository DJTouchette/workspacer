---
title: Retired intent data requires a live startup preservation test
date: 2026-09-29
suggested_doc: headless-desktop-services
promoted: false
---

# Retired intent data requires a live startup preservation test

## Observation
The old private Node integration seeded intent-workspaces.sqlite at user_version10 plus intent-artifacts/legacy.txt with ui.intentWorkspaces still true, then required byte preservation/no WAL and rejection of the retired RPC. A name/catalog inventory does not establish that upgrade guarantee. The Rust legacy_preservation fixture now exercises two real Backend starts and current config/brief RPCs around the retired-method rejection, with temp-only paths and explicit disabled usage polling.
