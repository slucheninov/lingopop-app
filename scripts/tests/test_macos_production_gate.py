"""Tests for scripts/check-macos-production.sh against the real tree and intentional fixtures.

The guard complements the behaviour tests: it is the only thing that fails when the macOS native
wiring, a release configuration or the macOS identity drifts back to something that must not ship.
Each fixture is a temporary copy of the tree with exactly one defect.
"""
import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "scripts" / "check-macos-production.sh"
COPIED = ["crates", "app/src-tauri", ".github/workflows", "app/package.json", "Cargo.toml"]
IGNORE = shutil.ignore_patterns("target", "node_modules", "dist", "icons", ".git")


def run(root: Path) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["bash", str(SCRIPT), str(root)], capture_output=True, text=True, check=False
    )


class MacosProductionGate(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        for rel in COPIED:
            src = REPO / rel
            dst = self.root / rel
            dst.parent.mkdir(parents=True, exist_ok=True)
            if src.is_dir():
                shutil.copytree(src, dst, ignore=IGNORE)
            elif src.exists():
                shutil.copy2(src, dst)

    def edit(self, rel: str, old: str, new: str):
        path = self.root / rel
        text = path.read_text()
        self.assertIn(old, text, f"fixture anchor missing in {rel}")
        path.write_text(text.replace(old, new, 1))

    def assert_fails(self, needle: str):
        result = run(self.root)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn(needle, result.stderr)

    def test_the_real_tree_passes(self):
        result = run(REPO)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("macOS production readiness: ok", result.stdout)

    def test_the_copied_fixture_base_passes(self):
        result = run(self.root)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_plaintext_protector_wiring_fails(self):
        self.edit(
            "crates/lingopop-platform/src/native.rs",
            "protector: Arc::new(MacProtector::new()),",
            "protector: Arc::new(crate::DevPlaintextProtector::new()),",
        )
        self.assert_fails("DevPlaintextProtector")

    def test_dev_plaintext_enabled_for_a_dependency_fails(self):
        cargo = self.root / "app/src-tauri/Cargo.toml"
        cargo.write_text(
            cargo.read_text()
            + '\n[dependencies.lingopop-platform-fixture]\nfeatures = ["dev-plaintext"]\n'
        )
        self.assert_fails("dev-plaintext")

    def test_dev_plaintext_in_a_default_feature_fails(self):
        self.edit(
            "crates/lingopop-platform/Cargo.toml",
            "dev-plaintext = []",
            'dev-plaintext = []\ndefault = ["dev-plaintext"]',
        )
        self.assert_fails("dev-plaintext")

    def test_dev_plaintext_in_the_release_workflow_fails(self):
        path = self.root / ".github/workflows/release.yml"
        path.write_text(path.read_text() + "\n      - run: cargo build --features dev-plaintext\n")
        self.assert_fails("dev-plaintext")

    def test_an_ungated_production_reference_to_the_plaintext_protector_fails(self):
        path = self.root / "app/src-tauri/src/state.rs"
        path.write_text(path.read_text() + "\nfn leak() { let _ = lingopop_platform::DevPlaintextProtector::new(); }\n")
        self.assert_fails("DevPlaintextProtector")

    def test_a_dev_stub_marker_in_the_mac_adapters_fails(self):
        path = self.root / "crates/lingopop-platform/src/macos/ocr.rs"
        path.write_text(path.read_text() + '\nfn stub() -> PlatformError { PlatformError::Unsupported("OCR on macOS") }\n')
        self.assert_fails("OCR on macOS")

    def test_a_removed_mac_native_function_does_not_pass_vacuously(self):
        self.edit(
            "crates/lingopop-platform/src/native.rs",
            "pub fn native() -> Platform {\n    use crate::macos::",
            "pub fn native_renamed() -> Platform {\n    use crate::macos::",
        )
        self.assert_fails("macOS native() was not found")

    def test_a_missing_production_adapter_fails(self):
        self.edit(
            "crates/lingopop-platform/src/native.rs",
            "ocr: Arc::new(MacOcr::new()),",
            "ocr: Arc::new(crate::fake::FakeOcr::new(Err(PlatformError::OcrUnavailable))),",
        )
        self.assert_fails("FakeOcr")

    def test_the_macos_identity_and_minimum_os_are_checked(self):
        path = "app/src-tauri/tauri.macos.conf.json"
        config = json.loads((self.root / path).read_text())
        self.assertEqual(config["identifier"], "app.lingopop.tauri.macos")
        self.assertEqual(config["bundle"]["macOS"]["minimumSystemVersion"], "13.0")
        self.edit(path, "app.lingopop.tauri.macos", "com.lingopop.app")
        self.assert_fails("identifier")

    def test_a_missing_minimum_os_fails(self):
        self.edit("app/src-tauri/tauri.macos.conf.json", '"minimumSystemVersion": "13.0"', '"minimumSystemVersion": "12.0"')
        self.assert_fails("minimumSystemVersion")

    def test_a_version_key_in_the_macos_config_fails(self):
        self.edit("app/src-tauri/tauri.macos.conf.json", '"productName"', '"version": "9.9.9",\n  "productName"')
        self.assert_fails("version")

    def test_a_misplaced_minimum_os_key_fails(self):
        # The key exists, but at the top level instead of under bundle.macOS: grep would pass it.
        path = self.root / "app/src-tauri/tauri.macos.conf.json"
        config = json.loads(path.read_text())
        config["minimumSystemVersion"] = config["bundle"]["macOS"].pop("minimumSystemVersion")
        path.write_text(json.dumps(config, indent=2))
        self.assert_fails("bundle.macOS.minimumSystemVersion must be 13.0")

    def test_a_minimum_os_inside_a_string_fails(self):
        path = self.root / "app/src-tauri/tauri.macos.conf.json"
        config = json.loads(path.read_text())
        config["bundle"]["macOS"] = {"note": '"minimumSystemVersion": "13.0"'}
        path.write_text(json.dumps(config, indent=2))
        self.assert_fails("bundle.macOS.minimumSystemVersion must be 13.0")

    def test_an_invalid_macos_config_fails(self):
        (self.root / "app/src-tauri/tauri.macos.conf.json").write_text("{ not json")
        self.assert_fails("not valid JSON")

    def test_all_features_in_the_release_workflow_fails(self):
        path = self.root / ".github/workflows/release.yml"
        path.write_text(path.read_text() + "\n      - run: cargo build --release --all-features\n")
        self.assert_fails("release.yml builds with --all-features")

    def test_all_features_in_a_packaging_script_fails(self):
        script = self.root / "scripts" / "package-macos.sh"
        script.parent.mkdir(exist_ok=True)
        script.write_text("#!/usr/bin/env bash\ncargo tauri build -- --all-features\n")
        self.assert_fails("package-macos.sh builds with --all-features")

    def test_all_features_in_a_packaging_workflow_fails(self):
        # `packaging.yml` is not matched by a `*package*` name glob.
        path = self.root / ".github/workflows/packaging.yml"
        path.write_text("name: packaging\njobs:\n  b:\n    steps:\n      - run: cargo build --release --all-features\n")
        self.assert_fails("packaging.yml builds with --all-features")

    def test_all_features_in_packaging_macos_script_fails(self):
        script = self.root / "scripts" / "packaging-macos.sh"
        script.parent.mkdir(exist_ok=True)
        script.write_text("#!/usr/bin/env bash\ncargo build --release --all-features\n")
        self.assert_fails("packaging-macos.sh builds with --all-features")

    def test_all_features_in_any_workflow_or_script_that_builds_fails(self):
        # Discovery follows what the file does, whatever it is called.
        path = self.root / ".github/workflows/nightly.yml"
        path.write_text("jobs:\n  b:\n    steps:\n      - run: npm run tauri build -- -- --all-features\n")
        self.assert_fails("nightly.yml builds with --all-features")

    def test_all_features_in_a_comment_or_a_dependency_check_is_fine(self):
        script = self.root / "scripts" / "package-macos.sh"
        script.parent.mkdir(exist_ok=True)
        script.write_text("#!/usr/bin/env bash\n# never use --all-features here\ncargo build --release\n")
        # A dependency listing (not a build) in a script that does not package stays allowed.
        (self.root / "scripts" / "check-core-deps.sh").write_text("cargo tree --all-features\n")
        result = run(self.root)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_cargo_tree_all_features_in_a_file_that_also_builds_is_fine(self):
        # Detection is per command: a dependency listing next to a normal build is not a build
        # with every feature, whatever the file is called.
        for rel, text in [
            (
                ".github/workflows/deps.yml",
                "jobs:\n  b:\n    steps:\n      - run: cargo tree --all-features\n"
                "      - run: cargo build --release\n",
            ),
            (
                "scripts/package-macos.sh",
                "#!/usr/bin/env bash\ncargo tree --all-features | grep dev\ncargo build --release\n",
            ),
            (
                "scripts/check-deps.sh",
                "#!/usr/bin/env bash\ncargo tree --all-features && cargo build --release\n"
                "cargo metadata --all-features \\\n  --format-version 1\n",
            ),
        ]:
            path = self.root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        result = run(self.root)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_a_build_on_a_continued_line_or_after_a_tree_still_fails(self):
        path = self.root / ".github/workflows/nightly.yml"
        path.write_text(
            "jobs:\n  b:\n    steps:\n      - run: |\n"
            "          cargo tree --all-features\n"
            "          cargo build --release \\\n            --all-features\n"
        )
        self.assert_fails("nightly.yml builds with --all-features")
        path.write_text("jobs:\n  b:\n    steps:\n      - run: cargo tree && cargo test --all-features\n")
        self.assert_fails("nightly.yml builds with --all-features")

    def test_a_folded_or_literal_multiline_yaml_build_fails(self):
        path = self.root / ".github/workflows/nightly.yml"
        for body in [
            # `>` folds the lines into ONE command.
            "      - run: >\n          cargo build --release\n          --all-features\n",
            "      - run: >-\n          cargo build\n            --release --all-features\n",
            # `|` keeps the lines; the continuation joins them.
            "      - run: |\n          cargo build --release \\\n            --all-features\n",
            # A plain multi-line scalar folds too.
            "      - run: cargo build --release\n          --all-features\n",
        ]:
            with self.subTest(body=body):
                path.write_text("jobs:\n  b:\n    steps:\n" + body)
                self.assert_fails("nightly.yml builds with --all-features")

    def test_a_blank_line_in_a_folded_scalar_is_a_real_command_boundary(self):
        path = self.root / ".github/workflows/nightly.yml"
        for body in [
            # YAML decodes this to "cargo tree --edges normal\ncargo build --release --all-features".
            "      - run: >\n          cargo tree --edges normal\n\n          cargo build --release --all-features\n",
            # A more-indented line also keeps its newline.
            "      - run: >\n          cargo tree\n            --edges normal\n          cargo build --all-features\n",
            "      - run: >-\n          cargo tree --all-features\n\n          cargo test --all-features\n",
        ]:
            with self.subTest(body=body):
                path.write_text("jobs:\n  b:\n    steps:\n" + body)
                self.assert_fails("nightly.yml builds with --all-features")

    def test_cargo_global_options_before_the_subcommand_do_not_hide_a_build(self):
        path = self.root / ".github/workflows/nightly.yml"
        for command in [
            "cargo --color always build --release --all-features",
            "cargo --color=always test --all-features",
            "cargo +nightly -Z unstable-options build --all-features",
            "cargo --locked -C ../x clippy --all-features",
        ]:
            with self.subTest(command=command):
                path.write_text(f"jobs:\n  b:\n    steps:\n      - run: {command}\n")
                self.assert_fails("nightly.yml builds with --all-features")

    def test_a_global_option_before_cargo_tree_is_still_fine(self):
        (self.root / ".github/workflows/deps.yml").write_text(
            "jobs:\n  b:\n    steps:\n      - run: >\n          cargo --color always tree\n"
            "          --all-features\n      - run: cargo --color always build --release\n"
        )
        result = run(self.root)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_an_inline_comment_is_not_a_flag(self):
        (self.root / ".github/workflows/nightly.yml").write_text(
            "jobs:\n  b:\n    steps:\n"
            "      - run: cargo build --release # --all-features is forbidden here\n"
            "      - run: |\n          cargo test --release   # never --all-features\n"
            "          echo \"a # b\" && cargo build   # --all-features\n"
        )
        notes = self.root / "scripts" / "package-notes.sh"  # a name that makes every command count
        notes.parent.mkdir(parents=True, exist_ok=True)
        notes.write_text(
            "#!/usr/bin/env bash\ncargo build --release # --all-features is forbidden\n"
        )
        result = run(self.root)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_a_flag_after_a_quoted_hash_is_still_caught(self):
        (self.root / ".github/workflows/nightly.yml").write_text(
            "jobs:\n  b:\n    steps:\n      - run: cargo build --features \"a#b\" --all-features\n"
        )
        self.assert_fails("nightly.yml builds with --all-features")


if __name__ == "__main__":
    unittest.main()
