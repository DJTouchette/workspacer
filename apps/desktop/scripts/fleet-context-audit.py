#!/usr/bin/env python3
"""Measure a Fleet conversation without printing prompts or tool-result contents.

npm run audit:fleet-context -- --session <id>
npm run audit:fleet-context -- --file /path/to/conversation.json

Counts are UTF-8 bytes, not model tokens or billed usage. Nested exec results
cannot be reliably attributed to their underlying tools and remain under exec.
"""
import argparse
from collections import Counter, defaultdict
import json
from pathlib import Path
import urllib.parse
import urllib.request


def byte_size(value):
    text = value if isinstance(value, str) else json.dumps(value, ensure_ascii=False, separators=(',', ':'))
    return len(text.encode('utf-8'))


def audit(snapshot):
    items = snapshot.get('items')
    if not isinstance(items, list):
        raise ValueError('expected a conversation snapshot with an items array')
    calls = {i['id']: i for i in items if i.get('kind') == 'tool_use' and 'id' in i}
    groups = defaultdict(lambda: dict(calls=0, inputBytes=0, resultBytes=0, largestResultBytes=0, errors=0))
    for call in calls.values():
        row = groups[call.get('name', 'unknown')]
        row['calls'] += 1
        row['inputBytes'] += byte_size(call.get('input', {}))
    for item in items:
        if item.get('kind') != 'tool_result':
            continue
        row = groups[calls.get(item.get('tool_use_id'), {}).get('name', 'unmatched')]
        size = byte_size(item.get('content', ''))
        row['resultBytes'] += size
        row['largestResultBytes'] = max(row['largestResultBytes'], size)
        row['errors'] += bool(item.get('is_error'))
    turns = [byte_size(i.get('text', '')) for i in items if i.get('kind') == 'user_message']
    return {
        'sessionId': snapshot.get('session_id'),
        'seq': snapshot.get('seq'),
        'items': len(items),
        'kinds': dict(Counter(i.get('kind', 'unknown') for i in items)),
        'userTurnBytes': sum(turns),
        'largestUserTurnBytes': max(turns, default=0),
        'toolResultBytes': sum(r['resultBytes'] for r in groups.values()),
        'tools': [dict(name=name, **row) for name, row in sorted(groups.items(), key=lambda p: (-p[1]['resultBytes'], p[0]))],
        'note': 'UTF-8 payload bytes, not tokens, billing, or active context. Nested exec stays attributed to exec. Snapshot may be windowed; no prompt/result contents are printed.',
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument('--session', help='exact session id to read from the daemon')
    source.add_argument('--file', type=Path, help='saved conversation snapshot JSON')
    parser.add_argument('--api', default='http://127.0.0.1:7891', help='daemon API base URL')
    args = parser.parse_args()
    try:
        if args.file:
            snapshot = json.loads(args.file.read_text())
        else:
            url = args.api.rstrip('/') + '/sessions/' + urllib.parse.quote(args.session, safe='') + '/conversation'
            with urllib.request.urlopen(url, timeout=15) as response:
                snapshot = json.load(response)
        print(json.dumps(audit(snapshot), indent=2, ensure_ascii=False))
    except (OSError, ValueError) as error:
        parser.exit(1, f'Cannot audit conversation: {error}\n')


if __name__ == '__main__':
    main()
