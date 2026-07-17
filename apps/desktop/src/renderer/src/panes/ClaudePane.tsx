import React, { useEffect, useLayoutEffect, useRef, useCallback, useState, useMemo } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebFontsAddon } from '@xterm/addon-web-fonts';
import '@xterm/xterm/css/xterm.css';
import { useClaudeSpawn } from '../hooks/useClaudeSpawn';
import { useClaudeSession } from '../hooks/useClaudeSession';
import { providerLabel } from '../hooks/useAgentManager';
import { useConfig } from '../hooks/useConfig';
import { useUiMode } from '../hooks/useUiMode';
import { useTheme } from '../hooks/useTheme';
import type { ConversationTurn, ToolCall, PendingQuestion } from '../types/claudeSession';
import { anchorWork } from '../lib/anchorWork';
import type { AgentProvider } from '../types/pane';
import {
  claudeColors as colors,
  ensureKeyframes,
  StatusBadge,
  sendApproval,
} from '../components/claude-shared';
import { BrandSpinner } from '../components/Brand';
import { RefreshCw } from '../components/icons';
import { PanelRight, ArrowRightLeft, Clock } from 'lucide-react';
import { ContextMenu, ContextMenuItem, ContextMenuLabel } from '../components/ContextMenu';
import { requestHandoff } from '../lib/watchBus';
import { quoteFontFamily, isTermVisible, refitAndRepaint } from '../lib/terminalUtils';
import ErrorBoundary from '../components/ErrorBoundary';
import { clearMdCache } from '../components/markdown';

// ── Sub-components ──
import { InlineWorkLog } from '../components/claude/InlineWorkLog';
import { TasksCard, planSignature } from '../components/claude/TasksCard';
import { ConversationMessage } from '../components/claude/ConversationMessage';
import { CommandCard } from '../components/claude/CommandCard';
import { ConversationEmptyState, AgentHero } from '../components/claude/ConversationEmptyState';
import { permissionModeLabel } from '../lib/providerCaps';
import { TurnDivider } from '../components/claude/TurnDivider';
import { NeedsYouDock } from '../components/claude/NeedsYouDock';
import {
  AnsweredQuestionCard,
  type ResolvedQuestionRecord,
} from '../components/claude/AnsweredQuestionCard';
import { Composer } from '../components/claude/Composer';
import { WorkCard } from '../components/claude/WorkCard';
import { ToolTraceCard } from '../components/claude/ToolTraceCard';
import { ChangedFilesCard } from '../components/claude/ChangedFilesCard';
import {
  collectEditedFiles,
  ensureTurnSnapshot,
  getTurnSnapshot,
  estimateSnapshot,
} from '../lib/turnChanges';
import { InspectorRail } from '../components/claude/InspectorRail';
import { DropOverlay } from '../components/claude/DropOverlay';
import { ScrollToBottomButton } from '../components/claude/ScrollToBottomButton';
import { SessionStatusBar } from '../components/claude/SessionStatusBar';
import { ComposerControls, type RestartOverrides } from '../components/claude/ComposerControls';
import {
  classifyFile,
  buildPromptPrefix,
  extractFilePaths,
} from '../components/claude/fileAttachment';
import type { AttachedFile } from '../components/claude/fileAttachment';
import { useLibrary } from '../hooks/useLibrary';
import { runLibraryItem } from '../lib/libraryBus';
import type { SlashItem } from '../lib/slashItems';
import type { LibraryItem } from '../types/library';

/**
 * Map a submitted answer payload back to human-readable display strings (one per
 * question) for the persistent answered-question card. The picker encodes picks
 * as 1-indexed option numbers; here we resolve those back to their labels, and
 * pass free text / joined multi-select labels through unchanged.
 */
function describeAnswers(
  questions: PendingQuestion[],
  payload: { option?: number; text?: string; answers?: string[] },
): string[] {
  if (payload.option !== undefined) {
    const opt = questions[0]?.options?.[payload.option - 1];
    return [opt?.label ?? String(payload.option)];
  }
  if (payload.text !== undefined) return [payload.text];
  if (payload.answers) {
    return payload.answers.map((raw, i) => {
      const n = Number(raw);
      if (Number.isInteger(n) && String(n) === raw) {
        return questions[i]?.options?.[n - 1]?.label ?? raw;
      }
      return raw;
    });
  }
  return [];
}

interface ClaudePaneProps {
  paneId: string;
  title: string;
  isActive: boolean;
  cwd?: string;
  profileId?: string;
  resumeSessionId?: string;
  /** If set, this pane is a viewer for an already-running daemon session. */
  attachSessionId?: string;
  /** This pane attaches to a session with prior history (resume / respawn /
   *  boot restore) — an empty conversation means the replay is coming. */
  expectHistory?: boolean;
  /** Text to seed the message input with on first mount (library spawn). */
  initialPrompt?: string;
  /** Coding-agent backend. Only 'claude' (undefined) has GUI telemetry today;
   *  other providers (codex/opencode) run their own TUI and are locked to the
   *  terminal view until their managed adapters land. */
  provider?: AgentProvider;
  /** Claude only: 'stream' when the session runs on the headless stream-json
   *  transport (no PTY — GUI-only pane). undefined defers to the session
   *  snapshot, then the config default. */
  transport?: 'pty' | 'stream';
  onPtyReady?: (paneId: string, ptySessionId: string) => void;
}

type ViewMode = 'gui' | 'terminal';

/** Number of conversation turns rendered per page (oldest load on scroll-up) */
const CONVERSATION_PAGE_SIZE = 60;

// ── Main component ──

