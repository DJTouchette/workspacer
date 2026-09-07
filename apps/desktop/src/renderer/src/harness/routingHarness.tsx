/// <reference types="vite/client" />
/** Deterministic UI fixture: Go-generated defaults, no services or provider calls. */
import React from 'react';
import ReactDOM from 'react-dom/client';
import '../App.css';
import RoutingSection from '../components/settings/RoutingSection';
import { applyTheme, resolveTheme } from '../themes';
import raw from '../../../../../../services/hub/internal/routing/testdata/preferences-view.json?raw';
import type {
  RoutingAPI,
  RoutingPreferencesView,
  RoutingPreferencesRequest,
} from '../../../main/shared/routingPreferences';
let view: RoutingPreferencesView = JSON.parse(raw);
const copy = <T,>(v: T): T => JSON.parse(JSON.stringify(v));
const validation = {
  catalogChecked: true,
  valid: true,
  catalogPending: false,
  issues: [],
  changedPaths: [],
};
function merge(target: Record<string, any>, patch: Record<string, any>) {
  for (const [k, v] of Object.entries(patch)) {
    if (v && typeof v === 'object' && !Array.isArray(v)) merge((target[k] ??= {}), v);
    else target[k] = v;
  }
}
const api: RoutingAPI = {
  routingPreferencesGet: async () => copy(view),
  routingPreferencesValidate: async () => ({ status: 'valid', view: copy(view), validation }),
  routingPreferencesSave: async (req: RoutingPreferencesRequest) => {
    merge(view.effective, req.patch);
    merge(view.overrides, req.patch);
    const mark = (obj: object, prefix = '') => {
      for (const [key, value] of Object.entries(obj)) {
        const path = prefix ? `${prefix}.${key}` : key;
        if (value && typeof value === 'object' && !Array.isArray(value)) mark(value, path);
        else view.sourceByPath[path] = 'managed';
      }
    };
    mark(req.patch);
    view.managedFields = ['roles.scout'];
    view.revision += '-saved';
    return { status: 'applied', view: copy(view), validation };
  },
  routingPreferencesReset: async () => {
    view = JSON.parse(raw);
    return { status: 'applied', view: copy(view), validation };
  },
  routingPreview: async (req) => {
    const capability = view.effective.roles[req.role];
    const a = view.effective.profiles[req.profile || view.effective.activeProfile][capability];
    return {
      role: req.role,
      profile: view.effective.activeProfile,
      provider: a.provider,
      model: a.model,
      effort: a.effort || '',
      capability,
      baseCapability: capability,
      fresh: !!a.fresh,
      eligible: true,
      capped: false,
      mode: 'normal',
      reason: ['Selected from the applied policy. No agent was launched.'],
      observedAt: 1,
      usageState: 'fixture',
    };
  },
};
(window as unknown as { electronAPI: RoutingAPI }).electronAPI = api;
applyTheme(resolveTheme(new URLSearchParams(location.search).get('theme') || 'dracula'));
document.body.style.background = 'var(--wks-bg-base)';
document.body.style.color = 'var(--wks-text-primary)';
ReactDOM.createRoot(document.getElementById('root')!).render(
  <main
    style={{
      padding: 16,
      maxWidth: 980,
      margin: '0 auto',
      fontFamily: 'var(--wks-font-sans)',
      height: '100vh',
      boxSizing: 'border-box',
      overflow: 'auto',
    }}
  >
    <RoutingSection />
  </main>,
);
