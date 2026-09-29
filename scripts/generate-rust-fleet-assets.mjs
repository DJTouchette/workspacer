#!/usr/bin/env node
// Export the existing fleet wire text and portable examples, without a Node
// dependency in the Rust runtime. --check detects reference drift.
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { Script } from 'node:vm';
const root = new URL('../', import.meta.url);
const require = createRequire(new URL('apps/desktop/package.json', root));
const ts = require('typescript');
function load(name, extra = '') {
  const source = readFileSync(new URL(`apps/desktop/src/main/shared/${name}.ts`, root), 'utf8');
  const compiled = ts.transpileModule(source + extra, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } }).outputText;
  const exports = {};
  new Script(compiled).runInNewContext({ exports, require: (path) => {
    if (path !== './workerFailure') throw new Error(`Unexpected reference import ${path}`);
    return load('workerFailure');
  } }, { timeout: 1000 });
  return exports;
}
const reference = load('fleetMessages', '\nexport const __rustFleetAssets = { headers: HEADERS, alternateHeaders: ALT_HEADERS, tails: TAILS, ordinaryTails: ORDINARY_PARENT_TAILS, failedNote: FAILED_NOTE, ordinaryFailedNote: ORDINARY_PARENT_FAILED_NOTE, creditBalanceNote: CREDIT_BALANCE_NOTE, stoppedNote: STOPPED_NOTE };');
const cases = [];
for (const kind of Object.keys(reference.__rustFleetAssets.headers)) {
  for (const audience of ['manager', 'ordinary-parent']) {
    const entry = { label: 'Worker', sessionId: 'worker-1', cwd: '/project' };
    if (kind === 'progress') Object.assign(entry, { note: 'phase landed', needsDecision: true });
    else if (kind === 'threshold') entry.crossed = 'tokens 1,000 ≥ 900';
    else if (kind === 'blocked') entry.blockedOn = 'approval';
    else entry.lastReply = 'done';
    cases.push({ name: `${kind} for ${audience}`, kind, audience, entries: [entry], expected: reference.buildFleetMessage(kind, [entry], audience) });
  }
}
for (const [name, entries] of [
  ['all failed', [{ label: 'Failed', sessionId: 'one', failed: 'API failed' }]],
  ['credit balance advice', [{ label: 'Failed', sessionId: 'one', failed: 'Credit balance is too low' }]],
  ['mixed outcome', [{ label: 'Failed', sessionId: 'one', failed: 'API failed' }, { label: 'Done', sessionId: 'two', lastReply: 'done' }]],
  ['structured extras', [{ label: 'Done', sessionId: 'one', result: '{"commit":"abc"}', fullReply: 'Final summary', reviewEvidenceId: 'review-1' }]],
  ['missing result and stopped', [{ label: 'Stopped', sessionId: 'one', stopped: true, resultError: 'missing contract', escalationError: 'invalid marker' }]],
  ['validated escalation', [{ label: 'Blocked', sessionId: 'one', escalation: '{"status":"blocked"}' }]],
]) cases.push({ name, kind: 'worker-finished', audience: 'manager', entries, expected: reference.buildFleetMessage('worker-finished', entries) });
const excerpts = ['one\n two', 'x'.repeat(401), '😀'.repeat(201)].map(input => ({ input, expected: reference.excerptReply(input) }));
const corpus = { schemaVersion: 1, vocabulary: { blocks: {
  cases: { why: 'Wake text is a stored wire format shared with renderer cards.', required: ['name', 'kind', 'audience', 'entries', 'expected'], loaders: ['apps/desktop/src/main/shared/fleetMessages.test.ts::portableFleetMessageContracts', 'services/hub-rs/tests/fleet_messages.rs::shared_fleet_message_contracts'] },
  excerpts: { why: 'Single-line UTF16 excerpt limits remain compatible.', required: ['input', 'expected'], loaders: ['apps/desktop/src/main/shared/fleetMessages.test.ts::portableFleetMessageContracts', 'services/hub-rs/tests/fleet_messages.rs::shared_fleet_message_contracts'] }
} }, cases, excerpts };
for (const [path, value] of [['services/hub-rs/assets/fleet-messages.json', reference.__rustFleetAssets], ['contracts/fleet-message-cases.json', corpus]]) {
  const output = JSON.stringify(value, null, 2) + '\n';
  const target = new URL(path, root);
  if (process.argv.includes('--check')) { if (readFileSync(target, 'utf8') !== output) throw new Error(`${path} is stale`); }
  else writeFileSync(target, output);
}
