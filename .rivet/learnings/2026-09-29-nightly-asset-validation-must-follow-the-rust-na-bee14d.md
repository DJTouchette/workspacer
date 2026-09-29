---
title: Nightly asset validation must follow the Rust native installer name
date: 2026-09-29
promoted: false
---

# Nightly asset validation must follow the Rust native installer name

## Observation
release.yml still required Workspacer-Native-Setup-*-x64.exe after package-windows.mjs and installer smoke changed to Workspacer-Native-Rust-Preview-Setup-<version>-x64.exe. Manual packaging passed because only nightly publication executes the asset gate. Corrected the required glob while preserving migration-ready and all-platform build gates.
