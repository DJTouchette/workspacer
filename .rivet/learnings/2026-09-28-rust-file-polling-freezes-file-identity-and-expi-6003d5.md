---
title: Rust file polling freezes file identity and expires browser leases
date: 2026-09-28
promoted: false
---

# Rust file polling freezes file identity and expires browser leases

## Observation
Go filewatch intentionally polls metadata rather than reading bytes, preserving watches across deletion/atomic replacement. Identity must be sampled immediately on Windows: size/mtime equality does not prove the same file, and lazy pathname identity can observe a later replacement. Rust filewatch uses native volume/file IDs on Windows and device/inode on Unix, 500ms owned polling, 3-minute renewable named leases, reference counts for old clients, bounded1024 paths/128 leases, and drops a canonical path whose symlink target changes. Source assertPathAllowed is ambient absolute-path authority (roots retained only for compatibility); semantic containment remains object-specific.
