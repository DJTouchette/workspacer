/**
 * Which ORIGIN a browser should frame hub-served plugin UI from.
 *
 * ── The problem ───────────────────────────────────────────────────────────
 *
 * `/app` is served by the hub, and so is a webview-only plugin's UI
 * (`/plugins/ui/<id>/`). Same host, same port, same scheme: **same origin**. An
 * `<iframe>` of a same-origin document can read `parent.document`, call
 * `parent.window.electronAPI.*` and lift the hub's host token out of
 * `sessionStorage` — escalating from the scoped per-pane bus token `PluginPane`
 * mints to the whole control plane. So `guestFramePolicy` frames a same-origin
 * guest OPAQUE (no `allow-same-origin`), and an opaque document sends
 * `Origin: null`, which the hub's `/bus` DNS-rebinding guard refuses by design.
 * The plugin renders and cannot talk to anything. That is the honest floor, and
 * neither half of it may be loosened to raise it.
 *
 * ── The fix that loosens nothing ──────────────────────────────────────────
 *
 * Give the plugin a DIFFERENT origin for the SAME hub. Then the browser's own
 * same-origin policy is the wall — `allow-same-origin` is safe, the frame keeps
 * a real origin, its storage works, its same-origin fetches work, and its `/bus`
 * socket presents an Origin the hub already accepts (`Origin == Host`). Nothing
 * about the sandbox policy, the origin guard or the token model changes; the
 * guest simply stops being same-origin with the app.
 *
 * Two spellings of "a different origin for the same hub", in preference order:
 *
 *   1. **What the operator declared** — `hub --plugin-origin https://…`, served
 *      at `/plugins/origin`. A second host or port already routed to this hub
 *      (a fly.io service on :8443, `tailscale serve --https=8443`, any reverse
 *      proxy). This is the only one that works for a genuinely REMOTE browser.
 *   2. **The loopback sibling** — `127.0.0.1:P` ⇄ `localhost:P`. Two origins to
 *      a browser, one endpoint to the OS, and both already accepted by the
 *      hub's `isLoopbackHost`. Free, needs no configuration, and helps only
 *      when the browser is on the hub's machine.
 *
 * Neither is assumed: a candidate is PROBED before it is used, because a wrong
 * guess is a plugin pane that loads nothing at all. If none answers, we return
 * `''` — no override, same-origin, opaque, and the pane says so out loud.
 *
 * Desktop is untouched: it frames guests in a `<webview>`, which is never
 * same-origin with the app document, so there is nothing to fix and no probe to
 * pay for.
 */

import { guestHost } from './guestFrame';

/** How long a reachability probe may take before the candidate is dropped. */
const PROBE_TIMEOUT_MS = 2500;
/** How long the hub has to answer with its advertised plugin origin. */
const ADVERTISE_TIMEOUT_MS = 3000;

/**
 * The loopback spellings the hub's own `/bus` origin guard accepts
 * (`bus.go: isLoopbackHost`) — keep the two in agreement, or we would frame a
 * plugin at an origin the bus then refuses.
 */
export function isLoopbackOrigin(origin: string): boolean {
  let host: string;
  try {
    host = new URL(origin).hostname;
  } catch {
    return false;
  }
  // URL.hostname keeps the brackets on an IPv6 literal ("[::1]").
  host = host.replace(/^\[|\]$/g, '');
  if (host.toLowerCase() === 'localhost') return true;
  if (host === '::1' || host === '0:0:0:0:0:0:0:1') return true;
  return /^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(host);
}

/**
 * The OTHER spelling of a loopback origin: `127.0.0.1:P` ⇄ `localhost:P`, and
 * `[::1]:P` → `localhost:P`. Null for anything not on loopback — a remote host
 * has no second name we are entitled to invent.
 */
export function loopbackSibling(origin: string): string | null {
  if (!isLoopbackOrigin(origin)) return null;
  let u: URL;
  try {
    u = new URL(origin);
  } catch {
    return null;
  }
  const port = u.port ? `:${u.port}` : '';
  const host = u.hostname.toLowerCase();
  const sibling = host === 'localhost' ? '127.0.0.1' : 'localhost';
  return `${u.protocol}//${sibling}${port}`;
}

/** An absolute http(s) origin, or `null`. Anything else (`javascript:`, `data:`,
 *  a bare host, a path) is refused: this value becomes an iframe `src` prefix. */
function asOrigin(value: string | undefined): string | null {
  if (!value) return null;
  try {
    const u = new URL(value);
    if (u.protocol !== 'http:' && u.protocol !== 'https:') return null;
    return u.origin;
  } catch {
    return null;
  }
}

/**
 * The distinct origins worth trying for plugin content, best first. Empty means
 * "nothing distinct is available" — frame same-origin and say what it costs.
 */
export function pluginFrameCandidates(appOrigin: string, advertised?: string): string[] {
  const out: string[] = [];
  const declared = asOrigin(advertised);
  if (declared && declared !== appOrigin) out.push(declared);
  const sibling = loopbackSibling(appOrigin);
  if (sibling && sibling !== appOrigin && !out.includes(sibling)) out.push(sibling);
  return out;
}

/** Ask the hub whether the operator declared a second origin for plugin content. */
async function advertisedPluginOrigin(appOrigin: string): Promise<string | undefined> {
  try {
    const res = await fetch(`${appOrigin}/plugins/origin`, {
      cache: 'no-store',
      signal: AbortSignal.timeout(ADVERTISE_TIMEOUT_MS),
    });
    if (!res.ok) return undefined;
    const body = (await res.json()) as { origin?: string };
    return typeof body?.origin === 'string' ? body.origin : undefined;
  } catch {
    return undefined;
  }
}

/**
 * Does this origin actually answer? `no-cors` because we are deliberately
 * cross-origin and want none of the response — only whether the connection
 * happened. (`localhost` resolving to `::1` while the hub binds `127.0.0.1` is
 * the exact case this catches: a sibling that looks right and loads nothing.)
 */
async function reachable(origin: string): Promise<boolean> {
  try {
    await fetch(`${origin}/health`, {
      mode: 'no-cors',
      cache: 'no-store',
      signal: AbortSignal.timeout(PROBE_TIMEOUT_MS),
    });
    return true;
  } catch {
    return false;
  }
}

/** One probe per page load; the answer cannot change without a reload. */
let pending: Promise<string> | null = null;

/**
 * The origin to load hub-served plugin UI from, or `''` for "no override — use
 * the base the manifest was stamped with".
 *
 * `''` is the right answer, not a failure, for the desktop (a `<webview>` guest
 * is never same-origin with the app) and for a browser with no distinct origin
 * available.
 */
export function resolvePluginFrameOrigin(): Promise<string> {
  if (pending) return pending;
  pending = (async () => {
    // A <webview> guest has no same-origin hazard to escape, so it keeps the
    // manifest's own base. This reads the transport seam, not the user agent —
    // desktop REMOTE-CLIENT mode is a web backend inside Electron and must be
    // treated as a desktop here.
    if (guestHost() !== 'iframe') return '';
    if (typeof location === 'undefined' || typeof fetch !== 'function') return '';
    const appOrigin = location.origin;
    if (!appOrigin || appOrigin === 'null') return '';

    const advertised = await advertisedPluginOrigin(appOrigin);
    for (const candidate of pluginFrameCandidates(appOrigin, advertised)) {
      if (await reachable(candidate)) return candidate;
    }
    return '';
  })();
  return pending;
}

/** Tests only: drop the memoized probe. */
export function resetPluginFrameOrigin(): void {
  pending = null;
}
