"""The required CI status must fail closed on missing or failed jobs."""
import os
from pathlib import Path
import subprocess
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'check-ci-results.sh'


def run(changes='success', code='true', frontend='success', general='success', core='success'):
    """Run the actual required-check script with selected matrix results."""
    env = dict(os.environ, CHANGES=changes, CODE=code, FRONTEND=frontend, GENERAL=general, CORE=core)
    return subprocess.run(['bash', str(SCRIPT)], env=env, capture_output=True, text=True, check=False)


class CiResults(unittest.TestCase):
    def test_code_changes_require_every_matrix_to_succeed(self):
        """A failed, cancelled or skipped required matrix blocks code changes."""
        self.assertEqual(run().returncode, 0)
        for job in ('frontend', 'general', 'core'):
            for state in ('failure', 'cancelled', 'skipped', ''):
                with self.subTest(job=job, state=state):
                    self.assertNotEqual(run(**{job: state}).returncode, 0)

    def test_docs_only_require_every_matrix_to_be_skipped(self):
        """Docs-only changes pass only when every code matrix is skipped."""
        values = dict(code='false', frontend='skipped', general='skipped', core='skipped')
        self.assertEqual(run(**values).returncode, 0)
        for job in ('frontend', 'general', 'core'):
            for state in ('success', 'failure', 'cancelled', ''):
                with self.subTest(job=job, state=state):
                    self.assertNotEqual(run(**(values | {job: state})).returncode, 0)

    def test_invalid_classification_and_failed_changes_never_pass(self):
        """Missing classifications and unsuccessful classification jobs fail closed."""
        for code in ('', 'True', 'unknown'):
            self.assertNotEqual(run(code=code).returncode, 0)
        for result in ('failure', 'cancelled', 'skipped', ''):
            self.assertNotEqual(run(changes=result).returncode, 0)
