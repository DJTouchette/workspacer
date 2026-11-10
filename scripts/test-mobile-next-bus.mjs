#!/usr/bin/env node
// /m-next's reconnect (assets/web/m-next/js/bus.js) against a fake browser:
// scripted sockets and a controlled clock, so the cases a real phone produces
// (a zombie socket after sleep, a handshake that stalls on a waking tailnet)
// are exact instead of flaky. Proves:
//   - a stalled handshake is raced, not waited out, and the first to open wins
//   - reopening the app PROBES a quiet socket and keeps it when it answers
//   - a zombie is replaced by the spare handshake, and its calls fail at once
//   - a refused connection backs off instead of hammering
import path from 'node:path';
import { mock } from 'node:test';
import { fileURLToPath, pathToFileURL } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
mock.timers.enable({ apis: ['setTimeout', 'setInterval', 'Date'], now: 1_000_000 });

// ── the fake browser ───────────────────────────────────────────────────────
const sockets = [];
const RTT = 100;
class FakeSocket {
  static CONNECTING = 0; static OPEN = 1; static CLOSED = 3;
  constructor(url) { this.url = url; this.readyState = 0; this.sent = []; sockets.push(this); }
  send(text) {
    const f = JSON.parse(text);
    this.sent.push(f);
    // A live hub acknowledges a subscribe one round trip later; a zombie
    // (the socket a sleeping phone was left holding) says nothing, ever.
    if (f.op === 'subscribe' && !this.zombie) setTimeout(() => { if (this.readyState === 1) this.frame({ op: 'subscribed', topics: f.topics }); }, RTT);
  }
  close() {
    if (this.readyState === 3) return;
    this.readyState = 3;
    this.closedByClient = true;
  }
  // What the network does:
  open() { this.readyState = 1; if (this.onopen) this.onopen(); }
  frame(f) { if (this.onmessage) this.onmessage({ data: JSON.stringify(f) }); }
  drop(code = 1006) { this.readyState = 3; if (this.onclose) this.onclose({ code }); }
}
const handlers = { document: new Map(), window: new Map() };
const target = (kind) => ({
  addEventListener(type, fn) {
    if (!handlers[kind].has(type)) handlers[kind].set(type, []);
    handlers[kind].get(type).push(fn);
  },
});
const store = new Map([['hubToken', 't0k']]);
globalThis.WebSocket = FakeSocket;
globalThis.location = { search: '', protocol: 'https:', host: 'hub.example' };
globalThis.localStorage = {
  getItem: (k) => (store.has(k) ? store.get(k) : null),
  setItem: (k, v) => store.set(k, String(v)),
  removeItem: (k) => store.delete(k),
};
globalThis.document = Object.assign(target('document'), { visibilityState: 'visible' });
globalThis.window = target('window');
const fire = (kind, type) => { for (const fn of handlers[kind].get(type) || []) fn({}); };
// In steps: a timer armed by a timer that just fired must get its turn.
const tick = (ms) => { for (let left = ms; left > 0; left -= 250) mock.timers.tick(Math.min(250, left)); };
const settle = () => new Promise((resolve) => setImmediate(resolve));

const { bus, connect, call, on } = await import(
  pathToFileURL(path.join(ROOT, 'services/hub-rs/assets/web/m-next/js/bus.js')).href
);
let opens = 0;
on('open', () => { opens++; });

const failures = [];
const check = (name, ok, detail = '') => { if (!ok) failures.push(name + (detail ? ': ' + detail : '')); };
/** A socket the client gave up on mid-handshake is left to open and close. */
const pendingDials = () => sockets.filter((s) => s.readyState === 0 && s.onclose);

// 1. A stalled handshake is raced, and the first to open wins.
connect();
check('dials once at first', pendingDials().length === 1, String(pendingDials().length));
tick(1499);
check('no spare before the race delay', pendingDials().length === 1);
tick(1);
check('a second handshake joins the stalled one', pendingDials().length === 2, String(pendingDials().length));
tick(3000);
check('and a third', pendingDials().length === 3, String(pendingDials().length));
tick(60_000);
const [first, second, third] = sockets;
check('the stalled handshakes were let go after their budget, and retried', sockets.length > 3 && !first.onclose);
// Whatever is dialling now, the newest opens first.
const winner = pendingDials().at(-1);
const losers = pendingDials().filter((s) => s !== winner);
winner.open();
check('connected on the first to open', bus.connected === true && opens === 1);
check('it subscribed', winner.sent.some((f) => f.op === 'subscribe'));
check('the losers are released, not adopted', losers.every((s) => !s.onclose && !s.onmessage));
for (const s of losers) { s.open(); }
check('a late loser closes itself and changes nothing', losers.every((s) => s.readyState === 3) && opens === 1 && bus.connected);
void second; void third;

