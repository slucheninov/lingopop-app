#!/usr/bin/env python3
"""UI catalogs (pl / uk / de) of LingoPop, owned by this repository.

Files under i18n/:
  catalogs/{pl,uk,de}.json   shared: every key shown on both platforms (flat `key -> value`)
  windows.json               Windows-only keys (`key -> {pl, uk, de}`)
  macos.json                 macOS-only keys (`key -> {pl, uk, de}`)
  import-skip.txt            Swift keys `generate` must not import (one per line, `\\n` = newline)

Usage:
  gen-localizations.py check      no Swift checkout, no network (CI): validate every catalog
  gen-localizations.py generate   optional, macOS / dev only: read the Swift App/Localizable.xcstrings
                                  ($LINGOPOP_MAC_REPO, default ~/Works/slava/lingo-pop) and ADD the keys
                                  missing from every repo catalog to the shared catalogs; an existing
                                  value is never overwritten

`check` prints one `KEY | lang | problem` line per issue (sorted) and exits 1, or on success a single
`ok: <n> shared keys, <w> windows keys, <m> macos keys` line.

The signature rule is the one of the mac repo's scripts/check-localizations.py: the specifier
multiset, the newline count and the counts of the tokens below must match the key. `⌘` is
additionally forbidden in shared and Windows values. A key may be in the shared catalogs or in
platform files, never both; the same key may be in both windows.json and macos.json.
Standard library only (Python 3.9+).
"""
import json
import os
import re
import sys
from collections import Counter
from pathlib import Path

LANGS = ("pl", "uk", "de")
PLATFORMS = ("windows", "macos")
SPEC_RE = re.compile(r"%(?:\d+\$)?(?:lld|ld|d|@|%)")
TOKENS = ("…", "→", "⌘", "{{targetLanguage}}")

ROOT = Path(__file__).resolve().parent.parent


def signature(text):
    return Counter(SPEC_RE.findall(text)), text.count("\n"), tuple(text.count(t) for t in TOKENS)


def describe(sig):
    specs, newlines, tokens = sig
    parts = ["specs=%s" % (sorted(specs.elements()),), "newlines=%d" % newlines]
    parts += ["%r=%d" % (t, n) for t, n in zip(TOKENS, tokens)]
    return " ".join(parts)


def report_key(key):
    return key.replace("\n", "\\n")


def paths(root):
    i18n = Path(root) / "i18n"
    return {
        "shared": {lang: i18n / "catalogs" / ("%s.json" % lang) for lang in LANGS},
        "windows": i18n / "windows.json",
        "macos": i18n / "macos.json",
        "skip": i18n / "import-skip.txt",
    }


def read_json(path):
    with open(path, encoding="utf-8") as handle:
        return json.load(handle)


def write_json(path, value):
    text = json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True)
    with open(path, "w", encoding="utf-8", newline="\n") as handle:
        handle.write(text + "\n")


def load_skip(root):
    """One key per line; a newline inside a key is written as the two characters `\\n`."""
    path = paths(root)["skip"]
    if not path.is_file():
        return []
    with open(path, encoding="utf-8") as handle:
        lines = [line.rstrip("\n") for line in handle]
    return [line.replace("\\n", "\n") for line in lines if line]


def load_shared(root):
    return {lang: read_json(path) for lang, path in paths(root)["shared"].items()}


def load_platform(root, platform):
    path = paths(root)[platform]
    return read_json(path) if path.is_file() else {}


def merged_catalog(root, platform, lang):
    """The catalog the app resolves on `platform` for `lang`: shared plus that platform's file."""
    merged = dict(load_shared(root)[lang])
    for key, values in load_platform(root, platform).items():
        if lang in values:
            merged.setdefault(key, values[lang])
    return merged


def check_catalogs(root):
    """Validation issues of the catalogs under `root`, as sorted `KEY | lang | problem` lines."""
    problems = []

    def add(key, lang, problem):
        problems.append("%s | %s | %s" % (report_key(key), lang, problem))

    def compare(key, lang, value, where, allow_command):
        if not isinstance(value, str):
            add(key, lang, "value is not a string")
            return
        want, got = signature(key), signature(value)
        if want != got:
            add(key, lang, "mismatch: expected {%s} got {%s}" % (describe(want), describe(got)))
        if not allow_command and "⌘" in value:
            add(key, lang, "⌘ is not allowed in a %s value" % where)

    shared = load_shared(root)
    platform_files = {name: load_platform(root, name) for name in PLATFORMS}

    for lang in LANGS:
        for key, value in shared[lang].items():
            compare(key, lang, value, "shared", False)
            for name, entries in platform_files.items():
                if key in entries:
                    add(key, lang, "key appears in the shared catalogs and in %s.json" % name)
        for key in shared["pl"].keys() ^ shared[lang].keys():
            add(key, lang, "key is not present in all shared languages")

    for name, entries in platform_files.items():
        for key, values in entries.items():
            if not isinstance(values, dict):
                add(key, "-", "%s.json entry is not an object" % name)
                continue
            for lang in LANGS:
                if lang not in values:
                    add(key, lang, "missing translation")
                else:
                    compare(key, lang, values[lang], name.capitalize(), name == "macos")
            for lang in values:
                if lang not in LANGS:
                    add(key, lang, "unexpected language")

    everywhere = set(platform_files["windows"]) | set(platform_files["macos"])
    for lang in LANGS:
        everywhere |= set(shared[lang])
    for key in load_skip(root):
        if key in everywhere:
            add(key, "-", "key is in import-skip.txt and in a catalog")

    return sorted(set(problems))


