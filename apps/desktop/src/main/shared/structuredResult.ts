/**
 * Structured worker results — a dispatch may name a result SCHEMA, and the
 * worker's finish then delivers a validated OBJECT alongside its prose.
 *
 * Why this exists: a worker's outcome arrives as prose in its final message and
 * the manager hand-transcribes it into brief lines, which is transcription work
 * and a place to lose detail. `spawn_agent`'s `resultSchema` closes that: the
 * schema is compiled into a REPORT CONTRACT injected at spawn (Claude PTY:
 * `--append-system-prompt`; managed/stream providers: the daemon's first-turn
 * `instructions`), and at finish the wake carries the parsed, validated object.
 *
 * It deliberately mirrors the Workflow tool's `agent({schema})` /
 * `StructuredOutput` shape rather than inventing a second vocabulary: ONE JSON
 * Schema in, ONE validated object out, validation failures reported back rather
 * than silently dropped. The difference is the channel — a workspacer worker is
 * a real Claude Code / Codex / OpenCode process with no tool wired back to its
 * dispatcher, so the "forced tool call" becomes a fenced block in the final
 * message, extracted and validated here.
 *
 * ADDITIVE BY CONSTRUCTION. The prose never goes away: the contract tells the
 * worker to write its normal summary FIRST and the block after it, the wake
 * still carries `lastReply`/`fullReply`, and a worker that ignores or botches
 * the contract simply reports `resultError` beside the prose it did send. A
 * schema is a request for structure, never a gate on the report.
 *
 * The validator is a deliberate SUBSET of JSON Schema (type/properties/required/
 * items/enum/additionalProperties) implemented here rather than pulled in as a
 * dependency: the schemas are model-authored dispatch contracts a few keys wide,
 * and every unknown keyword is IGNORED — an over-rich schema under-constrains,
 * it never rejects a well-formed report over a keyword we don't implement.
 */

/** Fence tag the contract asks for, and the one extraction prefers. */
export const RESULT_FENCE = 'wks-result';

/** Cap on the serialized schema a spawn may carry. A dispatch contract is a
 *  handful of keys; anything larger is prompt injection by volume (the schema
 *  is pasted verbatim into the worker's system prompt), so it is refused at
 *  spawn — loudly — rather than truncated into something that no longer means
 *  what the caller wrote. */
export const RESULT_SCHEMA_MAX = 4096;

/** Cap on the result JSON carried into a wake. Generous (a report object is
 *  small) but bounded, and truncation is announced rather than silent. */
export const RESULT_MAX = 8192;

/** Most validation errors a single report reports — enough to fix the shape,
 *  short enough that a wake stays readable. */
const MAX_ERRORS = 8;

export interface StructuredResult {
  /** The validated object, pretty-printed. Present only on success. */
  json?: string;
  /** Why no validated object could be produced (missing block, unparseable
   *  JSON, or schema violations). Present only on failure. */
  error?: string;
}

