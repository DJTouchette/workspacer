---
title: Capability provenance can survive deletion only through a sealed reviewed capture
date: 2026-09-30
confidence: high
related_paths:
  - tools/capability-source-check/src/reference.rs
  - tools/capability-source-check/go-reference-provenance.json
promoted: false
---

# Capability provenance can survive deletion only through a sealed reviewed capture

## Observation
Preparation keeps go-reference.json byte-identical with all84 dangerous bindings. An explicit version1 captured-provenance manifest pins reference commit0c077b82e62965ce51f85b2d2a4c0f1aec872fea/tree and capture SHA256; Rust pins the manifest digest. Only absent original paths in that exact capture are permitted. Present originals and current vocabulary still require original digests; missing live Rust owners still fail. Strict --verify-go-reference additionally requires pinned Git HEAD/tree and every historical scanner/source/vocabulary byte. Corrupt/missing manifests, corrupt capture, unknown paths and altered bindings all fail.

## Recommendation
Treat capture resealing as a deliberate source/binding review. Never generalize missing historical bytes into an unchecked fallback, and do not conflate portability preparation with authorization to delete Go or with release gate execution.
