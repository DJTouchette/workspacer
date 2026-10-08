// Settings (native ui/settings.rs, phone-sized): Appearance (the eight native
// themes rendered from their real tokens, Match the phone, text size, reduce
// motion), Notifications (push prefs, shared with /m), Workspace (hub,
// machine power, projects, jobs, briefs, history) and About.
import { bus, can, isOperator, stopMachine, machineIdleText, reconnectNow } from '../bus.js';
import { peers, proposals, sessions } from '../store.js';
import { esc, ic, errText } from '../util.js';
import { notice, pick, ask } from '../ui.js';
import { THEMES, pref, setPref, textSize, setTextSize, reduceMotion, setReduceMotion, resolved } from '../theme.js';
import * as push from '../push.js';

export function mount(root, ctx) {
  root.innerHTML = `<div class="screen page-screen">
    <div class="pagehead"><button class="ibtn" data-back aria-label="Back">${ic('chevron-left', 's20')}</button><span class="grow"></span></div>
    <div class="page scrolly" data-page></div>
  </div>`;
  const page = root.querySelector('[data-page]');
  root.querySelector('[data-back]').onclick = () => ctx.back();

  const toggle = (on, attr, label) => `<button class="toggle${on ? ' on' : ''}" role="switch" aria-checked="${on}" aria-label="${esc(label)}" ${attr}></button>`;
  function update() {
    const p = pref();
    const current = resolved();
    const perm = push.permission();
    const after = push.FINISHED_CHOICES.find((c) => c.sec === push.finishedAfter()) || push.FINISHED_CHOICES[1];
    const scope = bus.scope.name || 'operator';
    const props = proposals().length;
    page.innerHTML = `
      <div class="ptitle"><span class="overline">Make it yours</span><h1>Settings</h1></div>
      <div class="glabel">Appearance</div>
      <div class="group">
        <div class="themes" role="radiogroup" aria-label="Theme">${THEMES.map((t) => `<button class="tt-tile${!p.match && p.theme === t.slug ? ' on' : p.match && current === t.slug ? ' sys' : ''}" data-theme-pick="${t.slug}" role="radio" aria-checked="${!p.match && p.theme === t.slug}" aria-label="${esc(t.label)}">
          <span class="sw" data-theme="${t.slug}"><span class="sb"></span><span class="mn"><i></i><i></i><b></b></span></span><span>${esc(t.label.replace('Catppuccin ', ''))}</span></button>`).join('')}</div>
        <div class="gi"><span class="tx">Match the phone<span class="s">Dark and Light follow the system setting</span></span>${toggle(p.match, 'data-match', 'Match the phone')}</div>
        <button class="gi" data-size><span class="tx">Text size<span class="s">Conversation text</span></span><span class="v">${textSize()} ${ic('chevron-right', 's14')}</span></button>
        <div class="gi"><span class="tx">Reduce motion<span class="s">Sheets and the island change at once</span></span>${toggle(reduceMotion(), 'data-motion', 'Reduce motion')}</div>
      </div>

      <div class="glabel">Notifications</div>
      <div class="group">
        ${perm === 'granted' ? '' : `<div class="gi"><span class="ico">${ic('bell', 's16')}</span><span class="tx">${perm === 'unsupported' ? 'Add Workspacer to your Home Screen' : perm === 'denied' ? 'Notifications are blocked' : 'Notifications are off'}
          <span class="s">${perm === 'unsupported' ? 'Phones only deliver web notifications to installed apps (and over HTTPS).' : perm === 'denied' ? 'Allow them for this site in the system settings.' : 'Approvals and questions reach you even when the app is closed.'}</span></span>
          ${perm === 'default' ? '<button class="btn sm primary" data-enable>Turn on</button>' : ''}</div>`}
        <div class="gi"><span class="ico">${ic('bell', 's16')}</span><span class="tx">Needs you<span class="s">Approvals and questions</span></span>${toggle(push.prefOn('needs'), 'data-pref="needs"', 'Needs you')}</div>
        <div class="gi"><span class="ico">${ic('circle-check', 's16')}</span><span class="tx">Finished turns</span>${toggle(push.prefOn('finished'), 'data-pref="finished"', 'Finished turns')}</div>
        <button class="gi" data-after${push.prefOn('finished') ? '' : ' disabled'}><span class="ico">${ic('clock', 's16')}</span><span class="tx">Only when a turn ran</span><span class="v">${esc(after.label)} ${ic('chevron-right', 's14')}</span></button>
        <div class="gi"><span class="ico">${ic('circle-stop', 's16')}</span><span class="tx">Session ends</span>${toggle(push.prefOn('ended'), 'data-pref="ended"', 'Session ends')}</div>
        <div class="gi"><span class="ico">${ic('loader-circle', 's16')}</span><span class="tx">Still working<span class="s">A ping at 10 and 30 minutes</span></span>${toggle(push.prefOn('checkpoints'), 'data-pref="checkpoints"', 'Still working')}</div>
        <div class="gi"><span class="ico">${ic('message-square-plus', 's16')}</span><span class="tx">Show message contents</span>${toggle(push.prefOn('preview'), 'data-pref="preview"', 'Show message contents')}</div>
        ${perm === 'granted' && can('push.test') ? `<button class="gi" data-test><span class="ico">${ic('megaphone', 's16')}</span><span class="tx">Send a test notification</span></button>` : ''}
      </div>

      <div class="glabel">Workspace</div>
      <div class="group">
        <button class="gi" data-hub><span class="ico">${ic('server', 's16')}</span><span class="tx">${esc(location.host)}
          <span class="s">${bus.connected ? 'Connected' : 'Reconnecting…'} · ${esc(scope)} token${peers.length ? ` · ${peers.length} peer${peers.length === 1 ? '' : 's'}` : ''} · ${sessions.size} session${sessions.size === 1 ? '' : 's'}</span></span>
          <span class="v">${bus.connected ? '' : 'Retry'}</span></button>
        ${bus.machinePower && bus.machinePower.idleMode && bus.machinePower.idleMode !== 'off' ? `<button class="gi" data-idle><span class="ico">${ic('clock', 's16')}</span><span class="tx">Idle mode<span class="s">${esc(bus.machinePower.idleMode)}</span></span><span class="v">${ic('chevron-right', 's14')}</span></button>` : ''}
        ${bus.machineCanStop ? `<button class="gi danger" data-stop-machine><span class="ico">${ic('power', 's16')}</span><span class="tx">Stop this server<span class="s">Ends running work and disconnects everyone</span></span></button>` : ''}
        <button class="gi" data-go="#/projects"><span class="ico">${ic('folder', 's16')}</span><span class="tx">Projects</span><span class="v">${ic('chevron-right', 's14')}</span></button>
        ${can('jobs.list') ? `<button class="gi" data-go="#/jobs"><span class="ico">${ic('calendar', 's16')}</span><span class="tx">Jobs<span class="s">${props ? `${props} proposed job${props === 1 ? '' : 's'} waiting for approval` : 'Scheduled work on this hub'}</span></span><span class="v">${props ? '<span class="dot t-warning"></span>' : ''}${ic('chevron-right', 's14')}</span></button>` : ''}
        ${can('fs.read') && isOperator() ? `<button class="gi" data-go="#/briefs"><span class="ico">${ic('file-text', 's16')}</span><span class="tx">Briefs<span class="s">Each project’s living brief, read-only</span></span><span class="v">${ic('chevron-right', 's14')}</span></button>` : ''}
        <button class="gi" data-go="#/history"><span class="ico">${ic('history', 's16')}</span><span class="tx">Session history</span><span class="v">${ic('chevron-right', 's14')}</span></button>
      </div>

      <div class="glabel">About</div>
      <div class="group">
        <div class="gi"><span class="ico">${ic('smartphone', 's16')}</span><span class="tx">Workspacer for phones<span class="s">The native-aligned client (/m-next). The previous one stays at /m.</span></span></div>
        <button class="gi" data-diag><span class="ico">${ic('info', 's16')}</span><span class="tx">Viewport diagnostics</span><span class="v">${ic('copy', 's14')}</span></button>
        <button class="gi danger" data-signout><span class="ico">${ic('x', 's16')}</span><span class="tx">Forget this device’s token</span></button>
      </div>`;
  }

  page.addEventListener('click', async (e) => {
    const b = e.target.closest('button');
    if (!b) return;
    if (b.dataset.themePick) setPref({ match: false, theme: b.dataset.themePick });
    else if (b.hasAttribute('data-match')) { const p = pref(); setPref({ match: !p.match, theme: p.match ? resolved() : p.theme }); }
    else if (b.hasAttribute('data-size')) {
      const v = await pick('Text size', [14, 15, 16, 17, 18, 20].map((n) => ({ value: n, label: `${n}${n === 16 ? ' (default)' : ''}`, on: textSize() === n })));
      if (v) setTextSize(v);
    } else if (b.hasAttribute('data-motion')) setReduceMotion(!reduceMotion());
    else if (b.hasAttribute('data-enable')) notice((await push.ensurePush(false)) || 'Notifications on');
    else if (b.dataset.pref) push.savePrefs({ ...push.prefs, [b.dataset.pref]: !push.prefOn(b.dataset.pref) });
    else if (b.hasAttribute('data-after')) {
      const v = await pick('Notify when a turn ran', push.FINISHED_CHOICES.map((c) => ({ value: String(c.sec), label: c.label, on: c.sec === push.finishedAfter() })));
      if (v !== undefined) push.savePrefs({ ...push.prefs, finishedAfterSec: Number(v) });
    } else if (b.hasAttribute('data-test')) notice(await push.testPush());
    else if (b.hasAttribute('data-hub')) { if (!bus.connected) { reconnectNow(); notice('Reconnecting…'); } }
    else if (b.hasAttribute('data-idle')) { try { alert(await machineIdleText()); } catch (err) { notice(errText(err), 'error'); } }
    else if (b.hasAttribute('data-stop-machine')) stopMachine();
    else if (b.dataset.go) ctx.go(b.dataset.go);
    else if (b.hasAttribute('data-diag')) {
      const vv = window.visualViewport;
      const d = { standalone: navigator.standalone === true || matchMedia('(display-mode: standalone)').matches, screen: `${screen.width}x${screen.height}`,
        inner: `${innerWidth}x${innerHeight}`, vv: vv ? `${Math.round(vv.width)}x${Math.round(vv.height)} off=${Math.round(vv.offsetTop)}` : 'none',
        vh: getComputedStyle(document.documentElement).getPropertyValue('--vh').trim(), ua: navigator.userAgent };
      try { await navigator.clipboard.writeText('Viewport diagnostics: ' + JSON.stringify(d)); notice('Copied — paste it to an agent'); }
      catch { alert(JSON.stringify(d, null, 2)); }
    } else if (b.hasAttribute('data-signout')) {
      if (await ask({ title: 'Forget this token?', body: 'This phone disconnects and asks for a token next time. The token itself stays valid until you revoke it on the desktop.', confirm: 'Forget', danger: true })) {
        localStorage.removeItem('hubToken');
        location.replace(location.pathname);
      }
    }
    update();
  });
  update();
  return { update };
}
