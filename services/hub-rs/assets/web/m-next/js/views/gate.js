// The two states before any data: no token yet (pair this phone), and a
// server whose stop was requested (reconnect paused until Wake). Native's
// "Connecting to your workspace" empty state.
import { bus, setToken, wakeMachine } from '../bus.js';
import { brand } from '../util.js';

export function tokenGate(root) {
  root.innerHTML = `<div class="screen gate">
    <div class="gatebox">${brand}
      <h1>Connect to your workspace</h1>
      <p>Paste the remote token from Workspacer on your computer (Settings → Remote).</p>
      <input data-token type="password" autocomplete="off" autocapitalize="off" spellcheck="false" placeholder="Token" aria-label="Token">
      <button class="btn block primary" data-go>Connect</button>
    </div>
  </div>`;
  const input = root.querySelector('[data-token]');
  const go = () => { const v = input.value.trim(); if (v) setToken(v); };
  root.querySelector('[data-go]').onclick = go;
  input.onkeydown = (e) => { if (e.key === 'Enter') go(); };
}

export function machineGate(root) {
  root.innerHTML = `<div class="screen gate">
    <div class="gatebox">${brand}
      <h1>Server disconnected</h1>
      <p>Stop was requested, so automatic reconnect is paused. Let the machine finish shutting down before waking it.</p>
      <button class="btn block primary" data-wake>Wake server</button>
      <p class="muted small">Starting may take a minute.${bus.machineWakeURL ? '' : ' This hub published no wake address; Wake only reconnects.'}</p>
    </div>
  </div>`;
  const b = root.querySelector('[data-wake]');
  b.onclick = async () => {
    b.disabled = true; b.textContent = 'Waking…';
    try { await wakeMachine(); }
    catch { b.disabled = false; b.textContent = 'Wake server'; alert('Could not reach the wake endpoint. Try again.'); }
  };

}
