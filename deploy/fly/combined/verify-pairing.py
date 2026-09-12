#!/usr/bin/env python3
"""Read-only pairing capability check using the private owner credential."""
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
const timer = setTimeout(()=>{ console.error('pairing check timed out'); process.exit(1); },15000);
ws.onopen = () => ws.send(JSON.stringify({op:'call',id:'permissions',method:'remote.pairingInfo',params:{}}));
ws.onmessage = ({data}) => {
  const f = JSON.parse(data);
  if (f.op === 'error') { console.error('pairing capability check refused'); process.exit(1); }
  if (f.op === 'result' && f.id === 'permissions') {
    console.log(JSON.stringify(f.result));
    if (f.result.scope !== 'operator' || f.result.canManageTokens !== true) process.exit(1);
    clearTimeout(timer); ws.close();
  }
};
ws.onerror = () => { console.error('pairing connection failed'); process.exit(1); };
'''
subprocess.run(["flyctl", "ssh", "console", "-a", args.app, "--machine", args.machine,
    "-C", shlex.join(["node", "-e", code])], check=True)
