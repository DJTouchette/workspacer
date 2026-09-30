---
title: Automatic native previews need supersession during frequent pushes
date: 2026-09-30
promoted: false
---

# Automatic native previews need supersession during frequent pushes

## Observation
rust-native-preview.yml previously had no concurrency group, leaving many old main pushes building Windows native artifacts while newer validated batches arrived. Automatic runs now share a workflow/event/ref group with cancellation, while workflow_dispatch uses its unique run ID and remains independent. This workflow has contents:read and never publishes releases. Actionlint and whitespace checks passed; no claimed compiler or app runtime speedup.
