import React, { useState, useCallback, useEffect } from 'react';
import { Circle, Star, X } from 'lucide-react';
import { Section, SmallButton, inputStyle } from './primitives';
import type { LibraryItem } from '../../types/library';
import { profileFormFields, configRootNote } from '../../lib/profileFields';
import {
  PROFILE_CAPS,
  PROFILE_PROVIDERS,
  profileProviderOf,
  type ProfileProvider,
} from '../../../../main/shared/agentProfiles';
import type { ProfileAccount } from '../../../../main/shared/ipcTypes';

interface ClaudeProfile {
  id: string;
  name: string;
  /** Which harness this profile configures. ABSENT MEANS CLAUDE — every row
   *  written before harnesses existed is a Claude one, and they are in daily
   *  use, so nothing migrates them. */
  provider?: ProfileProvider;
  configDir: string;
  extraArgs: string[];
  mcpItemIds?: string[];
  isDefault: boolean;
  /** Automatic-failover weight: 0 = manual only; heavier wins first when a
   *  session's account exhausts a usage window. */
  weight?: number;
  /** Codex only: `codex -p <name>`, a same-account settings preset. */
  preset?: string;
  /** Copilot only: the NAME of an env var holding a GitHub token, never the
   *  token itself. */
  tokenEnvVar?: string;
}

/** The greyed-out stand-in for a field this harness cannot honour. Rendering
 *  the reason IS the point — the alternative is an input that takes a value the
 *  write path throws away. */
const InertField: React.FC<{ label: string; why: string }> = ({ label, why }) => (
  <div
    style={{
      display: 'flex',
      alignItems: 'flex-start',
      gap: 8,
      padding: '4px 2px',
      opacity: 0.55,
    }}
  >
    <span
      style={{
        fontSize: '0.66rem',
        color: 'var(--wks-text-disabled)',
        minWidth: 92,
        paddingTop: 1,
        textDecoration: 'line-through',
      }}
    >
      {label}
    </span>
    <span style={{ fontSize: '0.64rem', color: 'var(--wks-text-faint)', lineHeight: 1.45 }}>
      {why}
    </span>
  </div>
);

const fieldHint: React.CSSProperties = {
  fontSize: '0.62rem',
  color: 'var(--wks-text-faint)',
  lineHeight: 1.45,
  margin: '0 0 2px 2px',
};

interface EditState {
  provider: ProfileProvider;
  name: string;
  configDir: string;
  args: string;
  weight: string;
  preset: string;
  tokenEnvVar: string;
  mcp: string[];
}

