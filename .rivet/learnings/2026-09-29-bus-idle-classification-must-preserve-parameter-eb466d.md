---
title: Bus idle classification must preserve parameter presence and types
date: 2026-09-29
promoted: false
---

# Bus idle classification must preserve parameter presence and types

## Observation
Go passiveCall decodes typed structs, accepting case-insensitive known keys and explicit null but rejecting missing readiness parameters or malformed known values. Rust Value-only checks previously treated CHECK:true as a passive readiness poll and lost absent/null distinction. Broker now passes Option<&Value>; ambiguous duplicate aliases conservatively count activity because Value cannot recover wire field order. NO PROVIDER diagnostics are once per method outage, reset by accepted registration, bounded to avoid untrusted-name memory growth, and never include parameters.
