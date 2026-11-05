#!/usr/bin/env python3
"""Release asset contract: verify, assemble and publish the platform artifacts.

    verify-release-assets.py verify  --version V --platforms windows,macos [--manifest NAME] DIR
    verify-release-assets.py publish --version V --platforms-file release-platforms.json \\
        --artifacts DIR --out DIR --changelog CHANGELOG.md --install docs/INSTALL.md \\
        --notes-out notes.md -- gh release create TAG --prerelease ...

Asset contract (fixed names, derived from the version; no file lists them):

- `windows-x64`: `LingoPop-<version>-windows-x64-portable.exe`
- `macos-arm64`: `LingoPop-<version>-macos-arm64.dmg`, `LingoPop-<version>-macos-arm64.zip`
- `macos-x64`:   `LingoPop-<version>-macos-x64.dmg`,   `LingoPop-<version>-macos-x64.zip`

Each build leg uploads the artifact `LingoPop-<version>-<platform>` with its assets,
`SHA256SUMS-<platform>.txt` and, for the macOS legs, `signing-<platform>.txt` (`adhoc` or
`developer-id`). `publish` reads `release-platforms.json` (`{"macosPreview": bool}`; anything else
fails closed), copies the selected assets into `--out`, concatenates the per-platform manifests
into `SHA256SUMS.txt`, runs `verify_assets` on the result, renders the release notes (CHANGELOG
section + the Installation section of `docs/INSTALL.md`) and only then runs the given command
with the asset paths appended. Any error means the command never runs.

`verify_assets` inspects everything without mounting a disk image, so it runs on Linux: the zip's
Info.plist (bundle id, version, minimum macOS, LSUIElement), the Mach-O header of the bundle
executable (architecture, deployment target, code signature), the PE header of the Windows exe
and the UDIF trailer of each dmg. The native macOS legs additionally mount the dmg and check the
signature with codesign / spctl (`scripts/package-macos.sh`). Python standard library only, 3.9+.
"""
import argparse
import hashlib
import json
import plistlib
import re
import shutil
import struct
import subprocess
import sys
import zipfile
from pathlib import Path
from typing import Callable, Dict, List, Optional, Sequence, Set, Tuple

PLATFORMS = ("windows-x64", "macos-arm64", "macos-x64")
ALIASES = {
    "windows": ("windows-x64",),
    "macos": ("macos-arm64", "macos-x64"),
}
MANIFEST = "SHA256SUMS.txt"
BUNDLE_ID = "app.lingopop.tauri.macos"
APP_DIR = "LingoPop Preview.app"
MIN_MACOS = "13.0"
SIGNING_MODES = ("adhoc", "developer-id")
MAX_ASSET_BYTES = 1 << 30  # 1 GiB; a real asset is a few MB, an empty one is always wrong
MAX_EXECUTABLE_BYTES = 512 << 20
VERSION_RE = re.compile(
    r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$"
)
MANIFEST_LINE = re.compile(r"^([0-9a-f]{64})  ([^\s/\\]+)$")

# Mach-O
MH_MAGIC_64 = 0xFEEDFACF
FAT_MAGIC = 0xCAFEBABE
FAT_MAGIC_64 = 0xCAFEBABF
CPU_TYPES = {"arm64": 0x0100000C, "x86_64": 0x01000007}
MACHO_ARCH = {"macos-arm64": "arm64", "macos-x64": "x86_64"}
LC_CODE_SIGNATURE = 0x1D
LC_VERSION_MIN_MACOSX = 0x24
LC_BUILD_VERSION = 0x32
PLATFORM_MACOS = 1
# PE
IMAGE_FILE_MACHINE_AMD64 = 0x8664


# ---------------------------------------------------------------------------------------------
# Contract