const ProfileEditForm: React.FC<{
  state: EditState;
  /** New rows may choose a harness; an existing row may not — the config root
   *  it names belongs to the harness that wrote it, and repointing CODEX_HOME
   *  at a Claude root is the exact mistake the provider field prevents. */
  canPickProvider: boolean;
  mcpItems: LibraryItem[];
  onChange: (patch: Partial<EditState>) => void;
  onToggleMcp: (id: string) => void;
  onSave: () => void;
  onCancel: () => void;
}> = ({ state, canPickProvider, mcpItems, onChange, onToggleMcp, onSave, onCancel }) => {
  const caps = PROFILE_CAPS[state.provider];
  const fields = profileFormFields(state.provider);
  const saveOnEnter = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') onSave();
  };
  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        gap: '4px',
        padding: '8px',
        backgroundColor: 'var(--wks-bg-surface)',
        borderRadius: '4px',
        border: '1px solid var(--wks-border-input)',
      }}
    >
      {canPickProvider && (
        <div style={{ display: 'flex', gap: 4, marginBottom: 2 }}>
          {PROFILE_PROVIDERS.map((p) => {
            const on = state.provider === p;
            return (
              <button
                key={p}
                onClick={() => onChange({ provider: p })}
                title={`A profile for ${PROFILE_CAPS[p].label} — sets ${PROFILE_CAPS[p].configRootEnv}`}
                style={{
                  padding: '3px 10px',
                  fontSize: '0.68rem',
                  fontFamily: 'inherit',
                  fontWeight: 500,
                  borderRadius: 'var(--wks-radius-pill)',
                  cursor: 'pointer',
                  border: on ? '1px solid var(--wks-accent)' : '1px solid var(--wks-border-input)',
                  background: on ? 'var(--wks-accent-bg)' : 'transparent',
                  color: on ? 'var(--wks-accent-text)' : 'var(--wks-text-tertiary)',
                }}
              >
                {PROFILE_CAPS[p].label}
              </button>
            );
          })}
        </div>
      )}
      <input
        value={state.name}
        onChange={(e) => onChange({ name: e.target.value })}
        placeholder="Profile name"
        style={inputStyle}
        autoFocus
      />
      <input
        value={state.configDir}
        onChange={(e) => onChange({ configDir: e.target.value })}
        placeholder={`Config dir (e.g. ${caps.configRootHint})`}
        style={{ ...inputStyle, fontFamily: 'var(--wks-font-mono)' }}
      />
      <div style={fieldHint}>{configRootNote(state.provider)}</div>
      <input
        value={state.args}
        onChange={(e) => onChange({ args: e.target.value })}
        placeholder="Extra args (e.g. --dangerously-skip-permissions)"
        style={{ ...inputStyle, fontFamily: 'var(--wks-font-mono)' }}
        onKeyDown={saveOnEnter}
      />

      {/* Codex only: a SAME-ACCOUNT settings preset, deliberately its own field
          beside the root rather than an alias for it. */}
      {fields.preset.shown && (
        <>
          <input
            value={state.preset}
            onChange={(e) => onChange({ preset: e.target.value })}
            placeholder={`Preset name (${caps.presetFlag} <name>, optional)`}
            style={{ ...inputStyle, fontFamily: 'var(--wks-font-mono)' }}
            onKeyDown={saveOnEnter}
          />
          <div style={fieldHint}>{caps.presetHint}</div>
        </>
      )}

      {/* Copilot only: the NAME of a variable, never the token. */}
      {fields.tokenEnvVar.shown && (
        <>
          <input
            value={state.tokenEnvVar}
            onChange={(e) => onChange({ tokenEnvVar: e.target.value })}
            placeholder="Token env var name (e.g. GH_TOKEN_WORK, optional)"
            style={{ ...inputStyle, fontFamily: 'var(--wks-font-mono)' }}
            onKeyDown={saveOnEnter}
          />
          <div style={fieldHint}>{caps.tokenEnvHint}</div>
        </>
      )}

      {fields.weight.shown && !fields.weight.disabled ? (
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <input
            value={state.weight}
            onChange={(e) => onChange({ weight: e.target.value })}
            placeholder="0"
            inputMode="numeric"
            style={{ ...inputStyle, width: 64 }}
            onKeyDown={saveOnEnter}
          />
          <span style={{ fontSize: '0.66rem', color: 'var(--wks-text-faint)', lineHeight: 1.4 }}>
            Failover weight — 0 keeps this profile manual. Any higher number joins the automatic
            rotation: when a session's account hits its usage window, it restarts onto the heaviest
            signed-in {caps.label} profile (same conversation) and cycles on until one works.
          </span>
        </div>
      ) : (
        <InertField label="Failover weight" why={fields.weight.why!} />
      )}

      {fields.mcpItemIds.disabled ? (
        <InertField label="Default MCP servers" why={fields.mcpItemIds.why!} />
      ) : (
        mcpItems.length > 0 && (
          <div>
            <div
              style={{
                fontSize: '0.72rem',
                color: 'var(--wks-text-disabled)',
                textTransform: 'uppercase',
                letterSpacing: '0.04em',
                margin: '4px 0 2px',
              }}
            >
              Default MCP servers
            </div>
            <div
              style={{
                display: 'flex',
                flexDirection: 'column',
                gap: 1,
                maxHeight: 120,
                overflowY: 'auto',
              }}
            >
              {mcpItems.map((it) => (
                <label
                  key={it.id}
                  title={it.description || it.id}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 6,
                    fontSize: '0.68rem',
                    color: 'var(--wks-text-secondary)',
                    cursor: 'pointer',
                    padding: '1px 0',
                  }}
                >
                  <input
                    type="checkbox"
                    checked={state.mcp.includes(it.id)}
                    onChange={() => onToggleMcp(it.id)}
                    style={{ cursor: 'pointer' }}
                  />
                  <span
                    style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}
                  >
                    {it.title}
                  </span>
                  <span style={{ fontSize: '0.55rem', color: 'var(--wks-text-faint)' }}>
                    {it.scope}
                  </span>
                </label>
              ))}
            </div>
          </div>
        )
      )}
      <div style={{ display: 'flex', gap: '4px', justifyContent: 'flex-end' }}>
        <SmallButton label="Cancel" onClick={onCancel} />
        <SmallButton label="Save" onClick={onSave} primary />
      </div>
    </div>
  );
};

