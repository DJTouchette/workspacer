---
title: TextView keyed state survives only while constructed each frame; heading anchors via TextViewHandle
date: 2026-10-02
confidence: medium
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/ui/file_viewer.rs
  - vendor/gpui-component/src/text/text_view.rs
promoted: false
---

# TextView keyed state survives only while constructed each frame; heading anchors via TextViewHandle

## Observation
gpui-component TextView::markdown keeps parse + ListState in window.use_keyed_state; if a frame does not construct it (e.g. a Source tab is shown instead) the state is released and the next Preview re-parses synchronously and resets scroll. Constructing the TextView every frame (not drawing it) retains it. Its first parse runs synchronously in request_layout on the UI thread (later updates are debounced in the background), so native caps rendered Markdown at 256 KiB. The vendored TextViewHandle exposes headings/block kinds/scroll for app keyboard scrolling and #heading anchors; in scrollable mode root children are list items, and scroll_to past content is clamped at layout, so short documents cannot put a late heading at the top.

## Recommendation
Keep constructing document TextViews while hidden if their state matters; test heading scroll with documents taller than the viewport.
