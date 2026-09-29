---
title: Go malformed regexp repeats are literals, but bad numeric bounds are errors
date: 2026-09-29
promoted: false
---

# Go malformed regexp repeats are literals, but bad numeric bounds are errors

## Observation
Go regexp.parseRepeat treats a malformed shape such as a{foo}, unmatched {, or leading-zero counts a{01} as literal text. Well-shaped reversed/excessive numeric bounds remain invalid. The Rust jobs adapter now tokenizes that distinction before regex-syntax, keeping braces inside p/P/x escapes intact. The optional named Go oracle captured16 additional independent cases (79 total), while direct Rust guard tests retain ASCII classes/boundaries and Unicode-property support.
