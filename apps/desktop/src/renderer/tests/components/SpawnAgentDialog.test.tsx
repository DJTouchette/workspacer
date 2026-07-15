import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import React from 'react';
import SpawnAgentDialog from '../../src/components/SpawnAgentDialog';

const api = window.electronAPI as unknown as Record<string, ReturnType<typeof vi.fn>>;
let localStore: Record<string, string>;

function renderDialog(onSpawn = vi.fn()) {
  render(<SpawnAgentDialog defaultCwd="/repo" onSpawn={onSpawn} onCancel={vi.fn()} />);
  return { onSpawn };
}

function permissionSelect(): HTMLSelectElement {
  const found = screen
    .getAllByRole('combobox')
    .find((el) =>
      Array.from((el as HTMLSelectElement).options).some(
        (opt) => opt.textContent === 'Full access',
      ),
    );
  if (!found) throw new Error('permission select not found');
  return found as HTMLSelectElement;
}

function effortSelect(): HTMLSelectElement {
  const found = screen
    .getAllByRole('combobox')
    .find((el) =>
      Array.from((el as HTMLSelectElement).options).some(
        (opt) => opt.value === 'xhigh' || opt.value === 'minimal',
      ),
    );
  if (!found) throw new Error('effort select not found');
  return found as HTMLSelectElement;
}

function providerModelSelect(): HTMLSelectElement {
  const found = screen
    .getAllByRole('combobox')
    .find((el) =>
      Array.from((el as HTMLSelectElement).options).some((opt) => opt.value === 'gpt-5.5'),
    );
  if (!found) throw new Error('provider model select not found');
  return found as HTMLSelectElement;
}

function advancedButton(): HTMLButtonElement {
  return screen.getByRole('button', { name: /advanced/i }) as HTMLButtonElement;
}

beforeEach(() => {
  localStore = {};
  Object.defineProperty(window, 'localStorage', {
    configurable: true,
    value: {
      getItem: vi.fn((key: string) => localStore[key] ?? null),
      setItem: vi.fn((key: string, value: string) => {
        localStore[key] = value;
      }),
      removeItem: vi.fn((key: string) => {
        delete localStore[key];
      }),
    },
  });
  api.claudeListModels = vi.fn().mockResolvedValue({
    defaultModel: '',
    skipPermissionsDefault: false,
    defaultPermissionMode: '',
    aliases: [],
    seen: [],
  });
  api.providerCheckAll = vi.fn().mockResolvedValue([]);
  api.providerListModels = vi.fn().mockResolvedValue([
    {
      id: 'gpt-5.5',
      label: 'GPT-5.5',
      default: true,
      effortLevels: ['low', 'medium', 'high', 'xhigh'],
    },
    {
      id: 'legacy-codex',
      label: 'Legacy Codex',
      default: false,
      effortLevels: ['minimal', 'low', 'medium', 'high'],
    },
  ]);
});

describe('SpawnAgentDialog permissions', () => {
  it('keeps advanced controls collapsed while allowing directory-only spawn', () => {
    const { onSpawn } = renderDialog();

    expect(advancedButton()).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryAllByRole('combobox')).toHaveLength(0);

    fireEvent.click(screen.getByRole('button', { name: /spawn agent/i }));

    expect(onSpawn).toHaveBeenCalledWith(
      expect.objectContaining({
        cwd: '/repo',
        permissionMode: 'default',
        skipPermissions: false,
      }),
    );
  });

  it('shows safe approval defaults when Advanced is opened', () => {
    renderDialog();

    fireEvent.click(advancedButton());

    expect(permissionSelect().value).toBe('');
  });

  it('keeps an explicit saved full-access opt-in sticky', async () => {
    api.claudeListModels = vi.fn().mockResolvedValue({
      defaultModel: '',
      skipPermissionsDefault: true,
      defaultPermissionMode: 'bypassPermissions',
      aliases: [],
      seen: [],
    });
    const { onSpawn } = renderDialog();

    expect(await screen.findByText(/bypasses all approval prompts/i)).toBeInTheDocument();
    expect(advancedButton()).toHaveTextContent(/full access/i);

    fireEvent.click(advancedButton());
    await waitFor(() => expect(permissionSelect().value).toBe('bypassPermissions'));

    fireEvent.click(screen.getByRole('button', { name: /spawn agent/i }));

    expect(onSpawn).toHaveBeenCalledWith(
      expect.objectContaining({
        cwd: '/repo',
        permissionMode: 'bypassPermissions',
        skipPermissions: true,
      }),
    );
  });

  it('remembers when a power user leaves Advanced open', () => {
    window.localStorage.setItem('workspacer.spawn.advancedOpen', 'true');

    renderDialog();

    expect(advancedButton()).toHaveAttribute('aria-expanded', 'true');
    expect(permissionSelect().value).toBe('');
  });

  it('keeps effort selections harness-specific and sends the Claude selection', async () => {
    const { onSpawn } = renderDialog();
    fireEvent.click(advancedButton());

    // Claude has its own ladder and remembers its own selection.
    expect(Array.from(effortSelect().options).map((o) => o.value)).toContain('max');
    fireEvent.change(effortSelect(), { target: { value: 'xhigh' } });

    fireEvent.click(screen.getByText('Codex').closest('button')!);
    await waitFor(() => expect(providerModelSelect()).toBeInTheDocument());
    expect(effortSelect().value).toBe('');
    expect(Array.from(effortSelect().options).map((o) => o.value)).toContain('xhigh');
    expect(Array.from(effortSelect().options).map((o) => o.value)).not.toContain('max');

    // A different Codex model can expose a different exact ladder.
    fireEvent.change(providerModelSelect(), { target: { value: 'legacy-codex' } });
    await waitFor(() =>
      expect(Array.from(effortSelect().options).map((o) => o.value)).toContain('minimal'),
    );
    expect(Array.from(effortSelect().options).map((o) => o.value)).not.toContain('xhigh');
    fireEvent.change(effortSelect(), { target: { value: 'high' } });

    fireEvent.click(screen.getByText('Claude Code').closest('button')!);
    await waitFor(() => expect(effortSelect().value).toBe('xhigh'));

    fireEvent.click(screen.getByRole('button', { name: /spawn agent/i }));
    expect(onSpawn).toHaveBeenCalledWith(expect.objectContaining({ effort: 'xhigh' }));
  });
});
