import React, { useState, useEffect, useCallback } from 'react';
import { claudeColors as colors, ensureKeyframes } from '../components/claude-shared';

// ── Types (mirrors main process) ──

interface Account { id: string; provider: string; label: string; config: Record<string, string> }
interface ProviderInfo { id: string; name: string; configFields: Array<{ key: string; label: string; placeholder: string; type: string; required: boolean }>; tokenField: { label: string; placeholder: string; helpText?: string } }
interface PR { id: string; number: number; title: string; description: string; status: string; sourceBranch: string; targetBranch: string; author: string; reviewers: Array<{ name: string; vote: string }>; url: string; isDraft: boolean; mergeConflicts: boolean; created: string }
interface Pipeline { id: string; name: string; status: string; sourceBranch: string; commitSha: string; author: string; url: string; startedAt: string; finishedAt: string; duration?: number }
interface Repo { id: string; name: string; defaultBranch: string; url: string }

type View =
  | { kind: 'home'; accountId: string }
  | { kind: 'accounts' }
  | { kind: 'add-account' };

// ── Status colors ──

function prStatusColor(s: string): string {
  if (s === 'open') return colors.success;
  if (s === 'merged') return '#c084fc';
  if (s === 'draft') return colors.muted;
  return colors.error;
}

function pipelineColor(s: string): string {
  if (s === 'succeeded') return colors.success;
  if (s === 'failed') return colors.error;
  if (s === 'running') return colors.accent;
  if (s === 'queued') return colors.warning;
  return colors.muted;
}

function reviewColor(v: string): string {
  if (v === 'approved') return colors.success;
  if (v === 'rejected') return colors.error;
  if (v === 'waiting') return colors.warning;
  return colors.muted;
}

// ── Shared styles ──

const inputStyle: React.CSSProperties = {
  padding: '6px 10px', borderRadius: 6, border: `1px solid ${colors.borderSubtle}`,
  backgroundColor: 'rgba(255,255,255,0.03)', color: colors.text, fontSize: '0.75rem',
  outline: 'none', fontFamily: 'inherit', width: '100%', boxSizing: 'border-box',
};

function btnStyle(color: string): React.CSSProperties {
  return { padding: '5px 14px', borderRadius: 6, border: `1px solid ${color}`, backgroundColor: 'transparent', color, fontSize: '0.7rem', fontWeight: 600, cursor: 'pointer', fontFamily: 'inherit' };
}

// ── Add Account Form ──

const AddAccountForm: React.FC<{ providers: ProviderInfo[]; onDone: () => void; onCancel: () => void }> = ({ providers, onDone, onCancel }) => {
  const [pid, setPid] = useState(providers[0]?.id ?? '');
  const [label, setLabel] = useState('');
  const [config, setConfig] = useState<Record<string, string>>({});
  const [token, setToken] = useState('');
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const prov = providers.find(p => p.id === pid);

  const submit = async () => {
    setError(''); setLoading(true);
    try { await window.electronAPI.devopsAddAccount(pid, label || prov?.name || 'Account', config, token); onDone(); }
    catch (e: any) { setError(e?.message ?? 'Failed'); }
    finally { setLoading(false); }
  };

  return (
    <div style={{ padding: '20px 24px', maxWidth: 480 }}>
      <div style={{ fontSize: '0.9rem', fontWeight: 700, color: colors.textBright, marginBottom: 16 }}>Connect Git Provider</div>
      {providers.length > 1 && (
        <div style={{ marginBottom: 12 }}>
          <Label>Provider</Label>
          <select value={pid} onChange={e => { setPid(e.target.value); setConfig({}); }} style={inputStyle}>
            {providers.map(p => <option key={p.id} value={p.id}>{p.name}</option>)}
          </select>
        </div>
      )}
      <div style={{ marginBottom: 12 }}><Label>Label</Label><input value={label} onChange={e => setLabel(e.target.value)} placeholder={prov?.name} style={inputStyle} /></div>
      {prov?.configFields.map(f => (
        <div key={f.key} style={{ marginBottom: 12 }}>
          <Label>{f.label}{f.required && ' *'}</Label>
          <input type={f.type} value={config[f.key] ?? ''} onChange={e => setConfig({ ...config, [f.key]: e.target.value })} placeholder={f.placeholder} style={inputStyle} />
        </div>
      ))}
      <div style={{ marginBottom: 12 }}>
        <Label>{prov?.tokenField.label ?? 'Token'} *</Label>
        <input type="password" value={token} onChange={e => setToken(e.target.value)} placeholder={prov?.tokenField.placeholder} style={inputStyle} />
        {prov?.tokenField.helpText && <div style={{ fontSize: '0.6rem', color: colors.muted, marginTop: 4 }}>{prov.tokenField.helpText}</div>}
      </div>
      {error && <div style={{ fontSize: '0.72rem', color: colors.error, marginBottom: 12 }}>{error}</div>}
      <div style={{ display: 'flex', gap: 8 }}>
        <button onClick={submit} disabled={loading || !token} style={{ ...btnStyle(colors.accent), opacity: token && !loading ? 1 : 0.4 }}>{loading ? 'Connecting...' : 'Connect'}</button>
        <button onClick={onCancel} style={btnStyle(colors.muted)}>Cancel</button>
      </div>
    </div>
  );
};

