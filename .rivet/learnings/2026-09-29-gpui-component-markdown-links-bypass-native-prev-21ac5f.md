---
title: GPUI component Markdown links bypass native preview requests
date: 2026-09-29
confidence: high
suggested_doc: filelink-openable-files
related_paths:
  - apps/native/src/ui/markdown.rs
  - apps/native/src/ui/transcript.rs
promoted: false
---

# GPUI component Markdown links bypass native preview requests

## Observation
gpui-component 0.5.1 TextView's internal Inline link MouseUp handler calls cx.open_url directly and offers no public link callback. Native supplemental full-path file rows issue FilePreview correctly, but clicking the original Markdown link opens a system URL instead. Native rendering must retain host/session ownership when routing Markdown file links, rather than using the system URL opener.
