import React from 'react';
import { act, renderHook, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { useAgentRuntimeStatus } from '../src/hooks/useAgentRuntimeStatus';

describe('runtime snapshot compatibility and request ordering', () => {
  it('ignores an older ready reply after a newer failed read and keeps remote unknown', async () => {
    let finish!: (status: any) => void;
    const read = vi
      .fn()
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            finish = resolve;
          }),
      )
      .mockResolvedValue({ claudemon: 'failed', hub: 'ready', facade: 'ready' });
    window.electronAPI.agentRuntimeStatus = read;
    const { result, rerender } = renderHook(({ remote }) => useAgentRuntimeStatus(false, remote), {
      initialProps: { remote: false },
    });
    await act(async () => {
      await result.current.refresh();
    });
    expect(result.current.blocked).toBe(true);
    await act(async () => {
      finish({ claudemon: 'ready', hub: 'ready', facade: 'ready' });
    });
    expect(result.current.blocked).toBe(true);
    rerender({ remote: true });
    await waitFor(() => expect(result.current.detail).toContain('unknown'));
    expect(read).toHaveBeenCalledTimes(2);
  });
  it('treats absent and malformed old-host facts as unknown, never missing or ready', async () => {
    window.electronAPI.agentRuntimeStatus = vi
      .fn()
      .mockResolvedValue({ claudemon: ['ready'], hub: null });
    const { result } = renderHook(() => useAgentRuntimeStatus(true));
    await act(async () => {
      await result.current.refresh();
    });
    expect(result.current).toMatchObject({ blocked: false });
    expect(result.current.detail).toContain('unknown');
  });
});
