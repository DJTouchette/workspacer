#!/usr/bin/env python3
"""Inspect the approved MCP scope and work blockers without exposing credentials."""
import argparse
import shlex
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('app')
parser.add_argument('machine')
parser.add_argument('--provision', action='store_true', help='Provision the approved MCP grant and an empty owner-only job registry after checking idle')
parser.add_argument('--fix-legacy-jobs-flag', action='store_true', help='Restore the backed-up disabled legacy flag while using the new owner-only marker')
args = parser.parse_args()
code = r'''
import json, os, pathlib, shutil, subprocess, tempfile, time
root = pathlib.Path('/data/combined')
runtime = json.loads((root/'runtime.json').read_text())
file = pathlib.Path('/data/hub/home/.config/workspacer/tokens.json')
records = json.loads(file.read_text())
selected = [r for r in records if r.get('label') == runtime['mcpTokenLabel']]
if len(selected) != 1: raise SystemExit('Approved MCP service credential is not unique')
record = selected[0]
env = dict(os.environ, XDG_CONFIG_HOME='/data/hub/home/.config')
env.pop('HUB_TOKEN',None)
result = subprocess.run(['workspacer','fleet','idle','--json'],env=env,capture_output=True,text=True)
blockers = ['unknown']
if result.returncode in (0,1):
    state = json.loads(result.stdout)['idle']
    blockers = [b['kind'] for b in state.get('blockers',[]) if b['kind'] not in ('client-active','dwell')]
jobs_file = pathlib.Path('/data/hub/home/.config/workspacer-hub/jobs.json')
jobs_count = 0
if jobs_file.exists():
    jobs_doc = json.loads(jobs_file.read_text())
    jobs = jobs_doc.get('jobs', []) if isinstance(jobs_doc, dict) else jobs_doc
    if not isinstance(jobs, list): raise SystemExit('Jobs store has an unknown shape')
    jobs_count = len(jobs)
if GRANT:
    if blockers: raise SystemExit('Work or unknown state blocks provisioning: '+str(blockers))
    if runtime['mcpScope'] != 'operator' or record['scope'] != 'operator':
        raise SystemExit('The approved MCP credential is not an operator; no scope was widened')
    if not runtime.get('jobsEnabled') and not runtime.get('ownerOnlyJobsEnabled') and jobs_count:
        raise SystemExit('Existing dormant jobs need review before enabling the scheduler; nothing changed')
    if not record.get('facadeAuthority'):
        backup = root/('tokens-before-parity-'+str(time.time_ns())+'.json')
        shutil.copyfile(file,backup);os.chmod(backup,0o600)
        info = file.stat()
        record['facadeAuthority'] = True
        fd,temporary = tempfile.mkstemp(prefix='.tokens-parity-',dir=file.parent)
        try:
            os.fchown(fd,info.st_uid,info.st_gid);os.fchmod(fd,0o600)
            with os.fdopen(fd,'w') as stream:
                json.dump(records,stream,indent=2);stream.write('\n');stream.flush();os.fsync(stream.fileno())
            os.replace(temporary,file)
        finally:
            pathlib.Path(temporary).unlink(missing_ok=True)
    if FIX_LEGACY or (not runtime.get('jobsEnabled') and not runtime.get('ownerOnlyJobsEnabled')):
        if FIX_LEGACY:
            backups=sorted(root.glob('runtime-before-parity-*.json'))
            if not backups or json.loads(backups[-1].read_text()).get('jobsEnabled') is not False:
                raise SystemExit('No matching disabled legacy jobs backup; refusing to change it')
            runtime['jobsEnabled']=False
        runtime_file=root/'runtime.json'
        backup=root/('runtime-before-parity-'+str(time.time_ns())+'.json')
        shutil.copyfile(runtime_file,backup);os.chmod(backup,0o600)
        runtime['ownerOnlyJobsEnabled']=True
        info=runtime_file.stat()
        fd,temporary=tempfile.mkstemp(prefix='.runtime-parity-',dir=root)
        try:
            os.fchown(fd,info.st_uid,info.st_gid);os.fchmod(fd,0o600)
            with os.fdopen(fd,'w') as stream:
                json.dump(runtime,stream,indent=2);stream.write('\n');stream.flush();os.fsync(stream.fileno())
            os.replace(temporary,runtime_file)
        finally:
            pathlib.Path(temporary).unlink(missing_ok=True)
print(json.dumps({'mcpScope':record['scope'],'facadeAuthority':record.get('facadeAuthority',False),'profileGrantCount':len(record.get('profilesAllowed',[])),'workBlockers':blockers,'jobsEnabled':runtime.get('jobsEnabled',False),'ownerOnlyJobsEnabled':runtime.get('ownerOnlyJobsEnabled',False),'existingJobs':jobs_count}))
'''.replace('GRANT', repr(args.provision)).replace('FIX_LEGACY', repr(args.fix_legacy_jobs_flag))
subprocess.run(['flyctl','ssh','console','-a',args.app,'--machine',args.machine,'-C',shlex.join(['python3','-c',code])],check=True)
