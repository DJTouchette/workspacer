---
title: Headless companion context must distinguish live Electron dispatch from retired Node transport
date: 2026-09-29
confidence: high
suggested_doc: headless-desktop-services
related_paths:
  - .rivet/context/modules/headless-desktop-services.md
promoted: false
---

# Headless companion context must distinguish live Electron dispatch from retired Node transport

## Observation
The headless-desktop-services context still presented deleted build:desktop-host/test:desktop-host and stdio callback transport as current. The shipped default Rust backend owns those services now, while Electron still imports the public headless/desktopHost dispatcher and shared helpers. Added a current-ownership preface and marked old sections as historical reference so future audits do not invoke removed scripts or delete live Electron code based on the directory name.
