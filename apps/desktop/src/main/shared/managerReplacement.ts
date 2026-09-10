/** Local desktop only. The web backend deliberately returns unavailable. */
export const MANAGER_REPLACEMENT_UNAVAILABLE =
  'Automatic manager replacement requires an owned local desktop manager using stream transport with recorded launch settings and only local workers. Remote, headless and older hosts are unavailable.';

export type ReplacementPhase =
  | 'preparing'
  | 'spawning'
  | 'transferring'
  | 'binding'
  | 'activating'
  | 'complete'
  | 'failed'
  | 'cancelled'
  | 'recovery-required';
export type DeliveryStatus = 'pending' | 'sending' | 'accepted' | 'uncertain' | 'reconciled';
export interface ReplacementDelivery {
  sourceRequest?: { requestId: string; deliveryId: string };
  id: string;
  kind: 'preparation' | 'kickoff' | 'message';
  text: string;
  status: DeliveryStatus;
  error?: string;
}
export interface ManagerReplacementView {
  operationId: string;
  sourceSessionId: string;
  successorSessionId: string;
  paneId: string;
  workspaceId: string;
  phase: ReplacementPhase;
  createdAt: number;
  updatedAt: number;
  committed: boolean;
  bound: boolean;
  artifactPath: string;
  sealedArtifactPath?: string;
  error?: string;
  workerIds: string[];
  taskIds: string[];
  deliveries: ReplacementDelivery[];
}
export type ManagerReplacementRequest =
  | { action: 'list' }
  | { action: 'start'; sourceSessionId: string; paneId: string; workspaceId: string }
  | { action: 'cancel' | 'bind' | 'reconcile'; operationId: string }
  | {
      action: 'resolve-delivery';
      operationId: string;
      deliveryId: string;
      resolution: 'retry' | 'accepted';
      acknowledgeDuplicateRisk: true;
    };
export interface ManagerReplacementResponse {
  available: boolean;
  operations: ManagerReplacementView[];
  error?: string;
}

/** An explicit daemon rejection is distinct from a missing acknowledgement. */
export class ManagerDeliveryRejected extends Error {
  constructor(readonly status: number) {
    super(`Daemon rejected message (HTTP ${status}); it was not accepted`);
  }
}

export class ManagerReplacementUnavailable extends Error {}

/** The task transaction refused before any ownership side effect. */
export class ManagerOwnershipUnchanged extends Error {}
