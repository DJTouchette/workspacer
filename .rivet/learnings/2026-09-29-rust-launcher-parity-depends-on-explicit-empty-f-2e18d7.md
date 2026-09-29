---
title: Rust launcher parity depends on explicit empty flags and ownership modes
date: 2026-09-29
suggested_doc: workspacer-serve-cli
promoted: false
---

# Rust launcher parity depends on explicit empty flags and ownership modes

## Observation
Go launcher contracts include explicit empty plugin paths, config-first usage.pollOnBoot, single-dash long flags, encoded token pairing links and a quiet scan before plugin rebuild. Rust CLI now preserves those boundaries using OsString path parsers, context-aware argv normalization and raw-config EngineOptions. Bare external-claudemon remains accepted only with explicit hub-only mode and exact daemon health proof; old full-stack borrowed-engine supervision is intentionally not restored. Portable CLI tests separately prove stdin EOF and declared-parent PID death while the stdin writer remains open.
