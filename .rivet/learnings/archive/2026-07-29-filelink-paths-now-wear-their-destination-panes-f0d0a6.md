---
title: FileLink paths now wear their destination pane's icon
date: 2026-07-29
promoted: true
---

# FileLink paths now wear their destination pane's icon

## Observation
defaultOpenTarget(path) is the single source for what a click opens; openFileDefault dispatches from it and FileLink renders PaneIcon for the same target, plus data-open-target for tests. Replaced the M-down/circle-plus typographic marks, so .html now shows the editor icon (openFileDefault never special-cased html — browser is right-click only). Two layout gotchas found by screenshotting real Chromium: the link root needs display:inline-block/max-width:100%/overflow-wrap:anywhere or a prose line break strands the icon from its path, and WorkCard's file row had to move from alignItems:baseline to center because an SVG's baseline is its bottom edge. DESIGN_LANGUAGE.md lost its 'FileLink badges are typographic' exception.

## Disposition
Not folded: already reflected in .rivet/context/modules/filelink-openable-files.md (hand-authored notes 2026-07-29, verbatim).
