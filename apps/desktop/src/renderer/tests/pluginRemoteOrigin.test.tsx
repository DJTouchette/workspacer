/**
 * Where a plugin pane's content is loaded FROM, for a browser that is not on the
 * hub's machine.
 *
 * `pluginContentURL` used to answer that with two constants: a sidecar plugin
 * got `http://127.0.0.1:<its port>` and a hub-served `ui` plugin got
 * `http://127.0.0.1:7895`. On the desktop both are true by construction — main
 * spawns the hub and the sidecars on that same machine. From a browser on
 * ANOTHER machine (the whole point of `/app`) both spellings name *the viewer's
 * own loopback*, so a plugin pane either loaded nothing or, worse, framed
 * whatever unrelated service happened to answer on that port on the user's own
 * computer.
 *
 * So this file pins three promises:
 *
 *  1. A hub-served plugin's UI is loaded from the hub THIS CLIENT is talking to
 *     — derived, never a constant.
 *  2. A sidecar plugin whose loopback port this client cannot reach resolves to
 *     NOTHING, and the pane says so, rather than pointing a frame at the
 *     viewer's own 127.0.0.1.
 *  3. "Am I remote?" is answered by an observable fact — is the hub endpoint I
 *     am talking to on my own loopback — and never by `platform === 'web'`,
 *     which is also true of desktop remote-client mode (Electron, real host
 *     platform, remote hub) and false for a desktop pointed at a remote hub.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import React from 'react';
import { render, screen, cleanup, waitFor } from '@testing-library/react';
import { ConfigProvider } from '../src/contexts/ConfigContext';
import { PluginsContext } from '../src/contexts/PluginsContext';
import PluginPane from '../src/panes/PluginPane';
import type { PluginManifest } from '../src/types/plugin';
import { pluginPaneURL, pluginWidgetURL, relocatePluginURL } from '../src/types/plugin';
import {
  isLoopbackOrigin,
  loopbackSibling,
  pluginFrameCandidates,
  resolvePluginFrameOrigin,
  resetPluginFrameOrigin,
} from '../src/lib/pluginOrigin';

/** jsdom serves the document from here. */
const APP_ORIGIN = 'http://localhost:3000';

const uiPlugin = (over: Partial<PluginManifest> = {}): PluginManifest => ({
  id: 'acme.tracker',
  name: 'Tracker',
  apiVersion: '1',
  ui: 'ui',
  panes: [{ type: 'acme.tracker', title: 'Tracker', path: '/' }],
  ...over,
});

const sidecarPlugin = (over: Partial<PluginManifest> = {}): PluginManifest => ({
  id: 'acme.ship',
  name: 'Ship',
  apiVersion: '1',
  server: { command: 'node', args: ['server.js'], port: 9211 },
  panes: [{ type: 'acme.ship', title: 'Ship', path: '/' }],
  widgets: [{ id: 'lamp', title: 'Lamp', path: '/widget/lamp' }],
  ...over,
});

describe('plugin content URLs are derived from the hub this client talks to', () => {
  it('loads a hub-served UI from the stamped hub origin, not a loopback constant', () => {
    const m = uiPlugin({ uiBase: 'https://wks.example.dev' });
    expect(pluginPaneURL(m, m.panes![0])).toBe('https://wks.example.dev/plugins/ui/acme.tracker/');
  });

  it('prefers an explicit frame origin over the stamped one (distinct-origin framing)', () => {
    const m = uiPlugin({ uiBase: 'http://localhost:3000' });
    expect(pluginPaneURL(m, m.panes![0], 'http://127.0.0.1:3000')).toBe(
      'http://127.0.0.1:3000/plugins/ui/acme.tracker/',
    );
  });

  it('resolves a sidecar pane against the base the fetcher stamped', () => {
    const m = sidecarPlugin({ serverBase: 'http://127.0.0.1:9211' });
    expect(pluginPaneURL(m, m.panes![0])).toBe('http://127.0.0.1:9211/');
    expect(pluginWidgetURL(m, m.widgets![0])).toBe('http://127.0.0.1:9211/widget/lamp');
  });

  it('NEVER points an unreachable sidecar at the viewer’s own loopback', () => {
    // No serverBase: whoever fetched the manifest could not reach the sidecar's
    // port from here (the hub is on another machine). The honest answer is
    // "nowhere", not "127.0.0.1 — good luck".
    const m = sidecarPlugin();
    expect(pluginPaneURL(m, m.panes![0])).toBe('');
    expect(pluginWidgetURL(m, m.widgets![0])).toBe('');
  });

  it('resolves nothing for a manifest with neither ui nor server (public projection)', () => {
    const m: PluginManifest = {
      id: 'acme.thin',
      name: 'Thin',
      apiVersion: '1',
      panes: [{ type: 'acme.thin', title: 'Thin' }],
    };
    expect(pluginPaneURL(m, m.panes![0])).toBe('');
  });
});

