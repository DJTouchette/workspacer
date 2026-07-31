/**
 * `agents.binaries.<provider>` has to reach claudemon as environment.
 *
 * claudemon no longer honours a caller-supplied `bin` on GET
 * /providers/:provider/models — that route is reachable cross-origin by any page
 * in the user's browser, so a caller-supplied path was a caller-supplied
 * program. It resolves the launcher itself, reading WKS_<PROVIDER>_BIN first.
 * The Go brain reads the config key directly, so if this binding broke, the two
 * would disagree about which binary a provider means and the model picker would
 * silently run whatever was on PATH.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';

const config: { agents?: { binaries?: Record<string, unknown> } } = {};

vi.mock('./configService', () => ({
  configService: { getConfig: () => config },
  getConfigDir: () => '/tmp/wks-test-config',
}));
vi.mock('electron', () => ({
  app: { getPath: () => '/tmp', getAppPath: () => '/tmp/app', isPackaged: false },
}));
vi.mock('./systemNotice', () => ({ notifySystem: () => {} }));

async function providerBinaryEnv() {
  vi.resetModules();
  return (await import('./claudemonDaemon')).providerBinaryEnv();
}

describe('providerBinaryEnv', () => {
  beforeEach(() => {
    delete config.agents;
  });

  it('maps each configured binary to the daemon env var claudemon reads', async () => {
    config.agents = { binaries: { codex: '/opt/codex/bin/codex', pi: '/usr/local/bin/pi' } };
    expect(await providerBinaryEnv()).toEqual({
      WKS_CODEX_BIN: '/opt/codex/bin/codex',
      WKS_PI_BIN: '/usr/local/bin/pi',
    });
  });

  it('omits the providers left at their default empty string', async () => {
    // config_defaults ships every provider present and empty, so an unset
    // override must not become WKS_CLAUDE_BIN="" — that would shadow PATH
    // resolution with a path that cannot execute.
    config.agents = { binaries: { claude: '', codex: '  ', opencode: '/x/opencode' } };
    expect(await providerBinaryEnv()).toEqual({ WKS_OPENCODE_BIN: '/x/opencode' });
  });

  it('trims, matching how the daemon reads the value', async () => {
    config.agents = { binaries: { codex: '  /opt/codex  ' } };
    expect(await providerBinaryEnv()).toEqual({ WKS_CODEX_BIN: '/opt/codex' });
  });

  it('is empty when nothing is configured', async () => {
    expect(await providerBinaryEnv()).toEqual({});
    config.agents = {};
    expect(await providerBinaryEnv()).toEqual({});
  });

  it('ignores a non-string value rather than stringifying it', async () => {
    config.agents = { binaries: { codex: 42, pi: null } };
    expect(await providerBinaryEnv()).toEqual({});
  });
});
