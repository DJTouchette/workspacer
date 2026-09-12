import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

test('headless analytics retains real usage and model splits across process restarts', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(),'workspacer-analytics-'));
  const home = path.join(root,'home');
  const transcripts = path.join(home,'.claude','projects','fixture');
  fs.mkdirSync(transcripts,{recursive:true});
  const file = path.join(transcripts,'claude-session.jsonl');
  const turn = (id,model,input,output) => JSON.stringify({type:'assistant',message:{id,model,usage:{input_tokens:input,output_tokens:output}}});
  fs.writeFileSync(file,[turn('one','claude-sonnet-4-5',1000,100),turn('one','claude-sonnet-4-5',1000,100)].join('\n'));
  const subDir = path.join(transcripts,'claude-session','subagents');fs.mkdirSync(subDir,{recursive:true});
  fs.writeFileSync(path.join(subDir,'agent-sub.jsonl'),turn('two','claude-haiku-4-5',500,50));
  const env = {...process.env,HOME:home,USERPROFILE:home,XDG_CONFIG_HOME:path.join(root,'config'),APPDATA:path.join(root,'config'),CLAUDE_CONFIG_DIR:path.join(home,'.claude')};
  delete env.HUB_TOKEN;
  const context = {workspaceRoots:[home],setupRoots:[home],snapshots:[],analyticsSnapshots:[
    {session_id:'claude-session',cwd:home,provider:'claude',mode:'stopped',started_at:'2026-09-10T00:00:00Z',updated_at:'2026-09-10T01:00:00Z',transcript_path:file},
    {session_id:'codex-session',cwd:home,provider:'codex',mode:'stopped',started_at:'2026-09-11T00:00:00Z',updated_at:'2026-09-11T01:00:00Z',usage:{costUSD:0},status_line:{model_display:'gpt-5',total_input_tokens:2000,total_output_tokens:200,cost_usd:0.75}},
    {session_id:'unknown-session',cwd:home,provider:'claude',mode:'stopped',started_at:'2026-09-11T00:00:00Z'},
  ]};
  const call = (method,params={}) => {
    // A fresh process per call proves both persistence and idempotent migrations.
    const result = spawnSync(process.execPath,['dist/headless/desktop-host.cjs'],{env,encoding:'utf8',input:JSON.stringify({id:'1',method,params,context})+'\n'});
    assert.equal(result.status,0,result.stderr);
    const reply = result.stdout.trim().split('\n').map(line=>JSON.parse(line)).find(row=>row.id==='1');
    assert.ok(reply,result.stdout);assert.equal(reply.error,undefined,reply.error);
    return reply.result;
  };
  try {
    const first = call('internal.analyticsSummary');
    assert.equal(first.totals.sessions,3);assert.equal(first.totals.unrecordedSessions,1);
    assert.equal(first.totals.inputTokens,3500);assert.equal(first.totals.outputTokens,350);
    assert.equal(first.byModel.length,4); // two Claude models, managed model, unknown
    assert.equal(first.byModel.find(r=>r.key==='gpt-5').costUSD,0.75);
    const filtered = call('internal.analyticsSummary',{provider:'claude'});
    assert.equal(filtered.totals.inputTokens,1500);assert.equal(filtered.byProvider.length,2);
    const recent = call('internal.analyticsRecent',{provider:'codex',limit:1});assert.equal(recent[0].sessionId,'codex-session');
    fs.unlinkSync(file);
    const after = call('internal.analyticsSummary');assert.deepEqual(after.totals,first.totals,'transcript cleanup lost recorded usage');
    context.analyticsSnapshots=[];
    assert.deepEqual(call('internal.analyticsSummary').totals,first.totals,'daemon absence erased history');
  } finally {fs.rmSync(root,{recursive:true,force:true});}
});
