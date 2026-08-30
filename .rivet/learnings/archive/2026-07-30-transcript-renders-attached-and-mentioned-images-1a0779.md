---
title: Transcript renders attached and mentioned images as thumbnails
date: 2026-07-30
promoted: true
---

# Transcript renders attached and mentioned images as thumbnails

## Observation
The composer's thumbnail machinery moved out of FileChips into components/claude/imagePreviews.ts (module-level path->dataUrl cache + useImagePreviews(paths)), so the composer chips and the new transcript tiles share ONE cache and decode a screenshot once. lib/messageImages.ts is the pure split: extractImageAttachments() strips the composer's '[Image: /path]' markers out of a USER message (returning cleaned text + paths) while deliberately LEAVING [File:]/[PDF:] markers and any Image marker whose path isn't a renderable raster — deleting it would erase the only evidence a file was sent; imagePathsInText() finds absolute image paths an ASSISTANT mentions (capped at 4, punctuation-stripped) and leaves them in the prose, where they're already FileLinks. MessageImages renders a tile only for paths that actually decoded, so a stale transcript degrades to no tile rather than a broken one; click opens a browser pane via fileUrlFromPath, right-click reuses FileActionMenuItems. ConversationMessage gained a cwd prop (ClaudePane passes effectiveCwd, AgentWatchPane the watched cwd) purely to resolve relative image paths.

## Disposition
Folded into .rivet/context/domains/chat-tool-rendering.md (transcript images note).
