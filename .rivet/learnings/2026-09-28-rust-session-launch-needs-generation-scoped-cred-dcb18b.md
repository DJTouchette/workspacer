---
title: Rust session launch needs generation-scoped credential cleanup
date: 2026-09-28
promoted: false
---

# Rust session launch needs generation-scoped credential cleanup

## Observation
The Rust lifecycle journal preregisters attribution before token preparation and engine spawn, owns the operation after RPC cancellation, and requires generation-matched revocation. Old stopped observations must not revoke a resumed session bearer. Session token mint/revoke must share auth::update_records with pairing/plugin mutations to avoid load-modify-save lost updates. Exact facade health currently refuses the Rust migration facade by design; do not remove the agents.spawn admission gate merely because launch/injection code exists.
