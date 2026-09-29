---
title: Last-exit metadata needs canonical bounded reads and strict object shape
date: 2026-09-29
related_paths:
  - services/hub-rs/src/provider_relay/last_exit.rs
  - services/hub-rs/src/cli/serve.rs
promoted: false
---

# Last-exit metadata needs canonical bounded reads and strict object shape

## Observation
The last-exit relay reader now rejects positional JSON arrays instead of serde struct sequence decoding, preserves null optional at/exitCode as Go zero defaults, and bounds actual reads to64KiB. Host-selected relative and symlink file paths must be OS-canonicalized before files::bounded_bytes, which accepts canonical absolute paths. WKS_DATA retains Go whitespace trimming; explicit WKS_LAST_EXIT_FILE keeps precedence. Actual cached brain.info responses retain only reason/exitCode/at for a relay lifetime.
