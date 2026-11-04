"""Tests for scripts/gen-localizations.py (catalog ownership, validation, add-only import).

Run: python3 -m unittest discover -s scripts/tests -p 'test_localizations.py'
No Swift checkout and no network are needed; `LINGOPOP_MAC_REPO` points at a nonexistent path.
"""
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "gen-localizations.py"
REPO_ROOT = SCRIPT.parent.parent
LANGS = ("pl", "uk", "de")

spec = importlib.util.spec_from_file_location("gen_localizations", SCRIPT)
gen = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gen)


def triple(pl, uk=None, de=None):
    return {"pl": pl, "uk": uk or pl, "de": de or pl}


def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2), encoding="utf-8")


def make_root(shared=None, windows=None, macos=None, skip=()):
    """A temp repo root. `shared` is `key -> {pl, uk, de}`; the per-language files are derived."""
    tmp = tempfile.TemporaryDirectory()
    root = Path(tmp.name)
    shared = shared or {}
    for lang in LANGS:
        write(root / "i18n" / "catalogs" / ("%s.json" % lang), {k: v[lang] for k, v in shared.items()})
    write(root / "i18n" / "windows.json", windows or {})
    write(root / "i18n" / "macos.json", macos or {})
    (root / "i18n" / "import-skip.txt").write_text("".join(k + "\n" for k in skip), encoding="utf-8")
    return tmp, root


class CheckCatalogs(unittest.TestCase):
    def check(self, **kwargs):
        tmp, root = make_root(**kwargs)
        self.addCleanup(tmp.cleanup)
        return gen.check_catalogs(root)

    def test_valid_catalogs_have_no_issues(self):
        issues = self.check(
            shared={"Hello %@": triple("Cześć %@")},
            windows={"Press Ctrl+V.": triple("Naciśnij Ctrl+V.")},
            macos={"Press ⌘V.": triple("Naciśnij ⌘V.")},
        )
        self.assertEqual(issues, [])

    def test_windows_rejects_command_symbols_but_macos_accepts_them(self):
        key = "Press ⌘V to paste."
        values = triple("Naciśnij ⌘V, aby wkleić.", "Натисніть ⌘V.", "Drücke ⌘V.")
        tmp_w, root_w = make_root(windows={key: values})
        tmp_m, root_m = make_root(macos={key: values})
        self.addCleanup(tmp_w.cleanup)
        self.addCleanup(tmp_m.cleanup)
        windows_issues = gen.check_catalogs(root_w)
        macos_issues = gen.check_catalogs(root_m)
        self.assertNotEqual(windows_issues, [])
        self.assertEqual(macos_issues, [])

        # Adding the macos copy never changes what Windows resolves.
        tmp_b, root_b = make_root(windows={key: values})
        self.addCleanup(tmp_b.cleanup)
        merged_windows_before = {lang: gen.merged_catalog(root_b, "windows", lang) for lang in LANGS}
        write(root_b / "i18n" / "macos.json", {key: values})
        merged_windows_after = {lang: gen.merged_catalog(root_b, "windows", lang) for lang in LANGS}
        self.assertEqual(merged_windows_before, merged_windows_after)
        self.assertEqual(gen.check_catalogs(root_b) != [], True)  # the Windows copy still fails

    def test_command_symbol_in_shared_value_fails(self):
        key = "Press ⌘V to paste."
        issues = self.check(shared={key: triple("Naciśnij ⌘V.")})
        self.assertTrue(any("⌘" in line for line in issues), issues)

    def test_bad_signatures_fail_for_every_catalog(self):
        cases = {
            "specifier": ("Hello %1$@ and %2$@", "Cześć %1$@"),
            "newline": ("Line one\nLine two", "Linia jeden Linia dwa"),
            "target language": ("Say {{targetLanguage}}", "Powiedz coś"),
        }
        for name, (key, bad) in cases.items():
            for where in ("shared", "windows", "macos"):
                issues = self.check(**{where: {key: triple(bad)}})
                self.assertNotEqual(issues, [], "%s in %s" % (name, where))

    def test_key_in_shared_and_a_platform_file_fails(self):
        for where in ("windows", "macos"):
            issues = self.check(shared={"Open": triple("Otwórz")}, **{where: {"Open": triple("Otwórz")}})
            self.assertTrue(any("shared" in line for line in issues), (where, issues))

    def test_same_key_in_windows_and_macos_is_allowed(self):
        issues = self.check(windows={"Open": triple("Otwórz")}, macos={"Open": triple("Otwórz")})
        self.assertEqual(issues, [])

    def test_missing_language_and_unexpected_language_fail(self):
        issues = self.check(windows={"Open": {"pl": "Otwórz", "uk": "Відкрити"}})
        self.assertTrue(any("| de |" in line for line in issues), issues)
        issues = self.check(macos={"Open": {**triple("Otwórz"), "fr": "Ouvrir"}})
        self.assertTrue(any("| fr |" in line for line in issues), issues)

    def test_shared_languages_must_have_the_same_keys(self):
        tmp, root = make_root(shared={"Open": triple("Otwórz")})
        self.addCleanup(tmp.cleanup)
        write(root / "i18n" / "catalogs" / "de.json", {})
        self.assertTrue(any("not present in all" in line for line in gen.check_catalogs(root)))

    def test_issues_are_deterministic_and_sorted(self):
        kwargs = dict(
            shared={"B %@": triple("B"), "A %@": triple("A")},
            windows={"C %@": triple("C")},
        )
        first, second = self.check(**kwargs), self.check(**kwargs)
        self.assertEqual(first, second)
        self.assertEqual(first, sorted(first))

    def test_skipped_import_key_must_not_be_in_a_catalog(self):
        issues = self.check(shared={"Open": triple("Otwórz")}, skip=("Open",))
        self.assertTrue(any("import-skip" in line for line in issues), issues)


