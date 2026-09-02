#!/usr/bin/env node
/**
 * Fake claudemon for limit-aware routing harnesses.
 *
 * Serves the real claudemon GET /usage/report wire shape with selectable
 * scenarios. The shape follows services/claudemon/src/session/usage_report.rs.
 * The stale Codex window is the verbatim capture pinned in
 * services/claudemon/src/providers/codex_usage.rs:
 * used_percent 67.0, resets_at 1787953526, window_minutes 300.
 *
 * Standalone usage:
 *   node services/hub/scripts/fake-claudemon.mjs --port 18891
 *
 * Scenario controls:
 *   GET  /usage/report
 *   GET  /usage/script
 *   POST /usage/script         {"scenario":"stale-codex","clearRequests":true}
 *   POST /usage/script         {"script":["healthy-current","reset-now"],"index":0}
 *   POST /usage/advance
 *   GET  /usage/requests
 *   POST /usage/requests/clear
 *
 * Live-availability controls (the model catalog the hub probes per provider):
 *   GET  /providers/:provider/models   200 {models:[]} once marked unavailable,
 *                                      404 otherwise — which is what keeps every
 *                                      other provider UNKNOWN and therefore
 *                                      routable, exactly as it was before the
 *                                      availability feature existed.
 *   POST /providers/availability       {"provider":"codex","available":false}
 *   GET  /providers/probes             {count, probes} — so a harness can wait
 *                                      for the hub to actually re-probe.
 */
import http from 'node:http';

const CODEX_CAPTURE = Object.freeze({
  observedAt: 1787943726,
  planType: 'team',
  fiveHour: { usedPercent: 67.0, windowMinutes: 300, resetsAt: 1787953526 },
  sevenDay: { usedPercent: 11.0, windowMinutes: 10080, resetsAt: 1788492430 },
});

const COPILOT_403 =
  'GitHub exposes no local quota record, and copilot_internal/v2/token answers ' +
  '403 to a gh OAuth token (probed live against copilot CLI v1.0.81)';

// The Anthropic window lengths the real daemon now stamps on every OAuth
// reading (services/claudemon/src/session/usage_report.rs). The endpoint never
// states them; the window names assert them, and without them a Claude window
// could report a utilization and a reset but never how far THROUGH the window
// that utilization was — which is the one term pacing needs.
const CLAUDE_FIVE_HOUR_MINUTES = 300;
const CLAUDE_SEVEN_DAY_MINUTES = 10080;

const SCENARIOS = Object.freeze([
  'healthy-current',
  'stale-codex',
  'reset-now',
  'copilot-403',
  'absent-opencode-pi',
  // Pacing scenarios. Both are CURRENT windows with ordinary health — the whole
  // point is that the used-percentage ladder cannot tell them apart from
  // 'healthy-current' and the clock can.
  'claude-overpace',
  'claude-pace-blocks-spend-down',
  // A Codex five-hour window past the shipped 90% red band. The captured
  // stale-codex row cannot serve for this: it is deliberately NON-CURRENT, so
  // the currency guard makes it UNKNOWN rather than red, and a fallover case
  // needs a primary that is genuinely, currently out of allowance.
  'codex-red',
]);

const args = process.argv.slice(2);
const opt = (name, fallback = '') => {
  const i = args.indexOf(name);
  return i >= 0 && i + 1 < args.length ? args[i + 1] : fallback;
};

const host = opt('--host', '127.0.0.1');
const port = Number(opt('--port', '18891'));
const initialScenario = opt('--scenario', 'healthy-current');

if (!SCENARIOS.includes(initialScenario)) {
  console.error(`unknown scenario ${initialScenario}; choose one of ${SCENARIOS.join(', ')}`);
  process.exit(2);
}

const state = {
  scenario: initialScenario,
  script: [],
  index: 0,
  requests: [],
  // Providers this fake reports as having NO launchable model — the live
  // "the CLI is not installed" answer. Everything not named here 404s, which the
  // hub reads as "could not ask" and routes around exactly as it always did.
  unavailable: new Set(),
  modelProbes: [],
};

const epoch = () => Math.floor(Date.now() / 1000);
const currentScenario = () => state.script[state.index] ?? state.scenario;

