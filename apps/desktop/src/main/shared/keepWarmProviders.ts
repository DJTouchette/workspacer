/**
 * WHICH HARNESSES KEEP-WARM CAN WARM — the one list both `main` and the
 * renderer read.
 *
 * This is deliberately NOT all five providers, and it is not an oversight:
 * Claude and Codex are the only harnesses that meter a 5-hour subscription
 * window a ping can START. Copilot bills monthly premium requests and exposes
 * no local quota at all (GitHub answers 403 to `copilot_internal/v2/token`),
 * and opencode/pi are bring-your-own-key with no window — warming any of the
 * three would spend tokens to start something that does not exist.
 *
 * The two sides read it for opposite reasons and must not disagree:
 * `services/keepWarmService.ts` FILTERS the configured provider list through
 * it before pinging, and `settings/SessionSection.tsx` builds the buttons that
 * WRITE that config from it. A provider offered by one and dropped by the
 * other is a button that writes config the service silently ignores, which is
 * exactly what a second hand-kept copy of the pair produces.
 *
 * Living in `main/shared` rather than `main/services` is the point, the same
 * reason `agentProfiles.ts` lives here: the control that sets a value and the
 * code that acts on it cannot be allowed to hold separate tables.
 *
 * Detection is the OTHER half and does not belong here: of these two, the
 * settings row offers the ones this machine actually has (`visibleProviderOptions`).
 * The buttons are the intersection, not either half alone.
 */
export const WARMABLE_PROVIDERS = ['claude', 'codex'] as const;

export type WarmableProvider = (typeof WARMABLE_PROVIDERS)[number];

/** Whether a configured provider id is one keep-warm can actually ping. */
export function isWarmableProvider(provider: string): provider is WarmableProvider {
  return (WARMABLE_PROVIDERS as readonly string[]).includes(provider);
}
