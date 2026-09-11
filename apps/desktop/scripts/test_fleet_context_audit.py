import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('audit', Path(__file__).with_name('fleet-context-audit.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class AuditTest(unittest.TestCase):
    def test_correlates_by_id_and_counts_bytes_without_contents(self):
        report = module.audit({'items': [
            {'kind': 'tool_use', 'id': 'a', 'name': 'list_manager_requests', 'input': {}},
            {'kind': 'tool_result', 'tool_use_id': 'a', 'content': 'private é', 'is_error': True},
            {'kind': 'tool_result', 'tool_use_id': 'outside-window', 'content': 'x'},
            {'kind': 'user_message', 'text': 'secret'},
        ]})
        self.assertEqual(report['toolResultBytes'], 11)
        self.assertEqual(report['tools'][0]['errors'], 1)
        self.assertEqual(report['tools'][0]['largestResultBytes'], 10)
        self.assertEqual(report['tools'][1]['name'], 'unmatched')
        self.assertNotIn('private', str(report))
        self.assertNotIn('secret', str(report))

    def test_empty_and_invalid(self):
        self.assertEqual(module.audit({'items': []})['largestUserTurnBytes'], 0)
        with self.assertRaises(ValueError):
            module.audit({})


if __name__ == '__main__':
    unittest.main()
