/**
 * One question in, one short answer out — without spawning an agent session.
 *
 * The little derived-text chores (title this conversation, summarize this line,
 * classify this status) don't want a session: a session means a PTY, a system
 * prompt, an MCP/skill/memory init, a lifecycle and a row in the sidebar, all to
 * produce eight words. This module is the cheap path for those: one headless
 * turn, bounded output, a hard timeout, and a result the caller can ignore.
 *
 * ## Transport: the harness CLI, not a provider HTTP API
 *
 * The obvious implementation is an HTTPS POST to a provider's completions API.
 * It is rejected here for a concrete, checked reason: **this machine has no API
 * keys.** `ANTHROPIC_API_KEY`/`OPENAI_API_KEY` are unset, `~/.config/workspacer/
 * config.yaml` holds none, and every harness carries its own subscription auth
 * (`codex login status` → "Logged in using ChatGPT"). A primitive that needs a
 * secret the user does not have is not a feature. So each provider is driven
 * through its own CLI's non-interactive mode, which inherits that auth for free.
 *
 * The seam is [`CompletionAdapter`], one per provider, so a direct-HTTP adapter
 * is a new entry in [`ADAPTERS`] the day an API key exists — not a refactor.
 *
 * ## Provider-aware, not claude-shaped
 *
 * The provider is a REQUIRED parameter, never read from a `claude.*` config key.
 * Each adapter owns its own argv, its own model vocabulary and its own default.
 * A model string one provider cannot serve is REFUSED
 * (`reason: 'unsupported-model'`) rather than passed through to a CLI that will
 * reject it in some less legible way — see [`resolveCompletionModel`] for the
 * graceful-downgrade helper consumers should use when they have a configured
 * model that may predate multi-provider support.
 *
 * ## Failure is a value, never an exception
 *
 * The callers are things like brief writes and session titling: work that must
 * complete whether or not a model answered. [`complete`] never throws and never
 * hangs — every outcome is a [`CompletionResult`], and the failure reasons are
 * distinguishable so a caller can log "not logged in" differently from "timed
 * out" while falling back identically.
 */
import { spawn } from 'child_process';
import * as os from 'os';

import { resolveAgentBinary, isAgentBinaryInstalled, type AgentProvider } from './agentProviders';
import { CLAUDEMON_API_URL } from './claudemonDaemon';
import { claudeBaseArgv } from './claudeResolver';
import { configService } from './configService';

export type CompletionProvider = AgentProvider;

/**
 * Why a completion produced nothing. Distinguishable on purpose: every one of
 * these degrades the same way for the caller, but they are very different
 * things to show a user or write to a log.
 */
export type CompletionFailureReason =
  /** No adapter for this provider id (or an id from an older config). */
  | 'unsupported-provider'
  /** The requested model is not in this provider's vocabulary. */
  | 'unsupported-model'
  /** The provider's CLI isn't installed / isn't on PATH. */
  | 'binary-missing'
  /** claudemon isn't running, or predates the route we need. */
  | 'daemon-unavailable'
  /** The harness ran but isn't logged in / has no credentials. */
  | 'not-authed'
  /** Rate-limited, out of quota, or overage-disabled. */
  | 'rate-limited'
  /** Blew the deadline; the child was killed. */
  | 'timeout'
  /** Ran fine and said nothing usable. */
  | 'empty'
  /** Anything else — message carries the tail of stderr. */
  | 'failed';

export interface CompletionOk {
  ok: true;
  /** Raw model output, trimmed only at the ends. Callers own their sanitizing. */
  text: string;
  provider: CompletionProvider;
  /** The model actually used, or null when the CLI picked its own. */
  model: string | null;
  elapsedMs: number;
}

export interface CompletionError {
  ok: false;
  reason: CompletionFailureReason;
  /** Human-readable detail — safe to log, not meant for a user-facing string. */
  message: string;
  provider: CompletionProvider;
  model: string | null;
  elapsedMs: number;
}

export type CompletionResult = CompletionOk | CompletionError;

