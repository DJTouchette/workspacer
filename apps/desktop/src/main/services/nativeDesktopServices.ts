/** Browser owner requests hosted by a running native desktop. The headless
 * process uses the same service implementations with its own lifecycle source.
 */
import { reconcileSessionFacadeToken } from './remoteTokens';
import { desiredSessionGrants } from './fullAccessGrants';
import { requestManagerReplacement } from './managerReplacement';
import { setManagerViewerBindings } from './managerViewerBindings';
import { desktopHostCall, configureNativeDesktopRuntime, type HostContext } from '../headless/desktopHost';
import * as workflow from './fleetWorkflowRuntime';
import * as workflows from './fleetWorkflowService';
import { claudeSessionStore } from './claudeSessionStore';
import { providerReadinessService } from './providerReadinessRuntime';
import { readAgentRuntimeStatus } from './agentRuntimeStatus';
import { CLAUDEMON_API_URL } from './claudemonDaemon';
import { managerRequests } from './managerRequestService';
import { managerReplacementState } from './managerReplacementState';
import { claudemonSessionClient } from './claudemonSessionClient';
import { buildManagerKickoff } from '../shared/managerDoctrine';
import { configService } from './configService';
import './briefBoardService'; // installs the native SQLite recent-directory source

configureNativeDesktopRuntime(workflow, workflows, (id) => claudeSessionStore.getSnapshot(id) ?? undefined);

export async function nativeDesktopService(method: string, raw: unknown, context: HostContext): Promise<unknown> {
  const p = (raw ?? {}) as Record<string, unknown>;
  switch (method) {
    case 'desktop.sessionGrantReconcile': {
      const session = typeof p.sessionId === 'string' ? claudeSessionStore.getSnapshot(p.sessionId) : undefined;
      return p.role === 'manager' && session?.isWakeTarget && session.status !== 'ended'
        ? reconcileSessionFacadeToken(session.sessionId, 'manager', desiredSessionGrants().manager) : false;
    }
    case 'desktop.managerReplacement':
      setManagerViewerBindings(p.bindings);
      return requestManagerReplacement(p.request as Parameters<typeof requestManagerReplacement>[0]);
    case 'desktop.agentRuntimeStatus': return readAgentRuntimeStatus();
    case 'desktop.providerReadiness':
      if (typeof p.provider !== 'string') throw new Error('Invalid provider');
      return p.check === true ? providerReadinessService.check(p.provider) : providerReadinessService.read(p.provider);
    case 'desktop.keepWarmHeartbeats': {
      const limit = typeof p.limit === 'number' ? Math.min(200, Math.max(1, Math.floor(p.limit))) : 20;
      const res = await fetch(`${CLAUDEMON_API_URL}/heartbeats?limit=${limit}`, { signal: AbortSignal.timeout(5000) });
      if (!res.ok) throw new Error(`Heartbeat history HTTP ${res.status}`);
      return res.json();
    }
    case 'desktop.managerRequestSend': {
      if (typeof p.sessionId !== 'string' || typeof p.requestId !== 'string' || !p.requestId) throw new Error('Invalid request identity');
      const target = managerReplacementState.wakeTarget(p.sessionId);
      const service = managerRequests();
      const delivery = service.beginDelivery(target, p.requestId);
      if (delivery) {
        try {
          const content = delivery.bootstrap ? buildManagerKickoff(delivery.text, !!configService.getConfig().agents?.fleetFullAccess) : delivery.text;
          await claudemonSessionClient.message(target, content, undefined, { requestId: p.requestId, deliveryId: delivery.deliveryId });
        } catch { /* Return the durable receipt; never replay through another path. */ }
      }
      const receipt = service.request(managerReplacementState.wakeTarget(target), p.requestId);
      return { ok: receipt.delivery === 'accepted' || receipt.delivery === 'pending', requestId: receipt.requestId, delivery: receipt.delivery, mode: receipt.delivery };
    }
    default: return desktopHostCall(method, p, { ...context, daemonURL: CLAUDEMON_API_URL });
  }
}
