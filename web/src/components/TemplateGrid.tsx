import React, { useState, useEffect } from 'react';
import type { Template } from '../types';
import { colors } from '../utils';
import { api } from '../hooks/useApi';

interface Props {
  projectPath?: string;
  onLaunch: (template: Template, cwd: string) => void;
}

const autonomyBadgeColors: Record<string, string> = {
  manual: '#94a3b8',
  semi: '#fbbf24',
  full: '#4ade80',
};

const TemplateGrid: React.FC<Props> = ({ projectPath, onLaunch }) => {
  const [templates, setTemplates] = useState<Template[]>([]);
  const [showForm, setShowForm] = useState(false);
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [prompt, setPrompt] = useState('');
  const [autonomy, setAutonomy] = useState('semi');
  const [budgetDollars, setBudgetDollars] = useState(2);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState('');

  const fetchTemplates = () => {
    api.getTemplates(projectPath).then(setTemplates).catch(() => setTemplates([]));
  };

  useEffect(() => {
    fetchTemplates();
  }, [projectPath]);

  const handleCreate = async () => {
    if (!name.trim() || !prompt.trim()) return;
    setCreating(true);
    setError('');
    try {
      await api.createTemplate({ name, description, prompt, autonomy, budgetDollars, projectPath: projectPath || undefined });
      setName(''); setDescription(''); setPrompt(''); setShowForm(false);
      fetchTemplates();
    } catch (err: any) {
      setError(err?.message || 'Failed to create template');
    } finally {
      setCreating(false);
    }
  };

  const handleDelete = async (e: React.MouseEvent, id: string) => {
    e.stopPropagation();
    try {
      await api.deleteTemplate(id);
      fetchTemplates();
    } catch {}
  };

  if (templates.length === 0 && !showForm) return null;

  return (
    <div style={{ marginBottom: 16 }}>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 8 }}>
        <span style={{ fontSize: '0.72rem', fontWeight: 600, color: colors.textBright }}>Templates</span>
        <span
          onClick={() => setShowForm(!showForm)}
          style={{ fontSize: '0.58rem', color: colors.accent, cursor: 'pointer' }}
        >
          {showForm ? 'Cancel' : '+ Create Custom'}
        </span>
      </div>

      {/* Inline creation form */}
      {showForm && (
        <div style={{
          padding: '10px 14px', borderRadius: 8, marginBottom: 10,
          border: `1px solid ${colors.borderSubtle}`, backgroundColor: colors.bgSurface,
        }}>
          <div style={{ display: 'flex', gap: 6, marginBottom: 6 }}>
            <input value={name} onChange={e => setName(e.target.value)} placeholder="Template name..." style={fieldStyle} />
            <input value={description} onChange={e => setDescription(e.target.value)} placeholder="Short description..." style={fieldStyle} />
          </div>
          <textarea value={prompt} onChange={e => setPrompt(e.target.value)} placeholder="Prompt template..." rows={2} style={{ ...fieldStyle, marginBottom: 6, resize: 'vertical', fontFamily: 'monospace', fontSize: '0.62rem' }} />
          <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
            <select value={autonomy} onChange={e => setAutonomy(e.target.value)} style={{ ...fieldStyle, width: 'auto' }}>
              <option value="manual">Manual</option>
              <option value="semi">Semi-auto</option>
              <option value="full">Full auto</option>
            </select>
            <span style={{ fontSize: '0.55rem', color: colors.textMuted }}>$</span>
            <input type="number" step="0.5" value={budgetDollars} onChange={e => setBudgetDollars(parseFloat(e.target.value) || 0)} style={{ ...fieldStyle, width: 50, textAlign: 'center' }} />
            <button onClick={handleCreate} disabled={creating || !name.trim() || !prompt.trim()} style={{
              padding: '5px 12px', borderRadius: 4, border: `1px solid ${colors.accent}`,
              backgroundColor: name && prompt && !creating ? colors.accent : 'transparent',
              color: name && prompt && !creating ? '#fff' : colors.textMuted,
              fontSize: '0.65rem', fontWeight: 600, cursor: creating ? 'wait' : 'pointer', whiteSpace: 'nowrap',
            }}>{creating ? 'Creating...' : 'Create'}</button>
          </div>
          {error && <div style={{ marginTop: 6, fontSize: '0.62rem', color: colors.error }}>{error}</div>}
        </div>
      )}

      {/* Template cards — single row that wraps */}
      <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
        {templates.map(t => (
          <div
            key={t.id}
            onClick={() => onLaunch(t, t.projectPath || '')}
            style={{
              padding: '8px 12px', borderRadius: 8, cursor: 'pointer',
              border: `1px solid ${colors.borderSubtle}`, backgroundColor: colors.bgSurface,
              minWidth: 160, maxWidth: 220, flex: '0 0 auto',
              transition: 'border-color 0.15s',
            }}
            onMouseEnter={e => { e.currentTarget.style.borderColor = colors.accent; }}
            onMouseLeave={e => { e.currentTarget.style.borderColor = colors.borderSubtle; }}
          >
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 4 }}>
              <span style={{ fontSize: '0.72rem', fontWeight: 600, color: colors.textBright, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', flex: 1 }}>
                {t.name}
              </span>
              {!t.isBuiltIn && (
                <span
                  onClick={e => handleDelete(e, t.id)}
                  style={{ fontSize: '0.55rem', color: colors.textMuted, cursor: 'pointer', marginLeft: 4, flexShrink: 0 }}
                >{'\u00D7'}</span>
              )}
            </div>
            {t.description && (
              <div style={{ fontSize: '0.58rem', color: colors.textMuted, marginBottom: 6, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                {t.description}
              </div>
            )}
            <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
              <span style={{
                fontSize: '0.5rem', fontWeight: 600, padding: '1px 5px', borderRadius: 4,
                backgroundColor: `${autonomyBadgeColors[t.autonomy] || colors.textMuted}15`,
                color: autonomyBadgeColors[t.autonomy] || colors.textMuted,
              }}>
                {t.autonomy}
              </span>
              <span style={{ fontSize: '0.5rem', color: colors.textMuted }}>
                ${t.budgetDollars.toFixed(2)}
              </span>
              {t.isBuiltIn && (
                <span style={{ fontSize: '0.45rem', color: colors.textMuted, fontStyle: 'italic', marginLeft: 'auto' }}>built-in</span>
              )}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
};

const fieldStyle: React.CSSProperties = {
  width: '100%', padding: '5px 8px', borderRadius: 4, border: `1px solid ${colors.borderSubtle}`,
  backgroundColor: 'transparent', color: colors.text, fontSize: '0.68rem', outline: 'none', fontFamily: 'inherit', boxSizing: 'border-box',
};

export default TemplateGrid;
