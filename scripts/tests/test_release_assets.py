"""Tests for scripts/verify-release-assets.py (the release asset contract and the publish gate).

Run: python3 -m unittest discover -s scripts/tests -p 'test_release_assets.py'
Every fixture is synthetic (a minimal PE header, a minimal thin Mach-O with LC_BUILD_VERSION and
LC_CODE_SIGNATURE, a zip with an Info.plist, a file ending in a UDIF `koly` trailer), so the tests
run on every OS without Xcode, a dmg mount or the network. The publish command is a double that
counts calls: it must never run when anything is wrong.
"""
import importlib.util
import io
import json
import plistlib
import re
import struct
import subprocess
import sys
import tempfile
import textwrap
import unittest
import zipfile
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "verify-release-assets.py"
REPO_ROOT = SCRIPT.parent.parent

spec = importlib.util.spec_from_file_location("verify_release_assets", SCRIPT)
vra = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vra)

VERSION = "0.3.0-alpha.1"
ALL = {"windows", "macos"}
APP = "LingoPop.app"
INSTALL = """# Installing LingoPop

Intro shown only in the docs.

<!-- release-notes-start -->
## Installation

<!-- if windows -->
Windows text.
<!-- endif -->

<!-- if macos -->
macOS text.

<!-- if macos-unnotarized -->
Gatekeeper text.
<!-- endif -->
<!-- if macos-adhoc -->
Re-grant text.
<!-- endif -->
<!-- endif -->
"""
CHANGELOG = f"""# Changelog

## Unreleased

- later

## [{VERSION}] - 2026-11-05

### Added

- macOS builds.

## [0.2.0] - 2026-11-01

- older
"""


def pe_bytes(machine=0x8664):
    data = bytearray(0x200)
    data[0:2] = b"MZ"
    struct.pack_into("<I", data, 0x3C, 0x80)
    data[0x80:0x84] = b"PE\0\0"
    struct.pack_into("<H", data, 0x84, machine)
    return bytes(data)


def macho_bytes(arch="arm64", minos=(13, 0), signed=True, fat=False):
    cputype = vra.CPU_TYPES[arch]
    if fat:
        header = struct.pack(">II", vra.FAT_MAGIC, 1) + struct.pack(">IIIII", cputype, 0, 4096, 64, 12)
        return header + b"\0" * 4096
    commands = struct.pack("<IIIIII", vra.LC_BUILD_VERSION, 24, vra.PLATFORM_MACOS, (minos[0] << 16) | (minos[1] << 8), 27 << 16, 0)
    ncmds = 1
    if signed:
        commands += struct.pack("<IIII", vra.LC_CODE_SIGNATURE, 16, 4096, 128)
        ncmds += 1
    header = struct.pack("<IiiIIIII", vra.MH_MAGIC_64, cputype, 0, 2, ncmds, len(commands), 0, 0)
    return header + commands + b"\0" * 256


def app_zip(arch="arm64", version=VERSION, bundle_id=vra.BUNDLE_ID, minimum="13.0", minos=(13, 0),
            signed=True, executable_mode=0o755, ui_element=True, extra_top=None, fat=False):
    plist = {
        "CFBundleIdentifier": bundle_id,
        "CFBundleShortVersionString": version,
        "CFBundleVersion": version,
        "CFBundleExecutable": "lingopop",
        "LSMinimumSystemVersion": minimum,
    }
    if ui_element:
        plist["LSUIElement"] = True
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr(f"{APP}/Contents/Info.plist", plistlib.dumps(plist))
        exe = zipfile.ZipInfo(f"{APP}/Contents/MacOS/lingopop")
        exe.external_attr = (0o100000 | executable_mode) << 16
        archive.writestr(exe, macho_bytes(arch, minos, signed=signed, fat=fat))
        if signed:
            archive.writestr(f"{APP}/Contents/_CodeSignature/CodeResources", b"<plist/>")
        if extra_top:
            archive.writestr(extra_top, b"x")
    return buffer.getvalue()


def dmg_bytes():
    return b"\0" * 4096 + b"koly" + b"\0" * 508


def write_manifest(directory: Path, names, manifest=vra.MANIFEST):
    lines = "".join(vra.manifest_line(directory / n) for n in names)
    (directory / manifest).write_text(lines, encoding="utf-8", newline="\n")


def write_assets(directory: Path, slugs=("windows-x64", "macos-arm64", "macos-x64"), manifest=vra.MANIFEST, **overrides):
    """A complete, valid asset set; `overrides` maps a file name to replacement bytes."""
    directory.mkdir(parents=True, exist_ok=True)
    names = []
    for slug in slugs:
        for name in vra.asset_names(VERSION, slug):
            if name.endswith(".exe"):
                data = pe_bytes()
            elif name.endswith(".dmg"):
                data = dmg_bytes()
            else:
                data = app_zip(arch=vra.MACHO_ARCH[slug])
            (directory / name).write_bytes(overrides.get(name, data))
            names.append(name)
    write_manifest(directory, names, manifest)
    return names


def write_artifacts(root: Path, slugs=("windows-x64", "macos-arm64", "macos-x64"), mode="adhoc"):
    for slug in slugs:
        leg = root / vra.artifact_name(VERSION, slug)
        write_assets(leg, (slug,), manifest=f"SHA256SUMS-{slug}.txt")
        if slug.startswith("macos-"):
            (leg / f"signing-{slug}.txt").write_text(mode + "\n", encoding="utf-8", newline="\n")


def minisig_text(name, comment=None):
    """A minisign-shaped signature file; the bytes are not a real signature (the cryptographic
    check is the Rust tool's job, in the workflow), only its trusted-comment line matters here."""
    comment = f"lingopop-update {name}" if comment is None else comment
    return (
        "untrusted comment: signature from minisign secret key\n"
        "RUQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n"
        f"trusted comment: {comment}\n"
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==\n"
    )


def fake_sign(paths):
    for path in paths:
        path = Path(path)
        Path(f"{path}.minisig").write_text(minisig_text(path.name), encoding="utf-8", newline="\n")


