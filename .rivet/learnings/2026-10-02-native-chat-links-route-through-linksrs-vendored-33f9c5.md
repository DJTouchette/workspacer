---
title: Native chat links route through links.rs; vendored TextView images were client-side loads
date: 2026-10-02
confidence: high
suggested_doc: native-embedded-backend
related_paths:
  - apps/native/src/links.rs
  - apps/native/src/ui/file_viewer.rs
  - vendor/gpui-component/src/text/node.rs
promoted: false
---

# Native chat links route through links.rs; vendored TextView images were client-side loads

## Observation
Upstream gpui-component rendered Markdown/HTML images with img(url): a relative/absolute path loaded from the CLIENT's disk (wrong machine for remote agents) and URLs were fetched unasked; its image click called cx.open_url directly with any scheme, bypassing on_link_click. HTML card TextViews had no on_link_click at all, so <a href> opened any scheme. Native now classifies every chat link in links.rs (http/https -> browser, files -> hub fs.read/fs.readImage on the session's machine, other schemes refused visibly) and node.rs renders images as labels routed through on_link_click when that hook is set.

## Impact
Any new TextView in native must pass Workspace::link_handler or it regains default open_url-any-scheme and client-side image loads.

## Recommendation
Use link_handler(cx) for every native TextView that shows agent-authored content; keep classification in links.rs.