const ok = (value) => ({ state: 'ok', value });
const unavailable = (reason) => ({ state: 'unavailable', reason });

const missingWindow = (usedPercent) => ({
  used_percent: usedPercent,
  resets_at: null,
  window_minutes: null,
  is_current: null,
});

const windowReport = ({ usedPercent, resetsAt, windowMinutes }, now) => ({
  used_percent: ok(usedPercent),
  resets_at: resetsAt ?? null,
  window_minutes: windowMinutes ?? null,
  is_current: resetsAt == null ? null : resetsAt > now,
});

const spend = (basis = 'none', cost = unavailable('no metered spend for this fake account'), nano = unavailable('AIU not reported for this provider')) => ({
  cost_usd: cost,
  nano_aiu: nano,
  basis,
});

const tokensAll = (m) => ({
  input: m,
  cache_read: m,
  cache_write: m,
  output: m,
  reasoning: m,
  total: m,
});

const noSplitTokens = () =>
  tokensAll(unavailable('this fake account does not report a token split'));

const zeroTokens = () => ({
  input: ok(0),
  cache_read: ok(0),
  cache_write: ok(0),
  output: ok(0),
  reasoning: ok(0),
  total: ok(0),
});

function account({
  account: accountKey,
  label,
  isDefault,
  source,
  observedAt,
  fresh,
  failure = null,
  windows,
  spendReport,
  tokensReport,
  models = [],
  liveSessions = 0,
}) {
  return {
    account: accountKey,
    label,
    is_default: isDefault,
    source,
    observed_at: observedAt ?? null,
    fresh: fresh ?? null,
    failure,
    windows,
    spend: spendReport,
    tokens: tokensReport,
    models,
    live_sessions: liveSessions,
  };
}

function claudeProvider(now, scenario) {
  const noMonthly = unavailable('extra usage (monthly overage) is not enabled on this account');
  // Healthy and ON PACE: 12% of the five-hour window gone with an hour left
  // (80% elapsed), 40% of the week gone three days in.
  let fiveHour = { usedPercent: 12.0, resetsAt: now + 60 * 60, windowMinutes: CLAUDE_FIVE_HOUR_MINUTES };
  let sevenDay = { usedPercent: 40.0, resetsAt: now + 4 * 24 * 60 * 60, windowMinutes: CLAUDE_SEVEN_DAY_MINUTES };

  if (scenario === 'claude-overpace') {
    // 80% of a five-hour window gone with half the window left: YELLOW on the
    // health ladder (the 90% red band is untouched) and 1.6x over the curve.
    fiveHour = { usedPercent: 80.0, resetsAt: now + 150 * 60, windowMinutes: CLAUDE_FIVE_HOUR_MINUTES };
    sevenDay = { usedPercent: 10.0, resetsAt: now + 4 * 24 * 60 * 60, windowMinutes: CLAUDE_SEVEN_DAY_MINUTES };
  } else if (scenario === 'claude-pace-blocks-spend-down') {
    // The five-hour window resets within the spend-down window with 80% of it
    // left — a textbook spend-down — while the WEEK is running 1.15x over the
    // curve, so what is left is already spoken for.
    fiveHour = { usedPercent: 20.0, resetsAt: now + 60 * 60, windowMinutes: CLAUDE_FIVE_HOUR_MINUTES };
    sevenDay = { usedPercent: 60.0, resetsAt: now + 3.5 * 24 * 60 * 60, windowMinutes: CLAUDE_SEVEN_DAY_MINUTES };
  }

  return {
    provider: 'claude',
    accounts: [
      account({
        account: '',
        label: 'default',
        isDefault: true,
        source: 'oauth_poll',
        observedAt: now - 30,
        fresh: true,
        windows: {
          five_hour: windowReport(fiveHour, now),
          seven_day: windowReport(sevenDay, now),
          // No length on the monthly window, ever: a calendar month is not a
          // fixed number of minutes, so it is never paced.
          monthly: missingWindow(noMonthly),
        },
        spendReport: spend('estimated', ok(0)),
        tokensReport: zeroTokens(),
      }),
    ],
    note: null,
  };
}

