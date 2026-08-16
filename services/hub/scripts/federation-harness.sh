#!/usr/bin/env bash
# Two-hubs-on-one-machine federation harness.
#
# Starts a PEER hub on 127.0.0.1:8895 pretending to be your other PC, with a
# synthetic fleet that is both PUBLISHED (agent.snapshot every 2s) and
# ANSWERABLE: the feeder registers the capability methods a federated client
# actually calls — agents.list, sessions.snapshots, sessions.snapshot,
# sessions.conversation, agents.sendMessage, claude.approve, claude.signal —
# so qualified `hub:<peer>/<method>` calls, seeding, remote chat, and the
# cross-hub facade tools can all be exercised end to end. Prints the
# peers.json entry that makes your REAL hub federate with it.
#
# Usage:
#   ./scripts/federation-harness.sh                  # peer named "demo-pc"
#   PEER_NAME=work ./scripts/federation-harness.sh
#   SPARSE=1 ./scripts/federation-harness.sh         # emulate a headless-brain
#                                                    # peer: sparse:true rows only
#
# Then either write peers.json (printed below) and restart the desktop app, or
# run a scratch local hub by hand (NOT on 7895 if the desktop app is running):
#   env -u WORKSPACER_PARENT_PID go run ./cmd/hub --addr 127.0.0.1:9895 \
#     -peer name=demo-pc,url=ws://127.0.0.1:8895/bus
#
# Ctrl-C stops the peer; watch the local fleet tombstone ("hub offline"), then
# restart this script and watch it reseed.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

PEER_NAME="${PEER_NAME:-demo-pc}"
PEER_ADDR="${PEER_ADDR:-127.0.0.1:8895}"
SPARSE="${SPARSE:-0}"

echo "── federation harness ─────────────────────────────────────────────"
echo "peer hub:   ws://${PEER_ADDR}/bus   (name: ${PEER_NAME}, sparse=${SPARSE})"
echo
echo "to federate your real hub, write this to ~/.config/workspacer/peers.json:"
echo "  [{\"name\": \"${PEER_NAME}\", \"url\": \"ws://${PEER_ADDR}/bus\"}]"
echo "then restart the desktop app (it restarts its hub), or run a scratch hub:"
echo "  env -u WORKSPACER_PARENT_PID go run ./cmd/hub --addr 127.0.0.1:9895 -peer name=${PEER_NAME},url=ws://${PEER_ADDR}/bus"
echo "───────────────────────────────────────────────────────────────────"

go build -o /tmp/wks-fed-hub ./cmd/hub
# env -u: inside a workspacer-launched shell WORKSPACER_PARENT_PID is inherited,
# and the hub would treat OUR backgrounding as parent death and self-exit.
env -u WORKSPACER_PARENT_PID /tmp/wks-fed-hub --addr "${PEER_ADDR}" --layout-file "" &
HUB_PID=$!
trap 'kill "${HUB_PID}" "${FEEDER_PID:-}" 2>/dev/null || true' EXIT
sleep 0.5

# Synthetic fleet: publisher + capability provider in one client, the way the
# desktop's hubTelemetry + hubCapabilities pair up for real sessions. Node >= 22
# (built-in WebSocket), same as plugin sidecars.
node - "${PEER_ADDR}" "${PEER_NAME}" "${SPARSE}" <<'EOF' &
const [addr, peer, sparseFlag] = process.argv.slice(2);
const sparse = sparseFlag === '1';
const ws = new WebSocket(`ws://${addr}/bus`);

// The fleet. fed-demo-2 sits on a pending approval so approve flows are
// exercisable; conversations grow when you sendMessage at them.
const conv = new Map(); // sessionId -> { seq, items }
const agents = [
  { sessionId: 'fed-demo-1', cwd: `/home/${peer}/projects/api`, provider: 'claude',
    ambientState: 'working', mode: 'responding', model: 'claude-opus-4-8', name: 'api refactor' },
  { sessionId: 'fed-demo-2', cwd: `/home/${peer}/projects/web`, provider: 'codex',
    ambientState: 'waiting_approval', mode: 'approval', name: 'web tests',
    pendingApproval: { toolName: 'Bash', toolInput: { command: 'npm test' }, timestamp: Date.now() } },
];
for (const a of agents) {
  conv.set(a.sessionId, { seq: 2, items: [
    { seq: 1, role: 'user', kind: 'message', text: `work on ${a.name}` },
    { seq: 2, role: 'assistant', kind: 'message', text: `(${peer}) on it — this conversation came over a federated hub:${peer}/sessions.conversation call.` },
  ]});
}

