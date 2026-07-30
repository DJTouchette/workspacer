/**
 * Names an agent after what it is actually doing, the way a chat service names
 * a conversation: one cheap model call on the first exchange, then never again.
 *
 * The call goes through claudemon's `POST /oneshot`, NOT a local `claude
 * --print` — a headless claude fires the user's Claude Code hooks, and the
 * daemon would register a stray session for every one of them (a ghost row in
 * RECENT per titled agent, verified). The daemon pins and suppresses the run's
 * session id the way keep-warm already does. We only resolve the launcher argv,
 * the same division of labor as spawns and heartbeats.
 *
 * The model comes from `config.agents.autoTitle.model` — a title is a two-token
 * task, so it defaults to haiku and is deliberately NOT the agent's own model.
 * When the call can't run at all (daemon down or too old for the route, no
 * claude, timeout, refusal) the caller still gets a title: the first line of
 * what the user asked, via sessionTitles' `cleanTitle`, which is exactly what
 * the RECENT list has always shown.
 */
import { CLAUDEMON_API_URL } from './claudemonDaemon';
import { claudeBaseArgv } from './claudeResolver';
import { configService } from './configService';
import { cleanTitle } from './sessionTitles';

/** A title is a handful of words; anything longer is the model ignoring us. */
const MAX_TITLE_WORDS = 7;
const MAX_TITLE_CHARS = 52;
/** How much of the opening exchange the model gets to look at. */
const PROMPT_CAP = 1200;
const REPLY_CAP = 600;
/** Titles are not worth waiting on — the agent is already working by now. */
const TIMEOUT_MS = 25_000;

const INSTRUCTION = [
  'Write a title for this coding-session conversation, from the first exchange below.',
  'Rules: 3 to 6 words. Imperative or noun phrase. No quotes, no trailing period,',
  'no preamble, no markdown. Name the actual task, not the tools.',
  'Reply with the title and nothing else.',
].join(' ');

/** Function words a title must never END on — capping at a word count or a
 *  char budget otherwise leaves "…the intermittent failure in", which reads as
 *  a sentence someone cut off rather than a title. */
const DANGLING = new Set([
  'a',
  'an',
  'and',
  'at',
  'by',
  'for',
  'from',
  'in',
  'into',
  'of',
  'on',
  'or',
  'that',
  'the',
  'this',
  'to',
  'with',
]);

/** Drop trailing function words (repeatedly — "in the" can both be trailing). */
function dropDangling(words: string[]): string[] {
  const out = [...words];
  while (out.length > 1 && DANGLING.has(out[out.length - 1].toLowerCase())) out.pop();
  return out;
}

/** Trim to a whole word at or under `max` chars (no mid-word cut). */
function clipWords(text: string, max: number): string {
  if (text.length <= max) return text;
  const cut = text.slice(0, max);
  const lastSpace = cut.lastIndexOf(' ');
  return (lastSpace > max * 0.5 ? cut.slice(0, lastSpace) : cut).trim();
}

/**
 * Turn raw model output into a title, or null when it doesn't look like one.
 *
 * Pure and exported because this is where the failure modes live: a model that
 * answers in a sentence, wraps in quotes, prefixes "Title:", apologises, or
 * emits an empty line should all end as null so the caller falls back rather
 * than naming an agent "Sure! Here is a title for that conversation:".
 */
export function sanitizeTitle(raw: string): string | null {
  const firstLine = (raw ?? '')
    .split('\n')
    .map((l) => l.trim())
    .find((l) => l.length > 0);
  if (!firstLine) return null;

  let t = firstLine
    // "Title:" / "Here's a title:" style preambles.
    .replace(/^[^:]{0,24}title[^:]{0,12}:\s*/i, '')
    // Markdown bullets/headers and surrounding quotes or backticks.
    .replace(/^[#>*\-\s]+/, '')
    .replace(/^["'`“”‘’]+|["'`“”‘’]+$/g, '')
    .replace(/\s+/g, ' ')
    .replace(/[.,;:!]+$/, '')
    .trim();

  if (!t) return null;
  // A refusal or a chatty answer, not a title.
  if (/^(sorry|i'm sorry|i cannot|i can't|unfortunately|as an ai)\b/i.test(t)) return null;
  // Way past a title's length means the model wrote prose; don't salvage it.
  if (t.length > MAX_TITLE_CHARS * 3) return null;

  const words = t.split(' ').filter(Boolean);
  t = dropDangling(words.slice(0, MAX_TITLE_WORDS)).join(' ');
  t = dropDangling(clipWords(t, MAX_TITLE_CHARS).split(' ')).join(' ');
  return t || null;
}

/** The prompt handed to the titling model. Exported for tests. */
export function buildTitlePrompt(userMessage: string, assistantReply?: string): string {
  const parts = [INSTRUCTION, '', `User: ${userMessage.slice(0, PROMPT_CAP)}`];
  if (assistantReply?.trim()) parts.push(`Assistant: ${assistantReply.slice(0, REPLY_CAP)}`);
  return parts.join('\n');
}

/** Ask the daemon for one headless turn. Null on any failure — never throws. */
async function runOneshot(prompt: string, model: string): Promise<string | null> {
  try {
    const res = await fetch(`${CLAUDEMON_API_URL}/oneshot`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        argv: claudeBaseArgv(),
        model,
        prompt,
        timeout_secs: Math.round(TIMEOUT_MS / 1000),
      }),
      signal: AbortSignal.timeout(TIMEOUT_MS + 5_000),
    });
    if (!res.ok) {
      // A daemon predating /oneshot answers 404 — fall back, don't shell out.
      console.log(`[agentTitler] /oneshot returned ${res.status}`);
      return null;
    }
    const body = (await res.json()) as { ok?: boolean; text?: string; error?: string };
    if (!body?.ok) {
      console.log(`[agentTitler] title call failed: ${body?.error ?? 'unknown'}`);
      return null;
    }
    return body.text ?? null;
  } catch (err) {
    console.log(`[agentTitler] title call failed: ${(err as Error).message}`);
    return null;
  }
}

export interface TitleRequest {
  /** The first thing the user asked this agent. */
  userMessage: string;
  /** The agent's first reply, when there is one — sharpens vague openers. */
  assistantReply?: string;
}

/**
 * A title for an agent, or null to leave its name alone.
 *
 * Null means "don't rename": the feature is off, or there was nothing to title.
 * A model that fails does NOT return null — the user's own first line is a
 * perfectly good title and better than the cwd basename.
 */
export async function generateAgentTitle(req: TitleRequest): Promise<string | null> {
  const cfg = configService.getConfig().agents?.autoTitle;
  if (cfg?.enabled === false) return null;
  const userMessage = (req.userMessage ?? '').trim();
  if (!userMessage) return null;

  const fallback = cleanTitle(userMessage) ?? null;
  const raw = await runOneshot(
    buildTitlePrompt(userMessage, req.assistantReply),
    cfg?.model ?? 'haiku',
  );
  if (raw === null) return fallback && clipWords(fallback, MAX_TITLE_CHARS * 2);
  return sanitizeTitle(raw) ?? (fallback && clipWords(fallback, MAX_TITLE_CHARS * 2));
}
