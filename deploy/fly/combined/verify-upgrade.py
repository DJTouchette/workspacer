#!/usr/bin/env python3
"""Read-only deployment checks; never prints pairing or cloud credentials."""
import argparse
import shlex
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("app")
parser.add_argument("machine")
args = parser.parse_args()
code = r'''
import json, os, pathlib, stat, subprocess, urllib.request
root = pathlib.Path('/opt/combined')
power = json.loads((root/'power.json').read_text())
approval = json.loads(pathlib.Path('/data/combined/runtime.json').read_text())
path = '/run/workspacer-power-token'
s = os.stat(path)
def can_read(uid):
    p = subprocess.run(['setpriv','--reuid='+str(uid),'--regid='+str(uid),'--clear-groups','python3','-c',
        'import os,sys;sys.exit(0 if os.access("/run/workspacer-power-token",os.R_OK) else 1)'], capture_output=True)
    return p.returncode == 0
def status(url):
    with urllib.request.urlopen(url, timeout=15) as response: return response.status
credential = pathlib.Path(path).read_text().strip()
req = urllib.request.Request('https://api.machines.dev/v1/apps/'+power['app']+'/machines/'+power['machine'],
    headers={'Authorization':'Bearer '+credential})
with urllib.request.urlopen(req, timeout=15) as response: machine = json.load(response)
env = dict(os.environ, XDG_CONFIG_HOME='/data/hub/home/.config')
env.pop('HUB_TOKEN', None)
idle = subprocess.run(['workspacer','fleet','idle','--json'], env=env, capture_output=True, text=True)
if idle.returncode not in (0, 1): raise SystemExit('idle query failed')
out = dict(hubHealth=status('http://127.0.0.1:7895/health'),
    workerHealth=status('http://127.0.0.1:7891/sessions'),
    cloudState=machine['state'], tokenOwner=s.st_uid, tokenMode=oct(stat.S_IMODE(s.st_mode)),
    hubCanReadPowerToken=can_read(10002), workerCanReadPowerToken=can_read(10001),
    appUrl='https://'+approval['expectedDnsName']+'/app/',
    mobileUrl='https://'+approval['expectedDnsName']+'/m', idle=json.loads(idle.stdout))
print(json.dumps(out, indent=2))
assert out['hubCanReadPowerToken'] and not out['workerCanReadPowerToken']
assert out['tokenOwner']==10002 and out['tokenMode']=='0o600'
'''
subprocess.run(["flyctl", "ssh", "console", "-a", args.app, "--machine", args.machine,
    "-C", shlex.join(["python3", "-c", code])], check=True)
