/**
 * Which harnesses a picker should offer — the one rule every provider picker
 * shares (Spawn, Ask the Fleet, Handoff, Settings → Session/Supervisor).
 *
 * Workspacer speaks five harnesses (claude/codex/copilot/opencode/pi) but a
 * given machine usually has one or two installed, and a picker that offers all
 * five is offering four spawn failures. Detection already exists — the
 * `provider:checkAll` IPC / `providers.checkAll` bus capability, both backed by
 * main/services/agentProviders.checkAllProviders (PATH scan honouring the
 * `agents.binaries` overrides) — so this module only decides what to DO with it.
 *
 * Two rules, both deliberate:
 *
 *  - **Fail open.** A provider the detection list does not mention at all (no
 *    answer yet, an older host, a bus peer that doesn't serve the capability)
 *    is `unknown` and stays VISIBLE. Hiding on ignorance would silently remove
 *    a working harness; showing a spare card costs nothing.
 *  - **Never vanish something in use.** A harness named by config (the default
 *    provider, the supervisor's harness) or by an existing session (the source
 *    of a handoff) stays in the list even when it is missing — flagged `missing`
 *    so the UI can label it "not installed", the same philosophy as the
 *    stale-model warning in SupervisorSection. A picker whose current value is
 *    absent from its own options renders as nothing selected, which reads as a
 *    bug rather than as the diagnosis it is.
 */

/** One row of `provider:checkAll` (mirrors main-process ProviderStatus). */
export interface ProviderDetection {
  provider: string;
  found: boolean;
  /** Absolute path when found; null when missing. */
  resolvedPath: string | null;
  /** The user-configured override path ('' = not set). */
  customBin: string;
}

export type ProviderAvailability = 'installed' | 'missing' | 'unknown';

/**
 * Availability of one provider. `unknown` when detection hasn't answered yet or
 * doesn't cover this provider — callers treat that as visible (fail open).
 */
export function providerAvailability(
  detection: ProviderDetection[] | null | undefined,
  provider: string,
): ProviderAvailability {
  if (!detection?.length) return 'unknown';
  const row = detection.find((d) => d.provider === provider);
  if (!row) return 'unknown';
  return row.found ? 'installed' : 'missing';
}

/** Convenience: false only when detection positively says the CLI is missing. */
export function isProviderOffered(
  detection: ProviderDetection[] | null | undefined,
  provider: string,
): boolean {
  return providerAvailability(detection, provider) !== 'missing';
}

/** A picker option annotated with why it survived the filter. */
export type WithMissing<T> = T & {
  /** True when detection says this CLI is absent — shown, but flagged. */
  missing: boolean;
};

/**
 * Filter a picker's option list to what is installed, keeping any option named
 * in `keep` (the current value, the configured default, an existing session's
 * harness) even when missing — flagged rather than dropped.
 *
 * `keep` takes undefined/null entries so callers can pass optional config
 * values straight through.
 */
export function visibleProviderOptions<T extends { value: string }>(
  options: readonly T[],
  detection: ProviderDetection[] | null | undefined,
  keep: readonly (string | null | undefined)[] = [],
): WithMissing<T>[] {
  const kept = new Set(keep.filter((k): k is string => !!k));
  const out: WithMissing<T>[] = [];
  for (const opt of options) {
    const availability = providerAvailability(detection, opt.value);
    if (availability === 'missing' && !kept.has(opt.value)) continue;
    out.push({ ...opt, missing: availability === 'missing' });
  }
  return out;
}

/** Label suffix for a flagged-but-kept option ("Codex (not installed)"). */
export const NOT_INSTALLED_SUFFIX = ' (not installed)';
