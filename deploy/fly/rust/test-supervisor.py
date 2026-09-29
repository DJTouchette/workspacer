#!/usr/bin/env python3
"""Offline transformation contract; never imports/runs a supervisor or reads state."""
import ast
import importlib.util
from pathlib import Path
import unittest
ROOT=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('upgrade',ROOT/'upgrade-supervisor.py')
upgrade=importlib.util.module_from_spec(spec);spec.loader.exec_module(upgrade)
class Contract(unittest.TestCase):
    def test_roles_preserve_privileged_guards_and_never_spawn_legacy_backend(self):
        source=(ROOT/'fixtures/isolated-supervisor.py').read_text()
        result=upgrade.transform(source)
        self.assertEqual(upgrade.transform(result),result)
        tree=ast.parse(result)
        self.assertFalse(any(upgrade.executable(n) in ('hub','brain','mcp','claudemon') for n in ast.walk(tree)))
        self.assertIn("dict(we, HUB_TOKEN=provider['token'], WKS_MCP_HUB_TOKEN=mcp['token'])",result)
        self.assertIn("hub = spawn(hub_args, he, 10002",result)
        self.assertIn("mcp.get('facadeAuthority') is True",result)
        for name in ('env_for','spawn','ready','stop'):
            before=next(n for n in ast.parse(source).body if isinstance(n,ast.FunctionDef) and n.name==name)
            after=next(n for n in tree.body if isinstance(n,ast.FunctionDef) and n.name==name)
            self.assertEqual(ast.dump(before,include_attributes=False),ast.dump(after,include_attributes=False))
        for guard in ("hashlib.sha256(raw).hexdigest() == active['manifestSha256']", "status['Self']['ID'] == approval['expectedTailscaleNodeId']", "os.chown(token_path, 10002, 10002)","--uploads-to-worker","--no-jobs"):
            self.assertIn(guard,result)
    def test_unknown_uid_or_intervening_policy_refuses(self):
        source=(ROOT/'fixtures/isolated-supervisor.py').read_text()
        for changed in (source.replace("], he, 10002, home", "], he, 10001, home"),source.replace("    CHILDREN.remove(init)\n","    CHILDREN.remove(init)\n    require(False, 'new policy')\n")):
            with self.assertRaises(ValueError):upgrade.transform(changed)
if __name__=='__main__':unittest.main()
