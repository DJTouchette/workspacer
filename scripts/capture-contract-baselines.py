#!/usr/bin/env python3
"""Explicitly capture/check the six retired-Go reference corpora.

Capture never executes or claims to re-run the reference. It pins the retained
reference source and existing fixture bytes to a Git checkpoint; behavioral
replays are the named Rust tests. Existing independent TS contracts cannot use
this opt-in. --check works after deleting the retired source tree.
"""
import argparse
import hashlib
import json
import pathlib
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]
MANIFEST = ROOT / 'contracts/reference-baselines/manifest.json'
SOURCES = {
    'hub-bus-cases.json': ['services/hub/internal/bus'],
    'hub-job-cases.json': ['services/hub/internal/jobs'],
    'hub-snapshot-cases.json': ['services/hub/cmd/brain/enrich.go', 'services/hub/cmd/brain/migration_snapshots_test.go'],
    'routing-policy-cases.json': ['services/hub/internal/routing', 'services/hub/internal/limits'],
    'usage-pacing-cases.json': ['services/hub/internal/limits'],
    'fleet-quiescence-cases.json': ['services/hub/internal/quiescence'],
}

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def describe(name):
    path = ROOT / 'contracts' / name
    doc = json.loads(path.read_text())
    floors, loaders = {}, set()
    for block, spec in doc['vocabulary']['blocks'].items():
        if block == '_comment':
            continue
        rows = doc
        for part in block.split('.'):
            rows = rows[part]
        assert isinstance(rows, list) and rows, (name, block)
        floors[block] = len(rows)
        for loader in spec['loaders']:
            assert loader.startswith('services/hub-rs/'), (name, 'independent live loader cannot retire', loader)
            source, needle = loader.split('::', 1)
            assert needle and needle in (ROOT / source).read_text(), loader
            loaders.add(loader)
    return path, floors, sorted(loaders)

def capture():
    commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    entries = {}
    for name, roots in SOURCES.items():
        path, floors, loaders = describe(name)
        sources = {}
        for rel in roots:
            ref = ROOT / rel
            paths = sorted(p for p in ref.glob('*.go') if not p.name.endswith('_test.go') or p.name in ('rust_contract_test.go', 'migration_test.go')) if ref.is_dir() else [ref]
            assert paths and all(p.is_file() for p in paths), f'reference source unavailable: {rel}'
            for source in paths:
                relative = source.relative_to(ROOT).as_posix()
                committed = subprocess.check_output(['git', 'show', f'{commit}:{relative}'], cwd=ROOT)
                assert committed == source.read_bytes(), f'uncommitted reference change: {relative}'
                sources[relative] = digest(source)
        entries[name] = {
            'referenceCommit': commit,
            'captureKind': 'retained Go/Rust fixture contract at migration checkpoint; not a new reference execution',
            'referenceSources': sources,
            'fixtureSha256': digest(path),
            'caseFloors': floors,
            'rustLoaders': loaders,
        }
    MANIFEST.write_text(json.dumps({'version': 1, 'fixtures': entries}, indent=2) + '\n')

def check():
    manifest = json.loads(MANIFEST.read_text())
    assert manifest['version'] == 1 and set(manifest['fixtures']) == set(SOURCES)
    for name, entry in manifest['fixtures'].items():
        path, floors, loaders = describe(name)
        assert digest(path) == entry['fixtureSha256'], f'{name}: captured fixture changed'
        assert floors == entry['caseFloors'] and loaders == entry['rustLoaders'], name
        assert len(entry['referenceCommit']) == 40 and entry['referenceSources'], name
        for rel, expected in entry['referenceSources'].items():
            source = ROOT / rel
            assert len(expected) == 64 and not pathlib.PurePosixPath(rel).is_absolute() and '..' not in pathlib.PurePosixPath(rel).parts
            if source.exists():
                assert digest(source) == expected, f'{rel}: reference changed since capture'
    print(f'checked {len(SOURCES)} captured reference baselines')

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument('--capture', action='store_true')
    mode.add_argument('--check', action='store_true')
    args = parser.parse_args()
    if args.capture:
        capture()
    check()
