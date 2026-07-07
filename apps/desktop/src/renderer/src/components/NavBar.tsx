import React, { useState, useRef, useEffect } from 'react';
import { PaneType, TabConfig } from '../types/pane';
import { useConfig, ScriptEntry, Config } from '../hooks/useConfig';
import { themes, resolveTheme, themeDisplayName } from '../themes';
import { useIsSmallScreen } from '../hooks/useMediaQuery';
import { PaneIcon, Play, Settings, Plus, Columns3 } from './icons';
import { AgentLogo } from './agentLogos';
import { ContextMenu, ContextMenuItem, ContextMenuSeparator } from './ContextMenu';
import { resolveNavHeight } from '../lib/layoutUtils';

interface NavBarProps {
  tabs: TabConfig[];
  activeTabId: string;
  onTabClick: (id: string) => void;
  onAddTab?: (
    type: PaneType,
    shell?: string,
    label?: string,
    cwd?: string,
    profileId?: string,
    resumeSessionId?: string,
    attachSessionId?: string,
  ) => void;
  onCloseTab?: (tabId: string) => void;
  onRenameTab?: (tabId: string) => void;
  onSplitTab?: (tabId: string, type: PaneType) => void;
  onMoveTab?: (tabId: string, toIndex: number) => void;
  /** Pixels to inset the bar from the left (to clear the agent sidebar). */
  leftOffset?: number;
  /** Active agent's workspace root — scripts are scoped to this directory. */
  cwd?: string;
  /** Script buttons for the current directory. */
  scripts?: ScriptEntry[];
  /** Run a script in a new terminal tab at `cwd`. */
  onRunScript?: (name: string, command: string) => void;
  /** Persist the current directory's script list. */
  onSaveScripts?: (entries: ScriptEntry[]) => void;
}

interface SessionPickerState {
  folder: string;
  profileId?: string;
  sessions: { sessionId: string; timestamp: string; summary: string }[];
}

