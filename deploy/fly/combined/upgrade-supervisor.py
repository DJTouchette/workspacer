#!/usr/bin/env python3
"""Add hub-only power configuration to the existing isolated combined image."""
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
source = path.read_text()
anchor = "    he.update(HUB_TOKEN=host, WORKSPACER_PLUGIN_SANDBOX='enforce')\n"
assert source.count(anchor) == 1, "unrecognized combined supervisor; refusing to replace it"
addition = '''    # Power configuration is root-installed, and its credential is readable
    # only by the hub UID. Worker and agent environments still come from we.
    power = read_json(Path('/opt/combined/power.json'))
    require(power['mode'] in ('observe', 'stop', 'off'), 'invalid power mode')
    require(power['app'] == os.environ.get('FLY_APP_NAME') and power['machine'] == os.environ.get('FLY_MACHINE_ID'), 'power target identity mismatch')
    token_path = Path('/run/workspacer-power-token')
    real(token_path)
    os.chown(token_path, 10002, 10002)
    os.chmod(token_path, 0o600)
    he.update(WKS_MACHINE_POWER='fly', WKS_MACHINE_WAKE='http',
        WKS_MACHINE_WAKE_URL=power['wakeUrl'], WKS_MACHINE_IDLE_MODE=power['mode'],
        WKS_MACHINE_IDLE_TIMEOUT=power['timeout'], FLY_APP_NAME=power['app'],
        FLY_MACHINE_ID=power['machine'], FLY_API_TOKEN_FILE=str(token_path))
'''
path.write_text(source.replace(anchor, anchor + addition))
