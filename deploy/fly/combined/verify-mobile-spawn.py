#!/usr/bin/env python3
"""Verify mobile spawn catalogs and permissions without spawning an agent."""
import argparse
import shlex
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("app")
parser.add_argument("machine")
args = parser.parse_args()
code = r'''
const fs = require('node:fs');
const token = fs.readFileSync('/data/hub/home/.config/workspacer/remote-token','utf8').trim();
const ws = new WebSocket('ws://127.0.0.1:7895/bus?token='+encodeURIComponent(token));
const timer = setTimeout(()=>{ console.error('spawn catalog check timed out'); process.exit(1); },30000);
const output = {}; let remaining = 3;
ws.onopen = () => {
  for (const [id,method,params] of [['folders','fs.listDir',{path:''}],['projects','config.get',{}],['models','claude.listModels',{}]])
    ws.send(JSON.stringify({op:'call',id,method,params}));
};
ws.onmessage = ({data}) => {
  const f = JSON.parse(data);
  if (f.op === 'hello') output.fullAccess = f.spawnFullAccess === true;
  if (f.op === 'error') { console.error('spawn catalog request failed: '+(f.id || 'connection')+': '+f.error); process.exit(1); }
  if (f.op !== 'result') return;
  if (f.id === 'folders') output.folders = {path:f.result.path, directories:f.result.dirs.length};
  if (f.id === 'projects') {
    const projects = Object.keys(f.result.projects || {});
    output.projects = {paths:projects,configured:projects.length, favourites:(f.result.directories?.favourites || []).length};
    console.log(JSON.stringify({projects:output.projects}));
    if (projects.length) { remaining++; ws.send(JSON.stringify({op:'call',id:'projectFolder',method:'fs.listDir',params:{path:projects[0]}})); }
  }
  if (f.id === 'projectFolder') output.projectFolder = {path:f.result.path, directories:f.result.dirs.length};
  if (f.id === 'models') output.models = {aliases:(f.result.aliases || []).length, seen:(f.result.seen || []).length};
  if (--remaining === 0) { console.log(JSON.stringify(output)); clearTimeout(timer); ws.close(); if (!output.fullAccess) process.exit(1); }
};
ws.onerror = () => { console.error('spawn catalog connection failed'); process.exit(1); };
'''
subprocess.run(["flyctl", "ssh", "console", "-a", args.app, "--machine", args.machine,
    "-C", shlex.join(["node", "-e", code])], check=True)
