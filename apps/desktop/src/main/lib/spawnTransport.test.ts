/**
 * The transport default, in one place. Before this module the rule lived as
 * `opts.transport ?? cfg.claude.transport ?? 'pty'` copy-pasted at four spawn
 * entry points, and codex had no configured default at all — an absent
 * transport just meant "hybrid", spelled as the ABSENCE of a key. That is what
 * let the same fleet come up GUI-only on one machine and as TUI+viewer pairs on
 * another.
 *
 * TWIN: services/hub/cmd/brain/handlers.go `transportDefault` — a spawn that
 * never touches the desktop must land on the same shape (pinned there by
 * spawn_transport_test.go).
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';

let mockConfig: Record<string, unknown>;
vi.mock('../services/configService', () => ({
  configService: { getConfig: () => mockConfig },
}));

const { resolveTransport, parseTransport, hasTransportChoice, TRANSPORT_FALLBACK } =
  await import('./spawnTransport');

beforeEach(() => {
  mockConfig = {};
});

describe('resolveTransport', () => {
  it('an explicit request always wins, both directions', () => {
    mockConfig = { codex: { transport: 'stream' }, claude: { transport: 'stream' } };
    expect(resolveTransport('codex', 'pty')).toBe('pty');
    expect(resolveTransport('claude', 'pty')).toBe('pty');
    mockConfig = { codex: { transport: 'pty' }, claude: { transport: 'pty' } };
    expect(resolveTransport('codex', 'stream')).toBe('stream');
    expect(resolveTransport('claude', 'stream')).toBe('stream');
  });

  it('falls back to the harness config when nothing was requested', () => {
    mockConfig = { codex: { transport: 'pty' }, claude: { transport: 'stream' } };
    expect(resolveTransport('codex')).toBe('pty');
    expect(resolveTransport('claude')).toBe('stream');
  });

  // The reason this is per-harness at all: codex ships headless, claude's
  // headless transport is opt-in per install. One shared `?? 'pty'` was the bug.
  it('falls back per harness when config says nothing', () => {
    expect(resolveTransport('codex')).toBe('stream');
    expect(resolveTransport('claude')).toBe('pty');
    expect(TRANSPORT_FALLBACK).toEqual({ claude: 'pty', codex: 'stream' });
  });

  it('treats a junk value — in the request or in config — as unstated', () => {
    mockConfig = { codex: { transport: 'sideways' } };
    expect(resolveTransport('codex')).toBe('stream');
    expect(resolveTransport('codex', 'sideways')).toBe('stream');
    expect(resolveTransport('codex', '')).toBe('stream');
  });

  // opencode/pi have exactly one session shape. Inventing a transport for them
  // would put a key on a payload their adapter does not read.
  it.each(['opencode', 'pi'])('%s has no transport choice at all', (provider) => {
    expect(hasTransportChoice(provider)).toBe(false);
    expect(resolveTransport(provider)).toBeUndefined();
    expect(resolveTransport(provider, 'stream')).toBeUndefined();
  });

  it('parseTransport only admits the two real shapes', () => {
    expect(parseTransport('pty')).toBe('pty');
    expect(parseTransport('stream')).toBe('stream');
    for (const junk of ['', 'PTY', 'hybrid', undefined, null, 0, {}]) {
      expect(parseTransport(junk)).toBeUndefined();
    }
  });
});
