# Rust source assets

`provider-scope-overlaps.json` is a Rust-only migration review reference, consumed
by `src/provider_relay/tests.rs`. It captures the 55 declared legacy overlaps and
records current per-method rationales plus required adopted-provider disclosures.
It is not a shared cross-language behavioral corpus or a runtime authorization
policy. The Rust test reads the current desktop declarations and disclosure
markers; no TypeScript execution is claimed as evidence from that source read.

The retained Go path/hash records capture provenance only. Tests do not read Go
files, so deleting that reference implementation does not disable the guard.
Changing an overlap requires a current ownership rationale and review of the
corresponding desktop declaration, not merely increasing the expected count.

Other JSON files here are bundled/generated Rust assets; their generators and
checks are named by `make check-hub-rust-assets` or their owning test modules.