describe('relocatePluginURL — a pane URL written by another client', () => {
  it('rehomes a hub-served URL from a stale layout onto this client’s hub', () => {
    const m = uiPlugin({ uiBase: 'https://wks.example.dev' });
    // Written by a desktop client, whose hub was its own loopback.
    const stale = 'http://127.0.0.1:7895/plugins/ui/acme.tracker/?sessionId=s1';
    expect(relocatePluginURL(stale, m)).toBe(
      'https://wks.example.dev/plugins/ui/acme.tracker/?sessionId=s1',
    );
  });

  it('rehomes a sidecar URL onto the reachable base, keeping path and query', () => {
    const m = sidecarPlugin({ serverBase: 'http://127.0.0.1:9211' });
    expect(relocatePluginURL('http://127.0.0.1:9999/board?cwd=%2Ftmp', m)).toBe(
      'http://127.0.0.1:9211/board?cwd=%2Ftmp',
    );
  });

  it('empties a sidecar URL this client cannot reach', () => {
    expect(relocatePluginURL('http://127.0.0.1:9211/', sidecarPlugin())).toBe('');
  });

  it('leaves the URL alone when the manifest is unknown (list not loaded yet)', () => {
    const url = 'http://127.0.0.1:7895/plugins/ui/acme.tracker/';
    expect(relocatePluginURL(url, undefined)).toBe(url);
  });
});

describe('choosing the origin plugin content is framed from', () => {
  it('knows the loopback spellings the hub’s own /bus guard accepts', () => {
    expect(isLoopbackOrigin('http://127.0.0.1:7895')).toBe(true);
    expect(isLoopbackOrigin('http://localhost:7895')).toBe(true);
    expect(isLoopbackOrigin('http://[::1]:7895')).toBe(true);
    expect(isLoopbackOrigin('https://wks.example.dev')).toBe(false);
    expect(isLoopbackOrigin('https://127.0.0.1.evil.example')).toBe(false);
  });

  it('pairs the two loopback spellings of one endpoint', () => {
    expect(loopbackSibling('http://127.0.0.1:7895')).toBe('http://localhost:7895');
    expect(loopbackSibling('http://localhost:7895')).toBe('http://127.0.0.1:7895');
    expect(loopbackSibling('https://wks.example.dev')).toBe(null);
  });

  it('offers the advertised origin first, then the loopback sibling', () => {
    expect(pluginFrameCandidates('http://127.0.0.1:7895', 'https://plugins.example')).toEqual([
      'https://plugins.example',
      'http://localhost:7895',
    ]);
    // A remote hub with nothing advertised has no distinct origin to offer.
    expect(pluginFrameCandidates('https://wks.example.dev')).toEqual([]);
    // Never offers the app's own origin back (that is the same-origin fallback).
    expect(pluginFrameCandidates('https://wks.example.dev', 'https://wks.example.dev')).toEqual([]);
    // A non-absolute / non-http advertisement is ignored outright.
    expect(pluginFrameCandidates('https://wks.example.dev', 'javascript:alert(1)')).toEqual([]);
  });
});