def resolve_platforms(platforms) -> Tuple[List[str], List[str]]:
    """Expands `windows` / `macos` aliases; returns (slugs in contract order, errors)."""
    slugs: Set[str] = set()
    errors = []
    for name in platforms:
        if name in ALIASES:
            slugs.update(ALIASES[name])
        elif name in PLATFORMS:
            slugs.add(name)
        else:
            errors.append(f"unknown platform {name!r}")
    if not platforms:
        errors.append("no platform selected")
    return [p for p in PLATFORMS if p in slugs], errors


def asset_names(version: str, slug: str) -> List[str]:
    if slug == "windows-x64":
        return [f"LingoPop-{version}-windows-x64-portable.exe"]
    return [f"LingoPop-{version}-{slug}.dmg", f"LingoPop-{version}-{slug}.zip"]


def artifact_name(version: str, slug: str) -> str:
    return f"LingoPop-{version}-{slug}"


def label(version: str, name: str) -> str:
    """`LingoPop-0.3.0-macos-x64.zip` -> `macos-x64.zip` (the error prefix)."""
    prefix = f"LingoPop-{version}-"
    return name[len(prefix):] if name.startswith(prefix) else name


def read_release_platforms(path: Path) -> Optional[Dict[str, bool]]:
    """`{"macosPreview": <bool>}` or None for a missing, unreadable, invalid or extended file."""
    try:
        data = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    if not isinstance(data, dict) or set(data) != {"macosPreview"}:
        return None
    if not isinstance(data["macosPreview"], bool):
        return None
    return {"macosPreview": data["macosPreview"]}


def platforms_for(config: Dict[str, bool]) -> Set[str]:
    """Mac legs always build; `macosPreview: false` only keeps their assets off the release."""
    return {"windows", "macos"} if config["macosPreview"] else {"windows"}


# ---------------------------------------------------------------------------------------------
# Checksums


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def manifest_line(path: Path) -> str:
    return f"{sha256_of(path)}  {path.name}\n"


def parse_manifest(data: bytes, name: str) -> Tuple[List[Tuple[str, str]], List[str]]:
    """Strict `<64 lowercase hex>  <basename>` lines, LF only, trailing LF, no blank line."""
    errors = []
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        return [], [f"{name}: not UTF-8"]
    if "\r" in text:
        errors.append(f"{name}: must use LF line endings")
    if not text:
        return [], [f"{name}: empty"]
    if not text.endswith("\n"):
        errors.append(f"{name}: must end with a newline")
    entries = []
    for number, line in enumerate(text.rstrip("\n").split("\n"), start=1):
        m = MANIFEST_LINE.match(line.rstrip("\r"))
        if not m:
            errors.append(f"{name}: line {number} is malformed")
            continue
        entries.append((m.group(1), m.group(2)))
    return entries, errors


# ---------------------------------------------------------------------------------------------
# Binary inspection


def pe_machine(data: bytes) -> Optional[int]:
    if len(data) < 0x40 or data[:2] != b"MZ":
        return None
    (offset,) = struct.unpack_from("<I", data, 0x3C)
    if offset + 6 > len(data) or data[offset:offset + 4] != b"PE\0\0":
        return None
    (machine,) = struct.unpack_from("<H", data, offset + 4)
    return machine


def decode_macos_version(value: int) -> str:
    major, minor, patch = value >> 16, (value >> 8) & 0xFF, value & 0xFF
    return f"{major}.{minor}" + (f".{patch}" if patch else "")


