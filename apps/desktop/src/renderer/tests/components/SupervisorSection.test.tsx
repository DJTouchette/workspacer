/**
 * Pi core ships no MCP client at all (pi.rs), so `managedSpawn.ts` refuses to
 * mint it a facade token (`provider !== 'pi'`) and `agentSkillsRoot` returns
 * null for it, meaning a Pi supervisor gets role instructions and nothing
 * else: no fleet-observation tools, no /supervise skill. The settings copy
 * used to claim otherwise ("Codex, OpenCode, and Pi supervisors are wired to
 * the workspacer MCP facade … via their own MCP config") and the picker
 * offered Pi as a supervisor harness anyway. MANAGER_PROVIDERS already
 * excludes Pi from the manager picker for the identical reason — this file
 * pins the supervisor picker to the same rule.
 */

import { describe, it, expect, vi } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import React from 'react';
import SupervisorSection from '../../src/components/settings/SupervisorSection';
import type { Config } from '../../src/hooks/useConfig';

function renderSection(config: Partial<Config> = {}) {
  return render(<SupervisorSection config={config as Config} save={vi.fn()} />);
}

describe('SupervisorSection — Pi has no facade access, so it must not be offered or claimed', () => {
  it('does not offer Pi as a supervisor harness', () => {
    renderSection();
    const row = within(screen.getByText('Supervisor agent').closest('div') as HTMLElement);
    expect(row.queryByRole('button', { name: 'Pi' })).not.toBeInTheDocument();
    expect(row.getByRole('button', { name: 'Claude' })).toBeInTheDocument();
    expect(row.getByRole('button', { name: 'Codex' })).toBeInTheDocument();
    expect(row.getByRole('button', { name: 'OpenCode' })).toBeInTheDocument();
  });

  it('does not claim Pi supervisors are wired to the workspacer MCP facade', () => {
    const { container } = renderSection();
    // The false claim this pins: Pi cannot be "wired to the workspacer MCP
    // facade via their own MCP config" the way Codex and OpenCode genuinely
    // are (codex.rs / opencode.rs write real MCP config; pi.rs warns facade
    // tools are unavailable to it).
    expect(container.textContent).not.toMatch(/\bPi\b/);
  });
});
