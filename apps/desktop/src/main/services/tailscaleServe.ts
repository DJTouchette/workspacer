// Programmatic Tailscale Serve control, so the Remote Share dialog can put the
// /m PWA behind real HTTPS with one tap — which is what unlocks service workers
// + Web Push (browsers only allow those on a secure origin; a raw http://100.x
// Tailscale IP is encrypted on the wire but is NOT a secure context).
//
// `tailscale serve --bg <port>` fronts the local hub at
// https://<magic-dns-name>/ (WebSocket included). Reads never need privilege;
// writes (enable/disable) do on Linux unless the operator is set — we detect
// that and hand back a copy-paste hint instead of failing opaquely.
//
// Everything shells out to the `tailscale` CLI with a timeout; nothing here
// throws — callers get a typed result.

import { spawn } from 'child_process';

export interface TailscaleInfo {
  /** CLI present, backend Running, and logged in. */
  available: boolean;
  /** MagicDNS name of this node (no trailing dot), e.g. host.tailnet.ts.net. */
  magicName: string | null;
  /** `tailscale serve` currently proxies our hub port. */
  serveActive: boolean;
  /** Writes (enable/disable) are likely to succeed without sudo. */
  canServe: boolean;
  /** Present when !canServe: the one-time fix to run. */
  hint?: string;
}

export interface ServeResult {
  ok: boolean;
  error?: string;
  hint?: string;
}

interface RunResult {
  code: number;
  stdout: string;
  stderr: string;
}

/** Run `tailscale <args>` with a timeout; never rejects. code -1 = spawn/timeout. */
function run(args: string[], timeoutMs = 8000): Promise<RunResult> {
  return new Promise((resolve) => {
    let stdout = '';
    let stderr = '';
    let done = false;
    const finish = (r: RunResult) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      resolve(r);
    };
    let child: ReturnType<typeof spawn>;
    try {
      child = spawn('tailscale', args, { stdio: ['ignore', 'pipe', 'pipe'] });
    } catch (err) {
      resolve({ code: -1, stdout: '', stderr: String((err as Error)?.message ?? err) });
      return;
    }
    const timer = setTimeout(() => {
      try {
        child.kill();
      } catch {
        /* ignore */
      }
      finish({ code: -1, stdout, stderr: stderr || 'timeout' });
    }, timeoutMs);
    child.stdout?.on('data', (d) => (stdout += d));
    child.stderr?.on('data', (d) => (stderr += d));
    child.on('error', (err) => finish({ code: -1, stdout: '', stderr: String(err.message) }));
    child.on('close', (code) => finish({ code: code ?? -1, stdout, stderr }));
  });
}

const OPERATOR_HINT = 'Run once so workspacer can manage Tailscale: sudo tailscale set --operator=$USER';

/** The MagicDNS name of this node, or null if unavailable. Also the availability probe. */
async function readStatus(): Promise<{ running: boolean; magicName: string | null }> {
  const r = await run(['status', '--json']);
  if (r.code !== 0) return { running: false, magicName: null };
  try {
    const j = JSON.parse(r.stdout);
    const running = j?.BackendState === 'Running';
    let name: string | null = j?.Self?.DNSName ?? null;
    if (name) name = name.replace(/\.$/, ''); // strip the trailing dot
    return { running, magicName: name || null };
  } catch {
    return { running: false, magicName: null };
  }
}

/** Whether serve writes are permitted without sudo. */
async function canServe(): Promise<boolean> {
  // macOS/Windows manage the daemon through the GUI app; serve works for the
  // logged-in user. On Linux the local API gates writes on the operator.
  if (process.platform !== 'linux') return true;
  if (typeof process.getuid === 'function' && process.getuid() === 0) return true;
  const r = await run(['debug', 'prefs']);
  if (r.code !== 0) return false;
  try {
    const j = JSON.parse(r.stdout);
    return typeof j?.OperatorUser === 'string' && j.OperatorUser.length > 0;
  } catch {
    return false;
  }
}

/** Whether `tailscale serve` currently proxies the given local port. */
async function serveActiveFor(port: number): Promise<boolean> {
  const r = await run(['serve', 'status', '--json']);
  if (r.code !== 0 || !r.stdout.trim() || r.stdout.trim() === '{}') return false;
  // The serve config nests handlers by host/path; a proxy target shows up as
  // 127.0.0.1:<port> or localhost:<port>. A substring check is version-proof.
  return r.stdout.includes(`127.0.0.1:${port}`) || r.stdout.includes(`localhost:${port}`);
}

/** Gather everything the Remote Share dialog needs to offer HTTPS. */
export async function getTailscaleInfo(port: number): Promise<TailscaleInfo> {
  const { running, magicName } = await readStatus();
  if (!running) {
    return { available: false, magicName: null, serveActive: false, canServe: false };
  }
  const [serveActive, permitted] = await Promise.all([serveActiveFor(port), canServe()]);
  return {
    available: true,
    magicName,
    serveActive,
    canServe: permitted,
    hint: permitted ? undefined : OPERATOR_HINT,
  };
}

/** Enable or disable HTTPS serving of the hub port. Reversible. */
export async function setTailscaleServe(port: number, enable: boolean): Promise<ServeResult> {
  const r = enable
    ? await run(['serve', '--bg', String(port)])
    : // `reset` clears the whole serve config (there's no per-target off in the
      // stable CLI); the dialog warns about this before calling.
      await run(['serve', 'reset']);
  if (r.code === 0) return { ok: true };

  const err = (r.stderr || 'tailscale serve failed').trim();
  const lower = err.toLowerCase();
  // When a tailnet-level feature is off, Tailscale prints a node-specific opt-in
  // link (e.g. https://login.tailscale.com/f/serve?node=…). Pull it out so the
  // hint is one click instead of a hunt through the admin console.
  const optInUrl = err.match(/https:\/\/login\.tailscale\.com\/\S+/)?.[0];
  // Classify the failures worth guiding the user through. Order matters: the
  // "not enabled on your tailnet" messages embed an https:// URL, so they must be
  // matched before the generic https/cert branch, which would otherwise swallow
  // them and point at the wrong admin page.
  if (lower.includes('operator') || lower.includes('access denied') || lower.includes('permission')) {
    return { ok: false, error: err, hint: OPERATOR_HINT };
  }
  if (lower.includes('not enabled')) {
    // Serve (and/or HTTPS certs) must be turned on for the tailnet first — a
    // one-time admin opt-in, not something we can do from here.
    return {
      ok: false,
      error: err,
      hint: optInUrl
        ? `Enable this once for your tailnet: ${optInUrl}`
        : 'Enable HTTPS + Serve for your tailnet in the Tailscale admin console (DNS → HTTPS Certificates, then allow Serve).',
    };
  }
  if (lower.includes('https') || lower.includes('cert')) {
    return {
      ok: false,
      error: err,
      hint: 'Enable HTTPS for your tailnet in the Tailscale admin console (DNS → HTTPS Certificates).',
    };
  }
  return { ok: false, error: err };
}
