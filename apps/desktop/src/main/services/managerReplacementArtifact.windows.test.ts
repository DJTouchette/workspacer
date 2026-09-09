import { afterEach, expect, it, vi } from 'vitest';
import fs from 'fs';
import path from 'path';
import { createHash } from 'crypto';
import type { ReplacementRecord } from './managerReplacementState';
import { preparationPrompt, validateManagerArtifact } from './managerReplacementArtifact';

// Execute the production validator with Node's real Win32 path implementation.
// Filesystem responses model a drive spelling changed by realpath; native disk
// behavior is covered separately, including on the Windows CI runner.
vi.mock('path', async (original) => {
  const actual = await original<typeof import('path')>();
  return { ...actual, default: actual.win32 };
});
afterEach(() => vi.restoreAllMocks());
const hash = (bytes: string | Buffer) => createHash('sha256').update(bytes).digest('hex');

function fixture(cwd = 'c:\\Users\\Manager', pointerRoot = cwd) {
  const operationId = '12345678-1234-1234-1234-123456789abc';
  const canonicalRoot = 'C:\\Users\\Manager';
  const artifactPath = path.join(
    canonicalRoot,
    '.workspacer',
    'manager-handoffs',
    operationId,
    'handoff.json',
  );
  const op = {
    operationId,
    sourceSessionId: 'source',
    artifactPath,
    launch: { options: { cwd } },
    workerIds: [],
    taskIds: ['task'],
    metadata: [],
  } as unknown as ReplacementRecord;
  const prompt = preparationPrompt(op);
  const example = prompt.slice(prompt.indexOf('{\n'), prompt.lastIndexOf('\nInclude'));
  const artifact = JSON.parse(example);
  const brief = Buffer.from('## Now\r\n- café: pending review\r\n');
  artifact.checkpoint.files[0] = {
    path: path.join(pointerRoot, '.workspacer', 'brief.md'),
    sha256: hash(brief),
  };
  artifact.tasks[0].nextAction = 'Review';
  artifact.nextAction = 'Wait';
  const key = (p: fs.PathLike) =>
    String(p)
      .replaceAll('/', '\\')
      .replace(/^[a-z]:/, (d) => d.toUpperCase());
  const briefPath = path.join(canonicalRoot, '.workspacer', 'brief.md');
  let raw: Buffer;
  const write = () => {
    raw = Buffer.from(JSON.stringify(artifact, null, 2).replaceAll('\n', '\r\n'));
  };
  write();
  vi.spyOn(fs, 'realpathSync').mockImplementation(((p: fs.PathLike) =>
    key(p)) as typeof fs.realpathSync);
  const stat = (p: fs.PathLike) => ({
    isSymbolicLink: () => false,
    isFile: () => true,
    size: key(p) === key(artifactPath) ? raw.length : brief.length,
  });
  vi.spyOn(fs, 'lstatSync').mockImplementation(stat as typeof fs.lstatSync);
  vi.spyOn(fs, 'statSync').mockImplementation(stat as typeof fs.statSync);
  vi.spyOn(fs, 'readFileSync').mockImplementation(((p: fs.PathLike, encoding?: string) => {
    const bytes = key(p) === key(artifactPath) ? raw : key(p) === briefPath ? brief : undefined;
    if (!bytes) throw new Error('Unexpected fixture read');
    return encoding ? bytes.toString('utf8') : bytes;
  }) as typeof fs.readFileSync);
  const receipt = () =>
    '```wks-manager-handoff\n' +
    JSON.stringify({ operationId, sourceSessionId: 'source', sha256: hash(raw) }) +
    '\n```';
  return {
    op,
    artifact,
    write,
    receipt,
    brief,
    validate: () => validateManagerArtifact(op, receipt()),
  };
}

it.each([
  ['c:\\Users\\Manager', 'c:\\Users\\Manager'],
  ['C:/Users/Manager', 'C:\\Users\\Manager'],
  ['C:\\Users\\Manager', 'c:\\Users\\Manager'],
])('validates prompt JSON and exact CRLF bytes for root %s and brief root %s', (cwd, pointer) => {
  const f = fixture(cwd, pointer);
  const result = f.validate();
  expect(result.hash).toBe(hash(Buffer.from(result.raw)));
  expect(JSON.parse(result.raw).checkpoint.files[0].sha256).toBe(hash(f.brief));
});

it.each(['D:\\Users\\Manager', 'C:\\Users\\Manager-other', 'C:\\users\\manager'])(
  'keeps unrelated roots and component case rejected even with a correct file hash: %s',
  (pointer) => {
    const f = fixture('C:\\Users\\Manager', pointer);
    expect(f.validate).toThrow();
  },
);
