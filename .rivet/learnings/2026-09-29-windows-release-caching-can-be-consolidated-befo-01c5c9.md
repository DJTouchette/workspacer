---
title: Windows release caching can be consolidated before parallelizing builds
date: 2026-09-29
promoted: false
---

# Windows release caching can be consolidated before parallelizing builds

## Observation
Read-only review of successful run36587781767: Windows Electron package16m29 (hub10m34, claudemon4m27), then native17m18. Services cache restored1326bytes; native post-cache saved908MB, followed by service-cache cleanup unable to find cargo. The two cache actions share Cargo home and rust-cache documents removal of preexisting Cargo-bin entries. Proposed follow-up: one release cache for three workspaces with cache-bin:false, then separate native compilation and artifact-dependent native packaging jobs. Preserve pinned SHA/version, CRT provenance, smoke and publish dependencies. No speedup measured for that proposal; current publishing run is unchanged. https://github.com/Swatinem/rust-cache#cache-details
