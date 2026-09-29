import ts from 'typescript';
import { body, endOf, maskRust } from './rustHttpSource';

/** Current executable replacements for the old named host-gate scenarios.
 * This is a removal/disablement guard, paired with actual Cargo behavior tests. */
export const requiredRustHostTests: Record<string, string[]> = {
  'tests/library.rs': [
    'global_library_remains_visible_through_a_symlinked_config_root',
    'every_library_walker_refuses_escaped_aliases_and_retains_ordinary_items',
    'library_writes_use_resolved_targets_without_replacing_in_store_aliases',
    'selected_library_directory_cannot_redirect_into_another_project_or_config_store',
    'derived_symlinks_cannot_escape_semantic_library_roots',
  ],
  'tests/stores.rs': [
    'store_writes_and_deletes_use_resolved_alias_targets',
    'store_read_write_delete_and_quarantine_respect_resolved_entry_boundaries',
    'collision_at_a_symlink_slot_never_falls_back_to_overwriting_the_first_session',
  ],
  'tests/files.rs': [
    'listing_reports_symlink_targets_as_files_or_directories',
    'shared_active_path_contract',
    'canonical_walk_accepts_exact_fixture_link_budget_and_refuses_one_more',
    'file_tree_uses_git_ignore_rules_and_bytewise_directory_first_order',
  ],
  'tests/git.rs': [
    'review_reads_and_mutations_use_the_selected_repository_and_cwd',
    'canonical_symlink_cwd_is_used_instead_of_reopening_the_requested_spelling',
    'diff_refuses_symlink_escape_and_disables_external_diff_program',
    'stage_delete_unstage_commit_and_local_push_preserve_selected_cwd',
    'malformed_requests_never_turn_into_mutations_and_empty_repo_is_distinct',
  ],
  'src/services/filewatch/mod.rs': [
    'references_atomic_replacement_and_missing_file_are_distinct_changes',
    'leases_renew_without_reference_leaks_and_expire',
    'swapped_symlink_drops_observer_without_external_metadata_event',
  ],
  'src/services/library.rs': [
    'selected_library_item_directories_match_corpus',
    'unavailable_symlink_privilege_cannot_make_library_corpus_green',
  ],
  'src/services/git.rs': ['real_command_prefix_and_diff_family_match_desktop_twin'],
};

