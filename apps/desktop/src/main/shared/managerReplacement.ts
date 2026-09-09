/** Local desktop only. The web backend deliberately returns unavailable. */
export const MANAGER_REPLACEMENT_UNAVAILABLE =
  'Automatic manager replacement requires an owned local desktop manager with recorded launch settings and only local workers. Remote, headless and older hosts are unavailable.';

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
  phase: ReplacementPhase;
  createdAt: number;
  updatedAt: number;
  committed: boolean;
  bound: boolean;
  artifactPath: string;
  error?: string;
  workerIds: string[];
  taskIds: string[];
  deliveries: ReplacementDelivery[];
}
export type ManagerReplacementRequest =
  | { action: 'list' }
  | { action: 'start'; sourceSessionId: string; paneId: string }
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
