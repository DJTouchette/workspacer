#!/usr/bin/env bash
# Real Rust role images and fresh local volumes; only Tailscale is simulated.
# No Fly command, provider inference, existing account or existing volume access.
set -euo pipefail
umask 077
hub_image=${WKS_RUST_HUB_IMAGE:-workspacer-rust-hub:preview}
node_image=${WKS_RUST_NODE_IMAGE:-workspacer-rust-node:preview}
combined_image=${WKS_RUST_COMBINED_IMAGE:-workspacer-rust-combined:preview}
[ "$(docker image inspect --format '{{index .Config.Labels "dev.workspacer.node.role"}}' "$hub_image")" = rust-hub ]
[ "$(docker image inspect --format '{{index .Config.Labels "dev.workspacer.node.role"}}' "$node_image")" = rust-node ]
[ "$(docker image inspect --format '{{index .Config.Labels "dev.workspacer.node.role"}}' "$combined_image")" = rust-combined ]
fixture=$(mktemp -d)
suffix=${fixture##*/}
hub="wks-rust-hub-test-$suffix"
worker="wks-rust-worker-test-$suffix"
combined="wks-rust-combined-test-$suffix"
hub_volume="${hub}-data"
worker_volume="${worker}-data"
combined_volume="${combined}-data"
cleanup() { docker rm -f "$combined" "$worker" "$hub" >/dev/null 2>&1 || true; docker volume rm "$combined_volume" "$worker_volume" "$hub_volume" >/dev/null 2>&1 || true; rm -rf "$fixture"; }
trap cleanup EXIT
mkdir "$fixture/mock"
for volume in "$hub_volume" "$worker_volume" "$combined_volume"; do docker volume create "$volume" >/dev/null; done
cat >"$fixture/mock/tailscale" <<'MOCK'
#!/bin/sh
case "$*" in
  *status*--json*) printf '%s\n' '{"BackendState":"Running","Self":{"DNSName":"rust-hub.fixture.ts.net.","ID":"fixture"}}' ;;
  *ip*-4*) printf '%s\n' '100.64.0.10' ;;
  *) exit 0 ;;
esac
MOCK
cat >"$fixture/mock/tailscaled" <<'MOCK'
#!/bin/sh
trap 'exit 0' INT TERM
while :; do sleep 1; done
MOCK
chmod 755 "$fixture/mock/"*
docker run -d --name "$hub" -v "$hub_volume:/data" \
  -v "$fixture/mock/tailscale:/usr/local/bin/tailscale:ro" -v "$fixture/mock/tailscaled:/usr/local/bin/tailscaled:ro" \
  -e WKS_HUB_SERVE_ENABLED=0 -e WKS_HUB_SERVE_PROBE_ENABLED=0 -e WKS_TAILNET_WAIT_SECS=3 "$hub_image" >/dev/null
ready() {
  local container=$1 url=$2
  for _ in $(seq 1 150); do
    if docker exec "$container" curl -fsS "$url" >/dev/null 2>&1; then return 0; fi
    [ "$(docker inspect --format '{{.State.Running}}' "$container")" = true ] || { docker logs "$container" >&2; return 1; }
    sleep 0.2
  done
  return 1
}
ready "$hub" http://127.0.0.1:7895/health
provider=$(docker exec --user 10001 "$hub" workspacer-rust --config-dir /data/home/.config/workspacer token create --scope provider --label boot-worker-provider)
docker exec --user 10001 "$hub" /usr/local/lib/wks-rust/provision-worker-caller.sh \
  /data/home/.config/workspacer/tokens.json boot-worker /data/boot-worker-caller >/dev/null
caller=$(docker exec "$hub" cat /data/boot-worker-caller)
printf 'HUB_TOKEN=%s\nWKS_MCP_HUB_TOKEN=%s\n' "$provider" "$caller" >"$fixture/worker.env"
unset provider caller
docker run -d --name "$worker" --network "container:$hub" -v "$worker_volume:/data" \
  -v "$fixture/mock/tailscale:/usr/local/bin/tailscale:ro" -v "$fixture/mock/tailscaled:/usr/local/bin/tailscaled:ro" \
  --env-file "$fixture/worker.env" -e HUB_BUS_URL=ws://127.0.0.1:7895/bus -e WKS_WORKER_HUB_PORT=7896 \
  -e WKS_NODE_ID=boot-worker -e WORKSPACER_USAGE_POLL_ON_BOOT=0 -e WKS_TAILNET_WAIT_SECS=3 "$node_image" >/dev/null
ready "$worker" http://127.0.0.1:7891/sessions
ready "$worker" http://127.0.0.1:7897/health
# The probe runs inside the disposable hub and never prints a credential.
docker exec -i "$hub" node <<'JS'
const fs=require('fs');
const token=fs.readFileSync('/data/home/.config/workspacer/remote-token','utf8').trim();
const deadline=setTimeout(()=>{console.error('provider relay did not become ready');process.exit(1)},20000);
let finished=false;
function probe(){
  const ws=new WebSocket('ws://127.0.0.1:7895/bus?token='+encodeURIComponent(token));
  ws.onmessage=event=>{
    const msg=JSON.parse(event.data);
    if(msg.op==='hello')ws.send(JSON.stringify({op:'call',id:'probe',method:'brain.info',params:{}}));
    if(msg.id==='probe'){
      if(msg.op==='result'&&msg.result?.runtime==='rust'&&msg.result?.node==='boot-worker'){
        finished=true;clearTimeout(deadline);ws.close();console.log('Rust provider relay registered with the separate hub');
      }else ws.close();
    }
  };
  ws.onclose=()=>{if(!finished)setTimeout(probe,100)};
  ws.onerror=()=>ws.close();
}
probe();
JS
docker stop --time 60 "$worker" >/dev/null
docker stop --time 30 "$hub" >/dev/null
[ "$(docker inspect --format '{{.State.ExitCode}}' "$worker")" = 0 ]
[ "$(docker inspect --format '{{.State.ExitCode}}' "$hub")" = 0 ]
docker run -d --name "$combined" -v "$combined_volume:/data" \
  -e FLY_APP_NAME=fixture -e WORKSPACER_USAGE_POLL_ON_BOOT=0 "$combined_image" >/dev/null
ready "$combined" http://127.0.0.1:7895/health
ready "$combined" http://127.0.0.1:7891/sessions
ready "$combined" http://127.0.0.1:7897/health
docker stop --time 60 "$combined" >/dev/null
[ "$(docker inspect --format '{{.State.ExitCode}}' "$combined")" = 0 ]
printf '%s\n' 'PASS: real role entrypoints, owned Rust engine/MCP/relay, separate credentials, generic combined startup and graceful stop (Tailscale simulated)'