describe('resolvePluginFrameOrigin — probes before it commits', () => {
  const originalFetch = global.fetch;
  beforeEach(() => {
    resetPluginFrameOrigin();
    (window.electronAPI as unknown as Record<string, unknown>).platform = 'web';
  });
  afterEach(() => {
    global.fetch = originalFetch;
    resetPluginFrameOrigin();
    (window.electronAPI as unknown as Record<string, unknown>).platform = 'linux';
  });

  it('uses the hub-advertised origin when it answers', async () => {
    global.fetch = vi.fn(async (input: any) => {
      const url = String(input);
      if (url.endsWith('/plugins/origin'))
        return { ok: true, json: async () => ({ origin: 'https://plugins.example' }) } as any;
      if (url.startsWith('https://plugins.example')) return { ok: true } as any;
      throw new Error('unreachable');
    }) as unknown as typeof fetch;

    await expect(resolvePluginFrameOrigin()).resolves.toBe('https://plugins.example');
  });

  it('falls back to the loopback sibling when nothing is advertised', async () => {
    global.fetch = vi.fn(async (input: any) => {
      const url = String(input);
      if (url.endsWith('/plugins/origin')) return { ok: true, json: async () => ({}) } as any;
      if (url.startsWith('http://127.0.0.1:3000')) return { ok: true } as any;
      throw new Error('unreachable');
    }) as unknown as typeof fetch;

    await expect(resolvePluginFrameOrigin()).resolves.toBe('http://127.0.0.1:3000');
  });

  it('falls back to same-origin (no override) when no candidate answers', async () => {
    global.fetch = vi.fn(async () => {
      throw new Error('unreachable');
    }) as unknown as typeof fetch;

    await expect(resolvePluginFrameOrigin()).resolves.toBe('');
  });

  it('never overrides the origin on the desktop, which frames in a <webview>', async () => {
    (window.electronAPI as unknown as Record<string, unknown>).platform = 'darwin';
    global.fetch = vi.fn(async () => ({ ok: true, json: async () => ({}) })) as any;
    await expect(resolvePluginFrameOrigin()).resolves.toBe('');
    expect(global.fetch).not.toHaveBeenCalled();
  });
});

describe('a pane whose plugin UI cannot be reached from here', () => {
  beforeEach(() => {
    expect(location.origin).toBe(APP_ORIGIN);
    (window.electronAPI as unknown as Record<string, unknown>).getConfig = () =>
      Promise.resolve({
        ui: { theme: 'everforest' },
        browser: { homepage: 'https://google.com', bookmarks: [] },
      });
  });
  afterEach(cleanup);

  const mountPane = (url: string, plugins: PluginManifest[]) =>
    render(
      <ConfigProvider>
        <PluginsContext.Provider
          value={{ plugins, panes: [], widgets: [], hotkeys: [], frameOrigin: '' }}
        >
          <PluginPane paneId="p1" title="Ship" isActive url={url} pluginId="acme.ship" />
        </PluginsContext.Provider>
      </ConfigProvider>,
    );

  it('explains itself instead of framing the viewer’s own loopback', async () => {
    mountPane('http://127.0.0.1:9211/', [sidecarPlugin()]);
    await waitFor(() => expect(screen.getByTestId('plugin-unreachable')).toBeInTheDocument());
    expect(document.querySelector('iframe')).toBeNull();
    expect(document.querySelector('webview')).toBeNull();
    expect(screen.getByTestId('plugin-unreachable').textContent).toMatch(/sidecar/i);
  });

  it('still frames a sidecar this client CAN reach', async () => {
    mountPane('http://127.0.0.1:9211/', [sidecarPlugin({ serverBase: 'http://127.0.0.1:9211' })]);
    await waitFor(() => expect(document.querySelector('webview')).not.toBeNull());
    expect(screen.queryByTestId('plugin-unreachable')).toBeNull();
  });
});
