import React, { useState, useEffect, useCallback } from 'react';
import type { Skill } from '../types';
import { colors } from '../utils';
import { api } from '../hooks/useApi';

interface Props {
  projectPath: string;
  onInsertSkill?: (skillName: string) => void;
}

const SkillManager: React.FC<Props> = ({ projectPath, onInsertSkill }) => {
  const [skills, setSkills] = useState<Skill[]>([]);
  const [expandedSkill, setExpandedSkill] = useState<string | null>(null);
  const [editContent, setEditContent] = useState('');
  const [showNewForm, setShowNewForm] = useState(false);
  const [newName, setNewName] = useState('');
  const [newContent, setNewContent] = useState('');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);

  const fetchSkills = useCallback(() => {
    api.getSkills(projectPath).then(setSkills).catch(() => setSkills([]));
  }, [projectPath]);

  useEffect(() => {
    fetchSkills();
  }, [fetchSkills]);

  const handleExpand = async (name: string) => {
    if (expandedSkill === name) {
      setExpandedSkill(null);
      return;
    }
    try {
      const detail = await api.getSkillDetail(projectPath, name);
      setEditContent(detail.content || '');
      setExpandedSkill(name);
      setError('');
    } catch (err: any) {
      setError(err?.message || 'Failed to load skill');
    }
  };

  const handleUpdate = async (name: string) => {
    setSaving(true);
    setError('');
    try {
      await api.updateSkill(projectPath, name, editContent);
      setExpandedSkill(null);
      fetchSkills();
    } catch (err: any) {
      setError(err?.message || 'Failed to update skill');
    } finally {
      setSaving(false);
    }
  };

  const handleCreate = async () => {
    if (!newName.trim() || !newContent.trim()) return;
    setSaving(true);
    setError('');
    try {
      await api.createSkill(projectPath, newName.trim(), newContent);
      setNewName('');
      setNewContent('');
      setShowNewForm(false);
      fetchSkills();
    } catch (err: any) {
      setError(err?.message || 'Failed to create skill');
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (name: string) => {
    try {
      await api.deleteSkill(projectPath, name);
      setConfirmDelete(null);
      if (expandedSkill === name) setExpandedSkill(null);
      fetchSkills();
    } catch (err: any) {
      setError(err?.message || 'Failed to delete skill');
    }
  };

  return (
    <div style={{ marginBottom: 16 }}>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 8 }}>
        <span style={{ fontSize: '0.72rem', fontWeight: 600, color: colors.textBright }}>Skills</span>
        <span
          onClick={() => { setShowNewForm(!showNewForm); setError(''); }}
          style={{ fontSize: '0.58rem', color: colors.accent, cursor: 'pointer' }}
        >
          {showNewForm ? 'Cancel' : '+ New Skill'}
        </span>
      </div>

      {error && (
        <div style={{ marginBottom: 8, fontSize: '0.62rem', color: colors.error }}>{error}</div>
      )}

      {/* New skill form */}
      {showNewForm && (
        <div style={{
          padding: '10px 14px', borderRadius: 8, marginBottom: 10,
          border: `1px solid ${colors.borderSubtle}`, backgroundColor: colors.bgSurface,
        }}>
          <input
            value={newName}
            onChange={e => setNewName(e.target.value.replace(/[^a-zA-Z0-9-]/g, ''))}
            placeholder="skill-name (alphanumeric + hyphens)"
            style={{ ...fieldStyle, marginBottom: 6 }}
          />
          <textarea
            value={newContent}
            onChange={e => setNewContent(e.target.value)}
            placeholder="Skill content (markdown)..."
            rows={4}
            style={{ ...fieldStyle, marginBottom: 6, resize: 'vertical', fontFamily: 'monospace', fontSize: '0.62rem' }}
          />
          <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
            <button
              onClick={handleCreate}
              disabled={saving || !newName.trim() || !newContent.trim()}
              style={{
                padding: '5px 12px', borderRadius: 4, border: `1px solid ${colors.accent}`,
                backgroundColor: newName.trim() && newContent.trim() && !saving ? colors.accent : 'transparent',
                color: newName.trim() && newContent.trim() && !saving ? '#fff' : colors.textMuted,
                fontSize: '0.65rem', fontWeight: 600, cursor: saving ? 'wait' : 'pointer',
              }}
            >
              {saving ? 'Creating...' : 'Create'}
            </button>
          </div>
        </div>
      )}

      {/* Skills list */}
      {skills.length === 0 && !showNewForm && (
        <div style={{ fontSize: '0.58rem', color: colors.textMuted, padding: '4px 0' }}>
          No skills found. Skills are stored in .claude/commands/ as markdown files.
        </div>
      )}

      <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        {skills.map(skill => (
          <div
            key={skill.name}
            style={{
              borderRadius: 8,
              border: `1px solid ${expandedSkill === skill.name ? colors.accent : colors.borderSubtle}`,
              backgroundColor: colors.bgSurface,
              transition: 'border-color 0.15s',
            }}
          >
            {/* Compact card header */}
            <div
              onClick={() => handleExpand(skill.name)}
              style={{
                padding: '8px 12px', cursor: 'pointer',
                display: 'flex', alignItems: 'center', gap: 8,
              }}
              onMouseEnter={e => {
                if (expandedSkill !== skill.name) e.currentTarget.parentElement!.style.borderColor = colors.accent;
              }}
              onMouseLeave={e => {
                if (expandedSkill !== skill.name) e.currentTarget.parentElement!.style.borderColor = colors.borderSubtle;
              }}
            >
              <span style={{
                fontSize: '0.68rem', fontWeight: 600, color: colors.purple,
                fontFamily: 'monospace',
              }}>
                /{skill.name}
              </span>
              <span style={{
                fontSize: '0.58rem', color: colors.textMuted,
                overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', flex: 1,
              }}>
                {skill.description}
              </span>
              <div style={{ display: 'flex', gap: 4, flexShrink: 0 }}>
                {onInsertSkill && (
                  <span
                    onClick={e => { e.stopPropagation(); onInsertSkill(skill.name); }}
                    title="Insert into prompt"
                    style={{
                      fontSize: '0.55rem', color: colors.accent, cursor: 'pointer',
                      padding: '2px 6px', borderRadius: 4,
                      border: `1px solid ${colors.accent}`,
                    }}
                  >
                    Insert
                  </span>
                )}
                {confirmDelete === skill.name ? (
                  <>
                    <span
                      onClick={e => { e.stopPropagation(); handleDelete(skill.name); }}
                      style={{ fontSize: '0.55rem', color: colors.error, cursor: 'pointer', padding: '2px 6px' }}
                    >
                      Confirm
                    </span>
                    <span
                      onClick={e => { e.stopPropagation(); setConfirmDelete(null); }}
                      style={{ fontSize: '0.55rem', color: colors.textMuted, cursor: 'pointer', padding: '2px 6px' }}
                    >
                      Cancel
                    </span>
                  </>
                ) : (
                  <span
                    onClick={e => { e.stopPropagation(); setConfirmDelete(skill.name); }}
                    style={{ fontSize: '0.55rem', color: colors.textMuted, cursor: 'pointer', padding: '2px 6px' }}
                    title="Delete skill"
                  >
                    {'\u00D7'}
                  </span>
                )}
              </div>
            </div>

            {/* Expanded content editor */}
            {expandedSkill === skill.name && (
              <div style={{
                padding: '0 12px 10px 12px',
                borderTop: `1px solid ${colors.borderSubtle}`,
              }}>
                <textarea
                  value={editContent}
                  onChange={e => setEditContent(e.target.value)}
                  rows={8}
                  style={{
                    ...fieldStyle,
                    marginTop: 8,
                    resize: 'vertical',
                    fontFamily: 'monospace',
                    fontSize: '0.62rem',
                  }}
                />
                <div style={{ display: 'flex', gap: 6, marginTop: 6, alignItems: 'center' }}>
                  <button
                    onClick={() => handleUpdate(skill.name)}
                    disabled={saving}
                    style={{
                      padding: '5px 12px', borderRadius: 4, border: `1px solid ${colors.accent}`,
                      backgroundColor: !saving ? colors.accent : 'transparent',
                      color: !saving ? '#fff' : colors.textMuted,
                      fontSize: '0.65rem', fontWeight: 600, cursor: saving ? 'wait' : 'pointer',
                    }}
                  >
                    {saving ? 'Saving...' : 'Save'}
                  </button>
                  <span
                    onClick={() => setExpandedSkill(null)}
                    style={{ fontSize: '0.58rem', color: colors.textMuted, cursor: 'pointer' }}
                  >
                    Close
                  </span>
                  <span style={{
                    fontSize: '0.5rem', color: colors.textMuted, marginLeft: 'auto',
                    fontFamily: 'monospace',
                  }}>
                    {skill.filePath}
                  </span>
                </div>
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
};

const fieldStyle: React.CSSProperties = {
  width: '100%', padding: '5px 8px', borderRadius: 4, border: `1px solid ${colors.borderSubtle}`,
  backgroundColor: 'transparent', color: colors.text, fontSize: '0.68rem', outline: 'none',
  fontFamily: 'inherit', boxSizing: 'border-box' as const,
};

export default SkillManager;
