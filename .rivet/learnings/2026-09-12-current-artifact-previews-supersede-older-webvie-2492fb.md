---
title: Current artifact previews supersede older webview context caveats
date: 2026-09-12
confidence: high
suggested_doc: webview-security-hardening
related_paths:
  - apps/desktop/src/renderer/src/panes/BrowserPane.tsx
  - apps/desktop/src/renderer/src/components/GuestFrame.tsx
  - apps/desktop/src/main/lib/webviewGuard.ts
  - apps/desktop/src/renderer/src/lib/attachmentUpload.ts
promoted: false
---

# Current artifact previews supersede older webview context caveats

## Observation
Source confirms BrowserPane now embeds GuestFrame, which selects Electron webview versus browser iframe; older renderer-backend-seam context saying browser webviews remain blank is historical. webviewGuard now allows a confined regular-file extension allowlist including HTML/images, despite older webview-security-hardening overview saying all file URLs are denied. browserBus.previewFileAllowed checks markdown using the host and must use returned canonicalPath. Existing attachmentUpload only permits png/jpg/jpeg/gif/webp/pdf up to 24 MiB, so recorded video demos require a new upload/storage contract rather than merely reusing the picker.
