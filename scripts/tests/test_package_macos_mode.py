"""Tests for the signing-mode selection of scripts/package-macos.sh and the tag-run gate step of
.github/workflows/release.yml.

Run: python3 -m unittest discover -s scripts/tests -p 'test_package_macos_mode.py'
The script is run with `PACKAGE_MACOS_SELECT_ONLY=1`: it prints the selected mode and exits
before any build, keychain or codesign call, so these tests run on Linux and macOS without
Xcode or secrets. The environment is cleared except PATH and HOME. The secret values are
sentinels that must never appear in the output.
"""
import os
import re
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "scripts" / "package-macos.sh"
WORKFLOW = REPO / ".github" / "workflows" / "release.yml"

P12 = {"MACOS_SIGNING_P12": "SENTINEL-P12-VALUE", "MACOS_SIGNING_P12_PASSWORD": "SENTINEL-P12-PASSWORD"}
APPLE = {
    "APPLE_CERTIFICATE": "SENTINEL-APPLE-CERT",
    "APPLE_CERTIFICATE_PASSWORD": "SENTINEL-APPLE-CERT-PASSWORD",
    "APPLE_SIGNING_IDENTITY": "SENTINEL-APPLE-IDENTITY",
    "APPLE_ID": "SENTINEL-APPLE-ID",
    "APPLE_PASSWORD": "SENTINEL-APPLE-PASSWORD",
    "APPLE_TEAM_ID": "SENTINEL-APPLE-TEAM",
}
TAG_FAILURE = "tag releases must be signed with the pinned certificate"


def run_script(extra_env, stdin=None):
    env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": os.environ.get("HOME", "/tmp")}
    env.update(extra_env)
    return subprocess.run(
        ["bash", str(SCRIPT)], env=env, input=stdin, capture_output=True, text=True, check=False
    )


@unittest.skipIf(sys.platform == "win32", "runs package-macos.sh with bash")
class ModeSelection(unittest.TestCase):
    def select(self, **env):
        result = run_script({"PACKAGE_MACOS_SELECT_ONLY": "1", **env})
        for value in list(P12.values()) + list(APPLE.values()):
            self.assertNotIn(value, result.stdout + result.stderr)
        return result

    def assertMode(self, mode, **env):
        result = self.select(**env)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip().splitlines()[-1], f"signing mode: {mode}")

    def assertFails(self, needle, **env):
        result = self.select(**env)
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertNotIn("signing mode:", result.stdout)
        self.assertIn(needle, result.stderr)
        return result

    def test_no_secrets_is_adhoc(self):
        self.assertMode("adhoc")

    def test_both_p12_secrets_are_selfsigned(self):
        self.assertMode("selfsigned", **P12)

    def test_only_the_p12_fails_naming_the_password(self):
        result = self.assertFails("MACOS_SIGNING_P12_PASSWORD", MACOS_SIGNING_P12=P12["MACOS_SIGNING_P12"])
        self.assertNotIn("MACOS_SIGNING_P12 ", result.stderr.replace("MACOS_SIGNING_P12_PASSWORD", ""))

    def test_only_the_password_fails_naming_the_p12(self):
        result = self.assertFails(
            "MACOS_SIGNING_P12 not set", MACOS_SIGNING_P12_PASSWORD=P12["MACOS_SIGNING_P12_PASSWORD"]
        )
        self.assertNotIn("MACOS_SIGNING_P12_PASSWORD", result.stderr)

    def test_empty_values_count_as_unset(self):
        self.assertMode("adhoc", MACOS_SIGNING_P12="", MACOS_SIGNING_P12_PASSWORD="")

    def test_p12_takes_precedence_over_developer_id(self):
        self.assertMode("selfsigned", **P12, **APPLE)

    def test_developer_id_outside_a_tag_run_is_unchanged(self):
        self.assertMode("developer-id", **APPLE)

    def test_partial_developer_id_outside_a_tag_run_still_fails(self):
        env = dict(APPLE)
        del env["APPLE_TEAM_ID"]
        self.assertFails("APPLE_TEAM_ID not set", **env)

    def test_tag_run_without_p12_fails(self):
        self.assertFails(TAG_FAILURE, RELEASE_TAG_RUN="1")

    def test_tag_run_never_selects_developer_id(self):
        self.assertFails(TAG_FAILURE, RELEASE_TAG_RUN="1", **APPLE)

    def test_tag_run_with_p12_is_selfsigned(self):
        self.assertMode("selfsigned", RELEASE_TAG_RUN="1", **P12)

    def test_tag_run_with_p12_ignores_apple_variables(self):
        self.assertMode("selfsigned", RELEASE_TAG_RUN="1", **P12, **APPLE)
        partial = dict(APPLE)
        del partial["APPLE_ID"]
        self.assertMode("selfsigned", RELEASE_TAG_RUN="1", **P12, **partial)

    def test_tag_run_with_one_p12_secret_fails(self):
        self.assertFails("MACOS_SIGNING_P12_PASSWORD", RELEASE_TAG_RUN="1", MACOS_SIGNING_P12=P12["MACOS_SIGNING_P12"])

    def test_unexpected_tag_run_value_fails_closed(self):
        self.assertFails("RELEASE_TAG_RUN", RELEASE_TAG_RUN="yes", **P12)

    def test_no_tag_run_accepts_adhoc(self):
        # macosPreview false: the workflow does not set RELEASE_TAG_RUN, so ad hoc is accepted.
        self.assertMode("adhoc", RELEASE_TAG_RUN="")


