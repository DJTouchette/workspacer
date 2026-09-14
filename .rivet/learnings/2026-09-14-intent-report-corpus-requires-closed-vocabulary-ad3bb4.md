---
title: Intent report corpus requires closed vocabulary even with TS and Rust loaders
date: 2026-09-14
confidence: high
suggested_doc: registration-checklists
related_paths:
  - contracts/intent-report-cases.json
promoted: false
---

# Intent report corpus requires closed vocabulary even with TS and Rust loaders

## Observation

The intent-report corpus had working TypeScript and Rust adversarial loaders but lacked vocabulary.blocks.cases, which makes both repository vocabulary guards fail. Declare required input/output fields and exact loader file::needle entries; output is report text rather than a finite verdict vocabulary. Existing governance mutation tests cover missing declarations and unknown fields. The broader Go contract check also caught the missing contracts/README.md owner row, so passing the vocabulary validators alone was insufficient.

## Recommendation

Run both corpus vocabulary guards and the Go declared-block-loader resolver when adding or changing shared report fixtures.