const Label: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <div style={{ fontSize: '0.68rem', color: colors.muted, marginBottom: 4, fontWeight: 500 }}>{children}</div>
);

// ── PR Card ──

const PRCard: React.FC<{ pr: PR }> = ({ pr }) => (
  <div style={{
    padding: '8px 12px', borderRadius: 6, border: `1px solid ${colors.borderSubtle}`,
    backgroundColor: 'rgba(255,255,255,0.02)', display: 'flex', alignItems: 'center', gap: 8,
  }}>
    <span style={{ width: 6, height: 6, borderRadius: '50%', backgroundColor: prStatusColor(pr.status), flexShrink: 0 }} />
    <div style={{ flex: 1, minWidth: 0 }}>
      <div style={{ fontSize: '0.75rem', color: colors.textBright, fontWeight: 500, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
        {pr.isDraft && <span style={{ color: colors.muted, fontWeight: 400 }}>[Draft] </span>}
        {pr.title}
      </div>
      <div style={{ fontSize: '0.6rem', color: colors.muted, marginTop: 2, display: 'flex', gap: 8 }}>
        <span>{pr.sourceBranch} {'\u2192'} {pr.targetBranch}</span>
        <span>by {pr.author}</span>
        {pr.mergeConflicts && <span style={{ color: colors.error }}>Conflicts</span>}
      </div>
    </div>
    <div style={{ display: 'flex', gap: 3, flexShrink: 0 }}>
      {pr.reviewers.slice(0, 4).map((r, i) => (
        <span key={i} title={`${r.name}: ${r.vote}`} style={{
          width: 18, height: 18, borderRadius: '50%', display: 'flex', alignItems: 'center', justifyContent: 'center',
          fontSize: '0.5rem', fontWeight: 700, backgroundColor: 'rgba(255,255,255,0.06)',
          border: `1.5px solid ${reviewColor(r.vote)}`, color: reviewColor(r.vote),
        }}>
          {r.vote === 'approved' ? '\u2713' : r.vote === 'rejected' ? '\u2717' : '\u2022'}
        </span>
      ))}
    </div>
    <span style={{ fontSize: '0.6rem', padding: '1px 6px', borderRadius: 8, backgroundColor: 'rgba(255,255,255,0.05)', color: prStatusColor(pr.status), fontWeight: 600, flexShrink: 0 }}>
      #{pr.number}
    </span>
  </div>
);

// ── Pipeline Card ──

function formatDuration(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
  return `${Math.floor(seconds / 3600)}h ${Math.floor((seconds % 3600) / 60)}m`;
}

function formatTimeAgo(isoDate: string): string {
  const diff = Math.floor((Date.now() - new Date(isoDate).getTime()) / 1000);
  if (diff < 60) return `${diff}s ago`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}

const PipelineCard: React.FC<{ p: Pipeline }> = ({ p }) => {
  // For finished: show duration. For running: show elapsed since start.
  const [elapsed, setElapsed] = useState(0);
  const isRunning = p.status === 'running' || p.status === 'queued';

  useEffect(() => {
    if (!isRunning || !p.startedAt) return;
    const tick = () => setElapsed(Math.floor((Date.now() - new Date(p.startedAt).getTime()) / 1000));
    tick();
    const iv = setInterval(tick, 1000);
    return () => clearInterval(iv);
  }, [isRunning, p.startedAt]);

  const durStr = isRunning && p.startedAt
    ? formatDuration(elapsed)
    : p.duration
      ? formatDuration(p.duration)
      : p.startedAt ? formatTimeAgo(p.startedAt) : '';

  return (
    <div style={{
      padding: '6px 12px', borderRadius: 6, border: `1px solid ${colors.borderSubtle}`,
      backgroundColor: 'rgba(255,255,255,0.02)', display: 'flex', alignItems: 'center', gap: 8,
    }}>
      {isRunning ? (
        <span style={{ width: 12, height: 12, border: `1.5px solid ${colors.accent}`, borderTopColor: 'transparent', borderRadius: '50%', animation: 'claudeSpinner 0.8s linear infinite', flexShrink: 0 }} />
      ) : (
        <span style={{ width: 8, height: 8, borderRadius: '50%', backgroundColor: pipelineColor(p.status), flexShrink: 0 }} />
      )}
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: '0.72rem', color: colors.textBright, fontWeight: 500, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {p.name}
        </div>
        <div style={{ fontSize: '0.58rem', color: colors.muted, marginTop: 1 }}>
          {p.sourceBranch} {p.commitSha && `\u00B7 ${p.commitSha}`} {p.author && `\u00B7 ${p.author}`}
        </div>
      </div>
      <div style={{ textAlign: 'right', flexShrink: 0 }}>
        <div style={{ fontSize: '0.6rem', color: pipelineColor(p.status), fontWeight: 600 }}>{p.status}</div>
        {durStr && <div style={{ fontSize: '0.55rem', color: isRunning ? colors.accent : colors.muted, fontVariantNumeric: 'tabular-nums' }}>{durStr}</div>}
      </div>
    </div>
  );
};

// ── Home view (PRs + Pipelines for an account) ──

const HomeView: React.FC<{ accountId: string; onBack: () => void }> = ({ accountId, onBack }) => {
  const [prs, setPrs] = useState<PR[]>([]);
  const [pipelines, setPipelines] = useState<Pipeline[]>([]);
  const [loading, setLoading] = useState(true);
  const [tab, setTab] = useState<'prs' | 'pipelines'>('prs');
  const [selectedPR, setSelectedPR] = useState<PR | null>(null);
  const [selectedPipeline, setSelectedPipeline] = useState<Pipeline | null>(null);

  useEffect(() => {
    setLoading(true);
    Promise.all([
      window.electronAPI.devopsListPRs(accountId, { status: 'open' }),
      window.electronAPI.devopsListPipelines(accountId, { maxResults: 30 }),
    ]).then(([p, pl]) => { setPrs(p); setPipelines(pl); })
      .catch(e => console.error('[DevOpsPane] load failed:', e))
      .finally(() => setLoading(false));
  }, [accountId]);

  return (
    <div style={{ padding: '20px 24px' }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 16 }}>
        <button onClick={onBack} style={{ ...btnStyle(colors.muted), padding: '2px 8px' }}>{'\u2190'}</button>
        <div style={{ fontSize: '0.9rem', fontWeight: 700, color: colors.textBright }}>DevOps</div>
      </div>

      {/* Tabs */}
      <div style={{ display: 'flex', gap: 4, marginBottom: 14 }}>
        {(['prs', 'pipelines'] as const).map(t => (
          <button key={t} onClick={() => setTab(t)} style={{
            padding: '4px 12px', borderRadius: 6, border: `1px solid ${tab === t ? colors.accent : colors.borderSubtle}`,
            backgroundColor: tab === t ? 'rgba(255,255,255,0.05)' : 'transparent',
            color: tab === t ? colors.accent : colors.muted, fontSize: '0.7rem', fontWeight: 600, cursor: 'pointer', fontFamily: 'inherit',
          }}>
            {t === 'prs' ? `Pull Requests (${prs.length})` : `Pipelines (${pipelines.length})`}
          </button>
        ))}
      </div>

      {loading && <div style={{ color: colors.muted, fontSize: '0.75rem' }}>Loading...</div>}

      {!loading && tab === 'prs' && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          {prs.length === 0 && <div style={{ color: colors.muted, fontSize: '0.75rem' }}>No open pull requests</div>}
          {prs.map(pr => <div key={pr.id} onClick={() => setSelectedPR(pr)} style={{ cursor: 'pointer' }}><PRCard pr={pr} /></div>)}
        </div>
      )}

      {!loading && tab === 'pipelines' && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          {pipelines.length === 0 && <div style={{ color: colors.muted, fontSize: '0.75rem' }}>No recent pipelines</div>}
          {pipelines.map(p => <div key={p.id} onClick={() => setSelectedPipeline(p)} style={{ cursor: 'pointer' }}><PipelineCard p={p} /></div>)}
        </div>
      )}

      {/* PR Detail Popup */}
      {selectedPR && (
        <div onClick={() => setSelectedPR(null)} style={{ position: 'fixed', inset: 0, zIndex: 200, display: 'flex', alignItems: 'center', justifyContent: 'center', backgroundColor: 'rgba(0,0,0,0.5)' }}>
          <div onClick={e => e.stopPropagation()} style={{ width: 500, maxHeight: '75vh', overflow: 'auto', borderRadius: 10, border: `1px solid ${colors.border}`, backgroundColor: 'var(--wks-bg-surface)', padding: '16px 20px' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 12 }}>
              <span style={{ width: 8, height: 8, borderRadius: '50%', backgroundColor: prStatusColor(selectedPR.status) }} />
              <span style={{ fontSize: '0.82rem', fontWeight: 600, color: colors.textBright, flex: 1 }}>
                {selectedPR.isDraft && <span style={{ color: colors.muted, fontWeight: 400 }}>[Draft] </span>}
                #{selectedPR.number} {selectedPR.title}
              </span>
              <span onClick={() => setSelectedPR(null)} style={{ cursor: 'pointer', color: colors.muted, fontSize: '0.85rem' }}>{'\u00D7'}</span>
            </div>
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '6px 16px', fontSize: '0.68rem', marginBottom: 12 }}>
              <div><span style={{ color: colors.muted }}>From: </span><span style={{ color: colors.text, fontFamily: 'monospace' }}>{selectedPR.sourceBranch}</span></div>
              <div><span style={{ color: colors.muted }}>To: </span><span style={{ color: colors.text, fontFamily: 'monospace' }}>{selectedPR.targetBranch}</span></div>
              <div><span style={{ color: colors.muted }}>Author: </span><span style={{ color: colors.text }}>{selectedPR.author}</span></div>
              <div><span style={{ color: colors.muted }}>Status: </span><span style={{ color: prStatusColor(selectedPR.status), fontWeight: 600 }}>{selectedPR.status}</span></div>
              {selectedPR.mergeConflicts && <div style={{ color: colors.error, fontWeight: 600, gridColumn: 'span 2' }}>Has merge conflicts</div>}
              <div><span style={{ color: colors.muted }}>Created: </span><span style={{ color: colors.text }}>{new Date(selectedPR.created).toLocaleString()}</span></div>
            </div>
            {selectedPR.reviewers.length > 0 && (
              <div style={{ marginBottom: 12 }}>
                <div style={{ fontSize: '0.62rem', color: colors.muted, fontWeight: 600, marginBottom: 4 }}>Reviewers</div>
                {selectedPR.reviewers.map((r, i) => (
                  <div key={i} style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '2px 0', fontSize: '0.68rem' }}>
                    <span style={{ width: 14, height: 14, borderRadius: '50%', display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: '0.5rem', fontWeight: 700, border: `1.5px solid ${reviewColor(r.vote)}`, color: reviewColor(r.vote) }}>
                      {r.vote === 'approved' ? '\u2713' : r.vote === 'rejected' ? '\u2717' : '\u2022'}
                    </span>
                    <span style={{ color: colors.text }}>{r.name}</span>
                    <span style={{ color: reviewColor(r.vote), fontSize: '0.6rem' }}>{r.vote}</span>
                  </div>
                ))}
              </div>
            )}
            {selectedPR.description && (
              <div style={{ fontSize: '0.72rem', lineHeight: 1.5, color: colors.text, whiteSpace: 'pre-wrap', padding: '10px 12px', borderRadius: 6, border: `1px solid ${colors.borderSubtle}`, backgroundColor: 'rgba(255,255,255,0.02)', maxHeight: 200, overflow: 'auto', marginBottom: 12 }}>
                {selectedPR.description}
              </div>
            )}
            {selectedPR.url && (
              <div onClick={() => navigator.clipboard.writeText(selectedPR.url)} style={{ padding: '6px 10px', borderRadius: 6, border: `1px solid ${colors.borderSubtle}`, backgroundColor: 'rgba(255,255,255,0.02)', fontSize: '0.62rem', color: colors.accent, fontFamily: 'monospace', cursor: 'pointer', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title="Click to copy URL">
                {selectedPR.url}
              </div>
            )}
          </div>
        </div>
      )}

      {/* Pipeline Detail Popup */}
      {selectedPipeline && (
        <div onClick={() => setSelectedPipeline(null)} style={{ position: 'fixed', inset: 0, zIndex: 200, display: 'flex', alignItems: 'center', justifyContent: 'center', backgroundColor: 'rgba(0,0,0,0.5)' }}>
          <div onClick={e => e.stopPropagation()} style={{ width: 460, maxHeight: '70vh', overflow: 'auto', borderRadius: 10, border: `1px solid ${colors.border}`, backgroundColor: 'var(--wks-bg-surface)', padding: '16px 20px' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 12 }}>
              <span style={{ width: 8, height: 8, borderRadius: '50%', backgroundColor: pipelineColor(selectedPipeline.status) }} />
              <span style={{ fontSize: '0.82rem', fontWeight: 600, color: colors.textBright, flex: 1 }}>{selectedPipeline.name}</span>
              <span style={{ fontSize: '0.6rem', fontWeight: 600, padding: '2px 8px', borderRadius: 10, backgroundColor: 'rgba(255,255,255,0.05)', color: pipelineColor(selectedPipeline.status) }}>{selectedPipeline.status}</span>
              <span onClick={() => setSelectedPipeline(null)} style={{ cursor: 'pointer', color: colors.muted, fontSize: '0.85rem' }}>{'\u00D7'}</span>
            </div>
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '6px 16px', fontSize: '0.68rem', marginBottom: 14 }}>
              <div><span style={{ color: colors.muted }}>Branch: </span><span style={{ color: colors.text, fontFamily: 'monospace' }}>{selectedPipeline.sourceBranch}</span></div>
              {selectedPipeline.commitSha && <div><span style={{ color: colors.muted }}>Commit: </span><span style={{ color: colors.text, fontFamily: 'monospace' }}>{selectedPipeline.commitSha}</span></div>}
              {selectedPipeline.author && <div><span style={{ color: colors.muted }}>Author: </span><span style={{ color: colors.text }}>{selectedPipeline.author}</span></div>}
              {selectedPipeline.startedAt && <div><span style={{ color: colors.muted }}>Started: </span><span style={{ color: colors.text }}>{new Date(selectedPipeline.startedAt).toLocaleString()}</span></div>}
              {selectedPipeline.finishedAt && <div><span style={{ color: colors.muted }}>Finished: </span><span style={{ color: colors.text }}>{new Date(selectedPipeline.finishedAt).toLocaleString()}</span></div>}
              {selectedPipeline.duration && <div><span style={{ color: colors.muted }}>Duration: </span><span style={{ color: colors.text }}>{selectedPipeline.duration < 60 ? `${selectedPipeline.duration}s` : `${Math.floor(selectedPipeline.duration / 60)}m ${selectedPipeline.duration % 60}s`}</span></div>}
            </div>
            {selectedPipeline.url && (
              <div onClick={() => navigator.clipboard.writeText(selectedPipeline.url)} style={{ padding: '6px 10px', borderRadius: 6, border: `1px solid ${colors.borderSubtle}`, backgroundColor: 'rgba(255,255,255,0.02)', fontSize: '0.62rem', color: colors.accent, fontFamily: 'monospace', cursor: 'pointer', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title="Click to copy URL">
                {selectedPipeline.url}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
};

// ── Main ──

interface DevOpsPaneProps { paneId: string; title: string; isActive: boolean }

const DevOpsPane: React.FC<DevOpsPaneProps> = () => {
  const [view, setView] = useState<View>({ kind: 'accounts' });
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);

  useEffect(() => { ensureKeyframes(); }, []);

  const refresh = useCallback(() => { window.electronAPI.devopsGetAccounts().then(setAccounts); }, []);

  useEffect(() => {
    window.electronAPI.devopsGetProviders().then(setProviders);
    refresh();
  }, [refresh]);

  return (
    <div style={{ width: '100%', height: '100%', overflow: 'auto', backgroundColor: colors.bg, color: colors.text, fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif' }}>
      {view.kind === 'accounts' && (
        <div style={{ padding: '20px 24px' }}>
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 16 }}>
            <div style={{ fontSize: '0.9rem', fontWeight: 700, color: colors.textBright }}>Git & Pipelines</div>
            <button onClick={() => setView({ kind: 'add-account' })} style={btnStyle(colors.accent)}>+ Connect</button>
          </div>
          {accounts.length === 0 && (
            <div style={{ textAlign: 'center', marginTop: 40, color: colors.muted }}>
              <div style={{ fontSize: '1.5rem', marginBottom: 8, opacity: 0.4 }}>{'\u{1F527}'}</div>
              <div style={{ fontSize: '0.8rem' }}>No git providers connected</div>
              <div style={{ fontSize: '0.7rem', marginTop: 4 }}>Connect Azure DevOps to see PRs and pipelines</div>
            </div>
          )}
          {accounts.map(a => (
            <div key={a.id} onClick={() => setView({ kind: 'home', accountId: a.id })} style={{
              padding: '12px 14px', borderRadius: 8, border: `1px solid ${colors.borderSubtle}`,
              backgroundColor: 'rgba(255,255,255,0.02)', cursor: 'pointer', marginBottom: 8,
              display: 'flex', alignItems: 'center', justifyContent: 'space-between',
            }}>
              <div>
                <div style={{ fontSize: '0.8rem', fontWeight: 600, color: colors.textBright }}>{a.label}</div>
                <div style={{ fontSize: '0.65rem', color: colors.muted, marginTop: 2 }}>{a.provider} {'\u00B7'} {a.config.org}/{a.config.project}</div>
              </div>
              <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
                <span style={{ fontSize: '0.65rem', color: colors.accent }}>{'\u2192'}</span>
                <span onClick={e => { e.stopPropagation(); window.electronAPI.devopsRemoveAccount(a.id).then(refresh); }} style={{ fontSize: '0.7rem', color: colors.error, cursor: 'pointer' }}>{'\u00D7'}</span>
              </div>
            </div>
          ))}
        </div>
      )}
      {view.kind === 'add-account' && <AddAccountForm providers={providers} onDone={() => { refresh(); setView({ kind: 'accounts' }); }} onCancel={() => setView({ kind: 'accounts' })} />}
      {view.kind === 'home' && <HomeView accountId={view.accountId} onBack={() => setView({ kind: 'accounts' })} />}
    </div>
  );
};

export default DevOpsPane;
