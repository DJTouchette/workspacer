import React, { useEffect } from 'react';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import MachinePowerGate from '../../src/components/MachinePowerGate';

const power = vi.hoisted(() => ({
  state: { canStop: true, paused: false, connected: true, label: 'Remote', error: '' },
  listener: () => {},
  stop: vi.fn().mockResolvedValue(undefined),
  wake: vi.fn(),
}));
vi.mock('../../src/backend/machinePower', () => ({
  machinePowerSnapshot: () => power.state,
  subscribeMachinePower: (listener: () => void) => {
    power.listener = listener;
    return () => {};
  },
  stopMachine: power.stop,
  wakeMachine: power.wake,
}));
vi.mock('../../src/components/RemoteShareDialog', () => ({ default: () => null }));

describe('MachinePowerGate', () => {
  it('confirms Stop and unmounts app pollers while paused, then exposes explicit Wake', async () => {
    const cleanup = vi.fn();
    function App() {
      useEffect(() => cleanup, []);
      return <div>Connected app</div>;
    }
    render(
      <MachinePowerGate>
        <App />
      </MachinePowerGate>,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Stop server' }));
    expect(power.stop).not.toHaveBeenCalled();
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Stop machine' }));
    });
    expect(power.stop).toHaveBeenCalledOnce();
    act(() => {
      power.state = { ...power.state, paused: true, connected: false };
      power.listener();
    });
    expect(cleanup).toHaveBeenCalledOnce();
    expect(screen.queryByText('Connected app')).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Wake server' }));
    expect(power.wake).toHaveBeenCalledOnce();
  });
});
