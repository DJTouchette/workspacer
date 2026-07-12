import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import React from 'react';
import SpawnAgentDialog from '../../src/components/SpawnAgentDialog';

const api = window.electronAPI as unknown as Record<string, ReturnType<typeof vi.fn>>;

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

beforeEach(() => {
  api.claudeListModels = vi.fn().mockResolvedValue({
    defaultModel: '',
    skipPermissionsDefault: false,
    defaultPermissionMode: '',
    aliases: [],
    seen: [],
  });
  api.providerCheckAll = vi.fn().mockResolvedValue([]);
});

describe('SpawnAgentDialog permissions', () => {
  it('defaults new Claude spawns to ask/approve permissions', () => {
    const { onSpawn } = renderDialog();

    expect(permissionSelect().value).toBe('');
    fireEvent.click(screen.getByRole('button', { name: /spawn agent/i }));

    expect(onSpawn).toHaveBeenCalledWith(
      expect.objectContaining({
        cwd: '/repo',
        permissionMode: 'default',
        skipPermissions: false,
      }),
    );
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

    await waitFor(() => expect(permissionSelect().value).toBe('bypassPermissions'));
    expect(screen.getByText(/bypasses all approval prompts/i)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /spawn agent/i }));

    expect(onSpawn).toHaveBeenCalledWith(
      expect.objectContaining({
        cwd: '/repo',
        permissionMode: 'bypassPermissions',
        skipPermissions: true,
      }),
    );
  });
});
