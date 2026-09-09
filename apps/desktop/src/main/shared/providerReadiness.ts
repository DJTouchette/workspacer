/** Advisory inference facts only. Never carries model output, identity or paths. */
export type ProviderReadinessState =
  | 'unchecked'
  | 'checking'
  | 'responding'
  | 'unauthenticated'
  | 'limited'
  | 'network-error'
  | 'timeout'
  | 'unsupported'
  | 'error';
export interface ProviderReadiness {
  state: ProviderReadinessState;
  checkedAt?: number;
}
export const UNCHECKED_PROVIDER: ProviderReadiness = { state: 'unchecked' };
export const UNSUPPORTED_PROVIDER: ProviderReadiness = { state: 'unsupported' };
export function normalizeProviderReadiness(value: unknown): ProviderReadiness {
  const row = value as Partial<ProviderReadiness> | null;
  if (
    !row ||
    ![
      'unchecked',
      'checking',
      'responding',
      'unauthenticated',
      'limited',
      'network-error',
      'timeout',
      'unsupported',
      'error',
    ].includes(row.state ?? '')
  )
    return UNCHECKED_PROVIDER;
  return {
    state: row.state!,
    ...(typeof row.checkedAt === 'number' && Number.isFinite(row.checkedAt) && row.checkedAt > 0
      ? { checkedAt: row.checkedAt }
      : {}),
  };
}
export function providerReadinessDetail(result: ProviderReadiness): string {
  switch (result.state) {
    case 'responding':
      return 'Provider responded to a small test request.';
    case 'unauthenticated':
      return 'Provider reported an authentication failure. Sign in through its CLI, then check again.';
    case 'limited':
      return 'Provider reported a usage or rate limit. You can still try launching.';
    case 'network-error':
      return 'Provider check could not connect. You can still try launching.';
    case 'timeout':
      return 'Provider check timed out. You can still try launching.';
    case 'error':
      return 'Provider check failed; authentication is unknown. You can still try launching.';
    case 'unsupported':
      return 'An isolated provider check is unavailable for this provider, account or host.';
    case 'checking':
      return 'Checking provider with a small test request…';
    default:
      return 'Provider has not been checked. Authentication is unknown.';
  }
}
