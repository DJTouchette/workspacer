---
title: &lt;webview&gt; in Chromium degrades to a full-size blank block, not an empty inline box
date: 2026-08-24
confidence: high
suggested_doc: renderer-backend-seam
related_paths:
  - apps/desktop/src/renderer/tests/guestFrameFallback.test.tsx
  - apps/desktop/src/renderer/src/panes/BrowserPane.tsx
promoted: false
---

# &lt;webview&gt; in Chromium degrades to a full-size blank block, not an empty inline box

## Observation
Measured in a real Chromium against a scratch hub serving /app: an Electron `<webview>` is an `HTMLUnknownElement` — no shadow root, empty innerHTML, `loadURL`/`canGoBack` undefined, `addEventListener` present and harmless. Its computed `display` is `block`, and because BrowserPane gives it `flex:1; width:100%` inside a flex column it takes a REAL box (measured 1104x802 and 1104x866), so the failure mode is a large blank rectangle under a fully live toolbar — not the zero-size inline element the 2026-08-24 web-completeness audit predicted. Zero page errors: no crash path.

Practical consequence: the pane looks loaded-but-empty and every toolbar button stays clickable and inert, which reads as "the app is broken" rather than "this feature needs the desktop".

## Impact
The visible symptom is what a user reports, and "blank pane with a working toolbar" is a very different bug report from "pane missing". Also means no error boundary or crash telemetry will ever surface it — only a DOM-level test can.

## Recommendation
tests/guestFrameFallback.test.tsx is the regression net; backendParity.test.ts cannot catch this class (it checks electronAPI methods, not the DOM). Any new Electron-only ELEMENT (not method) needs a DOM-level test the same way.
