---
title: Rust resume picker reuses the existing transcript path contract
date: 2026-09-28
promoted: false
---

# Rust resume picker reuses the existing transcript path contract

## Observation
The new hub resume picker reuses claudemon::session::transcript::project_dir_name instead of adding another Rust encoder. Root-only separators encode to empty and must refuse despite older comments claiming a dash. The actual shared path corpus pins that behavior. Picker reads newest20 regular transcript heads (8KiB), ignores agent-prefixed files, clips to100 Unicode scalar values and prefers an explicit summary row; mtime order is independent of the transcript header, allowing bounded head reads without changing returned rows. Missing source conversation still cannot be reconstructed from a picker summary.
