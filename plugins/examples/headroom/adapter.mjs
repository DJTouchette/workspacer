export const PREPARE_METHOD = "workspacer.headroom.prepareLaunch";

export function proxyBaseURL(value = "http://127.0.0.1:8787") {
  const url = new URL(value);
  if (
    url.protocol !== "http:" ||
    !["127.0.0.1", "[::1]"].includes(url.hostname) ||
    url.username ||
    url.password ||
    url.search ||
    url.hash ||
    url.pathname !== "/"
  ) {
    throw new Error(
      "Headroom proxyUrl must be a loopback HTTP origin, for example http://127.0.0.1:8787",
    );
  }
  return url.origin;
}

export async function prepareLaunch(context, proxyUrl, request = fetch) {
  if (context?.version !== 1 || !["claude", "codex"].includes(context.agent)) {
    throw new Error(
      "This Headroom adapter supports version 1 Claude and Codex launches only",
    );
  }
  const base = proxyBaseURL(proxyUrl);
  const patch =
    context.agent === "codex"
      ? codexPatch(context.provider, base)
      : { env: { ANTHROPIC_BASE_URL: base, ENABLE_TOOL_SEARCH: "true" } };
  try {
    const response = await request(`${base}/readyz`, {
      signal: AbortSignal.timeout(2500),
      redirect: "error",
    });
    if (!response.ok) throw new Error("not ready");
    await response.body?.cancel();
  } catch {
    throw new Error(
      `Headroom is not ready at ${base}. Start your Headroom proxy and retry`,
    );
  }
  return patch;
}

function codexPatch(provider, base) {
  if (
    !provider ||
    typeof provider.id !== "string" ||
    !provider.id ||
    provider.id.length > 200
  ) {
    throw new Error(
      "Codex launches require resolved provider routing from Workspacer",
    );
  }
  const env = { OPENAI_BASE_URL: `${base}/v1` };
  const args = [];
  const config = (key, value) =>
    args.push("--config", `${key}=${JSON.stringify(value)}`);
  // Preserve the provider id: switching to a synthetic "headroom" provider can
  // change login behavior and hide existing conversations in Codex's picker.
  const segment = /^[A-Za-z0-9_-]+$/.test(provider.id)
    ? provider.id
    : JSON.stringify(provider.id);
  const prefix = `model_providers.${segment}`;
  if (provider.id === "openai") config("openai_base_url", env.OPENAI_BASE_URL);
  else {
    if (!provider.baseUrl)
      throw new Error("Custom Codex providers need an upstream base URL");
    config(`${prefix}.base_url`, env.OPENAI_BASE_URL);
    config(`${prefix}.supports_websockets`, true);
  }
  if (provider.baseUrl) {
    let upstream;
    try {
      upstream = new URL(provider.baseUrl);
    } catch {
      throw new Error("Invalid Codex upstream URL");
    }
    if (
      !["http:", "https:"].includes(upstream.protocol) ||
      upstream.username ||
      upstream.password ||
      upstream.search ||
      upstream.hash
    ) {
      throw new Error(
        "Codex upstream URL must not contain credentials, query, or fragment",
      );
    }
    if (upstream.origin === base)
      throw new Error(
        "Codex already routes through this Headroom proxy; remove that override before selecting the integration",
      );
    env.HEADROOM_CODEX_UPSTREAM_BASE_URL = provider.baseUrl.replace(/\/$/, "");
    config(
      `${prefix}.env_http_headers.X-Headroom-Base-Url`,
      "HEADROOM_CODEX_UPSTREAM_BASE_URL",
    );
  }
  return { env, args };
}
