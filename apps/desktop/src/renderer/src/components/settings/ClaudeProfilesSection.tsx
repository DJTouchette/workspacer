import React, { useState, useCallback, useEffect } from 'react';
import { Circle, Star, X } from 'lucide-react';
import { Section, SmallButton, inputStyle } from './primitives';
import type { LibraryItem } from '../../types/library';

interface ClaudeProfile {
  id: string;
  name: string;
  configDir: string;
  extraArgs: string[];
  mcpItemIds?: string[];
  isDefault: boolean;
}

const ProfileEditForm: React.FC<{
  name: string;
  configDir: string;
  args: string;
  mcpItems: LibraryItem[];
  mcpSel: string[];
  onToggleMcp: (id: string) => void;
  onNameChange: (v: string) => void;
  onConfigDirChange: (v: string) => void;
  onArgsChange: (v: string) => void;
  onSave: () => void;
  onCancel: () => void;
}> = ({
  name,
  configDir,
  args,
  mcpItems,
  mcpSel,
  onToggleMcp,
  onNameChange,
  onConfigDirChange,
  onArgsChange,
  onSave,
  onCancel,
}) => (
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
      value={name}
      onChange={(e) => onNameChange(e.target.value)}
      placeholder="Profile name"
      style={inputStyle}
      autoFocus
    />
    <input
      value={configDir}
      onChange={(e) => onConfigDirChange(e.target.value)}
      placeholder="Config dir (e.g. ~/.claude-work, blank = default)"
      style={{ ...inputStyle, fontFamily: 'var(--wks-font-mono)' }}
    />
    <input
      value={args}
      onChange={(e) => onArgsChange(e.target.value)}
      placeholder="Extra args (e.g. --dangerously-skip-permissions)"
      style={{ ...inputStyle, fontFamily: 'var(--wks-font-mono)' }}
      onKeyDown={(e) => {
        if (e.key === 'Enter') onSave();
      }}
    />
    {mcpItems.length > 0 && (
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
                checked={mcpSel.includes(it.id)}
                onChange={() => onToggleMcp(it.id)}
                style={{ cursor: 'pointer' }}
              />
              <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                {it.title}
              </span>
              <span style={{ fontSize: '0.55rem', color: 'var(--wks-text-faint)' }}>
                {it.scope}
              </span>
            </label>
          ))}
        </div>
      </div>
    )}
    <div style={{ display: 'flex', gap: '4px', justifyContent: 'flex-end' }}>
      <SmallButton label="Cancel" onClick={onCancel} />
      <SmallButton label="Save" onClick={onSave} primary />
    </div>
  </div>
);

const ClaudeProfilesSection: React.FC = () => {
  const [profiles, setProfiles] = useState<ClaudeProfile[]>([]);
  const [editing, setEditing] = useState<string | null>(null); // profile id or 'new'
  const [editName, setEditName] = useState('');
  const [editConfigDir, setEditConfigDir] = useState('');
  const [editArgs, setEditArgs] = useState('');
  const [editMcp, setEditMcp] = useState<string[]>([]);
  const [mcpItems, setMcpItems] = useState<LibraryItem[]>([]);
  // Second-account flow (desktop only — the API is absent on the web mirror).
  const canAddAccount = !!window.electronAPI.claudeProfilesAddAccount;
  const [addingAccount, setAddingAccount] = useState(false);
  const [accountName, setAccountName] = useState('');
  const [accountNote, setAccountNote] = useState<string | null>(null);
  const [loginStatus, setLoginStatus] = useState<Record<string, boolean>>({});

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
  }, []);

  const addAccount = async () => {
    const name = accountName.trim() || 'Second account';
    try {
      const res = await window.electronAPI.claudeProfilesAddAccount!(name);
      setAccountNote(
        `“${name}” shares this machine's memories, history, skills and settings. ` +
          `Spawn an agent with it — the pane will walk you through a one-time /login ` +
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

  const toggleMcp = (id: string) =>
    setEditMcp((sel) => (sel.includes(id) ? sel.filter((x) => x !== id) : [...sel, id]));

  const startEdit = (profile?: ClaudeProfile) => {
    if (profile) {
      setEditing(profile.id);
      setEditName(profile.name);
      setEditConfigDir(profile.configDir);
      setEditArgs(profile.extraArgs.join(' '));
      setEditMcp(profile.mcpItemIds ?? []);
    } else {
      setEditing('new');
      setEditName('');
      setEditConfigDir('');
      setEditArgs('');
      setEditMcp([]);
    }
  };

  const cancelEdit = () => setEditing(null);

  const saveEdit = async () => {
    const args = editArgs.trim() ? editArgs.trim().split(/\s+/) : [];
    if (editing === 'new') {
      await window.electronAPI.claudeProfilesAdd(
        editName || 'Profile',
        editConfigDir,
        args,
        editMcp,
      );
    } else if (editing) {
      await window.electronAPI.claudeProfilesUpdate(editing, {
        name: editName,
        configDir: editConfigDir,
        extraArgs: args,
        mcpItemIds: editMcp,
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
    <Section title="Claude Code">
      <div style={{ display: 'flex', flexDirection: 'column', gap: '4px' }}>
        {profiles.map((p) => (
          <div key={p.id}>
            {editing === p.id ? (
              <ProfileEditForm
                name={editName}
                configDir={editConfigDir}
                args={editArgs}
                mcpItems={mcpItems}
                mcpSel={editMcp}
                onToggleMcp={toggleMcp}
                onNameChange={setEditName}
                onConfigDirChange={setEditConfigDir}
                onArgsChange={setEditArgs}
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
                    {p.isDefault && (
                      <span
                        style={{ fontSize: '0.6rem', color: 'var(--wks-accent)', marginLeft: 6 }}
                      >
                        default
                      </span>
                    )}
                    {/* Own-login profiles (a configDir) show whether anyone has
                        signed in there yet; the login itself happens in the
                        first agent pane spawned with the profile. */}
                    {p.configDir && p.id in loginStatus && (
                      <span
                        style={{
                          fontSize: '0.6rem',
                          color: loginStatus[p.id] ? 'var(--wks-success)' : 'var(--wks-warning)',
                          marginLeft: 6,
                        }}
                      >
                        {loginStatus[p.id] ? 'signed in' : 'log in on first spawn'}
                      </span>
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
        ))}

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
            <div
              style={{ fontSize: '0.64rem', color: 'var(--wks-text-faint)', lineHeight: 1.5 }}
            >
              Creates a profile with its own Claude login that shares this machine's memories,
              session history, skills and settings. You'll log in inside the first agent you spawn
              with it.
            </div>
            <div style={{ display: 'flex', gap: '4px', justifyContent: 'flex-end' }}>
              <SmallButton label="Cancel" onClick={() => setAddingAccount(false)} />
              <SmallButton label="Create" onClick={() => void addAccount()} primary />
            </div>
          </div>
        )}

        {editing === 'new' && (
          <ProfileEditForm
            name={editName}
            configDir={editConfigDir}
            args={editArgs}
            mcpItems={mcpItems}
            mcpSel={editMcp}
            onToggleMcp={toggleMcp}
            onNameChange={setEditName}
            onConfigDirChange={setEditConfigDir}
            onArgsChange={setEditArgs}
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
