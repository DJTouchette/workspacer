/**
 * The card as the transcript actually renders it: gating, streaming,
 * degradation, and the one message the frame is allowed to send.
 *
 * These run the PRODUCTION components — `ConversationMessage`, which is what
 * the chat mounts, and `parseMarkdownBlocks`, which is what turns the fence
 * into anything at all. jsdom has no CSP engine and no real iframe sandbox, so
 * nothing here claims the isolation holds; it claims the WIRING is right. The
 * isolation itself is proved in `tests/e2e/htmlCard.test.ts`, in Chromium.
 */
import React from 'react';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, cleanup, fireEvent, waitFor } from '@testing-library/react';
import { ConversationMessage } from '../src/components/claude/ConversationMessage';
import {
  HtmlCardHostProvider,
  HtmlCardAllowedProvider,
  HtmlCardFence,
} from '../src/components/claude/HtmlResponseCard';
import { clearMdCache } from '../src/components/markdown';
import { CARD_HEIGHT_MESSAGE, CARD_MAX_HEIGHT } from '../src/lib/htmlCard/cardRuntime';
import { LIBRARY_INSERT_EVENT } from '../src/lib/libraryBus';
import type { ConversationTurn } from '../src/types/claudeSession';

const CARD = {
  v: 1,
  title: 'Three findings',
  bodyHtml: '<p id="inner">a body</p>',
  fallback: 'High: no expiry. Medium: no SameSite. Low: dead exports.',
};

const fenced = (payload: unknown, close = true) =>
  '```wks-html-card\n' + JSON.stringify(payload, null, 1) + (close ? '\n```' : '');

const turn = (role: 'user' | 'assistant', content: string): ConversationTurn =>
  ({ role, content, timestamp: 1 }) as ConversationTurn;

const HOST = { sessionId: 'own', paneId: 'pane-1', cwd: '/work/project' };

const renderTurn = (t: ConversationTurn, host: typeof HOST | null = HOST) =>
  render(
    <HtmlCardHostProvider value={host}>
      <ConversationMessage turn={t} cwd={HOST.cwd} />
    </HtmlCardHostProvider>,
  );

