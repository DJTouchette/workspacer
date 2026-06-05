/**
 * Characterization tests for the backward-compat session migration helper
 * exported from App.tsx.
 *
 * The function normalizes four distinct legacy session shapes into the canonical
 * { agents, activeAgentId, name } payload that loadAgentsFromSession expects:
 *
 *   1. Modern format    — data.agents is an array
 *   2. Tabs-only        — data.tabs present, no agents array
 *   3. Panes-only       — data.panes present, no tabs / agents array
 *   4. Neither / null   — empty or null data
 */
import { describe, it, expect } from 'vitest';
import { migrateSessionData } from '../../src/App';

// ── helpers ───────────────────────────────────────────────────────────────────

function makePane(id = 'p1', title = 'Claude') {
  return { id, type: 'claude', title };
}

function makeTab(id = 't1', title = 'Claude', panes = [makePane()]) {
  return { id, title, panes, activePaneId: panes[0]?.id ?? '' };
}

function makeAgent(id = 'agent-1', name = 'feat-auth', tabs = [makeTab()]) {
  return { id, name, cwd: '/projects/feat-auth', tabs, activeTabId: tabs[0]?.id ?? '' };
}

const FALLBACK_CWD = '/app/cwd';

// ── 1. Modern format (data.agents array) ─────────────────────────────────────

describe('modern format — data.agents array', () => {
  it('returns agents as-is', () => {
    const agent = makeAgent();
    const data = { agents: [agent], activeAgentId: agent.id, name: 'My Session' };
    const result = migrateSessionData(data, FALLBACK_CWD);
    expect(result.agents).toEqual([agent]);
  });

  it('forwards activeAgentId', () => {
    const agent = makeAgent();
    const data = { agents: [agent], activeAgentId: agent.id, name: 'My Session' };
    const result = migrateSessionData(data, FALLBACK_CWD);
    expect(result.activeAgentId).toBe(agent.id);
  });

  it('forwards the session name', () => {
    const data = { agents: [makeAgent()], activeAgentId: 'agent-1', name: 'Deep Work' };
    const result = migrateSessionData(data, FALLBACK_CWD);
    expect(result.name).toBe('Deep Work');
  });

  it('defaults name to "Default" when absent', () => {
    const data = { agents: [makeAgent()], activeAgentId: 'agent-1' };
    const result = migrateSessionData(data, FALLBACK_CWD);
    expect(result.name).toBe('Default');
  });

  it('defaults activeAgentId to empty string when absent', () => {
    const data = { agents: [makeAgent()] };
    const result = migrateSessionData(data, FALLBACK_CWD);
    expect(result.activeAgentId).toBe('');
  });

  it('supports an empty agents array (user cleared all agents)', () => {
    const data = { agents: [], activeAgentId: '' };
    const result = migrateSessionData(data, FALLBACK_CWD);
    expect(result.agents).toEqual([]);
  });

  it('supports multiple agents', () => {
    const a1 = makeAgent('a1', 'first');
    const a2 = makeAgent('a2', 'second');
    const data = { agents: [a1, a2], activeAgentId: 'a2' };
    const result = migrateSessionData(data, FALLBACK_CWD);
    expect(result.agents).toHaveLength(2);
    expect(result.activeAgentId).toBe('a2');
  });
});

// ── 2. Tabs-only (legacy flat workspace with tabs, no agents) ─────────────────

describe('tabs-only legacy format', () => {
  it('wraps tabs into a single migrated agent', () => {
    const tab = makeTab();
    const data = { tabs: [tab], activeTabId: tab.id, name: 'Old Session' };
    const result = migrateSessionData(data, FALLBACK_CWD);
    expect(result.agents).toHaveLength(1);
    expect(result.agents[0].tabs).toEqual([tab]);
  });

  it('assigns the fallback cwd to the migrated agent', () => {
    const data = { tabs: [makeTab()], activeTabId: 't1' };
    const result = migrateSessionData(data, FALLBACK_CWD);
    expect(result.agents[0].cwd).toBe(FALLBACK_CWD);
  });

  it('uses data.name as agent name ("Imported" fallback)', () => {
    const data = { tabs: [makeTab()], name: 'Sprint 12' };
    const result = migrateSessionData(data, FALLBACK_CWD);
    expect(result.agents[0].name).toBe('Sprint 12');
  });

  it('falls back to "Imported" when name is absent', () => {
    const data = { tabs: [makeTab()] };
    const result = migrateSessionData(data, FALLBACK_CWD);
    expect(result.agents[0].name).toBe('Imported');
  });

  it('preserves data.activeTabId on the migrated agent', () => {
    const tab = makeTab('my-tab');
    const data = { tabs: [tab], activeTabId: 'my-tab' };
    const result = migrateSessionData(data, FALLBACK_CWD);
    expect(result.agents[0].activeTabId).toBe('my-tab');
  });

  it('falls back to first tab id when activeTabId is absent', () => {
    const tab = makeTab('first-tab');
    const data = { tabs: [tab] };
    const result = migrateSessionData(data, FALLBACK_CWD);
    expect(result.agents[0].activeTabId).toBe('first-tab');
  });

  it('sets activeAgentId to the migrated agent id', () => {
    const data = { tabs: [makeTab()] };
    const result = migrateSessionData(data, FALLBACK_CWD);
    expect(result.activeAgentId).toBe(result.agents[0].id);
  });

  it('defaults session name to "Default" when data.name is absent', () => {
    const data = { tabs: [makeTab()] };
    const result = migrateSessionData(data, FALLBACK_CWD);
    expect(result.name).toBe('Default');
  });

  it('preserves multiple tabs', () => {
    const t1 = makeTab('t1', 'Tab 1');
    const t2 = makeTab('t2', 'Tab 2');
    const data = { tabs: [t1, t2], activeTabId: 't1' };
    const result = migrateSessionData(data, FALLBACK_CWD);
    expect(result.agents[0].tabs).toHaveLength(2);
    expect(result.agents[0].tabs[0].id).toBe('t1');
    expect(result.agents[0].tabs[1].id).toBe('t2');
  });

  it('prefers data.tabs over data.panes when both are present', () => {
    const tab = makeTab('tab-wins');
    const pane = makePane('pane-loses');
    // Both present: tabs wins.
    const data = { tabs: [tab], panes: [pane] };
    const result = migrateSessionData(data, FALLBACK_CWD);
    // Should have a single agent whose tab id is 'tab-wins', not 'tab-pane-loses'
    expect(result.agents[0].tabs[0].id).toBe('tab-wins');
  });
});

