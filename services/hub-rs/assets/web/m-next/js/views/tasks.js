// Background tasks (native ui/tasks.rs): the session's `background_task_list`
// as a bottom sheet. A shell's (or workflow's) row expands into its log, read
// through sessions.taskOutput — the tail first, then from where the last read
// ended, once a second while open — following the end until you scroll up.
// Agent rows open the subagent's own view. Stop asks first.
import { can } from '../bus.js';
import { sessions, tasksOf } from '../store.js';
import { esc, ic, fmtElapsed, errText } from '../util.js';
import { sheet, ask, closeSheet } from '../ui.js';
import * as act from '../actions.js';

const KIND = { local_bash: ['Shell', 'square-terminal'], local_agent: ['Subagent', 'bot'], in_process_teammate: ['Teammate', 'bot'],
  remote_agent: ['Cloud agent', 'globe'], local_workflow: ['Workflow', 'gallery-vertical-end'] };
const STATUS = { '': 'Running', running: 'Running', pending: 'Starting', completed: 'Done', failed: 'Failed', killed: 'Stopped', stopped: 'Stopped' };
const MAX_LOG = 1024 * 1024;

export function openTasks(s, { go }) {
  const id = s.sessionId;
  let openTask = null;          // task id whose log is showing
  let log = null;               // {task, text, next, size, done, running, truncated, error}
  let follow = true;
  let timer = null;
  let el = null;

  const rowsHtml = () => {
    const cur = sessions.get(id) || s;
    const tasks = tasksOf(cur);
    const running = tasks.filter((t) => t.running).length;
    const now = Date.now();
    const body = tasks.map((t) => {
      const [kind, icon] = KIND[t.taskType] || ['Task', 'square-terminal'];
      const tone = t.running ? 'busy' : t.status === 'failed' ? 'error' : t.status === 'completed' ? 'success' : 'muted';
      const elapsed = t.startedAt ? fmtElapsed((t.running ? now : t.endedAt || now) - t.startedAt) : '';
      const isAgent = t.taskType === 'local_agent' && t.subagentId;
      const shows = t.hasOutput && t.taskType !== 'local_agent';
      const isOpen = openTask === t.id;
      const title = t.description || kind;
      const meta = [kind, STATUS[t.status] || t.status, elapsed, t.usage && t.usage.totalTokens ? `${Math.round(t.usage.totalTokens / 100) / 10}k tokens` : ''].filter(Boolean).join(' · ');
      let detail = '';
      if (isOpen) {
        const text = log && log.task === t.id ? log.text : '';
        detail = `<div class="tkx"><pre class="log" data-log>${log && log.truncated ? '<span class="dim2">… earlier output not shown</span>\n' : ''}${esc(text) || '<span class="dim2">No output yet</span>'}</pre>
          ${log && log.error ? `<div class="t-error small">${esc(log.error)}</div>` : ''}
          <div class="tka"><button class="chip${follow ? ' on' : ''}" data-follow>${ic('arrow-down', 's12')}${follow ? 'Following' : 'Follow'}</button><span class="grow"></span>
          ${t.running && can('sessions.taskStop') ? `<button class="btn sm danger-text" data-stop="${esc(t.id)}">${ic('circle-stop', 's14')}Stop task</button>` : ''}</div></div>`;
      }
      return `<button class="tk${isOpen ? ' sel' : ''}" data-task="${esc(t.id)}" data-agent="${isAgent ? esc(t.subagentId) : ''}" data-shows="${shows ? 1 : ''}" aria-expanded="${isOpen}">
          ${ic(icon, `s18 t-${tone}`)}<span class="tx"><span class="n${t.taskType === 'local_bash' ? ' mono' : ''}">${esc(title)}</span><span class="m">${esc(meta)}</span>${t.error ? `<span class="m t-error">${esc(t.error)}</span>` : ''}</span>
          ${isAgent ? ic('arrow-right', 's16 muted') : shows ? ic(isOpen ? 'chevron-down' : 'chevron-right', 's16 muted') : ''}</button>${detail}`;
    }).join('');
    return `<div class="sh"><h3>Background tasks</h3>${running ? `<span class="badge busy">${running} running</span>` : ''}<button class="ibtn" data-close aria-label="Close">${ic('x', 's18')}</button></div>
      <div class="group tasklist scrolly">${body || '<div class="empty"><p>No background tasks.</p></div>'}</div>
      ${!can('sessions.taskOutput') ? '<div class="muted small">This token cannot read task logs.</div>' : ''}`;
  };

  const draw = () => {
    if (!el || !el.isConnected) return;
    const prev = el.querySelector('[data-log]');
    const top = prev ? prev.scrollTop : 0;
    el.innerHTML = '<div class="grab"></div>' + rowsHtml();
    for (const b of el.querySelectorAll('[data-close]')) b.onclick = () => closeSheet();
    const pre = el.querySelector('[data-log]');
    if (pre) {
      pre.scrollTop = follow ? pre.scrollHeight : top;
      // Scrolling up pauses following; reaching the end resumes it.
      pre.addEventListener('scroll', () => {
        const atEnd = pre.scrollHeight - pre.scrollTop - pre.clientHeight < 12;
        if (atEnd !== follow) { follow = atEnd; const chip = el.querySelector('[data-follow]'); if (chip) { chip.classList.toggle('on', follow); chip.lastChild.textContent = follow ? 'Following' : 'Follow'; } }
      }, { passive: true });
    }
  };

  async function read() {
    if (!openTask || !can('sessions.taskOutput')) return;
    const task = openTask;
    try {
      const r = await act.taskOutput(id, task, log && log.task === task ? log.next : undefined);
      if (openTask !== task) return;
      if (!log || log.task !== task || r.reset || log.next == null) {
        log = { task, text: r.text || '', truncated: (r.offset || 0) > 0 };
      } else if (r.offset === log.next) {
        log.text += r.text || '';
      } else return;
      log.next = r.next_offset; log.size = r.size; log.done = !!r.done; log.running = !!r.running; log.error = '';
      if (log.text.length > MAX_LOG) {
        let cut = log.text.length - MAX_LOG;
        const nl = log.text.indexOf('\n', cut);
        if (nl > 0) cut = nl + 1;
        log.text = log.text.slice(cut); log.truncated = true;
      }
    } catch (e) { if (log && log.task === task) log.error = errText(e); else log = { task, text: '', error: errText(e) }; }
    draw();
  }
  const poll = () => {
    clearInterval(timer);
    timer = setInterval(() => {
      if (!el || !el.isConnected) { clearInterval(timer); return; }
      if (document.visibilityState !== 'visible') return;
      if (openTask && !(log && log.done && !log.running)) read(); else draw();
    }, 1000);
  };

  sheet('', {
    label: 'Background tasks',
    cls: 'tall',
    bind(root) {
      el = root;
      draw();
      el.addEventListener('click', async (e) => {
        const b = e.target.closest('button');
        if (!b) return;
        if (b.dataset.stop) {
          e.stopPropagation();
          const t = tasksOf(sessions.get(id) || s).find((x) => x.id === b.dataset.stop);
          const sure = await ask({ title: 'Stop this task?', body: `${t ? t.description || 'This task' : 'This task'} stops now. Its output so far stays readable.`, confirm: 'Stop task', danger: true });
          if (sure && (await act.stopTask(id, b.dataset.stop))) setTimeout(() => openTasks(sessions.get(id) || s, { go }), 0);
          else if (!sure) openTasks(sessions.get(id) || s, { go });
          return;
        }
        if (b.hasAttribute('data-follow')) {
          follow = !follow; draw();
          return;
        }
        if (b.dataset.task !== undefined) {
          if (b.dataset.agent) { closeSheet(); go(`#/s/${encodeURIComponent(id)}/a/${encodeURIComponent(b.dataset.agent)}`); return; }
          if (!b.dataset.shows) return;
          if (openTask === b.dataset.task) { openTask = null; log = null; draw(); return; }
          openTask = b.dataset.task; log = null; follow = true;
          draw(); read();
        }
      });
      poll();
    },
    onClose: () => clearInterval(timer),
  });
}
