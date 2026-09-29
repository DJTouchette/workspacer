---
title: Rust upload suffix checks must not normalize trailing separators
date: 2026-09-29
related_paths:
  - services/hub-rs/src/services/uploads.rs
promoted: false
---

# Rust upload suffix checks must not normalize trailing separators

## Observation
Go filepath.Ext checks the final textual component. Rust Path::file_name normalizes trailing separators and dot components, so photo.png/ and photo.png/. were incorrectly accepted. uploads.rs now splits on native path separators before extracting the allowlisted suffix; regression cases also pin private file mode and both upload size limits.
