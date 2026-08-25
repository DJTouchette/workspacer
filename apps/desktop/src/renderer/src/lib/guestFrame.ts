/**
 * Where an embedded guest page (browser tab, plugin pane, plugin widget) is
 * hosted, and how much of it survives.
 *
 * The desktop embeds guests in an Electron `<webview>`: an out-of-process guest
 * with its own session partition, no access to the embedder's DOM, and a rich
 * API (`loadURL`, `canGoBack`, `insertCSS`, `before-input-event`). `<webview>`
 * is an Electron tag. In a plain browser — `/app`, the hub-served build of this
 * same renderer — it is an unknown element: it parses, it takes a box, it
 * accepts `addEventListener`, and it paints **nothing**. So every `<webview>`
 * pane was a blank rectangle on `/app`. `<iframe>` is the fallback.
 *
 * ── Which host are we on ──────────────────────────────────────────────────
 *
 * There is exactly one seam: `window.electronAPI.platform`. `webBackend` forces
 * it to `'web'`; `bridgedBackend` and `remoteBackend` both restore the real host
 * platform (`api.platform = ipc.platform`). That distinction is load-bearing
 * here: desktop REMOTE-CLIENT mode is Electron running against a web backend, so
 * "talks to a hub over the bus" is NOT the same question as "is a browser". It
 * still has `<webview>`, and must keep getting one.
 *
 * ── Why an iframe's sandbox is not a formality ────────────────────────────
 *
 * A `<webview>` guest can never touch the embedder. An `<iframe>` can, if it is
 * same-origin with the embedder. That is not hypothetical: a webview-only plugin
 * (manifest `ui`, no `server`) is served by the hub at `/plugins/ui/<id>/`, and
 * `/app` is served by the same hub — **same origin**. Framed with
 * `allow-same-origin`, that plugin page could read `parent.document`, call
 * `parent.window.electronAPI.*`, and lift the hub's full host token out of
 * `sessionStorage` — escalating from the deliberately scoped, per-pane bus token
 * `PluginPane` mints to total control of the control plane. So:
 *
 *   • cross-origin guest (sidecar plugin on `127.0.0.1:<port>`, any browsed
 *     site) → `allow-same-origin` is safe; the browser's own same-origin policy
 *     is the wall, and the frame keeps a real origin.
 *   • same-origin-with-`/app` guest (hub-served plugin UI) → NO
 *     `allow-same-origin`. The frame gets an opaque origin and stays walled off.
 *
 * The opaque case costs the guest its bus link, and that is deliberate, not an
 * oversight: the hub's `/bus` same-origin policy (`internal/bus/bus.go`,
 * `originAllowed`) rejects an opaque `Origin: null` by design — it is the
 * DNS-rebinding guard that stops any page the user visits from driving the
 * control plane. Making a same-origin plugin frame work would mean either
 * granting it `allow-same-origin` (escalation above) or teaching `originAllowed`
 * to accept `null` (re-opening that guard). Both widen a security boundary, so
 * neither is done here; `guestFramePolicy().canReachBus` reports the loss and
 * the pane says so out loud instead of pretending.
 */

/** How this build embeds guest pages. */
export type GuestHost = 'webview' | 'iframe';

/**
 * `'webview'` on the desktop (including remote-client mode), `'iframe'` in a
 * browser. Reads the single transport seam rather than sniffing the user agent,
 * so it cannot disagree with which backend was actually installed.
 */
export function guestHost(): GuestHost {
  // `platform` is typed as NodeJS.Platform; webBackend widens it to the sentinel
  // 'web' through the same cast (`'web' as unknown as NodeJS.Platform`), so the
  // comparison needs the matching widen back to string here.
  const platform = typeof window !== 'undefined' ? (window.electronAPI?.platform as string) : '';
  return platform === 'web' ? 'iframe' : 'webview';
}

/** The origin `/app` itself is served from; `''` outside a browser context. */
function appOrigin(): string {
  try {
    return typeof location !== 'undefined' ? location.origin : '';
  } catch {
    return '';
  }
}

/** What an embedded guest can and cannot do, given where it is loaded from. */
export interface GuestFramePolicy {
  /** `sandbox` attribute value; `undefined` means "render a `<webview>`". */
  sandbox?: string;
  /** The guest document shares `/app`'s origin, so it must stay opaque. */
  sameOriginWithApp: boolean;
  /** The guest can open a bus WebSocket the hub will accept. */
  canReachBus: boolean;
}

/**
 * Sandbox tokens for a guest we are content to let behave like a browser tab.
 * `allow-top-navigation` is deliberately absent: a framed page must not be able
 * to navigate the whole app away from itself.
 */
const CROSS_ORIGIN_SANDBOX = [
  'allow-scripts',
  'allow-same-origin',
  'allow-forms',
  'allow-popups',
  'allow-popups-to-escape-sandbox',
  'allow-modals',
  'allow-downloads',
].join(' ');

/** Same set minus `allow-same-origin` — an opaque origin, walled off from `/app`. */
const OPAQUE_SANDBOX = [
  'allow-scripts',
  'allow-forms',
  'allow-popups',
  'allow-popups-to-escape-sandbox',
  'allow-modals',
  'allow-downloads',
].join(' ');

/**
 * Decide how to embed `src`.
 *
 * `origin` defaults to the running document's origin and is a parameter only so
 * the policy is testable without a fake `location`. An unparseable `src` (or one
 * with no origin, e.g. `about:blank`) is treated as same-origin — fail closed:
 * "I could not prove this is cross-origin" must not hand out `allow-same-origin`.
 */
export function guestFramePolicy(
  src: string | undefined,
  host: GuestHost = guestHost(),
  origin: string = appOrigin(),
): GuestFramePolicy {
  if (host === 'webview') {
    return { sandbox: undefined, sameOriginWithApp: false, canReachBus: true };
  }
  let sameOrigin = true;
  try {
    const u = new URL(src ?? '', origin || undefined);
    // `URL.origin` is "null" for opaque schemes (about:, data:, blob: of an
    // opaque origin) — those are same-origin-or-worse, so keep them opaque.
    sameOrigin = u.origin === 'null' || u.origin === origin;
  } catch {
    sameOrigin = true;
  }
  return {
    sandbox: sameOrigin ? OPAQUE_SANDBOX : CROSS_ORIGIN_SANDBOX,
    sameOriginWithApp: sameOrigin,
    // An opaque origin sends `Origin: null`, which the hub's /bus DNS-rebinding
    // guard refuses. A cross-origin loopback frame keeps a real origin the guard
    // accepts, so a sidecar plugin's bus link survives the move to an iframe.
    canReachBus: !sameOrigin,
  };
}
