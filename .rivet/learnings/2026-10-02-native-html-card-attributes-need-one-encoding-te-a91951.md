---
title: Native HTML card attributes need one encoding; text needs two
date: 2026-10-02
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/transcript.rs
  - vendor/gpui-component/src/text/format/html5minify/mod.rs
promoted: false
---

# Native HTML card attributes need one encoding; text needs two

## Observation
gpui-component's html5minify writes text nodes decoded but re-escapes attribute values (write_attribute_value with reserved_entity), so native_card_html double-encodes text (escape_native_html_text) but single-encodes the href/src it now keeps. A quoted/ampersand href survives to on_link_click intact (verified by html_card_links_route_from_raw_fence_through_sanitizer). The sanitizer classifies destinations with links::classify('/', raw) only to decide keep/drop; the raw relative value is emitted so the click resolves against the session cwd.

## Recommendation
Keep only Web/File a href and File-image img src; never emit other attributes. Any new card consumer must render with on_link_click, or the vendor TextView will open/load these destinations itself.