// 2. Reopening the app: a quiet socket that answers the probe is kept.
tick(60_000);                       // asleep; nothing heard for a minute
let before = sockets.length;
let probes = winner.sent.filter((f) => f.op === 'subscribe').length;
fire('document', 'visibilitychange');
fire('window', 'focus');            // these arrive in bursts
fire('window', 'pageshow');
check('the quiet socket was probed once', winner.sent.filter((f) => f.op === 'subscribe').length === probes + 1);
check('a spare handshake was dialled beside it (one, not one per event)', sockets.length === before + 1, String(sockets.length - before));
check('still shown as connected while proving it', bus.connected === true);
const spare = sockets.at(-1);
tick(RTT);
check('the answer keeps the socket and drops the spare', !spare.onclose && bus.connected && opens === 1);
spare.open();
check('the unneeded spare closes itself', spare.readyState === 3 && opens === 1);
tick(10_000);
check('no reconnect followed', bus.connected === true && pendingDials().length === 0);

// A socket heard from moments ago is not probed at all.
winner.frame({ op: 'event', event: { type: 'x' } });
before = sockets.length; probes = winner.sent.filter((f) => f.op === 'subscribe').length;
fire('window', 'focus');
check('a fresh socket is trusted', sockets.length === before && winner.sent.filter((f) => f.op === 'subscribe').length === probes);

// 3. A zombie: the probe goes unanswered, the spare replaces it, calls fail now.
tick(60_000);
check('a quiet but live socket survives a minute in the foreground', bus.connected === true && opens === 1 && pendingDials().length === 0);
winner.zombie = true;
const stuck = call('sessions.snapshots', {}).then(() => 'resolved', (e) => e.message);
fire('document', 'visibilitychange');
const replacement = sockets.at(-1);
check('a spare is dialled beside the zombie', replacement !== winner && replacement.readyState === 0);
replacement.open();
check('the spare took over without a disconnected flash', bus.connected === true && opens === 2);
check('the zombie was closed', winner.readyState === 3);
await settle();
check('the call on the zombie failed at once, not at its timeout', (await Promise.race([stuck, Promise.resolve('still waiting')])) === 'offline');
const fresh = call('sessions.snapshots', {});
const sentCall = replacement.sent.find((f) => f.op === 'call');
check('new calls go to the new socket', !!sentCall);
replacement.frame({ op: 'result', id: sentCall.id, result: [1] });
check('and resolve', JSON.stringify(await fresh) === '[1]');

// A zombie with no spare in time: shown as reconnecting after the probe window.
tick(10_000);
replacement.zombie = true;
fire('document', 'visibilitychange');
check('connected while the probe is out', bus.connected === true);
tick(2500);
check('an unanswered probe with no spare yet reads as disconnected', bus.connected === false);
check('and the handshake race carries on', pendingDials().length >= 2, String(pendingDials().length));
pendingDials()[0].open();
check('until one opens', bus.connected === true && opens === 3);

// 4. A refused connection backs off rather than hammering.
const current = sockets.find((s) => s.readyState === 1 && s.onclose);
before = sockets.length;
current.drop();
check('a dropped socket reads as disconnected', bus.connected === false);
tick(500);
check('one retry after the first backoff', sockets.length === before + 1);
sockets.at(-1).drop();
tick(999);
check('the next waits longer', sockets.length === before + 1);
tick(1);
check('then retries', sockets.length === before + 2);
sockets.at(-1).drop();
tick(1999);
check('and longer again', sockets.length === before + 2);
// Reopening the app does not wait out a backoff.
fire('document', 'visibilitychange');
check('foregrounding retries now', sockets.length === before + 3);
sockets.at(-1).open();
check('and recovers', bus.connected === true);

// 5. A machine stop pauses everything, spares included.
tick(60_000);
check('still connected before the stop', bus.connected === true);
sockets.filter((s) => s.readyState === 1 && s.onclose)[0].drop(4001);
check('a machine stop pauses', bus.machinePaused === true && bus.connected === false);
before = sockets.length;
tick(120_000);
fire('document', 'visibilitychange');
check('and nothing reconnects until Wake', sockets.length === before && pendingDials().length === 0);

mock.timers.reset();
if (failures.length) { console.error('test-mobile-next-bus:\n  ' + failures.join('\n  ')); process.exit(1); }
console.log('test-mobile-next-bus: handshake race, resume probe, zombie swap, backoff and pause hold');
