// Web Push and the service worker, ported from /m (see
// .rivet/context/modules/hub-web-push.md). A PWA cannot hold a socket in the
// background; the hub pushes when an agent needs you, the worker shows the
// notification, and a tap deep-links back here.
//
// /m-next registers its own worker at ./sw.js (scope /m-next/), so /m's worker
// and subscription are untouched. Preferences are shared with /m (same
// localStorage key): they describe the person, not the client.
import { call, on as onBus } from './bus.js';
import { errText } from './util.js';

export const pushOk = 'serviceWorker' in navigator && 'PushManager' in window && 'Notification' in window;
let swReg = null;

export async function registerSW() {
  if (!('serviceWorker' in navigator)) return null;
  if (swReg) return swReg;
  try { swReg = await navigator.serviceWorker.register('./sw.js', { scope: './' }); return swReg; } catch { return null; }
}

const PUSH_PREF_KEY = 'pushPrefs';
export const FINISHED_CHOICES = [
  { sec: 0, label: 'Any length' },
  { sec: 60, label: 'Over 1 min' },
  { sec: 300, label: 'Over 5 min' },
  { sec: 900, label: 'Over 15 min' },
];
function load() {
  try { return JSON.parse(localStorage.getItem(PUSH_PREF_KEY)) || {}; } catch { return {}; }
}
export let prefs = load();
/** Absent = on (the hub's default) — except checkpoints, which default off. */
export const prefOn = (k) => (k === 'checkpoints' ? prefs.checkpoints === true : prefs[k] !== false);
export const finishedAfter = () => (typeof prefs.finishedAfterSec === 'number' ? prefs.finishedAfterSec : 60);
export function savePrefs(next) {
  prefs = next;
  try { localStorage.setItem(PUSH_PREF_KEY, JSON.stringify(next)); } catch { /* private mode */ }
  // Re-send now, so turning off an annoying notification takes effect now.
  if (pushOk && Notification.permission === 'granted') ensurePush(true);
}
export const permission = () => (pushOk ? Notification.permission : 'unsupported');

/** Subscribe (asking permission unless `silent`) and send prefs along.
 *  Resolves to a message for the person, or '' when silent/unchanged. */
export async function ensurePush(silent) {
  if (!pushOk) return silent ? '' : 'Notifications need the app on your Home Screen';
  if (Notification.permission === 'denied') return silent ? '' : 'Notifications are blocked in settings';
  if (Notification.permission !== 'granted') {
    if (silent) return '';
    if ((await Notification.requestPermission()) !== 'granted') return 'Notifications were not allowed';
  }
  const reg = await registerSW();
  if (!reg) return silent ? '' : 'The service worker did not register';
  try {
    let sub = await reg.pushManager.getSubscription();
    if (!sub) {
      const r = await call('push.key', {});
      if (!r || !r.publicKey) return silent ? '' : 'This hub has no push key';
      sub = await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: urlB64ToUint8(r.publicKey) });
    }
    // Prefs ride on push.subscribe: no extra bus surface; an older hub ignores them.
    await call('push.subscribe', Object.assign(sub.toJSON(), {
      prefs: {
        needs: prefOn('needs'),
        finished: prefOn('finished'),
        ended: prefOn('ended'),
        finishedAfterSec: finishedAfter(),
        preview: prefOn('preview'),
        checkpoints: prefOn('checkpoints'),
      },
    }));
    return silent ? '' : 'Notifications on';
  } catch (e) { return silent ? '' : "Couldn't enable: " + errText(e); }
}

/** Report what HAPPENED, not how many rows were stored. */
export async function testPush() {
  try {
    const r = (await call('push.test', {})) || {};
    const d = r.delivered || 0, gone = r.gone || 0, failed = r.failed || 0;
    if (!r.devices) return 'No devices subscribed yet';
    if (d) return `Delivered to ${d}${gone ? ` · ${gone} stale removed` : ''}${failed ? ` · ${failed} failed` : ''}`;
    if (gone) return `All ${gone} subscriptions were stale — removed. Turn notifications on again.`;
    return `The push service refused all ${failed} — see the hub log`;
  } catch (e) { return 'Test failed: ' + errText(e); }
}

function urlB64ToUint8(b64) {
  const pad = '='.repeat((4 - (b64.length % 4)) % 4);
  const s = (b64 + pad).replace(/-/g, '+').replace(/_/g, '/');
  const raw = atob(s), arr = new Uint8Array(raw.length);
  for (let i = 0; i < raw.length; i++) arr[i] = raw.charCodeAt(i);
  return arr;
}

/** A notification tap names a session: hand it to whoever opens chats. */
export function onOpenAgent(fn) {
  if (!('serviceWorker' in navigator)) return;
  navigator.serviceWorker.addEventListener('message', (e) => {
    if (e.data && e.data.type === 'open-agent' && e.data.sessionId) fn(e.data.sessionId);
  });
}

onBus('open', () => { if (pushOk && Notification.permission === 'granted') ensurePush(true); });
