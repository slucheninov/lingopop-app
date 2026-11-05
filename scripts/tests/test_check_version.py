"""Tests for scripts/check-version.py (the shared version check of CI and every release leg).

Run: python3 -m unittest discover -s scripts/tests -p 'test_check_version.py'
Every case writes a small temporary repository root; only `test_the_real_tree_agrees` reads the
real tree.
"""
import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "check-version.py"
REPO_ROOT = SCRIPT.parent.parent

spec = importlib.util.spec_from_file_location("check_version", SCRIPT)
cv = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cv)

CARGO = """[workspace]
resolver = "2"
members = ["crates/*"]

[workspace.package]
version = "{version}"
edition = "2021"

[workspace.dependencies]
serde = {{ version = "1" }}
"""


def write_json(path: Path, value) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


class CheckVersion(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.write_all("0.3.0-alpha.1")

    def write_all(self, version):
        (self.root / "Cargo.toml").write_text(CARGO.format(version=version), encoding="utf-8")
        write_json(self.root / "app/src-tauri/tauri.conf.json", {"productName": "LingoPop", "version": version})
        write_json(self.root / "app/package.json", {"name": "lingopop-app", "version": version})
        write_json(
            self.root / "app/package-lock.json",
            {"name": "lingopop-app", "version": version, "packages": {"": {"name": "lingopop-app", "version": version}}},
        )
        write_json(
            self.root / "app/src-tauri/tauri.macos.conf.json",
            {"identifier": "app.lingopop.tauri.macos", "bundle": {"macOS": {"minimumSystemVersion": "13.0"}}},
        )

    def edit_json(self, rel, mutate):
        path = self.root / rel
        data = json.loads(path.read_text(encoding="utf-8"))
        mutate(data)
        write_json(path, data)

    def assert_error(self, needle, tag=None):
        version, errors = cv.check(self.root, tag)
        self.assertIsNone(version)
        self.assertTrue(any(needle in e for e in errors), errors)
        return errors

    def test_the_real_tree_agrees(self):
        version, errors = cv.check(REPO_ROOT)
        self.assertEqual(errors, [])
        self.assertRegex(version, r"^\d+\.\d+\.\d+")

    def test_equal_versions_pass_and_print_the_version(self):
        self.assertEqual(cv.check(self.root), ("0.3.0-alpha.1", []))
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "--root", str(self.root), "--tag", "v0.3.0-alpha.1"],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "0.3.0-alpha.1\n")

    def test_cargo_mismatch(self):
        (self.root / "Cargo.toml").write_text(CARGO.format(version="0.3.0"), encoding="utf-8")
        self.assert_error("versions differ")

    def test_only_the_workspace_package_table_counts(self):
        # A `version` in another table (a dependency) must not be read as the app version.
        text = CARGO.format(version="0.3.0-alpha.1").replace(
            '[workspace.package]\nversion = "0.3.0-alpha.1"\n', "[workspace.package]\n"
        ) + '\n[package]\nversion = "0.3.0-alpha.1"\n'

        (self.root / "Cargo.toml").write_text(text, encoding="utf-8")
        self.assert_error("Cargo.toml [workspace.package] version: missing")

    def test_tauri_mismatch(self):
        self.edit_json("app/src-tauri/tauri.conf.json", lambda d: d.update(version="0.3.0-alpha.2"))
        self.assert_error("versions differ")

    def test_package_json_mismatch(self):
        self.edit_json("app/package.json", lambda d: d.update(version="0.2.9"))
        self.assert_error("versions differ")

    def test_package_lock_top_level_mismatch(self):
        self.edit_json("app/package-lock.json", lambda d: d.update(version="0.2.9"))
        self.assert_error("versions differ")

    def test_package_lock_root_package_mismatch(self):
        self.edit_json("app/package-lock.json", lambda d: d["packages"][""].update(version="0.2.9"))
        errors = self.assert_error("versions differ")
        self.assertTrue(any('packages[""].version = 0.2.9' in e for e in errors), errors)

    def test_missing_version_field(self):
        self.edit_json("app/package.json", lambda d: d.pop("version"))
        self.assert_error("app/package.json version: missing")

    def test_invalid_version_string(self):
        self.write_all("0.3")
        self.assert_error("is not a valid version")

    def test_macos_override_with_a_top_level_version_fails(self):
        self.edit_json("app/src-tauri/tauri.macos.conf.json", lambda d: d.update(version="0.3.0-alpha.1"))
        self.assert_error("tauri.macos.conf.json: must not contain a version key")

    def test_macos_override_with_a_nested_version_fails(self):
        self.edit_json("app/src-tauri/tauri.macos.conf.json", lambda d: d["bundle"].update(version="1"))
        self.assert_error("must not contain a version key")

    def test_missing_macos_override_fails(self):
        (self.root / "app/src-tauri/tauri.macos.conf.json").unlink()
        self.assert_error("tauri.macos.conf.json: missing")

    def test_invalid_json_fails(self):
        (self.root / "app/src-tauri/tauri.conf.json").write_text("{", encoding="utf-8")
        self.assert_error("tauri.conf.json: invalid JSON")

    def test_matching_tag_passes(self):
        self.assertEqual(cv.check(self.root, "v0.3.0-alpha.1"), ("0.3.0-alpha.1", []))

    def test_mismatched_tag_fails(self):
        self.assert_error("does not match the app version 0.3.0-alpha.1", tag="v0.3.0-alpha.2")

    def test_tag_without_v_fails(self):
        self.assert_error("must start with v", tag="0.3.0-alpha.1")

    def test_failure_exit_code_and_stderr(self):
        self.edit_json("app/package.json", lambda d: d.update(version="0.2.9"))
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "--root", str(self.root)], capture_output=True, text=True, check=False
        )
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, "")
        self.assertIn("error: versions differ", result.stderr)


if __name__ == "__main__":
    unittest.main()
