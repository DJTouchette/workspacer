---
title: Rust Fly defaults preserve explicit identity migration and backend ownership
date: 2026-09-29
suggested_doc: fly-node-deploy
related_paths:
  - deploy/fly/rust/Dockerfile
  - deploy/fly/rust/boot-rehearsal.sh
promoted: false
---

# Rust Fly defaults preserve explicit identity migration and backend ownership

## Observation
Default Fly role Dockerfiles are generated from deploy/fly/rust/Dockerfile and release archives require workspacer-rust/claudemon. The retained node bootstrap seeds config before backend startup, so a first worker/combined local identity must be explicitly provisioned before launching on that volume; production entrypoints never add allow-new-token. The Docker boot rehearsal performs that step only on its newly created volumes and uses distinct provider and facade caller credentials. Existing dual-UID combined supervisors use the audited AST transformer, not generic single-user image replacement.

## Impact
A Rust-only release cannot be consumed by Go-default image builders; reusing provider credentials or auto-resetting missing local identity would violate persisted authority.

## Recommendation
Run render-dockerfiles.py --check and the Rust container contract workflow; leave deployment/platform gates pending until actual image boots pass. Use build-upgrade.sh for custom/protected images rather than legacy WKS_BASE layering.
