import IntentCompletion from './IntentCompletion';
import IntentAutomation from './IntentAutomation';
import React, { useEffect, useRef, useState } from 'react';
import {
  ArrowLeft,
  BookOpen,
  CheckCheck,
  ChevronDown,
  Compass,
  FolderOpen,
  History,
  Images,
  Link2,
  MessageSquare,
  Play,
  Plus,
  RefreshCw,
  Target,
} from 'lucide-react';
import type { ProjectIdentity } from '../hooks/useConfig';
import { Surface } from './Surface';
import IntentExecutions, { type IntentExecutionsProps } from './IntentExecutions';
import IntentSteering, { type IntentDirectionDraft } from './IntentSteering';
import IntentEvidence, { type IntentEvidenceDraft } from './IntentEvidence';
import IntentControls, {
  type IntentControlDraft,
  type IntentReconciliationDraft,
} from './IntentControls';
import IntentSources from './IntentSources';
import type { IntentSourceDraft } from '../../../main/shared/intentSources';
import IntentProjects, { type IntentProjectsDraft } from './IntentProjects';
import type { IntentProject } from '../../../main/shared/intentProject';
import IntentKnowledge, { type IntentKnowledgeDraft } from './IntentKnowledge';
import IntentArtifacts from './IntentArtifacts';
import IntentOverview, { IntentAttentionBadge } from './IntentOverview';
import {
  INTENT_STATUSES,
  type IntentFields,
  type IntentRevision,
  type IntentWorkspace,
  type IntentWorkspaceRequest,
  type IntentSessionRef,
} from '../../../main/shared/intentWorkspace';
import './IntentWorkspaces.css';

const EMPTY: IntentFields = {
  title: '',
  outcome: '',
  constraints: '',
  successCriteria: '',
  sourceUrl: '',
  status: 'draft',
};
type Draft = { fields: IntentFields; revision: number; reason: string; updatedAt?: string };
const WORK_VIEWS = [
  { id: 'overview', label: 'Overview', icon: Compass },
  { id: 'intent', label: 'Intent', icon: Target },
  { id: 'execution', label: 'Execution', icon: Play },
  { id: 'direction', label: 'Direction', icon: MessageSquare },
  { id: 'review', label: 'Review', icon: CheckCheck },
  { id: 'sources', label: 'Sources', icon: Link2 },
  { id: 'knowledge', label: 'Knowledge', icon: BookOpen },
  { id: 'artifacts', label: 'Artifacts', icon: Images },
  { id: 'history', label: 'History', icon: History },
] as const;
type WorkView = (typeof WORK_VIEWS)[number]['id'];
interface Props {
  projects?: Record<string, ProjectIdentity>;
  defaultRoot?: string;
  onClose: () => void;
  execution?: Omit<IntentExecutionsProps, 'workspace' | 'disabled'>;
  onOpenUrl?: (url: string) => void;
}

async function request(input: IntentWorkspaceRequest) {
  const api = window.electronAPI.intentWorkspaceRequest;
  if (!api) throw new Error('Intent workspaces require an updated Workspacer host.');
  return api(input);
}

