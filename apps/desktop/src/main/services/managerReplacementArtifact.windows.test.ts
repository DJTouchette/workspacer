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

function fixture(cwd = 'c:\\Users\\Manager', pointerRoot = cwd, sameIdentity = false) {
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
  let raw: Buffer;
  const write = () => {
    raw = Buffer.from(JSON.stringify(artifact, null, 2).replaceAll('\n', '\r\n'));
  };
  write();
  vi.spyOn(fs, 'realpathSync').mockImplementation(((p: fs.PathLike) =>
    key(p)) as typeof fs.realpathSync);
  const identityKey = (p: fs.PathLike) => (sameIdentity ? key(p).toLowerCase() : key(p));
  const identities = new Map<string, bigint>();
  const stat = (p: fs.PathLike) => ({
    dev: 1n,
    ino: (() => {
      const k = identityKey(p);
      if (!identities.has(k)) identities.set(k, BigInt(identities.size + 1));
      return identities.get(k)!;
    })(),
    isDirectory: () => !String(p).endsWith('.md') && !String(p).endsWith('.json'),
    isSymbolicLink: () => false,
    isFile: () => true,
    size: key(p) === key(artifactPath) ? raw.length : brief.length,
  });
  vi.spyOn(fs, 'lstatSync').mockImplementation(stat as typeof fs.lstatSync);
  vi.spyOn(fs, 'statSync').mockImplementation(stat as typeof fs.statSync);
  let openedPath: fs.PathLike;
  vi.spyOn(fs, 'openSync').mockImplementation((p) => {
    openedPath = p;
    return 123;
  });
  vi.spyOn(fs, 'fstatSync').mockImplementation((() => stat(openedPath)) as typeof fs.fstatSync);
  vi.spyOn(fs, 'closeSync').mockImplementation(() => {});
  vi.spyOn(fs, 'readFileSync').mockImplementation(((p: fs.PathLike, encoding?: string) => {
    if (typeof p === 'number') p = openedPath;
    const bytes =
      key(p) === key(artifactPath) ? raw : String(p).endsWith('.md') ? brief : undefined;
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
    stat,
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

it('accepts differently cased Windows components only when filesystem identities agree', () => {
  const f = fixture('C:\\Users\\Manager', 'c:\\users\\manager', true);
  expect(f.validate().hash).toBe(
    hash(Buffer.from(JSON.stringify(f.artifact, null, 2).replaceAll('\n', '\r\n'))),
  );
});

it.each(['metadata', 'projectCwds'] as const)(
  'verifies differently spelled %s roots as well as the fleet root',
  (source) => {
    const f = fixture('C:\\Users\\Manager', 'C:\\Users\\Manager', true);
    const root = 'C:\\Projects\\Example';
    if (source === 'metadata') f.op.metadata = [{ sessionId: 'worker', cwd: root }];
    else f.op.projectCwds = [root];
    f.artifact.checkpoint.files.push({
      path: 'c:\\projects\\example\\.workspacer\\brief.md',
      sha256: hash(f.brief),
    });
    f.write();
    expect(f.validate().hash).toBeTruthy();
    expect(fs.openSync).toHaveBeenLastCalledWith(
      'C:\\Projects\\Example\\.workspacer\\brief.md',
      'r',
    );
  },
);

it('rejects different file identities even when root identities match and bytes agree', () => {
  const f = fixture('C:\\Users\\Manager', 'c:\\users\\manager', true);
  const stat = (p: fs.PathLike) => ({
    ...f.stat(p),
    ...(String(p).includes('\\users\\manager\\.workspacer\\brief.md') ? { ino: 99999n } : {}),
  });
  vi.mocked(fs.lstatSync).mockImplementation(stat as typeof fs.lstatSync);
  vi.mocked(fs.statSync).mockImplementation(stat as typeof fs.statSync);
  expect(f.validate).toThrow('path is not an allowed brief pointer');
  expect(fs.openSync).not.toHaveBeenCalled();
});

it('rejects unavailable filesystem identity instead of authorizing two zero IDs', () => {
  const f = fixture('C:\\Users\\Manager', 'c:\\users\\manager', true);
  const stat = (p: fs.PathLike) => ({ ...f.stat(p), ino: 0n });
  vi.mocked(fs.lstatSync).mockImplementation(stat as typeof fs.lstatSync);
  vi.mocked(fs.statSync).mockImplementation(stat as typeof fs.statSync);
  expect(f.validate).toThrow('Checkpoint files[0]');
  expect(fs.openSync).not.toHaveBeenCalled();
});

it('rejects a different identity at open before reading its bytes', () => {
  const f = fixture('C:\\Users\\Manager', 'c:\\users\\manager', true);
  vi.mocked(fs.fstatSync).mockImplementation((() => ({
    ...f.stat(f.artifact.checkpoint.files[0].path),
    ino: 99999n,
  })) as typeof fs.fstatSync);
  expect(f.validate).toThrow('path changed');
  expect(fs.readFileSync).not.toHaveBeenCalledWith(123);
  expect(fs.closeSync).toHaveBeenCalledWith(123);
});

it('rejects a junction even when realpath and stat identify the allowed target', () => {
  const f = fixture('C:\\Users\\Manager', 'c:\\users\\manager', true);
  vi.mocked(fs.lstatSync).mockImplementation(((p: fs.PathLike) => ({
    ...f.stat(p),
    isSymbolicLink: () => String(p) === 'c:\\users\\manager',
  })) as typeof fs.lstatSync);
  expect(f.validate).toThrow('Checkpoint files[0]');
  expect(fs.openSync).not.toHaveBeenCalled();
});

it('rejects dot segments before any normalization can erase them', () => {
  const f = fixture('C:\\Users\\Manager', 'c:\\users\\manager', true);
  f.artifact.checkpoint.files[0].path = 'c:\\users\\manager\\.\\.workspacer\\brief.md';
  f.write();
  expect(f.validate).toThrow('path is not an allowed brief pointer');
});

it('does not expand short-name aliases even when filesystem identities agree', () => {
  const f = fixture('C:\\Users\\Manager', 'C:\\Users\\MANAGE~1', true);
  const longName = (p: fs.PathLike) => String(p).replace('MANAGE~1', 'Manager');
  const stat = (p: fs.PathLike) => f.stat(longName(p));
  vi.mocked(fs.lstatSync).mockImplementation(stat as typeof fs.lstatSync);
  vi.mocked(fs.statSync).mockImplementation(stat as typeof fs.statSync);
  vi.mocked(fs.realpathSync).mockImplementation(longName as typeof fs.realpathSync);
  expect(f.validate).toThrow('path is not an allowed brief pointer');
  expect(fs.openSync).not.toHaveBeenCalled();
});

it.each([
  'C:Users\\Manager\\.workspacer\\brief.md',
  '\\Users\\Manager\\.workspacer\\brief.md',
  'C:\\Users\\Manager.\\.workspacer\\brief.md',
  'C:\\Users\\Manager \\.workspacer\\brief.md',
  'C:\\Users\\Manager\\.workspacer\\brief.md:stream',
  'C:\\Users\\Manager\\..\\Manager\\.workspacer\\brief.md',
  'C:\\Users\\Manager\\\\.workspacer\\brief.md',
])('rejects ambiguous or non-plain Windows pointer %s before opening a file', (pointer) => {
  const f = fixture();
  f.artifact.checkpoint.files[0].path = pointer;
  f.write();
  expect(f.validate).toThrow('path is not an allowed brief pointer');
  expect(fs.openSync).not.toHaveBeenCalled();
});
