---
title: Webview attach-time hardening (BrowserPane + PluginPane)
tags: [security, electron, webview, browser-pane, plugin-pane, main-process, origin, blocked, panes, policy, sidecar]
related_paths:
  - "apps/desktop/src/main/lib/webviewGuard.ts"
  - "apps/desktop/src/main/lib/webviewGuard.test.ts"
  - "apps/desktop/src/main/lib/webviewRoots.ts"
  - "apps/desktop/src/renderer/src/lib/guestFrame.ts"
  - "apps/desktop/src/main/index.ts"
  - "apps/desktop/src/renderer/src/panes/BrowserPane.tsx"
  - "apps/desktop/src/renderer/src/panes/PluginPane.tsx"
  - "apps/desktop/src/main/services/chromeCookieImport.ts"
owner: Damien Touchette
last_reviewed: 2026-09-26
---

# Browser and plugin guest boundaries

## Shared host implementation

Electron uses `BrowserPane` for ordinary browsing and for the guest inside
`PluginPane`. Main-process `installWebviewGuards` in
`apps/desktop/src/main/lib/webviewGuard.ts` owns both attach and navigation
checks. `index.ts` supplies the same `webviewFileRoots` function to both and
publishes blocked verdicts on `IPC.WEBVIEW_BLOCKED`, allowing the pane to show
an explanation instead of remaining silently blank.

Every attach strips preload/preloadURL, disables Node integration in top-level
and subframes, enables context isolation and web security, and disables
`allowFileAccessFromFileUrls`. The guard does not change the `sandbox` preference.
The renderer's URL normalization is convenience behavior, not this boundary.

## Source URLs and local files

Empty source, exact `about:blank`, HTTP and HTTPS are allowed. Other schemes
are refused, except for the explicitly checked local-file path below. In
particular, `about:blank#fragment` is not the exact empty-shell exemption.
It is no longer correct to say every `file:` URL is blocked.

`checkWebviewSrc` checks local files using these conditions:

1. Local authority only: empty host or localhost, never a remote file host.
2. One percent decode, followed by per-component filesystem canonicalization.
3. Containment inside an allowed browser root, then `isSecretPath` rejection.
4. Extension appropriate to the selected pane and an existing regular file.

`webviewRoots.ts` supplies home and registered project directories, rejecting
relative/unresolvable and volume-root entries. This **browser rendering**
policy remains enforced even though ordinary authenticated agent filesystem
calls have ambient host-path access. Do not remove it as a retired plugin grant.

`BROWSER_FILE_EXTENSIONS` contains HTML/HTM, SVG, common raster images, text,
CSS, and JS. JSON, PDF, directories, and extensionless files are not admitted.
Markdown is returned as a preview detour; `checkPreviewFileUrl` applies the same
file checks with the Markdown extension set. The preview flow carries the
canonical checked path into its subsequent read.

The attach path rewrites an allowed file source to its canonical URL while
preserving query and fragment. Navigation checks cannot atomically bind an
already-started Chromium load to that filesystem object, so they are not a
claim of race-free filesystem access. Subframe/resource loads inside an allowed
page are not all individually checked by the main-frame navigation guard.

## Navigation and popup ownership

`will-navigate` and `will-redirect` cancel refused navigation. Programmatic
`loadURL` also reaches `did-start-navigation`; for a refused main-frame load,
the guard stops the guest and loads `about:blank`. Do not rely on
`will-navigate` alone to cover address-bar actions.

Even an allowed local-file target requires a local-file source page, except for
the guest's first attach-approved load. A remote page or an empty/about:blank
shell cannot navigate into local files merely because the destination passes
path checks. Attach exemptions are queued FIFO across interleaved guest
attachments and consumed on the first load, not retained as future authority.

`setWindowOpenHandler` checks local-file popups, and `did-create-window` installs
the navigation guard on the new BrowserWindow. Without that second step a popup
would bypass the main window's `did-attach-webview` path. Blocked reports carry
a guest webContents ID when available so identical URLs in multiple panes do
not make one pane claim another pane's error.

## Plugin identity and browser fallback

A pane with a known plugin ID can mint its own revocable identity token; cwd is
optional routing context, not a narrower filesystem grant. `PluginPane` first
relocates the saved URL against the currently reachable manifest/frame origin.
A shared-layout URL may have had its token redacted, so minting is needed even
for a global pane. Mint failure falls back to the available URL, which may be
unauthenticated; a four-second renderer deadline prevents indefinite blankness.
Late tokens are revoked, as are owned tokens on unmount.

In a plain browser, `guestHost()` selects an iframe. Electron remote-client
mode still has webviews because its backend retains the native platform marker.
`guestFramePolicy` selects iframe sandbox tokens by origin:

- Cross-origin guests keep `allow-same-origin`; the browser's origin boundary
  separates their document from the host.
- Same-origin, opaque, or unparseable guests omit `allow-same-origin`. Their
  opaque Origin cannot pass the hub's WebSocket origin check, so the pane reports
  that its bus connection is unavailable.

A distinct configured plugin origin or the local loopback sibling spelling lets
hub-served UI retain an origin without sharing the host document. Do not solve
a bus failure by granting a same-origin plugin access to the host document or
by accepting opaque origins at the hub. A remote client also cannot reach a
server machine's loopback-only sidecar by opening its own localhost; URL
relocation and reachability checks must run before framing.

Plugin enablement trusts local extension code. Guest document isolation,
provider identity, revocation, and host-only administration remain distinct
boundaries; iframe sandboxing is not an OS sandbox for sidecar/install code.

## Theme, settings, and partition behavior

Electron guests use the `persist:browser` partition, also used by Chrome-cookie
import. Theme/settings injection runs on the plugin/app-mode path and retries
on a later document-ready event after a transient navigation failure. Theme
CSS values have their own interpolation filter; see
[theme system](theme-system.md#guest-document-token-validation).

## Validation

From `apps/desktop`, run:

```bash
npx vitest run src/main/lib/webviewGuard.test.ts src/main/lib/webviewRoots.test.ts
```

From `apps/desktop/src/renderer`, run:

```bash
npx vitest run tests/guestFrameFallback.test.tsx
```

These exercise policy and renderer fallback with controlled fixtures. Real
Electron navigation/popups, cookie import, and cross-origin browser integration
still require the corresponding platform/browser checks when those behaviors
change; mocked policy tests alone cannot establish Chromium behavior.
