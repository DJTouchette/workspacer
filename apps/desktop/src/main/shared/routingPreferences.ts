/** Wire contract with internal/routing/preferences.go; shared by IPC and web.
 * Product values come exclusively from the connected hub's compiled defaults. */
export interface RoutingAssignment {
  provider: string;
  model: string;
  effort?: string;
  minEffort?: string;
  fresh?: boolean;
  enabled?: boolean;
  alternatives?: RoutingAlternative[];
}
export type RoutingAlternative = Omit<RoutingAssignment, 'alternatives'>;
export interface RoutingPolicy {
  activeProfile: string;
  roles: Record<string, string>;
  profiles: Record<string, Record<string, RoutingAssignment>>;
  providers: Record<string, { enabled?: boolean }>;
  modes: { global: string; providers: Record<string, string> };
  modeShifts: Record<
    string,
    { roles?: Record<string, string>; effortStep?: number; effortStepCapabilities?: string[] }
  >;
  thresholds: {
    spendDown: {
      timeToResetMinutes: number;
      minRemainingPct: number;
      maxForecastPctOfRemaining: number;
    };
    health: { yellowAtUsedPct: number; redAtUsedPct: number };
    pacing: {
      enabled?: boolean;
      conserveAtRatio: number;
      blockSpendDownAtRatio: number;
      bootstrap: { minElapsedPct: number; expectedOffsetPct: number };
      sevenDay: {
        curve: string;
        timezone: string;
        weekendWeight: number;
        weekend: string;
        weekendReservePct: number;
      };
    };
  };
  forecastWeights: Record<string, number>;
}
export type RoutingPatch<T = RoutingPolicy> = {
  [K in keyof T]?: T[K] extends unknown[] ? T[K] : T[K] extends object ? RoutingPatch<T[K]> : T[K];
};
export interface RoutingValidation {
  catalogChecked: boolean;
  valid: boolean;
  catalogPending: boolean;
  issues: Array<{ where: string; detail: string }>;
  changedPaths: string[];
}
export interface RoutingPreferencesView {
  schemaVersion: 1;
  revision: string;
  configurable: boolean;
  defaults: RoutingPolicy;
  inherited: RoutingPolicy;
  overrides: RoutingPatch;
  effective: RoutingPolicy;
  sourceByPath: Record<string, 'shipped' | 'host' | 'managed'>;
  managedFields: string[];
  catalog: Record<
    string,
    {
      state: 'available' | 'unavailable' | 'unknown';
      observedAt?: number;
      models?: Array<{ id: string; label?: string; effortLevels?: string[] }>;
    }
  >;
  validation: RoutingValidation;
  warning?: string;
}
export interface RoutingPreferencesRequest {
  baseRevision: string;
  patch: RoutingPatch;
}
export interface RoutingPreferencesResult {
  status: 'applied' | 'valid' | 'invalid' | 'conflict' | 'unavailable';
  view: RoutingPreferencesView;
  validation: RoutingValidation;
}
export interface RoutingPreviewRequest {
  role: string;
  profile?: string;
  provider?: string;
  cwd?: string;
}
export interface RoutingPreview {
  role: string;
  profile: string;
  provider: string;
  model: string;
  effort: string;
  capability: string;
  baseCapability: string;
  fresh: boolean;
  eligible: boolean;
  capped: boolean;
  mode: string;
  reason: string[];
  effortStep?: { from?: string; to?: string; why: string };
  observedAt: number;
  usageState: string;
}
export interface RoutingAPI {
  routingPreferencesGet: () => Promise<RoutingPreferencesView>;
  routingPreferencesValidate: (
    request: RoutingPreferencesRequest,
  ) => Promise<RoutingPreferencesResult>;
  routingPreferencesSave: (request: RoutingPreferencesRequest) => Promise<RoutingPreferencesResult>;
  routingPreferencesReset: (request: { baseRevision: string }) => Promise<RoutingPreferencesResult>;
  routingPreview: (request: RoutingPreviewRequest) => Promise<RoutingPreview>;
}
/** One transport factory gives IPC and direct connected-hub web identical errors. */
export function routingAPI(call: <T>(method: string, params: unknown) => Promise<T>): RoutingAPI {
  return {
    routingPreferencesGet: () => call('routing.preferences.get', {}),
    routingPreferencesValidate: (request) => call('routing.preferences.validate', request),
    routingPreferencesSave: (request) => call('routing.preferences.save', request),
    routingPreferencesReset: (request) => call('routing.preferences.reset', request),
    routingPreview: (request) => call('routing.preview', request),
  };
}
