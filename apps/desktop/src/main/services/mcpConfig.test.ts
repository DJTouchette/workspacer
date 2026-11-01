import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const state = vi.hoisted(() => ({ userData: '' }));
vi.mock('electron', () => ({ app: { getPath: () => state.userData } }));

import {
  SCRATCH_NOTE,
  facadeSessionMcpConfig,
  facadeSpawnArgs,
  managedFacadeInstructions,
} from './mcpConfig';

describe('automatic Workspacer facade config', () => {
  beforeEach(() => {
    state.userData = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-mcp-config-'));
  });

  afterEach(() => fs.rmSync(state.userData, { recursive: true, force: true }));

  it('merges selected Library MCP servers with the authenticated facade', () => {
    const file = facadeSessionMcpConfig('session-1', 'token-1', [
      { id: 'docs', mcp: { command: 'docs-server', args: ['--stdio'] } },
    ]);
    const config = JSON.parse(fs.readFileSync(file, 'utf8')) as {
      mcpServers: Record<string, Record<string, unknown>>;
    };
    expect(config.mcpServers.workspacer).toMatchObject({
      type: 'http',
      headers: { Authorization: 'Bearer token-1' },
    });
    expect(config.mcpServers.docs).toEqual({ command: 'docs-server', args: ['--stdio'] });
  });

  it('points operator sessions at the project scratch directory on both launch paths', () => {
    expect(SCRATCH_NOTE).toContain('$WORKSPACER_SCRATCH');
    expect(SCRATCH_NOTE).toContain('list_scratch');
    expect(facadeSpawnArgs({ sessionId: 's1', token: 't1' }).appendSystemPrompt).toContain(
      SCRATCH_NOTE,
    );
    expect(managedFacadeInstructions({ sessionId: 's1' })).toContain(SCRATCH_NOTE);
    // The scratch tools are operator tier; lower tiers are not told about them.
    expect(managedFacadeInstructions({ scope: 'triage' })).not.toContain(SCRATCH_NOTE);
    expect(managedFacadeInstructions({ scope: 'view' })).not.toContain(SCRATCH_NOTE);
  });
});