const BLANK: EditState = {
  provider: 'claude',
  name: '',
  configDir: '',
  args: '',
  weight: '0',
  preset: '',
  tokenEnvVar: '',
  mcp: [],
};

/**
 * The identity line under a profile's name: what the harness's own credential
 * file says about it, never a guess.
 *
 * `signedIn` is TRI-STATE and must not be collapsed: undefined means the
 * harness keeps its credentials somewhere this app cannot read (Copilot's OS
 * credential store), which is a different sentence from "not signed in".
 */
function accountLine(account: ProfileAccount | undefined): { text: string; ok: boolean } | null {
  if (!account) return null;
  const caps = PROFILE_CAPS[account.provider];
  if (caps.tokenEnv) {
    // Copilot: the only answerable identity question is whether the referenced
    // variable actually resolves in THIS process — a name that resolves to
    // nothing means the agent silently runs as the default login.
    if (!account.tokenEnvVar) return { text: 'default login (no token referenced)', ok: true };
    return account.signedIn
      ? { text: `token from $${account.tokenEnvVar}`, ok: true }
      : { text: `$${account.tokenEnvVar} is not set — would run as the default login`, ok: false };
  }
  if (account.signedIn === false) return { text: 'log in on first dispatch', ok: false };
  if (account.signedIn === undefined) return null;
  const who = account.accountId ? `${account.accountId.slice(0, 8)}…` : 'signed in';
  return { text: account.authMode ? `${who} · ${account.authMode}` : who, ok: true };
}

