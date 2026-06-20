/**
 * Shared SSE stream consumer.
 *
 * Encapsulates the fetch → getReader → TextDecoder → buffer-accumulation →
 * frame-split → data-line-extraction loop that is common to all three SSE
 * consumers in the codebase (claudemonHookBridge, claudemonSessionClient,
 * terminalShare).
 *
 * IMPORTANT: The byte path is identical to the original copy-pasted loops —
 * do NOT change the framing, the data-line extraction, or the join logic
 * without also updating the callers.
 */

export interface ConsumeSseStreamOptions {
  /** AbortSignal — when aborted the loop exits on the next read iteration. */
  signal: AbortSignal;
  /** Accept header value. Defaults to 'text/event-stream'. */
  accept?: string;
  /**
   * Called for each fully-decoded SSE frame. The argument is the joined
   * data-line content (multiple `data:` lines within one frame are joined
   * with `\n`, matching the original loops in each call site — except for
   * claudemonSessionClient which joins with '' for base64).
   *
   * NOTE: `onFrame` receives the raw joined string; each call site is
   * responsible for doing exactly what its inline loop did with the string.
   */
  onFrame: (dataString: string) => void;
  /** Called when a single stream attempt errors (before backoff). Optional. */
  onError?: (err: unknown) => void;
  /** Initial backoff delay in ms. Defaults to 200. */
  backoffInitialMs?: number;
  /** Maximum backoff delay in ms. Defaults to 5000. */
  backoffMaxMs?: number;
  /**
   * Separator join string used when multiple `data:` lines appear in one
   * frame. Defaults to '\n'. Pass '' for the base64 stream consumer which
   * concatenates without separator.
   */
  joinWith?: string;
}

/**
 * Run a reconnecting SSE consumer loop against `url`.
 *
 * Loops until `signal` is aborted. Each connection attempt fetches `url`,
 * reads the byte stream, accumulates a text buffer, splits on `\n\n` OR
 * `\r\n\r\n` frame terminators (current servers use `\n\n`; CRLF support is
 * inert but correct), strips leading spaces from `data:` lines
 * (`.replace(/^ /, '')`), then calls `onFrame` with the joined data string.
 *
 * After a clean EOF or an error the loop sleeps with exponential backoff
 * (capped at `backoffMaxMs`) then reconnects, unless the signal is already
 * aborted.
 */
export async function consumeSseStream(
  url: string,
  opts: ConsumeSseStreamOptions,
): Promise<void> {
  const {
    signal,
    accept = 'text/event-stream',
    onFrame,
    onError,
    backoffInitialMs = 200,
    backoffMaxMs = 5000,
    joinWith = '\n',
  } = opts;

  let backoff = backoffInitialMs;
  /** Minimum open duration (ms) before a clean EOF resets backoff. */
  const MIN_PRODUCTIVE_MS = 5000;

  while (!signal.aborted) {
    try {
      const res = await fetch(url, {
        headers: { Accept: accept },
        signal,
      });
      if (!res.ok || !res.body) {
        throw new Error(`HTTP ${res.status}`);
      }

      const reader = res.body.getReader();
      const decoder = new TextDecoder();
      let buffer = '';
      let firstFrameSeen = false;
      const connectedAt = Date.now();

      /** Extract and emit all complete SSE frames from the current buffer. */
      function flushBuffer(): void {
        while (!signal.aborted) {
          const lfIdx = buffer.indexOf('\n\n');
          const crlfIdx = buffer.indexOf('\r\n\r\n');

          // Pick whichever terminator appears first.
          if (lfIdx === -1 && crlfIdx === -1) break;

          let frame: string;
          if (crlfIdx !== -1 && (lfIdx === -1 || crlfIdx < lfIdx)) {
            // CRLF terminator
            frame = buffer.slice(0, crlfIdx);
            buffer = buffer.slice(crlfIdx + 4);
          } else {
            // LF terminator
            frame = buffer.slice(0, lfIdx);
            buffer = buffer.slice(lfIdx + 2);
          }

          const dataLines: string[] = [];
          for (const line of frame.split('\n')) {
            // Normalize CRLF lines — strip a trailing \r if present.
            const normalized = line.endsWith('\r') ? line.slice(0, -1) : line;
            if (normalized.startsWith('data:')) {
              dataLines.push(normalized.slice(5).replace(/^ /, ''));
            }
          }
          if (dataLines.length === 0) continue;
          firstFrameSeen = true;
          onFrame(dataLines.join(joinWith));
        }
      }

      while (!signal.aborted) {
        const { done, value } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });

        // Support both LF-only (\n\n) and CRLF (\r\n\r\n) frame terminators.
        // Current servers only emit \n\n; CRLF handling is inert but correct.
        flushBuffer();
      }

      // After a clean EOF, flush any trailing frame that didn't end with a
      // double-newline terminator (servers sometimes omit the final \n\n).
      // Only attempt a flush if there's non-whitespace content left and the
      // buffer contains a data: line — avoids emitting empty frames.
      if (!signal.aborted && buffer.trim() && buffer.includes('data:')) {
        // Synthesize a terminator so the existing split logic fires cleanly.
        buffer += '\n\n';
        flushBuffer();
      }

      // Only reset backoff after a productive connection (delivered at least one
      // frame, or stayed open long enough). A server that closes immediately on
      // every attempt would otherwise cause a hot reconnect loop.
      if (firstFrameSeen || Date.now() - connectedAt >= MIN_PRODUCTIVE_MS) {
        backoff = backoffInitialMs;
      }
    } catch (err) {
      if (signal.aborted) return;
      if (onError) onError(err);
    }

    if (signal.aborted) return;
    await new Promise<void>((r) => setTimeout(r, backoff));
    backoff = Math.min(backoff * 2, backoffMaxMs);
  }
}
