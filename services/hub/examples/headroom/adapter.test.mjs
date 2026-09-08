import { test } from "node:test";
import assert from "node:assert/strict";
import http from "node:http";
import { once } from "node:events";
import { WebSocketServer } from "../../../../apps/desktop/node_modules/ws/wrapper.mjs";
import { prepareLaunch, proxyBaseURL, PREPARE_METHOD } from "./adapter.mjs";
import { startAdapter, hubURL } from "./server.mjs";

test("validates loopback proxy origins", () => {
  assert.equal(proxyBaseURL("http://127.0.0.1:8787/"), "http://127.0.0.1:8787");
  for (const url of [
    "https://example.com",
    "http://user:password@127.0.0.1",
    "http://127.0.0.1/path",
    "http://127.0.0.1?token=x",
  ]) {
    assert.throws(() => proxyBaseURL(url));
  }
});

test("rejects unsupported agents without calling a proxy", async () => {
  await assert.rejects(
    prepareLaunch({ version: 1, agent: "unsupported" }),
    /Claude and Codex launches only/,
  );
});

test("adapter registers, returns launch settings, and reports an unavailable proxy", async (t) => {
  let ready = true;
  const proxy = http.createServer((req, res) => {
    assert.equal(req.url, "/readyz");
    res.writeHead(ready ? 200 : 503).end("{}");
  });
  proxy.listen(0, "127.0.0.1");
  await once(proxy, "listening");
  const hub = new WebSocketServer({ host: "127.0.0.1", port: 0 });
  await once(hub, "listening");
  const connection = once(hub, "connection");
  const proxyURL = `http://127.0.0.1:${proxy.address().port}`;
  const adapter = await startAdapter({
    port: 0,
    busURL: `ws://127.0.0.1:${hub.address().port}`,
    getProxyURL: () => proxyURL,
  });
  t.after(async () => {
    await adapter.close();
    for (const client of hub.clients) client.terminate();
    await new Promise((resolve) => hub.close(resolve));
    proxy.closeAllConnections();
    await new Promise((resolve) => proxy.close(resolve));
  });
  const [socket] = await connection;
  const [raw] = await once(socket, "message");
  assert.deepEqual(JSON.parse(raw.toString()), {
    op: "register",
    methods: [PREPARE_METHOD],
  });
  socket.send(JSON.stringify({ op: "registered", methods: [PREPARE_METHOD] }));
  const call = async (id, agent = "claude") => {
    const response = once(socket, "message");
    socket.send(
      JSON.stringify({
        op: "call",
        method: PREPARE_METHOD,
        id,
        params: {
          version: 1,
          agent,
          provider: { id: "openai" },
          cwd: "/project",
          resume: true,
        },
      }),
    );
    const [reply] = await response;
    return JSON.parse(reply.toString());
  };
  assert.deepEqual(await call("first"), {
    op: "result",
    id: "first",
    result: {
      env: { ANTHROPIC_BASE_URL: proxyURL, ENABLE_TOOL_SEARCH: "true" },
    },
  });
  assert.deepEqual(await call("codex", "codex"), {
    op: "result",
    id: "codex",
    result: {
      env: { OPENAI_BASE_URL: `${proxyURL}/v1` },
      args: ["--config", `openai_base_url="${proxyURL}/v1"`],
    },
  });
  ready = false;
  const failed = await call("second");
  assert.equal(failed.op, "error");
  assert.match(failed.error, /Start your Headroom proxy/);
  const pane = await fetch(`http://127.0.0.1:${adapter.port}/`);
  assert.match(await pane.text(), /Proxy unavailable/);
});

test("Codex custom providers retain their identity and upstream while routing Responses through Headroom", async () => {
  const request = async () => ({ ok: true });
  const patch = await prepareLaunch(
    {
      version: 1,
      agent: "codex",
      provider: { id: "custom.provider", baseUrl: "https://example.com/v1" },
    },
    undefined,
    request,
  );
  assert.deepEqual(patch.env, {
    OPENAI_BASE_URL: "http://127.0.0.1:8787/v1",
    HEADROOM_CODEX_UPSTREAM_BASE_URL: "https://example.com/v1",
  });
  assert.deepEqual(patch.args, [
    "--config",
    'model_providers."custom.provider".base_url="http://127.0.0.1:8787/v1"',
    "--config",
    'model_providers."custom.provider".supports_websockets=true',
    "--config",
    'model_providers."custom.provider".env_http_headers.X-Headroom-Base-Url="HEADROOM_CODEX_UPSTREAM_BASE_URL"',
  ]);
  assert.ok(
    !patch.args.some(
      (a) =>
        a.startsWith("model_provider=") || a.includes("requires_openai_auth"),
    ),
  );
});

test("Codex refuses missing routing, credential URLs, and proxy loops before probing readiness", async () => {
  const request = () => {
    throw new Error("must not call");
  };
  for (const provider of [
    undefined,
    { id: "custom" },
    { id: "custom", baseUrl: "https://secret@example.com" },
    { id: "openai", baseUrl: "http://127.0.0.1:8787/v1" },
  ]) {
    await assert.rejects(
      prepareLaunch(
        { version: 1, agent: "codex", provider },
        undefined,
        request,
      ),
      /routing|upstream|credentials|already routes/,
    );
  }
});

test("uses only the scoped plugin token supplied by the supervisor", () => {
  const saved = process.env.HUB_TOKEN;
  try {
    delete process.env.HUB_TOKEN;
    assert.throws(() => hubURL({}), /scoped hub token/);
    process.env.HUB_TOKEN = "plugin-only";
    assert.equal(new URL(hubURL({})).searchParams.get("token"), "plugin-only");
    assert.throws(() => hubURL({ hubUrl: "ws://example.com/bus" }), /loopback/);
  } finally {
    if (saved === undefined) delete process.env.HUB_TOKEN;
    else process.env.HUB_TOKEN = saved;
  }
});
