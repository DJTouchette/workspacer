---
title: Hub context crosswalks must identify Rust owners without rewriting retained contracts
date: 2026-09-30
confidence: high
suggested_doc: architecture-overview
related_paths:
  - .rivet/context/modules/hub-bus-control-plane.md
  - .rivet/context/modules/hub-process-supervision.md
  - .rivet/context/modules/hub-plugin-system.md
promoted: false
---

# Hub context crosswalks must identify Rust owners without rewriting retained contracts

## Observation
Seven hub-* contexts still indexed old Go modules as current source owners. Prep now verifies and names actual Rust runtime/auth/protocol, federation/client, jobs/admin, plugin supervisor, parent/owned-process and push owners in frontmatter/current crosswalk notes. Existing protocol/policy descriptions remain, but Go-specific source/helper/rebuild/test details are explicitly historical and optional execution points to the pinned reference wrapper.

## Recommendation
Keep source crosswalk maintenance separate from protocol redesign or release certification. Verify every new related_path and actual owning test target; do not infer a Rust module name by translating a Go package name.
