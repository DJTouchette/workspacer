# Bundled trusted plugin examples

`examples/` owns the shipped clock, editor, Headroom and transcript-timeline
examples. Package destinations are unchanged; source ownership no longer depends
on the retiring Go module. Enabled plugin sidecars are public trusted extensions,
not the retired private Node desktop-services companion.

`examples.provenance.json` records the original tracked paths, checkpoint and
SHA256 values. All production/vendored bytes are identical to the retained
reference. Two explicit relocation-only exceptions have both hashes and reasons:
Headroom's test import climbs one fewer parent, and its README commands name the
new examples location and Rust plugin test. The original files remain untouched.

The same provenance manifest records the copied routing-preferences-view fixture
at `apps/desktop/tests/fixtures/routing-preferences-view.json`. It is an executing
TypeScript harness/test fixture, not a shared-language corpus. Both live renderer
imports now point there; its original reference bytes remain available.

Run `make check-retained-plugin-assets` for exact byte/provenance checks. This is
copy validation, not a claim that the legacy deletion or packaging gates passed.
