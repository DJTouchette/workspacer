/** Headless adapter for the same durable manager succession transaction. */
import { setManagerViewerBindings, managerViewerBound } from '../services/managerViewerBindings';
import { randomUUID } from 'node:crypto';
import { hostCall } from './hostBridge';
import { ManagerReplacementService } from '../services/managerReplacementService';
import { managerReplacementState as state, type ManagerLaunch, type ReplacementMetadata } from '../services/managerReplacementState';
import { managerLaunchConfiguration } from '../services/managerLaunchConfiguration';
import { sessionFacadeGrantFingerprint } from '../services/remoteTokens';
import { dispatchHistoryStore } from '../services/dispatchHistoryStore';
import { managerRequests } from '../services/managerRequestService';
import { configService } from '../services/configService';
import { buildManagerKickoff } from '../shared/managerDoctrine';
import { normalizeModelSelection } from '../shared/modelContextWindows';
import { ManagerDeliveryRejected, type ManagerReplacementRequest, type ReplacementDelivery } from '../shared/managerReplacement';

type Row = Record<string, any>;
let snapshots: Row[] = [];
const inFlight = new Map<string,{id:string;target:string;text:string;sourceRequest?:ReplacementDelivery['sourceRequest']}>();
let ready = false;
const snapshot = (id:string) => snapshots.find(s=>s.sessionId===id);
const fullAccess = () => configService.getConfig().agents?.fleetFullAccess === true || Object.values(configService.getConfig().projects ?? {}).some(p=>p.yolo === true);
async function refresh(receiptSessionId?:string): Promise<string> {
  const result = await hostCall('replacement.refresh',{receiptSessionId});
  snapshots=result.snapshots;
  return result.receipt ?? '';
}
function source(id:string,paneId?:string):ManagerLaunch {
  const row = snapshot(id), recorded=state.launch(id),grants=sessionFacadeGrantFingerprint(id);
  if (!row || row.status==='ended' || row.hub || !row.isWakeTarget || row.transport!=='stream' || !recorded || !grants || (paneId && !managerViewerBound(paneId,id))) throw new Error('Manager launch, live operator identity, and attached viewer are required');
  if (recorded.options.launchIntegrationId) throw new Error('Automatic manager replacement is unavailable for custom launch integrations');
  if (recorded.configuration!==managerLaunchConfiguration(recorded.options)) throw new Error('Manager profile configuration changed; replacement cannot reproduce this launch');
  const model=row.requestedSelection?.model || row.settings?.model || recorded.options.model || row.usage?.model;
  if(!model)throw new Error('Manager model is not recorded yet; wait for its first configured turn');
  const selection=normalizeModelSelection(model,row.requestedSelection ? row.requestedSelection.contextWindow : row.settings?.contextWindow !== undefined ? row.settings.contextWindow : recorded.options.contextWindow);
  const permissionMode=row.livePermissionMode ?? row.settings?.permissionMode ?? recorded.options.permissionMode;
  return {...recorded,grants,options:{...recorded.options,cwd:row.cwd,label:row.label ?? recorded.options.label,parentSessionId:row.parentSessionId,provider:row.provider,manager:true,toolScope:'operator',model:selection.model,modelIdentity:selection.model,contextWindow:selection.contextWindow,effort:row.liveEffort ?? row.statusLine?.effort ?? recorded.options.effort,permissionMode,skipPermissions:permissionMode==='yolo'||permissionMode==='bypassPermissions'}};
}
function inventory(id:string):ReplacementMetadata[] {
  return snapshots.filter(s=>!s.hub&&(s.sessionId===id||s.parentSessionId===id)).map(s=>({sessionId:s.sessionId,cwd:s.cwd,label:s.label,parentSessionId:s.parentSessionId,isWakeTarget:s.isWakeTarget,provider:s.provider,transport:s.transport,settings:s.settings,resultSchema:s.resultSchema,routing:s.routing}));
}
async function send(id:string,text:string,sourceRequest?:ReplacementDelivery['sourceRequest']) {
  if (sourceRequest) {
    const request=managerRequests().request(id,sourceRequest.requestId);
    const attempt=request.attempts.find(a=>a.deliveryId===sourceRequest.deliveryId);
    if(!attempt)throw new Error('Request delivery attempt unavailable');
    if(attempt.status==='accepted'||request.intents)return {ok:true};
    if(attempt.status==='unknown')throw new Error('Unknown request delivery must not replay');
    if(request.userContent===undefined)throw new Error('Unresolved request content unavailable');
    text=request.bootstrap ? buildManagerKickoff(request.userContent,fullAccess()) : request.userContent;
    managerRequests().finishDelivery(sourceRequest.requestId,sourceRequest.deliveryId,'unknown');
  }
  const result = await hostCall('replacement.send',{sessionId:id,text});
  if (sourceRequest) managerRequests().finishDelivery(sourceRequest.requestId,sourceRequest.deliveryId,result.status);
  if(result.status==='rejected')throw new ManagerDeliveryRejected(409);
  if(result.status!=='accepted')throw new Error('Daemon message acknowledgement is unknown');
  return {ok:true};
}
const service = new ManagerReplacementService(state,{
  source,inventory,
  tasks:id=>dispatchHistoryStore.list().filter(t=>t.ownerSessionId===id).map(t=>t.taskId),
  projects:id=>[...new Set(dispatchHistoryStore.list().filter(t=>t.ownerSessionId===id).map(t=>t.projectCwd))],
  readyForTransfer:id=>!dispatchHistoryStore.list().some(t=>t.ownerSessionId===id&&t.dispatchReservation),
  signatures:()=>({}),
  inFlightMessages:id=>[...inFlight.values()].filter(m=>m.target===id),
  finishes:()=>({}), // All Go finish wakes pass through the durable message outbox below.
  receipt:refresh,
  settled:id=>{const s=snapshot(id);return !!s&&s.status!=='ended'&&s.ambientState==='idle'&&!s.pendingApproval&&!s.pendingQuestions&&!s.activeToolCalls?.length;},
  async spawn(id,launch){
    await hostCall('replacement.spawn',{sessionId:id,sourceSessionId:state.related(id)?.sourceSessionId,launch});
    await refresh();
  },
  async validateSuccessor(id,launch){
    await refresh();
    const s=snapshot(id), actual=state.launch(id)?.options,op=state.related(id);
    if(!s||s.status==='ended'||!s.isWakeTarget||s.cwd!==launch.options.cwd||s.provider!==launch.options.provider||s.hub||sessionFacadeGrantFingerprint(id)!==launch.grants||managerLaunchConfiguration(launch.options)!==launch.configuration)throw new Error('Successor identity, liveness or grants do not match source');
    if(op&&!op.transferIntent&&s.user_prompts!==0)throw new Error('Successor must report zero user prompts before transfer');
    if(!actual||actual.effort!==launch.options.effort||actual.permissionMode!==launch.options.permissionMode||JSON.stringify(normalizeModelSelection(actual.model!,actual.contextWindow))!==JSON.stringify(normalizeModelSelection(launch.options.model!,launch.options.contextWindow)))throw new Error('Successor launch selection changed');
  },
  async transfer(oldId,newId,operationId){
    const op=state.get(operationId);
    if(op.sourceSessionId!==oldId||op.successorSessionId!==newId||!op.transferIntent)throw new Error('Missing durable ownership intent');
    // The journal makes this a roll-forward transaction across the two stores.
    // A lost ACK leaves recovery-required; it never rolls task ownership back.
    dispatchHistoryStore.adoptWorkflowTasks(oldId,newId);
    await hostCall('replacement.transfer',{source:oldId,successor:newId,metadata:op.metadata,launch:op.launch});
    await refresh();
  },
  async restore(metadata){await hostCall('replacement.restore',{metadata});await refresh();},
  bound:(paneId,id)=>managerViewerBound(paneId,id),
  send,
  retryRequest:(target,requestId)=>{
    const inbox=managerRequests();if(inbox.request(target,requestId).delivery!=='rejected')return;
    const next=inbox.beginDelivery(target,requestId);return next?{requestId,deliveryId:next.deliveryId}:undefined;
  },
  pause:async id=>{await hostCall('replacement.signal',{sessionId:id,signal:'SIGINT'});},
  close:async id=>{await hostCall('replacement.close',{sessionId:id});},
  kickoff:op=>buildManagerKickoff(`HOST-OWNED MANAGER HANDOFF ${op.operationId}. Your fresh manager session is ${op.successorSessionId}; predecessor ${op.sourceSessionId} is audit history only. The host committed worker AND task ownership. Do not adopt workers, resume or terminate the predecessor, or use a shared handoff.md. Use this validated handoff (retained at ${op.sealedArtifactPath ?? op.artifactPath}, SHA-256 ${op.artifactHash}). Preserve pending decisions; take the stated next action within existing authority.\n${op.artifact}`,fullAccess()),
  recoverFinishes:()=>{}, // Saved completion deliveries are already journaled by holdMessage.
  async flushFinishes(ids){
    const deadline=Date.now()+10_000;
    while([...inFlight.values()].some(m=>ids.includes(m.target))){if(Date.now()>deadline)throw new Error('Message acknowledgements are still in flight');await new Promise(r=>setTimeout(r,25));}
  },
});
export async function ensureReplacementReady(): Promise<void> {
  if(ready)return;
  state.enableProjection();
  if(state.recoveryMetadata().length){await refresh();await service.initialize();}
  ready=true;
}
export async function replacementRequest(request:ManagerReplacementRequest,bindings:unknown) {
  setManagerViewerBindings(bindings);
  await ensureReplacementReady();
  await refresh();
  const result=await service.request(request);ready=true;return result;
}
export function rememberManager(sessionId:string,options:ManagerLaunch['options']) {
  const grants=sessionFacadeGrantFingerprint(sessionId);
  if(!grants)throw new Error('Manager operator grant was not recorded');
  state.rememberLaunch(sessionId,{options,grants,configuration:managerLaunchConfiguration(options)});
}
export function routeReplacementMessage(sessionId:string,text:string,sourceRequest?:ReplacementDelivery['sourceRequest']) {
  // Initialize recovery before messages can move through a saved lineage.
  if(!ready&&state.records().length)throw new Error('Manager recovery must be reconciled before message delivery');
  const target=state.wakeTarget(sessionId);
  if(state.holdMessage(target,text,[],sourceRequest))return {target,held:true};
  const id=randomUUID();inFlight.set(id,{id,target,text,sourceRequest});return {target,id,held:false};
}
export function finishReplacementMessage(id:string,status:string) {
  const entry=inFlight.get(id);if(!entry)return;
  state.noteInFlightMessage(entry.target,id,status==='accepted',status==='accepted'?undefined:'Daemon acknowledgement '+status,status==='rejected');
  inFlight.delete(id);
}