def macho_info(data: bytes) -> Dict[str, object]:
    """{'archs': [...], 'minos': str | None, 'signed': bool} of a thin or universal Mach-O.
    'archs' is empty when the bytes are not Mach-O at all."""
    info: Dict[str, object] = {"archs": [], "minos": None, "signed": False}
    if len(data) < 32:
        return info
    (be_magic,) = struct.unpack_from(">I", data, 0)
    if be_magic in (FAT_MAGIC, FAT_MAGIC_64):
        (count,) = struct.unpack_from(">I", data, 4)
        names = {v: k for k, v in CPU_TYPES.items()}
        archs = []
        step = 20 if be_magic == FAT_MAGIC else 32
        for k in range(min(count, 16)):
            off = 8 + k * step
            if off + 4 > len(data):
                break
            (cputype,) = struct.unpack_from(">I", data, off)
            archs.append(names.get(cputype, f"cpu {cputype:#x}"))
        info["archs"] = ["universal"] + archs
        return info
    (magic,) = struct.unpack_from("<I", data, 0)
    if magic != MH_MAGIC_64:
        return info
    cputype, _sub, _ftype, ncmds, sizeofcmds = struct.unpack_from("<iiIII", data, 4)
    names = {v: k for k, v in CPU_TYPES.items()}
    info["archs"] = [names.get(cputype & 0xFFFFFFFF, f"cpu {cputype & 0xFFFFFFFF:#x}")]
    offset, end = 32, min(len(data), 32 + sizeofcmds)
    for _ in range(ncmds):
        if offset + 8 > end:
            break
        cmd, cmdsize = struct.unpack_from("<II", data, offset)
        if cmdsize < 8:
            break
        if cmd == LC_CODE_SIGNATURE:
            info["signed"] = True
        elif cmd == LC_BUILD_VERSION and offset + 12 <= end:
            platform, minos = struct.unpack_from("<II", data, offset + 8)
            if platform == PLATFORM_MACOS:
                info["minos"] = decode_macos_version(minos)
        elif cmd == LC_VERSION_MIN_MACOSX and offset + 12 <= end:
            (version,) = struct.unpack_from("<I", data, offset + 8)
            info["minos"] = decode_macos_version(version)
        offset += cmdsize
    return info


def is_udif(path: Path) -> bool:
    size = path.stat().st_size
    if size < 512:
        return False
    with open(path, "rb") as handle:
        handle.seek(size - 512)
        return handle.read(4) == b"koly"


def inspect_app_zip(path: Path, tag: str, version: str, slug: str) -> List[str]:
    errors = []
    try:
        archive = zipfile.ZipFile(path)
    except (zipfile.BadZipFile, OSError):
        return [f"{tag}: not a valid zip archive"]
    with archive:
        infos = {info.filename: info for info in archive.infolist()}
        for name in infos:
            parts = name.split("/")
            if name.startswith("/") or ".." in parts or parts[0] != APP_DIR:
                errors.append(f"{tag}: archive must contain only {APP_DIR}")
                break
        plist_name = f"{APP_DIR}/Contents/Info.plist"
        if plist_name not in infos:
            return errors + [f"{tag}: Info.plist missing"]
        try:
            plist = plistlib.loads(archive.read(plist_name))
        except Exception:  # plistlib raises several unrelated types on garbage
            return errors + [f"{tag}: Info.plist unreadable"]
        if not isinstance(plist, dict):
            return errors + [f"{tag}: Info.plist unreadable"]
        bundle_id = plist.get("CFBundleIdentifier")
        if bundle_id != BUNDLE_ID:
            errors.append(f"{tag}: bundle identifier is {bundle_id!r}, expected {BUNDLE_ID}")
        short = plist.get("CFBundleShortVersionString")
        if short != version:
            errors.append(f"{tag}: bundle version is {short!r}, expected {version}")
        minimum = plist.get("LSMinimumSystemVersion")
        if minimum != MIN_MACOS:
            errors.append(f"{tag}: minimum macOS version is {minimum!r}, expected {MIN_MACOS}")
        if plist.get("LSUIElement") is not True:
            errors.append(f"{tag}: LSUIElement is not set (the app must be a menu-bar app)")
        if f"{APP_DIR}/Contents/_CodeSignature/CodeResources" not in infos:
            errors.append(f"{tag}: app bundle is not signed")

        executable = plist.get("CFBundleExecutable")
        exe_name = f"{APP_DIR}/Contents/MacOS/{executable}" if isinstance(executable, str) else None
        if exe_name is None or "/" in executable or exe_name not in infos:
            return errors + [f"{tag}: bundle executable missing"]
        exe_info = infos[exe_name]
        if exe_info.file_size > MAX_EXECUTABLE_BYTES:
            return errors + [f"{tag}: bundle executable is too large"]
        if not (exe_info.external_attr >> 16) & 0o111:
            errors.append(f"{tag}: bundle executable has no execute permission")
        macho = macho_info(archive.read(exe_name))
    expected = MACHO_ARCH[slug]
    if not macho["archs"]:
        errors.append(f"{tag}: bundle executable is not a Mach-O file")
        return errors
    if macho["archs"] != [expected]:
        errors.append(f"{tag}: executable architecture is not {expected}")
        return errors
    if macho["minos"] != MIN_MACOS:
        errors.append(f"{tag}: executable deployment target is {macho['minos']!r}, expected {MIN_MACOS}")
    if not macho["signed"]:
        errors.append(f"{tag}: bundle executable has no code signature")
    return errors


