/**
 * commandRegistry — the ex-verb vocabulary behind `prefix :` (COMMAND_LAYER.md
 * Phase 5), and the one dispatch door other surfaces use to run keyboard
 * ACTIONS without owning the dispatcher.
 *
 * The verbs are vim/tmux muscle memory (`:q`, `:vs`, `:on`, `:term`, …), the
 * TUI's cmdline vocabulary kept consistent. Each maps to a registry action id
 * and is executed by useKeyboardNav's own executeAction via the
 * `command:action` CustomEvent — the same switch the chords fire, so cmdline
 * and keys can never disagree about what a verb does. (The event door is also
 * the seam a future hub-bus `command.run` lands on.)
 */
import { ACTION_LABELS } from './shortcuts';

export const COMMAND_ACTION_EVENT = 'command:action';

/** Run a keyboard action by registry id through the dispatcher's switch. */
export function runLayerAction(action: string): void {
  window.dispatchEvent(new CustomEvent(COMMAND_ACTION_EVENT, { detail: { action } }));
}

export interface ExVerb {
  /** What the user types after ':' (also matched as a prefix). */
  verb: string;
  /** Registry action id executed via runLayerAction. */
  action: string;
  /** Extra search terms. */
  aliases?: string[];
}

/** vim/tmux-flavored verbs, kept consistent with the TUI's cmdline. */
export const EX_VERBS: ExVerb[] = [
  { verb: 'q', action: 'close-pane', aliases: ['quit', 'clo', 'close'] },
  { verb: 'on', action: 'zoom-pane', aliases: ['only', 'zoom', 'z'] },
  { verb: 'vs', action: 'quick-split', aliases: ['vsplit'] },
  { verb: 'sp', action: 'split', aliases: ['split'] },
  { verb: 'new', action: 'new-claude', aliases: ['claude'] },
  { verb: 'term', action: 'new-terminal', aliases: ['terminal'] },
  { verb: 'pin', action: 'pin-agent', aliases: ['harpoon', 'mark'] },
  { verb: 'rename', action: 'rename-tab', aliases: ['ren'] },
  { verb: 'w', action: 'save-session', aliases: ['write', 'save'] },
  { verb: 'help', action: 'toggle-help', aliases: ['h', 'keys'] },
  { verb: 'hints', action: 'pane-hints', aliases: ['panes', 'display'] },
];

/** Display label for a verb's target action. */
export function exVerbLabel(v: ExVerb): string {
  return ACTION_LABELS[v.action] ?? v.action;
}
