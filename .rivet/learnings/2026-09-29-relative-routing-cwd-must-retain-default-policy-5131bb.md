---
title: Relative routing cwd must retain default policy rather than process cwd policy
date: 2026-09-29
suggested_doc: limit-aware-routing
related_paths:
  - services/hub-rs/src/services/routing.rs
  - services/hub-rs/src/services/routing/audit.rs
promoted: false
---

# Relative routing cwd must retain default policy rather than process cwd policy

## Observation
Go bus CanonicalizeRoot rejects non-absolute cwd. Rust routing canonical formerly called fs::canonicalize directly, accepting dot or an existing relative path and selecting a ceiling against the hub process directory. Routing canonical now requires Path::is_absolute first; a positive floor shows the same absolute directory still selects its explicit policy while dot retains default ceiling. Allowed clamp audit also needs a fixed routing.yaml remedy reason; outcome flags alone miss the legacy operator diagnostic.

## Impact
Relative request spellings must not make routing depend on the hub launch directory or bypass the default ceiling.

## Recommendation
Keep absolute-directory positive floor and relative-dot default-policy assertions; never serialize arbitrary request/error text into audit reasons.
