/** Fixed stage names and identifiers only: no prompts, paths, argv or tokens. */
let sequence = 0;
export function createSpawnTiming(provider: string, transport?: string) {
  const traceId = globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${++sequence}`;
  const started = performance.now();
  let last = started;
  function emit(stage: string, since: number, outcome: 'ok' | 'error', sessionId?: string) {
    const now = performance.now();
    // The headless host reserves stdout for its JSON protocol. stderr is also
    // captured by desktop file logging and is safe for operational timings.
    console.warn(
      '[spawn-timing]',
      JSON.stringify({
        traceId,
        provider,
        transport,
        stage,
        outcome,
        sessionId,
        durationMs: Math.round((now - since) * 100) / 100,
        elapsedMs: Math.round((now - started) * 100) / 100,
      }),
    );
    last = now;
  }
  return {
    mark(stage: string, sessionId?: string) {
      emit(stage, last, 'ok', sessionId);
    },
    async measure<T>(stage: string, run: () => Promise<T>): Promise<T> {
      const since = performance.now();
      try {
        const result = await run();
        emit(stage, since, 'ok');
        return result;
      } catch (error) {
        emit(stage, since, 'error');
        throw error;
      }
    },
  };
}
