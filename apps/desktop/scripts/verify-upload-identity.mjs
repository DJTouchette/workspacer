#!/usr/bin/env node
/** Prove upload files belong to the isolated worker, not the hub. */
import { execFileSync } from 'node:child_process';
import assert from 'node:assert/strict';
const image=process.argv[2];if(!image)throw new Error('usage: verify-upload-identity.mjs IMAGE');
const docker=(...args)=>execFileSync('docker',args,{encoding:'utf8',maxBuffer:1024*1024}).trim();
const launch=`
set -eu
mkdir -p /scratch/hub/config/workspacer /scratch/worker/config/workspacer
printf '%s' '[{"token":"provider-fixture","scope":"provider","label":"worker","created":"2026-01-01T00:00:00Z","provides":["*"]},{"token":"caller-fixture","scope":"operator","label":"worker-caller","facadeAuthority":true,"created":"2026-01-01T00:00:00Z"},{"token":"triage-fixture","scope":"triage","label":"phone","created":"2026-01-01T00:00:00Z"}]' > /scratch/hub/config/workspacer/tokens.json
printf '%s' 'worker-owner-fixture' > /scratch/worker/config/workspacer/remote-token
chmod 0600 /scratch/worker/config/workspacer/remote-token
chown -R 10002:10002 /scratch/hub
chown -R 10001:10001 /scratch/worker
chmod 0700 /scratch/hub /scratch/worker
chmod 0600 /scratch/hub/config/workspacer/tokens.json
setpriv --reuid=10002 --regid=10002 --clear-groups --bounding-set=-all --inh-caps=-all --ambient-caps=-all --no-new-privs -- env HOME=/scratch/hub XDG_CONFIG_HOME=/scratch/hub/config HUB_TOKEN=owner-fixture workspacer-rust --config-dir /scratch/hub/config/workspacer --host 0.0.0.0 --hub-port 7895 serve --hub-only --quiet --no-mcp --no-jobs --uploads-to-worker --data-dir /scratch/hub/state --plugins-dir /scratch/hub/plugins &
setpriv --reuid=10001 --regid=10001 --clear-groups --bounding-set=-all --inh-caps=-all --ambient-caps=-all --no-new-privs -- env HOME=/scratch/worker XDG_CONFIG_HOME=/scratch/worker/config HUB_TOKEN=provider-fixture WKS_MCP_HUB_TOKEN=caller-fixture workspacer-rust --config-dir /scratch/worker/config/workspacer --hub-port 7896 serve --quiet --upstream ws://127.0.0.1:7895/bus --provider-scope full --node-id fixture-worker --no-claudemon-init --no-jobs --home-dir /scratch/worker --data-dir /scratch/worker/state --plugins-dir /scratch/worker/plugins --claudemon-db-path /scratch/worker/sessions.db &
wait
`;
const container=docker('run','--rm','-d','-p','127.0.0.1::7895','--entrypoint','/bin/sh',image,'-c',launch);
let socket;
try{
 const port=docker('port',container,'7895/tcp').split(':').at(-1),origin=`http://127.0.0.1:${port}`;
 for(let n=0;n<100;n++){try{if((await fetch(origin+'/health')).ok)break}catch{};await new Promise(r=>setTimeout(r,100));}
 socket=new WebSocket(origin.replace('http','ws')+'/bus?token=triage-fixture');
 let seq=0;const pending=new Map();
 await new Promise((resolve,reject)=>{socket.onmessage=event=>{const f=JSON.parse(event.data);if(f.op==='hello'){resolve();return};const p=pending.get(f.id);if(p){pending.delete(f.id);clearTimeout(p.timer);f.op==='error'?p.reject(new Error(f.error)):p.resolve(f.result)}};socket.onerror=reject;});
 const call=(method,params={})=>new Promise((resolve,reject)=>{const id=String(++seq);const timer=setTimeout(()=>{pending.delete(id);reject(new Error('callback timed out'))},5000);pending.set(id,{resolve,reject,timer});socket.send(JSON.stringify({op:'call',id,method,params}));});
 for(let n=0;n<100;n++){try{await call('sessions.snapshots');break}catch(error){if(n===99)throw error;await new Promise(r=>setTimeout(r,100));}}
 const payload={name:'proof.png',dataBase64:Buffer.from('fixture upload bytes').toString('base64')};
 await assert.rejects(call('files.receiveUpload',payload));
 const uploaded=await call('files.upload',payload);
 assert.equal(uploaded.size,20);
 docker('exec','--user','10001',container,'python3','-c',"import pathlib,sys; assert pathlib.Path(sys.argv[1]).read_bytes()==b'fixture upload bytes'",uploaded.path);
 docker('exec','--user','10002',container,'python3','-c',"import pathlib,sys;\ntry: pathlib.Path(sys.argv[1]).read_bytes()\nexcept PermissionError: pass\nelse: raise SystemExit('hub could read worker-private upload')",uploaded.path);
 const metadata=JSON.parse(docker('exec',container,'python3','-c',"import os,json,sys; s=os.stat(sys.argv[1]); print(json.dumps({'uid':s.st_uid,'mode':s.st_mode&511}))",uploaded.path));
 assert.deepEqual(metadata,{uid:10001,mode:384});
 console.log(JSON.stringify({ok:true,checks:['triage upload works','receiver is owner-only','worker can read attachment','hub cannot read worker-private bytes','file is UID 10001 mode 0600']}));
}catch(error){console.error(String(error));console.error(docker('logs','--tail','20',container));process.exitCode=1}
finally{socket?.close();docker('rm','-f',container)}
