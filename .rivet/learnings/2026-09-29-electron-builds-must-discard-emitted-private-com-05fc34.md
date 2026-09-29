---
title: Electron builds must discard emitted private companion leftovers
date: 2026-09-29
suggested_doc: headless-desktop-services
promoted: false
---

# Electron builds must discard emitted private companion leftovers

## Observation
Removing private stdio/bridge TypeScript sources is insufficient because tsc preserves old emitted JavaScript. The prebuild-main helper now clears dist/main plus exact old dist/headless CJS/map outputs before compiling, with a temp fixture asserting sources and web assets remain. Electron keeps its public desktopHost dispatcher and native manager replacement controller; all internal.* callback cases and Node stdio/build entry points are removed. Real Rust account add/list and legacy preservation tests cover the final private test gaps.