export interface CompletionRequest {
  /** Which harness answers. Required — there is no implicit claude here. */
  provider: CompletionProvider;
  prompt: string;
  /**
   * Model to use. Omit to take the provider's default (see
   * [`defaultModelFor`]). A string the provider cannot serve fails the call
   * with `unsupported-model` rather than being forwarded.
   */
  model?: string | null;
  /** Whole-call deadline. Clamped to [`MIN_TIMEOUT_MS`, `MAX_TIMEOUT_MS`]. */
  timeoutMs?: number;
  /** Hard cap on captured output; anything past it is dropped. */
  maxOutputChars?: number;
}

/** Prompts here are one entry or one exchange, never a document. */
export const MAX_PROMPT_CHARS = 8_000;
/** Default deadline. A one-shot that hasn't answered by now isn't going to. */
export const DEFAULT_TIMEOUT_MS = 30_000;
export const MIN_TIMEOUT_MS = 2_000;
export const MAX_TIMEOUT_MS = 120_000;
/** An answer is a sentence; past this the model is running away with it. */
export const DEFAULT_MAX_OUTPUT_CHARS = 8_000;

/**
 * A provider's one-shot implementation.
 *
 * `run` may throw — [`complete`] converts anything thrown into a
 * [`CompletionError`]. Throwing a [`CompletionFailure`] picks the reason;
 * anything else lands on `'failed'`.
 */
interface CompletionAdapter {
  readonly provider: CompletionProvider;
  /**
   * Model used when the caller names none. `null` means "let the CLI use the
   * model the user already configured" — deliberately preferred over inventing
   * an id we have not verified against the installed catalog, which is exactly
   * how `supervisor.summarizerModel: 'sonnet'` became unservable under codex.
   */
  readonly defaultModel: string | null;
  /** True when `model` is in this provider's vocabulary. */
  servesModel(model: string): boolean;
  /** One headless turn. Returns raw stdout text. */
  run(ctx: RunContext): Promise<string>;
}

interface RunContext {
  prompt: string;
  /** Already validated against `servesModel`, or null for the CLI's default. */
  model: string | null;
  timeoutMs: number;
  maxOutputChars: number;
}

/** Thrown by adapters to name a specific failure reason. */
class CompletionFailure extends Error {
  constructor(
    readonly reason: CompletionFailureReason,
    message: string,
  ) {
    super(message);
    this.name = 'CompletionFailure';
  }
}

// ---------------------------------------------------------------------------
// Failure classification
// ---------------------------------------------------------------------------

/**
 * Read a failure reason out of a CLI's own words.
 *
 * Every harness reports "you're not logged in" and "you're rate limited"
 * differently, and all we have is the tail of stderr — so this is deliberately
 * pattern-based and deliberately falls back to `'failed'` rather than guessing.
 * Exported because it is the part most worth testing directly.
 */