function codexProvider(now, scenario) {
  let fiveHour = { usedPercent: 12.0, windowMinutes: 300, resetsAt: now + 75 * 60 };
  let sevenDay = { usedPercent: 11.0, windowMinutes: 10080, resetsAt: now + 4 * 24 * 60 * 60 };
  let observedAt = now - 45;
  let label = 'team';

  if (scenario === 'stale-codex') {
    fiveHour = CODEX_CAPTURE.fiveHour;
    sevenDay = CODEX_CAPTURE.sevenDay;
    observedAt = CODEX_CAPTURE.observedAt;
    label = CODEX_CAPTURE.planType;
  } else if (scenario === 'reset-now') {
    fiveHour = { usedPercent: 45.0, windowMinutes: 300, resetsAt: now };
  } else if (scenario === 'codex-red') {
    // Past the shipped red_at_used_pct (90) on a CURRENT window, and far enough
    // from the reset that nothing spends down.
    fiveHour = { usedPercent: 95.0, windowMinutes: 300, resetsAt: now + 4 * 60 * 60 };
    sevenDay = { usedPercent: 11.0, windowMinutes: 10080, resetsAt: now + 4 * 24 * 60 * 60 };
  }

  return {
    provider: 'codex',
    accounts: [
      account({
        account: '/tmp/wks-fake-codex',
        label,
        isDefault: true,
        source: 'disk',
        observedAt,
        fresh: null,
        windows: {
          five_hour: windowReport(fiveHour, now),
          seven_day: windowReport(sevenDay, now),
          monthly: missingWindow(unavailable('Codex publishes no monthly window')),
        },
        spendReport: spend('none', unavailable('Codex records no per-request cost on disk; its plan is a rate limit, not a meter')),
        tokensReport: {
          ...tokensAll(unavailable('state_5.sqlite records one cumulative token count per thread, with no input/output split')),
          total: ok(3886013),
        },
      }),
    ],
    note: 'credits: none',
  };
}

function copilotProvider(now) {
  const quota = () => missingWindow(unavailable(COPILOT_403));
  return {
    provider: 'copilot',
    accounts: [
      account({
        account: '/tmp/wks-fake-copilot/session-store.db',
        label: 'copilot',
        isDefault: true,
        source: 'disk',
        observedAt: now - 300,
        fresh: null,
        windows: {
          five_hour: quota(),
          seven_day: quota(),
          monthly: quota(),
        },
        spendReport: spend('vendor_recorded', ok(0.031033), ok(3103300000)),
        tokensReport: noSplitTokens(),
        models: [
          {
            model: 'gpt-5-mini',
            requests: ok(1),
            tokens: noSplitTokens(),
            spend: spend('vendor_recorded', ok(0.031033), ok(3103300000)),
          },
        ],
      }),
    ],
    note: '1 requests over 1 sessions, recorded by GitHub itself',
  };
}

function usageReportFor(scenario) {
  const now = epoch();
  return {
    generated_at: now,
    providers: [
      claudeProvider(now, scenario),
      codexProvider(now, scenario),
      copilotProvider(now),
      // Deliberately no opencode or pi rows. The real usage_report.rs only
      // emits claude/codex/copilot; opencode and pi are absent, not empty.
    ],
  };
}

function sendJSON(res, status, body) {
  const text = JSON.stringify(body, null, 2);
  res.writeHead(status, {
    'content-type': 'application/json; charset=utf-8',
    'content-length': Buffer.byteLength(text),
  });
  res.end(text);
}

async function readJSON(req) {
  let body = '';
  for await (const chunk of req) body += chunk;
  if (!body.trim()) return {};
  return JSON.parse(body);
}

function validateScenario(name) {
  if (!SCENARIOS.includes(name)) throw new Error(`unknown scenario ${name}`);
}

