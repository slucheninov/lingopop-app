#!/usr/bin/env python3
"""Generated UI catalogs (pl / uk / de) for LingoPop for Windows.

Usage:
  gen-localizations.py generate   macOS / dev only: read the mac App/Localizable.xcstrings
                                  ($LINGOPOP_MAC_REPO, default ~/Works/slava/lingo-pop) and write
                                  i18n/generated/{pl,uk,de}.json and i18n/SOURCE.txt
  gen-localizations.py check      no mac repo needed (CI): validate the committed catalogs

`check` prints one `KEY | lang | problem` line per issue and exits 1, or on success a single
`ok: <n> generated keys, <m> windows keys, <x> excluded` line.

The signature rule is the one of the mac repo's scripts/check-localizations.py: the specifier
multiset, the newline count and the counts of the tokens below must match the key. `⌘` is
additionally forbidden in every Windows-only value. Standard library only (Python 3.9+).
"""
import json
import os
import re
import subprocess
import sys
from collections import Counter
from pathlib import Path

LANGS = ("pl", "uk", "de")
SPEC_RE = re.compile(r"%(?:\d+\$)?(?:lld|ld|d|@|%)")
TOKENS = ("…", "→", "⌘", "{{targetLanguage}}")

ROOT = Path(__file__).resolve().parent.parent
I18N = ROOT / "i18n"
GENERATED = I18N / "generated"
EXCLUSIONS = I18N / "exclusions.txt"
WINDOWS = I18N / "windows.json"
SOURCE = I18N / "SOURCE.txt"


def signature(text):
    return Counter(SPEC_RE.findall(text)), text.count("\n"), tuple(text.count(t) for t in TOKENS)


def describe(sig):
    specs, newlines, tokens = sig
    parts = ["specs=%s" % (sorted(specs.elements()),), "newlines=%d" % newlines]
    parts += ["%r=%d" % (t, n) for t, n in zip(TOKENS, tokens)]
    return " ".join(parts)


def report_key(key):
    return key.replace("\n", "\\n")


def load_exclusions():
    """One key per line; a newline inside a key is written as the two characters `\\n`."""
    with open(EXCLUSIONS, encoding="utf-8") as handle:
        lines = [line.rstrip("\n") for line in handle]
    return [line.replace("\\n", "\n") for line in lines if line]


def read_json(path):
    with open(path, encoding="utf-8") as handle:
        return json.load(handle)


def write_json(path, value):
    text = json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True)
    with open(path, "w", encoding="utf-8", newline="\n") as handle:
        handle.write(text + "\n")


def mac_repo():
    return Path(os.environ.get("LINGOPOP_MAC_REPO", "~/Works/slava/lingo-pop")).expanduser()


def generate():
    repo = mac_repo()
    catalog_path = repo / "App" / "Localizable.xcstrings"
    if not catalog_path.is_file():
        print("mac catalog not found: %s (set LINGOPOP_MAC_REPO)" % catalog_path, file=sys.stderr)
        return 1
    strings = read_json(catalog_path).get("strings", {})
    excluded = load_exclusions()
    problems = []
    for key in excluded:
        if key not in strings:
            problems.append((key, "-", "excluded key is not in the mac catalog (stale exclusion)"))
    excluded_set = set(excluded)
    out = {lang: {} for lang in LANGS}
    for key, entry in strings.items():
        if entry.get("shouldTranslate") is False or key in excluded_set:
            continue
        locs = entry.get("localizations", {})
        for lang in LANGS:
            unit = locs.get(lang, {}).get("stringUnit")
            if unit is None:
                problems.append((key, lang, "missing translation"))
                continue
            if unit.get("state") != "translated":
                problems.append((key, lang, "state is %r, expected 'translated'" % unit.get("state")))
                continue
            out[lang][key] = unit["value"]
    for key, lang, problem in problems:
        print("%s | %s | %s" % (report_key(key), lang, problem))
    if problems:
        return 1
    GENERATED.mkdir(parents=True, exist_ok=True)
    for lang in LANGS:
        write_json(GENERATED / ("%s.json" % lang), out[lang])
    head = subprocess.run(
        ["git", "-C", str(repo), "rev-parse", "HEAD"], check=True, capture_output=True, text=True
    ).stdout.strip()
    with open(SOURCE, "w", encoding="utf-8", newline="\n") as handle:
        handle.write(head + "\n")
    print("generated %d keys x %d languages from %s" % (len(out["pl"]), len(LANGS), head))
    return 0


def check():
    problems = []

    def add(key, lang, problem):
        problems.append((report_key(key), lang, problem))

    def compare(key, lang, value, windows_only):
        if not isinstance(value, str):
            add(key, lang, "value is not a string")
            return
        want, got = signature(key), signature(value)
        if want != got:
            add(key, lang, "mismatch: expected {%s} got {%s}" % (describe(want), describe(got)))
        if windows_only and "⌘" in value:
            add(key, lang, "⌘ is not allowed in a Windows value")

    generated = {lang: read_json(GENERATED / ("%s.json" % lang)) for lang in LANGS}
    windows = read_json(WINDOWS)
    excluded = load_exclusions()

    for lang in LANGS:
        for key, value in generated[lang].items():
            compare(key, lang, value, False)
            if key in excluded:
                add(key, lang, "excluded key appears in a generated file")
            if key in windows:
                add(key, lang, "key appears in windows.json and in a generated file")
    for lang in LANGS:
        for key in generated["pl"].keys() ^ generated[lang].keys():
            add(key, lang, "key is not present in all generated languages")

    for key, values in windows.items():
        if not isinstance(values, dict):
            add(key, "-", "windows.json entry is not an object")
            continue
        for lang in LANGS:
            if lang not in values:
                add(key, lang, "missing translation")
            else:
                compare(key, lang, values[lang], True)
        for lang in values:
            if lang not in LANGS:
                add(key, lang, "unexpected language")

    for key, lang, problem in problems:
        print("%s | %s | %s" % (key, lang, problem))
    if problems:
        return 1
    print(
        "ok: %d generated keys, %d windows keys, %d excluded"
        % (len(generated["pl"]), len(windows), len(excluded))
    )
    return 0


def main(argv):
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
        sys.stderr.reconfigure(encoding="utf-8")
    commands = {"generate": generate, "check": check}
    if len(argv) != 2 or argv[1] not in commands:
        print(__doc__.strip().splitlines()[2], file=sys.stderr)
        print("usage: gen-localizations.py generate|check", file=sys.stderr)
        return 2
    return commands[argv[1]]()


if __name__ == "__main__":
    sys.exit(main(sys.argv))
