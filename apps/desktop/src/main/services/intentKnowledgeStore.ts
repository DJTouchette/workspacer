import fs from 'node:fs';
import path from 'node:path';
import { createHash, randomUUID } from 'node:crypto';
import type { DatabaseSync } from 'node:sqlite';
import type { IntentWorkspace } from '../shared/intentWorkspace';
import type {
  IntentFinding,
  IntentKnowledgeCapture,
  IntentKnowledgeDocument,
  IntentKnowledgePromotion,
  IntentKnowledgeResponse,
} from '../shared/intentKnowledge';
import {
  withIntentArtifactDirectory,
  readIntentFile,
  readIntentFiles,
  writeIntentFile,
} from './intentArtifactFiles';

export const INTENT_KNOWLEDGE_SCHEMA = `
CREATE TABLE IF NOT EXISTS intent_knowledge (id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL REFERENCES intent_workspaces(id), kind TEXT NOT NULL, snapshot TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS intent_knowledge_workspace ON intent_knowledge(workspace_id,kind);
`;
const hash = (text: string) => createHash('sha256').update(text).digest('hex');
const MAX_DOC = 128 * 1024;
function text(value: unknown, label: string, max = 128): string {
  if (typeof value !== 'string' || !value.trim() || value.includes('\0') || value.length > max)
    throw new Error(`Invalid ${label}`);
  return value.trim();
}
function stableId(value: unknown): string {
  const id = text(value, 'record ID');
  if (!/^[A-Za-z0-9_-]+$/.test(id)) throw new Error('Invalid record ID');
  return id;
}

/** Explicit project knowledge reads and reviewed writes. No provider execution,
 * automatic promotion, or arbitrary paths from the renderer are involved. */
