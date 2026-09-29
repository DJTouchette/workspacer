---
title: Filesystem creation and empty-root semantics need explicit Rust defaults
date: 2026-09-29
confidence: high
suggested_doc: hub-bus-control-plane
related_paths:
  - services/hub-rs/src/services/files.rs
  - services/hub-rs/src/services/paths.rs
promoted: false
---

# Filesystem creation and empty-root semantics need explicit Rust defaults

## Observation
Rust Path::starts_with(empty) is true, unlike the retained Go containsPath contract. Empty selected roots now explicitly refuse. Rust default file/directory creation modes are broader under umask 0 than Go fs.write modes; explicit0644/0755 and isolated-child umask tests preserve the contract without changing process-global state in the test runner. fs.listDir defaults only ASCII blank input to home and must preserve literal absolute trailing spaces; generic malformed path values must not silently become home.