class MergedCatalog(unittest.TestCase):
    def test_platforms_compose_shared_with_their_own_file(self):
        tmp, root = make_root(
            shared={"Open": triple("Otwórz")},
            windows={"Press Ctrl+V.": triple("Ctrl+V")},
            macos={"Press ⌘V.": triple("⌘V")},
        )
        self.addCleanup(tmp.cleanup)
        windows = gen.merged_catalog(root, "windows", "pl")
        macos = gen.merged_catalog(root, "macos", "pl")
        self.assertEqual(set(windows), {"Open", "Press Ctrl+V."})
        self.assertEqual(set(macos), {"Open", "Press ⌘V."})


class RepositoryCatalogs(unittest.TestCase):
    def env(self):
        env = dict(os.environ)
        env["LINGOPOP_MAC_REPO"] = "/nonexistent/lingopop-mac-repo"
        for name in ("HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"):
            env[name] = "http://127.0.0.1:9"
        return env

    def run_cli(self, *args):
        return subprocess.run(
            [sys.executable, str(SCRIPT), *args],
            capture_output=True,
            text=True,
            encoding="utf-8",
            env=self.env(),
            cwd=str(REPO_ROOT),
        )

    def test_committed_catalogs_pass_without_a_swift_checkout(self):
        self.assertEqual(gen.check_catalogs(REPO_ROOT), [])
        result = self.run_cli("check")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue(result.stdout.startswith("ok:"), result.stdout)

    def test_generate_without_a_swift_checkout_fails_and_changes_nothing(self):
        before = (REPO_ROOT / "i18n" / "catalogs" / "pl.json").read_bytes()
        result = self.run_cli("generate")
        self.assertEqual(result.returncode, 1)
        self.assertIn("mac catalog not found", result.stderr)
        self.assertEqual(before, (REPO_ROOT / "i18n" / "catalogs" / "pl.json").read_bytes())


def xcstrings(entries):
    strings = {}
    for key, values in entries.items():
        locs = {lang: {"stringUnit": {"state": "translated", "value": values[lang]}} for lang in values}
        strings[key] = {"localizations": locs}
    return {"strings": strings}


