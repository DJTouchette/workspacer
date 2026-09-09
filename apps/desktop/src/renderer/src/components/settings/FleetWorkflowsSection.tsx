import {
  openPolicySettings,
  consumeTaskWorkflowProject,
  TASK_WORKFLOW_SETTINGS_EVENT,
} from '../../lib/settingsBus';
import React, { useCallback, useEffect, useState } from 'react';
import { ArrowUp, ArrowDown } from 'lucide-react';
import { inputStyle, SmallButton } from './primitives';
import { Surface } from '../Surface';
import { TASK_STAGES } from '../../../../main/shared/dispatchHistory';
import {
  WORKFLOW_KINDS,
  WORKFLOW_ROLES,
  WORKFLOW_STARTERS,
  validateWorkflow,
  reviewPolicy,
  type WorkflowCatalog,
  type WorkflowDefinition,
  type WorkflowRequest,
} from '../../../../main/shared/fleetWorkflow';
const changed = 'wks-fleet-workflows-changed';
function useWorkflows() {
  const [catalog, setCatalog] = useState<WorkflowCatalog>();
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const reload = useCallback(async () => {
    try {
      const res = await window.electronAPI?.fleetWorkflowRequest?.({ op: 'list' });
      if (!res)
        throw new Error(
          'Fleet workflows require this host’s desktop app. Headless and older peers are unavailable.',
        );
      if (!res.ok) throw new Error(res.error);
      setCatalog(res.catalog);
      setError('');
    } catch (e) {
      setError(String(e));
    }
  }, []);
  useEffect(() => {
    void reload();
    const listener = () => void reload();
    window.addEventListener(changed, listener);
    return () => window.removeEventListener(changed, listener);
  }, [reload]);
  const request = async (r: WorkflowRequest) => {
    setBusy(true);
    try {
      const res = await window.electronAPI?.fleetWorkflowRequest?.(r);
      if (!res) throw new Error('Fleet workflows unavailable');
      if (!res.ok)
        throw new Error(
          res.code === 'conflict'
            ? `${res.error}. Your draft is preserved. Reload current values before retrying.`
            : res.error,
        );
      setError('');
      window.dispatchEvent(new Event(changed));
      return res;
    } catch (e) {
      setError(String(e));
      return undefined;
    } finally {
      setBusy(false);
    }
  };
  return { catalog, error, busy, request, reload };
}
export function ProjectWorkflowSelector({ cwd }: { cwd: string }) {
  const { catalog, error, busy, request } = useWorkflows();
  return (
    <div style={{ maxWidth: '100%', fontSize: '0.72rem' }}>
      <label>
        Workflow{' '}
        <select
          aria-label={`Fleet workflow for ${cwd}`}
          style={{ ...inputStyle, maxWidth: '100%', width: 220 }}
          disabled={!catalog || busy}
          value={catalog?.projects[cwd] ?? ''}
          onChange={(e) =>
            void request({
              op: 'select',
              cwd,
              workflowId: e.target.value || null,
              expectedRevision: catalog?.selectionRevision,
            })
          }
        >
          <option value="">Inherit global default</option>
          {catalog?.definitions
            .filter((d) => d.enabled)
            .map((d) => (
              <option key={d.id} value={d.id}>
                {d.name}
              </option>
            ))}
        </select>
      </label>
      {catalog && (
        <div>
          {catalog.definitions.some((d) => d.id === (catalog.projects[cwd] ?? catalog.defaultId))
            ? reviewPolicy(
                catalog.definitions.find(
                  (d) => d.id === (catalog.projects[cwd] ?? catalog.defaultId),
                )!,
              )
            : 'Selected workflow unavailable; select an enabled definition'}
        </div>
      )}
      {error && <div role="alert">{error}</div>}
    </div>
  );
}
export default function FleetWorkflowsSection() {
  const [taskProject, setTaskProject] = useState(consumeTaskWorkflowProject);
  useEffect(() => {
    const listener = () => setTaskProject(consumeTaskWorkflowProject());
    window.addEventListener(TASK_WORKFLOW_SETTINGS_EVENT, listener);
    return () => window.removeEventListener(TASK_WORKFLOW_SETTINGS_EVENT, listener);
  }, []);
  const { catalog, error, busy, request, reload } = useWorkflows();
  const [draft, setDraft] = useState<WorkflowDefinition>();
  const [editing, setEditing] = useState(false);
  const immutable = !!draft && WORKFLOW_STARTERS.some((d) => d.id === draft.id) && editing;
  let validation = '';
  if (draft)
    try {
      validateWorkflow(draft);
    } catch (e) {
      validation = String(e);
    }
  const change = (index: number, patch: Partial<WorkflowDefinition['steps'][number]>) =>
    setDraft((d) =>
      d ? { ...d, steps: d.steps.map((s, i) => (i === index ? { ...s, ...patch } : s)) } : d,
    );
  const reorder = (index: number, delta: number) =>
    setDraft((d) => {
      if (!d) return d;
      const steps = [...d.steps];
      [steps[index], steps[index + delta]] = [steps[index + delta], steps[index]];
      return { ...d, steps };
    });
  const field: React.CSSProperties = {
    ...inputStyle,
    width: '100%',
    minWidth: 0,
    boxSizing: 'border-box',
  };
  return (
    <section
      aria-label="Fleet workflows"
      style={{ marginTop: 20, fontSize: '0.8rem', minWidth: 0 }}
    >
      {taskProject && (
        <div>
          <h3>New tasks in {taskProject}</h3>
          <p>
            This selection applies to new tasks only. Existing tasks keep their recorded workflow.
          </p>
          <ProjectWorkflowSelector cwd={taskProject} />
        </div>
      )}
      <h3 style={{ fontSize: '0.9rem' }}>Workflows</h3>
      <p>
        Choose the ordered policy for new Fleet tasks. Active tasks retain their definition,
        templates and result contracts. Editing never launches an agent.
      </p>
      <SmallButton
        label="Model routing"
        disabled={busy || editing}
        onClick={() => openPolicySettings('routing')}
      />
      {error && (
        <div role="alert" style={{ color: 'var(--wks-error)', overflowWrap: 'anywhere' }}>
          {error} <SmallButton label="Reload current values" onClick={() => void reload()} />
        </div>
      )}
      {catalog && (
        <>
          <label>
            Global default{' '}
            <select
              aria-label="Default Fleet workflow"
              style={field}
              disabled={busy}
              value={catalog.defaultId}
              onChange={(e) =>
                void request({
                  op: 'select',
                  workflowId: e.target.value,
                  expectedRevision: catalog.selectionRevision,
                })
              }
            >
              {catalog.definitions
                .filter((d) => d.enabled)
                .map((d) => (
                  <option key={d.id} value={d.id}>
                    {d.name}
                  </option>
                ))}
            </select>
          </label>
          <div style={{ display: 'grid', gap: 8, marginTop: 12 }}>
            {catalog.definitions.map((d) => (
              <Surface key={d.id} elevation="flat" pad="md">
                <strong>{d.name}</strong> · revision {d.revision}
                {!d.enabled && ' · disabled'}
                <p
                  style={{
                    color: d.steps.some((s) => s.kind === 'review')
                      ? 'var(--wks-text-muted)'
                      : 'var(--wks-warning)',
                  }}
                >
                  {reviewPolicy(d)}
                </p>
                <p>
                  {d.steps
                    .map((s) => s.label + (s.when === 'material_risk' ? ' (conditional)' : ''))
                    .join(' → ')}
                </p>
                <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
                  <SmallButton
                    label="Edit / inspect"
                    onClick={() => {
                      setDraft(structuredClone(d));
                      setEditing(true);
                    }}
                  />
                  <SmallButton
                    label="Clone"
                    onClick={() =>
                      void request({
                        op: 'clone',
                        id: d.id,
                        expectedRevision: d.revision,
                        name: `${d.name} copy`,
                      })
                    }
                  />
                  {!WORKFLOW_STARTERS.some((s) => s.id === d.id) && (
                    <>
                      <SmallButton
                        label="Disable"
                        disabled={
                          busy ||
                          !d.enabled ||
                          d.id === catalog.defaultId ||
                          Object.values(catalog.projects).includes(d.id)
                        }
                        onClick={() =>
                          void request({ op: 'disable', id: d.id, expectedRevision: d.revision })
                        }
                      />
                      <SmallButton
                        label="Delete"
                        disabled={
                          busy ||
                          d.id === catalog.defaultId ||
                          Object.values(catalog.projects).includes(d.id)
                        }
                        onClick={() =>
                          void request({ op: 'delete', id: d.id, expectedRevision: d.revision })
                        }
                      />
                    </>
                  )}
                </div>
              </Surface>
            ))}
          </div>
          <div style={{ marginTop: 12 }}>
            <SmallButton
              label="Create workflow"
              onClick={() => {
                setEditing(false);
                setDraft({
                  ...structuredClone(WORKFLOW_STARTERS[1]),
                  id: 'custom-workflow',
                  name: 'Custom workflow',
                });
              }}
            />
          </div>
          {draft && (
            <div
              role="group"
              aria-label="Workflow definition editor"
              style={{ display: 'grid', gap: 10, marginTop: 16 }}
            >
              <h4>Definition editor</h4>
              {immutable && (
                <p>Shipped starters are immutable. Clone this definition to customize it.</p>
              )}
              <fieldset
                disabled={immutable || busy}
                style={{ border: 0, padding: 0, minWidth: 0, display: 'grid', gap: 10 }}
              >
                <label>
                  Workflow id
                  <input
                    aria-label="Workflow id"
                    disabled={editing}
                    style={field}
                    value={draft.id}
                    onChange={(e) => setDraft({ ...draft, id: e.target.value })}
                  />
                </label>
                <label>
                  Name
                  <input
                    aria-label="Workflow name"
                    style={field}
                    value={draft.name}
                    onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                  />
                </label>
                <label>
                  Description
                  <textarea
                    style={field}
                    value={draft.description}
                    onChange={(e) => setDraft({ ...draft, description: e.target.value })}
                  />
                </label>
                <label>
                  <input
                    type="checkbox"
                    checked={draft.enabled}
                    onChange={(e) => setDraft({ ...draft, enabled: e.target.checked })}
                  />{' '}
                  Enabled for new tasks
                </label>
                {draft.steps.map((s, i) => (
                  <fieldset
                    key={i}
                    style={{
                      border: '1px solid var(--wks-border-subtle)',
                      borderRadius: 'var(--wks-radius-sm)',
                      padding: 10,
                      minWidth: 0,
                      display: 'grid',
                      gap: 8,
                    }}
                  >
                    <legend>
                      Step {i + 1}: {s.label}
                    </legend>
                    <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
                      <SmallButton
                        label={
                          <>
                            <ArrowUp size={12} /> Move up
                          </>
                        }
                        disabled={i === 0}
                        onClick={() => reorder(i, -1)}
                      />
                      <SmallButton
                        label={
                          <>
                            <ArrowDown size={12} /> Move down
                          </>
                        }
                        disabled={i === draft.steps.length - 1}
                        onClick={() => reorder(i, 1)}
                      />
                      <SmallButton
                        label="Remove step"
                        disabled={draft.steps.length === 1}
                        onClick={() =>
                          setDraft({ ...draft, steps: draft.steps.filter((_, n) => n !== i) })
                        }
                      />
                    </div>
                    <label>
                      Step id
                      <input
                        style={field}
                        value={s.id}
                        onChange={(e) => change(i, { id: e.target.value })}
                      />
                    </label>
                    <label>
                      Label
                      <input
                        style={field}
                        value={s.label}
                        onChange={(e) => change(i, { label: e.target.value })}
                      />
                    </label>
                    <label>
                      Kind
                      <select
                        style={field}
                        value={s.kind}
                        onChange={(e) =>
                          change(i, {
                            kind: e.target.value as typeof s.kind,
                            independentOf: undefined,
                            repairOf: undefined,
                          })
                        }
                      >
                        {WORKFLOW_KINDS.map((v) => (
                          <option key={v}>{v}</option>
                        ))}
                      </select>
                    </label>
                    <label>
                      History stage
                      <select
                        style={field}
                        value={s.stage}
                        onChange={(e) => change(i, { stage: e.target.value as typeof s.stage })}
                      >
                        {TASK_STAGES.map((v) => (
                          <option key={v}>{v}</option>
                        ))}
                      </select>
                    </label>
                    <label>
                      Routing role
                      <select
                        style={field}
                        value={s.role}
                        onChange={(e) => change(i, { role: e.target.value as typeof s.role })}
                      >
                        {WORKFLOW_ROLES.map((v) => (
                          <option key={v}>{v}</option>
                        ))}
                      </select>
                    </label>
                    <label>
                      Condition
                      <select
                        style={field}
                        value={s.when}
                        onChange={(e) => change(i, { when: e.target.value as typeof s.when })}
                      >
                        <option value="always">Always</option>
                        <option value="material_risk">Material risk decision</option>
                      </select>
                    </label>
                    <label>
                      Dispatch template
                      <select
                        style={field}
                        value={s.template}
                        onChange={(e) => change(i, { template: e.target.value })}
                      >
                        {catalog.templates.map((t) => (
                          <option key={t.id}>{t.id}</option>
                        ))}
                      </select>
                    </label>
                    <p>
                      Inputs:{' '}
                      {catalog.templates
                        .find((t) => t.id === s.template)
                        ?.params.map(
                          (p) => `${p.name}${p.required ? ' (required)' : ' (optional)'}`,
                        )
                        .join(', ') || 'none'}
                      . The manager supplies these for each dispatch.
                    </p>
                    <details>
                      <summary>Preview template and result contract</summary>
                      <pre
                        style={{
                          whiteSpace: 'pre-wrap',
                          overflowWrap: 'anywhere',
                          fontSize: '0.72rem',
                        }}
                      >
                        {catalog.templates.find((t) => t.id === s.template)?.body}
                      </pre>
                      <pre
                        style={{
                          whiteSpace: 'pre-wrap',
                          overflowWrap: 'anywhere',
                          fontSize: '0.72rem',
                        }}
                      >
                        {JSON.stringify(
                          catalog.templates.find((t) => t.id === s.template)?.resultSchema,
                          null,
                          2,
                        )}
                      </pre>
                    </details>
                    <label>
                      Instructions
                      <textarea
                        style={{ ...field, minHeight: 80 }}
                        value={s.instructions}
                        onChange={(e) => change(i, { instructions: e.target.value })}
                      />
                    </label>
                    {s.kind === 'review' && (
                      <label>
                        Independent of
                        <select
                          style={field}
                          value={s.independentOf ?? ''}
                          onChange={(e) => {
                            const next = { ...s };
                            if (e.target.value) next.independentOf = e.target.value;
                            else next.independentOf = undefined;
                            change(i, next);
                          }}
                        >
                          <option value="">All steps use fresh sessions</option>
                          {draft.steps
                            .slice(0, i)
                            .filter((s) => ['implement', 'repair'].includes(s.kind))
                            .map((s) => (
                              <option key={s.id} value={s.id}>
                                {s.label}
                              </option>
                            ))}
                        </select>
                      </label>
                    )}
                    {s.kind === 'repair' && (
                      <label>
                        Optional bounded repair of
                        <select
                          style={field}
                          value={s.repairOf ?? ''}
                          onChange={(e) => {
                            const next = { ...s };
                            if (e.target.value) next.repairOf = e.target.value;
                            else delete next.repairOf;
                            setDraft({
                              ...draft,
                              steps: draft.steps.map((x, n) => (n === i ? next : x)),
                            });
                          }}
                        >
                          <option value="">No link (ordinary ordered step)</option>
                          {draft.steps
                            .slice(0, i)
                            .filter((s) => s.kind === 'review')
                            .map((s) => (
                              <option key={s.id} value={s.id}>
                                {s.label}
                              </option>
                            ))}
                        </select>
                      </label>
                    )}
                  </fieldset>
                ))}
                <SmallButton
                  label="Add step"
                  disabled={draft.steps.length >= 8}
                  onClick={() =>
                    setDraft({
                      ...draft,
                      steps: [
                        ...draft.steps,
                        {
                          id: `step-${draft.steps.length + 1}`,
                          label: 'Research',
                          kind: 'research',
                          stage: 'scout',
                          role: 'scout',
                          when: 'always',
                          template: 'scout-task',
                          instructions: '',
                        },
                      ],
                    })
                  }
                />
                {validation && (
                  <div role="alert" style={{ color: 'var(--wks-error)' }}>
                    {validation}
                  </div>
                )}
                <p>{reviewPolicy(draft)}</p>
                <button
                  type="button"
                  onClick={() =>
                    void request({
                      op: editing ? 'update' : 'create',
                      id: draft.id,
                      expectedRevision: draft.revision,
                      definition: draft,
                    }).then((res) => {
                      if (res) setDraft(undefined);
                    })
                  }
                  disabled={!!validation || busy}
                  style={{ ...inputStyle, cursor: 'pointer' }}
                >
                  Save definition
                </button>
              </fieldset>
              {editing && (
                <SmallButton
                  label="Load latest definition (discard draft)"
                  onClick={() => {
                    const current = catalog.definitions.find((d) => d.id === draft.id);
                    if (current) setDraft(structuredClone(current));
                  }}
                />
              )}
              <SmallButton label="Close editor" onClick={() => setDraft(undefined)} />
            </div>
          )}
        </>
      )}
    </section>
  );
}
