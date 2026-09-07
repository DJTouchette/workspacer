import React from 'react';
import type { AgentWorkspace } from '../types/pane';
import type { ClaudeSessionSnapshot } from '../types/claudeSession';

// Unknown timestamps stay unknown, including invalid or out-of-range wire values.
export function fleetActivityTime(value: number | undefined): number | undefined {
  return typeof value === 'number' && value > 0 && Number.isFinite(new Date(value).getTime())
    ? value
    : undefined;
}

export function orderFleetTimeline(
  agents: AgentWorkspace[],
  snapshots: Record<string, ClaudeSessionSnapshot>,
  query: string,
): AgentWorkspace[] {
  const q = query.trim().toLowerCase();
  const time = (agent: AgentWorkspace) =>
    fleetActivityTime(agent.sessionId ? snapshots[agent.sessionId]?.lastActivity : undefined) ?? 0;
  return agents
    .filter(
      (agent) =>
        !agent.global &&
        (agent.manager ||
          !q ||
          [agent.name, agent.provider, agent.cwd, agent.sessionId].some((value) =>
            value?.toLowerCase().includes(q),
          )),
    )
    .sort((a, b) => {
      if (a.manager || b.manager) return a.manager && b.manager ? 0 : a.manager ? -1 : 1;
      // Stable ties preserve source order; missing times follow all recorded times.
      return time(b) - time(a);
    });
}

export function FleetActivityTime({ timestamp, now }: { timestamp?: number; now: number }) {
  const value = fleetActivityTime(timestamp);
  if (value === undefined) return <span>Time unknown</span>;
  const date = new Date(value);
  const today = date.toDateString() === new Date(now).toDateString();
  return (
    <time dateTime={date.toISOString()} title={`Last activity: ${date.toLocaleString()}`}>
      <span>
        {today
          ? 'Today'
          : date.toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' })}
      </span>
      <span>{date.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })}</span>
    </time>
  );
}
