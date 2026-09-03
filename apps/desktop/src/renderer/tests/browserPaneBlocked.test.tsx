/**
 * A refusal from the main-process webview guard has to be VISIBLE.
 *
 * A prevented `will-attach-webview` destroys the guest before it exists, so the
 * pane never fires `did-fail-load` and BrowserPane's error banner — which is
 * driven entirely by guest events — was unreachable. Every `open_browser` on a
 * `file://` URL was therefore a silent empty rectangle. These cases pin the push
 * channel that replaces the blankness with a reason, and the markdown detour
 * that keeps a `.md` target out of the pane in the first place.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import React from 'react';
import { render, screen, cleanup, waitFor, fireEvent } from '@testing-library/react';
import { ConfigProvider } from '../src/contexts/ConfigContext';
import BrowserPane from '../src/panes/BrowserPane';
import { pathFromFileUrl, markdownPathFromFileUrl, fileUrlFromPath } from '../src/lib/browserBus';
import { MARKDOWN_PREVIEW_EVENT } from '../src/lib/previewBus';
import { readFileSync } from 'fs';
import * as path from 'path';
import { fileURLToPath } from 'url';

type Blocked = { url: string; reason: string; phase: 'attach' | 'navigate' };

/** The main → renderer push, captured so a test can fire it. */
let emitBlocked: ((info: Blocked) => void) | null = null;

const PAGE = '/home/dev/proj/.workspacer/design/index.html';
const PAGE_URL = fileUrlFromPath(PAGE);

const mount = (node: React.ReactElement) => render(<ConfigProvider>{node}</ConfigProvider>);

