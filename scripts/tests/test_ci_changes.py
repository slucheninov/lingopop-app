"""Tests for scripts/ci-changes.py, the docs-only CI classifier."""
import importlib.util
import subprocess
import sys
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "ci-changes.py"
spec = importlib.util.spec_from_file_location("ci_changes", SCRIPT)
ci_changes = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci_changes)


class NeedsBuild(unittest.TestCase):
    """needs_build and the CLI output."""

    def test_markdown_only_skips_the_build(self):
        """A Markdown-only change set needs no build."""
        self.assertFalse(ci_changes.needs_build(["README.md", "docs/RELEASING.md", "AGENTS.md", "CHANGELOG.MD"]))

    def test_any_project_file_builds(self):
        """Any non-Markdown path needs the build."""
        self.assertTrue(ci_changes.needs_build(["README.md", "app/src/main.ts"]))
        self.assertTrue(ci_changes.needs_build([".github/workflows/ci.yml"]))
        self.assertTrue(ci_changes.needs_build(["docs/icon.png"]))

    def test_install_template_builds(self):
        """docs/INSTALL.md is read by a test, so it builds."""
        self.assertTrue(ci_changes.needs_build(["docs/INSTALL.md"]))

    def test_empty_change_set_builds(self):
        """An empty change set builds (fail safe)."""
        self.assertTrue(ci_changes.needs_build([]))
        self.assertTrue(ci_changes.needs_build(["", "  "]))

    def test_cli(self):
        """The CLI prints one code= line."""
        run = lambda text: subprocess.run([sys.executable, str(SCRIPT)], input=text, capture_output=True, text=True, check=True).stdout
        self.assertEqual(run("README.md\n"), "code=false\n")
        self.assertEqual(run("Cargo.toml\nREADME.md\n"), "code=true\n")
        self.assertEqual(run(""), "code=true\n")


if __name__ == "__main__":
    unittest.main()