/** Feature-owned state; agent panes remain mounted behind this work surface. */
export default function IntentWorkspaces({
  projects = {},
  defaultRoot = '',
  onClose,
  execution,
  onOpenUrl,
}: Props) {
  const [workspaces, setWorkspaces] = useState<IntentWorkspace[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [drafts, setDrafts] = useState<Record<string, Draft>>({});
  const [creating, setCreating] = useState(false);
  const [newFields, setNewFields] = useState<IntentFields>({ ...EMPTY });
  const [projectRoot, setProjectRoot] = useState(defaultRoot || Object.keys(projects)[0] || '');
  const [statusEvents, setStatusEvents] = useState<
    { at: string; status: string; reason: string }[]
  >([]);
  const [revisions, setRevisions] = useState<IntentRevision[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [historyError, setHistoryError] = useState('');
  const [notice, setNotice] = useState('');
  const [views, setViews] = useState<Record<string, WorkView>>({});
  const [directionDrafts, setDirectionDrafts] = useState<Record<string, IntentDirectionDraft>>({});
  const [evidenceDrafts, setEvidenceDrafts] = useState<Record<string, IntentEvidenceDraft>>({});
  const [controlDrafts, setControlDrafts] = useState<Record<string, IntentControlDraft>>({});
  const [directionAssessments, setDirectionAssessments] = useState<
    Record<string, Record<string, IntentReconciliationDraft>>
  >({});
  const [controlAssessments, setControlAssessments] = useState<
    Record<string, Record<string, IntentReconciliationDraft>>
  >({});
  const [sourceDrafts, setSourceDrafts] = useState<Record<string, IntentSourceDraft>>({});
  const [knowledgeDrafts, setKnowledgeDrafts] = useState<Record<string, IntentKnowledgeDraft>>({});
  const [projectDraft, setProjectDraft] = useState<IntentProjectsDraft>();
  const [showProjects, setShowProjects] = useState(false);
  const [stableProjects, setStableProjects] = useState<IntentProject[]>([]);
  const [executionIndex, setExecutionIndex] = useState<Record<string, IntentSessionRef[]>>({});
  const [visitedArtifacts, setVisitedArtifacts] = useState<string[]>([]);
  const [workListOpen, setWorkListOpen] = useState(false);
  const [filter, setFilter] = useState('');
  const listGeneration = useRef(0);
  const saveBusy = useRef(false);
  const contentRef = useRef<HTMLDivElement>(null);
  const tabsRef = useRef<HTMLDivElement>(null);
  const scrollPositions = useRef<Record<string, number>>({});
  const view = selected ? views[selected] || 'overview' : 'intent';
  const workspace = workspaces.find((item) => item.id === selected);
  const draft = selected ? drafts[selected] : undefined;
  const fields = creating ? newFields : (draft?.fields ?? workspace);
  const roots = [
    ...new Set([
      ...Object.keys(projects),
      ...workspaces.map((item) => item.projectRoot),
      ...stableProjects.flatMap((project) =>
        project.repositories.map((repository) => repository.root),
      ),
    ]),
  ].sort();
  const projectGroups = [
    ...stableProjects.map((project) => ({
      id: project.id,
      label: project.name,
      roots: project.repositories.map((repository) => repository.root),
      items: workspaces.filter((item) => item.projectId === project.id),
    })),
    ...roots
      .filter(
        (root) =>
          !stableProjects.some((project) =>
            project.repositories.some((repository) => repository.root === root),
          ),
      )
      .map((root) => ({
        id: root,
        label: projects[root]?.label || root.split(/[\\/]/).filter(Boolean).pop() || root,
        roots: [root],
        items: workspaces.filter((item) => item.projectRoot === root),
      })),
  ];
  const filteredGroups = projectGroups
    .map((group) => ({
      ...group,
      items: group.items.filter((item) =>
        `${item.title} ${item.outcome} ${group.label}`.toLowerCase().includes(filter.toLowerCase()),
      ),
    }))
    .filter((group) => !filter || group.items.length);

  function navigate(next: WorkView) {
    if (!selected) return;
    setNotice('');
    setViews((current) => ({ ...current, [selected]: next }));
    if (next === 'artifacts')
      setVisitedArtifacts((current) =>
        current.includes(selected) ? current : [...current, selected],
      );
  }
  useEffect(() => {
    const key = `${selected || 'new'}:${creating ? 'create' : showProjects ? 'projects' : view}`;
    const element = contentRef.current;
    if (element) element.scrollTop = scrollPositions.current[key] || 0;
    const active = tabsRef.current?.querySelector<HTMLElement>('[aria-selected="true"]');
    const rail = tabsRef.current;
    if (active && rail) {
      const left = active.offsetLeft - rail.offsetLeft;
      if (left < rail.scrollLeft) rail.scrollLeft = left;
      else if (left + active.offsetWidth > rail.scrollLeft + rail.clientWidth)
        rail.scrollLeft = left + active.offsetWidth - rail.clientWidth;
    }
  }, [selected, creating, view, showProjects]);

  async function refresh() {
    const mine = ++listGeneration.current;
    setLoading(true);
    try {
      const result = await request({ action: 'list' });
      if (mine !== listGeneration.current) return;
      if (result?.action !== 'list' || !Array.isArray(result.workspaces))
        throw new Error('Unexpected workspace response');
      setWorkspaces(result.workspaces);
      setStableProjects(result.projects || []);
      setExecutionIndex(result.executionIndex || {});
      setSelected((current) => current ?? result.workspaces[0]?.id ?? null);
    } catch (error) {
      if (mine === listGeneration.current)
        setError(String(error instanceof Error ? error.message : error));
    } finally {
      if (mine === listGeneration.current) setLoading(false);
    }
  }

  useEffect(() => {
    void refresh();
    return () => {
      listGeneration.current++;
    };
  }, []);
  useEffect(() => {
    let current = true;
    setRevisions([]);
    setHistoryError('');
    if (selected && !creating)
      void request({ action: 'history', id: selected })
        .then((result) => {
          if (current && result.action === 'history') {
            setRevisions(result.revisions);
            setStatusEvents(result.statusEvents || []);
          }
        })
        .catch((error) => {
          if (current) setHistoryError(String(error instanceof Error ? error.message : error));
        });
    return () => {
      current = false;
    };
  }, [selected, workspace?.revision, workspace?.updatedAt, creating]);

  useEffect(() => {
    let alive = true;
    const timer = setInterval(() => {
      if (saveBusy.current) return;
      const generation = listGeneration.current;
      void request({ action: 'list' })
        .then((result) => {
          if (
            alive &&
            !saveBusy.current &&
            generation === listGeneration.current &&
            result.action === 'list'
          ) {
            setWorkspaces(result.workspaces);
            setExecutionIndex(result.executionIndex || {});
          }
        })
        .catch(() => {});
    }, 3000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, []);

  function edit(patch: Partial<IntentFields>) {
    setNotice('');
    if (creating) setNewFields((current) => ({ ...current, ...patch }));
    else if (workspace)
      setDrafts((current) => {
        const nextDraft = {
          fields: { ...(current[workspace.id]?.fields ?? workspace), ...patch },
          revision: current[workspace.id]?.revision ?? workspace.revision,
          updatedAt: current[workspace.id]?.updatedAt ?? workspace.updatedAt,
          reason: current[workspace.id]?.reason ?? '',
        };
        const next = { ...current };
        if (
          !(Object.keys(EMPTY) as (keyof IntentFields)[]).some(
            (key) => nextDraft.fields[key] !== workspace[key],
          ) &&
          !nextDraft.reason.trim()
        )
          delete next[workspace.id];
        else next[workspace.id] = nextDraft;
        return next;
      });
  }

  async function save(event: React.FormEvent) {
    event.preventDefault();
    if (!fields || saveBusy.current) return;
    saveBusy.current = true;
    listGeneration.current++;
    const wasCreating = creating;
    const savedDraft = draft;
    setSaving(true);
    setError('');
    setNotice('');
    try {
      const result = await request(
        creating
          ? { action: 'create', projectRoot, fields }
          : {
              action: 'update',
              id: workspace!.id,
              expectedRevision: draft?.revision ?? workspace!.revision,
              fields,
              expectedUpdatedAt: draft?.updatedAt ?? workspace!.updatedAt,
              reason: draft?.reason ?? '',
            },
      );
      if (result.action !== 'create' && result.action !== 'update')
        throw new Error('Unexpected workspace response');
      const saved = result.workspace;
      setWorkspaces((current) => [saved, ...current.filter((item) => item.id !== saved.id)]);
      setDrafts((current) => {
        const next = { ...current };
        if (current[saved.id] === savedDraft) delete next[saved.id];
        return next;
      });
      setSelected(saved.id);
      if (saved.status === 'active' || wasCreating)
        setViews((current) => ({
          ...current,
          [saved.id]: saved.status === 'active' ? 'overview' : 'intent',
        }));
      setCreating(false);
      setWorkListOpen(false);
      setNewFields({ ...EMPTY });
      setNotice(`Saved revision ${saved.revision}.`);
    } catch (error) {
      setError(String(error instanceof Error ? error.message : error));
    } finally {
      saveBusy.current = false;
      setSaving(false);
    }
  }

  const dirty = creating || !!draft;
  return (
    <section className="intent-workspaces" aria-label="Intent workspaces">
      <header className="intent-header">
        <div>
          <strong>
            <Target size={16} /> Work
          </strong>
          <span className="intent-preview-badge">Preview</span>
        </div>
        <div className="intent-header-actions">
          <button
            type="button"
            className="intent-list-toggle"
            aria-expanded={workListOpen}
            aria-controls="intent-project-list"
            onClick={() => setWorkListOpen((open) => !open)}
          >
            <FolderOpen size={14} aria-hidden="true" /> Work list{' '}
            <ChevronDown size={12} aria-hidden="true" />
          </button>
          <button type="button" onClick={onClose} aria-label="Back to agents">
            <ArrowLeft size={14} aria-hidden="true" />
            <span className="intent-back-label">Back to agents</span>
            <span className="intent-back-compact">Agents</span>
          </button>
        </div>
      </header>
      {notice && (
        <p role="status" className="intent-notice">
          {notice}
        </p>
      )}
      <div className="intent-layout">
        <aside
          id="intent-project-list"
          className="intent-list"
          data-open={workListOpen}
          aria-label="Project work"
        >
          <div className="intent-actions">
            <button
              type="button"
              disabled={saving || loading}
              onClick={() => {
                setCreating(true);
                setShowProjects(false);
                setWorkListOpen(false);
                setError('');
                setNotice('');
              }}
            >
              <Plus size={14} /> New intent
            </button>
            <button
              type="button"
              disabled={saving || loading}
              aria-label="Refresh workspaces"
              onClick={() => {
                setError('');
                void refresh();
              }}
            >
              <RefreshCw size={14} />
            </button>
            <button
              type="button"
              aria-pressed={showProjects}
              onClick={() => setShowProjects((current) => !current)}
              title="Manage projects and repositories"
            >
              <FolderOpen size={14} aria-hidden="true" />
              <span className="intent-sr-only">Projects</span>
            </button>
          </div>
          {workspaces.length > 0 && (
            <label className="intent-filter">
              <span className="intent-sr-only">Find work</span>
              <input
                type="search"
                placeholder="Find work…"
                value={filter}
                onChange={(event) => setFilter(event.target.value)}
              />
            </label>
          )}
          {loading && <p role="status">Loading work…</p>}
          {!loading && filter && !filteredGroups.length && (
            <p role="status" className="intent-muted">
              No work matches “{filter}”.
            </p>
          )}
          {filteredGroups.map((group) => {
            const items = group.items;
            return (
              <div key={group.id} className="intent-project">
                <h3 title={group.roots.join('\n')}>
                  <span>{group.label}</span>
                  <span className="intent-project-count">{items.length}</span>
                </h3>
                {items.length === 0 && <p className="intent-muted">No work yet</p>}
                {items.map((item) => (
                  <button
                    key={item.id}
                    type="button"
                    disabled={saving}
                    aria-label={`${item.title}${drafts[item.id] ? ' · Unsaved' : ''} ${item.status}`}
                    aria-current={!creating && selected === item.id ? 'page' : undefined}
                    className="intent-item"
                    onClick={() => {
                      setSelected(item.id);
                      setShowProjects(false);
                      setWorkListOpen(false);
                      setCreating(false);
                      setError('');
                      setNotice('');
                    }}
                  >
                    <span>
                      {item.title}
                      {drafts[item.id] ? ' · Unsaved' : ''}
                    </span>
                    <small>
                      <span className="intent-status-dot" data-status={item.status} />
                      {item.status}
                    </small>
                    <IntentAttentionBadge
                      refs={executionIndex[item.id] || []}
                      sessions={execution?.sessions || []}
                    />
                  </button>
                ))}
              </div>
            );
          })}
        </aside>
        <main
          className="intent-detail"
          ref={contentRef}
          onScroll={(event) => {
            scrollPositions.current[
              `${selected || 'new'}:${creating ? 'create' : showProjects ? 'projects' : view}`
            ] = event.currentTarget.scrollTop;
          }}
        >
          <IntentProjects
            visible={showProjects && execution?.visible !== false}
            disabled={saving}
            draft={projectDraft}
            onDraftChange={setProjectDraft}
            onChanged={() => void refresh()}
          />
          <div hidden={showProjects}>
            {error && (
              <p role="alert" className="intent-error">
                {error}
              </p>
            )}
            {!fields && !loading && !error && (
              <div className="intent-empty">
                <Target size={24} />
                <h2>Give your next feature a home</h2>
                <p>Capture what you want to achieve and keep the decisions with the work.</p>
                <button type="button" onClick={() => setCreating(true)}>
                  Create an intent
                </button>
              </div>
            )}
            {fields && (
              <>
                <div className="intent-workspace-chrome">
                  <div className="intent-workspace-heading">
                    <div>
                      <p className="intent-eyebrow">
                        {creating
                          ? 'Start with an outcome'
                          : stableProjects.find((project) => project.id === workspace?.projectId)
                              ?.name ||
                            workspace?.projectRoot.split(/[\\/]/).filter(Boolean).pop() ||
                            'Project work'}
                      </p>
                      <h2>{creating ? 'New intent' : workspace?.title}</h2>
                    </div>
                    {!creating && (
                      <div className="intent-workspace-meta">
                        <span className="intent-status-pill" data-status={workspace?.status}>
                          <span className="intent-status-dot" data-status={workspace?.status} />
                          {workspace?.status === 'review' ? 'Review needed' : workspace?.status}
                        </span>
                        <span className="intent-muted">
                          Revision {workspace?.revision}
                          {dirty ? ' · Unsaved changes' : ''}
                        </span>
                      </div>
                    )}
                  </div>
                  {!creating && execution && (
                    <div
                      className="intent-tabs"
                      ref={tabsRef}
                      role="tablist"
                      aria-label="Workspace views"
                      onKeyDown={(event) => {
                        if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
                        event.preventDefault();
                        const current = WORK_VIEWS.findIndex((tab) => tab.id === view);
                        const next =
                          event.key === 'Home'
                            ? 0
                            : event.key === 'End'
                              ? WORK_VIEWS.length - 1
                              : (current +
                                  (event.key === 'ArrowRight' ? 1 : -1) +
                                  WORK_VIEWS.length) %
                                WORK_VIEWS.length;
                        navigate(WORK_VIEWS[next].id);
                        tabsRef.current
                          ?.querySelectorAll<HTMLButtonElement>('[role="tab"]')
                          [next]?.focus();
                      }}
                    >
                      {WORK_VIEWS.map(({ id: tab, label, icon: Icon }) => (
                        <button
                          key={tab}
                          type="button"
                          role="tab"
                          id={`intent-tab-${selected}-${tab}`}
                          aria-controls={`intent-panel-${selected}`}
                          aria-selected={view === tab}
                          tabIndex={view === tab ? 0 : -1}
                          onClick={() => navigate(tab)}
                        >
                          <Icon size={14} aria-hidden="true" />
                          {label}
                        </button>
                      ))}
                    </div>
                  )}
                </div>
                <div
                  id={`intent-panel-${selected}`}
                  role={!creating && execution ? 'tabpanel' : undefined}
                  aria-labelledby={
                    !creating && execution ? `intent-tab-${selected}-${view}` : undefined
                  }
                  className="intent-view-panel"
                >
                  <form
                    className="intent-editor"
                    onSubmit={save}
                    hidden={!creating && !!execution && view !== 'intent'}
                  >
                    <fieldset disabled={saving}>
                      {creating && (
                        <label>
                          Project directory
                          <input
                            required
                            list="intent-project-roots"
                            value={projectRoot}
                            onChange={(event) => setProjectRoot(event.target.value)}
                            placeholder="Absolute directory on the connected host"
                            maxLength={4096}
                          />
                          <datalist id="intent-project-roots">
                            {roots.map((root) => (
                              <option key={root} value={root}>
                                {projects[root]?.label || root}
                              </option>
                            ))}
                          </datalist>
                        </label>
                      )}
                      {!creating && (
                        <p className="intent-root" title={workspace?.projectRoot}>
                          {workspace?.projectRoot}
                        </p>
                      )}
                      <label>
                        Title
                        <input
                          required
                          maxLength={240}
                          aria-label="Title"
                          value={fields.title}
                          onChange={(event) => edit({ title: event.target.value })}
                          placeholder="Export filtered results"
                        />
                      </label>
                      <label>
                        Desired outcome
                        <textarea
                          rows={3}
                          maxLength={32000}
                          aria-label="Desired outcome"
                          value={fields.outcome}
                          onChange={(event) => edit({ outcome: event.target.value })}
                          placeholder="What should become possible, and for whom?"
                        />
                      </label>
                      <div className="intent-columns">
                        <label>
                          Constraints
                          <textarea
                            rows={4}
                            maxLength={32000}
                            aria-label="Constraints"
                            value={fields.constraints}
                            onChange={(event) => edit({ constraints: event.target.value })}
                            placeholder="What must hold true? What is out of scope?"
                          />
                        </label>
                        <label>
                          Success criteria
                          <textarea
                            rows={4}
                            maxLength={32000}
                            aria-label="Success criteria"
                            value={fields.successCriteria}
                            onChange={(event) => edit({ successCriteria: event.target.value })}
                            placeholder="One criterion per line. What evidence will show this is done?"
                          />
                        </label>
                      </div>
                      <label>
                        Source link
                        <input
                          type="url"
                          maxLength={4096}
                          aria-label="Source link"
                          value={fields.sourceUrl}
                          onChange={(event) => edit({ sourceUrl: event.target.value })}
                          placeholder="ADO, Jira, or another source URL"
                        />
                        <small className="intent-muted">
                          A quick reference. Use Sources to import and review ticket requirements.
                        </small>
                      </label>
                      <label>
                        Workspace status
                        <select
                          aria-describedby="intent-status-help"
                          value={fields.status}
                          onChange={(event) =>
                            edit({ status: event.target.value as IntentFields['status'] })
                          }
                        >
                          {INTENT_STATUSES.map((status) => (
                            <option key={status} value={status}>
                              {status[0].toUpperCase() + status.slice(1)}
                            </option>
                          ))}
                        </select>
                      </label>
                      <p id="intent-status-help" className="intent-muted">
                        Saving Active starts or resumes a dedicated manager using your Fleet Manager
                        settings, existing permissions, and a 60-minute work limit. It asks for
                        decisions and returns work to Review. Status changes preserve evidence;
                        requirement edits create a new revision and are sent to active work. PR
                        links do not track merge status. Use Review to verify and accept results.
                      </p>
                      {!creating && (
                        <label>
                          Reason for this revision
                          <input
                            maxLength={4000}
                            value={draft?.reason ?? ''}
                            onChange={(event) => {
                              if (workspace)
                                setDrafts((current) => ({
                                  ...current,
                                  [workspace.id]: {
                                    fields: current[workspace.id]?.fields ?? workspace,
                                    revision: current[workspace.id]?.revision ?? workspace.revision,
                                    updatedAt:
                                      current[workspace.id]?.updatedAt ?? workspace.updatedAt,
                                    reason: event.target.value,
                                  },
                                }));
                            }}
                            placeholder="What changed, and why?"
                          />
                        </label>
                      )}
                      <div className="intent-actions">
                        <button
                          type="submit"
                          className="intent-primary"
                          disabled={!dirty || loading}
                        >
                          {saving
                            ? 'Saving…'
                            : fields.status === 'active' &&
                                (creating || workspace?.status !== 'active')
                              ? creating
                                ? 'Create and activate'
                                : 'Save and activate'
                              : creating
                                ? 'Create workspace'
                                : 'Save revision'}
                        </button>
                        {!creating && dirty && (
                          <button
                            type="button"
                            onClick={() => {
                              setDrafts((current) => {
                                const next = { ...current };
                                delete next[selected!];
                                return next;
                              });
                              void refresh();
                            }}
                          >
                            Discard draft and reload
                          </button>
                        )}
                      </div>
                    </fieldset>
                  </form>
                  {!creating && workspace && execution && (
                    <div hidden={view !== 'execution'}>
                      <IntentExecutions
                        key={workspace.id}
                        workspace={workspace}
                        disabled={dirty || saving}
                        {...execution}
                        onLinksChanged={() => void refresh()}
                        visible={execution.visible !== false && view === 'execution'}
                      />
                    </div>
                  )}
                  {!creating && workspace && execution && (
                    <IntentSteering
                      key={`direction-${workspace.id}`}
                      workspace={workspace}
                      visible={execution.visible !== false && view === 'direction'}
                      disabled={dirty || saving}
                      draft={directionDrafts[workspace.id]}
                      onDraftChange={(next) =>
                        setDirectionDrafts((current) => ({ ...current, [workspace.id]: next }))
                      }
                      sessions={execution.sessions}
                      onOpenSession={execution.onOpenSession}
                      assessmentDrafts={directionAssessments[workspace.id]}
                      onAssessmentDraftsChange={(next) =>
                        setDirectionAssessments((current) => ({ ...current, [workspace.id]: next }))
                      }
                    />
                  )}
                  {!creating && workspace && execution && (
                    <>
                      {['overview', 'review'].includes(view) && (
                        <IntentCompletion
                          key={`completion-${workspace.id}`}
                          workspace={workspace}
                          disabled={dirty || saving}
                          onChanged={() => void refresh()}
                          onEvidence={() => navigate('review')}
                          onOpenSession={execution.onOpenSession}
                        />
                      )}
                      {view === 'overview' && (
                        <IntentAutomation
                          key={`automation-${workspace.id}`}
                          workspace={workspace}
                          disabled={dirty || saving}
                          sessions={execution.sessions}
                          onChanged={() => void refresh()}
                          onOpenSession={execution.onOpenSession}
                        />
                      )}
                      <IntentOverview
                        key={`overview-${workspace.id}`}
                        workspace={workspace}
                        visible={execution.visible !== false && view === 'overview'}
                        sessions={execution.sessions}
                        onNavigate={navigate}
                        onOpenSession={execution.onOpenSession}
                      />
                      <IntentSources
                        key={`sources-${workspace.id}`}
                        workspace={workspace}
                        visible={execution.visible !== false && view === 'sources'}
                        disabled={dirty || saving}
                        draft={sourceDrafts[workspace.id]}
                        onDraftChange={(next) =>
                          setSourceDrafts((current) => ({ ...current, [workspace.id]: next }))
                        }
                      />
                      <IntentKnowledge
                        key={`knowledge-${workspace.id}`}
                        workspace={workspace}
                        visible={execution.visible !== false && view === 'knowledge'}
                        disabled={dirty || saving}
                        draft={knowledgeDrafts[workspace.id]}
                        onDraftChange={(next) =>
                          setKnowledgeDrafts((current) => ({ ...current, [workspace.id]: next }))
                        }
                      />
                      <IntentControls
                        key={`controls-${workspace.id}`}
                        workspace={workspace}
                        visible={execution.visible !== false && view === 'execution'}
                        disabled={dirty || saving}
                        draft={controlDrafts[workspace.id]}
                        onDraftChange={(next) =>
                          setControlDrafts((current) => ({ ...current, [workspace.id]: next }))
                        }
                        sessions={execution.sessions}
                        onOpenSession={execution.onOpenSession}
                        assessmentDrafts={controlAssessments[workspace.id]}
                        onAssessmentDraftsChange={(next) =>
                          setControlAssessments((current) => ({ ...current, [workspace.id]: next }))
                        }
                      />
                      <IntentEvidence
                        key={`evidence-${workspace.id}`}
                        workspace={workspace}
                        visible={execution.visible !== false && view === 'review'}
                        disabled={dirty || saving}
                        draft={evidenceDrafts[workspace.id]}
                        onDraftChange={(next) =>
                          setEvidenceDrafts((current) => ({ ...current, [workspace.id]: next }))
                        }
                      />
                    </>
                  )}
                  {visitedArtifacts.map((id) => {
                    const item = workspaces.find((workspace) => workspace.id === id);
                    return (
                      item && (
                        <IntentArtifacts
                          key={`artifacts-${id}`}
                          workspace={item}
                          visible={
                            !creating &&
                            item.id === selected &&
                            execution?.visible !== false &&
                            view === 'artifacts'
                          }
                          disabled={saving || !!drafts[id]}
                          onOpenUrl={onOpenUrl}
                        />
                      )
                    );
                  })}
                  {!creating && (
                    <Surface
                      elevation="flat"
                      className="intent-history"
                      hidden={!!execution && view !== 'history'}
                    >
                      <h3>Intent history</h3>
                      {historyError && (
                        <p role="alert" className="intent-error">
                          {historyError}
                        </p>
                      )}
                      {statusEvents.map((event, index) => (
                        <p key={`${event.at}-${index}`} className="intent-muted">
                          {new Date(event.at).toLocaleString()} · {event.status} · {event.reason}
                        </p>
                      ))}
                      {revisions.map((revision) => (
                        <details key={revision.revision}>
                          <summary>
                            Revision {revision.revision} · {revision.reason}
                            <small>{new Date(revision.at).toLocaleString()}</small>
                          </summary>
                          <dl>
                            <dt>Title</dt>
                            <dd>{revision.snapshot.title}</dd>
                            <dt>Outcome</dt>
                            <dd>{revision.snapshot.outcome || 'Not specified'}</dd>
                            <dt>Constraints</dt>
                            <dd>{revision.snapshot.constraints || 'Not specified'}</dd>
                            <dt>Success criteria</dt>
                            <dd>{revision.snapshot.successCriteria || 'Not specified'}</dd>
                            <dt>Source</dt>
                            <dd>{revision.snapshot.sourceUrl || 'None'}</dd>
                            <dt>Status</dt>
                            <dd>{revision.snapshot.status}</dd>
                          </dl>
                        </details>
                      ))}
                    </Surface>
                  )}
                </div>
              </>
            )}
          </div>
        </main>
      </div>
    </section>
  );
}
