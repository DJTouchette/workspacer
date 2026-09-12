#!/usr/bin/env python3
"""Read-only web capability inventory. Prints no tokens, prompts or config values."""
import argparse
import shlex
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("app")
parser.add_argument("machine")
parser.add_argument("--expect-plugin-admin", action="store_true")
parser.add_argument("--expect-parity", action="store_true")
args = parser.parse_args()
code = r'''
const fs = require('node:fs');
const token = fs.readFileSync('/data/hub/home/.config/workspacer/remote-token', 'utf8').trim();
const base = 'http://127.0.0.1:7895';
const output = { http: {}, probes: {} };
const checks = [
  ['health', '/health'], ['app', '/app/'], ['plugins', '/plugins'],
  ['examples', '/plugins/examples'],
];
(async () => {
  if (expectPluginAdmin) {
    const assets = '/usr/local/share/workspacer/web/assets';
    const marker = 'The server returned an invalid build command';
    const name = fs.readdirSync(assets).find(name => name.endsWith('.js') && fs.readFileSync(assets+'/'+name, 'utf8').includes(marker));
    if (!name) throw new Error('Expected plugin administration bundle missing');
    const response = await fetch(base+'/app/assets/'+encodeURIComponent(name), {headers:{Authorization:'Bearer '+token}, signal:AbortSignal.timeout(10000)});
    output.pluginAdminBundle = {status:response.status, containsImplementation:(await response.text()).includes(marker)};
    if (!response.ok || !output.pluginAdminBundle.containsImplementation) throw new Error('Expected plugin administration bundle not served');
  }
  for (const [name, route] of checks) {
    const response = await fetch(base + route, { headers: { Authorization: 'Bearer ' + token }, signal: AbortSignal.timeout(10000) });
    output.http[name] = response.status;
    if (name === 'health' && response.ok) {
      const health = await response.json();
      output.registeredMethods = health.methodNames ?? null;
      output.registeredCount = health.methods;
    }
    if ((name === 'plugins' || name === 'examples') && response.ok) {
      const rows = await response.json();
      output.http[name + 'Count'] = Array.isArray(rows) ? rows.length : null;
    }
  }
  const ws = new WebSocket(base.replace('http', 'ws') + '/bus?token=' + encodeURIComponent(token));
  const probes = ['machine.power', 'remote.pairingInfo', 'providers.checkAll', 'claude.listModels',
    'sessions.snapshots', 'sessions.recent', 'usage.report', 'usage.pacingSchedule',
    'routing.preferences.get', 'federation.peers', 'nodes.list', 'jobs.list', 'config.get', 'app.supervisorHome'];
  if (expectParity) probes.push('desktop.agentRuntimeStatus','desktop.pricingGetRates','desktop.dispatchHistoryRead','desktop.loadBriefBoard','remote.tailscaleInfo','remote.sharingInfo','analytics.summary','federation.peersConfig','ui.fonts');
  const pending = new Set(probes);
  function finish() {
    clearTimeout(timer); ws.close();
    if (expectParity) {
      const required=['git.stage','git.unstage','git.commit','git.push','git.commitDiff','git.commitNumstat','fs.watch','fs.unwatch','fleetWorkflows.request','desktop.managerReplacement','desktop.filePickerList','desktop.readFileBytes','desktop.sessionGrantReconcile','files.receiveUpload','plugins.prepareLaunch','federation.peersConfig','remote.tailscaleServe','ui.asset'];
      const failures=required.filter(method=>!output.registeredMethods?.includes(method)).map(method=>'missing '+method);
      for(const [method,result] of Object.entries(output.probes))if(result.status!=='ok')failures.push(method+': '+result.status);
      if(output.idle?.mode!=='stop'||output.idle?.timeoutSeconds!==900)failures.push('idle stop policy changed');
      if(output.http.pluginsCount<2||output.http.examplesCount<2)failures.push('bundled plugins unavailable');
      if(!output.network?.available||!output.network?.serveActive||!output.network?.canServe)failures.push('network adapter unavailable');
      if(output.runtime?.hub!=='ready'||output.runtime?.claudemon!=='ready'||output.runtime?.facade!=='ready')failures.push('runtime not ready');
      output.validation={ok:failures.length===0,failures};if(failures.length)process.exitCode=1;
    }
    console.log(JSON.stringify(output, null, 2));
  }
  const timer = setTimeout(() => { for (const method of pending) output.probes[method] = {status:'timeout'}; finish(); }, 25000);
  ws.onopen = () => { for (const method of probes) ws.send(JSON.stringify({op:'call', id:method, method, params:{}})); };
  ws.onmessage = ({data}) => {
    const f = JSON.parse(data);
    if (f.op === 'hello') output.authority = {scope:f.scope, patterns:f.methods, fullAccess:f.spawnFullAccess};
    if (!pending.has(f.id) || !['result','error'].includes(f.op)) return;
    pending.delete(f.id);
    // Errors may contain host paths: retain only a fixed classification.
    output.probes[f.id] = f.op === 'result'
      ? {status:'ok', ...(Array.isArray(f.result) ? {count:f.result.length} : {})}
      : {status:/no provider/i.test(f.error) ? 'not registered' : /denied|authority|unauthorized|forbidden/i.test(f.error) ? 'forbidden' : 'error'};
    if (f.id === 'remote.tailscaleInfo' && f.op === 'result') output.network={available:f.result.available,serveActive:f.result.serveActive,canServe:f.result.canServe};
    if (f.id === 'desktop.agentRuntimeStatus' && f.op === 'result') output.runtime=f.result;
    if (f.id === 'machine.power' && f.op === 'result') output.idle = {mode:f.result.idleMode, timeoutSeconds:f.result.idleTimeoutSeconds, quiescent:f.result.idle?.quiescent, calmSeconds:f.result.idle?.calmSeconds, blockerKinds:(f.result.idle?.blockers ?? []).map(b=>b.kind)};
    if (!pending.size) finish();
  };
  ws.onerror = () => { clearTimeout(timer); console.error('Capability connection failed'); process.exitCode = 1; };
})().catch(() => { console.error('Capability inventory failed'); process.exitCode = 1; });
'''
code = "const expectParity = " + ("true" if args.expect_parity else "false") + ";\n" + code
code = "const expectPluginAdmin = " + ("true" if args.expect_plugin_admin else "false") + ";\n" + code
subprocess.run(["flyctl", "ssh", "console", "-a", args.app, "--machine", args.machine,
                "-C", shlex.join(["node", "-e", code])], check=True)
