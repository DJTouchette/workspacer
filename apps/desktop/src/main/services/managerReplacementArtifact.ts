import fs from 'fs';
import path from 'path';
import { createHash } from 'crypto';
import type { ReplacementRecord } from './managerReplacementState';

export const artifactHash = (text: string) => createHash('sha256').update(text).digest('hex');
export interface ManagerHandoffArtifact {
  version: 1;
  operationId: string;
  sourceSessionId: string;
  cwd: string;
  checkpoint: { completed: true; files: Array<{ path: string; sha256: string }> };
  workers: Array<{ sessionId: string; instructions: string }>;
  tasks: Array<{ taskId: string; nextAction: string }>;
  pendingDecisions: string[];
  facts: string[];
  nextAction: string;
}
export function createArtifactPath(cwd: string, operationId: string): string {
  if (!/^[0-9a-f-]{36}$/.test(operationId))
    throw new Error('Invalid manager handoff operation identity');
  const root = fs.realpathSync(cwd);
  let dir = root;
  for (const part of ['.workspacer', 'manager-handoffs', operationId]) {
    dir = path.join(dir, part);
    if (!fs.existsSync(dir)) fs.mkdirSync(dir, { mode: 0o700 });
    if (fs.lstatSync(dir).isSymbolicLink() || !fs.statSync(dir).isDirectory())
      throw new Error('Manager handoff directory must be a real local directory, not a link');
  }
  return path.join(dir, 'handoff.json');
}
function text(value: unknown): value is string {
  return typeof value === 'string' && !!value.trim();
}
function exactIds(values: string[], expected: string[]): boolean {
  return (
    values.length === expected.length &&
    new Set(values).size === values.length &&
    values.every((id) => expected.includes(id))
  );
}
/** No generic idle/prose or shared handoff.md is a completion signal. The
 * operation-specific receipt must name the exact bytes and source identity. */
export function validateManagerArtifact(
  op: ReplacementRecord,
  receiptText: string,
): { raw: string; hash: string } {
  const expected = path.join(
    fs.realpathSync(op.launch.options.cwd!),
    '.workspacer',
    'manager-handoffs',
    op.operationId,
    'handoff.json',
  );
  if (op.artifactPath !== expected || fs.realpathSync(expected) !== expected)
    throw new Error('Handoff artifact path changed or is a symbolic link');
  const stat = fs.lstatSync(expected);
  if (!stat.isFile() || stat.size === 0 || stat.size > 256 * 1024)
    throw new Error('Handoff artifact must be a nonempty regular file, at most 256 KiB');
  const raw = fs.readFileSync(expected, 'utf8');
  const a = JSON.parse(raw) as ManagerHandoffArtifact;
  if (
    a.version !== 1 ||
    a.operationId !== op.operationId ||
    a.sourceSessionId !== op.sourceSessionId ||
    a.cwd !== op.launch.options.cwd ||
    a.checkpoint?.completed !== true ||
    !Array.isArray(a.checkpoint.files) ||
    !a.checkpoint.files.length ||
    !Array.isArray(a.workers) ||
    !Array.isArray(a.tasks) ||
    !exactIds(
      a.workers.map((w) => w.sessionId),
      op.workerIds,
    ) ||
    !a.workers.every((w) => text(w.instructions)) ||
    !exactIds(
      a.tasks.map((t) => t.taskId),
      op.taskIds,
    ) ||
    !a.tasks.every((t) => text(t.nextAction)) ||
    !Array.isArray(a.pendingDecisions) ||
    !a.pendingDecisions.every(text) ||
    !Array.isArray(a.facts) ||
    !a.facts.every(text) ||
    !text(a.nextAction)
  )
    throw new Error(
      'Handoff checkpoint, identity, worker instructions, tasks or pending-decision protocol is invalid',
    );
  const roots = new Set([
    op.launch.options.cwd!,
    ...op.metadata.map((m) => m.cwd),
    ...(op.projectCwds ?? []),
  ]);
  const fleetBrief = path.join(op.launch.options.cwd!, '.workspacer', 'brief.md');
  if (!a.checkpoint.files.some((f) => f.path === fleetBrief))
    throw new Error('Checkpoint must include the fleet brief');
  for (const f of a.checkpoint.files) {
    if (
      !text(f.path) ||
      !roots.has(path.dirname(path.dirname(f.path))) ||
      path.basename(path.dirname(f.path)) !== '.workspacer' ||
      !['brief.md', 'brief.archive.md'].includes(path.basename(f.path)) ||
      fs.realpathSync(f.path) !== f.path ||
      fs.lstatSync(f.path).isSymbolicLink() ||
      !fs.lstatSync(f.path).isFile() ||
      fs.statSync(f.path).size > 2 * 1024 * 1024 ||
      artifactHash(fs.readFileSync(f.path, 'utf8')) !== f.sha256
    )
      throw new Error('Checkpoint brief pointer or content hash is invalid');
  }
  const hash = artifactHash(raw);
  const blocks = [...receiptText.matchAll(/```wks-manager-handoff\s*\n([\s\S]*?)\n```/g)];
  if (blocks.length !== 1)
    throw new Error('Expected one operation-correlated wks-manager-handoff receipt');
  const receipt = JSON.parse(blocks[0][1]);
  if (
    receipt.operationId !== op.operationId ||
    receipt.sourceSessionId !== op.sourceSessionId ||
    receipt.sha256 !== hash
  )
    throw new Error('Handoff receipt identity or hash does not match');
  return { raw, hash };
}
export function preparationPrompt(op: ReplacementRecord): string {
  return (
    `HOST-OWNED MANAGER HANDOFF ${op.operationId}. Run /checkpoint first. This is preparation only: do not dispatch, adopt, terminate, ask the user to reopen, or continue task actions. Preserve pending user decisions and exact worker instructions. The host owns replacement and adoption. Standalone /handoff instructions to terminate/reopen do not apply.\n` +
    `Checkpoint project roots: ${JSON.stringify(op.projectCwds ?? [])}. If you know of any owned worker missing from the listed IDs, especially a remote or still-allocating dispatch, do not emit a receipt: explain the missing ownership instead. Never drop a known worker to fit this protocol.\nWrite exactly ${op.artifactPath} as JSON matching this example (all listed workers and tasks are required; fill the text from your context):\n` +
    JSON.stringify(
      {
        version: 1,
        operationId: op.operationId,
        sourceSessionId: op.sourceSessionId,
        cwd: op.launch.options.cwd,
        checkpoint: {
          completed: true,
          files: [
            {
              path: path.join(op.launch.options.cwd!, '.workspacer', 'brief.md'),
              sha256: '<SHA-256 of final file bytes after checkpoint>',
            },
          ],
        },
        workers: op.workerIds.map((sessionId) => ({
          sessionId,
          instructions: '<exact dispatch instructions and current state>',
        })),
        tasks: op.taskIds.map((taskId) => ({
          taskId,
          nextAction: '<state, pending decisions and next action>',
        })),
        pendingDecisions: [],
        facts: [],
        nextAction: '<immediate next action>',
      },
      null,
      2,
    ) +
    `\nInclude the checkpointed fleet brief and relevant project brief pointers with SHA-256 hashes, never copies of the briefs. Empty decision/fact arrays mean none; do not omit known decisions. After writing, end your turn with exactly one fenced wks-manager-handoff JSON receipt containing operationId=${op.operationId}, sourceSessionId=${op.sourceSessionId}, and sha256 of the handoff.json bytes. Do not continue work. Do not emit a receipt unless checkpoint and handoff are complete.`
  );
}