def write_signatures(directory: Path, slugs=("windows-x64", "macos-arm64", "macos-x64")):
    """Signs the updatable assets of an unsigned set and rewrites the manifest for the signed set."""
    for slug in slugs:
        fake_sign(directory / n for n in vra.updatable_names(VERSION, slug))
    write_manifest(directory, [n for s in slugs for n in vra.signed_asset_names(VERSION, s)])


def signed_order(slugs=vra.PLATFORMS):
    return [n for s in slugs for n in vra.signed_asset_names(VERSION, s)]


class PublishDouble:
    def __init__(self):
        self.calls = []

    def __call__(self, argv, check):
        self.assertion = check
        self.calls.append(list(argv))


class Base(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.tmp = Path(tmp.name)
        self.dir = self.tmp / "assets"
        (self.tmp / "CHANGELOG.md").write_text(CHANGELOG, encoding="utf-8", newline="\n")
        (self.tmp / "INSTALL.md").write_text(INSTALL, encoding="utf-8", newline="\n")
        self.platforms_file = self.tmp / "release-platforms.json"
        self.platforms_file.write_text('{"macosPreview": true}\n', encoding="utf-8", newline="\n")
        self.double = PublishDouble()

    def prepare(self, artifacts=None, require_mac_signing=None):
        return vra.prepare(
            artifacts or self.tmp / "artifacts",
            VERSION,
            self.platforms_file,
            self.tmp / "release",
            self.tmp / "CHANGELOG.md",
            self.tmp / "INSTALL.md",
            self.tmp / "notes.md",
            require_mac_signing=require_mac_signing,
        )

    def finalize(self):
        return vra.finalize(
            self.tmp / "release",
            VERSION,
            self.platforms_file,
            ["gh", "release", "create", f"v{VERSION}"],
            runner=self.double,
        )

    def publish(self, artifacts=None, signer=fake_sign, require_mac_signing=None):
        """The publish job's order: prepare (assemble + pre-signing check + notes), sign the
        updatable assets, finalize (manifest with the signatures + full check + command)."""
        errors = self.prepare(artifacts, require_mac_signing)
        if errors:
            return errors
        config = vra.read_release_platforms(self.platforms_file)
        names = vra.updatable_assets(VERSION, vra.platforms_for(config))
        signer([self.tmp / "release" / n for n in names])
        return self.finalize()

    @property
    def publish_calls(self):
        return len(self.double.calls)


class Anchors(Base):
    def test_missing_mac_asset_blocks_publication(self):
        write_assets(self.dir)
        (self.dir / f"LingoPop-{VERSION}-macos-x64.dmg").unlink()
        errors = vra.verify_assets(self.dir, VERSION, ALL)
        self.assertNotEqual(errors, [])
        self.assertIn("macos-x64.dmg: missing", errors)

        write_artifacts(self.tmp / "artifacts")
        (self.tmp / "artifacts" / vra.artifact_name(VERSION, "macos-x64") / f"LingoPop-{VERSION}-macos-x64.dmg").unlink()
        self.assertNotEqual(self.publish(), [])
        self.assertEqual(self.publish_calls, 0)

    def test_wrong_arch_asset_blocks_publication(self):
        bad = {f"LingoPop-{VERSION}-macos-x64.zip": app_zip(arch="arm64")}
        write_assets(self.dir, **bad)
        errors = vra.verify_assets(self.dir, VERSION, ALL)
        self.assertIn("macos-x64.zip: executable architecture is not x86_64", errors)

        write_artifacts(self.tmp / "artifacts")
        leg = self.tmp / "artifacts" / vra.artifact_name(VERSION, "macos-x64")
        write_assets(leg, ("macos-x64",), manifest="SHA256SUMS-macos-x64.txt", **bad)
        self.assertIn("macos-x64.zip: executable architecture is not x86_64", self.publish())
        self.assertEqual(self.publish_calls, 0)

    def test_checksum_mismatch_blocks_publication(self):
        write_assets(self.dir)
        manifest = self.dir / vra.MANIFEST
        text = manifest.read_text(encoding="utf-8")
        flipped = ("1" if text[0] != "1" else "2") + text[1:]
        manifest.write_text(flipped, encoding="utf-8", newline="\n")
        errors = vra.verify_assets(self.dir, VERSION, ALL)
        self.assertTrue(any("checksum mismatch" in e for e in errors), errors)

        write_artifacts(self.tmp / "artifacts")
        part = self.tmp / "artifacts" / vra.artifact_name(VERSION, "macos-arm64") / "SHA256SUMS-macos-arm64.txt"
        text = part.read_text(encoding="utf-8")
        part.write_text(("1" if text[0] != "1" else "2") + text[1:], encoding="utf-8", newline="\n")
        self.assertTrue(any("checksum mismatch" in e for e in self.publish()))
        self.assertEqual(self.publish_calls, 0)

    def test_malformed_release_platforms_fails_closed(self):
        write_artifacts(self.tmp / "artifacts")
        wrong_type = self.tmp / "wrong_type.json"
        wrong_type.write_text('{"macosPreview": "yes"}', encoding="utf-8", newline="\n")
        missing_file = self.tmp / "missing.json"
        invalid_json = self.tmp / "invalid.json"
        invalid_json.write_text('{"macosPreview": true', encoding="utf-8", newline="\n")
        extra_key = self.tmp / "extra.json"
        extra_key.write_text('{"macosPreview": true, "assets": []}', encoding="utf-8", newline="\n")
        for variant in (wrong_type, missing_file, invalid_json, extra_key):
            self.assertIsNone(vra.read_release_platforms(variant))
            self.platforms_file = variant
            self.assertNotEqual(self.publish(), [])
            self.assertEqual(self.publish_calls, 0)

    def test_mac_preview_false_manifest_and_platform_set(self):
        self.assertEqual(vra.platforms_for({"macosPreview": False}), {"windows"})
        self.assertEqual(vra.platforms_for({"macosPreview": True}), {"windows", "macos"})

        write_artifacts(self.tmp / "artifacts")
        self.platforms_file.write_text('{"macosPreview": false}\n', encoding="utf-8", newline="\n")
        seen = {}
        real_verify = vra.verify_assets

        def spy(directory, version, platforms, manifest=vra.MANIFEST, signed=False):
            seen.setdefault("platforms", []).append((set(platforms), signed))
            return real_verify(directory, version, platforms, manifest, signed)

        vra.verify_assets = spy
        try:
            self.assertEqual(self.publish(), [])
        finally:
            vra.verify_assets = real_verify
        # The pre-signing check, then the check of the signed set.
        self.assertEqual(seen["platforms"], [({"windows"}, False), ({"windows"}, True)])
        exe = f"LingoPop-{VERSION}-windows-x64-portable.exe"
        manifest = (self.tmp / "release" / vra.MANIFEST).read_text(encoding="utf-8")
        self.assertEqual([line.split("  ")[1] for line in manifest.splitlines()], [exe, f"{exe}.minisig"])
        self.assertEqual(
            sorted(p.name for p in (self.tmp / "release").iterdir()),
            sorted([vra.MANIFEST, exe, f"{exe}.minisig"]),
        )
        self.assertEqual(self.publish_calls, 1)
        self.assertEqual(
            [Path(a).name for a in self.double.calls[0][4:]], [exe, f"{exe}.minisig", vra.MANIFEST]
        )
        notes = (self.tmp / "notes.md").read_text(encoding="utf-8")
        self.assertIn("Windows text.", notes)
        self.assertNotIn("macOS text.", notes)


class Publish(Base):
    def test_complete_set_publishes_once_with_every_asset_and_the_combined_manifest(self):
        write_artifacts(self.tmp / "artifacts")
        self.assertEqual(self.publish(), [])
        self.assertEqual(self.publish_calls, 1)
        argv = self.double.calls[0]
        self.assertEqual(argv[:4], ["gh", "release", "create", f"v{VERSION}"])
        self.assertEqual([Path(a).name for a in argv[4:]], signed_order() + [vra.MANIFEST])
        self.assertEqual(vra.verify_assets(self.tmp / "release", VERSION, ALL, signed=True), [])
        manifest = (self.tmp / "release" / vra.MANIFEST).read_bytes()
        self.assertTrue(manifest.endswith(b"\n"))
        self.assertNotIn(b"\r", manifest)
        self.assertEqual(len(manifest.splitlines()), 8)
        # 9 files: exe, 2 dmg, 2 zip, 3 signatures, the manifest.
        self.assertEqual(
            sorted(p.name for p in (self.tmp / "release").iterdir()),
            sorted(signed_order() + [vra.MANIFEST]),
        )
        self.assertEqual(len(list((self.tmp / "release").iterdir())), 9)

    def test_adhoc_notes_show_the_gatekeeper_paragraph(self):
        write_artifacts(self.tmp / "artifacts", mode="adhoc")
        self.assertEqual(self.publish(), [])
        notes = (self.tmp / "notes.md").read_text(encoding="utf-8")
        self.assertTrue(notes.startswith("### Added\n\n- macOS builds."), notes)
        self.assertNotIn("older", notes)
        self.assertNotIn("Intro shown only in the docs", notes)
        self.assertNotIn("<!--", notes)
        for text in ("## Installation", "Windows text.", "macOS text.", "Gatekeeper text.", "Re-grant text."):
            self.assertIn(text, notes)
        self.assertLess(notes.index("macOS builds."), notes.index("## Installation"))

    def test_notarized_notes_omit_the_gatekeeper_paragraph(self):
        write_artifacts(self.tmp / "artifacts", mode="developer-id")
        self.assertEqual(self.publish(), [])
        notes = (self.tmp / "notes.md").read_text(encoding="utf-8")
        self.assertIn("macOS text.", notes)
        self.assertNotIn("Gatekeeper text.", notes)
        self.assertNotIn("Re-grant text.", notes)

    def test_selfsigned_is_accepted_by_assemble(self):
        write_artifacts(self.tmp / "artifacts", mode="selfsigned")
        errors, mode = vra.assemble(self.tmp / "artifacts", VERSION, ALL, self.tmp / "release")
        self.assertEqual(errors, [])
        self.assertEqual(mode, "selfsigned")

    def test_selfsigned_notes_show_open_anyway_but_not_the_regrant(self):
        write_artifacts(self.tmp / "artifacts", mode="selfsigned")
        self.assertEqual(self.publish(), [])
        self.assertEqual(self.publish_calls, 1)
        notes = (self.tmp / "notes.md").read_text(encoding="utf-8")
        self.assertIn("macOS text.", notes)
        self.assertIn("Gatekeeper text.", notes)
        self.assertNotIn("Re-grant text.", notes)

    def test_unknown_signing_mode_blocks_publication(self):
        write_artifacts(self.tmp / "artifacts", mode="self-signed")
        self.assertIn("signing-macos-arm64.txt: missing or invalid", self.publish())
        self.assertEqual(self.publish_calls, 0)

    def test_version_placeholder_is_replaced(self):
        (self.tmp / "INSTALL.md").write_text(INSTALL.replace("Windows text.", "Get LingoPop-<version>.exe"), encoding="utf-8", newline="\n")
        write_artifacts(self.tmp / "artifacts")
        self.assertEqual(self.publish(), [])
        notes = (self.tmp / "notes.md").read_text(encoding="utf-8")
        self.assertIn(f"Get LingoPop-{VERSION}.exe", notes)

    def test_missing_changelog_section_uses_the_fallback_line(self):
        (self.tmp / "CHANGELOG.md").write_text("# Changelog\n\n## Unreleased\n", encoding="utf-8", newline="\n")
        write_artifacts(self.tmp / "artifacts")
        self.assertEqual(self.publish(), [])
        notes = (self.tmp / "notes.md").read_text(encoding="utf-8")
        self.assertTrue(notes.startswith(f"Test pre-release {VERSION} of LingoPop."))

    def test_missing_signing_mode_blocks_publication(self):
        write_artifacts(self.tmp / "artifacts")
        (self.tmp / "artifacts" / vra.artifact_name(VERSION, "macos-arm64") / "signing-macos-arm64.txt").unlink()
        self.assertIn("signing-macos-arm64.txt: missing or invalid", self.publish())
        self.assertEqual(self.publish_calls, 0)

    def test_disagreeing_signing_modes_block_publication(self):
        write_artifacts(self.tmp / "artifacts")
        (self.tmp / "artifacts" / vra.artifact_name(VERSION, "macos-x64") / "signing-macos-x64.txt").write_text("developer-id\n", encoding="utf-8", newline="\n")
        self.assertIn("the macOS legs disagree on the signing mode", self.publish())
        self.assertEqual(self.publish_calls, 0)

    def test_missing_artifact_blocks_publication(self):
        write_artifacts(self.tmp / "artifacts", slugs=("windows-x64", "macos-arm64"))
        self.assertIn(f"artifact LingoPop-{VERSION}-macos-x64: missing", self.publish())
        self.assertEqual(self.publish_calls, 0)

    def test_foreign_entry_in_a_platform_manifest_blocks_publication(self):
        write_artifacts(self.tmp / "artifacts")
        leg = self.tmp / "artifacts" / vra.artifact_name(VERSION, "macos-arm64")
        exe = f"LingoPop-{VERSION}-windows-x64-portable.exe"
        (leg / exe).write_bytes(pe_bytes())
        part = leg / "SHA256SUMS-macos-arm64.txt"
        part.write_text(part.read_text(encoding="utf-8") + vra.manifest_line(leg / exe), encoding="utf-8", newline="\n")
        errors = self.publish()
        self.assertIn(f"SHA256SUMS-macos-arm64.txt: unexpected entry {exe}", errors)
        self.assertEqual(self.publish_calls, 0)

    def test_non_empty_output_directory_blocks_publication(self):
        write_artifacts(self.tmp / "artifacts")
        (self.tmp / "release").mkdir()
        (self.tmp / "release" / "stale.txt").write_text("x", encoding="utf-8", newline="\n")
        self.assertTrue(any("output directory is not empty" in e for e in self.publish()))
        self.assertEqual(self.publish_calls, 0)

    def test_broken_install_template_blocks_publication(self):
        (self.tmp / "INSTALL.md").write_text(INSTALL.replace("<!-- endif -->\n<!-- endif -->", "<!-- endif -->"), encoding="utf-8", newline="\n")
        write_artifacts(self.tmp / "artifacts")
        self.assertIn("install template: if without endif", self.publish())
        self.assertEqual(self.publish_calls, 0)

    def cli(self, *args):
        return subprocess.run([sys.executable, str(SCRIPT), *map(str, args)], capture_output=True, text=True, check=False)

    def cli_prepare(self, artifacts, *extra):
        return self.cli(
            "prepare", "--version", VERSION, "--platforms-file", self.platforms_file,
            "--artifacts", artifacts, "--out", self.tmp / "release",
            "--changelog", self.tmp / "CHANGELOG.md", "--install", self.tmp / "INSTALL.md",
            "--notes-out", self.tmp / "notes.md", *extra,
        )

    def test_cli_prepare_fails_on_missing_artifacts(self):
        result = self.cli_prepare(self.tmp / "none")
        self.assertEqual(result.returncode, 1)
        self.assertIn("artifact LingoPop-", result.stderr)

    def test_cli_finalize_runs_nothing_on_error(self):
        write_artifacts(self.tmp / "artifacts")
        self.assertEqual(self.cli_prepare(self.tmp / "artifacts").returncode, 0)
        result = self.cli(
            "finalize", "--version", VERSION, "--platforms-file", self.platforms_file, self.tmp / "release",
            "--", sys.executable, "-c", f"open({str(self.tmp / 'ran')!r}, 'w').write('x')",
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("windows-x64-portable.exe.minisig: missing", result.stderr)
        self.assertFalse((self.tmp / "ran").exists())

    def test_cli_prepare_sign_finalize_runs_the_command_after_success(self):
        write_artifacts(self.tmp / "artifacts", mode="selfsigned")
        prepared = self.cli_prepare(self.tmp / "artifacts", "--require-mac-signing", "selfsigned")
        self.assertEqual(prepared.returncode, 0, prepared.stderr)
        listed = self.cli("updatable", "--version", VERSION, "--platforms-file", self.platforms_file)
        self.assertEqual(listed.returncode, 0, listed.stderr)
        names = listed.stdout.splitlines()
        self.assertEqual(names, [
            f"LingoPop-{VERSION}-windows-x64-portable.exe",
            f"LingoPop-{VERSION}-macos-arm64.zip",
            f"LingoPop-{VERSION}-macos-x64.zip",
        ])
        fake_sign(self.tmp / "release" / n for n in names)
        marker = self.tmp / "argv.json"
        code = f"import json, sys; json.dump(sys.argv[1:], open({str(marker)!r}, 'w'))"
        result = self.cli(
            "finalize", "--version", VERSION, "--platforms-file", self.platforms_file, self.tmp / "release",
            "--", sys.executable, "-c", code, "first",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        argv = json.loads(marker.read_text(encoding="utf-8"))
        self.assertEqual(argv[0], "first")
        self.assertEqual([Path(a).name for a in argv[1:]], signed_order() + [vra.MANIFEST])

    def test_cli_prepare_refuses_adhoc_when_selfsigned_is_required(self):
        write_artifacts(self.tmp / "artifacts", mode="adhoc")
        result = self.cli_prepare(self.tmp / "artifacts", "--require-mac-signing", "selfsigned")
        self.assertEqual(result.returncode, 1)
        self.assertIn("signing-macos-arm64.txt: adhoc is not allowed for a release (selfsigned required)", result.stderr)


class Signatures(Base):
    def test_missing_signature_blocks_publication(self):
        write_artifacts(self.tmp / "artifacts")

        def skip_one(paths):
            fake_sign(p for p in paths if not str(p).endswith("macos-arm64.zip"))

        self.assertIn("macos-arm64.zip.minisig: missing", self.publish(signer=skip_one))
        self.assertEqual(self.publish_calls, 0)

    def test_wrong_trusted_comment_blocks_publication(self):
        write_artifacts(self.tmp / "artifacts")
        other = f"LingoPop-{VERSION}-macos-arm64.zip"

        def wrong(paths):
            fake_sign(paths)
            bad = self.tmp / "release" / f"LingoPop-{VERSION}-macos-x64.zip.minisig"
            bad.write_text(minisig_text(other), encoding="utf-8", newline="\n")

        self.assertIn("macos-x64.zip.minisig: trusted comment does not name the asset", self.publish(signer=wrong))
        self.assertEqual(self.publish_calls, 0)

    def test_old_version_comment_is_rejected(self):
        write_assets(self.dir)
        write_signatures(self.dir)
        exe = f"LingoPop-{VERSION}-windows-x64-portable.exe"
        (self.dir / f"{exe}.minisig").write_text(
            minisig_text(exe, comment="lingopop-update LingoPop-0.2.0-windows-x64-portable.exe"),
            encoding="utf-8", newline="\n",
        )
        write_manifest(self.dir, signed_order())
        errors = vra.verify_assets(self.dir, VERSION, ALL, signed=True)
        self.assertEqual(errors, ["windows-x64-portable.exe.minisig: trusted comment does not name the asset"])

    def test_signed_set_passes_and_unsigned_check_rejects_signatures(self):
        write_assets(self.dir)
        write_signatures(self.dir)
        self.assertEqual(vra.verify_assets(self.dir, VERSION, ALL, signed=True), [])
        unsigned = vra.verify_assets(self.dir, VERSION, ALL)
        self.assertIn(f"LingoPop-{VERSION}-macos-x64.zip.minisig: unexpected file", unsigned)

    def test_signed_check_requires_every_signature(self):
        write_assets(self.dir)
        errors = vra.verify_assets(self.dir, VERSION, ALL, signed=True)
        for name in ("windows-x64-portable.exe", "macos-arm64.zip", "macos-x64.zip"):
            self.assertIn(f"{name}.minisig: missing", errors)
            self.assertIn(f"SHA256SUMS.txt: no entry for LingoPop-{VERSION}-{name}.minisig", errors)

    def test_a_dmg_is_never_signed(self):
        write_assets(self.dir)
        write_signatures(self.dir)
        dmg = self.dir / f"LingoPop-{VERSION}-macos-arm64.dmg"
        fake_sign([dmg])
        self.assertIn(f"{dmg.name}.minisig: unexpected file", vra.verify_assets(self.dir, VERSION, ALL, signed=True))

    def test_malformed_signature_files(self):
        write_assets(self.dir)
        name = f"LingoPop-{VERSION}-macos-arm64.zip"
        good = minisig_text(name)
        cases = {
            "": "macos-arm64.zip.minisig: size 0 bytes is out of range",
            "x" * 5000: "macos-arm64.zip.minisig: size 5000 bytes is out of range",
            "just text\n": "macos-arm64.zip.minisig: not a minisign signature",
            good.replace("\n", "\r\n"): "macos-arm64.zip.minisig: not a minisign signature",
            good.replace("trusted comment: ", "comment: "): "macos-arm64.zip.minisig: not a minisign signature",
            good.replace(name, name + " extra"): "macos-arm64.zip.minisig: trusted comment does not name the asset",
        }
        for text, needle in cases.items():
            write_signatures(self.dir)
            (self.dir / f"{name}.minisig").write_bytes(text.encode())
            write_manifest(self.dir, signed_order())
            self.assertIn(needle, vra.verify_assets(self.dir, VERSION, ALL, signed=True), repr(text[:60]))
        write_signatures(self.dir)
        (self.dir / f"{name}.minisig").write_bytes(b"\xff\xfe\n")
        write_manifest(self.dir, signed_order())
        self.assertIn("macos-arm64.zip.minisig: not a minisign signature", vra.verify_assets(self.dir, VERSION, ALL, signed=True))

    def test_an_asset_changed_after_the_pre_signing_check_is_a_checksum_mismatch(self):
        write_artifacts(self.tmp / "artifacts")

        def swap_then_sign(paths):
            exe = self.tmp / "release" / f"LingoPop-{VERSION}-windows-x64-portable.exe"
            exe.write_bytes(pe_bytes() + b"tampered")
            fake_sign(paths)

        self.assertIn("windows-x64-portable.exe: checksum mismatch", self.publish(signer=swap_then_sign))
        self.assertEqual(self.publish_calls, 0)

    def test_a_stray_signature_in_a_leg_artifact_is_not_copied(self):
        write_artifacts(self.tmp / "artifacts")
        leg = self.tmp / "artifacts" / vra.artifact_name(VERSION, "windows-x64")
        exe = leg / f"LingoPop-{VERSION}-windows-x64-portable.exe"
        fake_sign([exe])
        # assemble copies only the contract names, so the leg's stray signature never reaches
        # the release directory; signing then produces the only one.
        self.assertEqual(self.publish(), [])
        self.assertEqual(self.publish_calls, 1)

    def test_finalize_before_signing_fails(self):
        write_artifacts(self.tmp / "artifacts")
        self.assertEqual(self.prepare(), [])
        errors = self.finalize()
        self.assertIn("windows-x64-portable.exe.minisig: missing", errors)
        self.assertEqual(self.publish_calls, 0)

    def test_updatable_assets(self):
        self.assertEqual(vra.updatable_assets(VERSION, {"windows"}), [f"LingoPop-{VERSION}-windows-x64-portable.exe"])
        self.assertEqual(vra.updatable_assets(VERSION, ALL), [
            f"LingoPop-{VERSION}-windows-x64-portable.exe",
            f"LingoPop-{VERSION}-macos-arm64.zip",
            f"LingoPop-{VERSION}-macos-x64.zip",
        ])
        self.assertEqual(vra.asset_names(VERSION, "macos-arm64"), [
            f"LingoPop-{VERSION}-macos-arm64.dmg", f"LingoPop-{VERSION}-macos-arm64.zip",
        ])


class RequiredMacSigning(Base):
    def test_tag_release_with_mac_assets_refuses_adhoc(self):
        write_artifacts(self.tmp / "artifacts", mode="adhoc")
        errors = self.publish(require_mac_signing="selfsigned")
        self.assertIn("signing-macos-arm64.txt: adhoc is not allowed for a release (selfsigned required)", errors)
        self.assertIn("signing-macos-x64.txt: adhoc is not allowed for a release (selfsigned required)", errors)
        self.assertEqual(self.publish_calls, 0)

    def test_tag_release_with_mac_assets_refuses_developer_id(self):
        write_artifacts(self.tmp / "artifacts", mode="developer-id")
        errors = self.publish(require_mac_signing="selfsigned")
        self.assertIn("signing-macos-arm64.txt: developer-id is not allowed for a release (selfsigned required)", errors)
        self.assertEqual(self.publish_calls, 0)

    def test_tag_release_with_mac_assets_accepts_selfsigned(self):
        write_artifacts(self.tmp / "artifacts", mode="selfsigned")
        self.assertEqual(self.publish(require_mac_signing="selfsigned"), [])
        self.assertEqual(self.publish_calls, 1)

    def test_windows_only_release_ignores_the_mac_signing_mode(self):
        write_artifacts(self.tmp / "artifacts", mode="adhoc")
        self.platforms_file.write_text('{"macosPreview": false}\n', encoding="utf-8", newline="\n")
        self.assertEqual(self.publish(require_mac_signing="selfsigned"), [])
        self.assertEqual(self.publish_calls, 1)


class Verify(Base):
    def test_complete_set_passes(self):
        write_assets(self.dir)
        self.assertEqual(vra.verify_assets(self.dir, VERSION, ALL), [])

    def test_single_leg_with_its_own_manifest_passes(self):
        write_assets(self.dir, ("macos-arm64",), manifest="SHA256SUMS-macos-arm64.txt")
        self.assertEqual(vra.verify_assets(self.dir, VERSION, {"macos-arm64"}, "SHA256SUMS-macos-arm64.txt"), [])

    def test_unknown_platform_and_bad_version(self):
        write_assets(self.dir)
        self.assertIn("unknown platform 'linux'", vra.verify_assets(self.dir, VERSION, {"windows", "linux"}))
        self.assertEqual(vra.verify_assets(self.dir, "0.3", ALL), ["invalid version '0.3'"])

    def test_extra_file_fails(self):
        write_assets(self.dir)
        (self.dir / "notes.md").write_text("x", encoding="utf-8", newline="\n")
        self.assertIn("notes.md: unexpected file", vra.verify_assets(self.dir, VERSION, ALL))

    def test_windows_only_set_rejects_mac_assets(self):
        write_assets(self.dir)
        errors = vra.verify_assets(self.dir, VERSION, {"windows"})
        self.assertIn(f"LingoPop-{VERSION}-macos-arm64.dmg: unexpected file", errors)
        self.assertIn(f"SHA256SUMS.txt: unexpected entry LingoPop-{VERSION}-macos-arm64.dmg", errors)

    def test_duplicate_manifest_entry(self):
        write_assets(self.dir)
        manifest = self.dir / vra.MANIFEST
        first = manifest.read_text(encoding="utf-8").splitlines(keepends=True)[0]
        manifest.write_text(manifest.read_text(encoding="utf-8") + first, encoding="utf-8", newline="\n")
        name = first.split("  ")[1].strip()
        self.assertIn(f"SHA256SUMS.txt: duplicate entry {name}", vra.verify_assets(self.dir, VERSION, ALL))

    def test_uppercase_checksum_is_malformed(self):
        write_assets(self.dir)
        manifest = self.dir / vra.MANIFEST
        lines = manifest.read_text(encoding="utf-8").splitlines(keepends=True)
        lines[0] = lines[0][:64].upper() + lines[0][64:]
        manifest.write_text("".join(lines), encoding="utf-8", newline="\n")
        errors = vra.verify_assets(self.dir, VERSION, ALL)
        self.assertIn("SHA256SUMS.txt: line 1 is malformed", errors)

    def test_malformed_manifest_variants(self):
        write_assets(self.dir)
        manifest = self.dir / vra.MANIFEST
        good = manifest.read_text(encoding="utf-8")
        cases = {
            good.replace("  ", " ", 1): "SHA256SUMS.txt: line 1 is malformed",
            good.rstrip("\n"): "SHA256SUMS.txt: must end with a newline",
            good.replace("\n", "\r\n"): "SHA256SUMS.txt: must use LF line endings",
            good.replace("\n", "\n\n", 1): "SHA256SUMS.txt: line 2 is malformed",
            good.replace(f"LingoPop-{VERSION}", f"dir/LingoPop-{VERSION}", 1): "SHA256SUMS.txt: line 1 is malformed",
            "": "SHA256SUMS.txt: empty",
        }
        for text, needle in cases.items():
            manifest.write_bytes(text.encode())
            self.assertIn(needle, vra.verify_assets(self.dir, VERSION, ALL), repr(text[:80]))
        manifest.unlink()
        self.assertIn("SHA256SUMS.txt: missing", vra.verify_assets(self.dir, VERSION, ALL))

    def test_manifest_missing_an_asset(self):
        write_assets(self.dir)
        manifest = self.dir / vra.MANIFEST
        manifest.write_text("".join(manifest.read_text(encoding="utf-8").splitlines(keepends=True)[1:]), encoding="utf-8", newline="\n")
        self.assertIn(
            f"SHA256SUMS.txt: no entry for LingoPop-{VERSION}-windows-x64-portable.exe",
            vra.verify_assets(self.dir, VERSION, ALL),
        )

    def test_empty_asset_is_out_of_range(self):
        write_assets(self.dir, **{f"LingoPop-{VERSION}-macos-arm64.dmg": b""})
        self.assertIn("macos-arm64.dmg: size 0 bytes is out of range", vra.verify_assets(self.dir, VERSION, ALL))

    def test_windows_exe_checks(self):
        exe = f"LingoPop-{VERSION}-windows-x64-portable.exe"
        write_assets(self.dir, **{exe: pe_bytes(machine=0xAA64)})
        self.assertIn("windows-x64-portable.exe: executable architecture is not x86_64", vra.verify_assets(self.dir, VERSION, ALL))
        write_assets(self.dir, **{exe: b"not a pe file" * 10})
        self.assertIn("windows-x64-portable.exe: not a Windows executable", vra.verify_assets(self.dir, VERSION, ALL))

    def test_dmg_without_udif_trailer(self):
        write_assets(self.dir, **{f"LingoPop-{VERSION}-macos-x64.dmg": b"x" * 2048})
        self.assertIn("macos-x64.dmg: not a UDIF disk image", vra.verify_assets(self.dir, VERSION, ALL))

    def zip_errors(self, **kwargs):
        name = f"LingoPop-{VERSION}-macos-arm64.zip"
        write_assets(self.dir, **{name: app_zip(**kwargs)})
        return vra.verify_assets(self.dir, VERSION, ALL)

    def test_wrong_bundle_identifier(self):
        errors = self.zip_errors(bundle_id="com.lingopop.app")
        self.assertIn("macos-arm64.zip: bundle identifier is 'com.lingopop.app', expected app.lingopop.tauri.macos", errors)

    def test_wrong_bundle_version(self):
        self.assertIn("macos-arm64.zip: bundle version is '0.2.0', expected 0.3.0-alpha.1", self.zip_errors(version="0.2.0"))

    def test_wrong_minimum_macos(self):
        self.assertIn("macos-arm64.zip: minimum macOS version is '11.0', expected 13.0", self.zip_errors(minimum="11.0"))

    def test_wrong_deployment_target(self):
        self.assertIn("macos-arm64.zip: executable deployment target is '11.0', expected 13.0", self.zip_errors(minos=(11, 0)))

    def test_not_a_menu_bar_app(self):
        self.assertIn("macos-arm64.zip: LSUIElement is not set (the app must be a menu-bar app)", self.zip_errors(ui_element=False))

    def test_unsigned_bundle(self):
        errors = self.zip_errors(signed=False)
        self.assertIn("macos-arm64.zip: app bundle is not signed", errors)
        self.assertIn("macos-arm64.zip: bundle executable has no code signature", errors)

    def test_executable_without_execute_bit(self):
        self.assertIn("macos-arm64.zip: bundle executable has no execute permission", self.zip_errors(executable_mode=0o644))

    def test_universal_executable_is_the_wrong_architecture(self):
        self.assertIn("macos-arm64.zip: executable architecture is not arm64", self.zip_errors(fat=True))

    def test_foreign_top_level_entry(self):
        self.assertIn("macos-arm64.zip: archive must contain only LingoPop.app", self.zip_errors(extra_top="Other.app/x"))

    def test_garbage_zip(self):
        write_assets(self.dir, **{f"LingoPop-{VERSION}-macos-arm64.zip": b"PK not really"})
        self.assertIn("macos-arm64.zip: not a valid zip archive", vra.verify_assets(self.dir, VERSION, ALL))

    def test_cli_verify(self):
        write_assets(self.dir)
        ok = subprocess.run(
            [sys.executable, str(SCRIPT), "verify", "--version", VERSION, "--platforms", "windows,macos", str(self.dir)],
            capture_output=True, text=True, check=False,
        )
        self.assertEqual(ok.returncode, 0, ok.stderr)
        self.assertIn("release assets: ok", ok.stdout)
        (self.dir / f"LingoPop-{VERSION}-macos-x64.zip").unlink()
        bad = subprocess.run(
            [sys.executable, str(SCRIPT), "verify", "--version", VERSION, "--platforms", "windows,macos", str(self.dir)],
            capture_output=True, text=True, check=False,
        )
        self.assertEqual(bad.returncode, 1)
        self.assertIn("error: macos-x64.zip: missing", bad.stderr)


class Repository(unittest.TestCase):
    @staticmethod
    def _job(workflow, name):
        """The text of the top-level job `name` (up to the next job or the end)."""
        start = workflow.index(f"\n  {name}:\n")
        following = re.search(r"\n  [a-z][a-z0-9-]*:\n", workflow[start + 1:])
        return workflow[start: start + 1 + following.start()] if following else workflow[start:]

    @staticmethod
    def _step(job, name):
        start = job.index(f"- name: {name}\n")
        end = job.find("\n      - ", start + 1)
        return job[start:] if end < 0 else job[start:end]

    def test_release_workflow_publishes_a_checked_draft(self):
        workflow = (REPO_ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
        publish_job = self._job(workflow, "publish")
        tools_job = self._job(workflow, "build-release-tools")
        # The verifier / signer is built in its own job without secrets; publish builds nothing.
        self.assertIn("cargo build -p lingopop-release-tools --release --locked", tools_job)
        self.assertNotIn("secrets.", tools_job)
        self.assertIn("sha256:", tools_job)
        self.assertRegex(publish_job, r"needs: \[build, build-macos, build-release-tools\]")
        for forbidden in ("cargo ", "npm ", "rust-toolchain", "rust-cache", "setup-node"):
            self.assertNotIn(forbidden, publish_job, forbidden)
        # Only the publish job's signing step holds the update key and its password.
        for name in ("UPDATE_SIGNING_KEY", "UPDATE_SIGNING_KEY_PASSWORD"):
            self.assertEqual(workflow.count(f"secrets.{name} }}}}"), 1, name)
        sign_step = self._step(publish_job, "Sign the update assets")
        self.assertIn("UPDATE_SIGNING_KEY: ${{ secrets.UPDATE_SIGNING_KEY }}", sign_step)
        self.assertIn("UPDATE_SIGNING_KEY_PASSWORD: ${{ secrets.UPDATE_SIGNING_KEY_PASSWORD }}", sign_step)
        self.assertIn("umask 077", sign_step)
        self.assertIn('"$RUNNER_TEMP/update-signing.key"', sign_step)
        self.assertIn("--password-env UPDATE_SIGNING_KEY_PASSWORD", sign_step)
        self.assertNotIn("set -x", workflow)
        self.assertNotIn("echo \"$UPDATE_SIGNING", workflow)
        remove_step = self._step(publish_job, "Remove the update signing key")
        self.assertIn("if: always()", remove_step)
        self.assertIn('rm -f "$RUNNER_TEMP/update-signing.key"', remove_step)

        # The secrets come from the tag-only `release` environment, never on a dispatch run.
        self.assertIn("\n    environment: release\n", publish_job)
        macos_job = self._job(workflow, "build-macos")
        self.assertIn("environment: ${{ github.event_name == 'push' && github.ref_type == 'tag' && 'release' || '' }}", macos_job)
        self.assertNotIn("environment:", tools_job)

        tool_check = publish_job.index("needs.build-release-tools.outputs.sha256")
        download = publish_job.index("pattern: LingoPop-${{ needs.build.outputs.version }}-*")
        prepare = publish_job.index("scripts/verify-release-assets.py prepare")
        require = publish_job.index("--require-mac-signing selfsigned")
        sign = publish_job.index("lingopop-release-tools sign --key")
        crypto = publish_job.index("lingopop-release-tools verify --public-key const")
        finalize = publish_job.index("scripts/verify-release-assets.py finalize")
        remove = publish_job.index("- name: Remove the update signing key")
        channel = workflow.index('if [[ "$VERSION" == *-* ]]; then channel=(--prerelease); fi')
        create = workflow.index('gh release create "$TAG" --draft "${channel[@]}"')
        check = workflow.index('gh release download "$TAG" --dir uploaded')
        publish_pre = workflow.index('gh release edit "$TAG" --draft=false --prerelease\n')
        publish_latest = workflow.index('gh release edit "$TAG" --draft=false --prerelease=false --latest')
        self.assertLess(tool_check, download)
        self.assertLess(download, prepare)
        self.assertLess(prepare, require)
        self.assertLess(require, sign)
        self.assertLess(sign, crypto)
        self.assertLess(crypto, finalize)
        self.assertLess(sign, remove)
        self.assertLess(workflow.index("scripts/verify-release-assets.py finalize"), create)
        self.assertLess(channel, create)
        self.assertLess(create, check)
        self.assertLess(check, publish_pre)
        self.assertLess(check, publish_latest)
        # Every version capture strips a Windows CR.
        captures = re.findall(r"version=\$\(python scripts/check-version.py[^)]*\)", workflow)
        self.assertEqual(len(captures), 5)
        for capture in captures:
            self.assertIn("| tr -d '\\r'", capture)

    @staticmethod
    def _run_block(block, version):
        """Run a workflow shell block with a recording `gh` stub; return the gh argv lines."""
        with tempfile.TemporaryDirectory() as tmp:
            stub = Path(tmp) / "gh"
            log = Path(tmp) / "gh.log"
            stub.write_text('#!/bin/sh\necho "$*" >> "$GH_LOG"\n', encoding="utf-8")
            stub.chmod(0o755)
            env = {"PATH": f"{tmp}:/usr/bin:/bin", "GH_LOG": str(log), "VERSION": version, "TAG": f"v{version}"}
            subprocess.run(["bash", "-c", block], env=env, check=True)
            return log.read_text(encoding="utf-8").splitlines() if log.exists() else []

    @unittest.skipIf(sys.platform == "win32", "runs the workflow bash blocks with a POSIX gh stub")
    def test_release_channel_follows_the_version(self):
        workflow = (REPO_ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
        channel_line = re.search(r"^ *(channel=\(\)\n *if .*channel=\(--prerelease\); fi)$", workflow, re.M)
        self.assertIsNotNone(channel_line)
        channel = textwrap.dedent(channel_line.group(0))
        create = channel + '\ngh release create "$TAG" --draft "${channel[@]}" --title "LingoPop $VERSION"'
        publish_start = workflow.index("      - name: Publish the release")
        publish_run = workflow[workflow.index("run: |", publish_start) + len("run: |"):]
        publish = textwrap.dedent(publish_run)
        for version, created, published in (
            ("0.2.0", "release create v0.2.0 --draft --title LingoPop 0.2.0",
             "release edit v0.2.0 --draft=false --prerelease=false --latest"),
            ("0.3.0-alpha.1", "release create v0.3.0-alpha.1 --draft --prerelease --title LingoPop 0.3.0-alpha.1",
             "release edit v0.3.0-alpha.1 --draft=false --prerelease"),
        ):
            with self.subTest(version=version):
                self.assertEqual(self._run_block(create, version), [created])
                self.assertEqual(self._run_block(publish, version), [published])

    def test_committed_release_platforms_file_is_valid(self):
        self.assertIsNotNone(vra.read_release_platforms(REPO_ROOT / "release-platforms.json"))

    def test_committed_install_template_renders_for_every_mode(self):
        template = (REPO_ROOT / "docs" / "INSTALL.md").read_text(encoding="utf-8")
        for platforms, mode in (({"windows"}, None), (ALL, "adhoc"), (ALL, "selfsigned"), (ALL, "developer-id")):
            notes, errors = vra.render_notes(CHANGELOG, template, VERSION, platforms, mode)
            self.assertEqual(errors, [])
            self.assertTrue(notes.startswith("### Added"), notes[:80])
            self.assertIn("## Installation", notes)
            self.assertNotIn("<!--", notes)
            self.assertNotIn("<version>", notes)
            self.assertNotIn("release-notes-start", notes)
            self.assertIn("SHA256SUMS.txt", notes)
            self.assertEqual("### Windows" in notes, True)
            self.assertEqual(f"LingoPop-{VERSION}-windows-x64-portable.exe" in notes, True)
            self.assertEqual("### macOS" in notes, "macos" in platforms)
            self.assertEqual(f"LingoPop-{VERSION}-macos-arm64.dmg" in notes, "macos" in platforms)
            self.assertEqual("shasum -a 256 -c" in notes, "macos" in platforms)
            self.assertEqual("Open Anyway" in notes, mode in ("adhoc", "selfsigned"))
            self.assertEqual("com.apple.quarantine" in notes, mode in ("adhoc", "selfsigned"))
            self.assertEqual("re-grant" in notes or "add it again" in notes, mode == "adhoc")


if __name__ == "__main__":
    unittest.main()