const server = http.createServer(async (req, res) => {
  try {
    const url = new URL(req.url ?? '/', `http://${req.headers.host ?? '127.0.0.1'}`);

    if (req.method === 'GET' && url.pathname === '/health') {
      res.writeHead(200, { 'content-type': 'text/plain; charset=utf-8' });
      res.end('ok');
      return;
    }

    if (req.method === 'GET' && url.pathname === '/usage/report') {
      const scenario = currentScenario();
      const body = usageReportFor(scenario);
      state.requests.push({
        at: epoch(),
        scenario,
        generated_at: body.generated_at,
        user_agent: req.headers['user-agent'] ?? '',
      });
      if (state.requests.length > 1000) state.requests.shift();
      sendJSON(res, 200, body);
      return;
    }

    if (req.method === 'GET' && url.pathname === '/usage/script') {
      sendJSON(res, 200, {
        scenario: currentScenario(),
        baseScenario: state.scenario,
        script: state.script,
        index: state.index,
        scenarios: SCENARIOS,
        requests: state.requests.length,
      });
      return;
    }

    if (req.method === 'POST' && url.pathname === '/usage/script') {
      const body = await readJSON(req);
      if (Array.isArray(body.script)) {
        if (body.script.length === 0) throw new Error('script must not be empty');
        for (const s of body.script) validateScenario(s);
        state.script = body.script.slice();
        state.index = Math.max(0, Math.min(Number(body.index ?? 0), state.script.length - 1));
        state.scenario = state.script[state.index];
      } else if (typeof body.scenario === 'string') {
        validateScenario(body.scenario);
        state.script = [];
        state.index = 0;
        state.scenario = body.scenario;
      } else {
        throw new Error('body must include scenario or script');
      }
      if (body.clearRequests) state.requests = [];
      sendJSON(res, 200, { ok: true, scenario: currentScenario(), script: state.script, index: state.index });
      return;
    }

    if (req.method === 'POST' && url.pathname === '/usage/advance') {
      if (state.script.length === 0) throw new Error('no script is active');
      state.index = Math.min(state.index + 1, state.script.length - 1);
      state.scenario = currentScenario();
      sendJSON(res, 200, { ok: true, scenario: currentScenario(), index: state.index });
      return;
    }

    if (req.method === 'GET' && url.pathname === '/usage/requests') {
      sendJSON(res, 200, { count: state.requests.length, requests: state.requests });
      return;
    }

    if (req.method === 'POST' && url.pathname === '/usage/requests/clear') {
      state.requests = [];
      sendJSON(res, 200, { ok: true });
      return;
    }

    // THE MODEL CATALOG, which is where live provider availability comes from.
    const models = url.pathname.match(/^\/providers\/([^/]+)\/models$/);
    if (req.method === 'GET' && models) {
      const provider = decodeURIComponent(models[1]);
      state.modelProbes.push({ at: epoch(), provider });
      if (state.modelProbes.length > 1000) state.modelProbes.shift();
      if (state.unavailable.has(provider)) {
        // A 200 carrying an empty list is the PROVIDER's own answer: it booted
        // (or failed to) and can launch nothing. That is the one shape the hub
        // reads as unavailable.
        sendJSON(res, 200, { models: [] });
        return;
      }
      // Anything else is "this fake does not model that catalog", which must
      // stay UNKNOWN rather than becoming a routing refusal.
      sendJSON(res, 404, { error: `no catalog for ${provider} in this fake` });
      return;
    }

    if (req.method === 'POST' && url.pathname === '/providers/availability') {
      const body = await readJSON(req);
      const provider = String(body.provider ?? '').trim();
      if (!provider) throw new Error('body must include provider');
      if (body.available === false) state.unavailable.add(provider);
      else state.unavailable.delete(provider);
      sendJSON(res, 200, { ok: true, unavailable: [...state.unavailable] });
      return;
    }

    if (req.method === 'GET' && url.pathname === '/providers/probes') {
      sendJSON(res, 200, { count: state.modelProbes.length, probes: state.modelProbes, unavailable: [...state.unavailable] });
      return;
    }

    sendJSON(res, 404, { error: `no fake claudemon route ${req.method} ${url.pathname}` });
  } catch (err) {
    sendJSON(res, 400, { error: String(err?.message ?? err) });
  }
});

server.listen(port, host, () => {
  const addr = server.address();
  const actualHost = typeof addr === 'object' && addr ? addr.address : host;
  const actualPort = typeof addr === 'object' && addr ? addr.port : port;
  console.log(JSON.stringify({
    type: 'listening',
    url: `http://${actualHost}:${actualPort}`,
    scenario: currentScenario(),
  }));
});

for (const sig of ['SIGINT', 'SIGTERM']) {
  process.on(sig, () => server.close(() => process.exit(0)));
}
