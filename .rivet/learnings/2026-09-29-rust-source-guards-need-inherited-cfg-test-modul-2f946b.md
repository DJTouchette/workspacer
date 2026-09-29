---
title: Rust source guards need inherited cfg test module ownership
date: 2026-09-29
promoted: false
---

# Rust source guards need inherited cfg test module ownership

## Observation
A cfg(test) parent can declare an ordinary #[path] child; treating every ordinary declaration as globally live incorrectly promotes nested test fixtures into production source scans. rustProductionSource now builds module edges, propagates definitely-nonproduction cfg ancestry, then restores actual live references. Production references to the same file and unreferenced files remain scanned, regardless of tests-like filenames. Regressions cover nested explicit/default modules, inline scopes, cfg all(test,..) versus any(test,..), attribute order and CRLF.
