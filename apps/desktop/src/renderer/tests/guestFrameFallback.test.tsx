/**
 * `<webview>` is an Electron tag. On `/app` — the hub-served build of this very
 * renderer, running in a plain browser — it is an unknown element: it parses, it
 * takes a full-size box, it accepts `addEventListener`, and it paints nothing.
 * So the Browser pane, EVERY plugin pane (they all go through BrowserPane) and
 * every plugin widget were blank rectangles there. This file is the regression
 * net for the `<iframe>` fallback, and it pins three separate promises:
 *
 *  1. A browser gets an `<iframe>` that actually loads the URL, and the desktop
 *     still gets a `<webview>` — including desktop REMOTE-CLIENT mode, which is
 *     Electron talking to a remote hub over the web backend. "Has a bus backend"
 *     is not the same question as "is a browser", and getting that wrong breaks
 *     the desktop.
 *
 *  2. The sandbox is chosen by origin, not by convenience. A hub-served plugin
 *     (`/plugins/ui/<id>/`) is SAME-ORIGIN with `/app`; framing it with
 *     `allow-same-origin` would let a plugin page read `parent.document` and
 *     lift the hub's full host token, escalating past the deliberately scoped
 *     per-pane token PluginPane mints. That must stay impossible.
 *
 *  3. A control an iframe cannot drive is disabled with a reason, not left live
 *     and inert — the silent-no-op failure mode this whole fallback exists to
 *     remove.
 */
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import React from 'react';
import { render, screen, cleanup, waitFor, fireEvent } from '@testing-library/react';
import { ConfigProvider } from '../src/contexts/ConfigContext';
import { guestFramePolicy, guestHost } from '../src/lib/guestFrame';
import BrowserPane from '../src/panes/BrowserPane';
import PluginPane from '../src/panes/PluginPane';

/** jsdom serves the document from here, so this is what "same origin" means. */
const APP_ORIGIN = 'http://localhost:3000';
/** A webview-only plugin: the hub serves its UI from the app's own origin. */
const HUB_SERVED_PLUGIN = `${APP_ORIGIN}/plugins/ui/acme.tracker/?busToken=t`;
/** A sidecar plugin: its own loopback port, a different origin from /app. */
const SIDECAR_PLUGIN = 'http://127.0.0.1:9211/?busToken=t';

function setPlatform(platform: string) {
  (window.electronAPI as unknown as Record<string, unknown>).platform = platform;
}

const mount = (node: React.ReactElement) => render(<ConfigProvider>{node}</ConfigProvider>);

/** The single guest element in the pane, whatever tag it turned out to be. */
function guestElement(): Element {
  const el = document.querySelector('iframe') ?? document.querySelector('webview');
  if (!el) throw new Error('the pane rendered no guest element at all');
  return el;
}

beforeEach(() => {
  expect(location.origin).toBe(APP_ORIGIN); // the premise every origin test rests on
  // The shared stub answers getConfig with `{}`, which useTheme reads as
  // `config.ui.theme` and throws on. Give it the shape the app boots with.
  (window.electronAPI as unknown as Record<string, unknown>).getConfig = () =>
    Promise.resolve({
      ui: { theme: 'everforest' },
      browser: { homepage: 'https://google.com', bookmarks: [] },
    });
});
afterEach(() => {
  cleanup();
  setPlatform('linux');
});

describe('guestFramePolicy', () => {
  it('leaves the desktop on a <webview> with no sandbox', () => {
    const p = guestFramePolicy(SIDECAR_PLUGIN, 'webview', APP_ORIGIN);
    expect(p.sandbox).toBeUndefined();
    expect(p.canReachBus).toBe(true);
  });

  it('grants allow-same-origin only to a CROSS-origin guest', () => {
    const p = guestFramePolicy(SIDECAR_PLUGIN, 'iframe', APP_ORIGIN);
    expect(p.sandbox).toContain('allow-same-origin');
    expect(p.sameOriginWithApp).toBe(false);
    // A real loopback origin is one the hub's /bus same-origin guard accepts.
    expect(p.canReachBus).toBe(true);
  });

  it('withholds allow-same-origin from a guest served by /app itself', () => {
    const p = guestFramePolicy(HUB_SERVED_PLUGIN, 'iframe', APP_ORIGIN);
    expect(p.sandbox).toBeDefined();
    expect(p.sandbox).not.toContain('allow-same-origin');
    expect(p.sameOriginWithApp).toBe(true);
    // An opaque origin sends `Origin: null`, which the hub refuses by design.
    expect(p.canReachBus).toBe(false);
  });

  it('never grants top-navigation — a framed page must not steer the whole app', () => {
    expect(guestFramePolicy(SIDECAR_PLUGIN, 'iframe', APP_ORIGIN).sandbox).not.toContain(
      'allow-top-navigation',
    );
  });

  it('fails closed on a src it cannot resolve to an origin', () => {
    for (const src of ['about:blank', 'data:text/html,x', '', undefined]) {
      const p = guestFramePolicy(src, 'iframe', APP_ORIGIN);
      expect(p.sandbox, String(src)).not.toContain('allow-same-origin');
    }
  });
});

