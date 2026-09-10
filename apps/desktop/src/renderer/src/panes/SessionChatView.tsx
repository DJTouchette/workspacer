import { ManagerHandoffStatus } from '../components/claude/ManagerHandoffStatus';
import { FirstTaskGuidance } from '../components/FirstTaskGuidance';
import '@xterm/xterm/css/xterm.css';
import { ArrowRightLeft, Clock, KeyRound, PanelRight } from 'lucide-react';
import React from 'react';
import { BrandSpinner } from '../components/Brand';
import { claudeColors as colors, StatusBadge } from '../components/claude-shared';
import { Composer } from '../components/claude/Composer';
import { ComposerControls } from '../components/claude/ComposerControls';
import { AgentHero, ConversationEmptyState } from '../components/claude/ConversationEmptyState';
import { DropOverlay } from '../components/claude/DropOverlay';
import { HandoffDialog } from '../components/claude/HandoffDialog';
import { HtmlCardHostProvider } from '../components/claude/HtmlResponseCard';
import { InlineWorkLog } from '../components/claude/InlineWorkLog';
import { InspectorRail } from '../components/claude/InspectorRail';
import { NeedsYouDock } from '../components/claude/NeedsYouDock';
import { ScrollToBottomButton } from '../components/claude/ScrollToBottomButton';
import { SessionStatusBar } from '../components/claude/SessionStatusBar';
import { TasksCard } from '../components/claude/TasksCard';
import { WorkingTimer } from '../components/claude/WorkingTimer';
import ErrorBoundary from '../components/ErrorBoundary';
import { RefreshCw } from '../components/icons';
import { MarkdownFileCwdProvider } from '../components/markdown';
import { SkillInventoryProvider } from '../contexts/SkillInventoryContext';
import { permissionModeLabel } from '../lib/providerCaps';
import { useSessionChatState } from '../hooks/useSessionChatUiState';
import { CONVERSATION_PAGE_SIZE, type useClaudePaneModel } from './ClaudePane';