function isPlainObject(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

/** Whether a caller-supplied `resultSchema` is usable at all: a JSON object
 *  (not an array, not a scalar) that serializes under the cap. Returns the
 *  reason it is not, so the spawn can refuse out loud. */
export function checkResultSchema(schema: unknown): string | null {
  if (!isPlainObject(schema)) return 'resultSchema must be a JSON Schema object';
  let text: string;
  try {
    text = JSON.stringify(schema);
  } catch {
    return 'resultSchema is not JSON-serializable';
  }
  if (text.length > RESULT_SCHEMA_MAX) {
    return `resultSchema is ${text.length} bytes; the limit is ${RESULT_SCHEMA_MAX}`;
  }
  return null;
}

/**
 * The report contract: what a worker is told when its dispatch carries a
 * schema. Written as an instruction to the worker, not a description of the
 * feature — it is injected into a system prompt, where prose about workspacer
 * would only spend context.
 */
export function buildResultContract(schema: Record<string, unknown>): string {
  return (
    'STRUCTURED RESULT CONTRACT. Whoever dispatched you asked for a machine-readable ' +
    'result as well as your prose. When your work is finished, write your normal ' +
    'summary first — it is read by a human and must not be dropped — and then END your ' +
    'final message with a fenced code block tagged `' +
    RESULT_FENCE +
    '` containing ONE JSON object that validates against this schema:\n\n' +
    JSON.stringify(schema, null, 2) +
    '\n\nFormat exactly:\n\n```' +
    RESULT_FENCE +
    '\n{ ... }\n```\n\n' +
    'Emit the block only in your FINAL message (not mid-task), only once, and put ' +
    'nothing after it. Report what actually happened — an empty list or a null is a ' +
    'truthful answer; an invented value is not. If you could not complete the task, ' +
    'still emit the block with whatever fields are true and say so in the prose.'
  );
}

/** Fenced blocks tagged `wks-result`, or (fallback) plain ```json blocks. */
const FENCE_RE = /```[ \t]*([A-Za-z0-9_-]*)[ \t]*\r?\n([\s\S]*?)```/g;

/**
 * The LAST candidate block's body. `wks-result` wins over `json` regardless of
 * order — a worker that quotes a JSON example mid-report and then emits its
 * real tagged block must not have the example win, and vice versa the last
 * TAGGED block is the report (an earlier one would be a draft).
 */
export function extractResultBlock(text: string): string | null {
  let tagged: string | null = null;
  let untagged: string | null = null;
  FENCE_RE.lastIndex = 0;
  for (let m = FENCE_RE.exec(text); m; m = FENCE_RE.exec(text)) {
    const tag = m[1].toLowerCase();
    if (tag === RESULT_FENCE) tagged = m[2];
    else if (tag === 'json') untagged = m[2];
  }
  return tagged ?? untagged;
}

function jsonTypeOf(v: unknown): string {
  if (v === null) return 'null';
  if (Array.isArray(v)) return 'array';
  if (Number.isInteger(v as number)) return 'integer';
  return typeof v;
}

function matchesType(v: unknown, t: string): boolean {
  switch (t) {
    case 'object':
      return isPlainObject(v);
    case 'array':
      return Array.isArray(v);
    case 'string':
      return typeof v === 'string';
    case 'number':
      return typeof v === 'number' && Number.isFinite(v);
    case 'integer':
      return typeof v === 'number' && Number.isInteger(v);
    case 'boolean':
      return typeof v === 'boolean';
    case 'null':
      return v === null;
    default:
      // An unknown type keyword constrains nothing — the subset rule.
      return true;
  }
}

function sameJson(a: unknown, b: unknown): boolean {
  try {
    return JSON.stringify(a) === JSON.stringify(b);
  } catch {
    return false;
  }
}

function join(path: string, key: string): string {
  return path ? `${path}.${key}` : key;
}

/**
 * Collect the ways `value` fails `schema`, as human-readable paths. Empty means
 * valid. Unknown keywords are ignored (see the module note): this validator
 * only ever narrows on keywords it implements, so it cannot reject a report the
 * caller's schema actually permits.
 */
export function validateAgainstSchema(value: unknown, schema: unknown): string[] {
  const errors: string[] = [];
  walk(value, schema, '', errors);
  return errors.slice(0, MAX_ERRORS);
}

function walk(value: unknown, schema: unknown, path: string, errors: string[]): void {
  if (errors.length >= MAX_ERRORS) return;
  if (!isPlainObject(schema)) return; // `true` / absent / malformed → unconstrained
  const where = path || 'result';

  const rawType = schema.type;
  const types =
    typeof rawType === 'string'
      ? [rawType]
      : Array.isArray(rawType)
        ? rawType.filter((t): t is string => typeof t === 'string')
        : [];
  if (types.length > 0 && !types.some((t) => matchesType(value, t))) {
    errors.push(`${where}: expected ${types.join(' or ')}, got ${jsonTypeOf(value)}`);
    return; // a wrong type would cascade into meaningless child errors
  }

  if (Array.isArray(schema.enum) && !schema.enum.some((e) => sameJson(e, value))) {
    errors.push(`${where}: ${JSON.stringify(value)} is not one of ${JSON.stringify(schema.enum)}`);
  }

  if (isPlainObject(value)) {
    const props = isPlainObject(schema.properties) ? schema.properties : {};
    if (Array.isArray(schema.required)) {
      for (const key of schema.required) {
        if (typeof key === 'string' && !(key in value)) {
          errors.push(`${join(path, key)}: required property missing`);
        }
      }
    }
    for (const [key, child] of Object.entries(value)) {
      if (key in props) walk(child, props[key], join(path, key), errors);
      else if (schema.additionalProperties === false) {
        errors.push(`${join(path, key)}: unexpected property`);
      }
    }
  }

  if (Array.isArray(value) && schema.items !== undefined) {
    for (let i = 0; i < value.length; i++) {
      walk(value[i], schema.items, `${where}[${i}]`, errors);
    }
  }
}

/**
 * Read a worker's structured result out of its FINAL message. Never throws:
 * every failure mode (no block, unparseable JSON, schema violations) comes back
 * as `error`, because the wake carries it beside the prose and a manager acting
 * on "the worker didn't report cleanly" is better served than one handed
 * nothing.
 */
export function readStructuredResult(
  finalMessage: string,
  schema: Record<string, unknown>,
): StructuredResult {
  const block = extractResultBlock(finalMessage ?? '');
  if (block === null) {
    return {
      error: `no \`${RESULT_FENCE}\` block in the worker's final message — it did not honor the result contract`,
    };
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(block);
  } catch (e) {
    return { error: `the \`${RESULT_FENCE}\` block is not valid JSON: ${(e as Error).message}` };
  }
  const errors = validateAgainstSchema(parsed, schema);
  if (errors.length > 0) {
    return { error: `the result does not match the requested schema: ${errors.join('; ')}` };
  }
  let json: string;
  try {
    json = JSON.stringify(parsed, null, 2);
  } catch {
    return { error: 'the result could not be re-serialized' };
  }
  if (json.length > RESULT_MAX) {
    json = `${json.slice(0, RESULT_MAX)}\n[truncated: ${json.length} bytes of validated result]`;
  }
  return { json };
}
