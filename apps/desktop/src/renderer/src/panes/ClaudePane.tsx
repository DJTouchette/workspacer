import React, { useEffect, useRef, useCallback, useState, useMemo } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebFontsAddon } from '@xterm/addon-web-fonts';
import '@xterm/xterm/css/xterm.css';
import { useClaudeSpawn } from '../hooks/useClaudeSpawn';
import { useClaudeSession } from '../hooks/useClaudeSession';
import { useConfig } from '../hooks/useConfig';
import { useTheme } from '../hooks/useTheme';
import type { ConversationTurn, ToolCall, SubagentInfo, WorkflowRunInfo } from '../types/claudeSession';
import {
  claudeColors as colors,
  ensureKeyframes,
  StatusBadge,
  StreamingDots,
  sendApproval,
} from '../components/claude-shared';
import { RefreshCw } from '../components/icons';
import { PanelRight } from 'lucide-react';
import { quoteFontFamily } from '../lib/terminalUtils';

// ── Sub-components ──
import { InlineWorkLog } from '../components/claude/InlineWorkLog';
import { ConversationMessage } from '../components/claude/ConversationMessage';
import { TurnDivider } from '../components/claude/TurnDivider';
import { NeedsYouDock } from '../components/claude/NeedsYouDock';
import { Composer } from '../components/claude/Composer';
import { WorkCard } from '../components/claude/WorkCard';
import { InspectorRail } from '../components/claude/InspectorRail';
import { DropOverlay } from '../components/claude/DropOverlay';
import { ScrollToBottomButton } from '../components/claude/ScrollToBottomButton';
import { SessionStatusBar } from '../components/claude/SessionStatusBar';
import { classifyFile, buildPromptPrefix, extractFilePaths } from '../components/claude/fileAttachment';
import type { AttachedFile } from '../components/claude/fileAttachment';

interface ClaudePaneProps {
  paneId: string;
  title: string;
  isActive: boolean;
  cwd?: string;
  profileId?: string;
  resumeSessionId?: string;
  /** If set, this pane is a viewer for an already-running daemon session. */
  attachSessionId?: string;
  /** Text to seed the message input with on first mount (library spawn). */
  initialPrompt?: string;
  onPtyReady?: (paneId: string, ptySessionId: string) => void;
}

type ViewMode = 'gui' | 'terminal';

/** Number of conversation turns rendered per page (oldest load on scroll-up) */
const CONVERSATION_PAGE_SIZE = 60;

// ── Main component ──

