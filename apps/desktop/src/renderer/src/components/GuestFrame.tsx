/**
 * The one place that decides `<webview>` vs `<iframe>`.
 *
 * Three call sites embed a guest page — `BrowserPane` (arbitrary browsing AND,
 * via `PluginPane`, every plugin pane) and `WidgetBoard`'s `PluginWidgetView`.
 * All three rendered a bare Electron `<webview>`, which paints nothing in a
 * browser, so every one of them was a blank rectangle on `/app`. Rather than
 * fork each component, they all render this and keep their existing ref-driven
 * logic: the ref still receives the real element, so a `<webview>`'s listeners
 * and methods are reached exactly as before, and on an `<iframe>` those
 * `addEventListener` calls are simply never fired (the callers already guard
 * every Electron-only method behind a `typeof` check).
 *
 * See `lib/guestFrame.ts` for why the iframe's `sandbox` is chosen per-URL and
 * what it deliberately costs a same-origin guest.
 */
import React, { forwardRef } from 'react';
import { guestHost, guestFramePolicy } from '../lib/guestFrame';

interface GuestFrameProps {
  /** The page to load. */
  src: string;
  /** Accessible name; also the iframe's `title`. */
  title: string;
  style?: React.CSSProperties;
  /** Electron only — the guest's persistent session partition. */
  partition?: string;
  /** Electron only — let the guest open new windows. */
  allowPopups?: boolean;
}

/**
 * `ref` receives the underlying element: an Electron `<webview>` (with its
 * `loadURL`/`canGoBack`/`insertCSS` surface) on the desktop, a plain
 * `HTMLIFrameElement` in a browser. Typed as `HTMLElement` because the Electron
 * namespace isn't in the renderer's tsconfig — the same compromise
 * `usePluginWebview` already makes.
 */
const GuestFrame = forwardRef<HTMLElement, GuestFrameProps>(
  ({ src, title, style, partition, allowPopups }, ref) => {
    const host = guestHost();
    if (host === 'webview') {
      return (
        <webview
          ref={ref as never}
          src={src}
          style={style}
          // @ts-ignore — Electron <webview> attributes aren't in lib.dom
          partition={partition}
          // @ts-ignore
          allowpopups={allowPopups ? 'true' : undefined}
        />
      );
    }
    const policy = guestFramePolicy(src, host);
    return (
      <iframe
        ref={ref as React.Ref<HTMLIFrameElement>}
        src={src}
        title={title}
        style={style}
        sandbox={policy.sandbox}
        // Plugin UIs and dev servers are local; don't leak the app's full URL
        // (which carries the hub token in the query) to a browsed site.
        referrerPolicy="no-referrer"
      />
    );
  },
);
GuestFrame.displayName = 'GuestFrame';

export default GuestFrame;
