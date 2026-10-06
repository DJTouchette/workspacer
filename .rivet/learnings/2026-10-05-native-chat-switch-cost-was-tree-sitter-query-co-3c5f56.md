---
title: Native chat switch cost was tree-sitter query compilation per code block
date: 2026-10-05
confidence: high
suggested_doc: native-embedded-backend
related_paths:
  - vendor/gpui-component/src/highlighter/**
  - apps/native/src/ui/tests/switching.rs
  - apps/native/src/harness.rs
promoted: false
---

# Native chat switch cost was tree-sitter query compilation per code block

## Observation
perf (frame pointers) on the ui-tests bench_switch_frames workload: 73% of a parent/child switch was SyntaxHighlighter::new -> build_combined_injections_query -> tree_sitter::Query::new. gpui-component built a fresh SyntaxHighlighter, recompiling the language's injections+locals+highlights query (plus each injection language's), for every fenced code block on every first parse; TextView keyed state is dropped whenever rows stop rendering, so every switch re-parsed every visible block. Release: ~86ms UI-thread per switch before, debug ~600ms. vendor/gpui-component now shares one compiled query set per requested language name (COMPILED_QUERIES), cleared by LanguageRegistry::register.

## Impact
This was production cost, not only dev-build overhead. Debug builds (make dev-native*) are opt-level 0 with debug assertions (precondition_check frames in perf) and still pay ~25ms per no-op view republish; the controller republishes at up to 30Hz whenever any bus event arrives, which pegged the user's dev UI thread at ~90% while idle.

## Recommendation
Measure with make bench-native-switch (PROFILE=release for optimized). Profile GPUI with RUSTFLAGS=-C force-frame-pointers=yes and a perf extracted from the Arch package (perf_event_paranoid=2 allows user-space sampling of own processes); dwarf unwinding fails on debug=0 builds.
