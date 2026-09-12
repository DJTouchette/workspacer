// Desktop adapter; the same core is hosted by the headless brain.
import { claudeSessionStore } from './claudeSessionStore';
import { createFleetWorkflowRuntime } from './fleetWorkflowCore';
export const { workflowBusy, ownerTask, workflowInstructions, workflowDispatchPlan, workflowSpawn, pinnedWorkflowTemplate, workflowWakeInstructions, workflowResultSchema, configuredWorkflowProjectKey, missingWorkflowEvidence } = createFleetWorkflowRuntime((id) => claudeSessionStore.getSnapshot(id) ?? undefined);
