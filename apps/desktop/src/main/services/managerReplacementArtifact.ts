import fs from 'fs';
import path from 'path';
import { createHash } from 'crypto';
import type { ReplacementRecord } from './managerReplacementState';

export const artifactHash = (bytes: string | Buffer) =>
  createHash('sha256').update(bytes).digest('hex');

/** Compare spellings without resolving dot segments, links or component case.
 * Windows drive letters and separators are interchangeable; directory names
 * can still be case-sensitive. POSIX paths retain byte-exact comparison. */
function samePath(a: string, b: string): boolean {
  const spelling = (p: string) =>
    path.sep === '\\'
      ? p.replaceAll('/', '\\').replace(/^[a-z]:/, (drive) => drive.toUpperCase())
      : p;
  return spelling(a) === spelling(b);
}
interface VerifiedPath {
  volume: string;
  canonical: string;
  stat: fs.BigIntStats;
}
function sameIdentity(a: fs.BigIntStats, b: fs.BigIntStats): boolean {
  // An unavailable file ID is not evidence of identity. BigInts avoid rounding
  // the 64-bit Windows file index into a false match.
  return a.ino > 0n && b.ino > 0n && a.dev === b.dev && a.ino === b.ino;
}
function plainAbsolutePath(value: string): boolean {
  if (!path.isAbsolute(value)) return false;
  const volume = path.parse(value).root;
  if (path.sep === '\\' && volume.length < 3) return false;
  if (value === volume) return true;
  const parts = value.slice(volume.length).split(path.sep === '\\' ? /[\\/]/ : /\//);
  return parts.every(
    (part) =>
      part !== '' && part !== '.' && part !== '..' && (path.sep !== '\\' || !/[ .]$|:/.test(part)),
  );
}
/** Walk before realpath: realpath alone cannot distinguish harmless case
 * spelling from a junction/symlink, or preserve the dot-segment policy. */
function verifyPath(value: string): VerifiedPath {
  if (!plainAbsolutePath(value)) throw new Error('invalid path spelling');
  const volume = path.parse(value).root;
  let current = volume;
  let stat = fs.lstatSync(current, { bigint: true });
  const parts =
    value === volume ? [] : value.slice(volume.length).split(path.sep === '\\' ? /[\\/]/ : /\//);
  for (const part of parts) {
    if (stat.isSymbolicLink() || !stat.isDirectory()) throw new Error('linked parent');
    current = path.join(current, part);
    stat = fs.lstatSync(current, { bigint: true });
  }
  if (stat.isSymbolicLink()) throw new Error('linked path');
  const canonical = fs.realpathSync(value);
  if (!sameIdentity(stat, fs.statSync(canonical, { bigint: true })))
    throw new Error('path changed');
  return { volume, canonical, stat };
}
function sameLocation(a: VerifiedPath, b: VerifiedPath): boolean {
  // No drive, UNC or mapped-drive alias expansion. Component spelling is not
  // authority on Windows: case-sensitive directories must still have equal IDs.
  return (
    samePath(a.volume, b.volume) &&
    (path.sep === '\\' || samePath(a.canonical, b.canonical)) &&
    sameIdentity(a.stat, b.stat)
  );
}
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
  if (!samePath(op.artifactPath, expected) || !samePath(fs.realpathSync(expected), expected))
    throw new Error('Handoff artifact path changed or is a symbolic link');
  const stat = fs.lstatSync(expected);
  if (!stat.isFile() || stat.size === 0 || stat.size > 256 * 1024)
    throw new Error('Handoff artifact must be a nonempty regular file, at most 256 KiB');
  const bytes = fs.readFileSync(expected);
  const raw = bytes.toString('utf8');
  if (!Buffer.from(raw).equals(bytes))
    throw new Error('Handoff artifact must contain valid UTF-8 bytes');
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
  const roots = [
    op.launch.options.cwd!,
    ...op.metadata.map((m) => m.cwd),
    ...(op.projectCwds ?? []),
  ];
  let includedFleetBrief = false;
  for (const [index, f] of a.checkpoint.files.entries()) {
    const invalid = (category: string): never => {
      throw new Error(`Checkpoint files[${index}] ${category}`);
    };
    if (
      !f ||
      !text(f.path) ||
      !plainAbsolutePath(f.path) ||
      path.basename(path.dirname(f.path)) !== '.workspacer' ||
      !['brief.md', 'brief.archive.md'].includes(path.basename(f.path))
    )
      invalid('path is not an allowed brief pointer');
    if (typeof f.sha256 !== 'string' || !/^[0-9a-f]{64}$/.test(f.sha256))
      invalid('sha256 must be 64 lowercase hexadecimal characters');
    let pointer: VerifiedPath;
    let pointerRoot: VerifiedPath;
    try {
      pointerRoot = verifyPath(path.dirname(path.dirname(f.path)));
    } catch {
      return invalid('path is not an allowed brief pointer');
    }
    try {
      pointer = verifyPath(f.path);
    } catch {
      return invalid('path changed or is not a regular file without links');
    }
    if (!pointer.stat.isFile()) invalid('path changed or is not a regular file without links');
    let target: VerifiedPath | undefined;
    for (const [rootIndex, root] of roots.entries()) {
      // Stale unrelated project roots must not disable the valid fleet brief.
      try {
        const knownRoot = verifyPath(root);
        if (!knownRoot.stat.isDirectory() || !sameLocation(pointerRoot, knownRoot)) continue;
        const knownFile = verifyPath(path.join(root, '.workspacer', path.basename(f.path)));
        if (!knownFile.stat.isFile() || !sameLocation(pointer, knownFile)) continue;
        target = knownFile;
        if (rootIndex === 0 && path.basename(f.path) === 'brief.md') includedFleetBrief = true;
        break;
      } catch {
        /* Unverifiable roots/files grant no authority. */
      }
    }
    if (!target) return invalid('path is not an allowed brief pointer');
    let briefBytes: Buffer;
    let fd: number | undefined;
    try {
      // Read the host-known canonical file, and bind the read to its verified
      // identity even if a pathname is replaced after inspection.
      fd = fs.openSync(target.canonical, 'r');
      const opened = fs.fstatSync(fd, { bigint: true });
      if (!opened.isFile() || !sameIdentity(opened, target.stat))
        invalid('path changed or is not a regular file without links');
      if (opened.size > 2n * 1024n * 1024n) invalid('file exceeds 2 MiB');
      briefBytes = fs.readFileSync(fd);
      if (briefBytes.length > 2 * 1024 * 1024) invalid('file exceeds 2 MiB');
    } catch (error) {
      if (error instanceof Error && error.message.startsWith(`Checkpoint files[${index}]`))
        throw error;
      return invalid('file could not be inspected or read');
    } finally {
      if (fd !== undefined) fs.closeSync(fd);
    }
    if (artifactHash(briefBytes!) !== f.sha256)
      invalid('sha256 does not match current file bytes; checkpoint may have changed');
  }
  if (!includedFleetBrief) throw new Error('Checkpoint must include the fleet brief');
  const hash = artifactHash(bytes);
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
