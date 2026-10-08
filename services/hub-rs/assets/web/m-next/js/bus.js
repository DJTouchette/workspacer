// The hub bus connection: token, socket, calls, reconnect, watchdogs, wake
// and the machine-stop pause. Ported from /m (mobile.html "transport" and
// "connection") with its behaviour intact — only the rendering moved out.
//
// Nothing here renders. Views subscribe with `on(type, fn)`:
//   'state'        connected flipped (bus.connected)
//   'open'         a socket opened (seed now; scope is not known yet)
//   'hello'        the bus said what this token may do (bus.scope)
//   'event'        a published event envelope {type, data, hub?}
//   'unauthorized' the token was refused; it has been forgotten
//   'paused'       a machine stop was requested; reconnect is paused
import { errText } from './util.js';

const listeners = new Map();
export function on(type, fn) {
  if (!listeners.has(type)) listeners.set(type, new Set());
  listeners.get(type).add(fn);
  return () => listeners.get(type).delete(fn);
}
function emit(type, arg) {
  for (const fn of listeners.get(type) || []) {
    try { fn(arg); } catch (e) { console.error('[bus]', type, e); }
  }
}

const qs = new URLSearchParams(location.search);
const busURL = `${location.protocol === 'https:' ? 'wss:' : 'ws:'}//${location.host}/bus`;

/** Shared with /m on purpose: the same origin, the same paired token. */
export const bus = {
  token: qs.get('token') || localStorage.getItem('hubToken') || '',
  connected: false,
  /** What the hello frame said this token may do. Empty = not known yet. */
  scope: { name: '', methods: [], fullAccess: false },
  /** A machine stop was requested; nothing reconnects until Wake. */
  machinePaused: false,
  machineCanStop: false,
  machinePower: null,
  machineWakeURL: '',
};
if (bus.token) localStorage.setItem('hubToken', bus.token);

/** May this token call `method`? Permissive before hello (older hubs send no
 *  method list); the bus enforces the real boundary either way. */
export const can = (method) =>
  !bus.scope.methods.length ||
  bus.scope.methods.some((p) => p === '*' || p === method || (p.endsWith('.*') && method.startsWith(p.slice(0, -1))));
export const isOperator = () => !bus.scope.name || bus.scope.name === 'operator';

// ── machine power (a node that was asked to stop) ────────────────────────
const machinePauseKey = 'wks.machine.paused:' + busURL;
const machineWakeKey = 'wks.machine.wake:' + busURL;
function safeWakeURL(raw) {
  try {
    const u = new URL(raw);
    return u.protocol === 'https:' && !u.username && !u.password && !u.search && !u.hash ? u.href : '';
  } catch { return ''; }
}
bus.machinePaused = localStorage.getItem(machinePauseKey) === '1';
bus.machineWakeURL = safeWakeURL(localStorage.getItem(machineWakeKey));

// ── socket ────────────────────────────────────────────────────────────────
let ws = null, callSeq = 0, backoff = 500, reconnectTimer = null, reconcileTimer = null;
let watchdog = null, lastActivity = 0, attempt = 0, lastInteractionSent = 0;
const pending = new Map();
const STALE_MS = 30000;
/** How long to let a handshake run, by consecutive failed attempt. A real
 *  blip reconnects in milliseconds; a cold tailnet leg needs room to finish. */
const HANDSHAKE_BUDGETS_MS = [4500, 9000, 20000];
const TOPICS = ['agent.snapshot', 'workflow.*', 'hub.peer.*', 'node.state_changed', 'sessionArchive.changed'];

export function call(method, params, timeoutMs) {
  return new Promise((resolve, reject) => {
    if (bus.machinePaused || !ws || ws.readyState !== WebSocket.OPEN) return reject(new Error('offline'));
    const id = 'c' + (++callSeq);
    pending.set(id, { resolve, reject });
    ws.send(JSON.stringify({ op: 'call', id, method, params: params || {} }));
    setTimeout(() => {
      if (pending.has(id)) { pending.delete(id); reject(new Error('timeout')); }
    }, timeoutMs || 15000);
  });
}