const NavBar: React.FC<NavBarProps> = ({
  tabs,
  activeTabId,
  onTabClick,
  onAddTab,
  onCloseTab,
  onRenameTab,
  onSplitTab,
  onMoveTab,
  leftOffset = 0,
  cwd,
  scripts = [],
  onRunScript,
  onSaveScripts,
}) => {
  const { config, save } = useConfig();
  const isSmallScreen = useIsSmallScreen();
  const navHeight = resolveNavHeight(config.ui.navBarHeight, isSmallScreen);
  // On Windows the native caption buttons (min/max/close) are drawn by the
  // titleBarOverlay in the top-right corner. Reserve that strip so the
  // right-aligned controls (theme switcher, scripts) don't slide under them.
  const winControlsWidth = window.electronAPI?.platform === 'win32' ? 138 : 0;
  const shells = config.terminal.shells || [];
  const [showMenu, setShowMenu] = useState(false);
  const [tabContextMenu, setTabContextMenu] = useState<{
    tabId: string;
    tabIdx: number;
    x: number;
    y: number;
  } | null>(null);
  const [profilePickerState, setProfilePickerState] = useState<{
    folder: string;
    profiles: any[];
  } | null>(null);
  const [sessionPickerState, setSessionPickerState] = useState<SessionPickerState | null>(null);
  const [managingScripts, setManagingScripts] = useState(false);
  const [scriptDraft, setScriptDraft] = useState<ScriptEntry[]>([]);
  const menuRef = useRef<HTMLDivElement>(null);
  const addTabBtnRef = useRef<HTMLButtonElement>(null);
  const [menuLeft, setMenuLeft] = useState(0);
  const scriptMenuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!showMenu && !managingScripts) return;
    const handler = (e: MouseEvent) => {
      if (showMenu && menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setShowMenu(false);
      }
      if (
        managingScripts &&
        scriptMenuRef.current &&
        !scriptMenuRef.current.contains(e.target as Node)
      ) {
        setManagingScripts(false);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [showMenu, managingScripts]);

  return (
    <nav
      className="navbar"
      style={{
        position: 'absolute',
        top: 0,
        left: leftOffset,
        right: 0,
        height: `${navHeight}px`,
        display: 'flex',
        alignItems: 'center',
        backgroundColor: 'var(--wks-bg-raised)',
        borderBottom: '1px solid var(--wks-border-subtle)',
        paddingTop: 0,
        paddingBottom: 0,
        paddingLeft: 10,
        paddingRight: 10 + winControlsWidth,
        zIndex: 100,
        userSelect: 'none',
        // @ts-ignore
        WebkitAppRegion: 'drag',
        appRegion: 'drag',
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: '2px',
          flex: 1,
          // Draggable container: the empty space after the tabs must move the
          // window. Interactive children (tab buttons, add-tab) opt back out
          // with their own `no-drag` below.
          // @ts-ignore
          WebkitAppRegion: 'drag',
          appRegion: 'drag',
          overflow: 'hidden',
        }}
      >
        {tabs.map((tab, idx) => {
          const isActive = tab.id === activeTabId;
          const singlePane = tab.panes.length === 1;
          const firstPaneType = tab.panes[0]?.type ?? 'terminal';
          const firstPaneProvider = tab.panes[0]?.provider;
          const hasHibernated = tab.panes.some((p) => p.hibernated);

          return (
            <button
              key={tab.id}
              onClick={() => onTabClick(tab.id)}
              onContextMenu={(e) => {
                e.preventDefault();
                setTabContextMenu({ tabId: tab.id, tabIdx: idx, x: e.clientX, y: e.clientY });
              }}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: '6px',
                padding: '0 13px',
                margin: 0,
                width: 'auto',
                height: '28px',
                lineHeight: 1,
                border: 'none',
                borderRadius: 'var(--wks-radius-md)',
                cursor: 'pointer',
                fontSize: '0.8rem',
                fontFamily: 'inherit',
                fontWeight: isActive ? 600 : 500,
                backgroundColor: isActive ? 'var(--wks-accent-bg)' : 'transparent',
                boxShadow: isActive ? 'inset 0 0 0 1px var(--wks-accent-glow)' : 'none',
                color: isActive ? 'var(--wks-text-primary)' : 'var(--wks-text-muted)',
                opacity: hasHibernated && !isActive ? 0.4 : 1,
                transition: 'background-color 0.1s, color 0.1s',
                // @ts-ignore
                WebkitAppRegion: 'no-drag',
                appRegion: 'no-drag',
              }}
              onMouseEnter={(e) => {
                if (!isActive) {
                  (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-hover)';
                  (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-tertiary)';
                }
              }}
              onMouseLeave={(e) => {
                if (!isActive) {
                  (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent';
                  (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-muted)';
                }
              }}
              title={`${tab.title} (Ctrl+${idx + 1})`}
            >
              <span style={{ display: 'flex', alignItems: 'center' }}>
                {singlePane ? (
                  firstPaneType === 'claude' ? (
                    <AgentLogo provider={firstPaneProvider ?? 'claude'} size={13} />
                  ) : (
                    <PaneIcon type={firstPaneType} size={13} />
                  )
                ) : (
                  <Columns3 size={13} strokeWidth={1.75} />
                )}
              </span>
              <span>{tab.title}</span>
              {!singlePane && (
                <span style={{ fontSize: '0.5rem', opacity: 0.6 }}>{tab.panes.length}</span>
              )}
            </button>
          );
        })}

        {onAddTab && (
          <div
            ref={menuRef}
            style={{
              position: 'relative',
              display: 'inline-flex',
              // @ts-ignore
              WebkitAppRegion: 'no-drag',
              appRegion: 'no-drag',
            }}
          >
            <button
              ref={addTabBtnRef}
              onClick={() => onAddTab('terminal')}
              onContextMenu={(e) => {
                e.preventDefault();
                if (addTabBtnRef.current) {
                  const rect = addTabBtnRef.current.getBoundingClientRect();
                  const left = Math.min(rect.left, window.innerWidth - 170);
                  setMenuLeft(Math.max(0, left));
                }
                setShowMenu((v) => !v);
              }}
              style={{
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                gap: 1,
                padding: '0 6px',
                margin: '0 0 0 4px',
                height: '28px',
                lineHeight: 1,
                border: 'none',
                borderRadius: 'var(--wks-radius-md)',
                cursor: 'pointer',
                fontSize: '0.9rem',
                fontFamily: 'inherit',
                fontWeight: 400,
                backgroundColor: 'transparent',
                color: 'var(--wks-text-faint)',
                transition: 'background-color 0.1s, color 0.1s',
              }}
              onMouseEnter={(e) => {
                (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-hover)';
                (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-secondary)';
              }}
              onMouseLeave={(e) => {
                (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent';
                (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-faint)';
              }}
              title="New terminal (Ctrl+T) | Right-click for more"
            >
              <Plus size={15} strokeWidth={2} />
              <span style={{ fontSize: '0.5rem', lineHeight: 1, opacity: 0.6 }}>▾</span>
            </button>

            {showMenu && (
              <div
                style={{
                  position: 'fixed',
                  top: `${navHeight}px`,
                  left: `${menuLeft}px`,
                  backgroundColor: 'var(--wks-bg-surface)',
                  border: '1px solid var(--wks-border-input)',
                  borderRadius: '4px',
                  padding: '4px 0',
                  zIndex: 10000,
                  minWidth: '160px',
                  boxShadow: '0 4px 12px var(--wks-shadow)',
                }}
              >
                <div
                  style={{
                    padding: '2px 8px',
                    fontSize: '0.55rem',
                    color: 'var(--wks-text-disabled)',
                    textTransform: 'uppercase',
                    letterSpacing: '0.05em',
                  }}
                >
                  New Tab
                </div>
                <MenuButton
                  label="Claude"
                  icon={<PaneIcon type="claude" size={13} />}
                  onClick={async () => {
                    setShowMenu(false);
                    const folder = await window.electronAPI.pickFolder(
                      config.agents?.defaultCwd?.trim() || undefined,
                    );
                    if (!folder) return;
                    // Check for profiles — show picker if more than 1
                    let profileId: string | undefined;
                    try {
                      const profiles = await window.electronAPI.claudeProfilesList();
                      if (profiles.length > 1) {
                        setProfilePickerState({ folder, profiles });
                        return;
                      }
                    } catch {}
                    // Check for existing sessions
                    try {
                      const sessions = await window.electronAPI.claudeListSessionsForDir(folder);
                      if (sessions.length > 0) {
                        setSessionPickerState({ folder, profileId, sessions });
                        return;
                      }
                    } catch {}
                    onAddTab('claude', undefined, undefined, folder);
                  }}
                />
                <MenuButton
                  label="Browser"
                  icon={<PaneIcon type="browser" size={13} />}
                  onClick={() => {
                    setShowMenu(false);
                    onAddTab('browser');
                  }}
                />
                <MenuButton
                  label="Notes"
                  icon={<PaneIcon type="notes" size={13} />}
                  onClick={() => {
                    setShowMenu(false);
                    onAddTab('notes');
                  }}
                />

                <div
                  style={{ height: '1px', backgroundColor: 'var(--wks-border)', margin: '4px 0' }}
                />

                <div
                  style={{
                    padding: '2px 8px',
                    fontSize: '0.55rem',
                    color: 'var(--wks-text-disabled)',
                    textTransform: 'uppercase',
                    letterSpacing: '0.05em',
                  }}
                >
                  Terminal
                </div>
                {shells.map((shell) => (
                  <MenuButton
                    key={shell.name}
                    label={shell.label}
                    icon={<PaneIcon type="terminal" size={13} />}
                    onClick={() => {
                      setShowMenu(false);
                      onAddTab('terminal', shell.path, shell.label);
                    }}
                  />
                ))}
              </div>
            )}
          </div>
        )}
      </div>

      {/* Theme switcher — current theme's accent/purple/blue swatch + name,
          with a dropdown to pick any theme. Mirrors the mockup's right cluster. */}
      <ThemeSwitcher config={config} save={save} />

      {/* Per-directory script bar (right side) */}
      {cwd && onRunScript && (
        <div
          ref={scriptMenuRef}
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: '4px',
            flexShrink: 0,
            paddingLeft: '8px',
            position: 'relative',
            // @ts-ignore
            WebkitAppRegion: 'no-drag',
            appRegion: 'no-drag',
          }}
        >
          {scripts.map((s, i) => (
            <button
              key={`${s.name}-${i}`}
              onClick={() => onRunScript(s.name, s.command)}
              title={s.command}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 4,
                height: '22px',
                padding: '0 10px',
                margin: 0,
                border: '1px solid var(--wks-border)',
                borderRadius: 'var(--wks-radius-pill)',
                cursor: 'pointer',
                fontSize: '0.68rem',
                fontFamily: 'inherit',
                backgroundColor: 'transparent',
                color: 'var(--wks-text-muted)',
                whiteSpace: 'nowrap',
                lineHeight: 1,
              }}
              onMouseEnter={(e) => {
                (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-hover)';
                (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-primary)';
              }}
              onMouseLeave={(e) => {
                (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent';
                (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-muted)';
              }}
            >
              <Play size={11} strokeWidth={2} style={{ opacity: 0.75 }} />
              {s.name}
            </button>
          ))}

          {/* Manage scripts */}
          {onSaveScripts && (
            <button
              onClick={() => {
                setScriptDraft(scripts.map((s) => ({ ...s })));
                setManagingScripts((v) => !v);
              }}
              title="Edit scripts for this directory"
              style={{
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                width: '22px',
                height: '22px',
                padding: 0,
                margin: 0,
                border: '1px dashed var(--wks-border-input)',
                borderRadius: 'var(--wks-radius-pill)',
                cursor: 'pointer',
                fontSize: '0.7rem',
                fontFamily: 'inherit',
                backgroundColor: 'transparent',
                color: 'var(--wks-text-faint)',
              }}
              onMouseEnter={(e) => {
                (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-secondary)';
              }}
              onMouseLeave={(e) => {
                (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-faint)';
              }}
            >
              {scripts.length === 0 ? (
                <Plus size={13} strokeWidth={2} />
              ) : (
                <Settings size={12} strokeWidth={1.75} />
              )}
            </button>
          )}

          {managingScripts && onSaveScripts && (
            <ScriptManager
              navHeight={navHeight}
              draft={scriptDraft}
              setDraft={setScriptDraft}
              onSave={(entries) => {
                onSaveScripts(entries);
                setManagingScripts(false);
              }}
              onCancel={() => setManagingScripts(false)}
            />
          )}
        </div>
      )}

      {/* Profile Picker Modal */}
      {profilePickerState && (
        <div
          onClick={() => setProfilePickerState(null)}
          style={{
            position: 'fixed',
            inset: 0,
            zIndex: 3000,
            display: 'flex',
            justifyContent: 'center',
            paddingTop: '20vh',
            backgroundColor: 'rgba(0,0,0,0.4)',
          }}
        >
          <div
            onClick={(e) => e.stopPropagation()}
            style={{
              backgroundColor: 'var(--wks-bg-raised)',
              border: '1px solid var(--wks-border-input)',
              borderRadius: 'var(--wks-radius-md)',
              width: 'min(320px, 92vw)',
              boxSizing: 'border-box',
              padding: '12px 0',
              maxHeight: 300,
              boxShadow: '0 8px 32px var(--wks-shadow)',
            }}
          >
            <div
              style={{
                padding: '0 16px 8px',
                fontSize: '0.75rem',
                fontWeight: 600,
                color: 'var(--wks-text-secondary)',
                borderBottom: '1px solid var(--wks-border)',
              }}
            >
              Select Claude Profile
            </div>
            {profilePickerState.profiles.map((p: any) => (
              <div
                key={p.id}
                onClick={async () => {
                  const { folder } = profilePickerState;
                  setProfilePickerState(null);
                  // Check for existing sessions before launching
                  try {
                    const sessions = await window.electronAPI.claudeListSessionsForDir(folder);
                    if (sessions.length > 0) {
                      setSessionPickerState({ folder, profileId: p.id, sessions });
                      return;
                    }
                  } catch {}
                  onAddTab?.('claude', undefined, undefined, folder, p.id);
                }}
                style={{
                  padding: '8px 16px',
                  cursor: 'pointer',
                  fontSize: '0.72rem',
                  display: 'flex',
                  alignItems: 'center',
                  gap: 8,
                }}
                onMouseEnter={(e) => {
                  (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-selected)';
                }}
                onMouseLeave={(e) => {
                  (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent';
                }}
              >
                <span
                  style={{
                    color: p.isDefault ? 'var(--wks-accent)' : 'var(--wks-text-disabled)',
                    fontSize: '0.7rem',
                  }}
                >
                  {p.isDefault ? '\u2666' : '\u25CB'}
                </span>
                <div style={{ flex: 1 }}>
                  <div style={{ color: 'var(--wks-text-primary)', fontWeight: 500 }}>{p.name}</div>
                  {p.extraArgs?.length > 0 && (
                    <div
                      style={{
                        fontSize: '0.58rem',
                        color: 'var(--wks-text-faint)',
                        fontFamily: 'var(--wks-font-mono)',
                      }}
                    >
                      {p.extraArgs.join(' ')}
                    </div>
                  )}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
      {/* Session Picker Modal */}
      {sessionPickerState && (
        <div
          onClick={() => setSessionPickerState(null)}
          style={{
            position: 'fixed',
            inset: 0,
            zIndex: 3000,
            display: 'flex',
            justifyContent: 'center',
            paddingTop: '15vh',
            backgroundColor: 'rgba(0,0,0,0.4)',
          }}
        >
          <div
            onClick={(e) => e.stopPropagation()}
            style={{
              backgroundColor: 'var(--wks-bg-raised)',
              border: '1px solid var(--wks-border-input)',
              borderRadius: 'var(--wks-radius-md)',
              width: 'min(400px, 92vw)',
              boxSizing: 'border-box',
              padding: '12px 0',
              maxHeight: 420,
              boxShadow: '0 8px 32px var(--wks-shadow)',
              display: 'flex',
              flexDirection: 'column',
            }}
          >
            <div
              style={{
                padding: '0 16px 8px',
                fontSize: '0.75rem',
                fontWeight: 600,
                color: 'var(--wks-text-secondary)',
                borderBottom: '1px solid var(--wks-border)',
              }}
            >
              Claude Session
            </div>
            <div style={{ overflowY: 'auto', flex: 1 }}>
              {/* New session option */}
              <div
                onClick={() => {
                  const { folder, profileId } = sessionPickerState;
                  setSessionPickerState(null);
                  onAddTab?.('claude', undefined, undefined, folder, profileId);
                }}
                style={{
                  padding: '10px 16px',
                  cursor: 'pointer',
                  fontSize: '0.72rem',
                  display: 'flex',
                  alignItems: 'center',
                  gap: 8,
                  borderBottom: '1px solid var(--wks-border)',
                }}
                onMouseEnter={(e) => {
                  (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-selected)';
                }}
                onMouseLeave={(e) => {
                  (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent';
                }}
              >
                <span style={{ color: 'var(--wks-accent)', fontSize: '0.8rem' }}>+</span>
                <div>
                  <div style={{ color: 'var(--wks-text-primary)', fontWeight: 600 }}>
                    New Session
                  </div>
                  <div style={{ fontSize: '0.58rem', color: 'var(--wks-text-faint)' }}>
                    Start a fresh conversation
                  </div>
                </div>
              </div>

              {/* Existing sessions */}
              <div
                style={{
                  padding: '6px 16px 2px',
                  fontSize: '0.55rem',
                  color: 'var(--wks-text-disabled)',
                  textTransform: 'uppercase',
                  letterSpacing: '0.05em',
                }}
              >
                Resume Session
              </div>
              {sessionPickerState.sessions.map((s) => (
                <div
                  key={s.sessionId}
                  onClick={() => {
                    const { folder, profileId } = sessionPickerState;
                    setSessionPickerState(null);
                    onAddTab?.('claude', undefined, undefined, folder, profileId, s.sessionId);
                  }}
                  style={{
                    padding: '8px 16px',
                    cursor: 'pointer',
                    fontSize: '0.72rem',
                    display: 'flex',
                    alignItems: 'center',
                    gap: 8,
                  }}
                  onMouseEnter={(e) => {
                    (e.currentTarget as HTMLElement).style.backgroundColor =
                      'var(--wks-bg-selected)';
                  }}
                  onMouseLeave={(e) => {
                    (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent';
                  }}
                >
                  <span style={{ color: 'var(--wks-text-disabled)', fontSize: '0.7rem' }}>
                    {'\u25B6'}
                  </span>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div
                      style={{
                        color: 'var(--wks-text-primary)',
                        fontWeight: 500,
                        overflow: 'hidden',
                        textOverflow: 'ellipsis',
                        whiteSpace: 'nowrap',
                      }}
                    >
                      {s.summary}
                    </div>
                    <div
                      style={{
                        fontSize: '0.58rem',
                        color: 'var(--wks-text-faint)',
                        fontFamily: 'var(--wks-font-mono)',
                      }}
                    >
                      {formatSessionDate(s.timestamp)}
                    </div>
                  </div>
                </div>
              ))}
            </div>
          </div>
        </div>
      )}
      {/* Tab context menu */}
      {tabContextMenu && (
        <ContextMenu
          x={tabContextMenu.x}
          y={tabContextMenu.y}
          minWidth={140}
          onClose={() => setTabContextMenu(null)}
        >
          <ContextMenuItem
            label="Rename"
            onClick={() => {
              setTabContextMenu(null);
              onRenameTab?.(tabContextMenu.tabId);
            }}
          />
          <ContextMenuItem
            label="Split: Terminal"
            onClick={() => {
              setTabContextMenu(null);
              onSplitTab?.(tabContextMenu.tabId, 'terminal');
            }}
          />
          <ContextMenuItem
            label="Split: Claude"
            onClick={() => {
              setTabContextMenu(null);
              onSplitTab?.(tabContextMenu.tabId, 'claude');
            }}
          />
          <ContextMenuItem
            label="Split: Browser"
            onClick={() => {
              setTabContextMenu(null);
              onSplitTab?.(tabContextMenu.tabId, 'browser');
            }}
          />
          <ContextMenuSeparator />
          {tabContextMenu.tabIdx > 0 && (
            <ContextMenuItem
              label="Move left"
              onClick={() => {
                setTabContextMenu(null);
                onMoveTab?.(tabContextMenu.tabId, tabContextMenu.tabIdx - 1);
              }}
            />
          )}
          {tabContextMenu.tabIdx < tabs.length - 1 && (
            <ContextMenuItem
              label="Move right"
              onClick={() => {
                setTabContextMenu(null);
                onMoveTab?.(tabContextMenu.tabId, tabContextMenu.tabIdx + 1);
              }}
            />
          )}
          <ContextMenuSeparator />
          <ContextMenuItem
            label="Close"
            danger
            onClick={() => {
              setTabContextMenu(null);
              onCloseTab?.(tabContextMenu.tabId);
            }}
          />
        </ContextMenu>
      )}
    </nav>
  );
};

/** Right-cluster theme picker: a pill showing the active theme's 3-dot swatch
 *  and name; clicking opens a dropdown of every theme. Wired through config so
 *  the choice persists (and re-adopts that theme's corners/border). */
const ThemeSwitcher: React.FC<{
  config: Config;
  save: (p: Partial<Config>) => Promise<Config>;
}> = ({ config, save }) => {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);
  const btnRef = useRef<HTMLButtonElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number }>({ left: 0, top: 0 });
  const customThemes = config.ui.customThemes;
  const active = resolveTheme(config.ui.theme, customThemes);
  const dots = [active.accent, active.terminal.magenta, active.terminal.blue];
  // Built-ins first, then the user's custom themes (each resolved so the
  // swatch dots reflect its actual palette).
  const entries: Array<[string, ReturnType<typeof resolveTheme>]> = [
    ...Object.entries(themes),
    ...Object.keys(customThemes ?? {}).map(
      (id) => [id, resolveTheme(id, customThemes)] as [string, ReturnType<typeof resolveTheme>],
    ),
  ];

  useEffect(() => {
    if (!open) return;
    const h = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', h);
    return () => document.removeEventListener('mousedown', h);
  }, [open]);

  const toggle = () => {
    if (btnRef.current) {
      const r = btnRef.current.getBoundingClientRect();
      setPos({
        left: Math.max(8, Math.min(r.right - 200, window.innerWidth - 208)),
        top: r.bottom + 4,
      });
    }
    setOpen((v) => !v);
  };
  const pick = (id: string) => {
    setOpen(false);
    void save({ ui: { ...config.ui, theme: id, cornerStyle: '', borderColor: '' } });
  };
  const dot = (c: string) => (
    <span
      style={{
        width: 8,
        height: 8,
        borderRadius: 'var(--wks-radius-pill)',
        background: c,
        flexShrink: 0,
      }}
    />
  );

  return (
    <div
      ref={wrapRef}
      style={
        {
          position: 'relative',
          display: 'inline-flex',
          flexShrink: 0,
          WebkitAppRegion: 'no-drag',
          appRegion: 'no-drag',
        } as React.CSSProperties
      }
    >
      <button
        ref={btnRef}
        onClick={toggle}
        title="Switch theme"
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 7,
          height: 28,
          padding: '0 10px',
          border: '1px solid var(--wks-border-subtle)',
          background: 'var(--wks-bg-base)',
          borderRadius: 'var(--wks-radius-md)',
          cursor: 'pointer',
          color: 'var(--wks-text-tertiary)',
          fontFamily: 'inherit',
          fontSize: '0.72rem',
          fontWeight: 600,
        }}
        onMouseEnter={(e) => {
          (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-primary)';
          (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-accent-glow)';
        }}
        onMouseLeave={(e) => {
          (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-tertiary)';
          (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-border-subtle)';
        }}
      >
        <span style={{ display: 'flex', gap: 3 }}>
          {dots.map((c, i) => (
            <React.Fragment key={i}>{dot(c)}</React.Fragment>
          ))}
        </span>
        <span style={{ whiteSpace: 'nowrap' }}>
          {themeDisplayName(config.ui.theme || 'dark', customThemes)}
        </span>
      </button>
      {open && (
        <div
          style={{
            position: 'fixed',
            left: pos.left,
            top: pos.top,
            width: 200,
            maxHeight: 360,
            overflowY: 'auto',
            background: 'var(--wks-bg-surface)',
            border: '1px solid var(--wks-border-input)',
            borderRadius: 'var(--wks-radius-md)',
            padding: '4px 0',
            zIndex: 10000,
            boxShadow: '0 8px 28px var(--wks-shadow)',
          }}
        >
          {entries.map(([id, t], i) => {
            const isCur = id === config.ui.theme;
            const firstCustom = i === Object.keys(themes).length;
            return (
              <React.Fragment key={id}>
                {firstCustom && (
                  <div
                    style={{
                      padding: '5px 12px 2px',
                      fontSize: '0.6rem',
                      fontWeight: 700,
                      letterSpacing: '0.06em',
                      textTransform: 'uppercase',
                      color: 'var(--wks-text-faint)',
                      borderTop: '1px solid var(--wks-border-subtle)',
                      marginTop: 4,
                    }}
                  >
                    Custom
                  </div>
                )}
                <button
                  onClick={() => pick(id)}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 8,
                    width: '100%',
                    padding: '6px 12px',
                    border: 'none',
                    background: isCur ? 'var(--wks-bg-selected)' : 'transparent',
                    color: isCur ? 'var(--wks-text-primary)' : 'var(--wks-text-tertiary)',
                    cursor: 'pointer',
                    fontFamily: 'inherit',
                    fontSize: '0.72rem',
                    textAlign: 'left',
                  }}
                  onMouseEnter={(e) => {
                    if (!isCur)
                      (e.currentTarget as HTMLElement).style.background = 'var(--wks-bg-hover)';
                  }}
                  onMouseLeave={(e) => {
                    if (!isCur) (e.currentTarget as HTMLElement).style.background = 'transparent';
                  }}
                >
                  <span style={{ display: 'flex', gap: 3 }}>
                    {dot(t.accent)}
                    {dot(t.terminal.magenta)}
                    {dot(t.terminal.blue)}
                  </span>
                  <span
                    style={{
                      flex: 1,
                      whiteSpace: 'nowrap',
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                    }}
                  >
                    {themeDisplayName(id, customThemes)}
                  </span>
                  {isCur && (
                    <span style={{ color: 'var(--wks-accent)', fontSize: '0.7rem' }}>✓</span>
                  )}
                </button>
              </React.Fragment>
            );
          })}
        </div>
      )}
    </div>
  );
};

