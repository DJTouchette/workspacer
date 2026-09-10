import { afterEach, expect, it } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import { createHash, randomUUID } from 'crypto';
import type { ReplacementRecord } from './managerReplacementState';
import {
  createArtifactPath,
  preparationPrompt,
  validateManagerArtifact,
} from './managerReplacementArtifact';

const dirs: string[] = [];
afterEach(() => dirs.splice(0).forEach((dir) => fs.rmSync(dir, { recursive: true, force: true })));
const hash = (bytes: Buffer) => createHash('sha256').update(bytes).digest('hex');
function fixture() {
  const cwd = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'handoff-artifact-')));
  dirs.push(cwd);
  const operationId = randomUUID();
  const artifactPath = createArtifactPath(cwd, operationId);
  const op = {
    operationId,
    sourceSessionId: 'source',
    artifactPath,
    launch: { options: { cwd } },
    workerIds: [],
    taskIds: [],
    metadata: [],
  } as unknown as ReplacementRecord;
  const brief = path.join(cwd, '.workspacer', 'brief.md');
  fs.writeFileSync(brief, Buffer.from('## Now\r\n- café: review pending\r\n'));
  const prompt = preparationPrompt(op);
  const artifact = JSON.parse(prompt.slice(prompt.indexOf('{\n'), prompt.lastIndexOf('\nInclude')));
  artifact.checkpoint.files[0].sha256 = hash(fs.readFileSync(brief));
  artifact.nextAction = 'Review pending decision';
  const write = () => {
    fs.writeFileSync(artifactPath, JSON.stringify(artifact, null, 2).replaceAll('\n', '\r\n'));
    return (
      '```wks-manager-handoff\n' +
      JSON.stringify({
        operationId,
        sourceSessionId: 'source',
        sha256: hash(fs.readFileSync(artifactPath)),
      }) +
      '\n```'
    );
  };
  return { cwd, op, brief, artifact, write, validate: () => validateManagerArtifact(op, write()) };
}

it('validates generated preparation JSON and seals the exact UTF-8/CRLF bytes', () => {
  const f = fixture();
  const result = f.validate();
  expect(Buffer.from(result.raw)).toEqual(fs.readFileSync(f.op.artifactPath));
  expect(result.hash).toBe(hash(fs.readFileSync(f.op.artifactPath)));
});

it.runIf(process.platform === 'win32')(
  'uses actual Windows files for mixed drive case and separators',
  () => {
    const f = fixture();
    f.op.launch.options.cwd = f.cwd
      .replaceAll('\\', '/')
      .replace(/^[A-Z]:/, (drive) => drive.toLowerCase());
    f.artifact.cwd = f.op.launch.options.cwd;
    f.artifact.checkpoint.files[0].path = f.brief.replace(/^[A-Z]:/, (drive) =>
      drive.toLowerCase(),
    );
    expect(f.validate().hash).toBe(hash(fs.readFileSync(f.op.artifactPath)));
  },
);

it.each(['invented', 'sha256:' + 'a'.repeat(64), 'A'.repeat(64), null])(
  'rejects malformed hash %s with a safe field diagnostic',
  (value) => {
    const f = fixture();
    f.artifact.checkpoint.files[0].sha256 = value;
    expect(f.validate).toThrow(
      'Checkpoint files[0] sha256 must be 64 lowercase hexadecimal characters',
    );
  },
);

it('rejects a well-formed synthesized hash', () => {
  const f = fixture();
  f.artifact.checkpoint.files[0].sha256 = 'a'.repeat(64);
  expect(f.validate).toThrow('sha256 does not match current file bytes');
});

it('rejects bytes changed after the receipt, including newline conversion', () => {
  const f = fixture();
  const receipt = f.write();
  fs.writeFileSync(f.brief, fs.readFileSync(f.brief, 'utf8').replaceAll('\r\n', '\n'));
  expect(() => validateManagerArtifact(f.op, receipt)).toThrow(
    'sha256 does not match current file bytes',
  );
});

it('hashes brief bytes without lossy UTF-8 decoding', () => {
  const f = fixture();
  fs.writeFileSync(f.brief, Buffer.from([0xff, 0x0d, 0x0a]));
  f.artifact.checkpoint.files[0].sha256 = hash(fs.readFileSync(f.brief));
  expect(f.validate().hash).toBe(hash(fs.readFileSync(f.op.artifactPath)));
});

