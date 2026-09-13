import type {
  IntentExecution,
  IntentSessionRef,
  IntentWorkspace,
  IntentWorkspaceRequest,
  IntentWorkspaceResponse,
} from '../../../main/shared/intentWorkspace';

export type IntentRequest = (request: IntentWorkspaceRequest) => Promise<IntentWorkspaceResponse>;
export interface IntentLaunchTarget {
  workspace: IntentWorkspace;
  executionId: string;
}

/** A launch claim is persisted before spawn. Never turn an uncertain spawn or
 * a failed association write into a retry that creates another session.
 */
export async function launchIntentExecution(
  target: IntentLaunchTarget,
  task: string,
  request: IntentRequest,
  spawn: (
    message: string,
    onReady: (session: IntentSessionRef) => Promise<void>,
  ) => Promise<unknown>,
): Promise<{ execution: IntentExecution; warning?: string }> {
  const prepared = await request({
    action: 'prepareExecution',
    id: target.workspace.id,
    expectedRevision: target.workspace.revision,
    executionId: target.executionId,
    task,
  });
  if (prepared.action !== 'prepareExecution')
    throw new Error('This host does not support intent execution.');
  if (!prepared.created)
    return {
      execution: prepared.execution,
      warning:
        'This launch was already recorded. Check its linked session or reconcile the attempt before starting another agent.',
    };
  let execution = prepared.execution;
  let readySession: IntentSessionRef | undefined;
  let linkFailed = false;
  try {
    await spawn(execution.contextPacket!, async (session) => {
      readySession = session;
      try {
        const linked = await request({
          action: 'linkExecution',
          id: execution.workspaceId,
          executionId: execution.id,
          session,
        });
        if (linked.action !== 'linkExecution') throw new Error('Unexpected link response');
        execution = linked.execution;
      } catch {
        // The session exists. Do not throw through spawnAgent's success path.
        linkFailed = true;
      }
    });
  } catch {
    if (!readySession) {
      try {
        const result = await request({
          action: 'markExecutionUnknown',
          id: execution.workspaceId,
          executionId: execution.id,
        });
        if (result.action === 'markExecutionUnknown') execution = result.execution;
      } catch {
        /* The original launch claim remains durable and unresolved. */
      }
      return {
        execution,
        warning:
          'Launch outcome is unconfirmed. Check existing agents and link the matching session before starting another agent.',
      };
    }
  }
  if (linkFailed || !readySession)
    return {
      execution,
      warning: readySession
        ? `Agent ${readySession.label} started, but its workspace link could not be saved. Use “Link to this attempt” to record it; do not dispatch it again.`
        : 'Launch outcome is unconfirmed. Check existing agents before starting another agent.',
    };
  return { execution };
}
