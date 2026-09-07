import React from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import raw from '../../../../../../services/hub/internal/routing/testdata/preferences-view.json?raw';
import RoutingSection from '../../src/components/settings/RoutingSection';
import type { RoutingPreferencesView } from '../../../main/shared/routingPreferences';
const fixture = (): RoutingPreferencesView => JSON.parse(raw);
const api = (v: Record<string, unknown>) => {
  (window as unknown as { electronAPI: unknown }).electronAPI = v;
};
afterEach(() => {
  cleanup();
  api({});
});
describe('Routing settings', () => {
  it('sends a sparse draft, validates and renders the installed value', async () => {
    const view = fixture();
    const validate = vi.fn(async () => ({
      status: 'valid',
      view,
      validation: { valid: true, catalogPending: false, issues: [], changedPaths: ['roles.scout'] },
    }));
    const save = vi.fn(async () => {
      const next = fixture();
      next.effective.roles.scout = 'cheap';
      next.sourceByPath['roles.scout'] = 'managed';
      return {
        status: 'applied',
        view: next,
        validation: { valid: true, issues: [], changedPaths: [] },
      };
    });
    api({
      routingPreferencesGet: async () => view,
      routingPreferencesValidate: validate,
      routingPreferencesSave: save,
    });
    render(<RoutingSection />);
    const user = userEvent.setup();
    await user.selectOptions(await screen.findByLabelText('scout capability'), 'cheap');
    expect(screen.getByRole('button', { name: 'Apply', exact: true })).toBeDisabled();
    await user.click(screen.getByRole('button', { name: 'Validate', exact: true }));
    await waitFor(() =>
      expect(validate).toHaveBeenCalledWith({
        baseRevision: view.revision,
        patch: { roles: { scout: 'cheap' } },
      }),
    );
    await user.click(screen.getByRole('button', { name: 'Apply', exact: true }));
    await screen.findByText('Applied to the live routing service.');
    expect(save).toHaveBeenCalledWith({
      baseRevision: view.revision,
      patch: { roles: { scout: 'cheap' } },
    });
    expect(screen.getByRole('button', { name: 'Apply', exact: true })).toBeDisabled();
  });
  it('keeps a conflicting draft unapplied and offers explicit reload', async () => {
    const view = fixture();
    const save = vi.fn();
    api({
      routingPreferencesGet: async () => view,
      routingPreferencesSave: save,
      routingPreferencesValidate: async () => ({
        status: 'conflict',
        view,
        validation: { valid: false, issues: [], changedPaths: [] },
      }),
    });
    render(<RoutingSection />);
    const user = userEvent.setup();
    await user.selectOptions(await screen.findByLabelText('Active preset'), 'codex_only');
    await user.click(screen.getByRole('button', { name: 'Validate', exact: true }));
    await screen.findByRole('button', { name: 'Reload changed policy' });
    expect(screen.getByLabelText('Active preset')).toHaveValue('codex_only');
    expect(save).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: 'Apply', exact: true })).toBeDisabled();
  });
  it('reports old hub unavailable without config fallback', async () => {
    const config = vi.fn();
    api({
      routingPreferencesGet: async () => {
        throw new Error('no provider');
      },
      saveConfig: config,
    });
    render(<RoutingSection />);
    await screen.findByText(/Routing unavailable on this connected hub/);
    expect(screen.queryByLabelText('Active preset')).toBeNull();
    expect(config).not.toHaveBeenCalled();
  });
  it('keeps unknown catalog edits pending, and reset reveals inherited values', async () => {
    const view = fixture();
    view.managedFields = ['activeProfile'];
    view.effective.activeProfile = 'codex_only';
    const reset = vi.fn(async () => ({
      status: 'applied',
      view: fixture(),
      validation: { valid: true, issues: [], changedPaths: [] },
    }));
    api({
      routingPreferencesGet: async () => view,
      routingPreferencesReset: reset,
      routingPreferencesValidate: async () => ({
        status: 'invalid',
        view,
        validation: { valid: false, catalogPending: true, issues: [], changedPaths: [] },
      }),
    });
    render(<RoutingSection />);
    const user = userEvent.setup();
    await user.selectOptions(await screen.findByLabelText('scout capability'), 'cheap');
    await user.click(screen.getByRole('button', { name: 'Validate', exact: true }));
    await screen.findByText(/Catalog unknown for a changed assignment/);
    expect(screen.getByRole('button', { name: 'Apply', exact: true })).toBeDisabled();
    await user.click(screen.getByRole('button', { name: 'Reset managed preferences' }));
    await screen.findByText(
      'Managed preferences cleared. Inherited host/shipped values are active.',
    );
    expect(screen.getByLabelText('Active preset')).toHaveValue('mixed');
    expect(reset).toHaveBeenCalledWith({ baseRevision: view.revision });
  });
});