// ── 3. Panes-only (legacy flat workspace with panes, no tabs / agents) ────────

describe('panes-only legacy format', () => {
  it('promotes each pane to its own tab', () => {
    const pane = makePane('pane-1', 'Terminal');
    const data = { panes: [pane] };
    const result = migrateSessionData(data, FALLBACK_CWD);
    expect(result.agents).toHaveLength(1);
    expect(result.agents[0].tabs).toHaveLength(1);
    // The synthesised tab id uses the pane id.
    expect(result.agents[0].tabs[0].id).toBe('tab-pane-1');
  });

  it('each synthesised tab contains exactly that pane', () => {
    const pane = makePane('pane-1', 'Claude');
    const data = { panes: [pane] };
    const result = migrateSessionData(data, FALLBACK_CWD);
    const tab = result.agents[0].tabs[0];
    expect(tab.panes).toHaveLength(1);
    expect(tab.panes[0]).toEqual(pane);
    expect(tab.activePaneId).toBe('pane-1');
  });

  it('uses pane title as the synthesised tab title', () => {
    const pane = makePane('p1', 'My Browser');
    const data = { panes: [pane] };
    const result = migrateSessionData(data, FALLBACK_CWD);
    expect(result.agents[0].tabs[0].title).toBe('My Browser');
  });

  it('handles multiple panes — one tab per pane', () => {
    const p1 = makePane('p1', 'Claude');
    const p2 = makePane('p2', 'Browser');
    const data = { panes: [p1, p2] };
    const result = migrateSessionData(data, FALLBACK_CWD);
    expect(result.agents[0].tabs).toHaveLength(2);
  });

  it('assigns fallback cwd to the migrated agent', () => {
    const data = { panes: [makePane()] };
    const result = migrateSessionData(data, FALLBACK_CWD);
    expect(result.agents[0].cwd).toBe(FALLBACK_CWD);
  });

  it('sets activeAgentId to the migrated agent id', () => {
    const data = { panes: [makePane()] };
    const result = migrateSessionData(data, FALLBACK_CWD);
    expect(result.activeAgentId).toBe(result.agents[0].id);
  });

  it('uses data.name as agent name ("Imported" fallback)', () => {
    const data = { panes: [makePane()], name: 'Pane Session' };
    const result = migrateSessionData(data, FALLBACK_CWD);
    expect(result.agents[0].name).toBe('Pane Session');
  });

  it('falls back to "Imported" as agent name when name is absent', () => {
    const data = { panes: [makePane()] };
    const result = migrateSessionData(data, FALLBACK_CWD);
    expect(result.agents[0].name).toBe('Imported');
  });
});

// ── 4. Neither / null / empty data ────────────────────────────────────────────

describe('neither / null / empty data', () => {
  it('returns an empty agents array for null data', () => {
    const result = migrateSessionData(null, FALLBACK_CWD);
    expect(result.agents).toEqual([]);
  });

  it('returns empty activeAgentId for null data', () => {
    const result = migrateSessionData(null, FALLBACK_CWD);
    expect(result.activeAgentId).toBe('');
  });

  it('returns "Default" name for null data', () => {
    const result = migrateSessionData(null, FALLBACK_CWD);
    expect(result.name).toBe('Default');
  });

  it('returns empty agents for an empty object', () => {
    const result = migrateSessionData({}, FALLBACK_CWD);
    expect(result.agents).toEqual([]);
  });

  it('returns empty agents when agents key is undefined and no tabs/panes', () => {
    const result = migrateSessionData({ name: 'No content' }, FALLBACK_CWD);
    expect(result.agents).toEqual([]);
  });

  it('returns empty agents for empty tabs array (length 0)', () => {
    const result = migrateSessionData({ tabs: [] }, FALLBACK_CWD);
    expect(result.agents).toEqual([]);
  });

  it('returns empty agents for empty panes array (length 0)', () => {
    const result = migrateSessionData({ panes: [] }, FALLBACK_CWD);
    expect(result.agents).toEqual([]);
  });
});
