#!/usr/bin/env bash
# Two-hubs-on-one-machine federation harness.
#
# Starts a PEER hub on 127.0.0.1:8895 pretending to be your other PC, feeds it
# a couple of synthetic agents (agent.snapshot every 2s, so tombstone/reseed
# behavior is exercisable by killing/restarting this script), and prints the
# peers.json entry that makes your REAL hub (the desktop app's, port 7895)
# federate with it.
#
# Usage:
#   ./scripts/federation-harness.sh            # peer hub named "demo-pc"
#   PEER_NAME=work ./scripts/federation-harness.sh
#
# Then either restart the desktop app after writing peers.json (printed below),
# or run a scratch local hub by hand:
#   go run ./cmd/hub --addr 127.0.0.1:7895 -peer name=demo-pc,url=ws://127.0.0.1:8895/bus
#
# Ctrl-C stops the peer; watch the local fleet tombstone, then restart this
# script and watch it come back.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

PEER_NAME="${PEER_NAME:-demo-pc}"
PEER_ADDR="${PEER_ADDR:-127.0.0.1:8895}"

echo "── federation harness ─────────────────────────────────────────────"
echo "peer hub:   ws://${PEER_ADDR}/bus   (name: ${PEER_NAME})"
echo
echo "to federate your real hub, write this to ~/.config/workspacer/peers.json:"
echo "  [{\"name\": \"${PEER_NAME}\", \"url\": \"ws://${PEER_ADDR}/bus\"}]"
echo "then restart the desktop app (it restarts its hub), or run a scratch hub:"
echo "  env -u WORKSPACER_PARENT_PID go run ./cmd/hub --addr 127.0.0.1:7895 -peer name=${PEER_NAME},url=ws://${PEER_ADDR}/bus"
echo "───────────────────────────────────────────────────────────────────"

go build -o /tmp/wks-fed-hub ./cmd/hub
# env -u: inside a workspacer-launched shell WORKSPACER_PARENT_PID is inherited,
# and the hub would treat OUR backgrounding as parent death and self-exit.
env -u WORKSPACER_PARENT_PID /tmp/wks-fed-hub --addr "${PEER_ADDR}" --layout-file "" &
HUB_PID=$!
trap 'kill "${HUB_PID}" "${FEEDER_PID:-}" 2>/dev/null || true' EXIT
sleep 0.5

# Synthetic fleet: two agents in different states, re-published every 2s the
# way the desktop's hubTelemetry does for real sessions. Node >= 22 (built-in
# WebSocket), same as plugin sidecars.
node - "${PEER_ADDR}" "${PEER_NAME}" <<'EOF' &
const [addr, peer] = process.argv.slice(2);
const ws = new WebSocket(`ws://${addr}/bus`);
const agents = [
  { sessionId: 'fed-demo-1', cwd: `/home/${peer}/projects/api`, provider: 'claude',
    ambientState: 'working', model: 'claude-opus-4-8', name: 'api refactor' },
  { sessionId: 'fed-demo-2', cwd: `/home/${peer}/projects/web`, provider: 'codex',
    ambientState: 'waiting_approval', name: 'web tests',
    pendingApproval: { toolName: 'Bash', toolInput: { command: 'npm test' }, timestamp: Date.now() } },
];
ws.addEventListener('open', () => {
  console.log(`[feeder] publishing ${agents.length} synthetic agents on ${peer}`);
  const tick = () => {
    for (const a of agents) {
      ws.send(JSON.stringify({ op: 'publish', event: {
        type: 'agent.snapshot', source: 'harness',
        data: { ...a, updatedAt: Date.now() },
      }}));
    }
  };
  tick();
  setInterval(tick, 2000);
});
ws.addEventListener('close', () => process.exit(0));
EOF
FEEDER_PID=$!

wait "${HUB_PID}"
