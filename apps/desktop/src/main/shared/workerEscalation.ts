/**
 * Terminal worker escalation — the fixed, always-available alternative to a
 * normal completion report.
 *
 * Unlike `wks-result`, this contract is host-authored and has one stable
 * shape. It is injected into fleet-dispatched workers (a non-manager spawn
 * carrying authoritative parent metadata), including dispatches with no
 * caller-supplied resultSchema, and is parsed independently so adding
 * escalation cannot change ordinary structured-result behavior.
 */

export const WORKER_ESCALATION_FENCE = 'wks-escalation';

/** Bounded because the validated JSON is copied into a manager wake. */
export const WORKER_ESCALATION_MAX = 4096;

export interface WorkerEscalation {
  type: 'worker-escalation';
  status: 'blocked';
  reason: string;
  requiredAuthorityOrDecision: string;
  changed: boolean;
  nextAction: string;
}

export interface WorkerEscalationRead {
  /** Pretty-printed, validated payload for the wake/result surface. */
  json?: string;
  value?: WorkerEscalation;
  /** A tagged block was present but invalid. Absence is represented by null. */
  error?: string;
}

/** Spawn metadata is the authority for whether this is a fleet worker. Labels,
 * facade access and first-message presence are deliberately irrelevant: user
 * panes can have tools, workers can start before their task arrives, and the
 * Fleet Manager itself has a facade. */
export function isFleetDispatchedWorker(meta: {
  parentSessionId?: string;
  manager?: boolean;
}): boolean {
  return !meta.manager && !!meta.parentSessionId?.trim();
}

const REQUIRED_KEYS = [
  'type',
  'status',
  'reason',
  'requiredAuthorityOrDecision',
  'changed',
  'nextAction',
] as const;

/** Always-on prompt contract. `wks-result` remains the completion path when a
 * dispatch also supplied resultSchema; escalation is the alternative terminal
 * path and therefore uses its own tag and fixed object. */
export function buildWorkerEscalationContract(): string {
  return (
    'STRUCTURED WORKER ESCALATION CONTRACT. If you cannot safely complete the task because ' +
    'you lack authority or need a manager/user decision, do not only refuse in prose. Stop ' +
    'and write a concise explanation first, then END your final message with exactly one ' +
    'fenced `' +
    WORKER_ESCALATION_FENCE +
    '` JSON block in this exact shape:\n\n```' +
    WORKER_ESCALATION_FENCE +
    '\n{\n' +
    '  "type": "worker-escalation",\n' +
    '  "status": "blocked",\n' +
    '  "reason": "concise blocker",\n' +
    '  "requiredAuthorityOrDecision": "specific authority or decision needed",\n' +
    '  "changed": false,\n' +
    '  "nextAction": "useful next action for the manager"\n' +
    '}\n```\n\n' +
    'Use this only as a terminal escalation, not for a successful completion or a routine ' +
    'progress update. Report truthfully whether anything changed. If a separate `wks-result` ' +
    'contract is also present, emit `wks-result` when you complete the task; when you escalate, ' +
    'emit `wks-escalation` instead. Emit the chosen terminal block only once and put nothing after it.'
  );
}

const ESCALATION_FENCE_RE = /```[ \t]*wks-escalation[ \t]*\r?\n([\s\S]*?)```/gi;

function extractEscalationBlock(text: string): string | null {
  let block: string | null = null;
  ESCALATION_FENCE_RE.lastIndex = 0;
  for (let m = ESCALATION_FENCE_RE.exec(text); m; m = ESCALATION_FENCE_RE.exec(text)) {
    block = m[1];
  }
  return block;
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/**
 * Parse and strictly validate a terminal escalation. Returns null when the
 * worker emitted no escalation tag at all (ordinary completion/refusal), and
 * `{error}` when it tried the protocol but produced an invalid shape.
 */
export function readWorkerEscalation(finalMessage: string): WorkerEscalationRead | null {
  const block = extractEscalationBlock(finalMessage ?? '');
  if (block === null) return null;
  if (block.length > WORKER_ESCALATION_MAX) {
    return {
      error: `the \`${WORKER_ESCALATION_FENCE}\` block is ${block.length} bytes; the limit is ${WORKER_ESCALATION_MAX}`,
    };
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(block);
  } catch (e) {
    return {
      error: `the \`${WORKER_ESCALATION_FENCE}\` block is not valid JSON: ${(e as Error).message}`,
    };
  }
  if (!isPlainObject(parsed)) {
    return { error: `the \`${WORKER_ESCALATION_FENCE}\` block must contain one JSON object` };
  }

  const extras = Object.keys(parsed).filter(
    (key) => !REQUIRED_KEYS.includes(key as (typeof REQUIRED_KEYS)[number]),
  );
  if (extras.length > 0) {
    return { error: `the escalation has unexpected properties: ${extras.join(', ')}` };
  }
  if (parsed.type !== 'worker-escalation') {
    return { error: 'type: expected "worker-escalation"' };
  }
  if (parsed.status !== 'blocked') {
    return { error: 'status: expected "blocked"' };
  }
  for (const key of ['reason', 'requiredAuthorityOrDecision', 'nextAction'] as const) {
    if (typeof parsed[key] !== 'string' || parsed[key].trim() === '') {
      return { error: `${key}: expected a non-empty string` };
    }
  }
  if (typeof parsed.changed !== 'boolean') {
    return { error: 'changed: expected boolean' };
  }

  const value: WorkerEscalation = {
    type: 'worker-escalation',
    status: 'blocked',
    reason: (parsed.reason as string).trim(),
    requiredAuthorityOrDecision: (parsed.requiredAuthorityOrDecision as string).trim(),
    changed: parsed.changed as boolean,
    nextAction: (parsed.nextAction as string).trim(),
  };
  return { value, json: JSON.stringify(value, null, 2) };
}
