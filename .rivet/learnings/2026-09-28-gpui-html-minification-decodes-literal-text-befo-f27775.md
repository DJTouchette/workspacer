---
title: GPUI HTML minification decodes literal text before a second parse
date: 2026-09-28
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/transcript.rs
  - apps/native/src/ui/transcript.rs
promoted: false
---

# GPUI HTML minification decodes literal text before a second parse

## Observation
GPUI Component 0.5.1 text/format/html.rs first calls cleanup_html, whose bundled html5minify writes NodeData::Text contents without re-escaping, then parses the serialized result again. A single HTML escape therefore loses literal angle-bracket tags and can turn escaped tags into image/link nodes. The native CI attachment screenshot exposed missing <tags> despite literal pre rendering. escape_native_html_text now encodes ampersands and angle brackets for both parser passes, in both user/tool literal rendering and sanitized response-card text nodes. The regression exercises tag/entity preservation and rejects image/script nodes after both passes.

## Impact
A normal one-pass HTML escape is insufficient at this pinned renderer boundary; readable text and inert-card guarantees both depend on accounting for the second parse.

## Recommendation
Recheck the native screenshot and the parser boundary regression when upgrading GPUI Component; remove the extra encoding only when its minifier correctly re-escapes text.
