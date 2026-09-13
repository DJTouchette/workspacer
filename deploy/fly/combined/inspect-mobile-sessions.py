#!/usr/bin/env python3
"""Read mobile session/transcript health without printing prompts or credentials."""
import argparse, shlex, subprocess
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('app');p.add_argument('machine');a=p.parse_args()
code=r'''
const fs=require('node:fs');
const token=fs.readFileSync('/data/hub/home/.config/workspacer/remote-token','utf8').trim();
const ws=new WebSocket('ws://127.0.0.1:7895/bus?token='+encodeURIComponent(token));
let seq=0;const pending=new Map();
const timer=setTimeout(()=>process.exit(1),45000);
function call(method,params={}){return new Promise((resolve,reject)=>{const id=String(++seq);pending.set(id,{resolve,reject});ws.send(JSON.stringify({op:'call',id,method,params}));});}
ws.onmessage=({data})=>{const f=JSON.parse(data),p=pending.get(f.id);if(!p)return;if(f.op==='result'){pending.delete(f.id);p.resolve(f.result)}else if(f.op==='error'){pending.delete(f.id);p.reject(new Error('RPC failed'))}};
ws.onopen=async()=>{try{
 const recent=await call('sessions.recent');
 console.log(JSON.stringify({recent:recent.slice(0,12).map(s=>({sessionId:s.sessionId,provider:s.provider,state:s.state,createdAt:s.createdAt,lastActivity:s.lastActivity}))}));
 const daemon=await (await fetch('http://127.0.0.1:7891/sessions?include_archived=true')).json();
 console.log(JSON.stringify({daemonShape:Array.isArray(daemon)?'array':Object.keys(daemon),daemonSessions:(Array.isArray(daemon)?daemon:daemon.sessions||[]).map(s=>({id:s.session_id||s.sessionId,provider:s.provider,transport:s.transport,state:s.state,mode:s.mode,startedAt:s.started_at,updatedAt:s.updated_at,userPromptCount:s.user_prompts?.length,toolCallCount:s.tool_calls?.length,hasTranscript:!!s.transcript_path,exitCode:s.exit_code}))}));
 const snapshots=await call('sessions.snapshots');
 console.log(JSON.stringify({snapshotCount:snapshots.length}));
 for(const s of [...snapshots,...recent.filter(r=>!snapshots.some(s=>s.sessionId===r.sessionId))]){
  const id=s.sessionId;let c;try{c=await call('sessions.conversation',{sessionId:id})}catch{}
  const counts={};for(const i of c?.items||[])counts[i.kind||i.type]=(counts[i.kind||i.type]||0)+1;
  console.log(JSON.stringify({sessionId:id,provider:s.provider,transport:s.transport,state:s.state,ambientState:s.ambientState,snapshotTurns:s.conversation?.length,seq:c?.seq,counts,conversationError:!c}));
 }
 console.log(JSON.stringify({runtime:await call('desktop.agentRuntimeStatus')}));
}catch{console.log('Session inspection failed');process.exitCode=1}finally{clearTimeout(timer);ws.close()}};
'''
subprocess.run(['flyctl','ssh','console','-a',a.app,'--machine',a.machine,'-C',shlex.join(['node','-e',code])],check=True)