const ClaudePane: React.FC<ClaudePaneProps> = ({ paneId, title, isActive, cwd, profileId, resumeSessionId, attachSessionId, initialPrompt, onPtyReady }) => {
  const { config } = useConfig();
  // A spawned-with-prompt pane always opens in GUI; otherwise honour the
  // configured default view (falls back to terminal until config loads).
  const [viewMode, setViewMode] = useState<ViewMode>(
    initialPrompt ? 'gui' : (config.claude?.defaultView ?? 'terminal'),
  );
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
  const conversationEndRef = useRef<HTMLDivElement>(null);
  const scrollContainerRef = useRef<HTMLDivElement>(null);
  const contentAreaRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  // Escape hatch for the rare backdrop-filter compositing garble: nudge the
  // content area onto a fresh raster (toggle a composited property for one
  // frame). Clears stale pixels without resetting scroll position or the PTY.
  const forceRepaint = useCallback(() => {
    const el = contentAreaRef.current;
    if (!el) return;
    el.style.transform = 'translateZ(0)';
    el.style.opacity = '0.999';
    requestAnimationFrame(() => {
      if (!el) return;
      el.style.transform = '';
      el.style.opacity = '';
    });
  }, []);

  const { terminalTheme } = useTheme();
  const termCfg = config.terminal;

  // Inject keyframes
  useEffect(() => { ensureKeyframes(); }, []);

  // Set CSS variable for mono font
  useEffect(() => {
    if (typeof document !== 'undefined') {
      document.documentElement.style.setProperty('--claude-mono-font', termCfg.fontFamily || 'monospace');
    }
  }, [termCfg.fontFamily]);

  const handleExit = useCallback(() => {
    if (terminalRef.current) {
      terminalRef.current.write('\r\n\x1b[90m[Claude session exited]\x1b[0m\r\n');
    }
  }, []);

  const { sessionId, isReady, write, resize, attachToTerminal, startSession } = useClaudeSpawn({
    paneId,
    cwd,
    profileId,
    resumeSessionId,
    attachSessionId,
    onExit: handleExit,
    defer: true,
  });

  const { session } = useClaudeSession({ ptySessionId: sessionId, active: isActive });

  // Enable the approval gateway in claudemon as soon as we have a session id
  // so PreToolUse hooks get parked for our UI to resolve.
  useEffect(() => {
    if (!sessionId) return;
    window.electronAPI.claudeGate(sessionId, true).catch(err =>
      console.warn('[ClaudePane] failed to enable approval gate:', err)
    );
  }, [sessionId]);

  // Notify parent of PTY session ID
  useEffect(() => {
    if (sessionId && onPtyReady) {
      onPtyReady(paneId, sessionId);
    }
  }, [sessionId, paneId, onPtyReady]);

  // Library: receive a prompt/skill inserted from the library. Targeted by
  // sessionId/paneId, or delivered to the active pane when untargeted.
  useEffect(() => {
    const handler = (e: Event) => {
      const d = (e as CustomEvent).detail as { text?: string; sessionId?: string; paneId?: string } | undefined;
      if (!d?.text) return;
      const targeted = d.sessionId || d.paneId;
      const matches = targeted ? (d.sessionId === sessionId || d.paneId === paneId) : isActive;
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
      try { fitAddon.fit(); } catch {}
      startSession(term.cols, term.rows);
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
        if (sel) { e.preventDefault(); navigator.clipboard.writeText(sel); term.clearSelection(); return false; }
        return true;
      }
      // Ctrl+V — paste. Handled by xterm's native paste event (single insert,
      // bracketed-paste aware); return false so xterm doesn't also emit ^V.
      // Manual clipboard.readText + write here caused a double paste, because
      // preventDefault on keydown does not stop the browser's native paste event.
      if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key === 'v') {
        return false;
      }
      if (e.ctrlKey && !e.altKey && !e.shiftKey && ['t', 'b', 'w', 'd', '/', '?', ',', 's', 'k'].includes(e.key)) return false;
      if (e.ctrlKey && !e.altKey && !e.shiftKey && /^[1-9]$/.test(e.key)) return false;
      if (e.altKey && !e.ctrlKey && ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(e.key)) return false;
      if (e.ctrlKey && e.altKey && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) return false;
      if (e.ctrlKey && e.shiftKey) return false;
      if (e.key === 'F2') return false;
      return true;
    });

    const fitRetry = () => { try { fitAddon.fit(); } catch {} };
    requestAnimationFrame(fitRetry);
    setTimeout(fitRetry, 100);
    setTimeout(fitRetry, 300);

    attachToTerminal(term);

    const onDataDisp = term.onData((data) => write(data));
    const onBinaryDisp = term.onBinary((data) => write(data));

    const observer = new ResizeObserver(() => {
      requestAnimationFrame(() => { try { fitAddonRef.current?.fit(); } catch {} });
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
    };
  }, [attachToTerminal, write, resize]);

  // Focus terminal or GUI input when pane becomes active
  useEffect(() => {
    if (!isActive) return;
    if (viewMode === 'terminal' && terminalRef.current) {
      terminalRef.current.focus();
      requestAnimationFrame(() => { try { fitAddonRef.current?.fit(); } catch {} });
    } else if (viewMode === 'gui') {
      requestAnimationFrame(() => inputRef.current?.focus());
    }
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

  // Re-fit terminal on active change
  useEffect(() => {
    if (isActive && viewMode === 'terminal' && terminalRef.current) {
      terminalRef.current.focus();
      requestAnimationFrame(() => {
        try {
          fitAddonRef.current?.fit();
          const t = terminalRef.current;
          if (t) resize(t.cols, t.rows);
        } catch {}
      });
    }
  }, [isActive, viewMode, resize]);

  // Update terminal theme when it changes
  useEffect(() => {
    if (terminalRef.current) {
      terminalRef.current.options.theme = terminalTheme;
    }
  }, [terminalTheme]);

  // Track scroll position for "scroll to bottom" button + lazy load older messages
  const handleScroll = useCallback(() => {
    const container = scrollContainerRef.current;
    if (!container) return;
    const distFromBottom = container.scrollHeight - container.scrollTop - container.clientHeight;
    setShowScrollBtn(distFromBottom > 150);
  }, []);

  const loadOlderMessages = useCallback(() => {
    const container = scrollContainerRef.current;
    const prevHeight = container?.scrollHeight ?? 0;
    setVisibleCount(prev => prev + CONVERSATION_PAGE_SIZE);
    // Preserve scroll position after DOM grows upward
    requestAnimationFrame(() => {
      if (container) {
        const newHeight = container.scrollHeight;
        container.scrollTop += newHeight - prevHeight;
      }
    });
  }, []);

  const scrollToBottom = useCallback(() => {
    conversationEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, []);

  // ── File drag & drop ──

  // Global drag & drop — document + window level with dropEffect to tell
  // Electron/Chromium this is a valid drop target (prevents 🚫 cursor)
  useEffect(() => {
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
          setAttachedFiles(prev => [...prev, ...paths.map(classifyFile)]);
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
  }, []);

  const handlePaste = useCallback((e: React.ClipboardEvent) => {
    const paths = extractFilePaths(e.clipboardData);
    if (paths.length > 0) {
      e.preventDefault();
      setAttachedFiles(prev => [...prev, ...paths.map(classifyFile)]);
    }
  }, []);

  const removeAttachedFile = useCallback((idx: number) => {
    setAttachedFiles(prev => prev.filter((_, i) => i !== idx));
  }, []);

  const openFilePicker = useCallback(async () => {
    const paths = await window.electronAPI.pickFiles(cwd);
    if (paths.length > 0) {
      setAttachedFiles(prev => [...prev, ...paths.map(classifyFile)]);
      if (viewMode === 'terminal') setViewMode('gui');
    }
  }, [cwd, viewMode]);

  const handleApprovalRespond = useCallback((response: 'yes' | 'no') => {
    if (!sessionId) return;
    // If a question picker is also pending (PermissionRequest racing with
    // AskUserQuestion's PreToolUse), the approval card is stale and shouldn't
    // do anything — the user actually wants to answer the picker. Writing a
    // keystroke fallback would select option 1 of the picker by accident.
    const hasPendingQuestion = (session?.pendingQuestions?.length ?? 0) > 0;
    window.electronAPI.claudeApprove(sessionId, response).catch(err => {
      console.warn('[ClaudePane] /approve failed:', err);
      if (!hasPendingQuestion) {
        sendApproval('', response === 'yes', write);
      } else {
        console.warn('[ClaudePane] suppressed keystroke fallback — question picker is active');
      }
    });
    setApprovalDismissedAt(Date.now());
  }, [sessionId, write, session?.pendingQuestions]);

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
    setOptimisticMessages(prev => [...prev, {
      role: 'user',
      content: fullMessage,
      timestamp: Date.now(),
    }]);
    setOptimisticLoading(true);

    const rawFallback = () => {
      // Last resort: type the text and submit with a slightly longer delay so
      // the \r doesn't race a mid-flight redraw.
      write(fullMessage);
      setTimeout(() => write('\r'), 80);
    };

    if (!sessionId) {
      rawFallback();
      return;
    }

    // Prefer claudemon's /message endpoint — it appends \r and sends the whole
    // line atomically through the daemon, which is reliable. It's mode-gated,
    // though: a freshly spawned session sits in `unknown` mode until the
    // SessionStart hook fires (claude's TUI is up and ready). The FIRST message
    // typically arrives during that window, so /message 409s and we'd fall to a
    // raw PTY write whose lone \r races claude's cold-start rendering and gets
    // dropped — the "typed but not sent" bug. Fix: while the daemon still
    // reports the startup `unknown` mode, retry briefly to wait for readiness,
    // then use the atomic path. Other non-input modes (responding / approval /
    // question) won't become input by waiting, so fall back immediately.
    const MAX_WAIT_MS = 4000;
    const RETRY_MS = 200;
    const deadline = Date.now() + MAX_WAIT_MS;
    for (;;) {
      let res: { ok: boolean; mode?: string };
      try {
        res = await window.electronAPI.claudeMessage(sessionId, fullMessage);
      } catch (err) {
        console.warn('[ClaudePane] /message failed:', err);
        rawFallback();
        return;
      }
      if (res.ok) return; // sent cleanly via the daemon
      if (res.mode !== 'unknown' || Date.now() >= deadline) {
        console.warn(`[ClaudePane] /message not accepted (mode=${res.mode}); raw PTY write`);
        rawFallback();
        return;
      }
      await new Promise(r => setTimeout(r, RETRY_MS));
    }
  }, [inputValue, write, attachedFiles, sessionId]);

  // Drop optimistic entries FIFO as session.conversation grows past the
  // count we last consumed. This avoids content-matching pitfalls.
  useEffect(() => {
    const userCount = (session?.conversation ?? []).filter(t => t.role === 'user').length;
    if (userCount > consumedUserCountRef.current) {
      const newlyConsumed = userCount - consumedUserCountRef.current;
      consumedUserCountRef.current = userCount;
      setOptimisticMessages(prev => (newlyConsumed >= prev.length ? [] : prev.slice(newlyConsumed)));
    }
    // Clear optimistic loading when server reports idle or we get a response
    if (optimisticLoading && (session?.ambientState === 'idle' || session?.ambientState === 'streaming')) {
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
  const subagents = session?.subagents ?? [];
  const workflows = session?.workflows ?? [];
  const pendingApproval = session?.pendingApproval ?? null;
  const pendingQuestions = session?.pendingQuestions ?? null;
  // Optimistic dismiss for the question picker — keeps the UI feeling snappy
  // even when /answer 409s and we fall back to a raw PTY write that takes
  // a moment to round-trip through the JSONL transcript.
  const [questionDismissedAt, setQuestionDismissedAt] = useState(0);

  const handleAnswer = useCallback((payload: { option?: number; text?: string; answers?: string[] }) => {
    if (!sessionId) return;
    setQuestionDismissedAt(Date.now());
    // We write directly to the PTY (via the MessagePort → /sessions/:id/input
    // path) instead of /sessions/:id/answer. /answer requires mode=Question,
    // which can race with concurrent hook events that flip the daemon's mode
    // back to Responding/Approval — and the renderer's view of "picker is up"
    // is what actually matters here. claude's own TUI picker accepts numeric
    // input + Enter the same way it accepts any other keystroke.
    if (payload.option !== undefined) {
      write(`${payload.option}\r`);
    } else if (payload.text !== undefined) {
      write(`${payload.text}\r`);
    } else if (payload.answers) {
      for (const ans of payload.answers) write(`${ans}\r`);
    }
  }, [sessionId, write]);
  const serverStreaming = optimisticLoading || session?.ambientState === 'thinking' || session?.ambientState === 'streaming';
  // If user cancelled, suppress streaming UI until a new activity cycle begins
  const isStreaming = serverStreaming && (session?.lastActivity ?? 0) > cancelledAt;

  // Needs-you dock visibility. Dismissal timestamps give an optimistic hide:
  // the dock vanishes on click while the response round-trips through the
  // daemon. New approvals/questions (newer timestamps) re-show it.
  const dockApproval = pendingApproval && pendingApproval.timestamp > approvalDismissedAt ? pendingApproval : null;
  const dockQuestions = pendingQuestions && pendingQuestions.length > 0 && (session?.lastActivity ?? 0) > questionDismissedAt
    ? pendingQuestions
    : null;

  // Cancel the current task — send Escape and suppress streaming UI
  const cancelTask = useCallback(() => {
    write('\x1b');
    setCancelledAt(Date.now());
  }, [write]);

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
    setRailOpen(open => {
      localStorage.setItem('wks-claude-rail', open ? '0' : '1');
      return !open;
    });
  }, []);

  // Inspector-rail hotkey (configurable: keybindings.shortcuts['toggle-inspector']).
  // The rail is per-pane state, so we match the combo here for the active pane
  // rather than routing through the global nav handler. Capture phase + stop
  // beats xterm's own key handling when the pane is in terminal mode.
  const inspectorCombo = config.keybindings?.shortcuts?.['toggle-inspector'];
  useEffect(() => {
    if (!isActive || !inspectorCombo) return;
    const parts = inspectorCombo.toLowerCase().split('+');
    const key = parts[parts.length - 1];
    const needCtrl = parts.includes('ctrl');
    const needAlt = parts.includes('alt');
    const needShift = parts.includes('shift');
    const needMeta = parts.includes('meta');
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.ctrlKey !== needCtrl || e.altKey !== needAlt || e.shiftKey !== needShift || e.metaKey !== needMeta) return;
      if (e.key.toLowerCase() !== key) return;
      e.preventDefault();
      e.stopPropagation();
      toggleRail();
    };
    window.addEventListener('keydown', handleKeyDown, true);
    return () => window.removeEventListener('keydown', handleKeyDown, true);
  }, [isActive, inspectorCombo, toggleRail]);

  // Anchor subagents/workflow runs to the Agent/Workflow tool calls that
  // spawned them so they render inline in the timeline. Hooks and the
  // transcript don't share ids, so match by order: the nth Agent tool call
  // in the conversation pairs with the nth hook-reported subagent (tool_use
  // blocks land in the JSONL before execution starts, so ordering holds).
  // Anything left over (hook arrived before the transcript caught up)
  // renders in the live section at the bottom.
  const { toolIdToSubagent, toolIdToWorkflow, unanchoredSubagents, unanchoredWorkflows } = useMemo(() => {
    const agentCalls: ToolCall[] = [];
    const workflowCalls: ToolCall[] = [];
    for (const turn of conversation) {
      for (const tc of turn.toolCalls ?? []) {
        if (tc.name === 'Agent') agentCalls.push(tc);
        else if (tc.name === 'Workflow') workflowCalls.push(tc);
      }
    }
    const toolIdToSubagent = new Map<string, SubagentInfo>();
    subagents.forEach((sub, i) => {
      if (i < agentCalls.length) toolIdToSubagent.set(agentCalls[i].id, sub);
    });
    const toolIdToWorkflow = new Map<string, WorkflowRunInfo>();
    workflows.forEach((run, i) => {
      if (i < workflowCalls.length) toolIdToWorkflow.set(workflowCalls[i].id, run);
    });
    return {
      toolIdToSubagent,
      toolIdToWorkflow,
      unanchoredSubagents: subagents.slice(agentCalls.length),
      unanchoredWorkflows: workflows.slice(workflowCalls.length),
    };
  }, [conversation, subagents, workflows]);

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
    return [...activeToolCalls, ...completedToolCalls]
      .filter(tc => !conversationToolIds.has(tc.id));
  }, [activeToolCalls, completedToolCalls, conversation]);

  // Auto-scroll conversation to bottom (only when this pane is active —
  // scrollIntoView scrolls all ancestors, which would yank the outer
  // ScrollContainer back to this tab even when viewing another tab)
  useEffect(() => {
    if (!isActive) return;
    if (viewMode !== 'gui') return;
    const container = scrollContainerRef.current;
    if (!container) return;
    // Only auto-scroll if user is near the bottom
    const isNearBottom = container.scrollHeight - container.scrollTop - container.clientHeight < 100;
    if (isNearBottom) {
      conversationEndRef.current?.scrollIntoView({ behavior: 'smooth' });
    }
  }, [
    isActive,
    viewMode,
    session?.conversation?.length,
    session?.activeToolCalls?.length,
    session?.completedToolCalls?.length,
    session?.lastActivity,
    session?.subagents?.length,
    session?.workflows?.length,
    liveToolCalls.length,
    optimisticMessages.length,
    isStreaming,
  ]);

  // Build rendered conversation with dividers (windowed to last visibleCount
  // turns). Consecutive tool-call turns collapse into one WorkCard so the
  // timeline reads as: user said → Claude worked → Claude said.
  const renderedConversation = useMemo(() => {
    const items: React.ReactNode[] = [];
    const startIdx = Math.max(0, conversation.length - visibleCount);
    const visibleTurns = conversation.slice(startIdx);
    // Seed prevRole from turn before the window so the first divider renders correctly
    let prevRole: string | null = startIdx > 0 ? conversation[startIdx - 1].role : null;

    const isToolTurn = (t: ConversationTurn) =>
      t.role === 'assistant' && (t.toolCalls?.length ?? 0) > 0 && !t.content;

    let pendingWork: { calls: ToolCall[]; keyStart: number; endIdx: number } | null = null;
    const flushWork = () => {
      if (!pendingWork) return;
      const { calls, keyStart, endIdx } = pendingWork;
      items.push(
        <WorkCard
          key={`work-${keyStart}`}
          toolCalls={calls}
          subagentByToolId={toolIdToSubagent}
          workflowByToolId={toolIdToWorkflow}
          live={isStreaming && endIdx === conversation.length - 1}
        />
      );
      pendingWork = null;
    };

    visibleTurns.forEach((turn, vi) => {
      const gi = startIdx + vi; // global index for stable keys
      if (isToolTurn(turn)) {
        if (!pendingWork) {
          if (prevRole === 'user' && gi > 0) items.push(<TurnDivider key={`div-${gi}`} />);
          pendingWork = { calls: [], keyStart: gi, endIdx: gi };
        }
        pendingWork.calls.push(...(turn.toolCalls ?? []));
        pendingWork.endIdx = gi;
        prevRole = 'assistant';
        return;
      }
      flushWork();
      if (turn.role === 'assistant' && prevRole === 'user' && gi > 0) {
        items.push(<TurnDivider key={`div-${gi}`} />);
      }
      items.push(
        <ConversationMessage
          key={`msg-${gi}`}
          turn={turn}
          isLast={gi === conversation.length - 1}
        />
      );
      prevRole = turn.role;
    });
    flushWork();

    return items;
  }, [conversation, visibleCount, toolIdToSubagent, toolIdToWorkflow, isStreaming, sessionId]);

  return (
    <div style={{
      width: '100%',
      height: '100%',
      display: 'flex',
      flexDirection: 'column',
      backgroundColor: colors.bg,
      color: colors.text,
      fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif',
    }}>
      {/* Toolbar */}
      <div style={{
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        padding: '3px 10px',
        backgroundColor: colors.bgToolbar,
        borderBottom: `1px solid ${colors.border}`,
        minHeight: 26,
        flexShrink: 0,
      }}>
        <StatusBadge session={session} approvalDismissed={!!(pendingApproval && pendingApproval.timestamp <= approvalDismissedAt)} />

        {/* In-app status line — model · ctx · tok/cost · 5h/7d (replaces the
            old working-timer + directory readouts). */}
        <SessionStatusBar snapshot={session} cwd={cwd} />

        {session && (
          <span style={{ fontSize: '0.55rem', color: colors.mutedDim }}>
            {session.totalToolCalls} tools
          </span>
        )}

        {(() => {
          const liveAgents =
            subagents.filter(s => s.status === 'running').length +
            workflows.flatMap(w => w.agents).filter(a => a.status === 'running').length;
          return liveAgents > 0 ? (
            <span style={{ fontSize: '0.55rem', color: '#c084fc' }}>
              {liveAgents} subagent(s)
            </span>
          ) : null;
        })()}

        {attachedFiles.length > 0 && (
          <span style={{ fontSize: '0.55rem', color: colors.accent }}>
            {attachedFiles.length} file{attachedFiles.length !== 1 ? 's' : ''} attached
          </span>
        )}

        <div style={{ flex: 1 }} />

        {/* Redraw — clears the rare backdrop-filter compositing garble */}
        <button
          onClick={forceRepaint}
          title="Redraw pane (fixes occasional rendering glitches)"
          style={{
            ...toggleBtnStyle,
            display: 'flex',
            alignItems: 'center',
            backgroundColor: 'transparent',
            color: colors.mutedDim,
          }}
        >
          <RefreshCw size={13} strokeWidth={1.9} />
        </button>

        {/* Attach files */}
        <button
          onClick={openFilePicker}
          title="Attach files"
          style={{
            ...toggleBtnStyle,
            backgroundColor: 'transparent',
            color: colors.mutedDim,
            fontSize: '0.7rem',
          }}
        >
          +
        </button>

        {/* Inspector rail toggle — available in both GUI and Terminal mode */}
        <button
          onClick={toggleRail}
          title={railOpen ? 'Hide inspector' : 'Show inspector (files / workflows / agents / usage)'}
          style={{
            ...toggleBtnStyle,
            display: 'flex',
            alignItems: 'center',
            backgroundColor: railOpen ? 'var(--wks-accent-bg)' : 'transparent',
            color: railOpen ? colors.accent : colors.mutedDim,
          }}
        >
          <PanelRight size={13} strokeWidth={1.9} />
        </button>

        {/* View mode toggle */}
        <div style={{ display: 'flex', gap: 2 }}>
          <button
            onClick={() => setViewMode('gui')}
            style={{
              ...toggleBtnStyle,
              backgroundColor: viewMode === 'gui' ? 'var(--wks-accent-bg)' : 'transparent',
              color: viewMode === 'gui' ? colors.accent : colors.mutedDim,
            }}
          >
            GUI
          </button>
          <button
            onClick={() => setViewMode('terminal')}
            style={{
              ...toggleBtnStyle,
              backgroundColor: viewMode === 'terminal' ? 'var(--wks-accent-bg)' : 'transparent',
              color: viewMode === 'terminal' ? colors.accent : colors.mutedDim,
            }}
          >
            Term
          </button>
        </div>
      </div>

      {/* Content + inspector rail row — the rail is a sibling of the content
          area (not nested in the GUI view) so it stays put across GUI/Term. */}
      <div style={{ flex: 1, display: 'flex', flexDirection: 'row', overflow: 'hidden' }}>
      <div ref={contentAreaRef} style={{ flex: 1, minWidth: 0, overflow: 'hidden', position: 'relative' }}>
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

        {/* GUI view */}
        {viewMode === 'gui' && (
          <div style={{
            height: '100%',
            display: 'flex',
            flexDirection: 'column',
            overflow: 'hidden',
          }}>
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
              <div style={{
                maxWidth: 720,
                margin: '0 auto',
              }}>
                {/* Empty states */}
                {conversation.length === 0 && !session && (
                  <div style={{ textAlign: 'center', marginTop: 60, color: colors.mutedDim }}>
                    <div style={{ fontSize: '2rem', marginBottom: 12, opacity: 0.4 }}>{'◆'}</div>
                    <div style={{ fontSize: '0.8rem', color: colors.muted }}>Claude Code session starting...</div>
                    <div style={{ fontSize: '0.7rem', marginTop: 6, color: colors.mutedDim }}>
                      Waiting for hook events. Make sure hooks are configured in ~/.claude/settings.json
                    </div>
                  </div>
                )}

                {conversation.length === 0 && session && (
                  <div style={{ textAlign: 'center', marginTop: 60, color: colors.mutedDim }}>
                    <div style={{ fontSize: '2rem', marginBottom: 12, opacity: 0.4 }}>{'◆'}</div>
                    <div style={{ fontSize: '0.8rem', color: colors.muted }}>Session connected</div>
                    <div style={{ fontSize: '0.7rem', marginTop: 6, color: colors.mutedDim }}>
                      Waiting for conversation activity...
                    </div>
                  </div>
                )}

                {/* Load older messages */}
                {hasOlderMessages && (
                  <div style={{ textAlign: 'center', padding: '8px 0 12px 0' }}>
                    <button
                      onClick={loadOlderMessages}
                      style={{
                        fontSize: '0.65rem',
                        fontWeight: 500,
                        padding: '4px 16px',
                        borderRadius: 12,
                        border: `1px solid ${colors.border}`,
                        backgroundColor: 'rgba(255,255,255,0.03)',
                        color: colors.muted,
                        cursor: 'pointer',
                        fontFamily: 'inherit',
                      }}
                    >
                      Load {Math.min(CONVERSATION_PAGE_SIZE, conversation.length - visibleCount)} earlier messages
                      {' '}({conversation.length - visibleCount} hidden)
                    </button>
                  </div>
                )}

                {/* Rendered conversation messages with dividers */}
                {renderedConversation}

                {/* Live work not yet absorbed into the timeline: in-flight tool
                    calls plus agents/workflows that hooks reported before the
                    transcript caught up. Anchored agents render in WorkCards. */}
                {(liveToolCalls.length > 0 || unanchoredSubagents.length > 0 || unanchoredWorkflows.length > 0) && (
                  <InlineWorkLog toolCalls={liveToolCalls} subagents={unanchoredSubagents} workflows={unanchoredWorkflows} />
                )}

                {/* Streaming indicator with cancel */}
                {isStreaming && (
                  <div style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 10,
                    padding: '8px 0 4px 0',
                  }}>
                    <StreamingDots />
                    <button
                      onClick={cancelTask}
                      style={{
                        fontSize: '0.65rem',
                        fontWeight: 500,
                        padding: '2px 10px',
                        border: `1px solid ${colors.muted}`,
                        borderRadius: 4,
                        backgroundColor: 'transparent',
                        color: colors.muted,
                        cursor: 'pointer',
                        fontFamily: 'inherit',
                      }}
                      title="Cancel (Esc)"
                    >
                      Cancel
                    </button>
                  </div>
                )}

                <div ref={conversationEndRef} />
              </div>
            </div>

            {/* Scroll to bottom button */}
            {showScrollBtn && <ScrollToBottomButton onClick={scrollToBottom} />}

            {/* Needs-you dock — approvals and questions pinned above the composer */}
            <NeedsYouDock
              approval={dockApproval}
              questions={dockQuestions}
              onApprove={handleApprovalRespond}
              onAnswer={handleAnswer}
            />

            {/* Composer / Input area */}
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
            />
          </div>
        )}
      </div>

      {/* Inspector rail — files / workflows / agents / usage. Sibling of the
          content area, so it persists in both GUI and Terminal mode. */}
      {railOpen && <InspectorRail session={session} onClose={toggleRail} />}
      </div>
    </div>
  );
};

const toggleBtnStyle: React.CSSProperties = {
  fontSize: '0.55rem',
  fontWeight: 600,
  padding: '2px 8px',
  borderRadius: 4,
  border: 'none',
  cursor: 'pointer',
};

export default ClaudePane;
