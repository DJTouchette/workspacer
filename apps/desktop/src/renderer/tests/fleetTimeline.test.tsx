import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import {
  FleetActivityTime,
  fleetActivityTime,
  orderFleetTimeline,
} from '../src/components/FleetTimeline';
import type { AgentWorkspace } from '../src/types/pane';

const agent = (id: string, manager = false) =>
  ({ id, name: id, sessionId: id, manager, cwd: '/project' }) as AgentWorkspace;
describe('Fleet runbook ordering', () => {
  it('anchors metadata managers and orders workers newest first, with stable ties and unknowns last', () => {
    const agents = [
      agent('unknown'),
      agent('old'),
      agent('manager', true),
      agent('new'),
      agent('Fleet Manager'),
      agent('second-manager', true),
      agent('tie'),
    ];
    const snapshots = {
      old: { lastActivity: 10 },
      new: { lastActivity: 30 },
      tie: { lastActivity: 30 },
      manager: { lastActivity: 1 },
    } as any;
    const order = (query = '') => orderFleetTimeline(agents, snapshots, query).map((a) => a.id);
    expect(order()).toEqual([
      'manager',
      'second-manager',
      'new',
      'tie',
      'old',
      'unknown',
      'Fleet Manager',
    ]);
    snapshots.old.lastActivity = 40;
    snapshots.manager.lastActivity = 100;
    expect(order()).toEqual([
      'manager',
      'second-manager',
      'old',
      'new',
      'tie',
      'unknown',
      'Fleet Manager',
    ]);
    expect(order('no match')).toEqual(['manager', 'second-manager']);
    expect(agents[0].id).toBe('unknown');
  });
  it.each([undefined, 0, -1, NaN, Infinity, 1e20])(
    'labels invalid time %s without inventing activity',
    (timestamp) => {
      expect(fleetActivityTime(timestamp)).toBeUndefined();
      render(<FleetActivityTime timestamp={timestamp} now={Date.now()} />);
      expect(screen.getByText('Time unknown')).toBeInTheDocument();
    },
  );
  it('exposes the exact timestamp and distinguishes earlier dates', () => {
    const timestamp = new Date(2026, 8, 5, 10, 43).getTime();
    const { container } = render(
      <FleetActivityTime timestamp={timestamp} now={new Date(2026, 8, 6).getTime()} />,
    );
    expect(container.querySelector('time')).toHaveAttribute(
      'datetime',
      new Date(timestamp).toISOString(),
    );
    expect(screen.queryByText('Today')).not.toBeInTheDocument();
  });
});
