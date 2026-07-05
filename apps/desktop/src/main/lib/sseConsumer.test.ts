/**
 * Characterization tests for consumeSseStream — the shared SSE frame parser.
 *
 * These tests pin the EXACT framing behaviour so any accidental divergence
 * from the original copy-pasted loops is caught. Live byte paths
 * (actual fetch calls against claudemon) are NOT tested here.
 */

import { describe, it, expect, vi } from 'vitest';
import { consumeSseStream } from './sseConsumer';

// ── helpers ────────────────────────────────────────────────────────────────────

/**
 * Build a minimal mock fetch that returns the supplied raw bytes as a
 * ReadableStream and then signals EOF. Used to feed canned SSE data into the
 * consumer without a real server.
 */
function makeFetch(chunks: Uint8Array[]) {
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const c of chunks) controller.enqueue(c);
      controller.close();
    },
  });
  return vi.fn().mockResolvedValue({
    ok: true,
    status: 200,
    body: stream,
  } as unknown as Response);
}

/** Encode a string to UTF-8 bytes. */
const enc = (s: string) => new TextEncoder().encode(s);

/**
 * Run the consumer with a mocked fetch and collect all onFrame calls.
 * Aborts after the first backoff cycle (after EOF, we abort immediately).
 */
async function collect(chunks: Uint8Array[], opts: { joinWith?: string } = {}): Promise<string[]> {
  const frames: string[] = [];
  const abort = new AbortController();
  const mockFetch = makeFetch(chunks);

  // Swap in mock fetch globally.
  const origFetch = global.fetch;
  (global as any).fetch = mockFetch;

  try {
    // The consumer reconnects after EOF; abort after first batch.
    let resolved = false;
    const p = consumeSseStream('http://test/stream', {
      signal: abort.signal,
      joinWith: opts.joinWith,
      onFrame(data) {
        frames.push(data);
        // Abort as soon as we have data (prevents infinite reconnect loop in tests).
        if (!resolved) {
          resolved = true;
          abort.abort();
        }
      },
      onError() {
        /* ignore in tests */
      },
    });

    await p;
  } finally {
    (global as any).fetch = origFetch;
  }

  return frames;
}

/**
 * Variant that collects frames but aborts only after the stream hits EOF
 * (for cases where we want all frames, not just the first).
 */
async function collectAll(
  chunks: Uint8Array[],
  opts: { joinWith?: string } = {},
): Promise<string[]> {
  const frames: string[] = [];
  const abort = new AbortController();
  const mockFetch = makeFetch(chunks);

  const origFetch = global.fetch;
  (global as any).fetch = mockFetch;

  try {
    // Abort after a short delay so the consumer can finish reading EOF + backoff.
    // We use a fake timer here to avoid real delays — the consumer calls
    // setTimeout for backoff. We override it so the backoff resolves immediately,
    // then we abort right away.
    const origSetTimeout = global.setTimeout;
    (global as any).setTimeout = (fn: () => void, _ms: number) => {
      abort.abort(); // abort on the first backoff so the loop exits
      return origSetTimeout(fn, 0);
    };

    const p = consumeSseStream('http://test/stream', {
      signal: abort.signal,
      joinWith: opts.joinWith,
      onFrame(data) {
        frames.push(data);
      },
      onError() {
        /* ignore */
      },
    });

    await p;
    (global as any).setTimeout = origSetTimeout;
  } finally {
    (global as any).fetch = origFetch;
  }

  return frames;
}

// ── tests ──────────────────────────────────────────────────────────────────────

describe('consumeSseStream — LF \\n\\n terminator', () => {
  it('delivers a single-line data frame', async () => {
    const raw = enc('data: hello\n\n');
    const frames = await collectAll([raw]);
    expect(frames).toEqual(['hello']);
  });

  it('strips the leading space after "data: " (the replace-leading-space strip)', async () => {
    // "data: foo" — note the space after the colon
    const raw = enc('data: foo\n\n');
    const frames = await collectAll([raw]);
    // The strip removes the single leading space, yielding "foo".
    expect(frames).toEqual(['foo']);
  });

  it('does NOT strip more than one leading space', async () => {
    // "data:  bar" — two spaces; only the first is stripped
    const raw = enc('data:  bar\n\n');
    const frames = await collectAll([raw]);
    expect(frames).toEqual([' bar']);
  });

  it('preserves a frame with no leading space (e.g. "data:foo")', async () => {
    const raw = enc('data:foo\n\n');
    const frames = await collectAll([raw]);
    expect(frames).toEqual(['foo']);
  });

  it('joins multiple data lines with "\\n" (default)', async () => {
    // Multi-line JSON: "data: line1\ndata: line2\n\n"
    const raw = enc('data: line1\ndata: line2\n\n');
    const frames = await collectAll([raw]);
    expect(frames).toEqual(['line1\nline2']);
  });

  it('joins multiple data lines with "" when joinWith="" (base64 use case)', async () => {
    const raw = enc('data: AAE=\ndata: AgM=\n\n');
    const frames = await collectAll([raw], { joinWith: '' });
    expect(frames).toEqual(['AAE=AgM=']);
  });

  it('ignores non-data lines (event:, id:, comment)', async () => {
    const raw = enc('event: update\nid: 1\ndata: payload\n\n');
    const frames = await collectAll([raw]);
    expect(frames).toEqual(['payload']);
  });

  it('skips frames with no data lines', async () => {
    // A frame consisting only of an event line and a comment.
    const raw = enc('event: ping\n: comment\n\ndata: real\n\n');
    const frames = await collectAll([raw]);
    expect(frames).toEqual(['real']);
  });

  it('delivers multiple frames from one read chunk', async () => {
    const raw = enc('data: first\n\ndata: second\n\n');
    const frames = await collectAll([raw]);
    expect(frames).toEqual(['first', 'second']);
  });
});

