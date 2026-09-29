---
title: Persisted job guards need explicit Go regex compatibility
date: 2026-09-29
promoted: false
---

# Persisted job guards need explicit Go regex compatibility

## Observation
Rust regex defaults give Unicode semantics to Perl shorthand classes while Go job skipUnlessMatch guards use ASCII classes and boundaries. A shared AST adapter preserves Go classes, quoting, octal and repetition validation; a reusable stdlib-only Go CLI captures independent cases and actually accepted property names. Pin Cn/LC/aliases and surrogate-class behavior, and state that Rust Unicode codepoint tables can be newer than Go15. The adapter is not yet a claim of exhaustive syntax equivalence; malformed-repeat literal cases remain under audit.
