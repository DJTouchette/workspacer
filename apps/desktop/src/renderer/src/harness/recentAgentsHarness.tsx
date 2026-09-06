import { SESSION_WATCH_EVENT } from '../lib/watchBus';
import React from 'react';
import { createRoot } from 'react-dom/client';
import '../App.css';
import RecentAgentsPane from '../panes/RecentAgentsPane';
import { createBridgedBackend } from '../backend/bridgedBackend';
import { recentAgentsFixture } from './recentAgentsFixture';
import { applyTheme, resolveTheme } from '../themes';
import type { ElectronAPI } from '../types/electron';
const params = new URLSearchParams(location.search);
applyTheme(resolveTheme(params.get('theme') ?? 'everforest'));
const calls: unknown[] = [];
const opened: unknown[] = [];
window.addEventListener(SESSION_WATCH_EVENT, (event) => opened.push((event as CustomEvent).detail));
Object.assign(window, { reviewCalls: calls, opened });
const ipc = {
  platform: 'linux',
  ...(params.get('mode') !== 'unsupported'
    ? {
        dispatchHistoryRead: async () => ({
          available: true,
          currentOwnerSessionId: params.get('mode') === 'stopped' ? undefined : 'manager',
          tasks: params.get('mode') === 'empty' ? [] : recentAgentsFixture(),
        }),
      }
    : {}),
  fleetReviewRead: async (request: unknown) => {
    calls.push(request);
    return { ok: false, error: 'Review evidence evicted or revoked' };
  },
} as unknown as ElectronAPI;
// This is the production transport factory, including its HOST_ONLY routing.
window.electronAPI = createBridgedBackend(ipc, 'fixture', `ws://${location.host}/fixture-bus`);
createRoot(document.getElementById('root')!).render(<RecentAgentsPane />);