export function checkRustHostTests(files: Map<string, string>): string[] {
  const errors: string[] = [];
  for (const [file, names] of Object.entries(requiredRustHostTests)) {
    const source = files.get(file);
    if (!source) {
      errors.push(`missing host-test source ${file}`);
      continue;
    }
    const code = maskRust(source);
    for (const name of names) {
      try {
        body(source, name); // A quoted name or duplicate cannot supply a test.
        const declaration = new RegExp(
          `(?:#\\[[^\\]]*\\]\\s*)+(?:async\\s+)?fn\\s+${name}\\s*\\(`,
        ).exec(code);
        const attrs = declaration?.[0].replace(/\s/g, '') ?? '';
        const supported = (attrs: string): boolean =>
          !attrs.includes('#[cfg_attr') &&
          [...attrs.matchAll(/#\[cfg\((.*?)\)\]/g)].every((row) =>
            ['test', 'any(unix,windows)'].includes(row[1]),
          );
        const hiddenByModule = [...code.matchAll(/\bmod\s+\w+\s*\{/g)].some((module) => {
          const open = code.indexOf('{', module.index);
          if (
            !declaration ||
            module.index! > declaration.index ||
            endOf(code, open) < declaration.index
          )
            return false;
          const prefix = code.slice(0, module.index).match(/((?:#\[[^\]]*\]\s*)+)$/)?.[0] ?? '';
          return !supported(prefix.replace(/\s/g, ''));
        });
        if (
          !/#\[(?:tokio::)?test(?:\([^\]]*\))?\]/.test(attrs) ||
          attrs.includes('#[ignore') ||
          !supported(attrs) ||
          hiddenByModule
        ) {
          errors.push(`host test is missing, ignored or platform-disabled ${file}::${name}`);
        }
      } catch {
        errors.push(`missing host test ${file}::${name}`);
      }
    }
  }
  return errors;
}

/** Structural replacement for sweepmeta's TypeScript half. Symbols distinguish
 * equally named counters in sibling describes; comments/string examples do not
 * count as declarations, observations, or host-capability use. */
export function checkSweepSources(files: Map<string, string>) {
  const options: ts.CompilerOptions = {
    noLib: true,
    noResolve: true,
    target: ts.ScriptTarget.Latest,
  };
  const host = ts.createCompilerHost(options);
  host.getSourceFile = (file, languageVersion) => {
    const source = files.get(file);
    return source === undefined
      ? undefined
      : ts.createSourceFile(file, source, languageVersion, true);
  };
  const program = ts.createProgram([...files.keys()], options, host);
  const checker = program.getTypeChecker();
  const errors: string[] = [];
  let counters = 0;
  const hostLines = new Set<string>();
  for (const source of program.getSourceFiles()) {
    const tallies = new Map<ts.Symbol, ts.Node>();
    const gates = new Map<ts.Symbol, ts.Node>();
    const observed = new Set<ts.Symbol>();
    const gateObserved = new Set<ts.Symbol>();
    const testAliases = new Set<ts.Symbol>();
    const symbol = (node: ts.Node) => checker.getSymbolAtLocation(node);
    const name = (node: ts.Node): string => {
      const declaration = symbol(node)?.declarations?.[0];
      return declaration && ts.isImportSpecifier(declaration)
        ? (declaration.propertyName ?? declaration.name).text
        : ts.isIdentifier(node)
          ? node.text
          : '';
    };
    const fail = (node: ts.Node, message: string) =>
      errors.push(
        `${source.fileName}:${source.getLineAndCharacterOfPosition(node.getStart()).line + 1}: ${message}`,
      );
    function visit(node: ts.Node, fn: (node: ts.Node) => void) {
      fn(node);
      ts.forEachChild(node, (child) => visit(child, fn));
    }
    visit(source, (node) => {
      if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.initializer) {
        const init = node.initializer;
        if (ts.isNewExpression(init) && name(init.expression) === 'SweepTally') {
          const key = symbol(node.name);
          if (key) tallies.set(key, node);
          else fail(node, 'unresolved tally declaration');
        }
        if (ts.isCallExpression(init) && name(init.expression) === 'gatedIt') {
          const key = symbol(node.name);
          if (key) testAliases.add(key);
        }
      }
      if (!ts.isCallExpression(node)) return;
      const called = name(node.expression);
      if (called === 'gatedIt') {
        const key = node.arguments[1] && symbol(node.arguments[1]);
        if (key) gates.set(key, node);
        else fail(node, 'unresolved gated counter');
      }
      if (/^itSwept/.test(called) || called === 'itRanEveryGatedTest') {
        const key = node.arguments[0] && symbol(node.arguments[0]);
        if (key) (called === 'itRanEveryGatedTest' ? gateObserved : observed).add(key);
      }
    });
    counters += tallies.size + gates.size;
    for (const [key, node] of tallies)
      if (!observed.has(key)) fail(node, 'tally has no execution floor');
    for (const [key, node] of gates)
      if (!gateObserved.has(key)) fail(node, 'host gate has no exact execution floor');
    visit(source, (node) => {
      if (
        ts.isCallExpression(node) &&
        ts.isPropertyAccessExpression(node.expression) &&
        node.expression.name.text === 'ran'
      ) {
        const key = symbol(node.expression.expression);
        if (key && tallies.has(key)) {
          let inTest = false;
          for (let parent: ts.Node | undefined = node.parent; parent; parent = parent.parent) {
            if (
              (ts.isArrowFunction(parent) || ts.isFunctionExpression(parent)) &&
              ts.isCallExpression(parent.parent)
            ) {
              const call = parent.parent.expression;
              const base = ts.isPropertyAccessExpression(call) ? call.expression : call;
              const key = symbol(base);
              if (['it', 'test'].includes(name(base)) || (key && testAliases.has(key)))
                inTest = true;
              // Fixture registration aliases such as `run = skipped ? it.skip : it`
              // still execute their callback as a test body, never the loop itself.
              const decl = key?.valueDeclaration;
              if (
                decl &&
                ts.isVariableDeclaration(decl) &&
                decl.initializer &&
                ts.isConditionalExpression(decl.initializer) &&
                /\bit\b/.test(decl.initializer.getText(source))
              )
                inTest = true;
            }
          }
          if (!inTest)
            fail(node, 'tally increments outside a test callback (registration is not execution)');
        }
      }
      if (
        ts.isCatchClause(node) &&
        node.block.statements.length === 1 &&
        ts.isReturnStatement(node.block.statements[0]) &&
        !node.block.statements[0].expression
      ) {
        fail(node, 'catch-return silently abandons a test');
      }
      if (!ts.isIdentifier(node) || !/^CAN_[A-Z0-9_]+$/.test(node.text)) return;
      hostLines.add(
        `${source.fileName}:${source.getLineAndCharacterOfPosition(node.getStart()).line}`,
      );
      let allowed = false;
      for (
        let parent: ts.Node | undefined = node;
        parent && !ts.isSourceFile(parent);
        parent = parent.parent
      ) {
        if (ts.isImportDeclaration(parent)) allowed = true;
        if (ts.isCallExpression(parent) && name(parent.expression) === 'gatedIt') allowed = true;
        if (ts.isVariableDeclaration(parent)) {
          if (ts.isIdentifier(parent.name) && /^CAN_/.test(parent.name.text)) allowed = true;
          if (parent.initializer)
            visit(parent.initializer, (child) => {
              if (ts.isPropertyAccessExpression(child) && /^needs[A-Z]/.test(child.name.text))
                allowed = true;
            });
        }
      }
      if (!allowed) fail(node, 'host capability is consumed by an uncounted gate');
    });
  }
  return { errors, counters, hostReferences: hostLines.size, files: files.size };
}