export class IntentKnowledgeStore {
  constructor(private db: DatabaseSync) {}
  private transaction<T>(run: () => T): T {
    this.db.exec('BEGIN IMMEDIATE');
    try {
      const result = run();
      this.db.exec('COMMIT');
      return result;
    } catch (error) {
      this.db.exec('ROLLBACK');
      throw error;
    }
  }
  private workspace(id: string, revision?: unknown): IntentWorkspace {
    const row = this.db.prepare('SELECT snapshot FROM intent_workspaces WHERE id=?').get(id);
    if (!row) throw new Error('Workspace no longer exists');
    const workspace = JSON.parse(String(row.snapshot)) as IntentWorkspace;
    if (revision !== undefined && workspace.revision !== revision)
      throw new Error('Intent changed. Reload the saved revision before continuing.');
    return workspace;
  }
  private records<T>(id: string, kind: string): T[] {
    return this.db
      .prepare(
        'SELECT snapshot FROM intent_knowledge WHERE workspace_id=? AND kind=? ORDER BY rowid DESC',
      )
      .all(id, kind)
      .map((row) => JSON.parse(String(row.snapshot)));
  }
  private record<T>(recordId: string, workspaceId: string, kind: string): T | undefined {
    const row = this.db
      .prepare('SELECT workspace_id,kind,snapshot FROM intent_knowledge WHERE id=?')
      .get(recordId);
    if (!row) return undefined;
    if (row.workspace_id !== workspaceId || row.kind !== kind)
      throw new Error('Record belongs to another workspace or record type');
    return JSON.parse(String(row.snapshot));
  }
  private insert<T extends IntentKnowledgeCapture | IntentFinding | IntentKnowledgePromotion>(
    id: string,
    workspaceId: string,
    kind: string,
    value: T,
  ): T {
    return this.transaction(() => {
      const existing = this.record<T>(id, workspaceId, kind);
      if (existing) {
        const identity = (record: T) =>
          JSON.stringify(
            Object.fromEntries(
              Object.entries(record).filter(
                ([key]) => !['createdAt', 'capturedAt', 'status', 'detail'].includes(key),
              ),
            ),
          );
        if (identity(existing) !== identity(value))
          throw new Error('Record ID already belongs to different content');
        return existing;
      }
      this.workspace(workspaceId, value.intentRevision);
      this.db
        .prepare('INSERT INTO intent_knowledge VALUES (?,?,?,?)')
        .run(id, workspaceId, kind, JSON.stringify(value));
      return value;
    });
  }
  private save(proposal: IntentKnowledgePromotion): void {
    this.db
      .prepare('UPDATE intent_knowledge SET snapshot=? WHERE id=?')
      .run(JSON.stringify(proposal), proposal.id);
  }
  private projectPath(root: string, relative: string, allowMissing = false): string {
    if (
      !/^\.rivet\/(context\/(domains|modules|paradigms)|learnings)\/[A-Za-z0-9][A-Za-z0-9_.-]*\.md$/.test(
        relative,
      ) ||
      relative.includes('..')
    )
      throw new Error('Choose a Rivet Markdown document in this project');
    const base = fs.realpathSync(root);
    let current = base;
    const parts = relative.split('/');
    for (let i = 0; i < parts.length; i++) {
      current = path.join(current, parts[i]);
      try {
        const stat = fs.lstatSync(current);
        if (stat.isSymbolicLink() || (i < parts.length - 1 ? !stat.isDirectory() : !stat.isFile()))
          throw new Error('Rivet paths must be ordinary project directories and files');
      } catch (error) {
        if (!allowMissing || (error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
      }
    }
    return current;
  }
  private read(root: string, relative: string): string {
    const file = this.projectPath(root, relative);
    if (process.platform === 'win32')
      return readIntentFile(path.dirname(file), path.basename(file), MAX_DOC).toString('utf8');
    return withIntentArtifactDirectory(path.dirname(file), false, (access) =>
      this.readBoundFile(access.leafPath(path.basename(file))),
    );
  }
  private readBoundFile(file: string): string {
    const fd = fs.openSync(file, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
    try {
      const stat = fs.fstatSync(fd);
      if (!stat.isFile() || stat.nlink !== 1 || stat.size > MAX_DOC)
        throw new Error('Rivet document exceeds 128 KiB');
      const buffer = Buffer.alloc(MAX_DOC + 1);
      const count = fs.readSync(fd, buffer, 0, buffer.length, 0);
      if (count > MAX_DOC) throw new Error('Rivet document exceeds 128 KiB');
      return buffer.subarray(0, count).toString('utf8');
    } finally {
      fs.closeSync(fd);
    }
  }
  private documents(root: string): { available: boolean; documents: IntentKnowledgeDocument[] } {
    const documents: IntentKnowledgeDocument[] = [];
    for (const category of ['domains', 'modules', 'paradigms']) {
      const dir = path.join(root, '.rivet', 'context', category);
      let entries: fs.Dirent[];
      try {
        entries = fs.readdirSync(dir, { withFileTypes: true });
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code === 'ENOENT') continue;
        throw error;
      }
      const files = entries.filter(
        (entry) =>
          entry.isFile() &&
          /^[A-Za-z0-9][A-Za-z0-9_.-]*\.md$/.test(entry.name) &&
          !entry.name.includes('..'),
      );
      if (documents.length + files.length > 256)
        throw new Error('Rivet context listing exceeds 256 documents');
      const batch =
        process.platform === 'win32' && files.length
          ? readIntentFiles(
              dir,
              files.map((entry) => entry.name),
              MAX_DOC,
            )
          : undefined;
      for (const [index, entry] of files.entries()) {
        const relative = `.rivet/context/${category}/${entry.name}`;
        const content = batch ? batch[index].toString('utf8') : this.read(root, relative);
        documents.push({
          path: relative,
          title: /^#\s+(.+)$/m.exec(content)?.[1] || entry.name,
          sha256: hash(content),
          bytes: Buffer.byteLength(content),
        });
      }
    }
    return {
      available: documents.length > 0,
      documents: documents.sort((a, b) => a.path.localeCompare(b.path)),
    };
  }
  contextPacket(workspaceId: string): string {
    const current = new Map<string, IntentKnowledgeCapture>();
    for (const capture of this.records<IntentKnowledgeCapture>(workspaceId, 'capture'))
      if (!current.has(capture.path)) current.set(capture.path, capture);
    let bytes = 0;
    const chunks: string[] = [];
    for (const capture of current.values()) {
      const header = `${capture.path} (sha256 ${capture.sha256}, captured ${capture.capturedAt})`;
      const remaining = 32_000 - bytes - header.length - 100;
      if (remaining <= 0) {
        chunks.push(
          '[Further captured documents omitted from this packet; inspect saved knowledge records.]',
        );
        break;
      }
      const content = capture.content.slice(0, remaining);
      const chunk = `${header}\n${content}${content.length < capture.content.length ? '\n[Snapshot excerpt; inspect the saved knowledge record for full content.]' : ''}`;
      chunks.push(chunk);
      bytes += chunk.length;
    }
    return chunks.join('\n\n');
  }
  request(input: Record<string, unknown>): IntentKnowledgeResponse {
    const id = text(input.id, 'workspace ID');
    const workspace = this.workspace(id);
    if (input.action === 'knowledge')
      return {
        action: 'knowledge',
        ...this.documents(workspace.projectRoot),
        captures: this.records(id, 'capture'),
        findings: this.records(id, 'finding'),
        proposals: this.records(id, 'proposal'),
      };
    if (
      input.action === 'publishKnowledgePromotion' ||
      input.action === 'reconcileKnowledgePromotion'
    )
      return this.publish(id, input);
    if (!Number.isSafeInteger(input.expectedRevision) || Number(input.expectedRevision) < 1)
      throw new Error('A saved intent revision is required');
    return (() => {
      this.workspace(id, input.expectedRevision);
      if (input.action === 'captureKnowledge') {
        const captureId = stableId(input.captureId),
          relative = text(input.path, 'document path', 4096);
        const existing = this.record<IntentKnowledgeCapture>(captureId, id, 'capture');
        if (existing) {
          if (existing.path !== relative || existing.sha256 !== input.expectedSha256)
            throw new Error('Capture ID already belongs to different content');
          return { action: 'captureKnowledge', capture: existing };
        }
        if (!relative.startsWith('.rivet/context/'))
          throw new Error('Only curated context documents can be captured');
        const content = this.read(workspace.projectRoot, relative);
        if (hash(content) !== input.expectedSha256)
          throw new Error('Rivet document changed. Refresh and review its new version.');
        const capture: IntentKnowledgeCapture = {
          id: captureId,
          workspaceId: id,
          intentRevision: workspace.revision,
          path: relative,
          title: /^#\s+(.+)$/m.exec(content)?.[1] || path.basename(relative),
          sha256: hash(content),
          bytes: Buffer.byteLength(content),
          content,
          capturedAt: new Date().toISOString(),
        };
        return {
          action: 'captureKnowledge',
          capture: this.insert(capture.id, id, 'capture', capture),
        };
      }
      if (input.action === 'recordFinding') {
        const findingId = stableId(input.findingId),
          title = text(input.title, 'finding title', 240),
          observation = text(input.observation, 'finding observation', 16000);
        if (
          !Array.isArray(input.captureIds) ||
          input.captureIds.length > 32 ||
          input.captureIds.some(
            (captureId) => typeof captureId !== 'string' || !this.record(captureId, id, 'capture'),
          )
        )
          throw new Error('Select knowledge captures from this workspace');
        const captureIds = [...new Set(input.captureIds as string[])];
        const prior = this.record<IntentFinding>(findingId, id, 'finding');
        if (prior) {
          if (
            prior.title !== title ||
            prior.observation !== observation ||
            JSON.stringify(prior.captureIds) !== JSON.stringify(captureIds)
          )
            throw new Error('Finding ID already belongs to different content');
          return { action: 'recordFinding', finding: prior };
        }
        const finding: IntentFinding = {
          id: findingId,
          workspaceId: id,
          intentRevision: workspace.revision,
          title,
          observation,
          captureIds,
          author: 'user',
          createdAt: new Date().toISOString(),
        };
        return {
          action: 'recordFinding',
          finding: this.insert(finding.id, id, 'finding', finding),
        };
      }
      if (input.action !== 'prepareKnowledgePromotion') throw new Error('Unknown knowledge action');
      const proposalId = stableId(input.proposalId),
        findingId = stableId(input.findingId),
        kind = input.kind;
      if (kind !== 'learning' && kind !== 'context')
        throw new Error('Choose learning capture or context promotion');
      const finding = this.record<IntentFinding>(findingId, id, 'finding');
      if (!finding) throw new Error('Finding does not belong to this workspace');
      const relative =
        kind === 'learning'
          ? `.rivet/learnings/${new Date().toISOString().slice(0, 10)}-intent-${proposalId}.md`
          : text(input.path, 'context path', 4096);
      if (kind === 'context' && !relative.startsWith('.rivet/context/'))
        throw new Error('Choose a curated context document');
      const prior = this.record<IntentKnowledgePromotion>(proposalId, id, 'proposal');
      if (prior) {
        if (
          prior.findingId !== findingId ||
          prior.kind !== kind ||
          (kind === 'context' && prior.path !== relative)
        )
          throw new Error('Proposal ID already belongs to another promotion');
        return { action: 'prepareKnowledgePromotion', proposal: prior };
      }
      const before = kind === 'context' ? this.read(workspace.projectRoot, relative) : null;
      if (
        kind === 'learning' &&
        fs.existsSync(this.projectPath(workspace.projectRoot, relative, true))
      )
        throw new Error('Learning file already exists');
      const provenance = finding.captureIds
        .map((captureId) => {
          const capture = this.record<IntentKnowledgeCapture>(captureId, id, 'capture')!;
          return `- ${capture.path} (sha256 ${capture.sha256})`;
        })
        .join('\n');
      const findingText = `## ${finding.title.replace(/[\r\n]/g, ' ')}\n\n${finding.observation}\n\nSource: intent workspace ${id}, revision ${finding.intentRevision}.\n${provenance}\n`;
      const content =
        kind === 'context'
          ? `${before!.trimEnd()}\n\n${findingText}`
          : `---\ntitle: ${JSON.stringify(finding.title)}\ndate: ${new Date().toISOString().slice(0, 10)}\nconfidence: medium\npromoted: false\n---\n\n${findingText}`;
      if (Buffer.byteLength(content) > MAX_DOC)
        throw new Error('Proposed document exceeds 128 KiB');
      const proposal: IntentKnowledgePromotion = {
        id: proposalId,
        workspaceId: id,
        findingId,
        intentRevision: workspace.revision,
        projectRoot: workspace.projectRoot,
        kind,
        path: relative,
        previousSha256: before === null ? null : hash(before),
        content,
        createdAt: new Date().toISOString(),
        status: 'draft',
        detail: 'Saved for review; no project file changed.',
      };
      return {
        action: 'prepareKnowledgePromotion',
        proposal: this.insert(proposal.id, id, 'proposal', proposal),
      };
    })();
  }
  private publish(id: string, input: Record<string, unknown>): IntentKnowledgeResponse {
    const proposalId = stableId(input.proposalId);
    const proposal = this.record<IntentKnowledgePromotion>(proposalId, id, 'proposal');
    if (!proposal) throw new Error('Promotion does not belong to this workspace');
    const workspace = this.workspace(id);
    if (workspace.projectRoot !== proposal.projectRoot)
      throw new Error('Project root moved. Prepare a new promotion.');
    if (input.action === 'reconcileKnowledgePromotion') {
      if (
        proposal.status === 'unknown' &&
        hash(this.read(proposal.projectRoot, proposal.path)) === hash(proposal.content)
      ) {
        proposal.status = 'written';
        proposal.detail = 'Host verified that the project file matches the reviewed proposal.';
        this.transaction(() => this.save(proposal));
      }
      return { action: 'reconcileKnowledgePromotion', proposal };
    }
    if (proposal.status !== 'draft') return { action: 'publishKnowledgePromotion', proposal };
    this.workspace(id, proposal.intentRevision);
    const target = this.projectPath(proposal.projectRoot, proposal.path, true);
    const matches = () =>
      proposal.previousSha256 === null
        ? !fs.existsSync(target)
        : hash(this.read(proposal.projectRoot, proposal.path)) === proposal.previousSha256;
    if (!matches())
      throw new Error('Project document changed. Prepare and review a new promotion.');
    const claimed = this.transaction(() => {
      const current = this.record<IntentKnowledgePromotion>(proposalId, id, 'proposal')!;
      if (current.status !== 'draft') return current;
      this.workspace(id, proposal.intentRevision);
      proposal.status = 'unknown';
      proposal.detail = 'File write started; completion has not been recorded.';
      this.save(proposal);
      return undefined;
    });
    if (claimed) return { action: 'publishKnowledgePromotion', proposal: claimed };
    try {
      if (process.platform === 'win32')
        writeIntentFile(
          path.dirname(target),
          path.basename(target),
          Buffer.from(proposal.content, 'utf8'),
          proposal.previousSha256,
          MAX_DOC,
        );
      else
        withIntentArtifactDirectory(path.dirname(target), true, (access) => {
          const anchoredTarget = access.leafPath(path.basename(target));
          const writeNew = (file: string, mode: number) => {
            const fd = fs.openSync(
              file,
              fs.constants.O_WRONLY |
                fs.constants.O_CREAT |
                fs.constants.O_EXCL |
                fs.constants.O_NOFOLLOW,
              mode,
            );
            try {
              fs.writeFileSync(fd, proposal.content, 'utf8');
              fs.fsyncSync(fd);
            } finally {
              fs.closeSync(fd);
            }
          };
          if (proposal.previousSha256 === null) writeNew(anchoredTarget, 0o600);
          else {
            const temporary = access.leafPath(`intent-${randomUUID()}.tmp`);
            try {
              writeNew(temporary, fs.lstatSync(anchoredTarget).mode & 0o777);
              if (
                hash(this.readBoundFile(anchoredTarget)) !== proposal.previousSha256 ||
                !matches()
              )
                throw new Error('Project document changed during promotion');
              fs.renameSync(temporary, anchoredTarget);
            } finally {
              try {
                fs.unlinkSync(temporary);
              } catch (error) {
                if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
              }
            }
          }
        });
      if (hash(this.read(proposal.projectRoot, proposal.path)) !== hash(proposal.content))
        throw new Error('Project file no longer matches the reviewed content');
      proposal.status = 'written';
      proposal.detail = 'Reviewed content written to the project.';
      this.transaction(() => this.save(proposal));
    } catch (error) {
      throw new Error(
        `Promotion outcome needs inspection: ${error instanceof Error ? error.message : String(error)}. Check the file before preparing another write.`,
      );
    }
    return { action: 'publishKnowledgePromotion', proposal };
  }
}
