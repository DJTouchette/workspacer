#!/usr/bin/env python3
"""Publish an uploaded nightly draft, then verify the actual release and tag."""
import argparse
import json
from pathlib import Path
import re
import subprocess
import tempfile


def api(repo, suffix, *flags):
    result = subprocess.run(['gh', 'api', *flags, f'repos/{repo}/{suffix}'],
                            check=True, capture_output=True, text=True)
    return json.loads(result.stdout)


def verify_release(release, release_id, sha, expected, *, draft):
    if release.get('id') != release_id or release.get('draft') is not draft:
        raise ValueError('release identity or draft state does not match expected publication state')
    if release.get('tag_name') != 'nightly' or release.get('target_commitish') != sha:
        raise ValueError('nightly release tag/target does not match the built commit')
    if release.get('prerelease') is not True:
        raise ValueError('nightly must remain a prerelease')
    assets = release.get('assets')
    if not isinstance(assets, list):
        raise ValueError('release asset inventory is missing')
    names = [a.get('name') for a in assets]
    if len(names) != len(set(names)) or set(names) != set(expected):
        raise ValueError('release asset names differ from the uploaded build')
    for asset in assets:
        if asset.get('size') != expected[asset['name']] or asset.get('state') != 'uploaded':
            raise ValueError(f"release asset incomplete or wrong size: {asset['name']}")
        if not draft and not asset.get('browser_download_url'):
            raise ValueError(f"published asset has no download URL: {asset['name']}")


def finalize(repo, release_id, sha, paths):
    if not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', repo):
        raise ValueError('invalid repository name')
    if release_id <= 0 or not re.fullmatch(r'[0-9a-f]{40}', sha):
        raise ValueError('positive release ID and exact commit SHA required')
    expected = {}
    for value in paths:
        path = Path(value)
        if not path.is_file() or path.name in expected or path.stat().st_size <= 0:
            raise ValueError('each expected asset must be a unique nonempty file')
        expected[path.name] = path.stat().st_size
    if not expected:
        raise ValueError('expected asset list must not be empty')
    suffix = f'releases/{release_id}'
    verify_release(api(repo, suffix), release_id, sha, expected, draft=True)
    # Typed JSON avoids form-field coercion and keeps the request reviewable.
    # Close before gh opens it so this also works on Windows.
    with tempfile.TemporaryDirectory(prefix='wks-nightly-publish-') as directory:
        request = Path(directory) / 'publish.json'
        request.write_text(json.dumps({'draft': False}), encoding='utf8')
        api(repo, suffix, '-X', 'PATCH', '--input', str(request))
    # PATCH exit0/response is not publication evidence. Read server state again.
    verify_release(api(repo, suffix), release_id, sha, expected, draft=False)
    ref = api(repo, 'git/ref/tags/nightly').get('object', {})
    for _ in range(4):
        if ref.get('type') == 'commit':
            break
        if ref.get('type') != 'tag' or not re.fullmatch(r'[0-9a-f]{40}', ref.get('sha', '')):
            raise ValueError('nightly tag has an invalid target')
        ref = api(repo, f"git/tags/{ref['sha']}").get('object', {})
    if ref.get('type') != 'commit' or ref.get('sha') != sha:
        raise ValueError('published nightly tag does not resolve to the built commit')
    print(f'Published nightly release {release_id}: draft=false, commit={sha}, assets={len(expected)} verified')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', required=True)
    parser.add_argument('--release-id', required=True, type=int)
    parser.add_argument('--sha', required=True)
    parser.add_argument('assets', nargs='+')
    args = parser.parse_args()
    try:
        finalize(args.repo, args.release_id, args.sha, args.assets)
    except (ValueError, OSError, subprocess.CalledProcessError, json.JSONDecodeError) as error:
        parser.exit(1, f'nightly publication verification failed: {error}\n')


if __name__ == '__main__':
    main()
