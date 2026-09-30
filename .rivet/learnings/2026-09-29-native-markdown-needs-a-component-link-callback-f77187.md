---
title: Native Markdown needs a component link callback to preserve selection and parse caching
date: 2026-09-29
confidence: high
suggested_doc: filelink-openable-files
related_paths:
  - apps/native/src/ui/markdown.rs
  - vendor/gpui-component/src/text/*
  - vendor/gpui-component/WORKSPACER-PATCHES.md
promoted: false
---

# Native Markdown needs a component link callback to preserve selection and parse caching

## Observation
Replacing file-linked Markdown paragraphs with GPUI InteractiveText makes links clickable but loses TextView's selection across paragraphs and bypasses its cached async parser. The native crate now pins gpui-component 0.5.1 under vendor/gpui-component with optional TextViewStyle on_link_click and unordered_list_marker fields. The existing inline selection handler dispatches the callback only for clicks; native callbacks check current session ownership/cwd. Callback identity stays outside style equality to prevent reparsing every render.

## Recommendation
Reapply the three documented vendor-file changes explicitly on component upgrades. Preserve selection and cached parsing rather than rendering file-link paragraphs as plain InteractiveText.