const ClaudePane: React.FC<ClaudePaneProps> = ({
  paneId,
  title,
  isActive,
  cwd,
  profileId,
  resumeSessionId,
  attachSessionId,
  expectHistory,
  initialPrompt,
  provider,
  transport: transportProp,
  onPtyReady,
}) => {
  const { config, save } = useConfig();
  // HH:MM stamps on chat turns — a global display preference (config-backed),
  // toggled from the pane header's clock button or Settings.
  const showTimestamps = config.claude?.showTimestamps ?? false;
  // Which surfaces this provider has:
  //   claude            — GUI (hooks/transcript telemetry) + terminal (its PTY)
  //   codex (hybrid)    — GUI (app-server JSON-RPC adapter) + terminal (the codex
  //                       TUI in a PTY, `resume --remote` onto the same live
  //                       app-server thread). On Windows the GUI is instead fed by
  //                       tailing the rollout, but it's a hybrid either way.
  //   opencode (hybrid) — GUI (the `opencode serve` /event adapter) + terminal
  //                       (`opencode attach` TUI in a PTY, same serve + session)
  //   pi (hybrid)       — terminal (the pi TUI in a PTY, `--session-id` pinned)
  //                       + GUI (the daemon tails pi's session JSONL). Only a
  //                       supervisor pi (MCP facade) is headless `--mode rpc`,
  //                       and its Term is simply blank.
  //   claude (stream)   — GUI only: the headless stream-json transport runs
  //                       through claudemon's managed adapter with no PTY, so
  //                       the Term surface doesn't exist (pi-supervisor-style).
  //                       Detected below once the session snapshot is in hand.
  const isClaude = (provider ?? 'claude') === 'claude';
  const isHybrid = provider === 'opencode' || provider === 'codex' || provider === 'pi';
  // Display name of the backend for user-facing copy (empty states, composer,
  // exit notice) so a Codex/OpenCode/Pi pane doesn't read as "Claude".
  const agentName = providerLabel(provider);
  // A spawned-with-prompt pane always opens in GUI; otherwise honour the
  // configured default view. The fallback is the structured GUI — the rich
  // conversation surface every provider has — with the Term a toggle away.
  const [viewModeState, setViewModeState] = useState<ViewMode>(
    initialPrompt ? 'gui' : (config.claude?.defaultView ?? 'gui'),
  );
  const noopSetView = useCallback((_v: React.SetStateAction<ViewMode>) => {}, []);
  // hasTerminal / viewMode are derived after the session snapshot is available
  // (the snapshot is the authority on the Claude transport) — see below.
  // Focus mode hides the per-pane inspector rail entirely (mount, composer
  // toggle, hotkey) — declared in the UI-mode manifest.
  const { manifest: uiManifest } = useUiMode();
  const [railOpen, setRailOpen] = useState(() => localStorage.getItem('wks-claude-rail') === '1');
  const [inputValue, setInputValue] = useState(initialPrompt ?? '');
  const [showScrollBtn, setShowScrollBtn] = useState(false);
  const [approvalDismissedAt, setApprovalDismissedAt] = useState(0);
  const [cancelledAt, setCancelledAt] = useState(0);
  const [visibleCount, setVisibleCount] = useState(CONVERSATION_PAGE_SIZE);
  const [attachedFiles, setAttachedFiles] = useState<AttachedFile[]>([]);
  const [isDragOver, setIsDragOver] = useState(false);
  const dragCounterRef = useRef(0);
  const termContainerRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const termInitRef = useRef(false);
  // Guards the one-shot session spawn so the visible-fit retry loop below can't
  // start it twice (sessionId only lands async, after the spawn resolves).
  const sessionStartedRef = useRef(false);
  const conversationEndRef = useRef<HTMLDivElement>(null);
  const scrollContainerRef = useRef<HTMLDivElement>(null);
  const contentAreaRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  const { terminalTheme } = useTheme();
  const termCfg = config.terminal;

  // Mirror viewMode into a ref so the run-once xterm-init effect can read the
  // current view without re-running (and re-spawning) on every toggle.
  // (Assigned below, once viewMode is derived from the session snapshot.)
  const viewModeRef = useRef<ViewMode>('gui');

  // Inject keyframes
  useEffect(() => {
    ensureKeyframes();
  }, []);

  // Set CSS variable for mono font
  useEffect(() => {
    if (typeof document !== 'undefined') {
      document.documentElement.style.setProperty(
        '--claude-mono-font',
        termCfg.fontFamily || 'var(--wks-font-mono)',
      );
    }
  }, [termCfg.fontFamily]);

  const handleExit = useCallback(() => {
    if (terminalRef.current) {
      terminalRef.current.write(`\r\n\x1b[90m[${agentName} session exited]\x1b[0m\r\n`);
    }
  }, [agentName]);

  const {
    sessionId,
    isReady,
    spawnError,
    write,
    resize,
    attachToTerminal,
    startSession,
    retry,
    restartSession,
  } = useClaudeSpawn({
    paneId,
    cwd,
    profileId,
    resumeSessionId,
    attachSessionId,
    onExit: handleExit,
    defer: true,
  });

  // Refresh button: clear rendering glitches across both views.
  //  1) Nudge the content area onto a fresh raster (toggle a composited property
  //     for one frame) — clears the rare backdrop-filter compositing garble.
  //  2) Refit + repaint the xterm grid and send the PTY a resize so the agent's
  //     TUI redraws. A same-size resize won't trigger the child's SIGWINCH
  //     redraw, so nudge the rows by one and restore next frame — Claude Code /
  //     Codex repaint fully on resize, which clears glitches a client-side
  //     refresh alone can't. No-op (safely) when the terminal view is hidden.
  const forceRepaint = useCallback(() => {
    const el = contentAreaRef.current;
    if (el) {
      el.style.transform = 'translateZ(0)';
      el.style.opacity = '0.999';
      requestAnimationFrame(() => {
        if (!el) return;
        el.style.transform = '';
        el.style.opacity = '';
      });
    }
    const term = terminalRef.current;
    const fit = fitAddonRef.current;
    if (term && fit && isTermVisible(termContainerRef.current)) {
      try {
        fit.fit();
        term.refresh(0, term.rows - 1);
        const { cols, rows } = term;
        if (rows > 1) {
          resize(cols, rows - 1);
          requestAnimationFrame(() => resize(cols, rows));
        } else {
          resize(cols, rows);
        }
      } catch {}
    }
  }, [resize]);

  // After a grace period with no session, surface the hook-config hint in the
  // connecting empty state (most spawns connect in well under this). Reset the
  // timer whenever we start fresh (no session, no error).
  const [showHookHint, setShowHookHint] = useState(false);
  useEffect(() => {
    if (sessionId || spawnError) {
      setShowHookHint(false);
      return;
    }
    setShowHookHint(false);
    const id = setTimeout(() => setShowHookHint(true), 10000);
    return () => clearTimeout(id);
  }, [sessionId, spawnError]);

  const { session } = useClaudeSession({ ptySessionId: sessionId, active: isActive });

  // Tasks/plan card (view-only, pinned above the composer). Dismissal is keyed
  // by the plan's signature so a stale/stuck list can be put away for good,
  // while any real change to the tasks brings the card back.
  const plan = session?.plan;
  const planSig = planSignature(plan);
  const [dismissedPlanSig, setDismissedPlanSig] = useState<string | null>(null);
  const showTasksCard = planSig !== '' && dismissedPlanSig !== planSig;

  // Where the agent is actually working right now. `cwd` (the spawn dir)
  // stays authoritative for spawn/restart; `effectiveCwd` follows the agent
  // into a git worktree (session.liveCwd, tracked from per-hook cwds) so file
  // opens, diffs, git lookups and the file picker resolve against the tree
  // the agent is editing.
  const effectiveCwd = session?.liveCwd || cwd;

  // "/" command picker data, from two sources:
  //  - the session's own slash commands (stream init capabilities): picking
  //    one keeps "/name " in the composer and the message is sent verbatim —
  //    the CLI expands it (verified live: custom commands and headless-safe
  //    built-ins both run over stream-json input);
  //  - skills and reusable prompts from the merged library (project + global)
  //    for this cwd: picking one inserts the item's *content* into the
  //    composer (the same templating path the command palette uses).
  // When a library skill/command shares a name with a real session command,
  // the run entry wins — invoking the real command beats pasting its text.
  const { items: libraryItems } = useLibrary(effectiveCwd);
  const sessionCommands = session?.statusLine?.capabilities?.inventory?.slashCommands;
  // Key by scope:kind:id, not the bare LibraryItem.id — that id is only a
  // filename slug, so a skill and a command (or a global and a claude item) can
  // share one, which would collide React keys and the pick lookup. The composite
  // is unique and the picker resolves back through this map.
  const slashLookup = useMemo(() => {
    const m = new Map<string, LibraryItem>();
    const cli = new Set((sessionCommands ?? []).map((c) => c.toLowerCase()));
    for (const it of libraryItems) {
      if (it.kind === 'skill' || it.kind === 'prompt' || it.kind === 'command') {
        if (it.kind !== 'prompt' && cli.has(it.title.toLowerCase())) continue;
        m.set(`${it.scope}:${it.kind}:${it.id}`, it);
      }
    }
    return m;
  }, [libraryItems, sessionCommands]);
  const slashItems: SlashItem[] = useMemo(() => {
    const run: SlashItem[] = (sessionCommands ?? []).map((name) => ({
      id: `run:${name}`,
      label: name,
      hint: 'Run in this session',
      kind: 'run',
    }));
    const insert: SlashItem[] = Array.from(slashLookup, ([key, it]) => ({
      id: key,
      label: it.title,
      hint: it.description,
      kind: it.kind,
    }));
    return [...run, ...insert];
  }, [slashLookup, sessionCommands]);
  const handleSlashPick = useCallback(
    (key: string) => {
      // A session command: leave "/name " in the composer — the user appends
      // args if any and Enter sends it verbatim for the CLI to expand.
      if (key.startsWith('run:')) {
        setInputValue(`/${key.slice(4)} `);
        return;
      }
      const item = slashLookup.get(key);
      if (!item) return;
      // Clear the "/query" first so the inserted content replaces it: the
      // library:insert handler appends to the current input, and LibraryHost's
      // (async) templating fires after this state update commits, so `prev` is
      // empty by the time it inserts.
      setInputValue('');
      // Force 'insert' regardless of the item's default action — in a message
      // composer the intent is always to drop the content in, never to spawn a
      // fresh agent or copy to the clipboard.
      runLibraryItem(item, 'insert');
    },
    [slashLookup],
  );

  // Which surfaces this pane has. The session snapshot is the authority on the
  // Claude transport ('stream' = headless stream-json, no PTY); until it loads
  // we trust the pane prop (set at spawn by the agent manager) and then the
  // config default, so a stream pane never flashes a terminal surface.
  const claudeTransport: 'pty' | 'stream' = !isClaude
    ? 'pty'
    : session
      ? (session.transport ?? 'pty')
      : (transportProp ?? config.claude?.transport ?? 'pty');
  const isStream = isClaude && claudeTransport === 'stream';
  // Codex has the same split: hybrid (native TUI PTY + GUI) or headless
  // 'stream' (daemon-owned thread, GUI only). The session snapshot / pane prop
  // carry the daemon-stamped transport; config.claude is claude-only, so the
  // non-claude fallback stays 'pty' (hybrid).
  const managedStream =
    isHybrid && (session ? session.transport === 'stream' : transportProp === 'stream');
  const hasGui = true; // every provider surfaces a structured GUI conversation
  // Only PTY claude + hybrid providers have a Term; stream claude AND headless
  // codex are GUI-only (there is no PTY to render or write keystrokes to).
  const hasTerminal = (isClaude && !isStream) || (isHybrid && !managedStream);
  const showViewToggle = hasGui && hasTerminal; // both surfaces → show the toggle
  // Lock to the sole available surface when the provider doesn't offer both;
  // any auto-switch below then becomes a no-op.
  const viewMode: ViewMode = !hasGui ? 'terminal' : !hasTerminal ? 'gui' : viewModeState;
  const setViewMode = showViewToggle ? setViewModeState : noopSetView;
  viewModeRef.current = viewMode;

  // Enable the approval gateway in claudemon as soon as we have a session id
  // so PreToolUse hooks get parked for our UI to resolve.
  useEffect(() => {
    if (!sessionId) return;
    window.electronAPI
      .claudeGate(sessionId, true)
      .catch((err) => console.warn('[ClaudePane] failed to enable approval gate:', err));
  }, [sessionId]);

  // Notify parent of PTY session ID
  useEffect(() => {
    if (sessionId && onPtyReady) {
      onPtyReady(paneId, sessionId);
    }
  }, [sessionId, paneId, onPtyReady]);

  // Clear the module-level markdown cache on session switch so stale ReactNode
  // trees from a previous session don't occupy memory or produce key collisions.
  useEffect(() => {
    clearMdCache();
  }, [sessionId]);

  // Library: receive a prompt/skill inserted from the library. Targeted by
  // sessionId/paneId, or delivered to the active pane when untargeted.
  useEffect(() => {
    const handler = (e: Event) => {
      const d = (e as CustomEvent).detail as
        { text?: string; sessionId?: string; paneId?: string } | undefined;
      if (!d?.text) return;
      const targeted = d.sessionId || d.paneId;
      const matches = targeted ? d.sessionId === sessionId || d.paneId === paneId : isActive;
      if (!matches) return;
      setViewMode('gui');
      setInputValue((prev) => (prev.trim() ? `${prev.replace(/\s+$/, '')}\n${d.text}` : d.text!));
      requestAnimationFrame(() => inputRef.current?.focus());
    };
    window.addEventListener('library:insert', handler as EventListener);
    return () => window.removeEventListener('library:insert', handler as EventListener);
  }, [sessionId, paneId, isActive]);

  // Initialize xterm.js
  useEffect(() => {
    const container = termContainerRef.current;
    if (!container || termInitRef.current) return;
    termInitRef.current = true;

    const term = new Terminal({
      cursorBlink: termCfg.cursorBlink,
      fontSize: termCfg.fontSize,
      fontFamily: quoteFontFamily(termCfg.fontFamily),
      theme: terminalTheme,
      allowProposedApi: true,
      scrollback: termCfg.scrollback,
      convertEol: false,
      cursorStyle: termCfg.cursorStyle as 'block' | 'underline' | 'bar',
    });

    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);

    // Work around an xterm 6.x-beta crash: its DECRQM handler ("request mode",
    // CSI ? Ps $ p and CSI Ps $ p) throws `ReferenceError: i is not defined`,
    // which aborts the whole write() and blanks the terminal. Claude's TUI probes
    // modes with this, and the web mirror replays it in one large coalesced chunk
    // so the crash eats the entire replay. Consume the sequence ourselves (no-op)
    // so the buggy default never runs — an unanswered mode query just reads as
    // "unsupported", which is safe.
    term.parser.registerCsiHandler({ prefix: '?', intermediates: '$', final: 'p' }, () => true);
    term.parser.registerCsiHandler({ intermediates: '$', final: 'p' }, () => true);

    terminalRef.current = term;
    fitAddonRef.current = fitAddon;

    // Use web-fonts addon to ensure @font-face fonts are loaded before canvas renders
    const webFontsAddon = new WebFontsAddon();
    term.loadAddon(webFontsAddon);

    webFontsAddon.loadFonts().then(() => {
      term.open(container);

      // Spawn only once the terminal is laid out at its real, visible size.
      // claude --resume (and an attach replay) re-render the WHOLE transcript
      // wrapped to the PTY width the daemon is told at spawn — fitting a hidden
      // or not-yet-laid-out 0×0 container bakes that replay in at the wrong
      // width, producing a garbled screen that no later refit can reflow.
      // While the terminal is the visible surface we wait for a real fit; if
      // it's hidden by design (GUI default view) its glyphs aren't what the
      // user sees, so we spawn promptly and let a later refit+resize repaint.
      let attempts = 0;
      const MAX_ATTEMPTS = 15; // ~1.5s of 100ms retries before giving up
      const startWhenSized = () => {
        if (sessionStartedRef.current || !termInitRef.current) return;
        const visible = isTermVisible(container);
        if (visible) {
          try {
            fitAddon.fit();
          } catch {}
        }
        const guiHidden = viewModeRef.current === 'gui' && !visible;
        if (visible || guiHidden || attempts >= MAX_ATTEMPTS) {
          sessionStartedRef.current = true;
          startSession(term.cols, term.rows);
          return;
        }
        attempts++;
        setTimeout(startWhenSized, 100);
      };
      startWhenSized();
    });

    term.attachCustomKeyEventHandler((e: KeyboardEvent) => {
      // Ctrl+Shift+C — copy from terminal
      if (e.ctrlKey && e.shiftKey && e.key === 'C') {
        e.preventDefault();
        const sel = term.getSelection();
        if (sel) navigator.clipboard.writeText(sel);
        return false;
      }
      // Ctrl+Shift+V — paste. Let xterm's native paste event deliver the text
      // (single insert, bracketed-paste aware); return false only to suppress ^V.
      if (e.ctrlKey && e.shiftKey && e.key === 'V') {
        return false;
      }
      // Ctrl+C — copy if selection, SIGINT if not
      if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key === 'c') {
        const sel = term.getSelection();
        if (sel) {
          e.preventDefault();
          navigator.clipboard.writeText(sel);
          term.clearSelection();
          return false;
        }
        return true;
      }
      // Ctrl+V — paste. Handled by xterm's native paste event (single insert,
      // bracketed-paste aware); return false so xterm doesn't also emit ^V.
      // Manual clipboard.readText + write here caused a double paste, because
      // preventDefault on keydown does not stop the browser's native paste event.
      if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key === 'v') {
        return false;
      }
      if (
        e.ctrlKey &&
        !e.altKey &&
        !e.shiftKey &&
        ['t', 'b', 'w', 'd', '/', '?', ',', 's', 'k'].includes(e.key)
      )
        return false;
      if (e.ctrlKey && !e.altKey && !e.shiftKey && /^[1-9]$/.test(e.key)) return false;
      if (
        e.altKey &&
        !e.ctrlKey &&
        ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(e.key)
      )
        return false;
      if (e.ctrlKey && e.altKey && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) return false;
      if (e.ctrlKey && e.shiftKey) return false;
      if (e.key === 'F2') return false;
      return true;
    });

    const fitRetry = () => {
      try {
        fitAddon.fit();
      } catch {}
    };
    requestAnimationFrame(fitRetry);
    setTimeout(fitRetry, 100);
    setTimeout(fitRetry, 300);

    attachToTerminal(term);

    const onDataDisp = term.onData((data) => write(data));
    const onBinaryDisp = term.onBinary((data) => write(data));

    const observer = new ResizeObserver(() => {
      // Skip while hidden: toggling a workspace to display:none fires a 0×0
      // resize, and fitting that collapses the grid and garbles the PTY on show.
      if (!isTermVisible(container)) return;
      requestAnimationFrame(() => {
        try {
          fitAddonRef.current?.fit();
        } catch {}
      });
    });
    observer.observe(container);

    const onResizeDisp = term.onResize(({ cols, rows }) => resize(cols, rows));

    return () => {
      onDataDisp.dispose();
      onBinaryDisp.dispose();
      onResizeDisp.dispose();
      observer.disconnect();
      term.dispose();
      terminalRef.current = null;
      fitAddonRef.current = null;
      termInitRef.current = false;
      sessionStartedRef.current = false;
    };
  }, [attachToTerminal, write, resize]);

  // Focus the GUI composer when the pane becomes active in GUI mode. Terminal
  // focus + refit is handled by the reveal layout-effect below.
  useEffect(() => {
    if (!isActive || viewMode !== 'gui') return;
    requestAnimationFrame(() => inputRef.current?.focus());
  }, [viewMode, isActive, isReady]);

  // Jump to the latest message whenever the GUI view opens — the scroll
  // container is freshly mounted on each GUI switch, so land at the bottom
  // (instant, no smooth animation) rather than wherever it last rendered.
  useEffect(() => {
    if (viewMode !== 'gui') return;
    const snap = () => {
      const c = scrollContainerRef.current;
      if (c) c.scrollTop = c.scrollHeight;
    };
    // Two frames: one for the GUI subtree to mount, one for content layout.
    const id = requestAnimationFrame(() => requestAnimationFrame(snap));
    return () => cancelAnimationFrame(id);
  }, [viewMode]);

  // Reveal the terminal cleanly when this pane becomes active (or switches to
  // Term view). Switching agents toggles the workspace display:none → block,
  // exposing a terminal that's stale and possibly mis-sized; refitting +
  // repainting it after the browser has already painted is the "PTY coming to
  // life" glitch. So mask it: useLayoutEffect runs *before* paint, so we hide
  // the container instantly (transition off, so stale cells don't fade out),
  // then refit + repaint while it's hidden and fade the correct terminal in.
  useLayoutEffect(() => {
    const container = termContainerRef.current;
    if (!isActive || viewMode !== 'terminal' || !container) return;
    container.style.transition = 'none';
    container.style.opacity = '0';
    terminalRef.current?.focus();
    const id = requestAnimationFrame(() =>
      requestAnimationFrame(() => {
        const t = terminalRef.current;
        refitAndRepaint(fitAddonRef.current, t, container);
        if (t) resize(t.cols, t.rows);
        container.style.transition = 'opacity 0.12s ease-out';
        container.style.opacity = '1';
      }),
    );
    return () => cancelAnimationFrame(id);
  }, [isActive, viewMode, resize]);

  // Update terminal theme when it changes
  useEffect(() => {
    if (terminalRef.current) {
      terminalRef.current.options.theme = terminalTheme;
    }
  }, [terminalTheme]);

  // Sticky-bottom autoscroll: follow new content while the user sits at the
  // bottom; the moment they scroll up (same threshold that reveals the
  // scroll-to-bottom button) the view stops following, and it resumes when
  // they return. A ref, not state — read from the ResizeObserver below.
  const stickToBottomRef = useRef(true);

  // Track scroll position for "scroll to bottom" button + lazy load older messages
  const handleScroll = useCallback(() => {
    const container = scrollContainerRef.current;
    if (!container) return;
    const distFromBottom = container.scrollHeight - container.scrollTop - container.clientHeight;
    setShowScrollBtn(distFromBottom > 150);
    stickToBottomRef.current = distFromBottom <= 150;
  }, []);

  const loadOlderMessages = useCallback(() => {
    const container = scrollContainerRef.current;
    const prevHeight = container?.scrollHeight ?? 0;
    setVisibleCount((prev) => prev + CONVERSATION_PAGE_SIZE);
    // Preserve scroll position after DOM grows upward
    requestAnimationFrame(() => {
      if (container) {
        const newHeight = container.scrollHeight;
        container.scrollTop += newHeight - prevHeight;
      }
    });
  }, []);

  const scrollToBottom = useCallback(() => {
    stickToBottomRef.current = true;
    conversationEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, []);

  // ── File drag & drop ──

  // Global drag & drop — document + window level with dropEffect to tell
  // Electron/Chromium this is a valid drop target (prevents 🚫 cursor).
  // Only the active pane registers listeners so panes don't compete and
  // isDragOver / dragCounterRef can't get stuck on an inactive pane.
  useEffect(() => {
    if (!isActive) return;
    const onDragOver = (e: DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy';
    };
    const onDragEnter = (e: DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy';
      dragCounterRef.current++;
      if (e.dataTransfer?.types.includes('Files')) setIsDragOver(true);
    };
    const onDragLeave = (e: DragEvent) => {
      e.preventDefault();
      dragCounterRef.current--;
      if (dragCounterRef.current === 0) setIsDragOver(false);
    };
    const onDrop = (e: DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      dragCounterRef.current = 0;
      setIsDragOver(false);
      if (e.dataTransfer) {
        const paths = extractFilePaths(e.dataTransfer);
        if (paths.length > 0) {
          setAttachedFiles((prev) => [...prev, ...paths.map(classifyFile)]);
          setViewMode('gui');
        }
      }
    };

    // Register on both document and window for maximum coverage
    document.addEventListener('dragover', onDragOver, true);
    document.addEventListener('dragenter', onDragEnter, true);
    document.addEventListener('dragleave', onDragLeave, true);
    document.addEventListener('drop', onDrop, true);
    window.addEventListener('dragover', onDragOver);
    window.addEventListener('drop', onDrop);

    return () => {
      document.removeEventListener('dragover', onDragOver, true);
      document.removeEventListener('dragenter', onDragEnter, true);
      document.removeEventListener('dragleave', onDragLeave, true);
      document.removeEventListener('drop', onDrop, true);
      window.removeEventListener('dragover', onDragOver);
      window.removeEventListener('drop', onDrop);
    };
  }, [isActive]);

  const handlePaste = useCallback((e: React.ClipboardEvent) => {
    const paths = extractFilePaths(e.clipboardData);
    if (paths.length > 0) {
      e.preventDefault();
      setAttachedFiles((prev) => [...prev, ...paths.map(classifyFile)]);
    }
  }, []);

  const removeAttachedFile = useCallback((idx: number) => {
    setAttachedFiles((prev) => prev.filter((_, i) => i !== idx));
  }, []);

  const openFilePicker = useCallback(async () => {
    const paths = await window.electronAPI.pickFiles(effectiveCwd);
    if (paths.length > 0) {
      setAttachedFiles((prev) => [...prev, ...paths.map(classifyFile)]);
      if (viewMode === 'terminal') setViewMode('gui');
    }
  }, [effectiveCwd, viewMode]);

  const handleApprovalRespond = useCallback(
    (response: 'yes' | 'no') => {
      if (!sessionId) return;
      // If a question picker is also pending (PermissionRequest racing with
      // AskUserQuestion's PreToolUse), the approval card is stale and shouldn't
      // do anything — the user actually wants to answer the picker. Writing a
      // keystroke fallback would select option 1 of the picker by accident.
      const hasPendingQuestion = (session?.pendingQuestions?.length ?? 0) > 0;
      window.electronAPI.claudeApprove(sessionId, response).catch((err) => {
        console.warn('[ClaudePane] /approve failed:', err);
        // The keystroke fallback needs a PTY — no-PTY (stream) sessions have
        // nothing to type into, so /approve is the only path for them. And the
        // keys encode Claude's 3-row permission menu: a hybrid provider's PTY
        // (codex/opencode/pi) has a different approval UI, so typing them there
        // would be garbage input.
        if (!hasTerminal || !isClaude) return;
        if (!hasPendingQuestion) {
          sendApproval('', response === 'yes', write);
        } else {
          console.warn('[ClaudePane] suppressed keystroke fallback — question picker is active');
        }
      });
      setApprovalDismissedAt(Date.now());
    },
    [sessionId, write, session?.pendingQuestions, hasTerminal, isClaude],
  );

  // Optimistic user messages (shown immediately before JSONL catches up).
  // We dequeue FIFO whenever session.conversation grows by a new user-message,
  // regardless of content — content-based matching was unreliable because
  // claude's JSONL records the post-input-processing text which can differ
  // from what we sent (whitespace, paste prefixes, autocomplete munging).
  const [optimisticMessages, setOptimisticMessages] = useState<ConversationTurn[]>([]);
  const [optimisticLoading, setOptimisticLoading] = useState(false);
  // Count of user-messages we've seen consumed by session.conversation.
  const consumedUserCountRef = useRef(0);

  // Handle send — detect issue keys, resolve context, then write to Claude's TUI
  const handleSend = useCallback(async () => {
    const hasFiles = attachedFiles.length > 0;
    const hasText = inputValue.trim().length > 0;
    if (!hasFiles && !hasText) return;

    const userText = inputValue.trim();
    setInputValue('');
    setAttachedFiles([]);

    // Build file prefix
    const filePrefix = hasFiles ? buildPromptPrefix(attachedFiles) : '';

    const fullMessage = filePrefix + userText;

    // Show message immediately and set loading state
    const optimisticTurn: ConversationTurn = {
      role: 'user',
      content: fullMessage,
      timestamp: Date.now(),
    };
    setOptimisticMessages((prev) => [...prev, optimisticTurn]);
    setOptimisticLoading(true);
    // Sending re-sticks the view: your own message (and the reply) should be
    // in sight even if you'd scrolled up — the ResizeObserver does the rest.
    stickToBottomRef.current = true;

    const rawFallback = () => {
      // Keystrokes need a PTY. No-PTY (stream) sessions can't fall back — the
      // POST /message path is their only transport, so a failure there means
      // the send visibly didn't take rather than silently going nowhere.
      if (!hasTerminal) {
        setOptimisticMessages((prev) => prev.filter((t) => t !== optimisticTurn));
        setOptimisticLoading(false);
        setInputValue((prev) => (prev.trim().length > 0 ? prev : userText));
        return;
      }
      // Bracketed paste + a separate Enter, in one frame. Writing raw `text\r`
      // makes the TUI fold the CR into the "paste" (a newline in the composer)
      // instead of submitting; the CR after the ESC[201~ end marker is a real
      // Enter that submits. Mirrors the daemon's send_message_now.
      write('\x1b[200~' + fullMessage.replace(/[\r\n]+$/, '') + '\x1b[201~\r');
    };

    if (!sessionId) {
      rawFallback();
      return;
    }

    // Prefer claudemon's /message endpoint — it owns the whole delivery
    // policy: buffers a message sent before the session is ready (cold-start
    // `unknown`, mid-turn `responding`, or an open approval/question dialog),
    // injects once the prompt has settled, and verifies the submit took
    // (re-pressing Enter if the TUI swallowed it). A single call suffices —
    // no client-side retry race. The only rejection left is a stopped session,
    // where the wrapper is gone and raw keystrokes can't help either — so the
    // raw PTY write stays reserved for transport failure (daemon unreachable).
    try {
      const res = await window.electronAPI.claudeMessage(sessionId, fullMessage);
      if (res.ok) return; // sent or queued by the daemon
      // The session has ended — nothing was delivered. Retract the optimistic
      // bubble and put the text back in the composer so the send visibly
      // didn't take (instead of a phantom message above a dead session).
      console.warn(
        `[ClaudePane] /message rejected (mode=${res.mode}); session is not accepting input`,
      );
      setOptimisticMessages((prev) => prev.filter((t) => t !== optimisticTurn));
      setOptimisticLoading(false);
      setInputValue((prev) => (prev.trim().length > 0 ? prev : userText));
      return;
    } catch (err) {
      console.warn('[ClaudePane] /message failed:', err);
    }
    rawFallback();
  }, [inputValue, write, attachedFiles, sessionId, hasTerminal]);

  // Drop optimistic entries FIFO as session.conversation grows past the
  // count we last consumed. This avoids content-matching pitfalls.
  useEffect(() => {
    const userCount = (session?.conversation ?? []).filter((t) => t.role === 'user').length;
    if (userCount < consumedUserCountRef.current) {
      // The conversation reset under the same session id (managed-provider
      // restart starts a fresh provider-side thread). Re-baseline the consumed
      // count and drop optimistic turns — their real counterparts belong to
      // the old thread and will never arrive to dequeue them.
      consumedUserCountRef.current = userCount;
      setOptimisticMessages([]);
      // The old thread's answered-question cards anchored to indices that no
      // longer exist — drop them so they don't render against the new thread.
      setResolvedQuestions([]);
    } else if (userCount > consumedUserCountRef.current) {
      const newlyConsumed = userCount - consumedUserCountRef.current;
      consumedUserCountRef.current = userCount;
      setOptimisticMessages((prev) =>
        newlyConsumed >= prev.length ? [] : prev.slice(newlyConsumed),
      );
    }
    // Clear optimistic loading when server reports idle or we get a response
    if (
      optimisticLoading &&
      (session?.ambientState === 'idle' || session?.ambientState === 'streaming')
    ) {
      setOptimisticLoading(false);
    }
  }, [session?.conversation, session?.ambientState, optimisticLoading]);

  // ── Derived data ──

  const activeToolCalls = session?.activeToolCalls ?? [];
  const completedToolCalls = session?.completedToolCalls ?? [];
  const conversation = useMemo(() => {
    const base = session?.conversation ?? [];
    if (optimisticMessages.length === 0) return base;
    return [...base, ...optimisticMessages];
  }, [session?.conversation, optimisticMessages]);
  const hasOlderMessages = conversation.length > visibleCount;

  // Restoring a prior session (resume spawn or attach): the daemon replays the
  // transcript a beat after the session appears, so an empty conversation here
  // means "history is on its way", not "fresh agent" — show a fetching loader
  // instead of the new-agent hero (which used to flash and then get replaced
  // by the transcript popping into existence). Falls back to the hero after a
  // wait cap: a resumed session that truly has no turns would spin forever.
  // NOTE: attachSessionId alone does NOT imply history — every fresh spawn's
  // pane attaches as a viewer, and gating on it showed "Fetching session…"
  // on brand-new agents. Only an explicit restore marker (or a live resume)
  // means a replay is coming.
  const expectsReplay = !!(resumeSessionId || expectHistory);
  const [historyWaitExpired, setHistoryWaitExpired] = useState(false);
  const historyPending = expectsReplay && conversation.length === 0 && !historyWaitExpired;
  useEffect(() => {
    if (!historyPending) return;
    const t = setTimeout(() => setHistoryWaitExpired(true), 15_000);
    return () => clearTimeout(t);
  }, [historyPending]);
  const subagents = session?.subagents ?? [];
  const workflows = session?.workflows ?? [];
  const pendingApproval = session?.pendingApproval ?? null;
  const pendingQuestions = session?.pendingQuestions ?? null;
  // Optimistic dismiss for the question picker, keyed on question CONTENT.
  // The old timestamp gate (lastActivity > dismissedAt) re-showed answered
  // questions on the very next hook/frame — lastActivity bumps on everything —
  // so the picker kept re-prompting until PostToolUse cleared the snapshot.
  // A signature only re-opens the picker when a *different* question set
  // arrives.
  const [dismissedQuestionSig, setDismissedQuestionSig] = useState<string | null>(null);
  // When the snapshot's questions clear (PostToolUse), the answered request is
  // over — reset the dismissal so a textually identical LATER question set
  // still re-opens the picker.
  useEffect(() => {
    if (!session?.pendingQuestions || session.pendingQuestions.length === 0) {
      setDismissedQuestionSig(null);
    }
  }, [session?.pendingQuestions]);
  const questionSig = pendingQuestions?.map((q) => q.question).join(' ') ?? null;

  // Durable record of resolved questions, injected into the transcript so you
  // can always see what you were asked and picked. Kept in renderer state (not
  // the daemon snapshot) and anchored by conversation index, so it survives the
  // snapshot rebuilds that happen on every resync. Cleared when the session or
  // its conversation is reset (see the optimistic-reset effect + sessionId one).
  const [resolvedQuestions, setResolvedQuestions] = useState<ResolvedQuestionRecord[]>([]);
  useEffect(() => {
    setResolvedQuestions([]);
  }, [sessionId]);
  const recordResolved = useCallback(
    (declined: boolean, answers: string[] | null) => {
      if (!pendingQuestions || pendingQuestions.length === 0) return;
      const sig = questionSig ?? '';
      const anchorLen = (session?.conversation ?? []).length;
      setResolvedQuestions((prev) => {
        if (prev.some((r) => r.sig === sig && r.anchorLen === anchorLen)) return prev;
        return [
          ...prev,
          { sig, anchorLen, timestamp: Date.now(), questions: pendingQuestions, answers, declined },
        ];
      });
    },
    [pendingQuestions, questionSig, session?.conversation],
  );

  const handleAnswer = useCallback(
    (payload: { option?: number; text?: string; answers?: string[] }) => {
      if (!sessionId) return;
      setDismissedQuestionSig(questionSig);
      if (pendingQuestions) recordResolved(false, describeAnswers(pendingQuestions, payload));
      // No-PTY sessions (claude 'stream' transport) have no keystroke path at
      // all — the answer must go through POST /sessions/:id/answer, which the
      // daemon delivers structurally over the adapter's control protocol.
      // Non-claude questions are ALWAYS structural, even with a Term attached:
      // they come from the daemon's parked AskUserQuestion MCP call, which the
      // provider's own TUI knows nothing about — keystrokes would be garbage.
      if (!hasTerminal || !isClaude) {
        window.electronAPI.claudeAnswer(sessionId, payload).catch((err) => {
          console.warn('[ClaudePane] /answer failed (no PTY fallback exists):', err);
        });
        return;
      }
      // PTY sessions: write directly to the PTY (via the MessagePort →
      // /sessions/:id/input path) instead of /sessions/:id/answer. /answer
      // requires mode=Question, which can race with concurrent hook events that
      // flip the daemon's mode back to Responding/Approval — and the renderer's
      // view of "picker is up" is what actually matters here. claude's own TUI
      // picker accepts numeric input + Enter the same way it accepts any other
      // keystroke.
      if (payload.option !== undefined) {
        write(`${payload.option}\r`);
      } else if (payload.text !== undefined) {
        write(`${payload.text}\r`);
      } else if (payload.answers) {
        for (const ans of payload.answers) write(`${ans}\r`);
      }
    },
    [sessionId, write, hasTerminal, isClaude, questionSig, pendingQuestions, recordResolved],
  );
  const serverStreaming =
    optimisticLoading ||
    session?.ambientState === 'thinking' ||
    session?.ambientState === 'streaming';
  // If user cancelled, suppress streaming UI until a new activity cycle begins
  const isStreaming = serverStreaming && (session?.lastActivity ?? 0) > cancelledAt;
  // 'background' counts as idle here: the parent turn ENDED (workflow /
  // subagents run detached), so turn-scoped UI — changed-files snapshot
  // freezing, work-card collapse — must behave exactly as on a real idle.
  const ambientIdle = session?.ambientState === 'idle' || session?.ambientState === 'background';

  // ── Changed-files snapshots ──
  //
  // On the busy→idle edge (a turn ending), freeze the git line counts for the
  // files the trailing assistant turn-group edited (see lib/turnChanges.ts).
  // Skipped on mount (prev === undefined): a restored conversation's old turns
  // must render estimate fallbacks, not today's git numbers as if historical.
  const prevAmbientRef = useRef<string | undefined>(undefined);
  const [changesVersion, setChangesVersion] = useState(0);
  useEffect(() => {
    const state = session?.ambientState;
    const prev = prevAmbientRef.current;
    prevAmbientRef.current = state;
    if (!sessionId || state !== 'idle' || prev === undefined || prev === 'idle') return;
    const base = session?.conversation ?? [];
    let start = base.length;
    while (start > 0 && base[start - 1].role === 'assistant') start--;
    if (start >= base.length) return;
    const edited = collectEditedFiles(base.slice(start).flatMap((t) => t.toolCalls ?? []));
    if (edited.size === 0) return;
    void ensureTurnSnapshot(sessionId, start, effectiveCwd, edited)
      .then(() => setChangesVersion((v) => v + 1))
      .catch(() => {});
  }, [session?.ambientState, session?.conversation, sessionId, effectiveCwd]);

  // Needs-you dock visibility. Dismissal timestamps give an optimistic hide:
  // the dock vanishes on click while the response round-trips through the
  // daemon. New approvals/questions (newer timestamps) re-show it.
  const dockApproval =
    pendingApproval && pendingApproval.timestamp > approvalDismissedAt ? pendingApproval : null;
  const dockQuestions =
    pendingQuestions && pendingQuestions.length > 0 && questionSig !== dismissedQuestionSig
      ? pendingQuestions
      : null;

  // Cancel the current task — send Escape and suppress streaming UI. No-PTY
  // (stream) sessions have no keystroke path; the daemon interrupts the
  // managed adapter on SIGINT instead.
  const cancelTask = useCallback(() => {
    if (hasTerminal) {
      write('\x1b');
    } else if (sessionId) {
      window.electronAPI
        .claudeSignal(sessionId, 'SIGINT')
        .catch((err) => console.warn('[ClaudePane] cancel (SIGINT) failed:', err));
    }
    setCancelledAt(Date.now());
  }, [write, hasTerminal, sessionId]);

  // Decline the pending question(s): leave a "declined" trace in the transcript,
  // hide the picker, and cancel the agent's current turn (the chosen semantics —
  // declining stops the turn rather than answering it).
  const handleDecline = useCallback(() => {
    recordResolved(true, null);
    setDismissedQuestionSig(questionSig);
    cancelTask();
  }, [recordResolved, questionSig, cancelTask]);

  // Restart the session with new launch settings (composer pills). Two spawn
  // ownerships: an attached viewer's session belongs to the agent manager
  // (dispatch, same pattern as library:insert); an owner pane restarts its own
  // spawn in place. Both resume the same pinned id, so the GUI snapshot stream
  // is continuous.
  const handleRestartWith = useCallback(
    (overrides: RestartOverrides) => {
      if (attachSessionId) {
        window.dispatchEvent(
          new CustomEvent('agent:respawn', {
            detail: { sessionId: sessionId ?? attachSessionId, overrides },
          }),
        );
      } else {
        // Carry the current transport so an owner stream session restarts on
        // stream even if the config default changed since it was spawned —
        // claude's from claudeTransport, a hybrid provider's from the
        // daemon-stamped session transport (headless codex restarts headless).
        const transport = isClaude ? claudeTransport : managedStream ? 'stream' : 'pty';
        void restartSession({ ...overrides, provider, transport });
      }
    },
    [
      attachSessionId,
      sessionId,
      restartSession,
      provider,
      claudeTransport,
      isClaude,
      managedStream,
    ],
  );

  // Escape key cancels in GUI mode (must be after cancelTask/isStreaming declarations)
  useEffect(() => {
    if (viewMode !== 'gui' || !isActive) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && isStreaming) {
        e.preventDefault();
        cancelTask();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [viewMode, isActive, isStreaming, cancelTask]);

  const toggleRail = useCallback(() => {
    setRailOpen((open) => {
      localStorage.setItem('wks-claude-rail', open ? '0' : '1');
      return !open;
    });
  }, []);

  // ── Cross-provider handoff ──
  //
  // Two-step: pick the successor's provider, then how the brief gets written —
  // by the source agent itself (best quality: it holds the session in context;
  // takes a turn, mechanical fallback on timeout) or mechanically (instant
  // digest from the conversation). Either way the brief lands under
  // ~/.workspacer/handoffs/ and App spawns the successor with its composer
  // pre-filled to read it. Any harness → any harness.
  const [handoffMenu, setHandoffMenu] = useState<{
    x: number;
    y: number;
    target?: AgentProvider;
  } | null>(null);
  const [handoffBusy, setHandoffBusy] = useState<'agent' | 'mechanical' | null>(null);
  const handleHandoff = useCallback(
    async (target: AgentProvider, kind: 'agent' | 'mechanical') => {
      const sid = sessionId ?? attachSessionId;
      if (!sid || handoffBusy) return;
      setHandoffBusy(kind);
      try {
        const res =
          kind === 'agent'
            ? await window.electronAPI.claudeHandoffAgentBrief(sid)
            : await window.electronAPI.claudeHandoffBrief(sid);
        if (!res.ok || !res.path) {
          console.warn('[ClaudePane] handoff brief failed:', res.error);
          return;
        }
        if ((res as { fallback?: boolean }).fallback) {
          console.warn('[ClaudePane] agent brief fell back to mechanical:', res.error);
        }
        requestHandoff({ targetProvider: target, cwd, briefPath: res.path, sourceSessionId: sid });
      } catch (err) {
        console.warn('[ClaudePane] handoff failed:', err);
      } finally {
        setHandoffBusy(null);
      }
    },
    [sessionId, attachSessionId, cwd, handoffBusy],
  );

  // Inspector-rail hotkey (configurable: keybindings.shortcuts['toggle-inspector']).
  // The rail is per-pane state, so we match the combo here for the active pane
  // rather than routing through the global nav handler. Capture phase + stop
  // beats xterm's own key handling when the pane is in terminal mode.
  const inspectorCombo = config.keybindings?.shortcuts?.['toggle-inspector'];
  const inspectorRailAvailable = uiManifest.inspectorRail;
  useEffect(() => {
    // No-op in focus mode — the rail never mounts there.
    if (!isActive || !inspectorCombo || !inspectorRailAvailable) return;
    const parts = inspectorCombo.toLowerCase().split('+');
    const key = parts[parts.length - 1];
    const needCtrl = parts.includes('ctrl');
    const needAlt = parts.includes('alt');
    const needShift = parts.includes('shift');
    const needMeta = parts.includes('meta');
    const handleKeyDown = (e: KeyboardEvent) => {
      if (
        e.ctrlKey !== needCtrl ||
        e.altKey !== needAlt ||
        e.shiftKey !== needShift ||
        e.metaKey !== needMeta
      )
        return;
      if (e.key.toLowerCase() !== key) return;
      e.preventDefault();
      e.stopPropagation();
      toggleRail();
    };
    window.addEventListener('keydown', handleKeyDown, true);
    return () => window.removeEventListener('keydown', handleKeyDown, true);
  }, [isActive, inspectorCombo, inspectorRailAvailable, toggleRail]);

  // Anchor subagents/workflow runs to the Agent/Workflow tool calls that
  // spawned them so they render inline in the timeline (exact toolUseId /
  // runId joins with order-match fallback — see lib/anchorWork.ts).
  // Running agents/workflows stay pinned in the bottom live log until every
  // run finishes — only completed runs anchor into the timeline's WorkCards
  // (anchoring them while running would scroll live work up into history and
  // render it twice).
  const finishedSubagents = useMemo(
    () => subagents.filter((s) => s.status !== 'running'),
    [subagents],
  );
  const finishedWorkflows = useMemo(
    () => workflows.filter((w) => w.status !== 'running'),
    [workflows],
  );
  const { toolIdToSubagent, toolIdToWorkflow, unanchoredSubagents, unanchoredWorkflows } = useMemo(
    () => anchorWork(conversation, finishedSubagents, finishedWorkflows),
    [conversation, finishedSubagents, finishedWorkflows],
  );
  const liveSubagents = useMemo(
    () => [...subagents.filter((s) => s.status === 'running'), ...unanchoredSubagents],
    [subagents, unanchoredSubagents],
  );
  const liveWorkflows = useMemo(
    () => [...workflows.filter((w) => w.status === 'running'), ...unanchoredWorkflows],
    [workflows, unanchoredWorkflows],
  );

  // Show active + completed tool calls, excluding any already in conversation
  // turns (from JSONL transcript) to avoid duplication while keeping history
  const liveToolCalls = useMemo(() => {
    const conversationToolIds = new Set<string>();
    for (const turn of conversation) {
      if (turn.toolCalls) {
        for (const tc of turn.toolCalls) {
          conversationToolIds.add(tc.id);
        }
      }
    }
    return [...activeToolCalls, ...completedToolCalls].filter(
      (tc) => !conversationToolIds.has(tc.id),
    );
  }, [activeToolCalls, completedToolCalls, conversation]);

  // Auto-scroll: a ResizeObserver on the content column follows EVERY kind of
  // growth — streaming text (which grows an existing bubble without changing
  // any list length, so a dependency-array effect misses it), late-loading
  // diffs, cards expanding — as long as the user is stuck to the bottom
  // (stickToBottomRef, maintained by handleScroll). Scrolling is an instant
  // scrollTop assignment on our own container: no smooth animation to race
  // the near-bottom check, and (unlike scrollIntoView) it can't yank ancestor
  // scrollers to this tab while another one is active.
  useEffect(() => {
    if (viewMode !== 'gui') return;
    const container = scrollContainerRef.current;
    const content = container?.firstElementChild;
    if (!container || !content) return;
    const ro = new ResizeObserver(() => {
      if (!stickToBottomRef.current) return;
      container.scrollTop = container.scrollHeight;
    });
    ro.observe(content);
    return () => ro.disconnect();
  }, [viewMode]);

  // Which work-log surface renders a run of tool calls: prose summary cards,
  // or the waterfall trace monitor (see ToolTraceCard). Same props either way.
  const WorkView = config.claude?.workLog === 'trace' ? ToolTraceCard : WorkCard;

  // Build rendered conversation with dividers (windowed to last visibleCount
  // turns). Consecutive tool-call turns collapse into one WorkCard so the
  // timeline reads as: user said → Claude worked → Claude said.
  const renderedConversation = useMemo(() => {
    const items: React.ReactNode[] = [];
    const startIdx = Math.max(0, conversation.length - visibleCount);
    const visibleTurns = conversation.slice(startIdx);
    // Seed prevRole from turn before the window so the first divider renders correctly
    let prevRole: string | null = startIdx > 0 ? conversation[startIdx - 1].role : null;

    // Resolved-question cards, bucketed by the conversation index they anchor
    // to (clamped into the visible window). Emitted right before the turn at
    // that index — or at the very end for ones answered at the current tail.
    const cardsByAnchor = new Map<number, ResolvedQuestionRecord[]>();
    for (const r of resolvedQuestions) {
      const a = Math.max(startIdx, Math.min(r.anchorLen, conversation.length));
      const arr = cardsByAnchor.get(a);
      if (arr) arr.push(r);
      else cardsByAnchor.set(a, [r]);
    }

    let pendingWork: { calls: ToolCall[]; keyStart: number; endIdx: number } | null = null;
    const workCardIdxs: number[] = []; // positions of WorkCards in `items`
    const flushWork = () => {
      if (!pendingWork) return;
      const { calls, keyStart, endIdx } = pendingWork;
      workCardIdxs.push(items.length);
      items.push(
        <WorkView
          key={`work-${keyStart}`}
          toolCalls={calls}
          subagentByToolId={toolIdToSubagent}
          workflowByToolId={toolIdToWorkflow}
          live={isStreaming && endIdx === conversation.length - 1}
          cwd={effectiveCwd}
        />,
      );
      pendingWork = null;
    };

    // Flush any answered-question cards anchored at this index, as their own
    // block between turns (flush pending work first so a card never lands mid-card).
    const emitCards = (anchor: number) => {
      const rs = cardsByAnchor.get(anchor);
      if (!rs) return;
      flushWork();
      rs.forEach((r, k) =>
        items.push(<AnsweredQuestionCard key={`aq-${anchor}-${k}`} record={r} />),
      );
    };

    // A "turn group" spans the assistant turns between two user messages —
    // its tool calls feed the end-of-turn ChangedFilesCard. Keyed by the
    // group's first assistant turn (global index), matching the capture
    // effect above so the frozen git snapshot is found; without one (app
    // restart, cache eviction) the card degrades to tool-input estimates.
    let group: { start: number; calls: ToolCall[] } | null = null;
    const closeGroup = (complete: boolean) => {
      if (!group) return;
      const g = group;
      group = null;
      if (!complete) return;
      const edited = collectEditedFiles(g.calls);
      if (edited.size === 0) return;
      const snap = sessionId ? getTurnSnapshot(sessionId, g.start) : undefined;
      items.push(
        <ChangedFilesCard
          key={`chg-${g.start}`}
          snapshot={snap ?? estimateSnapshot(edited, effectiveCwd)}
          cwd={effectiveCwd}
        />,
      );
    };

    visibleTurns.forEach((turn, vi) => {
      const gi = startIdx + vi; // global index for stable keys
      emitCards(gi);
      const calls = turn.toolCalls ?? [];

      if (turn.role === 'user') {
        flushWork();
        closeGroup(true);
        if (gi > 0) items.push(<TurnDivider key={`div-${gi}`} label={null} />);
        // Slash-command runs get their command card (invocation chip +
        // collapsible local output) instead of a plain text bubble.
        if (turn.command)
          items.push(<CommandCard key={`msg-${gi}`} turn={turn} showTimestamp={showTimestamps} />);
        else
          items.push(
            <ConversationMessage key={`msg-${gi}`} turn={turn} showTimestamp={showTimestamps} />,
          );
        prevRole = 'user';
        return;
      }

      if (!group) {
        // Walk back past the window edge so a partially-visible group still
        // keys on its true first assistant turn (where the snapshot lives).
        let gs = gi;
        while (gs > 0 && conversation[gs - 1].role === 'assistant') gs--;
        group = { start: gs, calls: [] };
      }
      group.calls.push(...calls);

      // Assistant turn. Text introduces the work that follows it, so close any
      // prior work card above the message, render the text, then open a fresh
      // card seeded with this turn's tool calls. Consecutive text-less tool
      // turns keep appending to that same card — so a run of tool calls always
      // reads as one collapsible card between two things Claude said, never a
      // flat flood of rows trailing the message.
      if (turn.content) {
        flushWork();
        if (prevRole === 'user' && gi > 0) items.push(<TurnDivider key={`div-${gi}`} />);
        items.push(
          <ConversationMessage key={`msg-${gi}`} turn={turn} showTimestamp={showTimestamps} />,
        );
        if (calls.length > 0) pendingWork = { calls: [...calls], keyStart: gi, endIdx: gi };
      } else if (calls.length > 0) {
        if (!pendingWork) {
          if (prevRole === 'user' && gi > 0) items.push(<TurnDivider key={`div-${gi}`} />);
          pendingWork = { calls: [], keyStart: gi, endIdx: gi };
        }
        pendingWork.calls.push(...calls);
        pendingWork.endIdx = gi;
      }
      prevRole = 'assistant';
    });
    flushWork();
    // The trailing group only gets its card once the turn actually ended —
    // idle, not merely "not streaming" (waiting_approval is mid-turn).
    closeGroup(ambientIdle);
    // Questions answered at the current tail (no turns have arrived after).
    emitCards(conversation.length);

    // Keep the most recent work card expanded after work ends, so the latest
    // step stays open without a click. Older cards collapse as usual.
    if (workCardIdxs.length > 0) {
      const lastIdx = workCardIdxs[workCardIdxs.length - 1];
      items[lastIdx] = React.cloneElement(items[lastIdx] as React.ReactElement, { isLast: true });
    }

    return items;
    // changesVersion re-renders cards once a frozen git snapshot lands.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    conversation,
    resolvedQuestions,
    visibleCount,
    toolIdToSubagent,
    toolIdToWorkflow,
    isStreaming,
    ambientIdle,
    sessionId,
    effectiveCwd,
    changesVersion,
    WorkView,
    showTimestamps,
  ]);

  return (
    <div
      style={{
        width: '100%',
        height: '100%',
        display: 'flex',
        flexDirection: 'column',
        backgroundColor: colors.bg,
        color: colors.text,
        fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif',
      }}
    >
      {/* Content + inspector rail row — the rail is a sibling of the content
          area (not nested in the GUI view) so it stays put across GUI/Term. */}
      <div style={{ flex: 1, display: 'flex', flexDirection: 'row', overflow: 'hidden' }}>
        <div
          ref={contentAreaRef}
          style={{ flex: 1, minWidth: 0, overflow: 'hidden', position: 'relative' }}
        >
          {isDragOver && <DropOverlay />}

          {/* Terminal view (always mounted, visibility toggled) */}
          <div
            ref={termContainerRef}
            style={{
              position: 'absolute',
              inset: 0,
              display: viewMode === 'terminal' ? 'block' : 'none',
            }}
          />

          {/* GUI view — always mounted; visibility toggled via CSS so scroll
            position, visibleCount, and optimisticMessages survive GUI↔Term. */}
          <div
            style={
              {
                height: '100%',
                display: viewMode === 'gui' ? 'flex' : 'none',
                flexDirection: 'column',
                overflow: 'hidden',
                // Drives the conversation/markdown font scaling (see ConversationMessage
                // + markdown.tsx). Defaults to 1 elsewhere, so the shared markdown
                // renderer (Library, etc.) is unaffected.
                ['--claude-gui-font-scale' as string]: config.ui.guiFontScale ?? 1.15,
              } as React.CSSProperties
            }
          >
            {/* Conversation scroll area */}
            <div
              ref={scrollContainerRef}
              onScroll={handleScroll}
              style={{
                flex: 1,
                overflowY: 'auto',
                padding: '12px 16px',
                position: 'relative',
                // Promote to its own compositor layer so streaming/markdown
                // repaints don't corrupt the backdrop-filter snapshots of the
                // surrounding glass (transient garble that cleared on repaint).
                transform: 'translateZ(0)',
                contain: 'paint',
              }}
            >
              {/* Centered content container */}
              <div
                style={{
                  maxWidth: 1040,
                  margin: '0 auto',
                }}
              >
                {/* Empty states */}
                {conversation.length === 0 && !session && spawnError && (
                  <div
                    style={{
                      position: 'relative',
                      textAlign: 'center',
                      marginTop: 48,
                      color: colors.mutedDim,
                      animation: 'claudeFadeIn 0.2s ease-out',
                    }}
                  >
                    <AgentHero
                      provider={provider ?? 'claude'}
                      dimLogo
                      title={`Couldn’t start ${agentName}`}
                      titleColor={colors.error}
                    />
                    <div
                      style={{
                        position: 'relative',
                        fontSize: '0.72rem',
                        margin: '8px auto 0',
                        maxWidth: 420,
                        lineHeight: 1.5,
                        color: colors.mutedDim,
                      }}
                    >
                      {spawnError.message || `The ${agentName} session failed to start.`}
                    </div>
                    <button
                      onClick={retry}
                      style={{
                        position: 'relative',
                        marginTop: 16,
                        fontSize: '0.7rem',
                        fontWeight: 600,
                        padding: '4px 16px',
                        borderRadius: 6,
                        border: `1px solid ${colors.accent}`,
                        backgroundColor: 'transparent',
                        color: colors.accent,
                        cursor: 'pointer',
                        fontFamily: 'inherit',
                      }}
                    >
                      Retry
                    </button>
                  </div>
                )}

                {conversation.length === 0 && !session && !spawnError && (
                  <div
                    style={{
                      position: 'relative',
                      textAlign: 'center',
                      marginTop: 48,
                      color: colors.mutedDim,
                      animation: 'claudeFadeIn 0.2s ease-out',
                    }}
                  >
                    <AgentHero
                      provider={provider ?? 'claude'}
                      title={<>Connecting to {agentName}…</>}
                    />
                    <div
                      style={{
                        position: 'relative',
                        display: 'flex',
                        justifyContent: 'center',
                        marginTop: 18,
                      }}
                    >
                      <BrandSpinner size={20} />
                    </div>
                    {showHookHint && isClaude && (
                      <div
                        style={{
                          position: 'relative',
                          fontSize: '0.7rem',
                          marginTop: 14,
                          color: colors.mutedDim,
                        }}
                      >
                        Still connecting — make sure hooks are configured in ~/.claude/settings.json
                      </div>
                    )}
                  </div>
                )}

                {/* Session restore in flight — the transcript replay is coming.
                    Same hero treatment as the "Connecting…" state above, so a
                    restore reads as one continuous sequence (connecting →
                    fetching → transcript) instead of the new-agent screen
                    flashing and the history popping into existence. */}
                {conversation.length === 0 && session && historyPending && (
                  <div
                    style={{
                      position: 'relative',
                      textAlign: 'center',
                      marginTop: 48,
                      color: colors.mutedDim,
                      animation: 'claudeFadeIn 0.2s ease-out',
                    }}
                  >
                    <AgentHero provider={provider ?? 'claude'} title={<>Fetching session…</>} />
                    <div
                      style={{
                        position: 'relative',
                        display: 'flex',
                        justifyContent: 'center',
                        marginTop: 18,
                      }}
                    >
                      <BrandSpinner size={20} />
                    </div>
                    <div
                      style={{
                        position: 'relative',
                        fontSize: '0.7rem',
                        marginTop: 14,
                        color: colors.mutedDim,
                      }}
                    >
                      Restoring your conversation history
                    </div>
                  </div>
                )}

                {conversation.length === 0 && session && !historyPending && (
                  <ConversationEmptyState
                    agentName={agentName}
                    provider={provider ?? 'claude'}
                    model={session.statusLine?.modelDisplay ?? session.settings?.model}
                    permissionMode={permissionModeLabel(
                      provider,
                      session.livePermissionMode ?? session.settings?.permissionMode,
                    )}
                    transport={claudeTransport}
                    cwd={session.liveCwd || session.cwd || cwd}
                    initialPrompt={initialPrompt}
                    onPick={(prompt) => {
                      setInputValue(prompt);
                      requestAnimationFrame(() => inputRef.current?.focus());
                    }}
                  />
                )}

                {/* Load older messages */}
                {hasOlderMessages && (
                  <div style={{ textAlign: 'center', padding: '8px 0 12px 0' }}>
                    <button
                      onClick={loadOlderMessages}
                      style={{
                        fontSize: '0.68rem',
                        fontWeight: 500,
                        padding: '4px 16px',
                        borderRadius: 'var(--wks-radius-lg)',
                        border: `1px solid ${colors.border}`,
                        backgroundColor: 'rgba(255,255,255,0.03)',
                        color: colors.muted,
                        cursor: 'pointer',
                        fontFamily: 'inherit',
                      }}
                    >
                      Load {Math.min(CONVERSATION_PAGE_SIZE, conversation.length - visibleCount)}{' '}
                      earlier messages ({conversation.length - visibleCount} hidden)
                    </button>
                  </div>
                )}

                {/* Rendered conversation messages with dividers */}
                <ErrorBoundary label="Conversation" resetKeys={[sessionId]}>
                  {renderedConversation}
                </ErrorBoundary>

                {/* Live work not yet absorbed into the timeline: in-flight tool
                    calls plus agents/workflows that hooks reported before the
                    transcript caught up. Anchored agents render in WorkCards. */}
                {(liveToolCalls.length > 0 ||
                  liveSubagents.length > 0 ||
                  liveWorkflows.length > 0) && (
                  <InlineWorkLog
                    toolCalls={liveToolCalls}
                    subagents={liveSubagents}
                    workflows={liveWorkflows}
                  />
                )}

                {/* Streaming indicator with cancel */}
                {isStreaming && (
                  <div
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      gap: 10,
                      padding: '8px 0 4px 0',
                    }}
                  >
                    <BrandSpinner size={15} />
                    <button
                      onClick={cancelTask}
                      className="wks-stop-btn"
                      title="Cancel (Esc)"
                      aria-label="Cancel"
                    >
                      <span className="wks-stop-square" />
                      <span className="wks-stop-hint">esc to stop</span>
                    </button>
                  </div>
                )}

                <div ref={conversationEndRef} />
              </div>
            </div>

            {/* Scroll to bottom button */}
            {showScrollBtn && <ScrollToBottomButton onClick={scrollToBottom} />}

            {/* Task list — the agent's plan/tasks pinned above the composer,
                view-only and dismissible (reappears when the tasks change). */}
            {showTasksCard && plan && (
              <TasksCard plan={plan} onDismiss={() => setDismissedPlanSig(planSig)} />
            )}

            {/* Needs-you dock — approvals and questions pinned above the composer */}
            <NeedsYouDock
              approval={dockApproval}
              questions={dockQuestions}
              onApprove={handleApprovalRespond}
              onAnswer={handleAnswer}
              onDecline={handleDecline}
            />

            {/* Composer / Input area — session pills live inside its bottom row */}
            <Composer
              value={inputValue}
              onChange={setInputValue}
              onSend={handleSend}
              onPaste={handlePaste}
              onPickFiles={openFilePicker}
              attachedFiles={attachedFiles}
              onRemoveFile={removeAttachedFile}
              dimmed={!!(dockApproval || dockQuestions)}
              inputRef={inputRef}
              showSendButton={config.ui.showComposerSend !== false}
              agentName={agentName}
              slashItems={slashItems}
              onSlashPick={handleSlashPick}
              controls={
                <ComposerControls
                  provider={provider ?? 'claude'}
                  sessionId={sessionId}
                  snapshot={session}
                  cwd={cwd}
                  onRestartWith={handleRestartWith}
                />
              }
            />
          </div>
          {/* Status / control bar — bottom of the CONTENT column (not the pane),
            so it shares the composer's width and stays centered under it even
            when the inspector rail is open; the rail runs full-height beside
            it. IDE/CLI status-line style: chromeless in GUI mode (a quiet
            footer under the floating composer); terminal mode keeps the solid
            toolbar treatment so it reads as an edge against the xterm surface. */}
          <div
            style={{
              padding: viewMode === 'gui' ? '2px 18px 8px' : '4px 12px',
              backgroundColor: viewMode === 'gui' ? 'transparent' : colors.bgToolbar,
              borderTop: viewMode === 'gui' ? 'none' : `1px solid ${colors.border}`,
              minHeight: 28,
              flexShrink: 0,
            }}
          >
            {/* In GUI mode the row aligns to the composer's centered 1040px column
            so the footer line sits flush under it; terminal mode stays
            edge-to-edge like a toolbar. */}
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 10,
                minWidth: 0,
                minHeight: 24,
                ...(viewMode === 'gui' ? { maxWidth: 1040, margin: '0 auto' } : {}),
              }}
            >
              <StatusBadge
                session={session}
                approvalDismissed={
                  !!(pendingApproval && pendingApproval.timestamp <= approvalDismissedAt)
                }
              />

              {/* Session controls — model / effort / permission-mode pills. In GUI
            mode these live inside the composer's bottom row (T3-style); keep
            them here for terminal mode, which has no composer. */}
              {viewMode === 'terminal' && (
                <ComposerControls
                  provider={provider ?? 'claude'}
                  sessionId={sessionId}
                  snapshot={session}
                  cwd={cwd}
                  onRestartWith={handleRestartWith}
                />
              )}

              {/* In-app status line — telemetry only (dir/branch · plan · ctx ·
            tok/cost · quota meters). Controls (model/effort/permissions) live
            in the ComposerControls pills, never here. */}
              <SessionStatusBar snapshot={session} cwd={cwd} />

              {(() => {
                const liveAgents =
                  subagents.filter((s) => s?.status === 'running').length +
                  // `w.agents` is typed as a required array, but a snapshot arriving
                  // over the hub bus (web/remote) can omit it — flatMap would then
                  // fold in `undefined` and the `.filter` below would throw, blanking
                  // the whole pane. Default to [] so a lean bus payload can't crash it.
                  workflows.flatMap((w) => w.agents ?? []).filter((a) => a?.status === 'running')
                    .length;
                return liveAgents > 0 ? (
                  <span
                    style={{
                      fontSize: '0.66rem',
                      fontWeight: 700,
                      fontFamily: 'var(--wks-font-mono, monospace)',
                      padding: '1px 7px',
                      borderRadius: 'var(--wks-radius-pill, 999px)',
                      letterSpacing: '0.03em',
                      color: 'var(--wks-purple, #c084fc)',
                      border:
                        '1px solid color-mix(in srgb, var(--wks-purple, #c084fc) 40%, transparent)',
                      background: 'color-mix(in srgb, var(--wks-purple, #c084fc) 10%, transparent)',
                      whiteSpace: 'nowrap',
                      flexShrink: 0,
                    }}
                  >
                    {liveAgents} subagent{liveAgents !== 1 ? 's' : ''}
                  </span>
                ) : null;
              })()}

              {/* Attached-files readout — terminal mode only; in GUI the composer
            already shows the attachments as chips, so this would duplicate. */}
              {viewMode === 'terminal' && attachedFiles.length > 0 && (
                <span
                  style={{
                    fontSize: '0.7rem',
                    fontFamily: 'var(--wks-font-mono, monospace)',
                    color: colors.accent,
                    whiteSpace: 'nowrap',
                  }}
                >
                  {attachedFiles.length} file{attachedFiles.length !== 1 ? 's' : ''} attached
                </span>
              )}

              <div style={{ flex: 1 }} />

              {/* Redraw — clears the rare backdrop-filter compositing garble */}
              <button
                onClick={forceRepaint}
                title="Redraw pane (fixes occasional rendering glitches)"
                className="wks-composer-icon-btn"
                style={{
                  ...toggleBtnStyle,
                  display: 'flex',
                  alignItems: 'center',
                  backgroundColor: 'transparent',
                  color: 'var(--wks-text-muted)',
                }}
              >
                <RefreshCw size={13} strokeWidth={1.9} />
              </button>

              {/* Attach files — terminal mode only; the composer has its own + in GUI */}
              {viewMode === 'terminal' && (
                <button
                  onClick={openFilePicker}
                  title="Attach files"
                  className="wks-composer-icon-btn"
                  style={{
                    ...toggleBtnStyle,
                    backgroundColor: 'transparent',
                    color: 'var(--wks-text-muted)',
                    fontSize: '0.8rem',
                  }}
                >
                  +
                </button>
              )}

              {/* Hand off to any provider (including the same one — fresh context,
            same harness) — brief goes to ~/.workspacer/handoffs */}
              <button
                onClick={(e) => {
                  const rect = e.currentTarget.getBoundingClientRect();
                  setHandoffMenu({ x: rect.left, y: rect.top - 4 });
                }}
                title={
                  handoffBusy === 'agent'
                    ? 'Waiting for the agent to write its handoff brief…'
                    : 'Hand off this session to a new agent — any provider, including this one (summarized brief, new session)'
                }
                className="wks-composer-icon-btn"
                disabled={!!handoffBusy || !(sessionId ?? attachSessionId)}
                style={{
                  ...toggleBtnStyle,
                  display: 'flex',
                  alignItems: 'center',
                  backgroundColor: 'transparent',
                  color: handoffBusy ? colors.accent : 'var(--wks-text-muted)',
                }}
              >
                <ArrowRightLeft size={13} strokeWidth={1.9} />
              </button>
              {handoffMenu && !handoffMenu.target && (
                <ContextMenu
                  x={handoffMenu.x}
                  y={handoffMenu.y}
                  onClose={() => setHandoffMenu(null)}
                  minWidth={170}
                >
                  <ContextMenuLabel>Hand off to…</ContextMenuLabel>
                  {(
                    [
                      ['claude', 'Claude'],
                      ['codex', 'Codex'],
                      ['opencode', 'OpenCode'],
                      ['pi', 'Pi'],
                    ] as Array<[AgentProvider, string]>
                  ).map(([id, label]) => (
                    <ContextMenuItem
                      key={id}
                      label={label}
                      onClick={() => setHandoffMenu((m) => (m ? { ...m, target: id } : m))}
                    />
                  ))}
                </ContextMenu>
              )}
              {handoffMenu?.target && (
                <ContextMenu
                  x={handoffMenu.x}
                  y={handoffMenu.y}
                  onClose={() => setHandoffMenu(null)}
                  minWidth={230}
                >
                  <ContextMenuLabel>Brief for {handoffMenu.target} — written by…</ContextMenuLabel>
                  <ContextMenuItem
                    label="This agent (best, takes a turn)"
                    onClick={() => {
                      const target = handoffMenu.target!;
                      setHandoffMenu(null);
                      void handleHandoff(target, 'agent');
                    }}
                  />
                  <ContextMenuItem
                    label="Mechanical digest (instant)"
                    onClick={() => {
                      const target = handoffMenu.target!;
                      setHandoffMenu(null);
                      void handleHandoff(target, 'mechanical');
                    }}
                  />
                </ContextMenu>
              )}

              {/* Timestamps toggle — GUI conversation only. Saved to config so it
            persists and applies to every chat pane at once. */}
              {viewMode === 'gui' && (
                <button
                  onClick={() =>
                    save({ claude: { ...config.claude, showTimestamps: !showTimestamps } } as any)
                  }
                  title={showTimestamps ? 'Hide message timestamps' : 'Show message timestamps'}
                  className={showTimestamps ? undefined : 'wks-composer-icon-btn'}
                  style={{
                    ...toggleBtnStyle,
                    display: 'flex',
                    alignItems: 'center',
                    backgroundColor: showTimestamps ? 'var(--wks-accent-bg)' : 'transparent',
                    color: showTimestamps ? colors.accent : 'var(--wks-text-muted)',
                  }}
                >
                  <Clock size={13} strokeWidth={1.9} />
                </button>
              )}

              {/* Inspector rail toggle — available in both GUI and Terminal mode,
            hidden in focus mode along with the rail itself. */}
              {inspectorRailAvailable && (
                <button
                  onClick={toggleRail}
                  title={
                    railOpen
                      ? 'Hide inspector'
                      : 'Show inspector (files / workflows / agents / usage)'
                  }
                  className={railOpen ? undefined : 'wks-composer-icon-btn'}
                  style={{
                    ...toggleBtnStyle,
                    display: 'flex',
                    alignItems: 'center',
                    backgroundColor: railOpen ? 'var(--wks-accent-bg)' : 'transparent',
                    color: railOpen ? colors.accent : 'var(--wks-text-muted)',
                  }}
                >
                  <PanelRight size={13} strokeWidth={1.9} />
                </button>
              )}

              {/* View mode toggle — only when the provider offers both surfaces (Claude). */}
              <div style={{ display: showViewToggle ? 'flex' : 'none', gap: 2 }}>
                <button
                  onClick={() => setViewMode('gui')}
                  className={viewMode === 'gui' ? undefined : 'wks-composer-icon-btn'}
                  style={{
                    ...toggleBtnStyle,
                    backgroundColor: viewMode === 'gui' ? 'var(--wks-accent-bg)' : 'transparent',
                    color: viewMode === 'gui' ? colors.accent : 'var(--wks-text-muted)',
                  }}
                >
                  GUI
                </button>
                <button
                  onClick={() => setViewMode('terminal')}
                  className={viewMode === 'terminal' ? undefined : 'wks-composer-icon-btn'}
                  style={{
                    ...toggleBtnStyle,
                    backgroundColor:
                      viewMode === 'terminal' ? 'var(--wks-accent-bg)' : 'transparent',
                    color: viewMode === 'terminal' ? colors.accent : 'var(--wks-text-muted)',
                  }}
                >
                  Term
                </button>
              </div>
            </div>
          </div>
        </div>

        {/* Inspector rail — files / workflows / agents / usage. Sibling of the
          content area, so it persists in both GUI and Terminal mode. Never
          mounts in focus mode (UI-mode manifest). */}
        {inspectorRailAvailable && railOpen && (
          <InspectorRail session={session} onClose={toggleRail} />
        )}
      </div>
    </div>
  );
};

const toggleBtnStyle: React.CSSProperties = {
  fontSize: '0.66rem',
  fontWeight: 600,
  padding: '3px 9px',
  borderRadius: 6,
  border: 'none',
  cursor: 'pointer',
};

export default ClaudePane;
