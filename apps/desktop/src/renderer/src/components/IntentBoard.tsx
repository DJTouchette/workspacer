import React, { useState } from 'react';
import { ArrowRight, CheckCheck, Plus, Play } from 'lucide-react';
import { Surface } from './Surface';
import { IntentAttentionBadge } from './IntentOverview';
import { intentSessionAttention } from '../../../main/shared/intentSummary';
import type {
  IntentLiveSession,
  IntentSessionRef,
  IntentWorkspace,
} from '../../../main/shared/intentWorkspace';

const COLUMNS = [
  {
    status: 'draft',
    label: 'Draft',
    description: 'Shape the outcome',
    empty: 'Your next idea starts here.',
  },
  {
    status: 'active',
    label: 'Active',
    description: 'Work in motion',
    empty: 'Start an intent when it is ready.',
  },
  {
    status: 'review',
    label: 'Needs review',
    description: 'Ready for your decision',
    empty: 'Results ready to inspect appear here.',
  },
  {
    status: 'complete',
    label: 'Complete',
    description: 'Finished work',
    empty: 'Completed intents appear here.',
  },
] as const;

export default function IntentBoard({
  groups,
  selected,
  drafts,
  executionIndex,
  sessions,
  disabled,
  loading,
  onOpen,
  onCreate,
}: {
  groups: { id: string; label: string; items: IntentWorkspace[] }[];
  selected: string | null;
  drafts: Record<string, unknown>;
  executionIndex: Record<string, IntentSessionRef[]>;
  sessions: IntentLiveSession[];
  disabled: boolean;
  loading: boolean;
  onOpen: (id: string, view?: 'overview' | 'review') => void;
  onCreate: () => void;
}) {
  const [needsMe, setNeedsMe] = useState(false);
  const [project, setProject] = useState('');
  const [dragged, setDragged] = useState<string | null>(null);
  const [dropTarget, setDropTarget] = useState('');
  const items = groups
    .filter((group) => !project || group.id === project)
    .flatMap((group) =>
      group.items.map((item) => ({
        item,
        project: group.label,
        attention: intentSessionAttention(executionIndex[item.id] || [], sessions).length > 0,
      })),
    );
  const attentionCount = items.filter(
    ({ item, attention }) => item.status === 'review' || attention,
  ).length;
  const visible = items.filter(
    ({ item, attention }) => !needsMe || item.status === 'review' || attention,
  );
  return (
    <section className="intent-board" aria-label="Intent board">
      <div className="intent-board-filters">
        <label>
          <span className="intent-sr-only">Filter by project</span>
          <select value={project} onChange={(event) => setProject(event.target.value)}>
            <option value="">All projects</option>
            {groups.map((group) => (
              <option key={group.id} value={group.id}>
                {group.label}
              </option>
            ))}
          </select>
        </label>
        <button type="button" aria-pressed={needsMe} onClick={() => setNeedsMe(!needsMe)}>
          Needs me <span className="intent-board-count">{attentionCount}</span>
        </button>
        <span className="intent-muted">{visible.length} intents</span>
      </div>
      {loading && <p role="status">Loading work…</p>}
      {!loading && !visible.length && (
        <p role="status" className="intent-muted">
          {needsMe ? 'Nothing needs your attention.' : 'No intents match this view.'}
        </p>
      )}
      <div className="intent-board-columns">
        {COLUMNS.map(({ status, label, description, empty }) => {
          const cards = visible.filter(({ item }) => item.status === status);
          const canDrop =
            !!dragged &&
            status !== 'draft' &&
            items.some(({ item }) => item.id === dragged && item.status !== status);
          return (
            <section
              key={status}
              className="intent-board-column"
              aria-label={label}
              data-drop={dropTarget === status}
              onDragOver={(event) => {
                if (canDrop && !disabled) {
                  event.preventDefault();
                  event.dataTransfer.dropEffect = 'move';
                  setDropTarget(status);
                }
              }}
              onDragLeave={(event) => {
                if (!event.currentTarget.contains(event.relatedTarget as Node)) setDropTarget('');
              }}
              onDrop={(event) => {
                event.preventDefault();
                if (canDrop && !disabled && dragged)
                  onOpen(dragged, status === 'active' ? 'overview' : 'review');
                setDragged(null);
                setDropTarget('');
              }}
            >
              <header>
                <h3>
                  <span className="intent-status-dot" data-status={status} />
                  {label}
                  <span className="intent-board-count">{cards.length}</span>
                </h3>
                <p>{description}</p>
              </header>
              {dragged && canDrop && (
                <p className="intent-board-drop-hint">
                  {status === 'active'
                    ? 'Open start / resume controls'
                    : 'Open review before changing status'}
                </p>
              )}
              <div className="intent-board-cards">
                {cards.map(({ item, project: projectLabel, attention }) => (
                  <Surface
                    key={item.id}
                    elevation="raised"
                    className="intent-board-card"
                    data-selected={selected === item.id}
                    draggable={!disabled}
                    onDragStart={(event) => {
                      setDragged(item.id);
                      event.dataTransfer.setData('text/plain', item.id);
                      event.dataTransfer.effectAllowed = 'move';
                    }}
                    onDragEnd={() => {
                      setDragged(null);
                      setDropTarget('');
                    }}
                  >
                    <button
                      type="button"
                      className="intent-board-card-open"
                      disabled={disabled}
                      aria-label={`${item.title}${drafts[item.id] ? ' · Unsaved' : ''} ${item.status}`}
                      aria-current={selected === item.id ? 'page' : undefined}
                      onClick={() => onOpen(item.id)}
                    >
                      <span className="intent-board-project">{projectLabel}</span>
                      <strong>{item.title}</strong>
                      <span className="intent-board-outcome">
                        {item.outcome || 'Add the outcome you want to achieve.'}
                      </span>
                      {drafts[item.id] ? (
                        <span className="intent-muted">Unsaved changes</span>
                      ) : null}
                      <IntentAttentionBadge
                        refs={executionIndex[item.id] || []}
                        sessions={sessions}
                      />
                    </button>
                    <div className="intent-board-card-footer">
                      <span className="intent-muted">
                        {attention
                          ? 'Waiting for you'
                          : item.status === 'review'
                            ? 'Review the result'
                            : `${(executionIndex[item.id] || []).length} linked sessions`}
                      </span>
                      <button
                        type="button"
                        disabled={disabled}
                        aria-label={`${item.status === 'draft' ? 'Start' : item.status === 'review' ? 'Review' : 'Open work for'} ${item.title}`}
                        onClick={() =>
                          onOpen(item.id, item.status === 'review' ? 'review' : 'overview')
                        }
                      >
                        {item.status === 'draft' ? (
                          <Play size={12} />
                        ) : item.status === 'review' ? (
                          <CheckCheck size={12} />
                        ) : (
                          <ArrowRight size={12} />
                        )}
                        {item.status === 'draft'
                          ? 'Start'
                          : item.status === 'review'
                            ? 'Review'
                            : 'Open work'}
                      </button>
                    </div>
                  </Surface>
                ))}
                {!cards.length && !loading && <p className="intent-board-empty">{empty}</p>}
                {status === 'draft' && (
                  <button
                    type="button"
                    className="intent-board-add"
                    onClick={onCreate}
                    disabled={disabled || loading}
                  >
                    <Plus size={14} />
                    Create an intent
                  </button>
                )}
              </div>
            </section>
          );
        })}
      </div>
    </section>
  );
}