@unittest.skipIf(sys.platform == "win32", "runs package-macos.sh with bash")
class IdentityParsing(unittest.TestCase):
    def parse(self, text):
        return run_script({"PACKAGE_MACOS_PARSE_IDENTITIES": "1"}, stdin=text)

    def test_an_untrusted_self_signed_identity_is_listed(self):
        # `security find-identity -p codesigning <kc>` for the untrusted self-signed identity:
        # it is a matching identity but never a valid one, so `-v` would list nothing.
        text = textwrap.dedent("""\
            Policy: Code Signing
              Matching identities
              1) FED8CA55AD74676A130508B87EA9D7051A2BC0F8 "LingoPop Release Signing" (CSSMERR_TP_NOT_TRUSTED)
                 1 identities found

              Valid identities only
                 0 valid identities found
            """)
        result = self.parse(text)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.split(), ["fed8ca55ad74676a130508b87ea9d7051a2bc0f8"])

    def test_duplicates_collapse_and_two_identities_are_two(self):
        text = (
            '  1) FED8CA55AD74676A130508B87EA9D7051A2BC0F8 "A"\n'
            '  1) FED8CA55AD74676A130508B87EA9D7051A2BC0F8 "A"\n'
            '  2) 0000000000000000000000000000000000000001 "B"\n'
        )
        self.assertEqual(
            sorted(self.parse(text).stdout.split()),
            ["0000000000000000000000000000000000000001", "fed8ca55ad74676a130508b87ea9d7051a2bc0f8"],
        )

    def test_nothing_found_prints_nothing(self):
        self.assertEqual(self.parse("     0 identities found\n").stdout, "")


class ScriptText(unittest.TestCase):
    def setUp(self):
        self.script = SCRIPT.read_text(encoding="utf-8")

    def tauri_command(self):
        joined = self.script.replace("\\\n", " ")
        lines = [line for line in joined.splitlines() if "tauri build" in line and "env " in line]
        self.assertEqual(len(lines), 1, lines)
        return lines[0]

    def test_tauri_build_runs_without_any_signing_secret(self):
        command = self.tauri_command()
        for var in ["MACOS_SIGNING_P12", "MACOS_SIGNING_P12_PASSWORD", *APPLE]:
            self.assertRegex(command, rf"-u {var}(\s|$)", var)

    def test_selfsigned_checks_the_pinned_requirement(self):
        self.assertIn("release-signing.json", self.script)
        self.assertIn('identifier \\"$bundle_id\\" and certificate leaf = H\\"$pinned_sha1\\"', self.script)
        self.assertIn("codesign -d -r-", self.script)
        self.assertIn("--timestamp=none", self.script)
        self.assertNotIn("set -x", self.script)


@unittest.skipIf(sys.platform == "win32", "runs the workflow bash block")
class TagRunGateStep(unittest.TestCase):
    STEP = "      - name: Require the pinned certificate on tag runs"

    def setUp(self):
        self.workflow = WORKFLOW.read_text(encoding="utf-8")

    def step_text(self):
        start = self.workflow.index(self.STEP)
        end = self.workflow.index("\n      - ", start + 1)
        return self.workflow[start:end]

    def test_step_runs_on_tag_runs_before_packaging(self):
        step = self.step_text()
        self.assertIn("if: github.ref_type == 'tag'", step)
        self.assertLess(self.workflow.index(self.STEP), self.workflow.index("- name: Build, sign, package and verify"))
        package = self.workflow[self.workflow.index("- name: Build, sign, package and verify"):]
        package = package[: package.index("\n      - ")]
        self.assertIn("MACOS_SIGNING_P12: ${{ secrets.MACOS_SIGNING_P12 }}", package)
        self.assertIn("MACOS_SIGNING_P12_PASSWORD: ${{ secrets.MACOS_SIGNING_P12_PASSWORD }}", package)
        self.assertNotIn("RELEASE_TAG_RUN", package.replace("# RELEASE_TAG_RUN", ""))

    def run_step(self, platforms_text):
        step = self.step_text()
        body = []
        for line in step[step.index("run: |") + len("run: |"):].split("\n")[1:]:
            if line.strip() and not line.startswith(" " * 10):
                break  # the block scalar ends at the first less indented line
            body.append(line)
        block = textwrap.dedent("\n".join(body)).strip("\n") + "\n"
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "scripts").mkdir()
            shutil.copy2(REPO / "scripts" / "verify-release-assets.py", root / "scripts")
            if platforms_text is not None:
                (root / "release-platforms.json").write_text(platforms_text, encoding="utf-8")
            github_env = root / "github_env"
            github_env.write_text("", encoding="utf-8")
            env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "GITHUB_ENV": str(github_env)}
            # The workflow calls `python`; point it at this interpreter.
            shim = root / "bin"
            shim.mkdir()
            (shim / "python").symlink_to(sys.executable)
            env["PATH"] = f"{shim}:{env['PATH']}"
            result = subprocess.run(["bash", "-c", block], cwd=root, env=env, capture_output=True, text=True)
            return result, github_env.read_text(encoding="utf-8")

    def test_mac_preview_true_sets_the_tag_run_flag(self):
        result, written = self.run_step('{"macosPreview": true}\n')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(written, "RELEASE_TAG_RUN=1\n")

    def test_mac_preview_false_leaves_adhoc_allowed(self):
        result, written = self.run_step('{"macosPreview": false}\n')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(written, "")

    def test_unreadable_release_platforms_fails_the_step(self):
        for text in (None, "not json", '{"macosPreview": "yes"}', '{"macosPreview": true, "x": 1}'):
            with self.subTest(text=text):
                result, written = self.run_step(text)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(written, "")


if __name__ == "__main__":
    unittest.main()
