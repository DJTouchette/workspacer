---
title: Public plugin docs survive Go retirement with their examples and schemas intact
date: 2026-09-30
confidence: high
suggested_doc: hub-plugin-system
related_paths:
  - docs/plugins/README.md
  - docs/plugins/provenance.json
promoted: false
---

# Public plugin docs survive Go retirement with their examples and schemas intact

## Observation
Prep copies plugin-theming, rules-engine-plugin and workflow-events from tracked legacy docs into docs/plugins with original/destination SHA256 provenance. Theming stays byte-identical; other changes only name the actual Rust owner/source paths. Every fenced example remains identical. Active landing/build and launch-integration links now point at plugins/examples, retained docs and current Rust MCP catalog/implementation, while original documents and historical fixtures remain untouched.

## Recommendation
Keep the rules-engine document classified as a plugin build spec, not a shipped-core implementation claim. Preserve optional public Go/Node sidecars independently of deleting private backend runtimes. Verify links against retained owners and keep schema/examples separate from relocation prose.

## Operational recipe review
The subsequent prep review found live Go hub/MCP launch recipes still present in
landing build/plugin pages and the retained rules-engine testing section. Those
now name the shared Rust builder/serve path and distinguish repository-root
commands from an installed workspacer CLI. Four-binary packaging and TUI brain
bootstrap descriptions were corrected; local federation proof points to its
Rust integration target. The rules document's provenance exception explicitly
includes this obsolete operational-recipe correction. Fenced plugin API/schema
examples remain identical; all23 changed/new docs' relative links validated.
