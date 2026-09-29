#!/usr/bin/env python3
"""Build/check Rust MCP presentation from authored contracts, without Go or services."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
ASSETS = Path('services/hub-rs/assets')
INPUTS = ('mcp-tools.json', 'mcp-help.json', 'mcp-rust-presentation.json', 'mcp-wire-contract.json', 'mcp-wire-overrides.json')


class ContractError(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise ContractError(message)


def read(path):
    return json.loads(path.read_text(encoding='utf-8'))


def encoded(value):
    return (json.dumps(value, indent=2, ensure_ascii=False) + '\n').encode('utf-8')


def validate_rule(rule, schema, where):
    require(isinstance(rule, dict), 'invalid wire rule: ' + where)
    kind = rule.get('kind')
    require(kind in {'object', 'map', 'array', 'string', 'number', 'boolean', 'opaque'}, 'unknown wire kind: ' + where)
    omission = rule.get('omit', '')
    require(omission in {'', 'nil', 'zero', 'empty'}, 'unknown omission rule: ' + where)
    require(not omission or (omission == 'nil' and rule.get('pointer') is True)
            or (omission == 'zero' and not rule.get('pointer') and kind in {'string', 'number', 'boolean'})
            or (omission == 'empty' and not rule.get('pointer') and kind in {'map', 'array'}),
            'wire omission disagrees with captured Go type: ' + where)
    require(not rule.get('pointer') or omission != 'zero', 'pointer zero would be lost: ' + where)
    if schema is not None and kind != 'opaque':
        types = schema.get('type', [])
        types = [types] if isinstance(types, str) else types
        accepted = {'integer', 'number'} if kind == 'number' else {'object'} if kind == 'map' else {kind}
        require(bool(accepted.intersection(types)), 'wire/schema type mismatch: ' + where)
    if kind == 'object':
        fields = rule.get('fields', {})
        require(isinstance(fields, dict), 'invalid wire fields: ' + where)
        properties = schema.get('properties', {}) if schema else {}
        require(set(properties).issubset(fields), 'schema field has no reviewed wire rule: ' + where)
        for field, child in fields.items():
            validate_rule(child, properties.get(field), where + '.' + field)
    if kind == 'array':
        require(isinstance(rule.get('item'), dict), 'array has no item rule: ' + where)
        validate_rule(rule['item'], schema.get('items') if schema else None, where + '[]')


def validate_contracts(tools, help_doc, wire):
    require(set(tools) == {'view', 'triage', 'operator'}, 'catalog must define all three public tiers')
    all_tools = {}
    tiers = {}
    for scope, rows in tools.items():
        require(isinstance(rows, list) and rows, 'empty catalog tier: ' + scope)
        names = set()
        for tool in rows:
            require(isinstance(tool, dict), 'invalid catalog tool')
            name = tool.get('name')
            require(isinstance(name, str) and re.fullmatch(r'[A-Za-z][A-Za-z0-9_.-]*', name), 'invalid tool name')
            require(name not in names, 'duplicate tool in tier: ' + name)
            names.add(name)
            require(isinstance(tool.get('description'), str) and tool['description'], 'missing tool description: ' + name)
            schema = tool.get('inputSchema')
            require(isinstance(schema, dict) and schema.get('type') == 'object', 'tool input must be an object: ' + name)
            properties = schema.get('properties', {})
            require(isinstance(properties, dict), 'invalid properties: ' + name)
            require(set(schema.get('required', [])).issubset(properties), 'required field has no schema: ' + name)
            if name in all_tools:
                require(all_tools[name]['inputSchema'] == schema, 'tier schema drift: ' + name)
            all_tools[name] = tool
        tiers[scope] = names
        groups = help_doc.get('groups', {}).get(scope)
        require(isinstance(groups, list), 'missing help groups: ' + scope)
        grouped = set()
        group_names = set()
        for group in groups:
            require(group['name'] not in group_names, 'duplicate help group')
            group_names.add(group['name'])
            for name in group['tools']:
                require(name in names and name not in grouped, 'unknown or repeated help tool: ' + name)
                grouped.add(name)
        require(grouped == names - {'help'}, 'help omits or invents tier tools: ' + scope)
    require(len(all_tools) >= 100, 'builtin catalog population collapsed')
    require(tiers['view'] <= tiers['triage'] <= tiers['operator'], 'public tier inheritance drift')
    require(isinstance(help_doc.get('guidance'), dict), 'missing help guidance')
    require(set(wire.get('inputs', {})) == set(all_tools), 'wire policy must cover each builtin exactly')
    for name, tool in all_tools.items():
        entry = wire['inputs'][name]
        require(bool(entry.get('rule')) != bool(entry.get('passthrough')), 'ambiguous wire policy: ' + name)
        if entry.get('rule'):
            validate_rule(entry['rule'], tool['inputSchema'], name)
        else:
            require(entry.get('type') in {'@object', 'routing.PreferencesRequest'}, 'unreviewed wire passthrough: ' + name)
            require(isinstance(entry['passthrough'], str) and entry['passthrough'].strip(), 'missing passthrough rationale: ' + name)
    require(wire.get('sourceSha256'), 'missing original wire provenance')
    for path, digest in wire['sourceSha256'].items():
        require(path.startswith('services/hub/cmd/mcp/') and re.fullmatch('[0-9a-f]{64}', digest), 'invalid original source digest')


def substitute(text, before, after, where):
    require(isinstance(text, str) and isinstance(before, str) and before and before in text, 'stale presentation edit: ' + where)
    require(isinstance(after, str) and after, 'empty replacement: ' + where)
    return text.replace(before, after)


def build(root=ROOT):
    inputs = {name: read(root / ASSETS / name) for name in INPUTS}
    tools = copy.deepcopy(inputs['mcp-tools.json'])
    help_doc = copy.deepcopy(inputs['mcp-help.json'])
    validate_contracts(tools, help_doc, inputs['mcp-wire-contract.json'])
    overlay = inputs['mcp-rust-presentation.json']
    require(set(overlay) == {'tools', 'guidance'}, 'unknown presentation overlay fields')
    for edit in overlay['tools']:
        require(set(edit) == {'tool', 'pointer', 'before', 'after'}, 'invalid tool edit fields')
        pointer = edit['pointer']
        require(pointer.startswith('/') and pointer.endswith('/description'), 'presentation must only change descriptions')
        matches = 0
        for rows in tools.values():
            for tool in rows:
                if tool['name'] != edit['tool']:
                    continue
                node = tool
                parts = pointer[1:].split('/')
                for part in parts[:-1]:
                    require(isinstance(node, dict) and part in node, 'unknown description path: ' + pointer)
                    node = node[part]
                require(isinstance(node, dict), 'description parent must be an object')
                node[parts[-1]] = substitute(node.get(parts[-1]), edit['before'], edit['after'], edit['tool'] + pointer)
                matches += 1
        require(matches, 'presentation edit has no tool: ' + edit['tool'])
    for edit in overlay['guidance']:
        require(set(edit) == {'topic', 'before', 'after'}, 'invalid guidance edit fields')
        topic = edit['topic']
        help_doc['guidance'][topic] = substitute(help_doc['guidance'].get(topic), edit['before'], edit['after'], topic)
    wire = copy.deepcopy(inputs['mcp-wire-contract.json'])
    overrides = inputs['mcp-wire-overrides.json']
    require(set(overrides) == {'preserveNull'}, 'unknown Rust wire override fields')
    seen = set()
    for override in overrides['preserveNull']:
        require(set(override) == {'tool', 'path', 'reason'}, 'invalid Rust wire override')
        path = override['path']
        require(isinstance(path, list) and path and path[-1] == 'contextWindow', 'only reviewed context-window null semantics may override the Go capture')
        key = (override['tool'], tuple(path))
        require(key not in seen, 'duplicate Rust wire override')
        seen.add(key)
        require(isinstance(override['reason'], str) and override['reason'].strip(), 'wire override requires a rationale')
        rule = wire['inputs'][override['tool']]['rule']
        for field in path:
            rule = rule['fields'][field]
        require(rule.get('pointer') is True and rule.get('kind') == 'number' and rule.get('omit') == 'nil', 'wire override must target a captured nullable numeric pointer')
        rule.pop('omit')
    validate_contracts(tools, help_doc, wire)
    # Semantic hashes are portable across checkout newline policies. Original Go
    # byte hashes remain in the archived wire capture, without requiring Go files.
    provenance = {'schemaVersion': 1, 'inputs': {
        name: hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode()).hexdigest()
        for name, value in inputs.items()
    }}
    return {'mcp-effective-tools.json': encoded(tools), 'mcp-effective-help.json': encoded(help_doc),
            'mcp-effective-provenance.json': encoded(provenance), 'mcp-effective-wire.json': encoded(wire)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=ROOT)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument('--write', action='store_true', help='regenerate effective Rust runtime assets')
    mode.add_argument('--check', action='store_true', help='verify checked-in generated assets (default)')
    args = parser.parse_args()
    try:
        for name, content in build(args.root).items():
            path = args.root / ASSETS / name
            if args.write:
                path.write_bytes(content)
            else:
                require(path.is_file() and path.read_bytes().replace(b'\r\n', b'\n') == content,
                        'generated MCP asset differs: ' + str(path) + '; run python3 scripts/mcp-catalog.py --write')
    except (ContractError, KeyError, TypeError, OSError, ValueError) as error:
        parser.exit(1, 'mcp-catalog: ' + str(error) + '\n')
    print('Rust MCP catalog, help, typed wire rules and provenance are valid')


if __name__ == '__main__':
    main()
