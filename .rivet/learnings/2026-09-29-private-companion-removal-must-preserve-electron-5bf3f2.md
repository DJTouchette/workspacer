---
title: Private companion removal must preserve Electron shared dispatcher imports
date: 2026-09-29
suggested_doc: headless-desktop-services
promoted: false
---

# Private companion removal must preserve Electron shared dispatcher imports

## Observation
nativeDesktopServices.ts imports headless/desktopHost.ts, which still imports the private hostBridge and headless manager-replacement adapter alongside retained files/uiAssets. stdio.ts and the explicit npm build/test scripts are removable transport/build paths, but deleting the headless directory breaks Electron. Strip companion-only internal cases and the already-intercepted managerReplacement fallback before removing bridge/analytics adapters. Negative Windows packaging fixtures intentionally mention old CJS/Node artifacts. Exact portable test gaps found: retired intent SQLite preservation and managed-model analytics cost assertion.
