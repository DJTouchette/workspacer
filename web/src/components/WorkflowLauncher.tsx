import React, { useState, useEffect, useCallback, useRef } from 'react';
import type { Workflow, WorkflowStep, WorkflowRun, WorkflowVariable, Skill } from '../types';
import { colors } from '../utils';
import { api } from '../hooks/useApi';
import FolderPicker from './FolderPicker';

interface Props {
  onClose: () => void;
}

const autonomyBadgeColors: Record<string, string> = {
  manual: '#94a3b8',
  semi: '#fbbf24',
  full: '#4ade80',
};

const stepStatusColors: Record<string, string> = {
  completed: '#4ade80',
  running: '#60a5fa',
  failed: '#f87171',
  skipped: '#94a3b8',
  pending: '#475569',
};

// ── Field style shared across forms ──
const fieldStyle: React.CSSProperties = {
  width: '100%', padding: '6px 10px', borderRadius: 5,
  border: `1px solid ${colors.borderSubtle}`,
  backgroundColor: 'transparent', color: colors.text, fontSize: '0.68rem',
  outline: 'none', fontFamily: 'inherit', boxSizing: 'border-box' as const,
};

// ── Main component ──

const WorkflowLauncher: React.FC<Props> = ({ onClose }) => {
  const [workflows, setWorkflows] = useState<Workflow[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [runs, setRuns] = useState<WorkflowRun[]>([]);
  const [showCreateForm, setShowCreateForm] = useState(false);
  const [expandedRunId, setExpandedRunId] = useState<string | null>(null);

  // Launch state
  const [projectPath, setProjectPath] = useState('');
  const [variables, setVariables] = useState<Record<string, string>>({});
  const [launching, setLaunching] = useState(false);
  const [error, setError] = useState('');

  // Skills for selected project
  const [skills, setSkills] = useState<Skill[]>([]);
  const lastFocusedTextarea = useRef<HTMLTextAreaElement | null>(null);

  const fetchWorkflows = useCallback(() => {
    api.getWorkflows().then(setWorkflows).catch(() => setWorkflows([]));
  }, []);

  const fetchRuns = useCallback(() => {
    api.getWorkflowRuns().then(setRuns).catch(() => setRuns([]));
  }, []);

  useEffect(() => {
    fetchWorkflows();
    fetchRuns();
    const iv = setInterval(fetchRuns, 3000);
    return () => clearInterval(iv);
  }, [fetchWorkflows, fetchRuns]);

  // Fetch skills when project changes
  useEffect(() => {
    if (projectPath) {
      api.getSkills(projectPath).then(setSkills).catch(() => setSkills([]));
    } else {
      setSkills([]);
    }
  }, [projectPath]);

  const selected = workflows.find(w => w.id === selectedId) ?? null;

  // Initialize variables when workflow is selected
  useEffect(() => {
    if (selected) {
      const vars: Record<string, string> = {};
      for (const step of selected.steps) {
        for (const v of (step.variables ?? [])) {
          if (v.source === 'user' && !(v.name in vars)) {
            vars[v.name] = v.default ?? '';
          }
        }
      }
      setVariables(vars);
    }
  }, [selectedId]); // eslint-disable-line react-hooks/exhaustive-deps

  const handleLaunch = async () => {
    if (!selected || !projectPath) return;
    setLaunching(true);
    setError('');
    try {
      await api.startWorkflowRun(selected.id, projectPath, variables);
      fetchRuns();
      setSelectedId(null);
      setProjectPath('');
      setVariables({});
    } catch (err: any) {
      setError(err?.message || 'Failed to start workflow');
    } finally {
      setLaunching(false);
    }
  };

  const handleCancelRun = async (id: string) => {
    try {
      await api.cancelWorkflowRun(id);
      fetchRuns();
    } catch {}
  };

  const handleInsertSkill = (skillName: string) => {
    if (lastFocusedTextarea.current) {
      const ta = lastFocusedTextarea.current;
      const start = ta.selectionStart;
      const end = ta.selectionEnd;
      const text = ta.value;
      const insert = `/${skillName}`;
      ta.value = text.slice(0, start) + insert + text.slice(end);
      ta.selectionStart = ta.selectionEnd = start + insert.length;
      ta.focus();
      ta.dispatchEvent(new Event('input', { bubbles: true }));
    }
  };

  // Collect user-facing variables from a workflow
  const getUserVariables = (wf: Workflow): WorkflowVariable[] => {
    const seen = new Set<string>();
    const result: WorkflowVariable[] = [];
    for (const step of wf.steps) {
      for (const v of (step.variables ?? [])) {
        if (v.source === 'user' && !seen.has(v.name)) {
          seen.add(v.name);
          result.push(v);
        }
      }
    }
    return result;
  };

  const activeRuns = runs.filter(r => r.status === 'running');
  const recentRuns = runs.slice(0, 10);

  return (
    <div onClick={onClose} style={{
      position: 'fixed', inset: 0, zIndex: 200, display: 'flex', justifyContent: 'center', alignItems: 'stretch',
      backgroundColor: 'rgba(0,0,0,0.6)', padding: '2vh 4vw',
    }}>
      <div onClick={e => e.stopPropagation()} style={{
        flex: 1, maxWidth: 1100, backgroundColor: colors.bgSurface,
        border: `1px solid ${colors.border}`, borderRadius: 12,
        display: 'flex', flexDirection: 'column', overflow: 'hidden',
      }}>
        {/* Header */}
        <div style={{
          padding: '14px 20px', borderBottom: `1px solid ${colors.borderSubtle}`,
          display: 'flex', alignItems: 'center', justifyContent: 'space-between', flexShrink: 0,
        }}>
          <div>
            <div style={{ fontSize: '0.92rem', fontWeight: 700, color: colors.textBright }}>Workflows</div>
            <div style={{ fontSize: '0.55rem', color: colors.textMuted, marginTop: 2 }}>
              Multi-step agent pipelines
            </div>
          </div>
          <span onClick={onClose} style={{ cursor: 'pointer', color: colors.textMuted, fontSize: '1rem' }}>{'\u00D7'}</span>
        </div>

        {/* Body: sidebar + content */}
        <div style={{ flex: 1, display: 'flex', overflow: 'hidden' }}>
          {/* Left sidebar */}
          <div style={{
            width: 240, borderRight: `1px solid ${colors.borderSubtle}`,
            display: 'flex', flexDirection: 'column', flexShrink: 0,
          }}>
            <div style={{ flex: 1, overflow: 'auto', padding: '10px 0' }}>
              {workflows.map(wf => (
                <div
                  key={wf.id}
                  onClick={() => { setSelectedId(wf.id); setShowCreateForm(false); setError(''); }}
                  style={{
                    padding: '8px 14px', cursor: 'pointer',
                    backgroundColor: selectedId === wf.id ? `${colors.accent}15` : 'transparent',
                    borderLeft: selectedId === wf.id ? `3px solid ${colors.accent}` : '3px solid transparent',
                  }}
                  onMouseEnter={e => { if (selectedId !== wf.id) e.currentTarget.style.backgroundColor = colors.bgHover; }}
                  onMouseLeave={e => { if (selectedId !== wf.id) e.currentTarget.style.backgroundColor = 'transparent'; }}
                >
                  <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                    <span style={{
                      fontSize: '0.72rem', fontWeight: 600, color: colors.textBright,
                      overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', flex: 1,
                    }}>
                      {wf.name}
                    </span>
                    {wf.isBuiltIn && (
                      <span style={{
                        fontSize: '0.42rem', fontWeight: 600, padding: '1px 5px', borderRadius: 4,
                        backgroundColor: `${colors.purple}15`, color: colors.purple,
                      }}>
                        built-in
                      </span>
                    )}
                  </div>
                  <div style={{
                    fontSize: '0.55rem', color: colors.textMuted, marginTop: 2,
                    overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                  }}>
                    {wf.steps.length} step{wf.steps.length !== 1 ? 's' : ''} &middot; {wf.description}
                  </div>
                </div>
              ))}
              {workflows.length === 0 && (
                <div style={{ padding: '20px 14px', fontSize: '0.68rem', color: colors.textMuted, textAlign: 'center' }}>
                  No workflows yet
                </div>
              )}
            </div>
            <div style={{ padding: '10px 14px', borderTop: `1px solid ${colors.borderSubtle}` }}>
              <button
                onClick={() => { setShowCreateForm(true); setSelectedId(null); setError(''); }}
                style={{
                  width: '100%', padding: '7px 0', borderRadius: 6,
                  border: `1px solid ${colors.accent}`,
                  backgroundColor: showCreateForm ? `${colors.accent}15` : 'transparent',
                  color: colors.accent, fontSize: '0.65rem', fontWeight: 600, cursor: 'pointer',
                }}
              >
                + Create Custom Workflow
              </button>
            </div>
          </div>

          {/* Right content */}
          <div style={{ flex: 1, overflow: 'auto', display: 'flex', flexDirection: 'column' }}>
            <div style={{ flex: 1, overflow: 'auto', padding: '16px 24px' }}>
              {/* Nothing selected */}
              {!selected && !showCreateForm && (
                <div style={{ textAlign: 'center', padding: '60px 0' }}>
                  <div style={{ fontSize: '0.9rem', color: colors.textBright, fontWeight: 600, marginBottom: 6 }}>
                    Select a workflow
                  </div>
                  <div style={{ fontSize: '0.72rem', color: colors.textMuted }}>
                    Choose a workflow from the sidebar, or create a custom one.
                  </div>
                </div>
              )}

              {/* Workflow detail / launch view */}
              {selected && !showCreateForm && (
                <WorkflowDetail
                  workflow={selected}
                  projectPath={projectPath}
                  onProjectChange={setProjectPath}
                  variables={variables}
                  onVariableChange={(name, val) => setVariables(prev => ({ ...prev, [name]: val }))}
                  userVariables={getUserVariables(selected)}
                  skills={skills}
                  onInsertSkill={handleInsertSkill}
                  lastFocusedTextarea={lastFocusedTextarea}
                  launching={launching}
                  error={error}
                  onLaunch={handleLaunch}
                  onDelete={!selected.isBuiltIn ? async () => {
                    try {
                      await api.deleteWorkflow(selected.id);
                      setSelectedId(null);
                      fetchWorkflows();
                    } catch {}
                  } : undefined}
                />
              )}

              {/* Create form */}
              {showCreateForm && (
                <CreateWorkflowForm
                  onCreated={() => { setShowCreateForm(false); fetchWorkflows(); }}
                  onCancel={() => setShowCreateForm(false)}
                />
              )}
            </div>

            {/* Active runs section */}
            {recentRuns.length > 0 && (
              <div style={{
                borderTop: `1px solid ${colors.borderSubtle}`, padding: '12px 24px',
                flexShrink: 0, maxHeight: 240, overflow: 'auto',
              }}>
                <div style={{
                  fontSize: '0.62rem', fontWeight: 600, color: colors.textMuted, marginBottom: 8,
                  textTransform: 'uppercase', letterSpacing: '0.05em',
                }}>
                  Workflow Runs {activeRuns.length > 0 && (
                    <span style={{ color: colors.accent, marginLeft: 4 }}>({activeRuns.length} active)</span>
                  )}
                </div>
                <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                  {recentRuns.map(run => (
                    <RunCard
                      key={run.id}
                      run={run}
                      expanded={expandedRunId === run.id}
                      onToggle={() => setExpandedRunId(expandedRunId === run.id ? null : run.id)}
                      onCancel={() => handleCancelRun(run.id)}
                    />
                  ))}
                </div>
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
};

// ── Workflow detail / launch view ──

interface WorkflowDetailProps {
  workflow: Workflow;
  projectPath: string;
  onProjectChange: (p: string) => void;
  variables: Record<string, string>;
  onVariableChange: (name: string, val: string) => void;
  userVariables: WorkflowVariable[];
  skills: Skill[];
  onInsertSkill: (name: string) => void;
  lastFocusedTextarea: React.MutableRefObject<HTMLTextAreaElement | null>;
  launching: boolean;
  error: string;
  onLaunch: () => void;
  onDelete?: () => void;
}

const WorkflowDetail: React.FC<WorkflowDetailProps> = ({
  workflow, projectPath, onProjectChange, variables, onVariableChange,
  userVariables, skills, onInsertSkill, lastFocusedTextarea,
  launching, error, onLaunch, onDelete,
}) => {
  return (
    <div>
      {/* Header */}
      <div style={{ marginBottom: 20 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 4 }}>
          <span style={{ fontSize: '1.05rem', fontWeight: 700, color: colors.textBright }}>
            {workflow.name}
          </span>
          {workflow.isBuiltIn && (
            <span style={{
              fontSize: '0.48rem', fontWeight: 600, padding: '2px 6px', borderRadius: 4,
              backgroundColor: `${colors.purple}15`, color: colors.purple,
            }}>
              built-in
            </span>
          )}
          {onDelete && (
            <span
              onClick={onDelete}
              style={{
                fontSize: '0.55rem', color: colors.error, cursor: 'pointer', marginLeft: 'auto',
                padding: '3px 8px', borderRadius: 4, border: `1px solid ${colors.error}40`,
              }}
            >
              Delete
            </span>
          )}
        </div>
        <div style={{ fontSize: '0.72rem', color: colors.textMuted }}>{workflow.description}</div>
      </div>

      {/* Step pipeline */}
      <div style={{ marginBottom: 24 }}>
        <div style={{
          fontSize: '0.62rem', fontWeight: 600, color: colors.textMuted, marginBottom: 10,
          textTransform: 'uppercase', letterSpacing: '0.05em',
        }}>
          Pipeline ({workflow.steps.length} steps)
        </div>
        <div style={{ display: 'flex', alignItems: 'flex-start', gap: 0, overflowX: 'auto', paddingBottom: 8 }}>
          {workflow.steps.map((step, i) => (
            <React.Fragment key={i}>
              <div style={{
                minWidth: 160, maxWidth: 220, flex: '0 0 auto',
                padding: '10px 14px', borderRadius: 8,
                border: `1px solid ${colors.borderSubtle}`, backgroundColor: colors.bg,
              }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 6 }}>
                  <span style={{
                    width: 18, height: 18, borderRadius: '50%', display: 'flex',
                    alignItems: 'center', justifyContent: 'center', fontSize: '0.52rem',
                    fontWeight: 700, backgroundColor: `${colors.accent}20`, color: colors.accent,
                    flexShrink: 0,
                  }}>
                    {i + 1}
                  </span>
                  <span style={{
                    fontSize: '0.72rem', fontWeight: 600, color: colors.textBright,
                    overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                  }}>
                    {step.name}
                  </span>
                </div>
                <div style={{
                  fontSize: '0.58rem', color: colors.textMuted, marginBottom: 6,
                  overflow: 'hidden', display: '-webkit-box',
                  WebkitLineClamp: 2, WebkitBoxOrient: 'vertical',
                  lineHeight: 1.4,
                }}>
                  {step.prompt}
                </div>
                <span style={{
                  fontSize: '0.48rem', fontWeight: 600, padding: '1px 6px', borderRadius: 4,
                  backgroundColor: `${autonomyBadgeColors[step.autonomy] || colors.textMuted}15`,
                  color: autonomyBadgeColors[step.autonomy] || colors.textMuted,
                }}>
                  {step.autonomy}
                </span>
              </div>
              {i < workflow.steps.length - 1 && (
                <div style={{
                  display: 'flex', alignItems: 'center', padding: '0 4px',
                  alignSelf: 'center', color: colors.textMuted, fontSize: '0.8rem',
                  flexShrink: 0,
                }}>
                  {'\u2192'}
                </div>
              )}
            </React.Fragment>
          ))}
        </div>
      </div>

      {/* Launch section */}
      <div style={{
        padding: '16px 18px', borderRadius: 10,
        border: `1px solid ${colors.borderSubtle}`, backgroundColor: colors.bg,
      }}>
        <div style={{
          fontSize: '0.72rem', fontWeight: 600, color: colors.textBright, marginBottom: 12,
        }}>
          Launch
        </div>

        {/* Project picker */}
        <div style={{ marginBottom: 12 }}>
          <div style={{ fontSize: '0.58rem', color: colors.textMuted, marginBottom: 4, fontWeight: 600 }}>
            Project
          </div>
          <FolderPicker value={projectPath} onChange={onProjectChange} />
        </div>

        {/* Variable form */}
        {userVariables.length > 0 && (
          <div style={{ marginBottom: 12 }}>
            <div style={{ fontSize: '0.58rem', color: colors.textMuted, marginBottom: 8, fontWeight: 600 }}>
              Variables
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
              {userVariables.map(v => (
                <div key={v.name}>
                  <label style={{ fontSize: '0.58rem', color: colors.text, fontWeight: 500, display: 'block', marginBottom: 3 }}>
                    {v.label}
                  </label>
                  {v.type === 'select' ? (
                    <select
                      value={variables[v.name] ?? v.default ?? ''}
                      onChange={e => onVariableChange(v.name, e.target.value)}
                      style={{ ...fieldStyle, width: 'auto', minWidth: 160 }}
                    >
                      {(v.options ?? []).map(opt => (
                        <option key={opt} value={opt}>{opt}</option>
                      ))}
                    </select>
                  ) : v.type === 'textarea' ? (
                    <textarea
                      value={variables[v.name] ?? v.default ?? ''}
                      onChange={e => onVariableChange(v.name, e.target.value)}
                      onFocus={e => { lastFocusedTextarea.current = e.target; }}
                      rows={3}
                      style={{ ...fieldStyle, resize: 'vertical', fontFamily: 'monospace', fontSize: '0.62rem' }}
                    />
                  ) : (
                    <input
                      value={variables[v.name] ?? v.default ?? ''}
                      onChange={e => onVariableChange(v.name, e.target.value)}
                      style={fieldStyle}
                    />
                  )}
                </div>
              ))}
            </div>
          </div>
        )}

        {/* Skills chips */}
        {projectPath && skills.length > 0 && (
          <div style={{ marginBottom: 12 }}>
            <div style={{ fontSize: '0.58rem', color: colors.textMuted, marginBottom: 6, fontWeight: 600 }}>
              Detected Skills (click to insert)
            </div>
            <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
              {skills.map(skill => (
                <span
                  key={skill.name}
                  onClick={() => onInsertSkill(skill.name)}
                  style={{
                    fontSize: '0.58rem', fontWeight: 600, color: colors.purple,
                    fontFamily: 'monospace', padding: '3px 8px', borderRadius: 5,
                    border: `1px solid ${colors.purple}40`, cursor: 'pointer',
                    backgroundColor: 'transparent',
                  }}
                  onMouseEnter={e => { e.currentTarget.style.backgroundColor = `${colors.purple}15`; }}
                  onMouseLeave={e => { e.currentTarget.style.backgroundColor = 'transparent'; }}
                  title={skill.description}
                >
                  /{skill.name}
                </span>
              ))}
            </div>
          </div>
        )}

        {error && (
          <div style={{ marginBottom: 10, fontSize: '0.62rem', color: colors.error }}>{error}</div>
        )}

        <button
          onClick={onLaunch}
          disabled={launching || !projectPath}
          style={{
            padding: '8px 20px', borderRadius: 6,
            border: `1px solid ${colors.accent}`,
            backgroundColor: projectPath && !launching ? colors.accent : 'transparent',
            color: projectPath && !launching ? '#fff' : colors.textMuted,
            fontSize: '0.72rem', fontWeight: 600,
            cursor: launching ? 'wait' : projectPath ? 'pointer' : 'not-allowed',
          }}
        >
          {launching ? 'Starting...' : 'Run Workflow'}
        </button>
      </div>
    </div>
  );
};

// ── Run card ──

interface RunCardProps {
  run: WorkflowRun;
  expanded: boolean;
  onToggle: () => void;
  onCancel: () => void;
}

const RunCard: React.FC<RunCardProps> = ({ run, expanded, onToggle, onCancel }) => {
  const projectLabel = run.projectPath.split(/[/\\]/).pop() || run.projectPath;
  const isRunning = run.status === 'running';
  const total = run.steps.length;
  const completed = run.steps.filter(s => s.status === 'completed').length;
  const failed = run.steps.filter(s => s.status === 'failed').length;

  return (
    <div style={{
      borderRadius: 8, border: `1px solid ${isRunning ? colors.accent : colors.borderSubtle}`,
      backgroundColor: colors.bg, overflow: 'hidden',
    }}>
      <div
        onClick={onToggle}
        style={{
          padding: '8px 12px', cursor: 'pointer',
          display: 'flex', alignItems: 'center', gap: 10,
        }}
      >
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
            <span style={{ fontSize: '0.68rem', fontWeight: 600, color: colors.textBright }}>
              {run.workflowName}
            </span>
            <span style={{ fontSize: '0.52rem', color: colors.textMuted }}>
              {projectLabel}
            </span>
          </div>
          {/* Progress bar */}
          <div style={{ display: 'flex', gap: 2, marginTop: 4, height: 4, borderRadius: 2, overflow: 'hidden' }}>
            {run.steps.map((step, i) => (
              <div
                key={i}
                style={{
                  flex: 1,
                  backgroundColor: stepStatusColors[step.status] || stepStatusColors.pending,
                  borderRadius: 1,
                  transition: 'background-color 0.3s',
                }}
              />
            ))}
          </div>
        </div>
        <span style={{
          fontSize: '0.52rem', fontWeight: 600, padding: '2px 8px', borderRadius: 4,
          color: isRunning ? colors.accent : run.status === 'completed' ? colors.success : run.status === 'failed' ? colors.error : colors.textMuted,
          backgroundColor: isRunning ? `${colors.accent}15` : run.status === 'completed' ? `${colors.success}15` : run.status === 'failed' ? `${colors.error}15` : `${colors.textMuted}15`,
        }}>
          {run.status} ({completed}/{total}{failed > 0 ? `, ${failed} failed` : ''})
        </span>
        {isRunning && (
          <span
            onClick={e => { e.stopPropagation(); onCancel(); }}
            style={{
              fontSize: '0.52rem', color: colors.error, cursor: 'pointer',
              padding: '2px 8px', borderRadius: 4, border: `1px solid ${colors.error}40`,
            }}
          >
            Cancel
          </span>
        )}
        <span style={{
          fontSize: '0.55rem', color: colors.textMuted,
          transform: expanded ? 'rotate(90deg)' : 'rotate(0deg)',
          transition: 'transform 0.15s',
        }}>{'\u25B6'}</span>
      </div>

      {expanded && (
        <div style={{
          padding: '8px 12px', borderTop: `1px solid ${colors.borderSubtle}`,
        }}>
          {run.steps.map((step, i) => (
            <div key={i} style={{
              display: 'flex', alignItems: 'flex-start', gap: 8, padding: '4px 0',
            }}>
              <span style={{
                width: 14, height: 14, borderRadius: '50%', display: 'flex',
                alignItems: 'center', justifyContent: 'center', fontSize: '0.42rem',
                fontWeight: 700, flexShrink: 0, marginTop: 1,
                backgroundColor: `${stepStatusColors[step.status] || stepStatusColors.pending}20`,
                color: stepStatusColors[step.status] || stepStatusColors.pending,
              }}>
                {step.status === 'completed' ? '\u2713' :
                 step.status === 'failed' ? '\u2717' :
                 step.status === 'running' ? '\u25CB' :
                 (i + 1)}
              </span>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  <span style={{ fontSize: '0.65rem', fontWeight: 600, color: colors.textBright }}>
                    {step.name}
                  </span>
                  <span style={{
                    fontSize: '0.48rem', fontWeight: 600,
                    color: stepStatusColors[step.status] || colors.textMuted,
                  }}>
                    {step.status}
                  </span>
                </div>
                {step.summary && (
                  <div style={{ fontSize: '0.55rem', color: colors.textMuted, marginTop: 2 }}>
                    {step.summary}
                  </div>
                )}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
};

// ── Create workflow form ──

interface CreateWorkflowFormProps {
  onCreated: () => void;
  onCancel: () => void;
}

const CreateWorkflowForm: React.FC<CreateWorkflowFormProps> = ({ onCreated, onCancel }) => {
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [steps, setSteps] = useState<WorkflowStep[]>([
    { name: '', prompt: '', autonomy: 'semi' },
  ]);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');

  const updateStep = (index: number, patch: Partial<WorkflowStep>) => {
    setSteps(prev => prev.map((s, i) => i === index ? { ...s, ...patch } : s));
  };

  const addStep = () => {
    setSteps(prev => [...prev, { name: '', prompt: '', autonomy: 'semi' }]);
  };

  const removeStep = (index: number) => {
    if (steps.length <= 1) return;
    setSteps(prev => prev.filter((_, i) => i !== index));
  };

  const handleSave = async () => {
    if (!name.trim() || steps.some(s => !s.name.trim() || !s.prompt.trim())) return;
    setSaving(true);
    setError('');
    try {
      await api.createWorkflow({ name: name.trim(), description: description.trim(), steps });
      onCreated();
    } catch (err: any) {
      setError(err?.message || 'Failed to create workflow');
    } finally {
      setSaving(false);
    }
  };

  const canSave = name.trim() && steps.every(s => s.name.trim() && s.prompt.trim());

  return (
    <div>
      <div style={{ fontSize: '1.05rem', fontWeight: 700, color: colors.textBright, marginBottom: 16 }}>
        Create Custom Workflow
      </div>

      {/* Name + description */}
      <div style={{ marginBottom: 16 }}>
        <div style={{ display: 'flex', gap: 8, marginBottom: 8 }}>
          <div style={{ flex: 1 }}>
            <label style={{ fontSize: '0.58rem', color: colors.textMuted, fontWeight: 600, display: 'block', marginBottom: 3 }}>
              Name
            </label>
            <input
              value={name}
              onChange={e => setName(e.target.value)}
              placeholder="Workflow name..."
              style={fieldStyle}
            />
          </div>
          <div style={{ flex: 2 }}>
            <label style={{ fontSize: '0.58rem', color: colors.textMuted, fontWeight: 600, display: 'block', marginBottom: 3 }}>
              Description
            </label>
            <input
              value={description}
              onChange={e => setDescription(e.target.value)}
              placeholder="What does this workflow do?"
              style={fieldStyle}
            />
          </div>
        </div>
      </div>

      {/* Steps */}
      <div style={{ marginBottom: 16 }}>
        <div style={{
          fontSize: '0.62rem', fontWeight: 600, color: colors.textMuted, marginBottom: 10,
          textTransform: 'uppercase', letterSpacing: '0.05em',
        }}>
          Steps
        </div>

        <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
          {steps.map((step, i) => (
            <div key={i} style={{
              padding: '12px 14px', borderRadius: 8,
              border: `1px solid ${colors.borderSubtle}`, backgroundColor: colors.bg,
            }}>
              <div style={{ display: 'flex', gap: 8, alignItems: 'center', marginBottom: 8 }}>
                <span style={{
                  width: 20, height: 20, borderRadius: '50%', display: 'flex',
                  alignItems: 'center', justifyContent: 'center', fontSize: '0.55rem',
                  fontWeight: 700, backgroundColor: `${colors.accent}20`, color: colors.accent,
                  flexShrink: 0,
                }}>
                  {i + 1}
                </span>
                <input
                  value={step.name}
                  onChange={e => updateStep(i, { name: e.target.value })}
                  placeholder="Step name..."
                  style={{ ...fieldStyle, flex: 1 }}
                />
                <select
                  value={step.autonomy}
                  onChange={e => updateStep(i, { autonomy: e.target.value })}
                  style={{ ...fieldStyle, width: 'auto', minWidth: 100 }}
                >
                  <option value="manual">Manual</option>
                  <option value="semi">Semi-auto</option>
                  <option value="full">Full auto</option>
                </select>
                {steps.length > 1 && (
                  <span
                    onClick={() => removeStep(i)}
                    style={{
                      fontSize: '0.72rem', color: colors.textMuted, cursor: 'pointer',
                      padding: '2px 6px', flexShrink: 0,
                    }}
                    title="Remove step"
                  >
                    {'\u00D7'}
                  </span>
                )}
              </div>
              <textarea
                value={step.prompt}
                onChange={e => updateStep(i, { prompt: e.target.value })}
                placeholder="Step prompt..."
                rows={3}
                style={{
                  ...fieldStyle, resize: 'vertical', fontFamily: 'monospace', fontSize: '0.62rem',
                }}
              />
            </div>
          ))}
        </div>

        <button
          onClick={addStep}
          style={{
            marginTop: 8, padding: '6px 14px', borderRadius: 6,
            border: `1px solid ${colors.borderSubtle}`,
            backgroundColor: 'transparent', color: colors.textMuted,
            fontSize: '0.62rem', fontWeight: 600, cursor: 'pointer',
          }}
          onMouseEnter={e => { e.currentTarget.style.borderColor = colors.accent; e.currentTarget.style.color = colors.accent; }}
          onMouseLeave={e => { e.currentTarget.style.borderColor = colors.borderSubtle; e.currentTarget.style.color = colors.textMuted; }}
        >
          + Add Step
        </button>
      </div>

      {error && (
        <div style={{ marginBottom: 10, fontSize: '0.62rem', color: colors.error }}>{error}</div>
      )}

      {/* Actions */}
      <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
        <button
          onClick={handleSave}
          disabled={saving || !canSave}
          style={{
            padding: '8px 20px', borderRadius: 6,
            border: `1px solid ${colors.accent}`,
            backgroundColor: canSave && !saving ? colors.accent : 'transparent',
            color: canSave && !saving ? '#fff' : colors.textMuted,
            fontSize: '0.72rem', fontWeight: 600,
            cursor: saving ? 'wait' : canSave ? 'pointer' : 'not-allowed',
          }}
        >
          {saving ? 'Creating...' : 'Create Workflow'}
        </button>
        <span
          onClick={onCancel}
          style={{ fontSize: '0.65rem', color: colors.textMuted, cursor: 'pointer' }}
        >
          Cancel
        </span>
      </div>
    </div>
  );
};

export default WorkflowLauncher;
