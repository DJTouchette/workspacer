/**
 * `usage.pollOnBoot` has to reach claudemon as environment.
 *
 * The daemon owns the account-usage poller and has no config file of its own:
 * it decides at boot, from WORKSPACER_USAGE_POLL_ON_BOOT, whether an idle
 * machine discovers the accounts with nothing running. If this binding broke,
 * the Settings checkbox would write config.yaml and change nothing — the
 * failure this fleet sees most often.
 *
 * The variable is written in BOTH directions on purpose. This process respawns
 * the daemon after a crash, so a variable only ever added when off would leave
 * a stale 0 behind after the user turned it back on.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';

const config: { usage?: { pollOnBoot?: unknown } } = {};

vi.mock('./configService', () => ({
  configService: { getConfig: () => config },
  getConfigDir: () => '/tmp/wks-test-config',
}));
vi.mock('electron', () => ({
  app: { getPath: () => '/tmp', getAppPath: () => '/tmp/app', isPackaged: false },
}));
vi.mock('./systemNotice', () => ({ notifySystem: () => {} }));

async function usagePollEnv() {
  vi.resetModules();
  return (await import('./claudemonDaemon')).usagePollEnv();
}

describe('usagePollEnv', () => {
  beforeEach(() => {
    delete config.usage;
  });

  it('says 1 when the config says nothing — absent means on', async () => {
    expect(await usagePollEnv()).toEqual({ WORKSPACER_USAGE_POLL_ON_BOOT: '1' });
    config.usage = {};
    expect(await usagePollEnv()).toEqual({ WORKSPACER_USAGE_POLL_ON_BOOT: '1' });
  });

  it('says 0 only when the user explicitly turned it off', async () => {
    config.usage = { pollOnBoot: false };
    expect(await usagePollEnv()).toEqual({ WORKSPACER_USAGE_POLL_ON_BOOT: '0' });
  });

  it('says 1 again the moment it is switched back on', async () => {
    config.usage = { pollOnBoot: true };
    expect(await usagePollEnv()).toEqual({ WORKSPACER_USAGE_POLL_ON_BOOT: '1' });
  });

  it('treats a non-boolean value as on rather than as off', async () => {
    // Only an explicit `false` is off, on both sides of the wire: the daemon's
    // own parser reads anything but 0/false/off/no as on.
    for (const junk of [undefined, null, 'false', 0, '']) {
      config.usage = { pollOnBoot: junk };
      expect(await usagePollEnv()).toEqual({ WORKSPACER_USAGE_POLL_ON_BOOT: '1' });
    }
  });
});
