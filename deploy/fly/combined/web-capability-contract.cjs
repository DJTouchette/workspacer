'use strict';

// Read-only evidence for the current owned Rust backend. A registered method
// alone does not establish that its engine/facade/coordinator can launch.
function validateWebCapabilityInventory(output) {
  const required = [
    'git.stage', 'git.unstage', 'git.commit', 'git.push', 'git.commitDiff',
    'git.commitNumstat', 'fs.watch', 'fs.unwatch', 'fleetWorkflows.request',
    'desktop.managerReplacement', 'desktop.filePickerList', 'desktop.readFileBytes',
    'desktop.sessionGrantReconcile', 'files.receiveUpload', 'agents.spawn',
    'plugins.manifests', 'federation.peersConfig', 'remote.tailscaleServe', 'ui.asset',
  ];
  const methods = Array.isArray(output.registeredMethods) ? output.registeredMethods : [];
  const failures = required
    .filter(method => !methods.includes(method))
    .map(method => 'missing ' + method);
  if (output.launchReady !== true) failures.push('owned launch service unavailable');
  for (const [method, result] of Object.entries(output.probes ?? {})) {
    if (result.status !== 'ok') failures.push(method + ': ' + result.status);
  }
  if (output.idle?.mode !== 'stop' || output.idle?.timeoutSeconds !== 900) {
    failures.push('idle stop policy changed');
  }
  if (!Number.isInteger(output.http?.pluginsCount) || output.http.pluginsCount < 2 ||
      !Number.isInteger(output.http?.examplesCount) || output.http.examplesCount < 2) {
    failures.push('bundled plugins unavailable');
  }
  if (!output.network?.available || !output.network?.serveActive || !output.network?.canServe) {
    failures.push('network adapter unavailable');
  }
  if (output.runtime?.hub !== 'ready' || output.runtime?.claudemon !== 'ready' ||
      output.runtime?.facade !== 'ready') {
    failures.push('runtime not ready');
  }
  return { ok: failures.length === 0, failures };
}

module.exports = { validateWebCapabilityInventory };
