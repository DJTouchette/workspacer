---
title: webviewGuard now allows bounded file: URLs, and both doors are wired by one call
date: 2026-09-03
confidence: high
suggested_doc: webview-security-hardening
related_paths:
  - apps/desktop/src/main/lib/webviewGuard.ts
  - apps/desktop/src/main/lib/webviewRoots.ts
  - apps/desktop/src/main/index.ts
  - apps/desktop/src/renderer/src/panes/BrowserPane.tsx
  - apps/desktop/src/renderer/src/lib/browserBus.ts
promoted: false
---

# webviewGuard now allows bounded file: URLs, and both doors are wired by one call

## Observation
isWebviewSrcAllowed no longer rejects every file: URL. It takes an allowedRoots argument (default []) and there is a new checkWebviewSrc(src, roots) that returns {allowed, reason}. A file: URL is admitted only when: no host (or 'localhost'); pathname decoded ONCE; canonicalized by pathConfinement.canonicalizePath (the per-component symlink walk the fs.* capabilities use, NOT path.resolve); contained by pathConfinement.pathWithinRoots; the target exists and is a regular file; and its extension is in BROWSER_FILE_EXTENSIONS (html htm svg png jpg jpeg gif webp txt json css js pdf). Markdown is refused on purpose with a reason naming the preview pane, because Chromium downloads text/markdown over file: rather than rendering it. index.ts no longer wires will-attach-webview and did-attach-webview by hand: it calls installWebviewGuards(mainWindow.webContents, {allowedRoots: webviewFileRoots, onBlocked}) once, and webviewGuard.test.ts has a source-level test that fails if a second hand-rolled door reappears. The navigation guard also carries a rule the attach guard cannot: a page already on http(s) may never navigate the pane to a file: URL, even an allowed one, and the same check now backs setWindowOpenHandler (allowpopups was previously an unguarded third door). Roots come from webviewRoots.webviewFileRoots(): os.homedir() plus every key of config.projects, read fresh on every check. Refusals are pushed to the renderer over IPC.WEBVIEW_BLOCKED so BrowserPane can show its guestError banner; a prevented ATTACH fires no did-fail-load, which is why the pane used to be silently blank.</observation>
<parameter name="impact">The context doc domains/modules note for webview-security-hardening now describes a policy that no longer holds ("rejects everything else including file:"), and its "no legitimate webview uses file://" claim is false. Anyone tightening or auditing the guard must know the allowance exists and that its containment is delegated to pathConfinement, so the contracts/path-containment-cases.json reasoning applies to it too.

## Recommendation
Update .rivet/context/modules/webview-security-hardening.md: isWebviewSrcAllowed has a second parameter, checkWebviewSrc is the reason-carrying core, installWebviewGuards is the single wiring point, and the allowance is bounded by webviewFileRoots(). Do not add a second call site for either Electron event.
