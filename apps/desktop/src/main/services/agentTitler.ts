/**
 * Names an agent after what it is actually doing, the way a chat service names
 * a conversation: one cheap model call on the first exchange, then never again.
 *
 * The call is a `directCompletion` one-shot — no agent session is spawned for
 * a seven-word title. Which harness answers is the agent's OWN provider, not
 * claude: this used to shell out to the claude binary unconditionally, so a
 * codex-primary user with no claude installed silently lost auto-titling
 * altogether. The primitive owns the per-provider transport (and the
 * ghost-session suppression claude needs); this file owns the prompt and what
 * counts as a title.
 *
 * The model comes from `config.agents.autoTitle` — a title is a two-token task,
 * so it is kept cheap and is deliberately NOT the agent's own model. Because
 * every agent is titled by its OWN harness, that setting is a per-harness map
 * (`autoTitle.models`, resolved by lib/roleModels): a mixed fleet needs a claude
 * title model AND a codex one live at once, not one field that can only ever be
 * right for whichever harness the user configured last. The legacy single
 * `autoTitle.model` still ships `'haiku'`, a CLAUDE alias, so it is honoured
 * only for harnesses that can serve it and otherwise falls through to that
 * harness's own default rather than being handed to a CLI that would reject it.
 *
 * When the call can't run at all (daemon down, no binary, not logged in,
 * timeout, refusal) the caller still gets a title: the first line of what the
 * user asked, via sessionTitles' `cleanTitle`, which is exactly what the RECENT
 * list has always shown.
 */
import type { AgentProvider } from './agentProviders';
import { complete, resolveCompletionModel, completionSupported } from './directCompletion';
import { resolveTitleModel } from '../lib/roleModels';
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

export interface TitleRequest {
  /** The first thing the user asked this agent. */
  userMessage: string;
  /** The agent's first reply, when there is one — sharpens vague openers. */
  assistantReply?: string;
  /**
   * The agent's own backend. Absent ⇒ 'claude', matching every other
   * provider-optional field in the tree (see types/pane.ts `resolveProvider`)
   * and keeping agents spawned before this parameter existed working.
   */
  provider?: AgentProvider;
}

/**
 * The model to title with, for THIS provider.
 *
 * Two layers, and both are load-bearing. `configured` has already been resolved
 * per-harness by [`resolveTitleModel`] — `autoTitle.models[provider]` first, the
 * legacy single field only when this harness can serve it — so on a configured
 * fleet it is already this provider's own id and passes straight through.
 *
 * `resolveCompletionModel` stays underneath it as the backstop, because the map
 * is user-written: someone can type a codex id into the claude row, or keep a
 * model the CLI has since retired. That downgrade is logged rather than
 * swallowed — a title quietly written by a different model than the user
 * configured is the sort of thing worth being able to grep for.
 *
 * Exported for tests — this is the provider seam, so it is the part worth
 * pinning.
 */
export function titleModelFor(provider: AgentProvider, configured?: string): string | null {
  const { model, downgraded } = resolveCompletionModel(provider, configured);
  if (downgraded) {
    console.log(
      `[agentTitler] configured title model '${configured}' is not a ${provider} model; ` +
        `using ${model ?? `${provider}'s own default`}`,
    );
  }
  return model;
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

  const provider = req.provider ?? 'claude';
  const fallback = cleanTitle(userMessage) ?? null;
  const degraded = () => fallback && clipWords(fallback, MAX_TITLE_CHARS * 2);

  // A provider with no one-shot adapter isn't a reason to leave the agent named
  // after its cwd — the user's own first line has always been a fine title.
  if (!completionSupported(provider)) return degraded();

  // `complete` is documented never to throw, but a cosmetic feature must not be
  // able to reject into a UI path even if that ever stops being true.
  const res = await complete({
    provider,
    prompt: buildTitlePrompt(userMessage, req.assistantReply),
    // Per-harness first (autoTitle.models[provider]), then the legacy single
    // field when this harness can serve it, then the harness's own default.
    model: titleModelFor(provider, resolveTitleModel(provider)),
    timeoutMs: TIMEOUT_MS,
    // A title is a handful of words; a model that writes an essay gets cut off
    // rather than being allowed to stream one back.
    maxOutputChars: MAX_TITLE_CHARS * 8,
  }).catch((err: unknown) => {
    console.log(`[agentTitler] title call threw: ${(err as Error)?.message}`);
    return { ok: false as const, reason: 'failed' as const, message: 'threw' };
  });
  if (!res.ok) {
    // One line, one reason, once per agent — a cosmetic feature must be legible
    // in the log without being noisy in it.
    console.log(`[agentTitler] no ${provider} title (${res.reason}): ${res.message}`);
    return degraded();
  }
  return sanitizeTitle(res.text) ?? degraded();
}
