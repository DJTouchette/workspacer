"""Portable MCP generator regression tests: contracts, not a Go SDK execution."""
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

SPEC=importlib.util.spec_from_file_location('mcp_catalog',Path(__file__).with_name('mcp-catalog.py'))
catalog=importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(catalog)


class CatalogTests(unittest.TestCase):
    def fixture(self):
        temporary=tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        root=Path(temporary.name)
        assets=root/catalog.ASSETS
        assets.mkdir(parents=True)
        for name in catalog.INPUTS:
            shutil.copyfile(catalog.ROOT/catalog.ASSETS/name,assets/name)
        return root

    def mutate(self,root,name,change):
        path=root/catalog.ASSETS/name
        value=json.loads(path.read_text())
        change(value)
        path.write_bytes(catalog.encoded(value))

    def test_checked_in_generated_assets_match_current_contracts(self):
        for name,content in catalog.build().items():
            self.assertEqual((catalog.ROOT/catalog.ASSETS/name).read_bytes(),content)

    def test_generation_requires_no_legacy_sources_or_go_sdk(self):
        root=self.fixture()
        self.assertFalse((root/'services/hub').exists())
        outputs=catalog.build(root)
        self.assertEqual(set(outputs),{'mcp-effective-tools.json','mcp-effective-help.json','mcp-effective-provenance.json','mcp-effective-wire.json'})
        self.assertEqual(outputs,catalog.build())

    def test_pointer_false_zero_projection_cannot_be_reclassified_as_empty_value(self):
        root=self.fixture()
        self.mutate(root,'mcp-wire-contract.json',lambda wire:wire['inputs']['spawn_agent']['rule']['fields']['skipPermissions'].update(omit='zero'))
        with self.assertRaisesRegex(catalog.ContractError,'omission disagrees|pointer zero'):
            catalog.build(root)

    def test_new_schema_field_requires_explicit_wire_review(self):
        root=self.fixture()
        def add_field(tools):
            next(tool for tool in tools['operator'] if tool['name']=='notify_when')['inputSchema']['properties']['anotherThreshold']={'type':'number'}
        self.mutate(root,'mcp-tools.json',add_field)
        with self.assertRaisesRegex(catalog.ContractError,'no reviewed wire rule'):
            catalog.build(root)

    def test_presentation_edits_cannot_change_constraints_or_silently_go_stale(self):
        for field,value,reason in [('pointer','/inputSchema/additionalProperties','only change descriptions'),('before','no-longer-in-the-capture','stale presentation'),('tool','unknown_tool','no tool')]:
            with self.subTest(field=field):
                root=self.fixture()
                self.mutate(root,'mcp-rust-presentation.json',lambda edits:edits['tools'][0].update({field:value}))
                with self.assertRaisesRegex(catalog.ContractError,reason):
                    catalog.build(root)

    def test_duplicate_tools_and_missing_help_entries_fail(self):
        root=self.fixture()
        self.mutate(root,'mcp-tools.json',lambda tools:tools['operator'].append(tools['operator'][0]))
        with self.assertRaisesRegex(catalog.ContractError,'duplicate tool'):
            catalog.build(root)
        root=self.fixture()
        self.mutate(root,'mcp-help.json',lambda help_doc:help_doc['groups']['operator'][0]['tools'].pop())
        with self.assertRaisesRegex(catalog.ContractError,'help omits'):
            catalog.build(root)

    def test_context_null_override_is_explicit_and_cannot_change_other_pointer_semantics(self):
        root=self.fixture()
        generated=json.loads(catalog.build(root)['mcp-effective-wire.json'])
        captured=catalog.read(root/catalog.ASSETS/'mcp-wire-contract.json')
        self.assertEqual(captured['inputs']['spawn_agent']['rule']['fields']['contextWindow']['omit'],'nil')
        self.assertNotIn('omit',generated['inputs']['spawn_agent']['rule']['fields']['contextWindow'])
        self.assertEqual(generated['inputs']['spawn_agent']['rule']['fields']['skipPermissions']['omit'],'nil')
        self.mutate(root,'mcp-wire-overrides.json',lambda value:value['preserveNull'][0].update(path=['skipPermissions']))
        with self.assertRaisesRegex(catalog.ContractError,'only reviewed context-window'):
            catalog.build(root)

    def test_cli_check_catches_generated_drift_and_write_repairs_it(self):
        root=self.fixture()
        command=[sys.executable,str(Path(__file__).with_name('mcp-catalog.py')),'--root',str(root)]
        def run(*args):
            return subprocess.run(command+list(args),capture_output=True,text=True)
        self.assertNotEqual(run('--check').returncode,0)
        self.assertEqual(run('--write').returncode,0)
        self.assertEqual(run('--check').returncode,0)
        (root/catalog.ASSETS/'mcp-effective-tools.json').write_text('{}\n')
        failed=run('--check')
        self.assertNotEqual(failed.returncode,0)
        self.assertIn('generated MCP asset differs',failed.stderr)
        self.assertEqual(run('--write').returncode,0)
        self.assertEqual(run('--check').returncode,0)


if __name__=='__main__':
    unittest.main()
