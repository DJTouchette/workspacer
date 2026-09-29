"""A manual testing release cannot borrow CI from a different revision or run."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("nightly_preview", Path(__file__).with_name("check-nightly-preview.py"))
policy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(policy)
SHA = "a" * 40


def run(**overrides):
    return {"head_sha": SHA, "head_branch": "main", "event": "push",
            "path": ".github/workflows/ci.yml", "status": "completed",
            "conclusion": "success", "run_number": 1, "run_attempt": 1,
            "id": 1, "html_url": "https://github.com/example/repo/actions/runs/1",
            **overrides}


class PreviewPolicyTests(unittest.TestCase):
    def test_only_manual_default_branch_context_is_eligible(self):
        policy.validate_context("workflow_dispatch", "refs/heads/main", "main", SHA)
        policy.validate_context("workflow_dispatch", "refs/heads/master", "master", SHA)
        for event, ref, sha in [("schedule", "refs/heads/main", SHA),
                                ("push", "refs/heads/main", SHA),
                                ("workflow_dispatch", "refs/heads/feature", SHA),
                                ("workflow_dispatch", "refs/tags/v1", SHA),
                                ("workflow_dispatch", "refs/heads/main", "main")]:
            with self.assertRaises(ValueError):
                policy.validate_context(event, ref, "main", sha)

    def test_exact_completed_success_is_required(self):
        self.assertEqual(policy.successful_ci({"workflow_runs": [run()]}, "main", SHA), run())
        for candidate in [run(head_sha="b" * 40), run(head_branch="feature"),
                          run(event="pull_request"), run(path=".github/workflows/release.yml"),
                          run(status="in_progress", conclusion=None), run(conclusion="failure"),
                          run(conclusion="cancelled"), run(conclusion="skipped")]:
            with self.assertRaises(ValueError):
                policy.successful_ci({"workflow_runs": [candidate]}, "main", SHA)
        with self.assertRaises(ValueError):
            policy.successful_ci({"workflow_runs": []}, "main", SHA)

    def test_newer_failed_or_pending_run_cannot_borrow_an_older_success(self):
        for newer in [run(run_number=2, conclusion="failure"),
                      run(run_attempt=2, status="queued", conclusion=None)]:
            for candidates in [[run(), newer], [newer, run()]]:
                with self.assertRaises(ValueError):
                    policy.successful_ci({"workflow_runs": candidates}, "main", SHA)

    def test_unrelated_newer_run_does_not_replace_candidate_evidence(self):
        evidence = policy.successful_ci({"workflow_runs": [
            run(run_number=99, head_sha="b" * 40, conclusion="failure"), run(),
        ]}, "main", SHA)
        self.assertEqual(evidence["id"], 1)


if __name__ == "__main__":
    unittest.main()
