// The hub bus connection: token, socket, calls, reconnect, wake and the
// machine-stop pause. The protocol and the machine-power half are /m's
// (mobile.html "transport" and "connection"); the reconnect is this client's.
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
// Reopening the app on a phone is the case this is shaped around. The socket
// the page went to sleep with may be fine or may be a zombie, and a fresh
// handshake over a tailnet that is still waking can stall for seconds. So:
//   - a quiet socket is PROBED (one frame, one round trip) while a spare
//     handshake is dialled beside it; whichever answers first is the socket.
//   - a handshake that has not opened is not abandoned for a newer one: more
//     are started beside it and the first to open wins.
//   - calls on a socket that is gone fail now, not at their 15s timeout.
let ws = null, callSeq = 0, backoff = 500, reconnectTimer = null, reconcileTimer = null;
let lastActivity = 0, lastInteractionSent = 0, lastDialAt = 0, raceTimer = null, probeTimer = null;
const pending = new Map();
/** Handshakes in flight: socket -> the timer that gives up on it. */
const dialing = new Map();
/** No frame for this long while visible: the socket needs proving. */
const STALE_MS = 30000;
/** Back in the foreground: a socket this recently heard from is taken as live. */
const FRESH_MS = 4000;
/** A live hub acknowledges a subscribe in one round trip. */
const PROBE_MS = 2500;
/** A cold tailnet leg can need this long to finish one handshake. */
const HANDSHAKE_MS = 20000;
/** While none has opened, start another this long after the previous one. */
const RACE_MS = [1500, 3000];
const MAX_DIALS = 4;
const TOPICS = ['agent.snapshot', 'workflow.*', 'hub.peer.*', 'node.state_changed', 'sessionArchive.changed', 'sessionOpen.changed'];
const isOpen = (sock) => !!sock && sock.readyState === WebSocket.OPEN;

export function call(method, params, timeoutMs) {
  return new Promise((resolve, reject) => {
    if (bus.machinePaused || !isOpen(ws)) return reject(new Error('offline'));
    const id = 'c' + (++callSeq), sock = ws, sentAt = Date.now();
    pending.set(id, { resolve, reject, sock });
    sock.send(JSON.stringify({ op: 'call', id, method, params: params || {} }));
    setTimeout(() => {
      if (!pending.has(id)) return;
      pending.delete(id);
      reject(new Error('timeout'));
      // Not one frame since this was sent: doubt the socket, not the method.
      if (ws === sock && lastActivity < sentAt) verify();
    }, timeoutMs || 15000);
  });
}
/** Replies to these can no longer arrive: say so now. */
function failPending(sock) {
  for (const [id, c] of pending) {
    if (c.sock === sock) { pending.delete(id); c.reject(new Error('offline')); }
  }
}

function killSocket(sock) {
  if (!sock) return;
  sock.onmessage = sock.onerror = sock.onclose = null;
  // Closing a socket mid-handshake is reported as a failed connection; let it
  // open and close cleanly instead.
  if (sock.readyState === WebSocket.CONNECTING) { sock.onopen = () => { try { sock.close(); } catch { /* gone */ } }; return; }
  sock.onopen = null;
  try { sock.close(); } catch { /* gone */ }
}
function setConnected(on) {
  if (bus.connected === on) return;
  bus.connected = on;
  emit('state', on);
}
function stopDialing() {
  for (const [sock, timer] of dialing) { clearTimeout(timer); killSocket(sock); }
  dialing.clear();
  clearTimeout(raceTimer); raceTimer = null;
}
function dropSocket() {
  clearTimeout(probeTimer); probeTimer = null;
  const old = ws;
  ws = null;
  killSocket(old);
  if (old) failPending(old);
  setConnected(false);
}

export function setToken(token) {
  bus.token = token;
  localStorage.setItem('hubToken', token);
  connect();
}

/** Start over: drop the socket and every handshake, then dial. */
export function connect() {
  if (bus.machinePaused) return;
  if (!bus.token) { setConnected(false); emit('state', false); return; }
  stopDialing();
  dropSocket();
  backoff = 500;
  startDial();
}