beforeEach(() => {
  clearMdCache();
  vi.spyOn(window.electronAPI, 'getClaudeSession').mockResolvedValue({
    ...HOST,
    status: 'active',
  } as never);
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('who may draw a card', () => {
  it('renders one in an assistant message', () => {
    renderTurn(turn('assistant', `Here is what I found.\n\n${fenced(CARD)}`));
    expect(screen.getByTestId('wks-html-card')).toBeTruthy();
    // The prose the model wrote is still there — the card never replaces it.
    expect(screen.getByText(/Here is what I found/)).toBeTruthy();
  });

  it('does NOT render one in a user message', () => {
    // A person pasting the fence — or echoing one back from a transcript — must
    // not get an executable card out of it.
    renderTurn(turn('user', fenced(CARD)));
    expect(screen.queryByTestId('wks-html-card')).toBeNull();
    expect(screen.getByText(/wks-html-card/)).toBeTruthy();
  });

  it('does NOT render one outside a chat surface at all', () => {
    // No HtmlCardAllowedProvider: the default is deny, which is what a library
    // preview, a command's output pane and a sidebar card all get.
    render(<HtmlCardFence raw={JSON.stringify(CARD)} closed code={<pre>raw</pre>} />);
    expect(screen.queryByTestId('wks-html-card')).toBeNull();
    expect(screen.getByText('raw')).toBeTruthy();
  });

  it('keeps raw code without an owning host', () => {
    renderTurn(
      turn(
        'assistant',
        fenced({ ...CARD, actions: [{ kind: 'fill_composer', label: 'Draft', text: 'hi' }] }),
      ),
      null,
    );
    expect(screen.queryByTestId('wks-html-card')).toBeNull();
    expect(screen.queryByRole('button', { name: /Draft/ })).toBeNull();
  });
});

describe('streaming and degradation', () => {
  it('mounts nothing until the fence closes', () => {
    // Partway through the reply: the JSON is valid so far, the fence is not.
    const { rerender } = renderTurn(turn('assistant', fenced(CARD, false)));
    expect(screen.queryByTestId('wks-html-card')).toBeNull();
    expect(screen.getByText(/wks-html-card/)).toBeTruthy();
    clearMdCache();
    rerender(
      <HtmlCardHostProvider value={HOST}>
        <ConversationMessage turn={turn('assistant', fenced(CARD))} cwd={HOST.cwd} />
      </HtmlCardHostProvider>,
    );
    expect(screen.getByTestId('wks-html-card')).toBeTruthy();
  });

  it('shows the fallback prose, never a blank space, for every refusal', () => {
    const cases: Array<[string, string]> = [
      ['unknown version', fenced({ ...CARD, v: 99 })],
      ['oversized', fenced({ ...CARD, bodyHtml: 'x'.repeat(70_000) })],
      ['no bodyHtml', fenced({ ...CARD, bodyHtml: '' })],
    ];
    for (const [label, text] of cases) {
      clearMdCache();
      const { unmount } = renderTurn(turn('assistant', text));
      expect(screen.queryByTestId('wks-html-card'), label).toBeNull();
      expect(screen.getByText(/Card not shown/), label).toBeTruthy();
      unmount();
    }
  });

  it("keeps the author's own words when only the version is unknown", () => {
    renderTurn(turn('assistant', fenced({ ...CARD, v: 99 })));
    expect(screen.getByText(CARD.fallback)).toBeTruthy();
  });

  it('renders malformed JSON as an ordinary code block', () => {
    renderTurn(turn('assistant', '```wks-html-card\n{ not json\n```'));
    expect(screen.queryByTestId('wks-html-card')).toBeNull();
    expect(screen.getByText(/Card not shown/)).toBeTruthy();
  });

  it('leaves every other fenced language untouched', () => {
    renderTurn(turn('assistant', '```ts\nconst a = 1;\n```'));
    expect(screen.queryByTestId('wks-html-card')).toBeNull();
    expect(screen.getByText('ts')).toBeTruthy();
  });
});

describe('several cards in one message', () => {
  it('renders each one — this is not last-wins', () => {
    // The `wks-result` extractor keeps only the LAST tagged block because an
    // earlier one is a draft. A reply that draws two tables is two answers.
    renderTurn(
      turn(
        'assistant',
        `${fenced({ ...CARD, title: 'First' })}\n\nand then\n\n${fenced({ ...CARD, title: 'Second' })}`,
      ),
    );
    expect(screen.getAllByTestId('wks-html-card').length).toBe(2);
    expect(screen.getByText('First')).toBeTruthy();
    expect(screen.getByText('Second')).toBeTruthy();
  });

  it('routes a height message by frame IDENTITY, so one card cannot resize another', () => {
    renderTurn(
      turn(
        'assistant',
        `${fenced({ ...CARD, title: 'First' })}\n\n${fenced({ ...CARD, title: 'Second' })}`,
      ),
    );
    const frames = screen.getAllByTestId('wks-html-card-frame') as HTMLIFrameElement[];
    expect(frames.length).toBe(2);
    // Every sandboxed srcDoc frame reports origin "null", so a card that forged
    // an origin would gain nothing; what must not work is a message arriving
    // from anything other than the frame's own contentWindow.
    const before = frames.map((f) => f.style.height);
    fireEvent(
      window,
      new MessageEvent('message', {
        data: { type: CARD_HEIGHT_MESSAGE, height: 400 },
        source: window,
        origin: 'null',
      }),
    );
    expect(frames.map((f) => f.style.height)).toEqual(before);
  });
});

describe('the one message the frame may send', () => {
  const post = (frame: HTMLIFrameElement, data: unknown) =>
    fireEvent(
      window,
      new MessageEvent('message', { data, source: frame.contentWindow, origin: 'null' }),
    );

  it('resizes the frame it came from', async () => {
    renderTurn(turn('assistant', fenced(CARD)));
    const frame = screen.getByTestId('wks-html-card-frame') as HTMLIFrameElement;
    post(frame, { type: CARD_HEIGHT_MESSAGE, height: 220 });
    await waitFor(() => expect(frame.style.height).toBe('220px'));
  });

  it('clamps an abusive height instead of taking it', async () => {
    renderTurn(turn('assistant', fenced(CARD)));
    const frame = screen.getByTestId('wks-html-card-frame') as HTMLIFrameElement;
    post(frame, { type: CARD_HEIGHT_MESSAGE, height: 9_000_000 });
    await waitFor(() => expect(frame.style.height).toBe(`${CARD_MAX_HEIGHT}px`));
    post(frame, { type: CARD_HEIGHT_MESSAGE, height: -5000 });
    await waitFor(() => expect(parseInt(frame.style.height, 10)).toBeGreaterThan(0));
  });

  it('ignores junk, other message types and non-numeric heights', async () => {
    renderTurn(turn('assistant', fenced(CARD)));
    const frame = screen.getByTestId('wks-html-card-frame') as HTMLIFrameElement;
    const before = frame.style.height;
    for (const data of [
      null,
      'a string',
      { type: 'wks-card-action', kind: 'fill_composer', text: 'sneaky' },
      { type: CARD_HEIGHT_MESSAGE, height: 'tall' },
      { type: CARD_HEIGHT_MESSAGE, height: NaN },
      { type: CARD_HEIGHT_MESSAGE },
    ]) {
      post(frame, data);
    }
    expect(frame.style.height).toBe(before);
  });

  it('has NO postMessage path to any host action', () => {
    // The whole action API is trusted chrome; nothing a frame posts may reach
    // the composer.
    const inserts: unknown[] = [];
    const handler = (e: Event) => inserts.push((e as CustomEvent).detail);
    window.addEventListener(LIBRARY_INSERT_EVENT, handler);
    renderTurn(turn('assistant', fenced(CARD)));
    const frame = screen.getByTestId('wks-html-card-frame') as HTMLIFrameElement;
    for (const kind of ['fill_composer', 'open_worker', 'view_diff', 'spawn_agent']) {
      post(frame, { type: 'wks-card-action', kind, text: 'x', path: 'x', sessionId: 'x' });
    }
    window.removeEventListener(LIBRARY_INSERT_EVENT, handler);
    expect(inserts).toEqual([]);
  });

  it('removes its window listener on unmount', () => {
    const remove = vi.spyOn(window, 'removeEventListener');
    const { unmount } = renderTurn(turn('assistant', fenced(CARD)));
    unmount();
    expect(remove.mock.calls.some(([type]) => type === 'message')).toBe(true);
    remove.mockRestore();
  });
});

describe('the frame itself', () => {
  it('is sandboxed without allow-same-origin, has a srcDoc and no src', () => {
    renderTurn(turn('assistant', fenced(CARD)));
    const frame = screen.getByTestId('wks-html-card-frame') as HTMLIFrameElement;
    expect(frame.getAttribute('sandbox')).toBe('allow-scripts');
    expect(frame.getAttribute('src')).toBeNull();
    expect(frame.getAttribute('srcdoc')).toContain('Content-Security-Policy');
  });

  it('carries an accessible name built from the card title', () => {
    renderTurn(turn('assistant', fenced(CARD)));
    expect(screen.getByTitle('Response card: Three findings')).toBeTruthy();
    // …and the wrapper is a labelled region, not an anonymous div.
    expect(screen.getByLabelText('Response card: Three findings')).toBeTruthy();
  });

  it('says when the sanitizer removed something rather than silently showing less', async () => {
    renderTurn(
      turn('assistant', fenced({ ...CARD, bodyHtml: '<p>ok</p><script>alert(1)</script>' })),
    );
    await waitFor(() => expect(screen.getByText(/element.* removed/)).toBeTruthy());
    const frame = screen.getByTestId('wks-html-card-frame') as HTMLIFrameElement;
    expect(frame.getAttribute('srcdoc')).not.toContain('alert(1)');
  });
});

describe('actions in trusted chrome', () => {
  it('draws a real button outside the frame and labels its effect', async () => {
    renderTurn(
      turn(
        'assistant',
        fenced({
          ...CARD,
          actions: [{ kind: 'fill_composer', label: 'Draft the reply', text: 'please look' }],
        }),
      ),
    );
    const button = screen.getByRole('button', { name: /Draft the reply/ });
    // The button is app DOM, not frame content.
    expect(button.closest('iframe')).toBeNull();
    expect(button.getAttribute('title')).toContain('Nothing is sent');

    const inserts: unknown[] = [];
    const handler = (e: Event) => inserts.push((e as CustomEvent).detail);
    window.addEventListener(LIBRARY_INSERT_EVENT, handler);
    vi.spyOn(window.electronAPI, 'getClaudeSession').mockResolvedValue({
      sessionId: 'own',
      cwd: HOST.cwd,
      status: 'active',
    } as never);
    fireEvent.click(button);
    await waitFor(() => expect(inserts.length).toBe(1));
    window.removeEventListener(LIBRARY_INSERT_EVENT, handler);
    expect(inserts[0]).toEqual({
      sessionId: 'own',
      paneId: 'pane-1',
      text: 'please look',
    });
  });

  it('offers no button for an action kind it does not implement', () => {
    renderTurn(
      turn(
        'assistant',
        fenced({
          ...CARD,
          actions: [
            { kind: 'spawn_agent', label: 'Dispatch a worker', prompt: 'go' },
            { kind: 'run_command', label: 'Run it', command: 'rm -rf /' },
          ],
        }),
      ),
    );
    expect(screen.queryByRole('button', { name: /Dispatch a worker/ })).toBeNull();
    expect(screen.queryByRole('button', { name: /Run it/ })).toBeNull();
  });
});