it('rejects a proposal altered after its receipt independently of checkpoint hashes', () => {
  const f = fixture();
  const receipt = f.write();
  fs.appendFileSync(f.op.artifactPath, '\r\n');
  expect(() => validateManagerArtifact(f.op, receipt)).toThrow(
    'Handoff receipt identity or hash does not match',
  );
});

it.each(['../brief.md', 'other.md', 'brief.md/../brief.md'])(
  'rejects an extra invalid pointer %s',
  (name) => {
    const f = fixture();
    f.artifact.checkpoint.files.push({
      path: path.dirname(f.brief) + path.sep + name,
      sha256: f.artifact.checkpoint.files[0].sha256,
    });
    expect(f.validate).toThrow('Checkpoint files[1]');
  },
);

it('rejects an outside root without exposing its path', () => {
  const f = fixture();
  f.artifact.checkpoint.files.push({
    path: path.join(f.cwd + '-private', '.workspacer', 'brief.md'),
    sha256: f.artifact.checkpoint.files[0].sha256,
  });
  expect(f.validate).toThrow('Checkpoint files[1] path is not an allowed brief pointer');
});

it('rejects a brief symlink and a linked parent directory', () => {
  const f = fixture();
  const target = path.join(f.cwd, 'target.md');
  fs.renameSync(f.brief, target);
  fs.symlinkSync(target, f.brief, 'file');
  expect(f.validate).toThrow('path changed or is not a regular file without links');
  fs.unlinkSync(f.brief);
  fs.renameSync(target, f.brief);
  const project = path.join(f.cwd, 'project');
  fs.mkdirSync(project);
  fs.symlinkSync(path.dirname(f.brief), path.join(project, '.workspacer'), 'junction');
  f.op.projectCwds = [project];
  f.artifact.checkpoint.files.push({
    path: path.join(project, '.workspacer', 'brief.md'),
    sha256: f.artifact.checkpoint.files[0].sha256,
  });
  expect(f.validate).toThrow('path changed or is not a regular file without links');
});

it('rejects oversized briefs and invalid artifact UTF-8', () => {
  const f = fixture();
  fs.writeFileSync(f.brief, Buffer.alloc(2 * 1024 * 1024 + 1));
  expect(f.validate).toThrow('file exceeds 2 MiB');
  const receipt = f.write();
  fs.appendFileSync(f.op.artifactPath, Buffer.from([0xff]));
  expect(() => validateManagerArtifact(f.op, receipt)).toThrow('valid UTF-8 bytes');
});

it.runIf(process.platform === 'win32')(
  'verifies actual Windows root and file identities across component case',
  () => {
    const f = fixture();
    const project = path.join(f.cwd, 'Users', 'Name');
    const projectBrief = path.join(project, '.workspacer', 'brief.md');
    fs.mkdirSync(path.dirname(projectBrief), { recursive: true });
    fs.writeFileSync(projectBrief, '## Now\r\n- Review the verified handoff\r\n');
    f.op.projectCwds = [project];
    const pointer = path.join(project.toLowerCase(), '.workspacer', 'brief.md');
    expect(fs.statSync(pointer, { bigint: true }).ino).toBe(
      fs.statSync(projectBrief, { bigint: true }).ino,
    );
    f.artifact.checkpoint.files[0].path = path.join(f.cwd.toUpperCase(), '.workspacer', 'brief.md');
    f.artifact.checkpoint.files.push({
      path: pointer,
      sha256: hash(fs.readFileSync(projectBrief)),
    });
    expect(f.validate().hash).toBe(hash(fs.readFileSync(f.op.artifactPath)));
  },
);

it.runIf(process.platform === 'linux')(
  'rejects real distinct case-sensitive directories even with identical brief bytes',
  () => {
    const f = fixture();
    const allowed = path.join(f.cwd, 'Project');
    const other = path.join(f.cwd, 'project');
    for (const root of [allowed, other]) {
      fs.mkdirSync(path.join(root, '.workspacer'), { recursive: true });
      fs.copyFileSync(f.brief, path.join(root, '.workspacer', 'brief.md'));
    }
    expect(fs.statSync(allowed, { bigint: true }).ino).not.toBe(
      fs.statSync(other, { bigint: true }).ino,
    );
    f.op.projectCwds = [allowed];
    f.artifact.checkpoint.files.push({
      path: path.join(other, '.workspacer', 'brief.md'),
      sha256: hash(fs.readFileSync(f.brief)),
    });
    expect(f.validate).toThrow('path is not an allowed brief pointer');
  },
);