const snapRow = (a) => sparse
  ? { sessionId: a.sessionId, cwd: a.cwd, provider: a.provider, mode: a.mode,
      ambientState: a.ambientState, sparse: true,
      ...(a.pendingApproval && { pendingApproval: a.pendingApproval }) }
  : { ...a, status: 'active', conversation: [], updatedAt: Date.now() };

const reply = (id, result) => ws.send(JSON.stringify({ op: 'result', id, result }));
const fail  = (id, error)  => ws.send(JSON.stringify({ op: 'error', id, error }));

ws.addEventListener('open', () => {
  console.log(`[feeder] ${peer}: publishing ${agents.length} agents (sparse=${sparse}) + answering capability calls`);
  ws.send(JSON.stringify({ op: 'register', methods: [
    'agents.list', 'sessions.snapshots', 'sessions.snapshot',
    'sessions.conversation', 'agents.sendMessage', 'claude.approve', 'claude.signal',
  ]}));
  const tick = () => {
    for (const a of agents) {
      ws.send(JSON.stringify({ op: 'publish', event: {
        type: 'agent.snapshot', source: 'harness', data: snapRow(a),
      }}));
    }
  };
  tick();
  setInterval(tick, 2000);
});

ws.addEventListener('message', (e) => {
  let f; try { f = JSON.parse(e.data); } catch { return; }
  if (f.op !== 'call') return;
  const p = f.params || {};
  const byId = agents.find((a) => a.sessionId === p.sessionId);
  switch (f.method) {
    case 'agents.list':
      return reply(f.id, agents.map((a) => ({ sessionId: a.sessionId, cwd: a.cwd,
        provider: a.provider, mode: a.mode, ambientState: a.ambientState })));
    case 'sessions.snapshots':
      return reply(f.id, agents.map(snapRow));
    case 'sessions.snapshot':
      return byId ? reply(f.id, snapRow(byId)) : fail(f.id, 'unknown session');
    case 'sessions.conversation': {
      const c = conv.get(p.sessionId);
      if (!c) return fail(f.id, 'unknown session');
      const since = p.sinceSeq ?? 0;
      return reply(f.id, { sessionId: p.sessionId, seq: c.seq,
        items: c.items.filter((it) => it.seq > since) });
    }
    case 'agents.sendMessage': {
      const c = conv.get(p.sessionId);
      if (!c || !byId) return fail(f.id, 'unknown session');
      c.items.push({ seq: ++c.seq, role: 'user', kind: 'message', text: String(p.text ?? '') });
      c.items.push({ seq: ++c.seq, role: 'assistant', kind: 'message',
        text: `(${peer}) ack: "${String(p.text ?? '').slice(0, 60)}"` });
      console.log(`[feeder] sendMessage → ${p.sessionId}: ${p.text}`);
      return reply(f.id, { ok: true });
    }
    case 'claude.approve': {
      if (!byId) return fail(f.id, 'unknown session');
      console.log(`[feeder] approve → ${p.sessionId}: ${p.decision}`);
      delete byId.pendingApproval;
      byId.ambientState = 'working'; byId.mode = 'responding';
      return reply(f.id, { ok: true });
    }
    case 'claude.signal':
      console.log(`[feeder] signal → ${p.sessionId}: ${p.signal}`);
      return reply(f.id, { ok: true });
    default:
      return fail(f.id, 'harness does not answer ' + f.method);
  }
});
ws.addEventListener('close', () => process.exit(0));
EOF
FEEDER_PID=$!

wait "${HUB_PID}"
