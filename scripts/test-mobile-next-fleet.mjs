#!/usr/bin/env node
// /m-next's fleet-wake parser (assets/web/m-next/js/fleet.js) against the
// shared grammar fixtures (contracts/fleet-message-cases.json): every wake the
// writer produces must parse back to its kind and entries.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const { parseFleetMessage, buildResultView } = await import(
  pathToFileURL(path.join(ROOT, 'services/hub-rs/assets/web/m-next/js/fleet.js')).href
);
const { cases } = JSON.parse(fs.readFileSync(path.join(ROOT, 'contracts/fleet-message-cases.json'), 'utf8'));
const failures = [];
const fields = ['label', 'sessionId', 'cwd', 'blockedOn', 'stopped', 'failed', 'crossed', 'needsDecision'];
for (const c of cases) {
  const parsed = parseFleetMessage(c.expected);
  const kind = c.kind === 'worker-finished' && c.entries.every((e) => e.failed) ? 'worker-finished' : c.kind;
  if (!parsed) { failures.push(`${c.name}: did not parse`); continue; }
  if (parsed.kind !== kind) failures.push(`${c.name}: kind ${parsed.kind} != ${kind}`);
  if (parsed.entries.length !== c.entries.length) { failures.push(`${c.name}: ${parsed.entries.length} entries != ${c.entries.length}`); continue; }
  c.entries.forEach((want, i) => {
    const got = parsed.entries[i];
    for (const f of fields) {
      if (want[f] === undefined || want[f] === false) continue;
      // A blocked entry prints what it is blocked on in place of its cwd.
      if (f === 'cwd' && got.blockedOn) continue;
      const w = f === 'failed' || f === 'crossed' ? String(want[f]).replace(/\s+/g, ' ').trim() : want[f];
      const g = typeof got[f] === 'string' ? got[f].replace(/\s+/g, ' ').trim() : got[f];
      if (f === 'failed' || f === 'crossed' ? !String(w).startsWith(String(g).replace(/…$/, '')) : g !== w) failures.push(`${c.name}: entry ${i} ${f} ${JSON.stringify(g)} != ${JSON.stringify(w)}`);
    }
  });
}
if (parseFleetMessage('[fleet] Worker finished:\n- not an entry')) failures.push('a malformed bullet parsed');
if (!buildResultView('{"merged":true}').fields[0] || buildResultView('{bad').fallback == null) failures.push('result view');
if (failures.length) { console.error('test-mobile-next-fleet:\n  ' + failures.join('\n  ')); process.exit(1); }
console.log(`test-mobile-next-fleet: ${cases.length} contract wakes parse`);
