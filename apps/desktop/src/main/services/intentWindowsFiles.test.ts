import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { afterEach, describe, expect, it, vi } from 'vitest';
vi.mock('./configService', () => ({
  getConfigDir: () => path.join(os.tmpdir(), 'unused-intent-windows-config'),
}));
import {
  readIntentFile,
  readIntentFiles,
  removeIntentFile,
  writeIntentFile,
} from './intentArtifactFiles';
import { INTENT_WINDOWS_NATIVE_SOURCE, runIntentWindowsFiles } from './intentWindowsFiles';
import { captureIntentGit } from './intentEvidenceCapture';
import { DatabaseSync } from 'node:sqlite';
import { IntentWorkspaceStore } from './intentWorkspaceStore';
import { IntentEvidenceStore } from './intentEvidenceStore';
import { execFileSync } from 'node:child_process';
const roots: string[] = [];
const hash = (value: string | Buffer) => createHash('sha256').update(value).digest('hex');
const root = () => {
  const value = fs.mkdtempSync(path.join(os.tmpdir(), 'intent-win-'));
  roots.push(value);
  return value;
};
afterEach(() => {
  for (const dir of roots.splice(0)) fs.rmSync(dir, { recursive: true, force: true });
});

describe.runIf(process.platform === 'win32')(
  'secure Windows intent files (real PowerShell 5.1 / Win32 handles)',
  () => {
    it('holds native parent handles that deny renaming until the operation lease is released', () => {
      const directory = path.join(root(), 'pinned');
      fs.mkdirSync(directory);
      const script = `$ErrorActionPreference='Stop'; Add-Type -TypeDefinition @'\n${INTENT_WINDOWS_NATIVE_SOURCE}\n'@\n$r=([Console]::In.ReadToEnd()|ConvertFrom-Json); $type=[IntentNativeFiles].GetNestedType('DirectoryLease',[System.Reflection.BindingFlags]::NonPublic); $lease=[Activator]::CreateInstance($type,[object[]]@([string]$r.directory,$false)); $blocked=$false; try { try {[IO.Directory]::Move($r.directory,$r.moved)} catch {$blocked=$true} } finally {$lease.Dispose()}; if(-not $blocked){throw 'Parent directory was renamed while its secure lease was active'}; [IO.Directory]::Move($r.directory,$r.moved); [IO.Directory]::Move($r.moved,$r.directory); '{"blocked":true}'`;
      const output = execFileSync(
        path.join(
          process.env.SystemRoot || 'C:\\Windows',
          'System32/WindowsPowerShell/v1.0/powershell.exe',
        ),
        ['-NoLogo', '-NoProfile', '-NonInteractive', '-Command', script],
        {
          input: JSON.stringify({ directory, moved: directory + '-moved' }),
          encoding: 'utf8',
          windowsHide: true,
          timeout: 30000,
        },
      );
      expect(JSON.parse(output)).toEqual({ blocked: true });
    }, 30000);

    it('roundtrips Unicode and bounded binary bytes, batches reads, and guards replacement and cleanup by digest', () => {
      const directory = path.join(root(), '新しい folder', 'nested');
      const bytes = Buffer.from('héllo 世界\n');
      writeIntentFile(directory, 'note.md', bytes);
      expect(readIntentFile(directory, 'note.md', 512)).toEqual(bytes);
      expect(() => writeIntentFile(directory, 'note.md', Buffer.from('duplicate'))).toThrow();
      expect(() =>
        writeIntentFile(directory, 'note.md', Buffer.from('wrong'), hash('stale')),
      ).toThrow('changed');
      writeIntentFile(directory, 'note.md', Buffer.from('new version'), hash(bytes));
      writeIntentFile(directory, 'other.bin', Buffer.from([1, 2, 3, 255]));
      expect(readIntentFiles(directory, ['note.md', 'other.bin'], 512)).toEqual([
        Buffer.from('new version'),
        Buffer.from([1, 2, 3, 255]),
      ]);
      expect(() => readIntentFile(directory, 'note.md', 2)).toThrow('limit');
      expect(() => removeIntentFile(directory, 'note.md', hash('different'))).toThrow('changed');
      removeIntentFile(directory, 'note.md', hash('new version'));
      expect(fs.existsSync(path.join(directory, 'note.md'))).toBe(false);
    }, 60000);

    it('rejects device namespaces, ADS, traversal, drive-relative paths and reserved DOS names', () => {
      const directory = root();
      for (const leaf of [
        '../escape',
        'x:stream',
        'NUL.txt',
        'COM1',
        'LPT².txt',
        'file.',
        'file ',
        'folder\\file',
        'CONOUT$',
      ])
        expect(() => writeIntentFile(directory, leaf, Buffer.from('never'))).toThrow();
      for (const dir of [
        'C:relative',
        '\\relative',
        '\\\\?\\C:\\Windows',
        '\\\\.\\C:\\Windows',
        directory + '\\..\\escape',
      ])
        expect(() =>
          runIntentWindowsFiles({
            action: 'write',
            directory: dir,
            leaf: 'test.md',
            data: 'eA==',
            limit: 10,
          }),
        ).toThrow();
      expect(fs.readdirSync(directory)).toEqual([]);
    }, 60000);

    it('rejects junctions, reparse-point leaves, and hard links without opening outside bytes', () => {
      const directory = root(),
        outside = root();
      fs.writeFileSync(path.join(outside, 'secret.md'), 'OUTSIDE SECRET');
      fs.symlinkSync(outside, path.join(directory, 'junction'), 'junction');
      expect(() => readIntentFile(path.join(directory, 'junction'), 'secret.md', 512)).toThrow();
      expect(() =>
        writeIntentFile(path.join(directory, 'junction', 'new'), 'never.md', Buffer.from('never')),
      ).toThrow();
      fs.linkSync(path.join(outside, 'secret.md'), path.join(directory, 'hardlink.md'));
      expect(() => readIntentFile(directory, 'hardlink.md', 512)).toThrow('nonordinary');
      expect(fs.readdirSync(outside)).toEqual(['secret.md']);
    }, 30000);

    it('cannot follow a parent being raced between an ordinary directory and outside junction', async () => {
      const directory = root(),
        outside = root(),
        active = path.join(directory, 'active');
      fs.mkdirSync(active);
      fs.writeFileSync(path.join(active, 'value.md'), 'INSIDE');
      fs.writeFileSync(path.join(outside, 'value.md'), 'OUTSIDE');
      const worker = spawn(
        process.execPath,
        [
          '-e',
          `const fs=require('fs');const [active,outside]=process.argv.slice(1);const moved=active+'-held';let stopped=false;process.on('message',()=>stopped=true);process.send('ready');function step(){try{fs.renameSync(active,moved);fs.symlinkSync(outside,active,'junction');fs.unlinkSync(active);fs.renameSync(moved,active);}catch{try{if(fs.existsSync(moved)){try{fs.unlinkSync(active);}catch{}fs.renameSync(moved,active);}}catch{}}if(stopped){process.send('stopped');return;}setImmediate(step);}step();`,
          active,
          outside,
        ],
        { stdio: ['ignore', 'ignore', 'pipe', 'ipc'] },
      );
      try {
        await new Promise<void>((resolve, reject) => {
          worker.once('message', () => resolve());
          worker.once('error', reject);
        });
        for (let i = 0; i < 5; i++) {
          let bytes: Buffer | undefined;
          try {
            bytes = readIntentFile(active, 'value.md', 512);
          } catch {
            /* A sharing violation or an observed junction must fail closed. */
          }
          if (bytes) expect(bytes.toString()).toBe('INSIDE');
        }
      } finally {
        worker.send('stop');
        await new Promise<void>((resolve) => {
          worker.once('message', () => {
            worker.kill();
            resolve();
          });
          setTimeout(() => {
            worker.kill();
            resolve();
          }, 1000);
        });
      }
      expect(fs.readFileSync(path.join(outside, 'value.md'), 'utf8')).toBe('OUTSIDE');
    }, 60000);

    it('persists artifact and evidence bytes and publishes/reconciles reviewed Rivet knowledge through the production store', async () => {
      const directory = root(),
        project = path.join(directory, 'project');
      fs.mkdirSync(path.join(project, '.rivet/context/modules'), { recursive: true });
      fs.writeFileSync(
        path.join(project, '.rivet/context/modules/test.md'),
        '# Native context\nOriginal.',
      );
      const filename = path.join(directory, 'intent.sqlite');
      let db = new DatabaseSync(filename);
      let store = new IntentWorkspaceStore(db);
      try {
        const created = store.request({
          action: 'create',
          projectRoot: project,
          fields: {
            title: 'Windows work',
            outcome: '',
            constraints: '',
            successCriteria: 'Works on Windows',
            sourceUrl: '',
            status: 'active',
          },
        });
        if (created.action !== 'create') throw new Error('Expected workspace');
        const id = created.workspace.id;
        const attached = store.request({
          action: 'attachSession',
          id,
          expectedRevision: 1,
          session: {
            sessionId: 'worker',
            hub: '',
            label: 'Windows worker',
            cwd: project,
            provider: 'codex',
          },
        });
        if (attached.action !== 'attachSession') throw new Error('Expected linked execution');
        const captured = await new IntentEvidenceStore(db, {
          capture: async () => ({
            cwd: project,
            repositoryRoot: project,
            headCommit: 'a'.repeat(40),
            capturedAt: new Date().toISOString(),
            scope: 'tracked-working-tree-against-head',
            changedFiles: ['result.ts'],
            omissions: [],
            artifact: '+Windows evidence bytes',
          }),
        }).capture(
          {
            action: 'captureEvidence',
            id,
            expectedRevision: 1,
            evidenceId: 'evidence',
            executionId: attached.execution.id,
            criterionId: 'r1:c1',
          },
          [{ sessionId: 'worker', cwd: project }],
        );
        expect(captured.action).toBe('captureEvidence');
        const artifact = store.request({
          action: 'addArtifact',
          id,
          expectedRevision: 1,
          artifactId: 'artifact',
          title: 'Windows note',
          mimeType: 'text/markdown',
          dataBase64: Buffer.from('Retained note').toString('base64'),
        });
        expect(artifact.action).toBe('addArtifact');
        const knowledge = store.request({ action: 'knowledge', id });
        if (knowledge.action !== 'knowledge') throw new Error('Expected knowledge');
        store.request({
          action: 'captureKnowledge',
          id,
          expectedRevision: 1,
          captureId: 'capture',
          path: knowledge.documents[0].path,
          expectedSha256: knowledge.documents[0].sha256,
        });
        store.request({
          action: 'recordFinding',
          id,
          expectedRevision: 1,
          findingId: 'finding',
          title: 'Windows handles',
          observation: 'Use pinned handles.',
          captureIds: ['capture'],
        });
        const prepared = store.request({
          action: 'prepareKnowledgePromotion',
          id,
          expectedRevision: 1,
          proposalId: 'promotion',
          findingId: 'finding',
          kind: 'context',
          path: knowledge.documents[0].path,
        });
        if (prepared.action !== 'prepareKnowledgePromotion') throw new Error('Expected promotion');
        db.exec(
          "CREATE TRIGGER fail_written BEFORE UPDATE ON intent_knowledge WHEN json_extract(new.snapshot,'$.status')='written' BEGIN SELECT RAISE(ABORT,'receipt failed'); END",
        );
        expect(() =>
          store.request({ action: 'publishKnowledgePromotion', id, proposalId: 'promotion' }),
        ).toThrow('inspection');
        db.exec('DROP TRIGGER fail_written');
        expect(
          store.request({ action: 'reconcileKnowledgePromotion', id, proposalId: 'promotion' }),
        ).toMatchObject({ proposal: { status: 'written' } });
        db.close();
        db = new DatabaseSync(filename);
        store = new IntentWorkspaceStore(db);
        expect(store.request({ action: 'readArtifact', id, artifactId: 'artifact' })).toMatchObject(
          { dataBase64: Buffer.from('Retained note').toString('base64') },
        );
        expect(store.request({ action: 'readEvidence', id, evidenceId: 'evidence' })).toMatchObject(
          { artifact: '+Windows evidence bytes' },
        );
      } finally {
        if (db.isOpen) db.close();
      }
    }, 60000);

    it('captures real Git for Windows staged/unstaged evidence with isolated metadata and inherited configuration disabled', async () => {
      const directory = root();
      const git = (...args: string[]) =>
        execFileSync('git', args, { cwd: directory, stdio: ['ignore', 'pipe', 'pipe'] });
      git('init');
      git('config', 'user.name', 'Intent test');
      git('config', 'user.email', 'test@example.com');
      fs.writeFileSync(path.join(directory, 'result.txt'), 'before\n');
      git('add', '.');
      git('commit', '-m', 'Baseline');
      fs.writeFileSync(path.join(directory, 'result.txt'), 'after\n');
      const captured = await captureIntentGit(directory);
      const { canonicalRoot, isWithin, isSecretPath } = await import('../lib/pathConfinement');
      const absolute = path.resolve(captured.repositoryRoot, 'result.txt');
      const parent = canonicalRoot(path.dirname(absolute));
      const diagnostics = JSON.stringify(
        {
          directory,
          canonicalDirectory: fs.realpathSync(directory),
          gitRoot: git('rev-parse', '--show-toplevel').toString().trim(),
          originalStatus: git('status', '--porcelain=v1', '-z').toString(),
          canonicalScope: canonicalRoot(captured.cwd),
          absolute,
          parent,
          lexicalWithin: isWithin(absolute, captured.cwd),
          parentWithin: parent ? isWithin(parent, captured.cwd) : false,
          secret: parent ? isSecretPath(path.join(parent, 'result.txt')) : null,
          captured,
        },
        null,
        2,
      );
      expect(captured.changedFiles, diagnostics).toContain('result.txt');
      expect(captured.artifact, diagnostics).toContain('+after');
    }, 60000);
  },
);
