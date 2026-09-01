/** Provider-level context-request policy shared by every desktop spawn door. */

/**
 * Fresh-Codex request from contracts/model-context-windows.json's
 * providerContextDefaults block. modelContextWindowsContract.test.ts pins this
 * consumer to that cross-language fixture; the renderer imports this exact
 * value rather than carrying its own dialog literal.
 */
export const DEFAULT_CODEX_CONTEXT_WINDOW = 1_000_000;

export function providerAcceptsContextRequest(provider: string): boolean {
  const normalized = provider.trim().toLowerCase();
  return normalized === 'claude' || normalized === 'codex';
}

/** Reject a bad explicit request without conflating null with omission. */
export function validateProviderContextRequest(
  provider: string,
  contextWindow: number | null | undefined,
): void {
  if (contextWindow == null) return;
  if (!Number.isSafeInteger(contextWindow) || contextWindow <= 0) {
    throw new Error('invalid-context-window');
  }
  if (!providerAcceptsContextRequest(provider)) {
    throw new Error('unsupported-context-window');
  }
}

/** Default only a genuinely new Codex life. Resumes reuse their durable request. */
export function contextRequestForSpawn(
  provider: string,
  contextWindow: number | null | undefined,
  resumeSessionId?: string,
): number | null | undefined {
  validateProviderContextRequest(provider, contextWindow);
  if (
    provider.trim().toLowerCase() === 'codex' &&
    contextWindow === undefined &&
    !resumeSessionId
  ) {
    return DEFAULT_CODEX_CONTEXT_WINDOW;
  }
  return contextWindow;
}
