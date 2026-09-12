#!/usr/bin/env python3
"""Update one existing combined machine, preserving its resources and volume."""
import argparse
import json
import os
import pathlib
import subprocess
import tempfile
import shlex

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("app")
parser.add_argument("machine")
parser.add_argument("image")
parser.add_argument("--restart-idle", action="store_true", help="Allow a running target only after its live detector reports no work blockers")
args = parser.parse_args()
rows = json.loads(subprocess.check_output(["flyctl", "machine", "list", "-a", args.app, "--json"]))
matches = [row for row in rows if row["id"] == args.machine]
if len(matches) != 1:
    raise SystemExit("Expected exactly one existing target machine")
row = matches[0]
if row["state"] != "stopped":
    if row["state"] != "started" or not args.restart_idle:
        raise SystemExit("Target is running; inspect its live work before applying an update")
    code = """
import os, json, subprocess, sys
env = dict(os.environ, XDG_CONFIG_HOME='/data/hub/home/.config')
env.pop('HUB_TOKEN',None)
r = subprocess.run(['workspacer','fleet','idle','--json'],env=env,capture_output=True,text=True)
if r.returncode not in (0,1): sys.exit('Could not inspect live work')
state = json.loads(r.stdout)['idle']
blockers = [b for b in state.get('blockers',[]) if b['kind'] not in ('client-active','dwell')]
if blockers: sys.exit('Work or unknown state blocks deployment: '+str([b['kind'] for b in blockers]))
print('No work blockers; restarting for deployment')
"""
    subprocess.run(['flyctl','ssh','console','-a',args.app,'--machine',args.machine,'-C',shlex.join(['python3','-c',code])],check=True)
backup = pathlib.Path(tempfile.mkdtemp(prefix="workspacer-power-rollback-"))
path = backup / "machine.json"
path.write_text(json.dumps(row, indent=2))
os.chmod(path, 0o600)
print("Previous machine configuration saved to", path, flush=True)
subprocess.run(["flyctl", "machine", "update", args.machine, "-a", args.app,
    "--image", args.image, "--restart", "on-failure", "--autostart", "--autostop=off",
    "--file-secret", "/run/workspacer-power-token=WORKSPACER_POWER_TOKEN_B64",
    "--yes", "--wait-timeout", "180"], check=True)
