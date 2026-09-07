/** Fleet policy is data, never authority. Independent of Claude artifact workflows. */
import { TASK_STAGES, type TaskStage } from './dispatchHistory';
export const WORKFLOW_ROLES = [
  'mechanical',
  'complex_fixer',
  'scout',
  'diagnostician',
  'implementer',
  'reviewer',
  'deep_reviewer',
  'fixer',
  'validator',
  'judge',
  'supervisor',
] as const;
export const WORKFLOW_KINDS = [
  'research',
  'implement',
  'review',
  'repair',
  'validate',
  'land',
] as const;
export type WorkflowStep = {
  id: string;
  label: string;
  kind: (typeof WORKFLOW_KINDS)[number];
  stage: TaskStage;
  role: (typeof WORKFLOW_ROLES)[number];
  when: 'always' | 'material_risk';
  template: string;
  instructions: string;
  independentOf?: string;
  repairOf?: string;
};
export type WorkflowDefinition = {
  id: string;
  revision: number;
  name: string;
  description: string;
  enabled: boolean;
  steps: WorkflowStep[];
};
export type WorkflowTemplate = {
  id: string;
  body: string;
  resultSchema: Record<string, unknown>;
  params: { name: string; required: boolean; default?: string }[];
};
export type WorkflowPin = {
  definition: WorkflowDefinition;
  hash: string;
  templates: Record<string, WorkflowTemplate>;
  steps: WorkflowStepRun[];
};
export type WorkflowStepRun = {
  id: string;
  state: 'planned' | 'dispatched' | 'skipped' | 'blocked' | 'failed' | 'completed';
  dispatchId?: string;
  sessionId?: string;
  reason?: string;
  decision?: boolean;
  outcome?: unknown;
};
export type WorkflowRequest = {
  op:
    | 'list'
    | 'get'
    | 'validate'
    | 'create'
    | 'update'
    | 'clone'
    | 'disable'
    | 'delete'
    | 'select'
    | 'start'
    | 'next'
    | 'decide';
  id?: string;
  expectedRevision?: number;
  definition?: WorkflowDefinition;
  name?: string;
  cwd?: string;
  workflowId?: string | null;
  taskId?: string;
  title?: string;
  stepId?: string;
  run?: boolean;
  reason?: string;
};
export type WorkflowCatalog = {
  available: true;
  definitions: WorkflowDefinition[];
  templates: WorkflowTemplate[];
  defaultId: string;
  projects: Record<string, string>;
  selectionRevision: number;
};
export type WorkflowResponse =
  | {
      ok: true;
      catalog?: WorkflowCatalog;
      definition?: WorkflowDefinition;
      task?: import('./dispatchHistory').DispatchTask;
      instructions?: string;
    }
  | { ok: false; code: string; error: string; currentRevision?: number };
export const DEFAULT_WORKFLOW_ID = 'scout-implement-review';
export const reviewPolicy = (d: WorkflowDefinition): string =>
  d.steps.some((s) => s.kind === 'review')
    ? 'Independent review required'
    : 'Independent review omitted by selected policy';
