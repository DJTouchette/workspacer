import http from "node:http";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { PREPARE_METHOD, prepareLaunch, proxyBaseURL } from "./adapter.mjs";

function readConfig() {
  return JSON.parse(process.env.WKS_SETTINGS || "{}");
}

export function hubURL(config) {
  const url = new URL(config.hubUrl || "ws://127.0.0.1:7895/bus");
  if (
    url.protocol !== "ws:" ||
    !["127.0.0.1", "[::1]"].includes(url.hostname) ||
    url.username ||
    url.password
  ) {
    throw new Error("Headroom adapter requires a loopback hub WebSocket URL");
  }
  if (!process.env.HUB_TOKEN)
    throw new Error(
      "Start this plugin through Workspacer to receive its scoped hub token",
    );
  url.searchParams.set("token", process.env.HUB_TOKEN);
  return url.toString();
}

/** The hub supervises this adapter; the user's Headroom proxy has its own lifecycle. */
export async function startAdapter({
  port = Number(process.env.WORKSPACER_HEADROOM_PORT || 9130),
  busURL,
  getProxyURL = () => proxyBaseURL(readConfig().proxyUrl),
} = {}) {
  const target = busURL || hubURL(readConfig());
  let socket;
  let stopped = false;
  let registered = false;
  let reconnect;
  let delay = 250;
  const connect = () => {
    if (stopped) return;
    const ws = new WebSocket(target);
    socket = ws;
    ws.addEventListener("open", () => {
      delay = 250;
      ws.send(JSON.stringify({ op: "register", methods: [PREPARE_METHOD] }));
    });
    ws.addEventListener("message", async (event) => {
      let frame;
      try {
        frame = JSON.parse(String(event.data));
      } catch {
        return;
      }
      if (frame.op === "registered") {
        registered = frame.methods?.includes(PREPARE_METHOD) === true;
        return;
      }
      if (frame.op !== "call" || typeof frame.id !== "string") return;
      let reply;
      try {
        if (frame.method !== PREPARE_METHOD)
          throw new Error("Unknown capability");
        reply = {
          op: "result",
          id: frame.id,
          result: await prepareLaunch(frame.params, getProxyURL()),
        };
      } catch (error) {
        reply = {
          op: "error",
          id: frame.id,
          error:
            error instanceof Error
              ? error.message
              : "Headroom preparation failed",
        };
      }
      if (ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(reply));
    });
    ws.addEventListener("error", () => ws.close());
    ws.addEventListener("close", () => {
      registered = false;
      if (!stopped) {
        reconnect = setTimeout(connect, delay);
        delay = Math.min(delay * 2, 5000);
      }
    });
  };
  const server = http.createServer(async (req, res) => {
    if (req.method !== "GET") {
      res.writeHead(405).end();
      return;
    }
    if (req.url === "/health") {
      res.writeHead(registered ? 200 : 503, {
        "Content-Type": "application/json",
      });
      res.end(JSON.stringify({ connected: registered }));
      return;
    }
    if (req.url !== "/") {
      res.writeHead(404).end();
      return;
    }
    try {
      const proxy = getProxyURL();
      let status = "Ready";
      try {
        await prepareLaunch({ version: 1, agent: "claude" }, proxy);
      } catch {
        status = "Proxy unavailable. Start Headroom and reload this pane.";
      }
      res.writeHead(200, {
        "Content-Type": "text/html; charset=utf-8",
        "Content-Security-Policy":
          "default-src 'none'; style-src 'unsafe-inline'",
      });
      res.end(
        `<!doctype html><meta charset="utf-8"><title>Headroom</title><style>body{font:16px system-ui;padding:24px;background:#181818;color:#eee}a{color:#9bd}</style><h1>Headroom</h1><p>${status}</p><p>Workspacer connection: ${registered ? "ready" : "reconnecting"}</p><p><a href="${proxy}/dashboard">Open Headroom dashboard</a></p><p>Choose Headroom under Launch integration in the Spawn agent dialog for Claude Code or Codex.</p>`,
      );
    } catch {
      res
        .writeHead(500, { "Content-Type": "text/plain" })
        .end(
          "Invalid Headroom adapter configuration. Check the plugin settings.",
        );
    }
  });
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, "127.0.0.1", resolve);
  });
  connect();
  return {
    port: server.address().port,
    async close() {
      stopped = true;
      clearTimeout(reconnect);
      socket?.close();
      server.closeAllConnections();
      await new Promise((resolve) => server.close(resolve));
    },
  };
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href
) {
  const adapter = await startAdapter();
  for (const signal of ["SIGTERM", "SIGINT"])
    process.once(signal, () => {
      void adapter.close();
    });
}