# ---------------------------------------------------------------------------------------------
# Verification


def verify_assets(directory: Path, version: str, platforms, manifest: str = MANIFEST) -> List[str]:
    """Sorted fixed-text errors; empty means `directory` holds exactly the selected asset set and
    its manifest, every size is plausible, every checksum matches and every binary is the
    expected kind, architecture and bundle."""
    directory = Path(directory)
    if not VERSION_RE.match(version or ""):
        return [f"invalid version {version!r}"]
    slugs, errors = resolve_platforms(sorted(platforms))
    if not directory.is_dir():
        return sorted(errors + [f"{directory}: not a directory"])

    expected: Dict[str, str] = {}
    for slug in slugs:
        for name in asset_names(version, slug):
            expected[name] = slug
    present = {entry.name: entry for entry in directory.iterdir()}

    for name, entry in present.items():
        if name == manifest:
            continue
        if name not in expected or not entry.is_file() or entry.is_symlink():
            errors.append(f"{name}: unexpected file")

    digests: Dict[str, str] = {}
    for name, slug in expected.items():
        tag = label(version, name)
        entry = present.get(name)
        if entry is None or not entry.is_file() or entry.is_symlink():
            errors.append(f"{tag}: missing")
            continue
        size = entry.stat().st_size
        if size == 0 or size > MAX_ASSET_BYTES:
            errors.append(f"{tag}: size {size} bytes is out of range")
            continue
        digests[name] = sha256_of(entry)
        if name.endswith(".exe"):
            with open(entry, "rb") as handle:
                head = handle.read(4096)
            machine = pe_machine(head)
            if machine is None:
                errors.append(f"{tag}: not a Windows executable")
            elif machine != IMAGE_FILE_MACHINE_AMD64:
                errors.append(f"{tag}: executable architecture is not x86_64")
        elif name.endswith(".dmg"):
            if not is_udif(entry):
                errors.append(f"{tag}: not a UDIF disk image")
        elif name.endswith(".zip"):
            errors.extend(inspect_app_zip(entry, tag, version, slug))

    manifest_path = directory / manifest
    if not manifest_path.is_file():
        errors.append(f"{manifest}: missing")
    else:
        entries, manifest_errors = parse_manifest(manifest_path.read_bytes(), manifest)
        errors.extend(manifest_errors)
        seen: Set[str] = set()
        for digest, name in entries:
            if name in seen:
                errors.append(f"{manifest}: duplicate entry {name}")
                continue
            seen.add(name)
            if name not in expected:
                errors.append(f"{manifest}: unexpected entry {name}")
            elif name in digests and digests[name] != digest:
                errors.append(f"{label(version, name)}: checksum mismatch")
        for name in expected:
            if name not in seen:
                errors.append(f"{manifest}: no entry for {name}")
    return sorted(set(errors))


# ---------------------------------------------------------------------------------------------
# Publication


