import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { PluginPermissions } from '../src/components/plugin/PluginPermissions';
import type { PluginManifest } from '../src/types/plugin';

function mf(partial: Partial<PluginManifest>): PluginManifest {
  return { id: 'p', name: 'P', apiVersion: '1', ...partial };
}

describe('<PluginPermissions>', () => {
  it('renders grouped, human-readable lines with scopes into the DOM', () => {
    // The rules-engine shape: a sensitive emit + capabilities + consumes.
    render(
      <PluginPermissions
        manifest={mf({
          capabilities: [{ method: 'fs.write', paths: ['${agentCwd}'] }, 'agents.spawn'],
          emits: ['command.*', 'rules.fired'],
          consumes: ['agent.state_changed'],
        })}
      />,
    );

    // Group headers.
    expect(screen.getByText('Can')).toBeTruthy();
    expect(screen.getByText('Publishes events')).toBeTruthy();
    expect(screen.getByText('Receives events')).toBeTruthy();

    // Plain-English capability labels + a rendered scope.
    expect(screen.getByText('Write & change files')).toBeTruthy();
    expect(screen.getByText(/in the agent's folder/)).toBeTruthy();
    expect(screen.getByText('Spawn new agents')).toBeTruthy();

    // The app-driving emit is surfaced verbatim with its warning detail.
    expect(screen.getByText('command.*')).toBeTruthy();
    expect(screen.getByText(/can drive the app/)).toBeTruthy();
  });

  it('shows a friendly line when a plugin requests no bus access', () => {
    render(<PluginPermissions manifest={mf({ panes: [{ type: 't', title: 'T' }] })} />);
    expect(screen.getByText(/no bus access/i)).toBeTruthy();
  });
});