function killSocket(sock) {
  if (!sock) return;
  try { sock.onopen = sock.onmessage = sock.onerror = sock.onclose = null; sock.close(); } catch { /* gone */ }
}
function setConnected(on) {
  if (bus.connected === on) return;
  bus.connected = on;
  emit('state', on);
}

export function setToken(token) {
  bus.token = token;
  localStorage.setItem('hubToken', token);
  connect();
}

export function connect() {
  if (bus.machinePaused) return;
  if (!bus.token) { setConnected(false); emit('state', false); return; }
  if (reconnectTimer) { clearTimeout(reconnectTimer); reconnectTimer = null; }
  clearTimeout(watchdog);
  setConnected(false);
  let sock;
  try { sock = new WebSocket(`${busURL}?token=${encodeURIComponent(bus.token)}`); } catch { return scheduleReconnect(); }
  ws = sock;
  // Phones hand us sockets whose handshake hangs (the host or the tailnet
  // leg was asleep): abandon early, with a budget that grows per attempt.
  const budget = HANDSHAKE_BUDGETS_MS[Math.min(attempt, HANDSHAKE_BUDGETS_MS.length - 1)];
  attempt++;
  watchdog = setTimeout(() => {
    if (sock.readyState !== WebSocket.OPEN) { killSocket(sock); if (ws === sock) ws = null; scheduleReconnect(); }
  }, budget);
  sock.onopen = () => {
    clearTimeout(watchdog);
    backoff = 500; attempt = 0; lastActivity = Date.now();
    setConnected(true);
    reportInteraction(true);
    sock.send(JSON.stringify({ op: 'subscribe', topics: TOPICS }));
    emit('open');
    // Reconcile every 25s so missed events self-heal; doubles as the
    // heartbeat wake() checks.
    clearInterval(reconcileTimer);
    reconcileTimer = setInterval(() => {
      if (document.visibilityState === 'visible' && ws && ws.readyState === WebSocket.OPEN) emit('reconcile');
    }, 25000);
  };
  sock.onmessage = (ev) => {
    lastActivity = Date.now();
    let f;
    try { f = JSON.parse(ev.data); } catch { return; }
    if (f.op === 'hello') {
      bus.scope = { name: f.scope || '', methods: Array.isArray(f.methods) ? f.methods : [], fullAccess: f.spawnFullAccess === true };
      emit('hello', bus.scope);
      discoverMachine();
    } else if (f.op === 'result') {
      const c = pending.get(f.id);
      if (c) { pending.delete(f.id); c.resolve(f.result); }
    } else if (f.op === 'error' && f.id) {
      const c = pending.get(f.id);
      if (c) { pending.delete(f.id); c.reject(new Error(f.error || 'error')); }
    } else if (f.op === 'event' && f.event) {
      emit('event', f.event);
    }
  };
  sock.onclose = (ev) => {
    if (ev.code === 4001) { pauseForMachineStop(); return; }
    clearTimeout(watchdog);
    if (ws === sock) ws = null;
    setConnected(false);
    if (ev.code === 1008 || ev.code === 4401) {
      bus.token = '';
      localStorage.removeItem('hubToken');
      bus.scope = { name: '', methods: [], fullAccess: false };
      emit('unauthorized');
      return;
    }
    scheduleReconnect();
  };
  sock.onerror = () => { try { sock.close(); } catch { /* gone */ } };
}

function scheduleReconnect() {
  if (bus.machinePaused || reconnectTimer) return;
  reconnectTimer = setTimeout(() => { reconnectTimer = null; connect(); }, backoff);
  backoff = Math.min(backoff * 2, 5000);
}

/** Retry now rather than waiting out the backoff (a tap on "Reconnecting"). */
export function reconnectNow() {
  backoff = 500; attempt = 0;
  killSocket(ws); ws = null;
  if (reconnectTimer) { clearTimeout(reconnectTimer); reconnectTimer = null; }
  connect();
}

