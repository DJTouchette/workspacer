---
title: Layout upgrade preserves missing and null version defaults
date: 2026-09-29
promoted: false
---

# Layout upgrade preserves missing and null version defaults

## Observation
The Go Document integer defaults to zero for omitted/null version, while the first Rust loader rejected those existing objects and discarded data. Preserve version zero with opaque data in Rust, reject malformed numeric shapes, and test disk loading plus diagnostics, live publication after persistence failure and concurrent atomic disk observation.