describe('consumeSseStream — partial frames across reads', () => {
  it('reassembles a frame split across two chunks', async () => {
    const a = enc('data: hel');
    const b = enc('lo\n\n');
    const frames = await collectAll([a, b]);
    expect(frames).toEqual(['hello']);
  });

  it('reassembles when the \\n\\n terminator is split across chunks', async () => {
    // First chunk ends with first \n, second chunk starts with second \n.
    const a = enc('data: world\n');
    const b = enc('\ndata: next\n\n');
    const frames = await collectAll([a, b]);
    expect(frames).toEqual(['world', 'next']);
  });

  it('handles many small single-byte chunks', async () => {
    const raw = 'data: abc\n\n';
    const chunks = Array.from(raw).map((c) => enc(c));
    const frames = await collectAll(chunks);
    expect(frames).toEqual(['abc']);
  });

  it('handles a frame arriving in three pieces', async () => {
    const chunks = [enc('data: '), enc('foo'), enc('\n\n')];
    const frames = await collectAll(chunks);
    expect(frames).toEqual(['foo']);
  });
});

describe('consumeSseStream — CRLF \\r\\n\\r\\n terminator', () => {
  it('delivers a frame terminated by \\r\\n\\r\\n', async () => {
    const raw = enc('data: crlf\r\n\r\n');
    const frames = await collectAll([raw]);
    expect(frames).toEqual(['crlf']);
  });

  it('handles CRLF line endings within the frame', async () => {
    // "data: line1\r\ndata: line2\r\n\r\n" — CRLF throughout
    const raw = enc('data: line1\r\ndata: line2\r\n\r\n');
    const frames = await collectAll([raw]);
    // Each line has its trailing \r stripped before processing.
    expect(frames).toEqual(['line1\nline2']);
  });

  it('strips leading space on CRLF-terminated data lines', async () => {
    const raw = enc('data: spaced\r\n\r\n');
    const frames = await collectAll([raw]);
    expect(frames).toEqual(['spaced']);
  });

  it('picks LF when it comes before CRLF in the buffer', async () => {
    // Buffer has a \n\n before the \r\n\r\n — the LF frame should fire first.
    const raw = enc('data: lf\n\ndata: crlf\r\n\r\n');
    const frames = await collectAll([raw]);
    expect(frames).toEqual(['lf', 'crlf']);
  });

  it('picks CRLF when it comes before LF in the buffer', async () => {
    const raw = enc('data: crlf\r\n\r\ndata: lf\n\n');
    const frames = await collectAll([raw]);
    expect(frames).toEqual(['crlf', 'lf']);
  });
});

describe('consumeSseStream — abort / signal behaviour', () => {
  it('exits immediately when signal is already aborted', async () => {
    const frames: string[] = [];
    const abort = new AbortController();
    abort.abort();

    const origFetch = global.fetch;
    (global as any).fetch = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      body: new ReadableStream({
        start(c) {
          c.enqueue(enc('data: x\n\n'));
          c.close();
        },
      }),
    });

    await consumeSseStream('http://test/', {
      signal: abort.signal,
      onFrame(d) {
        frames.push(d);
      },
    });

    (global as any).fetch = origFetch;
    // fetch should not have been called because signal was already aborted.
    expect(frames).toHaveLength(0);
  });
});

describe('consumeSseStream — leading-space strip exactness', () => {
  it('strips exactly one space from "data: value"', async () => {
    // Original regex: .replace(/^ /, '')
    // This is a non-global replace of a leading single space.
    const raw = enc('data: x\n\n');
    const frames = await collectAll([raw]);
    expect(frames[0]).toBe('x');
  });

  it('does not strip if no space follows the colon ("data:value")', async () => {
    const raw = enc('data:value\n\n');
    const frames = await collectAll([raw]);
    expect(frames[0]).toBe('value');
  });

  it('strips exactly one space even from multi-line frame', async () => {
    const raw = enc('data: a\ndata: b\ndata:c\n\n');
    const frames = await collectAll([raw]);
    // Lines: 'a' (stripped), 'b' (stripped), 'c' (no strip, no space)
    expect(frames[0]).toBe('a\nb\nc');
  });
});