function dial() {
  if (dialing.size >= MAX_DIALS) return false;
  let sock;
  try { sock = new WebSocket(`${busURL}?token=${encodeURIComponent(bus.token)}`); } catch { return false; }
  lastDialAt = Date.now();
  dialing.set(sock, setTimeout(() => lost(sock), HANDSHAKE_MS));
  sock.onopen = () => adopt(sock);
  sock.onmessage = (ev) => { if (ws === sock) received(ev); };
  sock.onclose = (ev) => closed(sock, ev);
  sock.onerror = () => { try { sock.close(); } catch { /* gone */ } };
  return true;
}
/** Dial, and keep adding handshakes beside a slow one until one opens. */
function startDial() {
  if (bus.machinePaused || !bus.token || dialing.size) return;
  if (reconnectTimer) { clearTimeout(reconnectTimer); reconnectTimer = null; }
  if (!dial()) return scheduleReconnect();
  let n = 0;
  const more = () => {
    raceTimer = n < RACE_MS.length ? setTimeout(() => { if (dialing.size) { dial(); more(); } }, RACE_MS[n++]) : null;
  };
  clearTimeout(raceTimer);
  more();
}
/** A handshake failed or ran out of time. */
function lost(sock) {
  clearTimeout(dialing.get(sock));
  dialing.delete(sock);
  killSocket(sock);
  if (dialing.size) return;
  clearTimeout(raceTimer); raceTimer = null;
  if (!ws) scheduleReconnect();
}
/** The first handshake to open is the socket; the rest are let go. */
function adopt(sock) {
  clearTimeout(dialing.get(sock));
  dialing.delete(sock);
  stopDialing();
  clearTimeout(probeTimer); probeTimer = null;
  const old = ws;
  ws = sock;
  if (old) { killSocket(old); failPending(old); }
  backoff = 500; lastActivity = Date.now();
  setConnected(true);
  reportInteraction(true);
  sock.send(JSON.stringify({ op: 'subscribe', topics: TOPICS }));
  emit('open');
  // Reconcile every 25s so missed events self-heal. Its replies are also the
  // heartbeat: a visible page that heard nothing since the last one has a
  // socket worth proving.
  clearInterval(reconcileTimer);
  reconcileTimer = setInterval(() => {
    if (document.visibilityState !== 'visible' || !isOpen(ws)) return;
    if (Date.now() - lastActivity > STALE_MS) verify();
    emit('reconcile');
  }, 25000);
}
function received(ev) {
  lastActivity = Date.now();
  // Any frame proves the socket: the spare handshakes are not needed.
  if (probeTimer) { clearTimeout(probeTimer); probeTimer = null; stopDialing(); }
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
}
function closed(sock, ev) {
  if (ev.code === 4001) { pauseForMachineStop(); return; }
  if (ev.code === 1008 || ev.code === 4401) {
    stopDialing();
    dropSocket();
    bus.token = '';
    localStorage.removeItem('hubToken');
    bus.scope = { name: '', methods: [], fullAccess: false };
    emit('unauthorized');
    return;
  }
  if (dialing.has(sock)) return lost(sock);
  if (ws !== sock) return;
  dropSocket();
  if (!dialing.size) scheduleReconnect();
}

function scheduleReconnect() {
  if (bus.machinePaused || reconnectTimer || dialing.size) return;
  reconnectTimer = setTimeout(() => { reconnectTimer = null; startDial(); }, backoff);
  backoff = Math.min(backoff * 2, 5000);
}

/** Retry now rather than waiting out the backoff (a tap on "Reconnecting"). */
export function reconnectNow() { connect(); }

/** Is the socket still there? Ask it (a subscribe is acknowledged, and asking
 *  again changes nothing) and dial a spare beside it, so a zombie costs no
 *  more than the handshake that replaces it and a live one costs a round trip. */
function verify() {
  if (bus.machinePaused || !bus.token || probeTimer) return;
  if (!isOpen(ws)) { dropSocket(); startDial(); return; }
  const sock = ws;
  try { sock.send(JSON.stringify({ op: 'subscribe', topics: TOPICS })); } catch { /* the timer decides */ }
  startDial();
  probeTimer = setTimeout(() => {
    probeTimer = null;
    if (ws !== sock) return;
    dropSocket();
    if (!dialing.size) startDial();
  }, PROBE_MS);
}

// Mobile browsers suspend a backgrounded tab's socket, often without a close.
// On every sign we are foreground again: prove a quiet socket, or hurry a
// reconnect along rather than waiting out its backoff.
function wake() {
  if (bus.machinePaused || !bus.token || document.visibilityState === 'hidden') return;
  if (isOpen(ws)) {
    if (Date.now() - lastActivity >= FRESH_MS) verify();
    return;
  }
  if (ws) dropSocket();
  backoff = 500;
  // Handshakes begun before the page slept may never finish: add a fresh one
  // beside them (these events arrive in bursts, so not one per event).
  if (!dialing.size) startDial();
  else if (Date.now() - lastDialAt > 1000) dial();
}
document.addEventListener('visibilitychange', wake);
window.addEventListener('online', wake);
window.addEventListener('focus', wake);
window.addEventListener('pageshow', wake);

/** Tell an idle-aware hub a person is here (at most every 30s). */
function reportInteraction(force = false) {
  if (bus.machinePaused || !bus.connected || !isOpen(ws)) return;
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
  clearInterval(reconcileTimer);
  stopDialing();
  // Reject with the reason before dropSocket's generic 'offline' can.
  const waiting = [...pending.values()];
  pending.clear();
  for (const c of waiting) c.reject(new Error('Machine disconnected after stop request'));
  dropSocket();
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
  connect();
}

/** The idle summary machine.power reports, as one readable block. */
export async function machineIdleText() {
  const info = await call('machine.power');
  const idle = info.idle;
  return 'Idle mode: ' + info.idleMode + '\n' +
    Math.floor((idle?.calmSeconds || 0) / 60) + ' / ' + Math.ceil((idle?.dwellSeconds || 0) / 60) + ' minutes quiet\n\n' +
    (idle?.quiescent ? 'Ready to stop.' : (idle?.blockers || []).map((b) => b.detail).join('\n'));
}