/** The single chat/compose/inspection renderer. Lifecycle and transport remain in ClaudePane. */
export function SessionChatView(model: ReturnType<typeof useClaudePaneModel>) {
  const {
    isManager,
    requestCaptureStatus,
    managerHandoffBusy,
    handleManagerHandoff,
    replacementOperation,
    replacementError,
    inFleet,
    agentName,
    approvalDismissedAt,
    attachSessionId,
    attachedFiles,
    canSwitchProfile,
    cancelTask,
    cardHost,
    claudeTransport,
    config,
    contentAreaRef,
    conversation,
    cwd,
    dockApproval,
    dockQuestions,
    effectiveCwd,
    forceRepaint,
    handleAnswer,
    handleApprovalRespond,
    handleDecline,
    handleHandoff,
    handlePaste,
    handleRestartWith,
    handleScroll,
    handleSend,
    handleSlashPick,
    handoffBusy,
    handoffOpen,
    hasOlderMessages,
    hasTerminal,
    historyPending,
    initialPrompt,
    inputRef,
    inputValue,
    isClaude,
    isDragOver,
    isStreaming,
    liveProfileId,
    liveSubagents,
    liveToolCalls,
    liveWorkflows,
    loadOlderMessages,
    needsSignIn,
    openFilePicker,
    paneRootRef,
    pendingApproval,
    plan,
    planSig,
    profileId,
    provider,
    railOpen,
    removeAttachedFile,
    renderedConversation,
    retry,
    save,
    scrollContainerRef,
    scrollToBottom,
    session,
    sessionExited,
    sessionId,
    setDismissedPlanSig,
    setHandoffOpen,
    setInputValue,
    setViewMode,
    showHookHint,
    showScrollBtn,
    showTasksCard,
    showTimestamps,
    showViewToggle,
    slashItems,
    spawnError,
    subagents,
    tailPad,
    termContainerRef,
    title,
    toggleRail,
    viewMode,
    visibleCount,
    workStartedAt,
    workflows,
  } = model;
  const [sendError, setSendError] = useSessionChatState(
    sessionId ?? attachSessionId,
    'composerSendError',
    '',
  );
  const sendFromComposer = async () => {
    setSendError('');
    // The shared owner restores failed drafts and retracts pending turns.
    const result = await handleSend();
    if (!result.ok) setSendError(result.error ?? 'Message could not be sent');
  };
  return (
    // The session's skill inventory, provided once for the whole pane: a Skill
    // tool call carries only a name, and every card that renders one looks the
    // rest up here (description, origin, file) instead of re-deriving it.
    <SkillInventoryProvider skills={session?.statusLine?.capabilities?.inventory?.skills}>
      <div
        ref={paneRootRef}
        data-session-chat-view
        onFocus={inFleet ? (e) => e.stopPropagation() : undefined}
        onMouseDown={inFleet ? (e) => e.stopPropagation() : undefined}
        onClick={inFleet ? (e) => e.stopPropagation() : undefined}
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
            style={{
              flex: 1,
              minWidth: 0,
              overflow: 'hidden',
              position: 'relative',
              // A real column: the term/GUI viewport fills the top, the status
              // bar takes its own row below. As a plain block (with the GUI view
              // at height:100%) the bar rendered past the clipped edge and was
              // invisible.
              display: 'flex',
              flexDirection: 'column',
            }}
          >
            {isDragOver && <DropOverlay />}

            {/* One-time account sign-in (fresh "Add Claude Account" profile):
              shown in BOTH views until the profile's credentials appear. */}
            {needsSignIn && (
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 8,
                  padding: '6px 12px',
                  flexShrink: 0,
                  fontSize: '0.72rem',
                  color: 'var(--wks-text-secondary)',
                  background: 'color-mix(in srgb, var(--wks-warning) 10%, transparent)',
                  borderBottom: '1px solid var(--wks-border-subtle)',
                }}
              >
                <KeyRound size={13} style={{ color: 'var(--wks-warning)', flexShrink: 0 }} />
                {hasTerminal ? (
                  <span>
                    One-time sign-in: this profile's account isn't logged in yet — run{' '}
                    <code style={{ fontFamily: 'var(--wks-font-mono)' }}>/login</code> in this
                    terminal. The chat view takes over automatically once you're in.
                  </span>
                ) : (
                  <span>
                    This profile's account isn't logged in yet. Dispatch it once on the Terminal
                    transport (or run{' '}
                    <code style={{ fontFamily: 'var(--wks-font-mono)' }}>claude</code> with this
                    profile in any terminal) to sign in.
                  </span>
                )}
              </div>
            )}

            {sessionId && conversation.some((turn) => turn.role === 'user') && (
              <FirstTaskGuidance inFleet={inFleet} />
            )}

            {/* Term/GUI viewport — both views fill this box; the status bar is
              its in-flow sibling below, inside the same content column. */}
            <div style={{ position: 'relative', flex: 1, minHeight: 0 }}>
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
                  {/* Centered content container — the shared chat measure
                    (--wks-chat-width), same as the composer and the docks. */}
                  <div
                    style={{
                      maxWidth: 'var(--wks-chat-width)',
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

                    {/* The attach target is a dead session (stopped daemon row —
                    typically after a machine reboot) and no snapshot will ever
                    arrive. Boot reconciliation usually auto-resumes it within
                    moments; this state covers the gap, and the button covers
                    the cases auto-resume can't (respawn failed, row gone). */}
                    {conversation.length === 0 && !session && !spawnError && sessionExited && (
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
                          title={<>Session stopped</>}
                          dimLogo
                        />
                        <div
                          style={{
                            position: 'relative',
                            fontSize: '0.72rem',
                            margin: '14px auto 0',
                            maxWidth: 420,
                            lineHeight: 1.5,
                            color: colors.mutedDim,
                          }}
                        >
                          This {agentName} session isn’t running — it was likely stopped by a reboot
                          or shutdown. Resuming brings the conversation back where it left off.
                        </div>
                        <button
                          onClick={() => handleRestartWith({})}
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
                          Resume session
                        </button>
                      </div>
                    )}

                    {conversation.length === 0 && !session && !spawnError && !sessionExited && (
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
                            Still connecting — make sure hooks are configured in
                            ~/.claude/settings.json
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
                        hub={session.hub}
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
                          Load{' '}
                          {Math.min(CONVERSATION_PAGE_SIZE, conversation.length - visibleCount)}{' '}
                          earlier messages ({conversation.length - visibleCount} hidden)
                        </button>
                      </div>
                    )}

                    {/* Rendered conversation messages with dividers. The cwd
                      provider lets file paths mentioned in assistant prose /
                      command output resolve + open like tool-call FileLinks. */}
                    <ErrorBoundary label="Conversation" resetKeys={[sessionId]}>
                      <MarkdownFileCwdProvider value={effectiveCwd}>
                        {/* Response-card actions act through THIS pane's own
                          live session/pane/cwd, bound here at render time. A
                          card three turns up, or one replayed out of history,
                          gets the same binding — it can name a target but never
                          the authority it is acted on with. */}
                        <HtmlCardHostProvider value={cardHost}>
                          {renderedConversation}
                        </HtmlCardHostProvider>
                      </MarkdownFileCwdProvider>
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
                        {/* Elapsed run time. Stopping lives in the composer now —
                          one place for actions, and reachable without leaving
                          the box you're typing in. */}
                        {workStartedAt !== null && <WorkingTimer since={workStartedAt} />}
                      </div>
                    )}

                    {/* Tail spacer — the room the newest user message is pinned
                      above, filled in by the reply as it streams. */}
                    {tailPad > 0 && <div data-tail-pad aria-hidden style={{ height: tailPad }} />}
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

                {isManager && (
                  <ManagerHandoffStatus
                    operation={replacementOperation}
                    error={replacementError}
                    provider={provider}
                    cwd={cwd}
                  />
                )}

                {/* Composer / Input area — session pills live inside its bottom row */}
                {isManager && requestCaptureStatus && (
                  <div role="status" style={{ padding: '4px 16px', color: 'var(--wks-text-secondary)', fontSize: '0.72rem' }}>
                    {requestCaptureStatus}
                  </div>
                )}
                {sendError && (
                  <div
                    role="alert"
                    style={{
                      width: '100%',
                      maxWidth: 'var(--wks-chat-width)',
                      margin: '0 auto',
                      padding: '8px 16px',
                      boxSizing: 'border-box',
                      color: 'var(--wks-error)',
                      fontSize: '0.8rem',
                    }}
                  >
                    {sendError}
                  </div>
                )}
                <Composer
                  value={inputValue}
                  onChange={setInputValue}
                  onSend={sendFromComposer}
                  onPaste={handlePaste}
                  onPickFiles={openFilePicker}
                  attachedFiles={attachedFiles}
                  onRemoveFile={removeAttachedFile}
                  dimmed={!!(dockApproval || dockQuestions)}
                  inputRef={inputRef}
                  showSendButton={config.ui.showComposerSend !== false}
                  working={isStreaming}
                  onStop={cancelTask}
                  agentName={agentName}
                  slashItems={slashItems}
                  onSlashPick={handleSlashPick}
                  controls={
                    <ComposerControls
                      provider={provider ?? 'claude'}
                      sessionId={sessionId}
                      snapshot={session}
                      cwd={cwd}
                      profileId={liveProfileId}
                      canSwitchProfile={canSwitchProfile}
                      onRestartWith={handleRestartWith}
                    />
                  }
                />
              </div>
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
              {/* In GUI mode the row aligns to the composer's centered chat column
            so the footer line sits flush under it; terminal mode stays
            edge-to-edge like a toolbar. */}
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 10,
                  minWidth: 0,
                  minHeight: 24,
                  ...(viewMode === 'gui'
                    ? { maxWidth: 'var(--wks-chat-width)', margin: '0 auto' }
                    : {}),
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
                    profileId={liveProfileId}
                    canSwitchProfile={canSwitchProfile}
                    onRestartWith={handleRestartWith}
                  />
                )}

                {/* In-app status line — telemetry only (dir/branch · plan · ctx ·
            tok/cost · quota meters). Controls (model/effort/permissions) live
            in the ComposerControls pills, never here. */}
                <SessionStatusBar snapshot={session} sessionId={sessionId ?? undefined} cwd={cwd} />

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
                        fontFamily: 'var(--wks-font-mono)',
                        padding: '1px 7px',
                        borderRadius: 'var(--wks-radius-pill)',
                        letterSpacing: '0.03em',
                        color: 'var(--wks-purple)',
                        border: '1px solid color-mix(in srgb, var(--wks-purple) 40%, transparent)',
                        background: 'color-mix(in srgb, var(--wks-purple) 10%, transparent)',
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
                      fontFamily: 'var(--wks-font-mono)',
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
                  onClick={() => (isManager ? void handleManagerHandoff() : setHandoffOpen(true))}
                  title={
                    isManager
                      ? 'Checkpoint and replace this Fleet Manager in the same pane — local desktop and local workers only'
                      : handoffBusy === 'agent'
                        ? 'Waiting for the agent to write its handoff brief…'
                        : 'Hand off this session to a new agent — pick provider, model, effort and permissions (summarized brief, new session)'
                  }
                  className="wks-composer-icon-btn"
                  disabled={!!handoffBusy || managerHandoffBusy || !(sessionId ?? attachSessionId)}
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
                {!isManager && handoffOpen && (
                  <HandoffDialog
                    provider={provider ?? 'claude'}
                    snapshot={session}
                    cwd={cwd}
                    busy={handoffBusy}
                    onCancel={() => setHandoffOpen(false)}
                    onConfirm={(settings) => void handleHandoff(settings)}
                  />
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
            and in both UI modes: per-agent depth is if anything MORE wanted when
            you're focused on one agent, so this is never mode-gated. */}
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

          {/* Inspector rail — the session inspector (files / workflows / agents /
          usage) plus this project's widget board. Sibling of the content area,
          so it persists in both GUI and Terminal mode. effectiveCwd is what the
          board is keyed by, so it works before a session attaches. */}
          {railOpen && !inFleet && (
            <InspectorRail
              session={session}
              sessionId={sessionId ?? undefined}
              cwd={effectiveCwd}
              onClose={toggleRail}
            />
          )}
        </div>
      </div>
    </SkillInventoryProvider>
  );
}
const toggleBtnStyle: React.CSSProperties = {
  fontSize: '0.66rem',
  fontWeight: 600,
  padding: '3px 9px',
  borderRadius: 6,
  border: 'none',
  cursor: 'pointer',
};
