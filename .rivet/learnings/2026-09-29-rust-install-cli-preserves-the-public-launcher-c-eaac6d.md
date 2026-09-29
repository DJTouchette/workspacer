---
title: Rust install-cli preserves the public launcher command
date: 2026-09-29
promoted: false
---

# Rust install-cli preserves the public launcher command

## Observation
The shipped artifact remains workspacer-rust, but install-cli must publish workspacer (workspacer.exe on Windows) from current_exe, never resolve an existing PATH command which may still be Go. Unix preserves writable /usr/local/bin preference with user fallback and symlink-to-copy fallback; Windows retains per-user destination and in-use executable rename. Tests inject directory probes and fake artifacts rather than installing into real system directories or mutating PATH.