// Mobile browsers suspend a backgrounded tab's socket, often without a close;
// reconnect on every sign we are foreground again unless provably live.
function wake() {
  if (bus.machinePaused || !bus.token || document.visibilityState === 'hidden') return;
  const live = ws && ws.readyState === WebSocket.OPEN && Date.now() - lastActivity < STALE_MS;
  if (live) return;
  backoff = 500; attempt = 0; clearTimeout(watchdog);
  killSocket(ws); ws = null; connect();
}
document.addEventListener('visibilitychange', wake);
window.addEventListener('online', wake);
window.addEventListener('focus', wake);
window.addEventListener('pageshow', wake);

/** Tell an idle-aware hub a person is here (at most every 30s). */
function reportInteraction(force = false) {
  if (bus.machinePaused || !bus.connected || !ws || ws.readyState !== WebSocket.OPEN) return;
  if (!force && Date.now() - lastInteractionSent < 30000) return;
  lastInteractionSent = Date.now();
  ws.send(JSON.stringify({ op: 'activity' }));
}
for (const type of ['pointerdown', 'keydown', 'wheel', 'touchmove']) {
  document.addEventListener(type, (event) => {
    if (event.isTrusted && document.visibilityState === 'visible') reportInteraction();
  }, { capture: true, passive: true });
}

// ── machine power ────────────────────────────────────────────────────────
function discoverMachine() {
  if (!can('machine.power')) return;
  call('machine.power').then((info) => {
    bus.machineCanStop = info.canStop === true;
    bus.machinePower = info;
    bus.machineWakeURL = safeWakeURL(info.wakeUrl);
    if (bus.machineWakeURL) localStorage.setItem(machineWakeKey, bus.machineWakeURL);
    else localStorage.removeItem(machineWakeKey);
    emit('machine');
    if (info.error) alert(info.error);
  }).catch(() => { bus.machineCanStop = false; emit('machine'); });
}

function pauseForMachineStop() {
  bus.machinePaused = true;
  localStorage.setItem(machinePauseKey, '1');
  clearTimeout(reconnectTimer); reconnectTimer = null;
  clearTimeout(watchdog); clearInterval(reconcileTimer);
  killSocket(ws); ws = null; setConnected(false);
  for (const c of pending.values()) c.reject(new Error('Machine disconnected after stop request'));
  pending.clear();
  emit('paused');
}

/** Stop the server this hub runs on (machine.stop), after a confirm. */
export async function stopMachine() {
  if (!bus.machineCanStop || !bus.connected) return;
  if (!confirm('Stop this server? This ends running work and disconnects everyone. Storage charges continue.')) return;
  try { await call('machine.stop'); pauseForMachineStop(); }
  catch (err) { if (!bus.machinePaused) alert(errText(err)); }
}

/** Wake a paused machine: ping its doorbell, then reconnect. */
export async function wakeMachine() {
  if (bus.machineWakeURL) {
    await fetch(bus.machineWakeURL, {
      mode: 'no-cors', credentials: 'omit', cache: 'no-store', redirect: 'follow', signal: AbortSignal.timeout(60000),
    });
  }
  bus.machinePaused = false;
  localStorage.removeItem(machinePauseKey);
  emit('paused');
  backoff = 500; attempt = 0; connect();
}

/** The idle summary machine.power reports, as one readable block. */
export async function machineIdleText() {
  const info = await call('machine.power');
  const idle = info.idle;
  return 'Idle mode: ' + info.idleMode + '\n' +
    Math.floor((idle?.calmSeconds || 0) / 60) + ' / ' + Math.ceil((idle?.dwellSeconds || 0) / 60) + ' minutes quiet\n\n' +
    (idle?.quiescent ? 'Ready to stop.' : (idle?.blockers || []).map((b) => b.detail).join('\n'));
}
