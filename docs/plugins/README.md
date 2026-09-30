# Public plugin documentation

- [Plugin theming](plugin-theming.md): retained theme tokens and webview hooks.
- [Workflow events](workflow-events.md): retained event envelopes and examples.
- [Rules engine build spec](rules-engine-plugin.md): an optional plugin design,
  not a claim that a rules-engine plugin ships in the core application.
- [Bundled examples](../../plugins/examples): live retained plugin source.
- [Launch integrations](../launch-integrations.md): public launch contract.
- [MCP tool catalog](../../services/hub-rs/assets/mcp-effective-tools.json) and
  [Rust MCP implementation](../../services/hub-rs/src/mcp.rs).

These documents were copied from the tracked `services/hub/docs` reference files;
the originals remain untouched. `provenance.json` records the source checkpoint,
original hashes, retained hashes and the narrow adjustments. The theming document
is byte-identical. The other documents update backend/source-location prose only;
rule schemas, event tables and plugin API code blocks are preserved. The obsolete
Go hub build/start recipe is replaced by the current Rust control-plane command;
this operational adjustment is recorded separately from API examples.

Optional sidecar language choices (including Go or Node) concern public trusted
plugin programs. They are separate from the retired private Go/Node backend.