def assemble(artifacts: Path, version: str, platforms, out: Path) -> Tuple[List[str], Optional[str]]:
    """Copies the selected assets of each `LingoPop-<version>-<platform>` artifact directory into
    `out` and writes the combined `SHA256SUMS.txt`. Returns (sorted errors, macOS signing mode)."""
    slugs, errors = resolve_platforms(sorted(platforms))
    if errors:
        return sorted(errors), None
    out = Path(out)
    if out.exists() and any(out.iterdir()):
        return [f"{out}: output directory is not empty"], None
    out.mkdir(parents=True, exist_ok=True)
    lines: List[str] = []
    copied: Set[str] = set()
    modes: Set[str] = set()
    for slug in slugs:
        source = Path(artifacts) / artifact_name(version, slug)
        if not source.is_dir():
            errors.append(f"artifact {artifact_name(version, slug)}: missing")
            continue
        names = asset_names(version, slug)
        part = f"SHA256SUMS-{slug}.txt"
        part_path = source / part
        if not part_path.is_file():
            errors.append(f"{part}: missing")
        else:
            entries, part_errors = parse_manifest(part_path.read_bytes(), part)
            errors.extend(part_errors)
            for digest, name in entries:
                if name not in names:
                    errors.append(f"{part}: unexpected entry {name}")
                lines.append(f"{digest}  {name}\n")
        for name in names:
            src = source / name
            if name in copied:
                errors.append(f"duplicate asset {name} in the artifacts")
                continue
            if not src.is_file() or src.is_symlink():
                errors.append(f"{label(version, name)}: missing")
                continue
            shutil.copyfile(src, out / name)
            copied.add(name)
        if slug.startswith("macos-"):
            mode_path = source / f"signing-{slug}.txt"
            try:
                mode = mode_path.read_text(encoding="utf-8").strip()
            except OSError:
                mode = None
            if mode not in SIGNING_MODES:
                errors.append(f"signing-{slug}.txt: missing or invalid")
            else:
                modes.add(mode)
    if len(modes) > 1:
        errors.append("the macOS legs disagree on the signing mode")
    (out / MANIFEST).write_text("".join(lines), encoding="utf-8", newline="\n")
    mode = next(iter(modes)) if len(modes) == 1 and not errors else None
    return sorted(set(errors)), mode


CONDITION = re.compile(r"^<!--\s*if\s+([a-z-]+)\s*-->$")
END_CONDITION = re.compile(r"^<!--\s*endif\s*-->$")
NOTES_START = "<!-- release-notes-start -->"
CONDITIONS = ("windows", "macos", "macos-adhoc")


def render_install(template: str, conditions: Set[str]) -> Tuple[str, List[str]]:
    """The part of `docs/INSTALL.md` after `<!-- release-notes-start -->` with each
    `<!-- if NAME -->` ... `<!-- endif -->` block kept only when NAME is true (blocks nest;
    NAME is `windows`, `macos` or `macos-adhoc`). `render_notes` then replaces `<version>`."""
    if NOTES_START not in template:
        return "", [f"install template: {NOTES_START} marker missing"]
    body = template.split(NOTES_START, 1)[1]
    stack: List[bool] = []
    kept: List[str] = []
    errors = []
    for number, line in enumerate(body.split("\n"), start=1):
        stripped = line.strip()
        m = CONDITION.match(stripped)
        if m:
            if m.group(1) not in CONDITIONS:
                errors.append(f"install template: unknown condition {m.group(1)!r}")
            stack.append(m.group(1) in conditions)
            continue
        if END_CONDITION.match(stripped):
            if not stack:
                errors.append("install template: endif without if")
            else:
                stack.pop()
            continue
        if stripped.startswith("<!--") and ("if " in stripped or "endif" in stripped):
            errors.append(f"install template: malformed marker {stripped!r}")
            continue
        if all(stack):
            kept.append(line)
    if stack:
        errors.append("install template: if without endif")
    text = re.sub(r"\n{3,}", "\n\n", "\n".join(kept)).strip() + "\n"
    return text, errors