describe('guestHost', () => {
  it('is an iframe only when the WEB backend is installed', () => {
    setPlatform('web');
    expect(guestHost()).toBe('iframe');
  });

  it('stays a webview in desktop remote-client mode', () => {
    // remoteBackend/bridgedBackend both restore the real host platform
    // (`api.platform = ipc.platform`) precisely so this stays distinguishable
    // from a browser. Only webBackend forces 'web'.
    setPlatform('darwin');
    expect(guestHost()).toBe('webview');
  });
});

describe('BrowserPane', () => {
  it('renders an <iframe> that loads the page on /app', async () => {
    setPlatform('web');
    mount(<BrowserPane paneId="p1" title="Docs" isActive initialUrl="https://example.com/docs" />);
    await waitFor(() => {
      const frame = document.querySelector('iframe');
      expect(frame).toBeTruthy();
      expect(frame!.getAttribute('src')).toBe('https://example.com/docs');
    });
    expect(document.querySelector('webview')).toBeNull();
  });

  it('still renders a <webview> on the desktop', async () => {
    setPlatform('linux');
    mount(<BrowserPane paneId="p1" title="Docs" isActive initialUrl="https://example.com/docs" />);
    await waitFor(() => {
      const wv = document.querySelector('webview');
      expect(wv).toBeTruthy();
      expect(wv!.getAttribute('src')).toBe('https://example.com/docs');
      // The hardening the main-process webview guard is written against.
      expect(wv!.getAttribute('partition')).toBe('persist:browser');
    });
    expect(document.querySelector('iframe')).toBeNull();
  });

  it('reloads by remounting the frame, and the new frame still clears the spinner', async () => {
    // Refresh on a cross-origin iframe can only work by remounting (there is no
    // reload() to reach). That swaps the element out from under the listener
    // effect, so without a re-attach the progress bar would never clear again.
    setPlatform('web');
    const { container } = mount(
      <BrowserPane paneId="p1" title="Docs" isActive initialUrl="https://example.com" />,
    );
    await waitFor(() => expect(container.querySelector('iframe')).toBeTruthy());
    const before = container.querySelector('iframe')!;

    fireEvent.click(screen.getByTitle('Refresh'));
    await waitFor(() => expect(container.querySelector('.wks-browser-progress')).toBeTruthy());
    const after = container.querySelector('iframe')!;
    expect(after).not.toBe(before);

    fireEvent.load(after);
    await waitFor(() => expect(container.querySelector('.wks-browser-progress')).toBeNull());
  });

  it('disables the controls an iframe cannot drive instead of leaving them inert', async () => {
    setPlatform('web');
    mount(<BrowserPane paneId="p1" title="Docs" isActive initialUrl="https://example.com" />);
    await waitFor(() => expect(document.querySelector('iframe')).toBeTruthy());
    for (const label of [
      /^Page history needs the desktop app/,
      /^Cookie sync needs the desktop app/,
    ]) {
      const buttons = screen.getAllByTitle(label);
      expect(buttons.length).toBeGreaterThan(0);
      for (const b of buttons) expect(b).toBeDisabled();
    }
  });
});

describe('PluginPane on /app', () => {
  it('frames a SIDECAR plugin cross-origin, so it keeps its bus link', async () => {
    setPlatform('web');
    mount(<PluginPane paneId="p1" title="Shiplight" isActive url={SIDECAR_PLUGIN} pluginId="x" />);
    await waitFor(() => expect(document.querySelector('iframe')).toBeTruthy());
    const frame = guestElement();
    expect(frame.getAttribute('src')).toBe(SIDECAR_PLUGIN);
    expect(frame.getAttribute('sandbox')).toContain('allow-same-origin');
    // Nothing is lost, so nothing is claimed to be.
    expect(screen.queryByText(/cannot open its hub connection/i)).toBeNull();
  });

  it('frames a HUB-SERVED plugin opaque, and says what that costs', async () => {
    setPlatform('web');
    mount(
      <PluginPane paneId="p1" title="Fleet Radar" isActive url={HUB_SERVED_PLUGIN} pluginId="x" />,
    );
    await waitFor(() => expect(document.querySelector('iframe')).toBeTruthy());
    const frame = guestElement();
    // The escalation this prevents: same-origin with /app means parent.document
    // and the hub's full host token are one property access away.
    expect(frame.getAttribute('sandbox')).not.toContain('allow-same-origin');
    expect(screen.getByText(/cannot open its hub connection/i)).toBeInTheDocument();
  });
});