class AddOnlyImport(unittest.TestCase):
    def setUp(self):
        self.mac = tempfile.TemporaryDirectory()
        self.addCleanup(self.mac.cleanup)

    def write_mac(self, entries, extra=None):
        catalog = xcstrings(entries)
        catalog["strings"].update(extra or {})
        write(Path(self.mac.name) / "App" / "Localizable.xcstrings", catalog)

    def test_adds_missing_keys_to_shared_and_never_overwrites(self):
        tmp, root = make_root(shared={"Open": triple("Otwórz")})
        self.addCleanup(tmp.cleanup)
        self.write_mac(
            {
                "Open": triple("SWIFT-PL", "SWIFT-UK", "SWIFT-DE"),
                "Close %@": triple("Zamknij %@", "Закрити %@", "Schließen %@"),
            }
        )
        problems, added = gen.import_missing(root, Path(self.mac.name))
        self.assertEqual(problems, [])
        self.assertEqual(added, ["Close %@"])
        pl = json.loads((root / "i18n" / "catalogs" / "pl.json").read_text(encoding="utf-8"))
        self.assertEqual(pl["Open"], "Otwórz")
        self.assertEqual(pl["Close %@"], "Zamknij %@")
        self.assertEqual(gen.check_catalogs(root), [])

    def test_keys_in_a_platform_file_or_the_skip_list_are_not_imported(self):
        tmp, root = make_root(
            windows={"Windows only": triple("W")},
            macos={"Press ⌘V.": triple("⌘V")},
            skip=("iCloud",),
        )
        self.addCleanup(tmp.cleanup)
        self.write_mac(
            {
                "Windows only": triple("x"),
                "Press ⌘V.": triple("⌘V"),
                "iCloud": triple("iCloud"),
            }
        )
        problems, added = gen.import_missing(root, Path(self.mac.name))
        self.assertEqual((problems, added), ([], []))

    def test_untranslated_or_invalid_keys_are_reported_and_nothing_is_written(self):
        tmp, root = make_root(shared={"Open": triple("Otwórz")})
        self.addCleanup(tmp.cleanup)
        self.write_mac(
            {"Fine": triple("Dobrze")},
            extra={
                "Half": {"localizations": {"pl": {"stringUnit": {"state": "translated", "value": "x"}}}},
                "Bad %@": xcstrings({"Bad %@": triple("Zły")})["strings"]["Bad %@"],
            },
        )
        before = (root / "i18n" / "catalogs" / "pl.json").read_bytes()
        problems, added = gen.import_missing(root, Path(self.mac.name))
        self.assertNotEqual(problems, [])
        self.assertEqual(added, [])
        self.assertEqual(before, (root / "i18n" / "catalogs" / "pl.json").read_bytes())

    def test_a_key_known_in_any_language_is_never_overwritten(self):
        # Inconsistent catalogs (checked separately): "Open" is only in the uk and de files.
        tmp, root = make_root(shared={"Open": triple("Otwórz", "AUTHORITATIVE-UK", "AUTHORITATIVE-DE")})
        self.addCleanup(tmp.cleanup)
        write(root / "i18n" / "catalogs" / "pl.json", {})
        self.write_mac({"Open": triple("SWIFT-pl", "SWIFT-uk", "SWIFT-de")})
        before = {
            lang: (root / "i18n" / "catalogs" / ("%s.json" % lang)).read_bytes() for lang in LANGS
        }
        problems, added = gen.import_missing(root, Path(self.mac.name))
        self.assertEqual((problems, added), ([], []))
        after = {lang: (root / "i18n" / "catalogs" / ("%s.json" % lang)).read_bytes() for lang in LANGS}
        self.assertEqual(before, after)
        uk = json.loads(after["uk"].decode("utf-8"))
        self.assertEqual(uk["Open"], "AUTHORITATIVE-UK")

    def test_a_key_only_in_one_platform_language_is_not_imported(self):
        tmp, root = make_root(windows={"Open": {"uk": "AUTHORITATIVE-UK"}})
        self.addCleanup(tmp.cleanup)
        self.write_mac({"Open": triple("SWIFT-pl", "SWIFT-uk", "SWIFT-de")})
        self.assertEqual(gen.import_missing(root, Path(self.mac.name)), ([], []))

    def test_should_translate_false_is_skipped(self):
        tmp, root = make_root()
        self.addCleanup(tmp.cleanup)
        entry = xcstrings({"Version": triple("Wersja")})["strings"]["Version"]
        entry["shouldTranslate"] = False
        self.write_mac({}, extra={"Version": entry})
        self.assertEqual(gen.import_missing(root, Path(self.mac.name)), ([], []))


if __name__ == "__main__":
    unittest.main()
