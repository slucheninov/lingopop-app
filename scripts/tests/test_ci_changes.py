"""Tests for scripts/ci-changes.py, the docs-only CI classifier."""
import importlib.util
import re
import subprocess
import tempfile
import sys
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "ci-changes.py"
WORKFLOW = Path(__file__).resolve().parents[2] / ".github" / "workflows" / "ci.yml"
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


class WorkflowDiff(unittest.TestCase):
    """The workflow's own `git diff` command feeds the classifier correctly."""

    def test_a_renamed_install_template_still_builds(self):
        """Renaming docs/INSTALL.md to another Markdown file, with rename detection on, builds."""
        line = re.search(r'files=\$\((git [^)]*diff [^)]*)\)', WORKFLOW.read_text(encoding="utf-8"))
        self.assertIsNotNone(line, "ci.yml has no `files=$(git ... diff ...)` line")
        argv = line.group(1).replace('"$base"', "HEAD^").split()
        with tempfile.TemporaryDirectory() as repo:
            git = lambda *a: subprocess.run(
                ["git", "-c", "user.name=t", "-c", "user.email=t@example.invalid", "-c", "diff.renames=true", *a],
                cwd=repo, capture_output=True, text=True, check=True,
            ).stdout
            git("init", "-q")
            (Path(repo) / "docs").mkdir()
            (Path(repo) / "docs" / "INSTALL.md").write_text("template\n", encoding="utf-8")
            git("add", "-A")
            git("commit", "-q", "-m", "a")
            git("mv", "docs/INSTALL.md", "docs/OTHER.md")
            git("commit", "-q", "-m", "b")
            files = git(*argv[1:])
            self.assertIn("docs/INSTALL.md", files)
            self.assertTrue(ci_changes.needs_build(files.splitlines()))
            # A non-ASCII Markdown path is printed verbatim, so a docs-only change still skips.
            (Path(repo) / "docs" / "über.md").write_text("doc\n", encoding="utf-8")
            git("add", "-A")
            git("commit", "-q", "-m", "c")
            files = git(*argv[1:])
            self.assertIn("docs/über.md", files)
            self.assertFalse(ci_changes.needs_build(files.splitlines()))


if __name__ == "__main__":
    unittest.main()
