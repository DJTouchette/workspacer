import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const state = vi.hoisted(() => ({ userData: '' }));
vi.mock('electron', () => ({ app: { getPath: () => state.userData } }));

import { facadeSessionMcpConfig } from './mcpConfig';

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
});