const ClaudeProfilesSection: React.FC = () => {
  const [profiles, setProfiles] = useState<ClaudeProfile[]>([]);
  const [editing, setEditing] = useState<string | null>(null); // profile id or 'new'
  const [edit, setEdit] = useState<EditState>(BLANK);
  const [mcpItems, setMcpItems] = useState<LibraryItem[]>([]);
  // Second-account flow (desktop only — the API is absent on the web mirror).
  const canAddAccount = !!window.electronAPI.claudeProfilesAddAccount;
  // Per-harness profiles are gated on the SAME desktop-only API that reads the
  // identity behind them: the bus twin of claude.profiles.add models only the
  // four Claude positionals, so on the web mirror a codex/copilot profile would
  // be created and come back as a Claude one.
  const canPerHarness = !!window.electronAPI.claudeProfilesAccounts;
  const [addingAccount, setAddingAccount] = useState(false);
  const [accountName, setAccountName] = useState('');
  const [accountNote, setAccountNote] = useState<string | null>(null);
  const [loginStatus, setLoginStatus] = useState<Record<string, boolean>>({});
  const [accounts, setAccounts] = useState<Record<string, ProfileAccount>>({});

  const load = useCallback(() => {
    window.electronAPI.claudeProfilesList().then((p) => {
      // Normalize at the boundary: the headless (Go brain) provider marshals an
      // empty extraArgs as null, and a hand-edited profiles file may omit it.
      const list = ((p as ClaudeProfile[]) ?? []).map((prof) => ({
        ...prof,
        extraArgs: prof.extraArgs ?? [],
      }));
      setProfiles(list);
    });
    window.electronAPI
      .claudeProfilesLoginStatus?.()
      .then((m) => setLoginStatus(m ?? {}))
      .catch(() => {});
    window.electronAPI
      .claudeProfilesAccounts?.()
      .then((m) => setAccounts(m ?? {}))
      .catch(() => {});
  }, []);

  const addAccount = async () => {
    const name = accountName.trim() || 'Second account';
    try {
      const res = await window.electronAPI.claudeProfilesAddAccount!(name);
      setAccountNote(
        `“${name}” shares this machine's memories, history, skills and settings. ` +
          `Dispatch an agent with it — the pane will walk you through a one-time /login ` +
          `in its terminal, then both accounts run side by side.` +
          (res.warnings.length > 0 ? ` (${res.warnings.join('; ')})` : ''),
      );
    } catch (err) {
      setAccountNote(`Could not create the account: ${err instanceof Error ? err.message : err}`);
    }
    setAddingAccount(false);
    setAccountName('');
    load();
  };

  useEffect(() => {
    load();
  }, [load]);
  useEffect(() => {
    window.electronAPI
      .libraryList?.()
      .then((list) => setMcpItems((list ?? []).filter((it) => it.kind === 'mcp')))
      .catch(() => {});
  }, []);

  const patchEdit = (patch: Partial<EditState>) => setEdit((cur) => ({ ...cur, ...patch }));
  const toggleMcp = (id: string) =>
    setEdit((cur) => ({
      ...cur,
      mcp: cur.mcp.includes(id) ? cur.mcp.filter((x) => x !== id) : [...cur.mcp, id],
    }));

  const startEdit = (profile?: ClaudeProfile) => {
    if (profile) {
      setEditing(profile.id);
      setEdit({
        provider: profileProviderOf(profile),
        name: profile.name,
        configDir: profile.configDir,
        args: profile.extraArgs.join(' '),
        weight: String(profile.weight ?? 0),
        preset: profile.preset ?? '',
        tokenEnvVar: profile.tokenEnvVar ?? '',
        mcp: profile.mcpItemIds ?? [],
      });
    } else {
      setEditing('new');
      setEdit(BLANK);
    }
  };

  const cancelEdit = () => setEditing(null);

  const saveEdit = async () => {
    const args = edit.args.trim() ? edit.args.trim().split(/\s+/) : [];
    const fields = profileFormFields(edit.provider);
    // Only send what this harness can honour. Main clamps the rest anyway
    // (clampProfileWeight / normalizeProfile), but a UI that posts a value it
    // has just told the user is impossible is the same lie one layer up.
    const weight = fields.weight.disabled ? 0 : Math.max(0, Math.round(Number(edit.weight) || 0));
    const mcpItemIds = fields.mcpItemIds.disabled ? [] : edit.mcp;
    const preset = fields.preset.shown ? edit.preset.trim() : '';
    const tokenEnvVar = fields.tokenEnvVar.shown ? edit.tokenEnvVar.trim() : '';
    if (editing === 'new') {
      const added = await window.electronAPI.claudeProfilesAdd(
        edit.name || 'Profile',
        edit.configDir,
        args,
        mcpItemIds,
        // OMITTED at the Claude defaults: a Claude profile added here has to
        // come out byte-identical to one the Go brain adds over the bus, and
        // the brain models neither key.
        canPerHarness && edit.provider !== 'claude'
          ? { provider: edit.provider, preset, weight, tokenEnvVar }
          : undefined,
      );
      // Weight rides the update path so the add IPC (and its bus twin on the
      // brain) keeps its shape.
      if (weight > 0 && added?.id) {
        await window.electronAPI.claudeProfilesUpdate(added.id, { weight });
      }
    } else if (editing) {
      await window.electronAPI.claudeProfilesUpdate(editing, {
        name: edit.name,
        configDir: edit.configDir,
        extraArgs: args,
        mcpItemIds,
        weight,
        // Same omission rule on update: a Claude row must not gain the keys.
        ...(edit.provider === 'claude' ? {} : { preset, tokenEnvVar }),
      });
    }
    setEditing(null);
    load();
  };

  const setDefault = async (id: string) => {
    await window.electronAPI.claudeProfilesUpdate(id, { isDefault: true });
    load();
  };

  const remove = async (id: string) => {
    await window.electronAPI.claudeProfilesRemove(id);
    load();
  };

  return (
    <Section title="Agent Profiles">
      <div style={{ display: 'flex', flexDirection: 'column', gap: '4px' }}>
        {profiles.map((p) => {
          const provider = profileProviderOf(p);
          const caps = PROFILE_CAPS[provider];
          const identity = accountLine(accounts[p.id]);
          return (
            <div key={p.id}>
              {editing === p.id ? (
                <ProfileEditForm
                  state={edit}
                  canPickProvider={false}
                  mcpItems={mcpItems}
                  onChange={patchEdit}
                  onToggleMcp={toggleMcp}
                  onSave={saveEdit}
                  onCancel={cancelEdit}
                />
              ) : (
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: '8px',
                    padding: '4px 8px',
                    borderRadius: '4px',
                  }}
                  onMouseEnter={(e) => {
                    (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-hover)';
                  }}
                  onMouseLeave={(e) => {
                    (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent';
                  }}
                >
                  <span
                    style={{
                      width: '16px',
                      display: 'inline-flex',
                      alignItems: 'center',
                      justifyContent: 'center',
                      color: p.isDefault ? 'var(--wks-accent)' : 'var(--wks-text-disabled)',
                    }}
                  >
                    {p.isDefault ? (
                      <Star size={10} strokeWidth={2.25} />
                    ) : (
                      <Circle size={10} strokeWidth={2.25} />
                    )}
                  </span>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div
                      style={{
                        fontSize: '0.7rem',
                        color: 'var(--wks-text-secondary)',
                        fontWeight: 500,
                      }}
                    >
                      {p.name}
                      {/* Which harness this profile drives. Shown on every row,
                          including Claude's: once a list can hold three, an
                          unlabelled row is a guess. */}
                      <span
                        title={`Sets ${caps.configRootEnv}`}
                        style={{
                          fontSize: '0.58rem',
                          color: 'var(--wks-text-disabled)',
                          border: '1px solid var(--wks-border-subtle)',
                          borderRadius: 'var(--wks-radius-pill)',
                          padding: '0 5px',
                          marginLeft: 6,
                        }}
                      >
                        {caps.label}
                      </span>
                      {p.isDefault && (
                        <span
                          style={{ fontSize: '0.6rem', color: 'var(--wks-accent)', marginLeft: 6 }}
                        >
                          default
                        </span>
                      )}
                      {(p.weight ?? 0) > 0 && (
                        <span
                          title={`Automatic failover weight ${p.weight} — heavier wins first`}
                          style={{
                            fontSize: '0.6rem',
                            color: 'var(--wks-text-muted)',
                            marginLeft: 6,
                          }}
                        >
                          auto·{p.weight}
                        </span>
                      )}
                      {p.preset && (
                        <span
                          title={`${caps.presetFlag} ${p.preset} — a settings preset on the same account`}
                          style={{
                            fontSize: '0.6rem',
                            color: 'var(--wks-text-muted)',
                            marginLeft: 6,
                            fontFamily: 'var(--wks-font-mono)',
                          }}
                        >
                          {caps.presetFlag} {p.preset}
                        </span>
                      )}
                      {/* Identity, read from the harness's own credential file.
                          Falls back to the login probe for the rows the
                          accounts read has nothing to say about. */}
                      {identity ? (
                        <span
                          style={{
                            fontSize: '0.6rem',
                            color: identity.ok ? 'var(--wks-success)' : 'var(--wks-warning)',
                            marginLeft: 6,
                          }}
                        >
                          {identity.text}
                        </span>
                      ) : (
                        p.configDir &&
                        p.id in loginStatus && (
                          <span
                            style={{
                              fontSize: '0.6rem',
                              color: loginStatus[p.id]
                                ? 'var(--wks-success)'
                                : 'var(--wks-warning)',
                              marginLeft: 6,
                            }}
                          >
                            {loginStatus[p.id] ? 'signed in' : 'log in on first dispatch'}
                          </span>
                        )
                      )}
                    </div>
                    {p.configDir && (
                      <div
                        style={{
                          fontSize: '0.62rem',
                          color: 'var(--wks-text-faint)',
                          fontFamily: 'var(--wks-font-mono)',
                          overflow: 'hidden',
                          textOverflow: 'ellipsis',
                          whiteSpace: 'nowrap',
                        }}
                      >
                        {p.configDir}
                      </div>
                    )}
                    {p.extraArgs.length > 0 && (
                      <div
                        style={{
                          fontSize: '0.72rem',
                          color: 'var(--wks-text-disabled)',
                          fontFamily: 'var(--wks-font-mono)',
                        }}
                      >
                        {p.extraArgs.join(' ')}
                      </div>
                    )}
                  </div>
                  {!p.isDefault && <SmallButton label="Default" onClick={() => setDefault(p.id)} />}
                  <SmallButton label="Edit" onClick={() => startEdit(p)} />
                  {p.id !== 'default' && (
                    <SmallButton
                      label={<X size={11} strokeWidth={2.25} />}
                      onClick={() => remove(p.id)}
                      danger
                    />
                  )}
                </div>
              )}
            </div>
          );
        })}

        {accountNote && (
          <div
            style={{
              fontSize: '0.66rem',
              color: 'var(--wks-text-muted)',
              lineHeight: 1.5,
              padding: '6px 8px',
              border: '1px solid var(--wks-border-subtle)',
              borderRadius: '4px',
            }}
          >
            {accountNote}
          </div>
        )}

        {addingAccount && (
          <div
            style={{
              display: 'flex',
              flexDirection: 'column',
              gap: '4px',
              padding: '8px',
              backgroundColor: 'var(--wks-bg-surface)',
              borderRadius: '4px',
              border: '1px solid var(--wks-border-input)',
            }}
          >
            <input
              value={accountName}
              onChange={(e) => setAccountName(e.target.value)}
              placeholder="Account name (e.g. Work)"
              style={inputStyle}
              autoFocus
              onKeyDown={(e) => {
                if (e.key === 'Enter') void addAccount();
                if (e.key === 'Escape') setAddingAccount(false);
              }}
            />
            <div style={{ fontSize: '0.64rem', color: 'var(--wks-text-faint)', lineHeight: 1.5 }}>
              Creates a profile with its own Claude login that shares this machine's memories,
              session history, skills and settings. You'll log in inside the first agent you
              dispatch with it.
            </div>
            <div style={{ display: 'flex', gap: '4px', justifyContent: 'flex-end' }}>
              <SmallButton label="Cancel" onClick={() => setAddingAccount(false)} />
              <SmallButton label="Create" onClick={() => void addAccount()} primary />
            </div>
          </div>
        )}

        {editing === 'new' && (
          <ProfileEditForm
            state={edit}
            canPickProvider={canPerHarness}
            mcpItems={mcpItems}
            onChange={patchEdit}
            onToggleMcp={toggleMcp}
            onSave={saveEdit}
            onCancel={cancelEdit}
          />
        )}
        {editing !== 'new' && !addingAccount && (
          <div style={{ display: 'flex', gap: '4px' }}>
            <button onClick={() => startEdit()} style={dashedAddStyle} {...dashedHover}>
              + Add Profile
            </button>
            {canAddAccount && (
              <button
                onClick={() => {
                  setAccountNote(null);
                  setAddingAccount(true);
                }}
                title="A second Claude login that shares this machine's memories, history and skills"
                style={dashedAddStyle}
                {...dashedHover}
              >
                + Add Claude Account
              </button>
            )}
          </div>
        )}
      </div>
    </Section>
  );
};

const dashedAddStyle: React.CSSProperties = {
  padding: '6px 12px',
  fontSize: '0.68rem',
  fontFamily: 'inherit',
  fontWeight: 500,
  backgroundColor: 'transparent',
  color: 'var(--wks-text-muted)',
  border: '1px dashed var(--wks-border-input)',
  borderRadius: '4px',
  cursor: 'pointer',
  height: 'auto',
  lineHeight: 1.4,
  margin: '4px 0 0',
  flex: 1,
};

/** The shared hover affordance of the dashed add buttons. */
const dashedHover = {
  onMouseEnter: (e: React.MouseEvent) => {
    (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-accent)';
    (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-secondary)';
  },
  onMouseLeave: (e: React.MouseEvent) => {
    (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-border-input)';
    (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-muted)';
  },
};

export default ClaudeProfilesSection;