export function classifyCliFailure(text: string): CompletionFailureReason {
  const t = (text || '').toLowerCase();
  if (!t.trim()) return 'failed';
  if (
    /no api key|not logged in|not authenticated|please (run )?(`?codex )?login|run \/login|use \/login|invalid api key|unauthorized|authentication (failed|required)|\b401\b/.test(
      t,
    )
  ) {
    return 'not-authed';
  }
  if (/rate.?limit|too many requests|\b429\b|quota|usage limit|out of credits|overage/.test(t)) {
    return 'rate-limited';
  }
  if (/unknown model|invalid model|model not found|unsupported model|no such model/.test(t)) {
    return 'unsupported-model';
  }
  if (/command not found|not recognized as an internal|\benoent\b/.test(t)) {
    return 'binary-missing';
  }
  return 'failed';
}

/** Last `n` characters of `text`, whitespace-trimmed — enough to say why. */
function tail(text: string, n = 300): string {
  const t = (text || '').trim();
  return t.length <= n ? t : `…${t.slice(-n)}`;
}

// ---------------------------------------------------------------------------
// Output extraction
// ---------------------------------------------------------------------------

// eslint-disable-next-line no-control-regex
const ANSI = /\[[0-9;?]*[ -/]*[@-~]/g;

/** Drop terminal escapes — the CLIs colour their output even when piped. */
export function stripAnsi(text: string): string {
  return (text || '').replace(ANSI, '');
}

/**
 * The agent's message out of `codex exec --json`'s event stream.
 *
 * Codex emits one JSON object per line; the answer is the last
 * `item.completed` whose item is an `agent_message`. Falls back to the raw text
 * when the stream isn't JSON at all (an older codex, or a hard early error),
 * because a plausible answer beats an empty one.
 */
export function extractCodexText(stdout: string): string {
  const lines = stripAnsi(stdout).split('\n');
  let latest = '';
  let sawJson = false;
  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed.startsWith('{')) continue;
    let event: unknown;
    try {
      event = JSON.parse(trimmed);
    } catch {
      continue;
    }
    sawJson = true;
    const item = (event as { item?: { type?: string; text?: string } }).item;
    if (item?.type === 'agent_message' && typeof item.text === 'string') latest = item.text;
  }
  if (latest.trim()) return latest;
  return sawJson ? '' : stripAnsi(stdout);
}

/**
 * The assistant text out of `opencode run --format json`'s event stream.
 *
 * OpenCode emits JSONL too, but streams the answer as one or more `text` parts
 * that must be concatenated in order — taking only the last one truncates any
 * answer the model split across parts.
 */
export function extractOpencodeText(stdout: string): string {
  const lines = stripAnsi(stdout).split('\n');
  const parts: string[] = [];
  let sawJson = false;
  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed.startsWith('{')) continue;
    let event: unknown;
    try {
      event = JSON.parse(trimmed);
    } catch {
      continue;
    }
    sawJson = true;
    const e = event as { type?: string; part?: { type?: string; text?: string } };
    if (e.type === 'text' && typeof e.part?.text === 'string') parts.push(e.part.text);
  }
  if (parts.join('').trim()) return parts.join('');
  return sawJson ? '' : stripAnsi(stdout);
}

// ---------------------------------------------------------------------------
// Child-process transport (every provider except claude)
// ---------------------------------------------------------------------------

interface CliRun {
  bin: string;
  args: string[];
  /** Written to the child's stdin, which is then closed. Empty = close at once. */
  stdin: string;
  timeoutMs: number;
  maxOutputChars: number;
}

/**
 * Run a CLI to completion and hand back its stdout.
 *
 * Three things here are load-bearing and were learned the hard way in
 * `services/claudemon/src/daemon/oneshot.rs`, which does the same job for
 * claude:
 *
 *  - **stdout and stderr drain concurrently.** Reading stdout to EOF first
 *    deadlocks any child that fills the stderr pipe: it blocks in write(2), so
 *    it never exits, so stdout never reaches EOF, and the call burns its whole
 *    timeout instead of returning the answer the child already produced.
 *  - **stdin is closed immediately.** These CLIs read to EOF; holding the
 *    handle open hangs them for exactly the same reason.
 *  - **cwd is the home directory.** A one-shot has no project to pick up, and
 *    running inside a repo makes some harnesses load project config and skills.
 */
function runCli(run: CliRun): Promise<string> {
  return new Promise((resolve, reject) => {
    let child: ReturnType<typeof spawn>;
    try {
      child = spawn(run.bin, run.args, {
        cwd: os.homedir(),
        stdio: ['pipe', 'pipe', 'pipe'],
        windowsHide: true,
        // Never a shell: the prompt and the model id are untrusted-ish text and
        // a shell would give them grammar. Windows `.cmd` launchers are handled
        // by the caller putting `cmd.exe /c` at the head of argv.
        shell: false,
      });
    } catch (err) {
      reject(new CompletionFailure('binary-missing', (err as Error).message));
      return;
    }

    let out = '';
    let err = '';
    let settled = false;
    const finish = (fn: () => void) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      fn();
    };

    const timer = setTimeout(() => {
      finish(() => {
        child.kill('SIGKILL');
        reject(new CompletionFailure('timeout', `timed out after ${run.timeoutMs}ms`));
      });
    }, run.timeoutMs);

    // Capped independently: a runaway stderr must not be able to push the
    // answer we care about out of memory either.
    child.stdout?.setEncoding('utf8');
    child.stdout?.on('data', (chunk: string) => {
      if (out.length < run.maxOutputChars) out += chunk;
    });
    child.stderr?.setEncoding('utf8');
    child.stderr?.on('data', (chunk: string) => {
      if (err.length < run.maxOutputChars) err += chunk;
    });

    child.on('error', (e: NodeJS.ErrnoException) => {
      finish(() =>
        reject(
          new CompletionFailure(
            e.code === 'ENOENT' ? 'binary-missing' : 'failed',
            `${run.bin}: ${e.message}`,
          ),
        ),
      );
    });

    child.on('close', (code, signal) => {
      finish(() => {
        if (code === 0) {
          resolve(out);
          return;
        }
        const why = signal ? `killed by ${signal}` : `exited ${code}`;
        // Some CLIs report "no api key" on stdout and exit non-zero, so both
        // streams feed the classifier.
        reject(
          new CompletionFailure(
            classifyCliFailure(`${err}\n${out}`),
            `${why}: ${tail(err || out)}`,
          ),
        );
      });
    });

    // Write the prompt and close: `--print`-style modes read to EOF.
    const stdin = child.stdin;
    if (stdin) {
      stdin.on('error', () => {
        /* the child may have exited already; `close` reports the real reason */
      });
      if (run.stdin) stdin.write(run.stdin);
      stdin.end();
    }
  });
}

/**
 * Launcher argv for a provider's CLI, Windows `.cmd` shims included.
 *
 * Node refuses to exec a `.cmd`/`.bat` directly, so those go through
 * `cmd.exe /d /s /c` — the same shape `claudeResolver` already produces for
 * claude, and the reason adapters prefer putting the prompt on stdin.
 */
function launcherArgv(provider: CompletionProvider): string[] {
  const customBin = configService.getConfig().agents?.binaries?.[provider] ?? '';
  const bin = resolveAgentBinary(provider, customBin);
  if (process.platform === 'win32' && /\.(cmd|bat)$/i.test(bin)) {
    return [process.env.ComSpec || 'cmd.exe', '/d', '/s', '/c', bin];
  }
  return [bin];
}

/** Guard the whole class of "the CLI isn't installed" before spawning. */
function assertInstalled(provider: CompletionProvider): void {
  const customBin = configService.getConfig().agents?.binaries?.[provider] ?? '';
  if (!isAgentBinaryInstalled(provider, customBin)) {
    throw new CompletionFailure('binary-missing', `${provider} CLI not found on PATH`);
  }
}

// ---------------------------------------------------------------------------
// Adapters
// ---------------------------------------------------------------------------

/** Claude's model vocabulary: the CLI aliases plus any `claude-*` id. */
const CLAUDE_ALIASES = new Set([
  'default',
  'haiku',
  'sonnet',
  'sonnet[1m]',
  'opus',
  'opusplan',
  'fable',
]);

/**
 * Claude goes through claudemon's `POST /oneshot`, NOT a local `claude
 * --print`.
 *
 * This is not indirection for its own sake: a real headless `claude` fires the
 * user's Claude Code hooks, and the daemon's `SessionStore::ingest` registers a
 * session for every one of them — a ghost row in RECENT per call. The daemon
 * pins and suppresses the run's session id the way keep-warm already does. No
 * other provider has this problem, which is why no other adapter needs the
 * daemon.
 */
const claudeAdapter: CompletionAdapter = {
  provider: 'claude',
  defaultModel: 'haiku',
  servesModel: (model) =>
    CLAUDE_ALIASES.has(model.trim().toLowerCase()) || /^claude-/i.test(model.trim()),
  async run({ prompt, model, timeoutMs }) {
    let res: Response;
    try {
      res = await fetch(`${CLAUDEMON_API_URL}/oneshot`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          argv: claudeBaseArgv(),
          model: model ?? claudeAdapter.defaultModel,
          prompt,
          timeout_secs: Math.max(1, Math.round(timeoutMs / 1000)),
        }),
        // The daemon enforces its own deadline; ours is the outer bound on a
        // daemon that has stopped answering at all.
        signal: AbortSignal.timeout(timeoutMs + 5_000),
      });
    } catch (err) {
      const e = err as Error;
      throw new CompletionFailure(
        e.name === 'TimeoutError' ? 'timeout' : 'daemon-unavailable',
        `claudemon /oneshot: ${e.message}`,
      );
    }
    if (!res.ok) {
      // A daemon predating /oneshot answers 404 — that is "unavailable", not a
      // model failure, and must not be retried by shelling out (see above).
      throw new CompletionFailure(
        'daemon-unavailable',
        `claudemon /oneshot returned ${res.status}`,
      );
    }
    const body = (await res.json().catch(() => null)) as {
      ok?: boolean;
      text?: string;
      error?: string;
    } | null;
    if (!body?.ok) {
      const detail = body?.error ?? 'unknown error';
      throw new CompletionFailure(
        /timed out after/i.test(detail) ? 'timeout' : classifyCliFailure(detail),
        detail,
      );
    }
    return body.text ?? '';
  },
};

/**
 * `codex exec` — codex's own non-interactive mode, verified against the
 * installed CLI (`codex exec --help`).
 *
 * `--ephemeral` keeps it out of the user's session history, `--sandbox
 * read-only` means a stray tool call can't touch the disk, `-` reads the prompt
 * from stdin, and `--json` gives a parseable stream instead of a rendered TUI
 * transcript.
 */
const codexAdapter: CompletionAdapter = {
  provider: 'codex',
  defaultModel: null,
  servesModel: (model) => /^(gpt-|o\d|codex-|gpt\d)/i.test(model.trim()),
  async run({ prompt, model, timeoutMs, maxOutputChars }) {
    assertInstalled('codex');
    const [bin, ...prefix] = launcherArgv('codex');
    const args = [
      ...prefix,
      'exec',
      '--skip-git-repo-check',
      '--ephemeral',
      '--sandbox',
      'read-only',
      '--color',
      'never',
      '--json',
    ];
    if (model) args.push('--model', model);
    args.push('-');
    return extractCodexText(await runCli({ bin, args, stdin: prompt, timeoutMs, maxOutputChars }));
  },
};

/**
 * `opencode run` — verified against the installed CLI (`opencode run --help`).
 *
 * `--pure` skips external plugins (a one-shot has no use for them and they cost
 * startup), and `--format json` gives the answer as `text` parts rather than a
 * decorated terminal frame with a session header in it.
 *
 * The message goes on stdin rather than as the documented positional: verified
 * to work against the installed CLI, and it keeps the prompt out of argv for
 * the same Windows-quoting reason codex and claude do.
 *
 * OpenCode's `--model` is `provider/model`, so its vocabulary is the slash form
 * — a bare `sonnet` is correctly refused before it reaches the CLI.
 */
const opencodeAdapter: CompletionAdapter = {
  provider: 'opencode',
  defaultModel: null,
  servesModel: (model) => /^[\w.-]+\/[\w.:-]+$/.test(model.trim()),
  async run({ prompt, model, timeoutMs, maxOutputChars }) {
    assertInstalled('opencode');
    const [bin, ...prefix] = launcherArgv('opencode');
    const args = [...prefix, 'run', '--pure', '--format', 'json'];
    if (model) args.push('--model', model);
    return extractOpencodeText(
      await runCli({ bin, args, stdin: prompt, timeoutMs, maxOutputChars }),
    );
  },
};

/**
 * `pi --print` — verified against the installed CLI (`pi --help`).
 *
 * `--no-tools` and `--no-session` are the difference between a completion and
 * an agent: no tool registry to build, nothing written to disk.
 *
 * Note this is the one provider that on this machine has no credentials at all
 * (`pi` defaults to google and reports "No API key found"), which the classifier
 * turns into `not-authed` rather than a generic failure.
 */
const piAdapter: CompletionAdapter = {
  provider: 'pi',
  defaultModel: null,
  servesModel: (model) => /^[\w.-]+\/[\w.:-]+$/.test(model.trim()),
  async run({ prompt, model, timeoutMs, maxOutputChars }) {
    assertInstalled('pi');
    const [bin, ...prefix] = launcherArgv('pi');
    const args = [...prefix, '--print', '--no-tools', '--no-session', '--mode', 'text'];
    if (model) args.push('--model', model);
    args.push(prompt);
    return stripAnsi(await runCli({ bin, args, stdin: '', timeoutMs, maxOutputChars }));
  },
};

const ADAPTERS: Record<CompletionProvider, CompletionAdapter> = {
  claude: claudeAdapter,
  codex: codexAdapter,
  opencode: opencodeAdapter,
  pi: piAdapter,
};

// ---------------------------------------------------------------------------
// Model resolution
// ---------------------------------------------------------------------------

/** True when this provider has a one-shot implementation at all. */
export function completionSupported(provider: string): provider is CompletionProvider {
  return provider in ADAPTERS;
}

/**
 * The model a provider uses when the caller names none. `null` means "whatever
 * the user already configured in that CLI" — see [`CompletionAdapter.defaultModel`].
 */
export function defaultModelFor(provider: CompletionProvider): string | null {
  return ADAPTERS[provider]?.defaultModel ?? null;
}

/** True when `provider` can serve `model`. */
export function servesModel(provider: CompletionProvider, model: string): boolean {
  return ADAPTERS[provider]?.servesModel(model) ?? false;
}

/**
 * Pick a usable model for `provider` from a possibly-wrong `requested` one.
 *
 * For consumers holding a model string from config that predates
 * multi-provider support — `agents.autoTitle.model: 'haiku'` is a claude alias,
 * and handing it to codex produces an invalid-model error at best. This
 * downgrades to the provider's own default and says it downgraded, so the
 * caller can log it once instead of silently doing the wrong thing.
 *
 * [`complete`] itself does NOT do this: given an unservable model it fails with
 * `unsupported-model`, because a primitive that quietly substitutes models is
 * how you end up not knowing which model wrote your text.
 */
export function resolveCompletionModel(
  provider: CompletionProvider,
  requested?: string | null,
): { model: string | null; downgraded: boolean } {
  const want = (requested ?? '').trim();
  if (!want) return { model: defaultModelFor(provider), downgraded: false };
  if (servesModel(provider, want)) return { model: want, downgraded: false };
  return { model: defaultModelFor(provider), downgraded: true };
}

// ---------------------------------------------------------------------------
// The primitive
// ---------------------------------------------------------------------------

function fail(
  reason: CompletionFailureReason,
  message: string,
  provider: CompletionProvider,
  model: string | null,
  startedAt: number,
): CompletionError {
  return { ok: false, reason, message, provider, model, elapsedMs: Date.now() - startedAt };
}

/**
 * Ask one model one question. Never throws, never hangs.
 *
 * Every outcome — including "there is no such provider" and "the binary isn't
 * installed" — comes back as a [`CompletionResult`], so this can sit in a write
 * path without being able to fail it.
 */
export async function complete(req: CompletionRequest): Promise<CompletionResult> {
  const startedAt = Date.now();
  const provider = req.provider;

  const adapter = ADAPTERS[provider];
  if (!adapter) {
    return fail(
      'unsupported-provider',
      `no one-shot adapter for '${provider}'`,
      provider,
      null,
      startedAt,
    );
  }

  const prompt = (req.prompt ?? '').trim();
  if (!prompt) {
    return fail('empty', 'prompt is empty', provider, null, startedAt);
  }
  // Truncate rather than refuse: a caller that overshot still wants an answer,
  // and the cap is what keeps this cheap.
  const capped = prompt.length > MAX_PROMPT_CHARS ? prompt.slice(0, MAX_PROMPT_CHARS) : prompt;

  const wanted = (req.model ?? '').trim();
  if (wanted && !adapter.servesModel(wanted)) {
    return fail(
      'unsupported-model',
      `${provider} cannot serve model '${wanted}'` +
        (adapter.defaultModel ? ` (its default is '${adapter.defaultModel}')` : ''),
      provider,
      wanted,
      startedAt,
    );
  }
  const model = wanted || adapter.defaultModel;

  const timeoutMs = Math.min(
    MAX_TIMEOUT_MS,
    Math.max(MIN_TIMEOUT_MS, req.timeoutMs ?? DEFAULT_TIMEOUT_MS),
  );
  const maxOutputChars = Math.max(1, req.maxOutputChars ?? DEFAULT_MAX_OUTPUT_CHARS);

  try {
    const raw = await adapter.run({ prompt: capped, model, timeoutMs, maxOutputChars });
    const text = (raw ?? '').trim().slice(0, maxOutputChars);
    if (!text) {
      return fail('empty', `${provider} returned no text`, provider, model, startedAt);
    }
    return { ok: true, text, provider, model, elapsedMs: Date.now() - startedAt };
  } catch (err) {
    if (err instanceof CompletionFailure) {
      return fail(err.reason, err.message, provider, model, startedAt);
    }
    return fail('failed', (err as Error)?.message ?? String(err), provider, model, startedAt);
  }
}