function formatSessionDate(iso: string): string {
  try {
    const d = new Date(iso);
    const now = new Date();
    const diff = now.getTime() - d.getTime();
    const mins = Math.floor(diff / 60000);
    if (mins < 1) return 'just now';
    if (mins < 60) return `${mins}m ago`;
    const hours = Math.floor(mins / 60);
    if (hours < 24) return `${hours}h ago`;
    const days = Math.floor(hours / 24);
    if (days < 7) return `${days}d ago`;
    return d.toLocaleDateString();
  } catch {
    return iso;
  }
}

const ScriptManager: React.FC<{
  navHeight: number;
  draft: ScriptEntry[];
  setDraft: React.Dispatch<React.SetStateAction<ScriptEntry[]>>;
  onSave: (entries: ScriptEntry[]) => void;
  onCancel: () => void;
}> = ({ navHeight, draft, setDraft, onSave, onCancel }) => {
  const update = (i: number, patch: Partial<ScriptEntry>) =>
    setDraft((prev) => prev.map((s, idx) => (idx === i ? { ...s, ...patch } : s)));
  const remove = (i: number) => setDraft((prev) => prev.filter((_, idx) => idx !== i));
  const add = () => setDraft((prev) => [...prev, { name: '', command: '' }]);

  const inputStyle: React.CSSProperties = {
    height: '24px',
    padding: '0 6px',
    fontSize: '0.68rem',
    backgroundColor: 'var(--wks-bg-input)',
    color: 'var(--wks-text-secondary)',
    border: '1px solid var(--wks-border)',
    borderRadius: '3px',
    outline: 'none',
    fontFamily: 'inherit',
    boxSizing: 'border-box',
  };

  return (
    <div
      style={{
        position: 'fixed',
        top: `${navHeight + 2}px`,
        right: '10px',
        backgroundColor: 'var(--wks-bg-surface)',
        border: '1px solid var(--wks-border-input)',
        // Never wider than the viewport (was a hard 380px that ran off the left
        // edge under ~400px).
        borderRadius: '6px',
        padding: '10px',
        zIndex: 10000,
        width: 'min(380px, calc(100vw - 20px))',
        boxSizing: 'border-box',
        boxShadow: '0 6px 20px var(--wks-shadow)',
        display: 'flex',
        flexDirection: 'column',
        gap: '6px',
      }}
    >
      <div
        style={{
          fontSize: '0.6rem',
          color: 'var(--wks-text-disabled)',
          textTransform: 'uppercase',
          letterSpacing: '0.05em',
        }}
      >
        Scripts for this directory
      </div>

      {draft.length === 0 && (
        <div style={{ fontSize: '0.65rem', color: 'var(--wks-text-faint)', padding: '4px 0' }}>
          No scripts yet. Add one below, e.g. “Test” →{' '}
          <code style={{ fontFamily: 'var(--wks-font-mono)' }}>npm test</code>.
        </div>
      )}

      {draft.map((s, i) => (
        <div key={i} style={{ display: 'flex', gap: '4px', alignItems: 'center' }}>
          <input
            value={s.name}
            onChange={(e) => update(i, { name: e.target.value })}
            placeholder="Name"
            style={{ ...inputStyle, width: '90px', flexShrink: 0 }}
          />
          <input
            value={s.command}
            onChange={(e) => update(i, { command: e.target.value })}
            placeholder="bash command (e.g. cargo test)"
            style={{ ...inputStyle, flex: 1, fontFamily: 'var(--wks-font-mono)' }}
            onKeyDown={(e) => {
              if (e.key === 'Enter') onSave(draft.filter((d) => d.name.trim() && d.command.trim()));
            }}
          />
          <button
            onClick={() => remove(i)}
            title="Remove"
            style={{
              width: '22px',
              height: '24px',
              flexShrink: 0,
              border: '1px solid var(--wks-border)',
              borderRadius: '3px',
              backgroundColor: 'transparent',
              color: 'var(--wks-error, #e05555)',
              cursor: 'pointer',
              fontSize: '0.7rem',
            }}
          >
            {'✕'}
          </button>
        </div>
      ))}

      <button
        onClick={add}
        style={{
          padding: '4px',
          fontSize: '0.65rem',
          fontFamily: 'inherit',
          fontWeight: 500,
          backgroundColor: 'transparent',
          color: 'var(--wks-text-muted)',
          border: '1px dashed var(--wks-border-input)',
          borderRadius: '4px',
          cursor: 'pointer',
        }}
      >
        + Add script
      </button>

      <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '6px', marginTop: '2px' }}>
        <button
          onClick={onCancel}
          style={{
            padding: '3px 10px',
            fontSize: '0.65rem',
            fontFamily: 'inherit',
            border: '1px solid var(--wks-border)',
            borderRadius: '3px',
            backgroundColor: 'transparent',
            color: 'var(--wks-text-muted)',
            cursor: 'pointer',
          }}
        >
          Cancel
        </button>
        <button
          onClick={() => onSave(draft.filter((d) => d.name.trim() && d.command.trim()))}
          style={{
            padding: '3px 10px',
            fontSize: '0.65rem',
            fontFamily: 'inherit',
            fontWeight: 600,
            border: '1px solid var(--wks-accent)',
            borderRadius: '3px',
            backgroundColor: 'var(--wks-accent)',
            color: '#fff',
            cursor: 'pointer',
          }}
        >
          Save
        </button>
      </div>
    </div>
  );
};

function MenuButton({
  label,
  icon,
  onClick,
}: {
  label: string;
  icon?: React.ReactNode;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: '8px',
        width: '100%',
        padding: '4px 12px',
        margin: 0,
        border: 'none',
        borderRadius: 0,
        cursor: 'pointer',
        fontSize: '0.65rem',
        fontFamily: 'inherit',
        fontWeight: 400,
        backgroundColor: 'transparent',
        color: 'var(--wks-text-tertiary)',
        textAlign: 'left',
        height: 'auto',
        lineHeight: 1.4,
        transition: 'none',
      }}
      onMouseEnter={(e) => {
        (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-selected)';
        (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-primary)';
      }}
      onMouseLeave={(e) => {
        (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent';
        (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-tertiary)';
      }}
    >
      {icon && <span style={{ display: 'flex', alignItems: 'center', opacity: 0.85 }}>{icon}</span>}
      {label}
    </button>
  );
}

export default NavBar;
