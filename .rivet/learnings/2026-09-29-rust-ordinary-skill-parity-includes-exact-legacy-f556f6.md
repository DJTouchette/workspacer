---
title: Rust ordinary-skill parity includes exact legacy-copy cleanup and byte-stable generation
date: 2026-09-29
confidence: high
suggested_doc: agent-spawn
related_paths:
  - services/hub-rs/src/services/launch_instructions.rs
  - scripts/generate-rust-launch-assets.py
promoted: false
---

# Rust ordinary-skill parity includes exact legacy-copy cleanup and byte-stable generation

## Observation
The Rust installer matched immutable versioned assets but omitted Go/desktop cleanup of exact older .claude/.agents native-discovery copies, including managers. Added provider-specific exact-byte regular-file cleanup after safe project validation, refusing symlink parents and preserving customized copies. Python generator now uses explicit UTF-8 bytes rather than locale/newline-translating text I/O; isolated ASCII-locale Unicode/CRLF regression verifies hash parity and stale detection.
