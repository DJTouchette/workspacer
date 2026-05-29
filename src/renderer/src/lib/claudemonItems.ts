/**
 * Thin client for claudemon's v2 items API. Used by the L1 inbox view.
 * Endpoints documented in spec §12 and implemented at
 * claudemon/src/daemon/api.rs.
 */

export interface ItemRow {
  id: string;
  session_id: string;
  state: 'unread' | 'read' | 'snoozed' | 'resolved';
  priority: number;
  kind: 'needs_input' | 'error' | 'stuck' | 'done' | 'working_milestone';
  summary: string | null;
  context_paragraph: string | null;
  next_action: string | null;
  triggering_event_id: number | null;
  created_at: number;
  updated_at: number;
  resolved_at: number | null;
  snoozed_until: number | null;
  snoozed_on_event: string | null;
  flagged: boolean;
  session_name: string;
  session_project: string;
  session_state: string;
  session_cwd: string;
}

export type ItemChange =
  | { type: 'item_created'; item: ItemRow }
  | { type: 'item_changed'; item: ItemRow }
  | { type: 'item_resolved'; id: string; session_id: string };

export type ItemAction =
  | { action: 'archive' }
  | { action: 'snooze_until'; until: number }
  | { action: 'snooze_on_event'; on: string }
  | { action: 'unsnooze' }
  | { action: 'flag' }
  | { action: 'unflag' };

export interface ListFilter {
  include_snoozed?: boolean;
  include_resolved?: boolean;
}

const DEFAULT_BASE = 'http://127.0.0.1:7891';

export class ClaudemonItemsClient {
  constructor(private readonly baseUrl: string = DEFAULT_BASE) {}

  async list(filter: ListFilter = {}): Promise<ItemRow[]> {
    const params = new URLSearchParams();
    if (filter.include_snoozed) params.set('include_snoozed', 'true');
    if (filter.include_resolved) params.set('include_resolved', 'true');
    const qs = params.toString();
    const url = qs ? `${this.baseUrl}/items?${qs}` : `${this.baseUrl}/items`;
    const res = await fetch(url);
    if (!res.ok) throw new Error(`list failed: ${res.status}`);
    const body = (await res.json()) as { items: ItemRow[] };
    return body.items;
  }

  async action(id: string, action: ItemAction): Promise<ItemRow> {
    const res = await fetch(`${this.baseUrl}/items/${id}/action`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(action),
    });
    if (!res.ok) {
      const text = await res.text();
      throw new Error(`action failed: ${res.status} ${text}`);
    }
    return (await res.json()) as ItemRow;
  }

  /**
   * Subscribe to live item changes. Returns a cleanup function that closes
   * the underlying EventSource.
   */
  subscribe(onChange: (change: ItemChange) => void, onError?: (err: Event) => void): () => void {
    const es = new EventSource(`${this.baseUrl}/items/stream`);
    es.addEventListener('item', (e) => {
      try {
        onChange(JSON.parse((e as MessageEvent).data) as ItemChange);
      } catch (err) {
        // Malformed payload: skip it; surface as a generic error.
        if (onError) onError(new Event('parse-error'));
      }
    });
    if (onError) es.addEventListener('error', onError);
    return () => es.close();
  }
}
