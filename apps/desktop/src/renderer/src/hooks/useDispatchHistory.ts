import { useSyncExternalStore } from 'react';
import {
  TASK_INSPECTOR_UNAVAILABLE,
  type DispatchHistoryResponse,
  type DispatchTask,
} from '../../../main/shared/dispatchHistory';

type HistoryState = { data?: DispatchHistoryResponse; error: string; busy: boolean };
const empty: HistoryState = { error: '', busy: false };
const remoteState: HistoryState = {
  ...empty,
  data: { available: false, reason: TASK_INSPECTOR_UNAVAILABLE },
};

/** One timer and one in-flight read for all mounted task/history views. */
export function createDispatchHistoryPoller(read: () => Promise<DispatchHistoryResponse>) {
  let state = empty;
  let generation = 0;
  let inFlight: Promise<void> | undefined;
  let timer: ReturnType<typeof setInterval> | undefined;
  const listeners = new Set<() => void>();
  const publish = (next: HistoryState) => {
    state = next;
    for (const listener of listeners) listener();
  };
  const refresh = (): Promise<void> => {
    if (inFlight) return inFlight;
    const current = ++generation;
    publish({ ...state, busy: true });
    // Defer read so even synchronously thrown backend errors take the error path.
    const request = Promise.resolve()
      .then(read)
      .then(
        (data) => {
          if (current === generation) publish({ data, error: '', busy: false });
        },
        (error) => {
          if (current === generation) publish({ ...state, error: String(error), busy: false });
        },
      )
      .finally(() => {
        if (inFlight === request) inFlight = undefined;
      });
    inFlight = request;
    return request;
  };
  return {
    getSnapshot: () => state,
    subscribe(listener: () => void) {
      listeners.add(listener);
      if (listeners.size === 1) {
        void refresh();
        timer = setInterval(() => void refresh(), 3000);
      }
      return () => {
        listeners.delete(listener);
        if (!listeners.size) {
          clearInterval(timer);
          timer = undefined;
          generation++;
          inFlight = undefined;
          state = empty;
        }
      };
    },
    refresh,
    updateTask(task: DispatchTask) {
      // A read started before an edit must not overwrite its authoritative result.
      generation++;
      inFlight = undefined;
      const data = state.data;
      publish({
        ...state,
        busy: false,
        data: data?.available
          ? { ...data, tasks: data.tasks.map((t) => (t.taskId === task.taskId ? task : t)) }
          : data,
      });
    },
  };
}

const history = createDispatchHistoryPoller(
  async () =>
    (await window.electronAPI?.dispatchHistoryRead?.()) ?? {
      available: false,
      reason: TASK_INSPECTOR_UNAVAILABLE,
    },
);
const subscribeRemote = () => () => {};
const getRemote = () => remoteState;
const refreshRemote = async () => {};
const updateRemote = (_task: DispatchTask) => {};
export function useDispatchHistory(remote = false) {
  const state = useSyncExternalStore(
    remote ? subscribeRemote : history.subscribe,
    remote ? getRemote : history.getSnapshot,
  );
  return {
    ...state,
    refresh: remote ? refreshRemote : history.refresh,
    updateTask: remote ? updateRemote : history.updateTask,
  };
}
