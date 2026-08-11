/**
 * WRITES THAT NEVER LANDED MUST NOT LOOK LIKE WRITES THAT DID.
 *
 * Three UI surfaces reported success for a write that went nowhere:
 *
 *  - ModelPricingSection printed the green "Saved" label for a
 *    `pricingSaveOverrides` that resolved `{ ok: false }`. The section is
 *    rendered unconditionally by SettingsPane (no platform gate) and on the web
 *    client that call is a stub that writes nothing, while the section's own
 *    body text promises the edit reaches ~/.workspacer/model-rates.json.
 *  - PluginsSection fired setPluginSettings with no await and no catch, so a
 *    refused write left the control visibly flipped until the plugin was
 *    reopened and reverted.
 *  - LayoutsDialog's save cleared the name field and ran the success animation
 *    for a rejected layouts.save; the reloaded list simply lacked the entry.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/react';

import ModelPricingSection from '../../src/components/settings/ModelPricingSection';
import { PluginDetailSection } from '../../src/components/settings/PluginsSection';
import LayoutsDialog from '../../src/components/LayoutsDialog';
import { saveLayoutTemplate } from '../../src/lib/saveLayoutTemplate';

vi.mock('../../src/hooks/usePlugins', () => ({
  usePlugins: () => ({ plugins: [pluginFixture], reload: vi.fn() }),
}));

const pluginFixture = {
  id: 'demo',
  name: 'Demo',
  apiVersion: '1',
  settings: [{ key: 'verbose', type: 'boolean', label: 'Verbose', default: false }],
} as any;

beforeEach(() => {
  (window as any).electronAPI = {};
});

describe('ModelPricingSection — a save the host refused', () => {
  function mount(saveResult: unknown) {
    const pricingSaveOverrides = vi.fn().mockResolvedValue(saveResult);
    (window as any).electronAPI = {
      // Exactly the webBackend.ts stubs.
      pricingGetRates: vi.fn().mockResolvedValue({ defaults: {}, overrides: {} }),
      pricingSaveOverrides,
    };
    render(<ModelPricingSection />);
    return pricingSaveOverrides;
  }

  it('does NOT print "Saved" when the write resolved { ok: false }', async () => {
    const save = mount({ ok: false });
    fireEvent.click(await screen.findByText(/Edit rate table/i));
    fireEvent.click(await screen.findByText('Save overrides'));

    await waitFor(() => expect(save).toHaveBeenCalled());
    await waitFor(() => expect(screen.getByText(/Not saved/i)).toBeTruthy());
    expect(screen.queryByText('Saved')).toBeNull();
  });

  it('prints "Saved" when the write actually landed', async () => {
    const save = mount({ ok: true });
    fireEvent.click(await screen.findByText(/Edit rate table/i));
    fireEvent.click(await screen.findByText('Save overrides'));

    await waitFor(() => expect(save).toHaveBeenCalled());
    await waitFor(() => expect(screen.getByText('Saved')).toBeTruthy());
    expect(screen.queryByText(/Not saved/i)).toBeNull();
  });
});

describe('PluginsSection — a plugin-settings write the hub refused', () => {
  function mount(setResult: unknown) {
    const setPluginSettings = vi.fn().mockResolvedValue(setResult);
    (window as any).electronAPI = {
      getPluginSettings: vi.fn().mockResolvedValue({ verbose: false }),
      onPluginSettingsChanged: vi.fn().mockReturnValue(() => {}),
      setPluginSettings,
    };
    render(<PluginDetailSection pluginId="demo" onBack={() => {}} />);
    return setPluginSettings;
  }

  it('reverts the control and says so when the write is refused', async () => {
    // `null` is the refusal signal — an empty map is a legitimate settings
    // document, so main returning `{}` (its old failure value) was
    // indistinguishable from a successful save of nothing.
    const set = mount(null);
    const box = (await screen.findByLabelText('Verbose', {
      selector: 'input',
    })) as HTMLInputElement;
    expect(box.checked).toBe(false);

    fireEvent.click(box);
    await waitFor(() => expect(set).toHaveBeenCalledWith('demo', { verbose: true }));
    await waitFor(() => expect(screen.getByText(/Not saved/i)).toBeTruthy());
    expect(box.checked, 'a refused write must not leave the toggle visibly ON').toBe(false);
  });

  it('keeps the new value when the write landed', async () => {
    const set = mount({ verbose: true });
    const box = (await screen.findByLabelText('Verbose', {
      selector: 'input',
    })) as HTMLInputElement;

    fireEvent.click(box);
    await waitFor(() => expect(set).toHaveBeenCalled());
    await waitFor(() => expect(box.checked).toBe(true));
    expect(screen.queryByText(/Not saved/i)).toBeNull();
  });
});

describe('LayoutsDialog — a layouts.save that never landed', () => {
  function mount(onSaveCurrent: (n: string) => Promise<void>) {
    (window as any).electronAPI = {
      layoutsList: vi.fn().mockResolvedValue([]),
      layoutsDelete: vi.fn().mockResolvedValue(undefined),
    };
    render(
      <LayoutsDialog
        agentCount={2}
        onSaveCurrent={onSaveCurrent}
        onRestore={() => {}}
        onClose={() => {}}
      />,
    );
  }

  it('says so, and keeps the typed name, when the save rejects', async () => {
    mount(() => Promise.reject(new Error('bus: no provider for layouts.save')));
    const input = (await screen.findByPlaceholderText(/Save 2 agents as/i)) as HTMLInputElement;
    fireEvent.change(input, { target: { value: 'my layout' } });
    fireEvent.click(screen.getByText('Save current'));

    await waitFor(() => expect(screen.getByText(/Not saved/i)).toBeTruthy());
    expect(screen.getByText(/no provider for layouts.save/)).toBeTruthy();
    expect(input.value, 'the typed name is the only copy of it').toBe('my layout');
  });

  it('clears the field and shows no error when the save lands', async () => {
    mount(() => Promise.resolve());
    const input = (await screen.findByPlaceholderText(/Save 2 agents as/i)) as HTMLInputElement;
    fireEvent.change(input, { target: { value: 'my layout' } });
    fireEvent.click(screen.getByText('Save current'));

    await waitFor(() => expect(input.value).toBe(''));
    expect(screen.queryByText(/Not saved/i)).toBeNull();
  });
});

describe('saveLayoutTemplate — the handler App hands to the dialog', () => {
  it('propagates the rejection instead of swallowing it into undefined', async () => {
    const layoutsSave = vi.fn().mockRejectedValue(new Error('bus: no provider for layouts.save'));
    (window as any).electronAPI = { layoutsSave };
    await expect(saveLayoutTemplate('x', [])).rejects.toThrow('no provider for layouts.save');
  });

  it('resolves undefined on success', async () => {
    (window as any).electronAPI = { layoutsSave: vi.fn().mockResolvedValue({ id: '1' }) };
    await expect(saveLayoutTemplate('x', [])).resolves.toBeUndefined();
  });
});