beforeEach(() => {
  emitBlocked = null;
  const api = window.electronAPI as unknown as Record<string, unknown>;
  api.platform = 'linux';
  api.getConfig = () =>
    Promise.resolve({
      ui: { theme: 'everforest' },
      browser: { homepage: 'https://google.com', bookmarks: [] },
    });
  api.onWebviewBlocked = (cb: (info: Blocked) => void) => {
    emitBlocked = cb;
    return () => {
      emitBlocked = null;
    };
  };
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('BrowserPane — blocked banner', () => {
  it('renders nothing about a block until one arrives', async () => {
    mount(<BrowserPane paneId="p1" title="Browser" isActive initialUrl={PAGE_URL} />);
    await waitFor(() => expect(emitBlocked).toBeTruthy());
    expect(screen.queryByText(/Workspacer blocked this URL/)).toBeNull();
  });

  it('turns a refused ATTACH of its own src into "Blocked: <reason>"', async () => {
    mount(<BrowserPane paneId="p1" title="Browser" isActive initialUrl={PAGE_URL} />);
    await waitFor(() => expect(emitBlocked).toBeTruthy());

    emitBlocked!({
      url: PAGE_URL,
      reason: 'this file is outside your home and project directories',
      phase: 'attach',
    });

    await screen.findByText('Workspacer blocked this URL.');
    expect(
      screen.getByText('Blocked: this file is outside your home and project directories'),
    ).toBeTruthy();
    // Retrying loads the same URL into the same guard, so it is not offered.
    expect(screen.queryByText('Retry')).toBeNull();
  });

  it('ignores a refusal aimed at some OTHER pane', async () => {
    mount(<BrowserPane paneId="p1" title="Browser" isActive initialUrl={PAGE_URL} />);
    await waitFor(() => expect(emitBlocked).toBeTruthy());

    emitBlocked!({ url: 'file:///etc/passwd', reason: 'no such file', phase: 'attach' });

    await Promise.resolve();
    expect(screen.queryByText(/Workspacer blocked this URL/)).toBeNull();
  });

  it('offers the preview pane when the refused target was markdown', async () => {
    const mdUrl = fileUrlFromPath('/home/dev/proj/DESIGN.md');
    mount(<BrowserPane paneId="p1" title="Browser" isActive initialUrl={mdUrl} />);
    await waitFor(() => expect(emitBlocked).toBeTruthy());

    emitBlocked!({
      url: mdUrl,
      reason: 'markdown files open in the preview pane, not the browser',
      phase: 'navigate',
    });

    const button = await screen.findByText('Open markdown preview');
    const seen: string[] = [];
    const handler = (e: Event) => seen.push((e as CustomEvent).detail.path);
    window.addEventListener(MARKDOWN_PREVIEW_EVENT, handler);
    fireEvent.click(button);
    window.removeEventListener(MARKDOWN_PREVIEW_EVENT, handler);

    expect(seen).toEqual(['/home/dev/proj/DESIGN.md']);
  });

  it('offers no preview button for a non-markdown refusal', async () => {
    mount(<BrowserPane paneId="p1" title="Browser" isActive initialUrl={PAGE_URL} />);
    await waitFor(() => expect(emitBlocked).toBeTruthy());

    emitBlocked!({ url: PAGE_URL, reason: 'no such file', phase: 'attach' });

    await screen.findByText('Workspacer blocked this URL.');
    expect(screen.queryByText('Open markdown preview')).toBeNull();
  });
});

/**
 * The markdown detour has to sit at the DISPATCH points, not inside the pane:
 * Chromium downloads `text/markdown` over file:, so a .md target must never
 * become a browser tab in the first place. App.tsx has exactly two places a
 * file: open is dispatched, and both have to take it — a source check, because
 * rendering App in jsdom is not a thing this suite can afford.
 */
describe('App.tsx routes a .md file: target to the preview pane', () => {
  const src = readFileSync(
    path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'src', 'App.tsx'),
    'utf8',
  );

  /** The window-event path: FileLink's "Open in browser" and friends. */
  it('detours the browser:open-url handler', () => {
    const start = src.indexOf('const t = (e as CustomEvent).detail as BrowserOpenTarget');
    const end = src.indexOf('window.addEventListener(BROWSER_OPEN_EVENT', start);
    expect(start).toBeGreaterThan(-1);
    const body = src.slice(start, end);
    expect(body).toMatch(/markdownPathFromFileUrl/);
    expect(body.indexOf('markdownPathFromFileUrl')).toBeLessThan(body.indexOf('addTab('));
  });

  /** The hub-bus path: an agent's open_browser → command.open_pane. */
  it('detours the openPane(browser, {url}) handler', () => {
    const start = src.indexOf("if (paneType === 'browser' && opts?.url)");
    expect(start).toBeGreaterThan(-1);
    const body = src.slice(start, src.indexOf("if (paneType === 'guide')", start));
    expect(body).toMatch(/markdownPathFromFileUrl/);
    expect(body.indexOf('markdownPathFromFileUrl')).toBeLessThan(body.indexOf('addTabWithConfig('));
  });

  it('has no third, undetoured file: dispatch point', () => {
    expect(src.match(/markdownPathFromFileUrl\(/g)).toHaveLength(2);
  });
});

describe('pathFromFileUrl / markdownPathFromFileUrl', () => {
  it('round-trips a path with spaces through fileUrlFromPath', () => {
    const p = '/home/dev/my notes/a report.html';
    expect(pathFromFileUrl(fileUrlFromPath(p))).toBe(p);
  });

  it('accepts the localhost spelling and rejects a remote host', () => {
    expect(pathFromFileUrl('file://localhost/home/dev/x.html')).toBe('/home/dev/x.html');
    expect(pathFromFileUrl('file://evil/share/x.html')).toBeNull();
  });

  it('returns null for anything that is not a local file URL', () => {
    for (const u of ['https://example.com/a.md', 'about:blank', 'not a url', '']) {
      expect(pathFromFileUrl(u), u).toBeNull();
    }
  });

  it('returns null on a malformed percent-escape', () => {
    expect(pathFromFileUrl('file:///home/%zz/x.html')).toBeNull();
  });

  it('names a markdown target and only a markdown target', () => {
    expect(markdownPathFromFileUrl('file:///home/dev/DESIGN.md')).toBe('/home/dev/DESIGN.md');
    expect(markdownPathFromFileUrl('file:///home/dev/notes.MARKDOWN')).toBe(
      '/home/dev/notes.MARKDOWN',
    );
    expect(markdownPathFromFileUrl('file:///home/dev/index.html')).toBeNull();
    expect(markdownPathFromFileUrl('https://example.com/README.md')).toBeNull();
  });

  it('strips the leading slash from a Windows drive path', () => {
    expect(pathFromFileUrl('file:///C:/work/site/index.html')).toBe('C:/work/site/index.html');
  });
});
