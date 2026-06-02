/**
 * Manual harness: point the compiled workflowWatcher at a real recorded
 * session and dump what the UI would receive. Not a unit test — uses the
 * leroy session that ran five Workflow tool invocations on 2026-06-01.
 *
 *   npm run build:main && node scripts/test-workflow-watcher.js [transcriptPath]
 */
const path = require('path');
const os = require('os');

const transcript = process.argv[2] ?? path.join(
  os.homedir(),
  '.claude', 'projects', 'C--Users-DamienTouchette-work-leroy',
  'eb65ba77-4ecc-4e33-8a67-e70c07d79e82.jsonl',
);

const { workflowWatcher } = require('../dist/main/services/workflowWatcher');

let updates = 0;
workflowWatcher.attach('test-session', transcript, (update) => {
  updates++;
  console.log(`\n=== update #${updates} ===`);
  for (const run of update.runs) {
    console.log(`run ${run.runId}  name=${run.name}  status=${run.status}  agents=${run.agents.length}  phases=[${run.phases.map(p => p.title).join(', ')}]  tokens=${run.totalTokens ?? '-'}  dur=${run.durationMs ?? '-'}ms`);
    for (const a of run.agents) {
      console.log(`  agent ${a.id}  [${a.status}]  label=${a.label ?? '-'}  phase=${a.phaseTitle ?? '-'}  model=${a.model ?? '-'}  tok=${a.tokens}  tools=${a.toolCalls}  lastTool=${a.lastToolName ?? '-'}(${a.lastToolSummary ?? ''})  prompt=${(a.promptPreview ?? '').slice(0, 50)}`);
    }
  }
  const plainIds = Object.keys(update.subagentActivity);
  console.log(`plain subagents: ${plainIds.length}`);
  for (const id of plainIds.slice(0, 5)) {
    const a = update.subagentActivity[id];
    console.log(`  ${id}  type=${a.agentType ?? '-'}  desc=${(a.description ?? '').slice(0, 50)}  tok=${a.tokens}  tools=${a.toolCalls}  lastTool=${a.lastToolName ?? '-'}(${a.lastToolSummary ?? ''})`);
  }
  console.log(`workflowAgentIds: ${update.workflowAgentIds.length}`);
});

// Give it a few ticks then exit
setTimeout(() => {
  workflowWatcher.detachAll();
  console.log(`\ndone — ${updates} update(s) received`);
  process.exit(updates > 0 ? 0 : 1);
}, 3500);
