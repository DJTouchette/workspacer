import { configService } from './configService';
import { checkAllProviders } from './agentProviders';
import { completeReadinessPing } from './directCompletion';
import { isRemoteClientMode } from './remoteServer';
import { ProviderReadinessService } from './providerReadiness';

export const providerReadinessService = new ProviderReadinessService({
  context: (selected) => {
    const cfg = configService.getConfig();
    const provider = selected ?? cfg.agents?.managerProvider ?? 'claude';
    const local = !isRemoteClientMode();
    const bin = local
      ? (checkAllProviders(cfg.agents?.binaries).find((row) => row.provider === provider)
          ?.resolvedPath ?? null)
      : null;
    return {
      provider,
      bin,
      local,
      enabled: cfg.agents?.checkProviderOnStartup !== false,
      key: JSON.stringify([local, provider, bin, cfg.agents, cfg.claude, cfg.codex]),
    };
  },
  ping: completeReadinessPing,
});
let started = false;
export function startProviderReadiness(): void {
  if (started) return;
  started = true;
  configService.onChange(() => providerReadinessService.invalidate());
  providerReadinessService.start();
}
