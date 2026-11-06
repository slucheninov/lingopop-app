#!/usr/bin/env python3
"""Shared version check for CI and every release leg.

    python3 scripts/check-version.py [--tag vX.Y.Z] [--root DIR]

The one app version lives in four places that must always be equal:

- `Cargo.toml` `[workspace.package]` `version`
- `app/src-tauri/tauri.conf.json` `version`
- `app/package.json` `version`
- `app/package-lock.json` `version` and `packages[""].version`

`app/src-tauri/tauri.macos.conf.json` must not contain a `version` key at any level (the Mac
override would silently replace the shared version). With `--tag` the tag must be `v<version>`.

On success the version is printed alone on stdout (a workflow can capture it) and the exit code
is 0. Otherwise every problem is printed to stderr and the exit code is 1. Python standard
library only, 3.9+.
"""
import argparse
import json
import re
import sys
from pathlib import Path
from typing import List, Optional, Tuple

CARGO = "Cargo.toml"
TAURI = "app/src-tauri/tauri.conf.json"
TAURI_MACOS = "app/src-tauri/tauri.macos.conf.json"
PACKAGE = "app/package.json"
LOCK = "app/package-lock.json"

# SemVer 2.0 core with an optional pre-release (no build metadata: tags and asset names use it).
VERSION_RE = re.compile(
    r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$"
)


def cargo_workspace_version(text: str) -> Optional[str]:
    """The `version` string of the `[workspace.package]` table, or None.

    A deliberately small reader (no tomllib before Python 3.11): it tracks table headers and
    reads only a plain `version = "..."` line inside that one table."""
    section = None
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        header = re.match(r"^\[\s*([^\[\]]+?)\s*\]\s*(#.*)?$", line)
        if header:
            section = header.group(1)
            continue
        if line.startswith("[["):
            section = None
            continue
        if section == "workspace.package":
            m = re.match(r'^version\s*=\s*"([^"\\]*)"\s*(#.*)?$', line)
            if m:
                return m.group(1)
    return None


def _load_json(root: Path, rel: str, errors: List[str]):
    path = root / rel
    try:
        text = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        errors.append(f"{rel}: missing")
        return None
    except OSError as exc:
        errors.append(f"{rel}: unreadable ({exc.__class__.__name__})")
        return None
    try:
        return json.loads(text)
    except ValueError:
        errors.append(f"{rel}: invalid JSON")
        return None


def _string_at(data, keys: List[str]) -> Optional[str]:
    node = data
    for key in keys:
        if not isinstance(node, dict):
            return None
        node = node.get(key)
    return node if isinstance(node, str) else None


def _has_version_key(node) -> bool:
    if isinstance(node, dict):
        return "version" in node or any(_has_version_key(v) for v in node.values())
    if isinstance(node, list):
        return any(_has_version_key(v) for v in node)
    return False


def check(root: Path, tag: Optional[str] = None) -> Tuple[Optional[str], List[str]]:
    """Returns (version, errors). `version` is the agreed version only when `errors` is empty."""
    errors: List[str] = []
    found: List[Tuple[str, Optional[str]]] = []

    try:
        cargo_text = (root / CARGO).read_text(encoding="utf-8")
    except FileNotFoundError:
        errors.append(f"{CARGO}: missing")
        cargo_text = None
    if cargo_text is not None:
        found.append((f"{CARGO} [workspace.package] version", cargo_workspace_version(cargo_text)))

    tauri = _load_json(root, TAURI, errors)
    if tauri is not None:
        found.append((f"{TAURI} version", _string_at(tauri, ["version"])))
    package = _load_json(root, PACKAGE, errors)
    if package is not None:
        found.append((f"{PACKAGE} version", _string_at(package, ["version"])))
    lock = _load_json(root, LOCK, errors)
    if lock is not None:
        found.append((f"{LOCK} version", _string_at(lock, ["version"])))
        found.append((f'{LOCK} packages[""].version', _string_at(lock, ["packages", "", "version"])))

    macos = _load_json(root, TAURI_MACOS, errors)
    if macos is not None and _has_version_key(macos):
        errors.append(f"{TAURI_MACOS}: must not contain a version key")

    for label, value in found:
        if value is None:
            errors.append(f"{label}: missing")
        elif not VERSION_RE.match(value):
            errors.append(f"{label}: {value!r} is not a valid version")

    values = {value for _, value in found if value is not None}
    version = None
    if len(values) > 1:
        listing = ", ".join(f"{label} = {value}" for label, value in found if value is not None)
        errors.append(f"versions differ: {listing}")
    elif len(values) == 1:
        version = next(iter(values))

    if tag is not None and version is not None:
        if not tag.startswith("v"):
            errors.append(f"tag {tag} must start with v")
        elif tag[1:] != version:
            errors.append(f"tag {tag} does not match the app version {version} (expected v{version})")

    errors.sort()
    return (version if not errors else None), errors


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Check that every app version agrees.")
    parser.add_argument("--tag", help="release tag; must equal v<version>")
    parser.add_argument("--root", default=str(Path(__file__).resolve().parents[1]))
    args = parser.parse_args(argv)
    version, errors = check(Path(args.root), args.tag)
    if errors:
        for error in errors:
            print(f"error: {error}", file=sys.stderr)
        return 1
    print(version)
    return 0


if __name__ == "__main__":
    sys.exit(main())
