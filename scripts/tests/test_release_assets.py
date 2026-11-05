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
import struct
import subprocess
import sys
import tempfile
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
APP = "LingoPop Preview.app"
INSTALL = """# Installing LingoPop

Intro shown only in the docs.

<!-- release-notes-start -->
## Installation

<!-- if windows -->
Windows text.
<!-- endif -->

<!-- if macos -->
macOS text.

<!-- if macos-adhoc -->
Gatekeeper text.
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
            (leg / f"signing-{slug}.txt").write_text(mode + "\n", encoding="utf-8")


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
        (self.tmp / "CHANGELOG.md").write_text(CHANGELOG, encoding="utf-8")
        (self.tmp / "INSTALL.md").write_text(INSTALL, encoding="utf-8")
        self.platforms_file = self.tmp / "release-platforms.json"
        self.platforms_file.write_text('{"macosPreview": true}\n', encoding="utf-8")
        self.double = PublishDouble()

    def publish(self, artifacts=None):
        return vra.publish(
            artifacts or self.tmp / "artifacts",
            VERSION,
            self.platforms_file,
            self.tmp / "release",
            self.tmp / "CHANGELOG.md",
            self.tmp / "INSTALL.md",
            self.tmp / "notes.md",
            ["gh", "release", "create", f"v{VERSION}"],
            runner=self.double,
        )

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
        wrong_type.write_text('{"macosPreview": "yes"}', encoding="utf-8")
        missing_file = self.tmp / "missing.json"
        invalid_json = self.tmp / "invalid.json"
        invalid_json.write_text('{"macosPreview": true', encoding="utf-8")
        extra_key = self.tmp / "extra.json"
        extra_key.write_text('{"macosPreview": true, "assets": []}', encoding="utf-8")
        for variant in (wrong_type, missing_file, invalid_json, extra_key):
            self.assertIsNone(vra.read_release_platforms(variant))
            self.platforms_file = variant
            self.assertNotEqual(self.publish(), [])
            self.assertEqual(self.publish_calls, 0)

    def test_mac_preview_false_manifest_and_platform_set(self):
        self.assertEqual(vra.platforms_for({"macosPreview": False}), {"windows"})
        self.assertEqual(vra.platforms_for({"macosPreview": True}), {"windows", "macos"})

        write_artifacts(self.tmp / "artifacts")
        self.platforms_file.write_text('{"macosPreview": false}\n', encoding="utf-8")
        seen = {}
        real_verify = vra.verify_assets

        def spy(directory, version, platforms, manifest=vra.MANIFEST):
            seen["platforms"] = set(platforms)
            return real_verify(directory, version, platforms, manifest)

        vra.verify_assets = spy
        try:
            self.assertEqual(self.publish(), [])
        finally:
            vra.verify_assets = real_verify
        self.assertEqual(seen["platforms"], {"windows"})
        exe = f"LingoPop-{VERSION}-windows-x64-portable.exe"
        manifest = (self.tmp / "release" / vra.MANIFEST).read_text(encoding="utf-8")
        self.assertEqual([line.split("  ")[1] for line in manifest.splitlines()], [exe])
        self.assertEqual(sorted(p.name for p in (self.tmp / "release").iterdir()), sorted([vra.MANIFEST, exe]))
        self.assertEqual(self.publish_calls, 1)
        self.assertEqual(
            [Path(a).name for a in self.double.calls[0][4:]], [exe, vra.MANIFEST]
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
        self.assertEqual(
            [Path(a).name for a in argv[4:]],
            [n for s in vra.PLATFORMS for n in vra.asset_names(VERSION, s)] + [vra.MANIFEST],
        )
        self.assertEqual(vra.verify_assets(self.tmp / "release", VERSION, ALL), [])
        manifest = (self.tmp / "release" / vra.MANIFEST).read_bytes()
        self.assertTrue(manifest.endswith(b"\n"))
        self.assertNotIn(b"\r", manifest)
        self.assertEqual(len(manifest.splitlines()), 5)

    def test_adhoc_notes_show_the_gatekeeper_paragraph(self):
        write_artifacts(self.tmp / "artifacts", mode="adhoc")
        self.assertEqual(self.publish(), [])
        notes = (self.tmp / "notes.md").read_text(encoding="utf-8")
        self.assertTrue(notes.startswith("### Added\n\n- macOS builds."), notes)
        self.assertNotIn("older", notes)
        self.assertNotIn("Intro shown only in the docs", notes)
        self.assertNotIn("<!--", notes)
        for text in ("## Installation", "Windows text.", "macOS text.", "Gatekeeper text."):
            self.assertIn(text, notes)
        self.assertLess(notes.index("macOS builds."), notes.index("## Installation"))

    def test_notarized_notes_omit_the_gatekeeper_paragraph(self):
        write_artifacts(self.tmp / "artifacts", mode="developer-id")
        self.assertEqual(self.publish(), [])
        notes = (self.tmp / "notes.md").read_text(encoding="utf-8")
        self.assertIn("macOS text.", notes)
        self.assertNotIn("Gatekeeper text.", notes)

    def test_version_placeholder_is_replaced(self):
        (self.tmp / "INSTALL.md").write_text(INSTALL.replace("Windows text.", "Get LingoPop-<version>.exe"))
        write_artifacts(self.tmp / "artifacts")
        self.assertEqual(self.publish(), [])
        notes = (self.tmp / "notes.md").read_text(encoding="utf-8")
        self.assertIn(f"Get LingoPop-{VERSION}.exe", notes)

    def test_missing_changelog_section_uses_the_fallback_line(self):
        (self.tmp / "CHANGELOG.md").write_text("# Changelog\n\n## Unreleased\n", encoding="utf-8")
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
        (self.tmp / "artifacts" / vra.artifact_name(VERSION, "macos-x64") / "signing-macos-x64.txt").write_text("developer-id\n")
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
        part.write_text(part.read_text() + vra.manifest_line(leg / exe), encoding="utf-8", newline="\n")
        errors = self.publish()
        self.assertIn(f"SHA256SUMS-macos-arm64.txt: unexpected entry {exe}", errors)
        self.assertEqual(self.publish_calls, 0)

    def test_non_empty_output_directory_blocks_publication(self):
        write_artifacts(self.tmp / "artifacts")
        (self.tmp / "release").mkdir()
        (self.tmp / "release" / "stale.txt").write_text("x")
        self.assertTrue(any("output directory is not empty" in e for e in self.publish()))
        self.assertEqual(self.publish_calls, 0)

    def test_broken_install_template_blocks_publication(self):
        (self.tmp / "INSTALL.md").write_text(INSTALL.replace("<!-- endif -->\n<!-- endif -->", "<!-- endif -->"))
        write_artifacts(self.tmp / "artifacts")
        self.assertIn("install template: if without endif", self.publish())
        self.assertEqual(self.publish_calls, 0)

    def test_cli_publish_runs_nothing_on_error(self):
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "publish", "--version", VERSION,
             "--platforms-file", str(self.platforms_file), "--artifacts", str(self.tmp / "none"),
             "--out", str(self.tmp / "release"), "--changelog", str(self.tmp / "CHANGELOG.md"),
             "--install", str(self.tmp / "INSTALL.md"), "--notes-out", str(self.tmp / "notes.md"),
             "--", sys.executable, "-c", f"open({str(self.tmp / 'ran')!r}, 'w').write('x')"],
            capture_output=True, text=True, check=False,
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("artifact LingoPop-", result.stderr)
        self.assertFalse((self.tmp / "ran").exists())

    def test_cli_publish_runs_the_command_after_success(self):
        write_artifacts(self.tmp / "artifacts")
        marker = self.tmp / "argv.json"
        code = f"import json, sys; json.dump(sys.argv[1:], open({str(marker)!r}, 'w'))"
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "publish", "--version", VERSION,
             "--platforms-file", str(self.platforms_file), "--artifacts", str(self.tmp / "artifacts"),
             "--out", str(self.tmp / "release"), "--changelog", str(self.tmp / "CHANGELOG.md"),
             "--install", str(self.tmp / "INSTALL.md"), "--notes-out", str(self.tmp / "notes.md"),
             "--", sys.executable, "-c", code, "first"],
            capture_output=True, text=True, check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        argv = json.loads(marker.read_text())
        self.assertEqual(argv[0], "first")
        self.assertEqual(len(argv), 1 + 5 + 1)


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
        (self.dir / "notes.md").write_text("x")
        self.assertIn("notes.md: unexpected file", vra.verify_assets(self.dir, VERSION, ALL))

    def test_windows_only_set_rejects_mac_assets(self):
        write_assets(self.dir)
        errors = vra.verify_assets(self.dir, VERSION, {"windows"})
        self.assertIn(f"LingoPop-{VERSION}-macos-arm64.dmg: unexpected file", errors)
        self.assertIn(f"SHA256SUMS.txt: unexpected entry LingoPop-{VERSION}-macos-arm64.dmg", errors)

    def test_duplicate_manifest_entry(self):
        write_assets(self.dir)
        manifest = self.dir / vra.MANIFEST
        first = manifest.read_text().splitlines(keepends=True)[0]
        manifest.write_text(manifest.read_text() + first, encoding="utf-8", newline="\n")
        name = first.split("  ")[1].strip()
        self.assertIn(f"SHA256SUMS.txt: duplicate entry {name}", vra.verify_assets(self.dir, VERSION, ALL))

    def test_uppercase_checksum_is_malformed(self):
        write_assets(self.dir)
        manifest = self.dir / vra.MANIFEST
        lines = manifest.read_text().splitlines(keepends=True)
        lines[0] = lines[0][:64].upper() + lines[0][64:]
        manifest.write_text("".join(lines), encoding="utf-8", newline="\n")
        errors = vra.verify_assets(self.dir, VERSION, ALL)
        self.assertIn("SHA256SUMS.txt: line 1 is malformed", errors)

    def test_malformed_manifest_variants(self):
        write_assets(self.dir)
        manifest = self.dir / vra.MANIFEST
        good = manifest.read_text()
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
        manifest.write_text("".join(manifest.read_text().splitlines(keepends=True)[1:]), encoding="utf-8", newline="\n")
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
        self.assertIn("macos-arm64.zip: archive must contain only LingoPop Preview.app", self.zip_errors(extra_top="Other.app/x"))

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
    def test_committed_release_platforms_file_is_valid(self):
        self.assertIsNotNone(vra.read_release_platforms(REPO_ROOT / "release-platforms.json"))

    def test_committed_install_template_renders_for_every_mode(self):
        template = (REPO_ROOT / "docs" / "INSTALL.md").read_text(encoding="utf-8")
        for platforms, mode in (({"windows"}, None), (ALL, "adhoc"), (ALL, "developer-id")):
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
            self.assertEqual("Open Anyway" in notes, mode == "adhoc")
            self.assertEqual("com.apple.quarantine" in notes, mode == "adhoc")
            self.assertEqual("re-grant" in notes or "add it again" in notes, mode == "adhoc")


if __name__ == "__main__":
    unittest.main()