def mac_repo():
    return Path(os.environ.get("LINGOPOP_MAC_REPO", "~/Works/slava/lingo-pop")).expanduser()


def import_missing(root, repo):
    """Adds Swift keys missing from every repo catalog to the shared catalogs, never overwriting.

    Returns `(problems, added)`. With any problem nothing is written. Skipped: `shouldTranslate:
    false`, keys listed in import-skip.txt and keys already present in any repo catalog in any
    language (a key missing from only some languages is an inconsistent catalog for `check` to
    report, never something the import fills in over existing values).
    """
    strings = read_json(Path(repo) / "App" / "Localizable.xcstrings").get("strings", {})
    shared = load_shared(root)
    known = set()
    for lang in LANGS:
        known |= set(shared[lang])
    for name in PLATFORMS:
        known |= set(load_platform(root, name))
    skipped = set(load_skip(root))
    problems = []
    additions = {lang: {} for lang in LANGS}
    added = []
    for key in sorted(strings):
        entry = strings[key]
        if entry.get("shouldTranslate") is False or key in skipped or key in known:
            continue
        locs = entry.get("localizations", {})
        values = {}
        for lang in LANGS:
            unit = locs.get(lang, {}).get("stringUnit")
            if unit is None:
                problems.append("%s | %s | missing translation" % (report_key(key), lang))
            elif unit.get("state") != "translated":
                problems.append(
                    "%s | %s | state is %r, expected 'translated'" % (report_key(key), lang, unit.get("state"))
                )
            else:
                values[lang] = unit["value"]
        if len(values) < len(LANGS):
            continue
        for lang in LANGS:
            value = values[lang]
            if signature(key) != signature(value):
                problems.append("%s | %s | signature mismatch" % (report_key(key), lang))
            elif "⌘" in value:
                problems.append(
                    "%s | %s | ⌘ value: add it to macos.json or import-skip.txt" % (report_key(key), lang)
                )
        if not any(p.startswith(report_key(key) + " | ") for p in problems):
            for lang in LANGS:
                additions[lang][key] = values[lang]
            added.append(key)
    if problems:
        return sorted(problems), []
    for lang in LANGS:
        if additions[lang]:
            merged = dict(shared[lang])
            for key, value in additions[lang].items():
                merged.setdefault(key, value)  # never overwrite an existing value
            write_json(paths(root)["shared"][lang], merged)
    return [], added


def generate():
    repo = mac_repo()
    catalog_path = repo / "App" / "Localizable.xcstrings"
    if not catalog_path.is_file():
        print("mac catalog not found: %s (set LINGOPOP_MAC_REPO)" % catalog_path, file=sys.stderr)
        return 1
    strings = read_json(catalog_path).get("strings", {})
    for key in load_skip(ROOT):
        if key not in strings:
            print("warning: import-skip key not in the mac catalog: %s" % report_key(key), file=sys.stderr)
    problems, added = import_missing(ROOT, repo)
    for line in problems:
        print(line)
    if problems:
        return 1
    for key in added:
        print("added: %s" % report_key(key))
    print("imported %d new key(s) x %d languages into i18n/catalogs" % (len(added), len(LANGS)))
    return 0


def check():
    issues = check_catalogs(ROOT)
    for line in issues:
        print(line)
    if issues:
        return 1
    print(
        "ok: %d shared keys, %d windows keys, %d macos keys"
        % (
            len(load_shared(ROOT)["pl"]),
            len(load_platform(ROOT, "windows")),
            len(load_platform(ROOT, "macos")),
        )
    )
    return 0


def main(argv):
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
        sys.stderr.reconfigure(encoding="utf-8")
    commands = {"generate": generate, "check": check}
    if len(argv) != 2 or argv[1] not in commands:
        print("usage: gen-localizations.py generate|check", file=sys.stderr)
        return 2
    return commands[argv[1]]()


if __name__ == "__main__":
    sys.exit(main(sys.argv))
