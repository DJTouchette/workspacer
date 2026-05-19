/**
 * Consumes the daemon's `/hooks/stream` SSE feed and forwards each event into
 * `claudeSessionStore.handleHookEvent` — the same code path the old in-process
 * hook server used. Reconnects with exponential backoff if the daemon
 * restarts.
 */

import { claudeSessionStore } from './claudeSessionStore';
import { CLAUDEMON_API_URL } from './claudemonDaemon';

let stopped = false;
let currentReader: ReadableStreamDefaultReader<Uint8Array> | null = null;

async function consumeStream(): Promise<void> {
  const url = `${CLAUDEMON_API_URL}/hooks/stream`;
  const res = await fetch(url, { headers: { Accept: 'text/event-stream' } });
  if (!res.ok || !res.body) {
    throw new Error(`hook stream HTTP ${res.status}`);
  }
  console.log(`[claudemon-bridge] subscribed to ${url}`);

  const reader = res.body.getReader();
  currentReader = reader;
  const decoder = new TextDecoder();
  let buffer = '';

  while (!stopped) {
    const { done, value } = await reader.read();
    if (done) return;
    buffer += decoder.decode(value, { stream: true });

    // SSE frames are separated by a blank line. A frame is one or more
    // `field: value` lines; we only care about `data:`.
    let sep: number;
    while ((sep = buffer.indexOf('\n\n')) !== -1) {
      const frame = buffer.slice(0, sep);
      buffer = buffer.slice(sep + 2);

      const dataLines: string[] = [];
      for (const line of frame.split('\n')) {
        if (line.startsWith('data:')) {
          dataLines.push(line.slice(5).replace(/^ /, ''));
        }
      }
      if (dataLines.length === 0) continue;
      try {
        const event = JSON.parse(dataLines.join('\n'));
        // claudemon emits `hook_event_name` as `event`. Translate back so
        // claudeSessionStore (which reads `hook_event_name ?? type`) is happy.
        // The Rust serializer also flattens payload into the same object, so
        // tool_name / tool_input / etc. are top-level — matching the schema
        // Claude Code itself POSTs.
        const normalized = { ...event, hook_event_name: event.event };
        claudeSessionStore.handleHookEvent(normalized);
      } catch (err) {
        console.error('[claudemon-bridge] bad frame', err);
      }
    }
  }
}

export async function startClaudemonHookBridge(): Promise<void> {
  stopped = false;
  let backoff = 200;
  while (!stopped) {
    try {
      await consumeStream();
      // Clean close — daemon shutting down. Reconnect on next ready.
      backoff = 200;
    } catch (err) {
      console.warn(`[claudemon-bridge] stream error, retrying in ${backoff}ms:`, err);
    }
    if (stopped) break;
    await new Promise(r => setTimeout(r, backoff));
    backoff = Math.min(backoff * 2, 5000);
  }
}

export function stopClaudemonHookBridge(): void {
  stopped = true;
  if (currentReader) {
    try { currentReader.cancel(); } catch {}
    currentReader = null;
  }
}
