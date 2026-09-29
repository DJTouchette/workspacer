import { hasRustBearing } from './compositionSource';
import ts from 'typescript';
import { body, endOf, maskRust } from './rustHttpSource';
import { desktopRegistrations, registrations } from './capabilitySource';

function verifyGitExecutionOrder(handler: string): void {
  const code = maskRust(handler);
  const topLevel = (at: number): boolean => {
    let depth = 0;
    for (const c of code.slice(0, at)) {
      if ('([{'.includes(c)) depth++;
      else if (')]}'.includes(c)) depth--;
    }
    return depth === 0;
  };
  const guards = [
    ...code.matchAll(
      /\blet\s+cwd\s*=\s*paths\s*::\s*canonicalize\s*\(\s*Path\s*::\s*new\s*\(\s*requested\s*\)\s*\)\s*\?\s*;/g,
    ),
  ];
  // Validation may match the method before canonicalization. Identify the
  // execution dispatch by its real run() calls, not the first lexical match.
  const dispatches = [...code.matchAll(/\bmatch\s+method\s*\{/g)].filter((match) => {
    if (!topLevel(match.index!)) return false;
    const open = code.indexOf('{', match.index);
    return /\brun\s*\(/.test(code.slice(open + 1, endOf(code, open) - 1));
  });
  const commands = [...code.matchAll(/\b(?:root|run)\s*\(/g)];
  if (
    guards.length !== 1 ||
    !topLevel(guards[0].index!) ||
    dispatches.length !== 1 ||
    commands.length === 0 ||
    guards[0].index! > dispatches[0].index! ||
    commands.some((call) => call.index! < guards[0].index!)
  ) {
    throw Error('git canonicalization moved behind execution dispatch');
  }
  if (!hasRustBearing(handler, 'letroot=root(&cwd).await?;')) {
    throw Error('git execution root does not use canonical cwd');
  }
}
/** Actual implementation bearings. The legacy symbol is an explicit mapping
 * key, not a claim that a same-named symbol still exists after the rewrite. */
export function guardVerifier(
  root: string,
  files: Map<string, string>,
): (method: string, guard: string) => void {
  const rows = registrations(files),
    desktop = desktopRegistrations(root, files);
  const source = (file: string): string => {
    const found = files.get('services/hub-rs/src/' + file);
    if (!found) throw Error('missing guard source ' + file);
    return found;
  };
  const has = (text: string, edge: string): void => {
    if (!hasRustBearing(text, edge)) throw Error('missing guard edge ' + edge);
  };
  const registered = (method: string, file: string, edge: string): void => {
    const found = rows.filter(
      (row) => row.method === method && row.file === 'services/hub-rs/src/' + file,
    );
    if (found.length !== 1) throw Error('missing guarded registration ' + method);
    has(found[0].handler, edge);
  };
  const implementation = (file: string, owner: string): string => {
    const text = source(file),
      code = maskRust(text),
      start = code.indexOf('impl ' + owner + ' {');
    if (start < 0) throw Error('missing implementation ' + owner);
    const open = code.indexOf('{', start);
    return text.slice(open + 1, endOf(code, open) - 1);
  };
  return (method, guard) => {
    if (
      ['assertPathAllowed', 'guardGitCwd', 'guardLibraryCwd', 'guardReplaySession'].includes(guard)
    ) {
      const found = desktop.filter((row) => row.method === method);
      if (found.length !== 1) throw Error('missing desktop guard registration ' + method);
      // By-argument bearing: require the actual capability literal, not a call
      // sitting somewhere else in the same source or another dispatcher branch.
      const calls: ts.CallExpression[] = [];
      const visit = (node: ts.Node): void => {
        if (
          ts.isCallExpression(node) &&
          node.expression.getText(found[0].source) === guard &&
          node.arguments[0] &&
          ts.isStringLiteral(node.arguments[0]) &&
          node.arguments[0].text === method
        )
          calls.push(node);
        ts.forEachChild(node, visit);
      };
      visit(found[0].node.arguments[1]);
      if (calls.length !== 1)
        throw Error('missing method-bound desktop guard ' + method + ' ' + guard);
      return;
    }
    switch (guard) {
      case 'canonicalize': {
        registered(method, 'services/git.rs', 'call(method,params).await');
        const handler = body(source('services/git.rs'), 'call');
        has(handler, 'letcwd=paths::canonicalize(Path::new(requested))?;');
        verifyGitExecutionOrder(handler);
        return;
      }
      case 'jobsTrusted':
        registered(method, 'services/jobs.rs', 'service.call(&caller,method,params)');
        has(
          body(source('services/jobs.rs'), 'call'),
          'if!caller.authenticated_host||!caller.trusted||caller.scope!="operator"{bail!',
        );
        return;
      case 'routingPreferencesTrusted':
        registered(
          method,
          'services/routing/sampler.rs',
          'service.preferences(&caller,method,params)',
        );
        has(
          body(source('services/routing.rs'), 'preferences'),
          'preferences::handle(self,caller,method,params)',
        );
        has(
          body(source('services/routing/preferences.rs'), 'handle'),
          'letauthorized=caller.authenticated_host&&caller.trusted&&caller.scope=="operator";ifmethod!="routing.preferences.get"&&!authorized{bail!',
        );
        return;
      case 'nodesTrusted':
        registered(method, 'services/nodes/mod.rs', 'trusted(method,&caller)?;');
        has(
          body(source('services/nodes/mod.rs'), 'trusted'),
          'if!caller.trusted||!caller.plugin_id.is_empty(){bail!',
        );
        return;
      case 'usagePrefsTrusted':
        registered(method, 'services/mod.rs', 'service.set(&caller,params)');
        has(body(source('services/usage_prefs.rs'), 'set'), 'if!caller.trusted{bail!');
        return;
      case 'machinePowerTrusted':
        registered(
          method,
          'services/quiescence/sampler.rs',
          'watcher.machine.manual_stop(&caller).await',
        );
        has(body(source('services/machine_power.rs'), 'manual_stop'), 'if!trusted(caller){bail!');
        has(
          body(source('services/machine_power.rs'), 'trusted'),
          'caller.trusted&&caller.scope=="operator"',
        );
        return;
      case 'remotePairingTrusted':
      case 'networkTrusted': {
        const pairing = guard === 'remotePairingTrusted',
          name = pairing ? 'pairings' : 'network';
        registered(method, 'services/remote_admin.rs', `${name}.call(&caller,method,&params)`);
        has(
          body(
            implementation('services/remote_admin.rs', pairing ? 'Pairings' : 'Network'),
            'call',
          ),
          `guard(caller,method,${pairing})?;`,
        );
        has(body(source('services/remote_admin.rs'), 'guard'), 'if!trusted(caller){bail!');
        has(
          body(source('services/remote_admin.rs'), 'trusted'),
          'caller.authenticated_host&&caller.trusted&&caller.scope=="operator"',
        );
        return;
      }
      case 'peerConfigTrusted':
        registered(method, 'federation/config.rs', 'owner(&caller)?;');
        has(body(source('federation/config.rs'), 'owner'), 'caller.authenticated_host');
        has(
          body(source('federation/config.rs'), 'owner'),
          'if!caller.authenticated_host||!caller.trusted||caller.scope!="operator"{bail!',
        );
        return;
      case 'authenticatedDesktopUser': {
        has(
          body(source('auth.rs'), 'may_call'),
          'ifmethod.starts_with("desktop."){returnself.authenticated_host()&&desktop_service(method);}',
        );
        has(body(source('auth.rs'), 'desktop_service'), '["ownerMethods"]');
        has(
          body(implementation('runtime.rs', 'Core'), 'call'),
          '!self.peers[&caller].identity.may_call(bare)',
        );
        return;
      }
      case 'authenticatedUploadReceiver':
        has(
          body(source('auth.rs'), 'may_call'),
          'ifmethod=="files.receiveUpload"{returnself.authenticated_host();}',
        );
        has(
          body(implementation('runtime.rs', 'Core'), 'call'),
          '!self.peers[&caller].identity.may_call(bare)',
        );
        return;
      case 'asset-containment':
        registered(method, 'services/ui_assets.rs', 'service.call(method,params)');
        has(
          body(source('services/ui_assets.rs'), 'call'),
          'letpath=paths::selected_path(root,name)?;letbytes=bounded_bytes(&path,limit)?;',
        );
        return;
      case 'authorizeLaunchPreparation':
        // Private Go brain callback retired; actual actor permit must be checked
        // around plugin preparation, rather than minting a public replacement.
        has(
          body(source('plugins/launch.rs'), 'prepare_selected'),
          'self.host.check(permit,false).await?;',
        );
        has(
          body(source('plugins/launch.rs'), 'prepare_selected'),
          'self.host.check(permit,true).await?;',
        );
        has(
          source('plugins/launch.rs'),
          'Box::pin(self.0.check_launch_preparation(permit,finish))',
        );
        has(body(implementation('runtime.rs', 'Core'), 'check_launch'), 'peer.closed.borrow()');
        return;
      default:
        throw Error('unreviewed guard witness ' + guard + ' for ' + method);
    }
  };
}