const slug = /^[a-z][a-z0-9-]{0,63}$/;
function keys(v: Record<string, unknown>, allowed: string[]): void {
  if (!v || typeof v !== 'object' || Array.isArray(v)) throw new Error('Expected an object');
  for (const k of Object.keys(v))
    if (!allowed.includes(k)) throw new Error(`Unknown workflow field: ${k}`);
}
function text(v: unknown, max: number, empty = false): void {
  if (typeof v !== 'string' || v.length > max || (!empty && !v.trim()))
    throw new Error(`Expected ${empty ? 'optional' : 'nonempty'} text, at most ${max} characters`);
}
export function validateWorkflow(value: unknown): WorkflowDefinition {
  const d = value as WorkflowDefinition;
  keys(d as unknown as Record<string, unknown>, [
    'id',
    'revision',
    'name',
    'description',
    'enabled',
    'steps',
  ]);
  if (!slug.test(d.id) || !Number.isSafeInteger(d.revision) || d.revision < 1)
    throw new Error('Invalid workflow id/revision');
  text(d.name, 120);
  text(d.description, 1000, true);
  if (
    typeof d.enabled !== 'boolean' ||
    !Array.isArray(d.steps) ||
    d.steps.length < 1 ||
    d.steps.length > 8
  )
    throw new Error('Workflow needs enabled and 1–8 steps');
  const seen = new Map<string, WorkflowStep>();
  const repairs = new Set<string>();
  for (const s of d.steps) {
    keys(s as unknown as Record<string, unknown>, [
      'id',
      'label',
      'kind',
      'stage',
      'role',
      'when',
      'template',
      'instructions',
      'independentOf',
      'repairOf',
    ]);
    if (!slug.test(s.id) || seen.has(s.id)) throw new Error('Step ids must be unique slugs');
    text(s.label, 120);
    text(s.instructions, 8000, true);
    text(s.template, 128);
    if (
      !WORKFLOW_KINDS.includes(s.kind) ||
      !TASK_STAGES.includes(s.stage) ||
      !WORKFLOW_ROLES.includes(s.role) ||
      !['always', 'material_risk'].includes(s.when)
    )
      throw new Error('Unknown step kind, stage, role or condition');
    if (s.kind === 'review' && s.when !== 'always')
      throw new Error('A configured independent review is required');
    if (
      s.independentOf &&
      (!['implement', 'repair'].includes(seen.get(s.independentOf)?.kind ?? '') ||
        s.kind !== 'review')
    )
      throw new Error('Independent review must reference an earlier implementation/repair');
    if (
      s.repairOf &&
      (s.kind !== 'repair' || seen.get(s.repairOf)?.kind !== 'review' || repairs.has(s.repairOf))
    )
      throw new Error('Repair must reference one earlier review, at most once per review');
    if (s.repairOf) repairs.add(s.repairOf);
    seen.set(s.id, s);
  }
  if (JSON.stringify(d).length > 64 * 1024) throw new Error('Workflow exceeds 64 KiB');
  return structuredClone(d);
}
const step = (
  id: string,
  kind: WorkflowStep['kind'],
  stage: TaskStage,
  role: WorkflowStep['role'],
  template: string,
): WorkflowStep => ({
  id,
  label: id,
  kind,
  stage,
  role,
  template,
  when: 'always',
  instructions: '',
});
export const WORKFLOW_STARTERS: WorkflowDefinition[] = [
  {
    id: DEFAULT_WORKFLOW_ID,
    revision: 1,
    name: 'Scout → implement → independent review',
    description: 'Scout when material architecture, security or compatibility risk warrants it.',
    enabled: true,
    steps: [
      { ...step('scout', 'research', 'scout', 'scout', 'scout-task'), when: 'material_risk' },
      step('implement', 'implement', 'implement', 'implementer', 'ship-task'),
      {
        ...step('review', 'review', 'review', 'reviewer', 'review-task'),
        independentOf: 'implement',
      },
    ],
  },
  {
    id: 'direct-implementation',
    revision: 1,
    name: 'Direct implementation',
    description: 'Explicit policy: implementation without independent review.',
    enabled: true,
    steps: [step('implement', 'implement', 'implement', 'implementer', 'ship-task')],
  },
];
export const WORKFLOW_DISCOVERY =
  'For each NEW project task, call start_workflow with cwd and title BEFORE dispatching. The host resolves the current selected policy and pins its revision/templates. Call next_workflow_step for its instructions; decide_workflow_step records conditional decisions with reasons. Copy its step metadata into select_model and spawn_agent. Existing tasks keep their pinned policy. If these tools are unavailable, report Fleet workflows unavailable on this headless/older host; never claim a workflow ran. Do not retrofit historical task IDs.';
