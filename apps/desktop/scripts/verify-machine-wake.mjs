// Exercise the real mobile button across a guarded Fly stop/start cycle.
// Usage: node scripts/verify-machine-wake.mjs APP MACHINE MOBILE_URL WAKE_URL --stop-idle
import { execFileSync } from 'node:child_process';
import { chromium, expect } from '@playwright/test';
const [app, machine, mobileURL, wakeURL, flag] = process.argv.slice(2);
if (!app || !machine || !mobileURL || !wakeURL || flag !== '--stop-idle')
  throw new Error('Expected APP MACHINE MOBILE_URL WAKE_URL --stop-idle');
const fly = (...args) => execFileSync('flyctl', args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
const browser = await chromium.launch();
try {
  const page = await browser.newPage();
  await page.addInitScript(wake => {
    const bus = `${location.protocol === 'https:' ? 'wss:' : 'ws:'}//${location.host}/bus`;
    localStorage.setItem('wks.machine.paused:' + bus, '1');
    localStorage.setItem('wks.machine.wake:' + bus, wake);
  }, wakeURL);
  await page.goto(mobileURL);
  const button = page.getByRole('button', { name: 'Wake server', exact: true });
  await expect(button).toBeVisible();
  // Inspect activity privately; only report blocker kinds, never credential files.
  const guard = `import os,json,subprocess,sys
env=dict(os.environ,XDG_CONFIG_HOME='/data/hub/home/.config')
env.pop('HUB_TOKEN',None)
r=subprocess.run(['workspacer','fleet','idle','--json'],env=env,capture_output=True,text=True)
if r.returncode not in (0,1):sys.exit('Cannot inspect live work')
state=json.loads(r.stdout)['idle']
blockers=[b['kind'] for b in (state.get('blockers') or []) if b['kind'] not in ('client-active','dwell')]
if blockers:sys.exit('Work blocks wake verification: '+str(blockers))
print('No work blockers')`;
  const quote = s => "'" + s.replaceAll("'", "'\\''") + "'";
  console.log(fly('ssh', 'console', '-a', app, '--machine', machine, '-C', `python3 -c ${quote(guard)}`).trim());
  fly('machine', 'stop', machine, '-a', app);
  fly('machine', 'wait', machine, '-a', app, '--state', 'stopped', '--wait-timeout', '60s');
  console.log('Machine stopped; clicking the real mobile Wake button');
  await button.click();
  await expect(page.locator('#machinePower')).toBeHidden({ timeout: 70000 });
  fly('machine', 'wait', machine, '-a', app, '--state', 'started', '--wait-timeout', '60s');
  console.log('PASS: mobile Wake started the stopped machine and cleared reconnect pause');
} finally {
  await browser.close();
}
