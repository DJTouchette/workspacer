#!/usr/bin/env node
// Build-time export of the shared desktop workflow definitions.
import { readFileSync, writeFileSync } from 'node:fs';
import { Script } from 'node:vm';
const root = new URL('../', import.meta.url);
const source = readFileSync(new URL('apps/desktop/src/main/shared/fleetWorkflow.ts', root), 'utf8');
const id = source.match(/export const DEFAULT_WORKFLOW_ID = ('[^']+');/);
const step = source.match(/\): WorkflowStep => \((\{[\s\S]*?\n\})\);/);
const starters = source.match(/export const WORKFLOW_STARTERS: WorkflowDefinition\[\] = (\[[\s\S]*?\n\]);/);
if (!id || !step || !starters) throw new Error('Workflow starter declaration changed; update generator');
const program = `const DEFAULT_WORKFLOW_ID = ${id[1]}; const step = (id,kind,stage,role,template) => (${step[1]}); JSON.stringify(${starters[1]}, null, 2)`;
const output = new Script(program).runInNewContext({}, { timeout: 1000 }) + '\n';
const target = new URL('services/hub-rs/assets/workflow-starters.json', root);
if (process.argv.includes('--check')) { if (readFileSync(target, 'utf8') !== output) throw new Error('Rust workflow assets are stale'); }
else writeFileSync(target, output);
