#!/usr/bin/env node
/** Scratch image/browser checks, never Fly APIs or live volumes. */
import { execFileSync } from 'node:child_process';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import { chromium, expect } from '@playwright/test';
const image=process.argv[2];
if(!image)throw new Error('usage: verify-parity-image.mjs IMAGE');
const docker=(...args)=>execFileSync('docker',args,{encoding:'utf8',maxBuffer:1024*1024}).trim();
const container=docker('run','--rm','-d','-p','127.0.0.1::7895','-e','HOME=/scratch','-e','XDG_CONFIG_HOME=/scratch/config','-e','XDG_DATA_HOME=/scratch/data','-e','XDG_CACHE_HOME=/scratch/cache','--entrypoint','/bin/sh',image,'-c',
  'mkdir -p /scratch/config/workspacer /scratch/repo; printf parity-image-fixture-token > /scratch/config/workspacer/remote-token; git -C /scratch/repo init -q; git -C /scratch/repo config user.name Test; git -C /scratch/repo config user.email test@example.invalid; git -C /scratch/repo config commit.gpgsign false; printf initial > /scratch/repo/file.txt; git -C /scratch/repo add file.txt; git -C /scratch/repo commit -qm initial; exec workspacer serve --host 0.0.0.0 --webapp-dir /usr/local/share/workspacer/web');
let browser;
try {
  const port=docker('port',container,'7895/tcp').split(':').at(-1), origin=`http://127.0.0.1:${port}`;
  for(let n=0;n<120;n++){
    try{if((await fetch(origin+'/health')).ok)break}catch{}
    if(n===119)throw new Error('scratch image did not become healthy');
    await new Promise(r=>setTimeout(r,250));
  }
  browser=await chromium.launch({headless:true});
  const page=await browser.newPage({viewport:{width:1440,height:1000}}), errors=[];
  page.on('pageerror',error=>errors.push(error.message));
  await page.goto(origin+'/app/?token=parity-image-fixture-token');
  await page.waitForFunction(()=>window.electronAPI?.platform==='web');
  const call=async(method,...args)=>{try{return await page.evaluate(async({method,args})=>window.electronAPI[method](...args),{method,args})}catch(error){throw new Error(method+': '+error.message)}};
  docker('exec',container,'python3','-c',"import json, urllib.request; data=json.dumps({'session_id':'fixture-session','cwd':'/scratch/repo'}).encode(); urllib.request.urlopen(urllib.request.Request('http://127.0.0.1:7890/hook/user_prompt_submit',data=data,headers={'Content-Type':'application/json'})).read()");
  await page.waitForFunction(async()=> (await window.electronAPI.getAllClaudeSessions()).some(row=>row.sessionId==='fixture-session'));
  await call('saveConfig',{onboardingDismissed:true});
  assert.equal((await call('worktreeInfo','/scratch/repo')).isRepo,true);
  let read;
  for(let attempt=0;attempt<100;attempt++){
    try{read=await call('readFile','/scratch/repo/file.txt');break}catch(error){
      if(attempt===99)throw error;
      await new Promise(resolve=>setTimeout(resolve,50));
    }
  }
  assert.equal(read.contents,'initial');
  await call('writeFile','/scratch/repo/file.txt','browser edit\n');
  await call('gitStage','/scratch/repo','file.txt');
  await call('gitCommit','/scratch/repo','Commit through real browser');
  assert.match(JSON.stringify(await call('gitLog','/scratch/repo',5)),/Commit through real browser/);
  const plugins=await call('listHubPlugins');
  assert.ok(plugins.some(plugin=>plugin.id==='workspacer.editor'),'bundled editor missing');
  const summary=await call('analyticsSummary');assert.equal(summary.unavailable,undefined);assert.equal(summary.totals.sessions,1);assert.equal(summary.totals.unrecordedSessions,1);
  await call('pricingSaveOverrides',{'fixture-model':{input:1,output:2}});
  assert.equal((await call('pricingGetRates')).overrides['fixture-model'].output,2);
  assert.deepEqual(await call('federationPeersConfig'),[]);
  assert.equal((await call('federationSavePeersConfig',[])).ok,true);
  assert.deepEqual((await call('nodesList')).nodes,[]);
  assert.ok(Array.isArray((await call('jobsList')).jobs));
  assert.equal((await call('managerReplacement',{action:'list'})).available,true);
  assert.equal((await call('agentRuntimeStatus')).claudemon,'ready');
  const png=readFileSync(new URL('../build/icon.png',import.meta.url));
  const upload=await call('uploadAttachment',{name:'proof.png',dataBase64:png.toString('base64')});
  assert.equal(upload.size,png.length);
  assert.match((await call('readImagePreview',upload.path)).dataUrl,/^data:image\/png;base64,/);
  await call('fileShowInFolder','/scratch/repo/file.txt');
  const editor=page.frameLocator('iframe[src*="workspacer.editor"]');
  await expect(editor.locator('.cm-content')).toContainText('browser edit',{timeout:15_000});
  assert.deepEqual(errors,[],'browser page errors');
  docker('exec','-d','-e','WKS_NETWORK_ADMIN_TOKEN=network-fixture',container,'setpriv','--reuid=0','--regid=0','--clear-groups','--bounding-set=-all','--inh-caps=-all','--ambient-caps=-all','--no-new-privs','--','python3','/opt/combined/network-admin.py');
  for(let n=0;n<50;n++){
    try{docker('exec',container,'test','-S','/run/workspacer-network/admin.sock');break}catch{}
    await new Promise(r=>setTimeout(r,50));
  }
  docker('exec',container,'chown','10002:10002','/run/workspacer-network/admin.sock');
  docker('exec',container,'chmod','0600','/run/workspacer-network/admin.sock');
  docker('exec','--user','10001',container,'python3','-c',"import socket; s=socket.socket(socket.AF_UNIX);\ntry: s.connect('/run/workspacer-network/admin.sock')\nexcept PermissionError: pass\nelse: raise SystemExit('worker reached root network broker')");
  docker('exec','--user','10002',container,'python3','-c',"import socket; s=socket.socket(socket.AF_UNIX); s.connect('/run/workspacer-network/admin.sock'); s.sendall(b'GET /status HTTP/1.0\\r\\n\\r\\n'); assert b'401' in s.recv(4096)");
  docker('exec','--user','10002',container,'python3','-c',"import socket; s=socket.socket(socket.AF_UNIX); s.connect('/run/workspacer-network/admin.sock'); s.sendall(b'GET /unknown HTTP/1.0\\r\\nAuthorization: Bearer network-fixture\\r\\n\\r\\n'); assert b'404' in s.recv(4096)");
  console.log(JSON.stringify({ok:true,checks:['browser boot','Git edit/stage/commit','bundled editor','analytics','pricing','peer administration','node/job registries','manager service','runtime','server file reveal','network broker UID isolation']}));
} catch(error){
  console.error(String(error));console.error(docker('logs','--tail','35',container));process.exitCode=1;
} finally {await browser?.close();docker('rm','-f',container);}
