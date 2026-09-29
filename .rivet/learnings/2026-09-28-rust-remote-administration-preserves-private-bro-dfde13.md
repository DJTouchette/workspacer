---
title: Rust remote administration preserves private broker and ordinary pairing boundaries
date: 2026-09-28
suggested_doc: remote-mobile
related_paths:
  - Inventory hub RegisterLocal methods separately from brain methods; do not infer operator-scoped tokens have authenticated-host administration authority.
promoted: false
---

# Rust remote administration preserves private broker and ordinary pairing boundaries

## Observation
Go hub remote.* is a separate local-owner surface, not a brain capability set. Tailscale status/serve use fixed argv or the explicitly configured WKS_NETWORK_ADMIN_SOCKET plus token file; remote.setSharing refuses launcher-managed listeners when no private broker exists and never rebinds the hub. Pairing administration only reads/revokes ordinary Remote Control-prefixed view/triage/operator records with no role, facade authority, YOLO/profile/plugin/provider grants. Rust get-or-create now uses the shared auth transaction and a mutation-free record constructor so concurrent calls cannot mint duplicates or overwrite infrastructure records. No real Tailscale mutation is exercised by fixtures.
