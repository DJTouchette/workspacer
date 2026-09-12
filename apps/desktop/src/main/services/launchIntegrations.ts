/** Native hub transport for shared launch preparation. */
import { readCodexProvider } from './codexRouting';
import { prepareLaunchIntegrationCore } from './launchIntegrationCore';
export * from './launchIntegrationCore';
async function listIntegrations(): Promise<any[]> {
  const { getHubToken, hubHttpUrl } = await import('./hubDaemon');
  const token = getHubToken();
  const response = await fetch(`${hubHttpUrl()}/plugins`, {
    headers: token ? { Authorization: `Bearer ${token}` } : {},
    signal: AbortSignal.timeout(3000),
  });
  if (!response.ok) throw new Error('Could not read plugins from the hub');
  const list = await response.json();
  if (!Array.isArray(list)) throw new Error('Invalid plugin list');
  return list;
}


export async function prepareLaunchIntegration(
 id: Parameters<typeof prepareLaunchIntegrationCore>[0],
 context: Parameters<typeof prepareLaunchIntegrationCore>[1],
 base: Parameters<typeof prepareLaunchIntegrationCore>[2],
 dependencies: Parameters<typeof prepareLaunchIntegrationCore>[3] = {
   list:listIntegrations,
   call:async (method,params)=>(await import('./hubClient')).callHub(method,params),
   routing:readCodexProvider,
 },
) {return prepareLaunchIntegrationCore(id,context,base,dependencies);}
