import fs from 'fs';
import { randomUUID, createHash } from 'crypto';
import { atomicWriteFileSync } from '../lib/atomicWriteFile';
import { withConfigLock } from '../lib/configLock';
import { checkResultSchema } from '../shared/structuredResult';
import {
  WORKFLOW_STARTERS,
  validateWorkflow,
  type WorkflowDefinition,
  type WorkflowTemplate,
  type WorkflowPin,
} from '../shared/fleetWorkflow';
type Document = { version: 1; seeded: string[]; definitions: WorkflowDefinition[] };
export class WorkflowConflict extends Error {
  constructor(public currentRevision: number) {
    super(`Workflow changed; reload revision ${currentRevision} before saving`);
  }
}
/** Host-global, strict, bounded and reread under the cross-process lock on every operation. */
export class FleetWorkflowStore {
  constructor(
    private filename: () => string,
    private templates: () => WorkflowTemplate[],
    private selected: () => string[],
  ) {}
  private read(): Document {
    let doc: Document;
    try {
      if (fs.statSync(this.filename()).size > 2 * 1024 * 1024)
        throw new Error('Workflow store exceeds 2 MiB');
      doc = JSON.parse(fs.readFileSync(this.filename(), 'utf8'));
    } catch (e) {
      if ((e as NodeJS.ErrnoException).code !== 'ENOENT') throw e;
      doc = { version: 1, seeded: [], definitions: [] };
    }
    if (
      doc.version !== 1 ||
      Object.keys(doc).some((k) => !['version', 'seeded', 'definitions'].includes(k)) ||
      !Array.isArray(doc.seeded) ||
      !doc.seeded.every((x) => typeof x === 'string') ||
      !Array.isArray(doc.definitions) ||
      doc.definitions.length > 100
    )
      throw new Error('Invalid workflow store/version');
    doc.definitions = doc.definitions.map(validateWorkflow);
    if (new Set(doc.definitions.map((d) => d.id)).size !== doc.definitions.length)
      throw new Error('Duplicate workflow ids');
    for (const d of WORKFLOW_STARTERS)
      if (!doc.seeded.includes(d.id)) {
        if (!doc.definitions.some((x) => x.id === d.id)) doc.definitions.push(structuredClone(d));
        doc.seeded.push(d.id);
      }
    if (doc.definitions.length > 100 || Buffer.byteLength(JSON.stringify(doc)) > 2 * 1024 * 1024)
      throw new Error('Workflow store is full');
    return doc;
  }
  private transaction<T>(fn: (d: Document) => T): T {
    return withConfigLock(this.filename(), () => {
      const d = this.read();
      const result = fn(d);
      const json = JSON.stringify(d);
      if (Buffer.byteLength(json) > 2 * 1024 * 1024 || d.definitions.length > 100)
        throw new Error('Workflow store is full');
      atomicWriteFileSync(this.filename(), json, { mode: 0o600 });
      return structuredClone(result);
    });
  }
  list(): WorkflowDefinition[] {
    return this.transaction((d) => d.definitions);
  }
  validate(value: unknown): WorkflowDefinition {
    const d = validateWorkflow(value);
    const templates = this.templates();
    for (const s of d.steps) {
      const t = templates.find((t) => t.id === s.template);
      if (!t) throw new Error(`Dispatch template unavailable: ${s.template}`);
      if (s.instructions && !t.params.some((p) => p.name === 'task'))
        throw new Error(`Template ${t.id} needs a task input to carry step instructions`);
      if (checkResultSchema(t.resultSchema))
        throw new Error(`Template ${t.id} needs a result contract`);
    }
    return d;
  }
  mutate(
    op: 'create' | 'update' | 'clone' | 'disable' | 'delete',
    id?: string,
    revision?: number,
    value?: unknown,
    name?: string,
  ): WorkflowDefinition | undefined {
    return this.transaction((doc) => {
      const old = doc.definitions.find((d) => d.id === id);
      if (op !== 'create' && !old) throw new Error('Workflow unavailable');
      if (old && revision !== old.revision) throw new WorkflowConflict(old.revision);
      if (
        (op === 'update' || op === 'disable' || op === 'delete') &&
        WORKFLOW_STARTERS.some((d) => d.id === id)
      )
        throw new Error('Shipped starters are immutable; clone to customize');
      if ((op === 'delete' || op === 'disable') && this.selected().includes(id!))
        throw new Error('Select another workflow globally and in projects first');
      if (op === 'delete') {
        doc.definitions = doc.definitions.filter((d) => d.id !== id);
        return;
      }
      const d =
        op === 'clone'
          ? this.validate({
              ...old!,
              id: `workflow-${randomUUID()}`,
              revision: 1,
              name: name || `${old!.name} copy`,
              enabled: true,
            })
          : op === 'disable'
            ? { ...old!, enabled: false, revision: old!.revision + 1 }
            : this.validate({
                ...(value as object),
                ...(op === 'update'
                  ? { id: old!.id, revision: old!.revision + 1 }
                  : { revision: 1 }),
              });
      if (!d.enabled && this.selected().includes(d.id))
        throw new Error('Select another workflow globally and in projects first');
      if (op === 'create' && doc.definitions.some((x) => x.id === d.id))
        throw new Error('Workflow id already exists');
      if (op === 'update' || op === 'disable') doc.definitions[doc.definitions.indexOf(old!)] = d;
      else doc.definitions.push(d);
      return d;
    });
  }
  withDefinition<T>(id: string, fn: (d: WorkflowDefinition) => T): T {
    return withConfigLock(this.filename(), () => {
      const doc = this.read();
      // Finish seed persistence before a callback commits selection or task state.
      atomicWriteFileSync(this.filename(), JSON.stringify(doc), { mode: 0o600 });
      const d = doc.definitions.find((d) => d.id === id);
      if (!d?.enabled) throw new Error('Workflow unavailable or disabled');
      return fn(d);
    });
  }
  pin(d: WorkflowDefinition): WorkflowPin {
    this.validate(d);
    const available = this.templates();
    const templates: Record<string, WorkflowTemplate> = Object.create(null);
    for (const s of d.steps) {
      const t = available.find((t) => t.id === s.template)!;
      templates[t.id] =
        structuredClone(t); /* Required inputs are surfaced by next, checked before spawn. */
    }
    const snapshot = { definition: structuredClone(d), templates };
    if (Buffer.byteLength(JSON.stringify(snapshot)) > 256 * 1024)
      throw new Error('Pinned workflow exceeds 256 KiB');
    return {
      ...snapshot,
      hash: createHash('sha256').update(JSON.stringify(snapshot)).digest('hex'),
      steps: d.steps.map((s) => ({ id: s.id, state: 'planned' })),
    };
  }
}