def changelog_section(changelog: str, version: str) -> str:
    lines, found = [], False
    for line in changelog.split("\n"):
        if line.startswith("## "):
            if found:
                break
            if line.startswith(f"## [{version}]"):
                found = True
                continue
        if found:
            lines.append(line)
    return "\n".join(lines).strip()


def render_notes(changelog: str, template: str, version: str, platforms, mode: Optional[str]) -> Tuple[str, List[str]]:
    slugs, errors = resolve_platforms(sorted(platforms))
    conditions = set()
    if "windows-x64" in slugs:
        conditions.add("windows")
    if any(s.startswith("macos-") for s in slugs):
        conditions.add("macos")
        if mode == "adhoc":
            conditions.add("macos-adhoc")
        elif mode != "developer-id":
            errors.append("macOS signing mode unknown")
    install, install_errors = render_install(template, conditions)
    install = install.replace("<version>", version)
    section = changelog_section(changelog, version) or f"Test pre-release {version} of LingoPop."
    return f"{section}\n\n{install}", errors + install_errors


def publish(
    artifacts: Path,
    version: str,
    platforms_file: Path,
    out: Path,
    changelog: Path,
    install: Path,
    notes_out: Path,
    command: Sequence[str],
    runner: Callable = subprocess.run,
) -> List[str]:
    """Returns the sorted errors. `runner(command + asset paths, check=True)` is called exactly
    once, and only when there are none."""
    if not command:
        return ["no publish command given"]
    config = read_release_platforms(platforms_file)
    if config is None:
        return [f"{Path(platforms_file).name}: missing or malformed (expected {{\"macosPreview\": true|false}})"]
    platforms = platforms_for(config)
    errors, mode = assemble(artifacts, version, platforms, out)
    if errors:
        return errors
    errors = verify_assets(out, version, platforms)
    if errors:
        return errors
    try:
        notes, errors = render_notes(
            Path(changelog).read_text(encoding="utf-8"),
            Path(install).read_text(encoding="utf-8"),
            version,
            platforms,
            mode,
        )
    except OSError as exc:
        return [f"release notes: {exc.__class__.__name__}"]
    if errors:
        return sorted(errors)
    Path(notes_out).write_text(notes, encoding="utf-8", newline="\n")
    slugs, _ = resolve_platforms(sorted(platforms))
    files = [str(Path(out) / name) for slug in slugs for name in asset_names(version, slug)]
    files.append(str(Path(out) / MANIFEST))
    runner(list(command) + files, check=True)
    return []


# ---------------------------------------------------------------------------------------------
# CLI


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = parser.add_subparsers(dest="action", required=True)
    v = sub.add_parser("verify", help="verify one directory of assets")
    v.add_argument("--version", required=True)
    v.add_argument("--platforms", required=True, help="comma list: windows, macos, windows-x64, macos-arm64, macos-x64")
    v.add_argument("--manifest", default=MANIFEST)
    v.add_argument("directory")
    p = sub.add_parser("publish", help="assemble, verify, write notes, then run the command")
    p.add_argument("--version", required=True)
    p.add_argument("--platforms-file", required=True)
    p.add_argument("--artifacts", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--changelog", required=True)
    p.add_argument("--install", required=True)
    p.add_argument("--notes-out", required=True)
    p.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args(argv)

    if args.action == "verify":
        platforms = {p for p in args.platforms.split(",") if p}
        errors = verify_assets(Path(args.directory), args.version, platforms, args.manifest)
    else:
        command = args.command[1:] if args.command[:1] == ["--"] else args.command
        errors = publish(
            Path(args.artifacts),
            args.version,
            Path(args.platforms_file),
            Path(args.out),
            Path(args.changelog),
            Path(args.install),
            Path(args.notes_out),
            command,
        )
    for error in errors:
        print(f"error: {error}", file=sys.stderr)
    if errors:
        return 1
    print("release assets: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
