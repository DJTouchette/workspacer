---
title: webviewGuard's src allowlist is enforced at the Electron main-process level, not in BrowserPane's renderer-side normalizeUrl
date: 2026-07-11
confidence: high
related_paths:
  - apps/desktop/src/main/lib/webviewGuard.ts
  - apps/desktop/src/main/index.ts
  - apps/desktop/src/renderer/src/panes/BrowserPane.tsx
  - apps/desktop/src/renderer/src/panes/PluginPane.tsx
promoted: true
---

# webviewGuard's src allowlist is enforced at the Electron main-process level, not in BrowserPane's renderer-side normalizeUrl

## Observation
apps/desktop/src/renderer/src/panes/BrowserPane.tsx's normalizeUrl() regex `/^(https?|about|file):/i` explicitly passes `file:` URLs through unchanged if typed in the omnibox (e.g. "file:///etc/passwd" survives normalizeUrl and reaches wv.loadURL()). This looks like a hole, but it is not one: apps/desktop/src/main/index.ts's did-attach-webview handler installs will-navigate/will-redirect listeners on every attached guest webContents that re-run webviewGuard.ts's isWebviewSrcAllowed() (http/https/about:blank only) and event.preventDefault() any disallowed navigation — so the actual security boundary for file:// blocking lives entirely in main/index.ts + webviewGuard.ts, never in the renderer's URL parsing. PluginPane.tsx also renders through BrowserPane (appMode=true) so there is only one <webview> implementation for both arbitrary browsing and plugin/hub-UI panes, sharing the same 'persist:browser' partition (and thus cookies/session storage) via the hardcoded partition="persist:browser" prop on the <webview> tag in BrowserPane.tsx.

## Impact
A future editor tightening or refactoring normalizeUrl() might assume it's a security control and could inadvertently loosen it without realizing the real gate is main/index.ts's will-navigate/will-redirect listeners; conversely, editing webviewGuard.ts without knowing BrowserPane/PluginPane share one component and one persistent partition risks missing that a plugin-pane change also affects the general browser pane and vice versa.

## Recommendation
When touching URL/navigation handling in BrowserPane.tsx, PluginPane.tsx, or webviewGuard.ts, always re-check apps/desktop/src/main/index.ts's will-attach-webview / did-attach-webview handlers (search 'will-attach-webview') since that is the actual enforcement point, and remember BrowserPane is the single shared <webview> implementation for both panes on partition 'persist:browser'.

## Disposition
Not folded: already reflected in .rivet/context/modules/webview-security-hardening.md (normalizeUrl-is-UX-only + shared BrowserPane/partition gotchas).
