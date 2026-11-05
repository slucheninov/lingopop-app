#!/usr/bin/env bash
# Readiness gate for the macOS build: fails when the native macOS wiring, a release configuration
# or the macOS identity drifts back to something that must not ship. It complements the behaviour
# tests (it is cheap text inspection, not proof that the adapters work) and needs only POSIX tools.
#
#   scripts/check-macos-production.sh [repo-root]
#
# Exit 0 = ready, exit 1 = at least one problem (each printed to stderr).
set -euo pipefail

root=${1:-"$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"}
cd "$root"

status=0
fail() {
  echo "error: $*" >&2
  status=1
}

native=crates/lingopop-platform/src/native.rs
macos_conf=app/src-tauri/tauri.macos.conf.json

# (1) The macOS `native()` selects production adapters only.
if [ ! -f "$native" ]; then
  fail "$native is missing"
  body=""
else
  body=$(awk '
    /^#\[cfg\(target_os = "macos"\)\]$/ { armed = 1; next }
    armed && /^pub fn native\(\) -> Platform \{/ { inside = 1 }
    { armed = 0 }
    inside { print }
    inside && /^}/ { exit }
  ' "$native")
fi
if [ -z "$body" ]; then
  fail "the macOS native() was not found in $native (the guard would pass vacuously)"
else
  for forbidden in DevPlaintextProtector dev_protector Fake NoLastForeground AlwaysGranted Unsupported 'todo!' 'unimplemented!'; do
    if printf '%s\n' "$body" | grep -n -F -e "$forbidden" >&2; then
      fail "the macOS native() wires a dev, fake, no-op or unsupported adapter ($forbidden)"
    fi
  done
  for required in MacProtector MacLoginItem MacCapture MacOcr MacOverlay MacClipboard MacKeys MacForeground MacPermissions MacPopupWindow MacDismissWatcher; do
    if ! printf '%s\n' "$body" | grep -q -F -e "$required"; then
      fail "the macOS native() does not wire the production adapter $required"
    fi
  done
fi

# (2) No leftover dev-stub marker text in the macOS adapters (the old Unsupported stubs).
if [ -d crates/lingopop-platform/src ]; then
  for marker in 'screen capture on macOS' 'OCR on macOS' 'region overlay on macOS'; do
    if grep -rn -F -e "$marker" crates/lingopop-platform/src/macos "$native" >&2 2>/dev/null; then
      fail "dev stub marker \"$marker\" is still in the macOS adapters"
    fi
  done
fi

# (3) DevPlaintextProtector exists only behind its cfg. Any production source that mentions it
# (outside dev_protector.rs) must have the gate on the line before.
while IFS= read -r file; do
  case "$file" in */dev_protector.rs) continue ;; esac
  hits=$(awk '
    /DevPlaintextProtector|dev_protector/ && prev !~ /cfg\(any\(test, feature = "dev-plaintext"\)\)/ { print FILENAME ":" FNR ": " $0 }
    NF { prev = $0 }
  ' "$file")
  if [ -n "$hits" ]; then
    printf '%s\n' "$hits" >&2
    fail "DevPlaintextProtector is referenced without its cfg gate"
  fi
done < <(find crates app/src-tauri/src -path '*/src/*' -name '*.rs' -not -path '*/target/*' -not -path '*/tests/*' 2>/dev/null | sort)

# (4) The dev-plaintext feature is never enabled by a manifest, a workflow, a packaging config or
# a cargo config. The one allowed mention is its definition in the platform manifest.
while IFS= read -r file; do
  hits=$(grep -n -F -e 'dev-plaintext' "$file" | grep -v -x -E '[0-9]+:dev-plaintext = \[\]' || true)
  if [ -n "$hits" ]; then
    printf '%s: %s\n' "$file" "$hits" >&2
    fail "dev-plaintext is enabled in $file"
  fi
done < <(
  {
    find . -name Cargo.toml -not -path '*/target/*' -not -path '*/node_modules/*'
    find .github/workflows app/src-tauri .cargo scripts -type f \
      \( -name '*.yml' -o -name '*.yaml' -o -name '*.json' -o -name '*.toml' -o -name '*.sh' \) \
      -not -path '*/target/*' -not -name 'check-macos-production.sh' 2>/dev/null
    [ -f app/package.json ] && echo app/package.json
  } | sort -u
)

# (5) The macOS release configuration: approved identity, minimum OS, and never a `version` key
# (the shared version stays in Cargo.toml / tauri.conf.json). The values are read by JSON path
# (python3, standard library), so a key in the wrong place (a top-level `minimumSystemVersion`,
# or one inside a string) cannot satisfy the check.
json_value() { # json_value <file> <dotted.path>: the string at the path, empty when absent
  python3 -c '
import json, sys
node = json.load(open(sys.argv[1]))
for key in sys.argv[2].split("."):
    node = node.get(key) if isinstance(node, dict) else None
print(node if isinstance(node, str) else "")
' "$1" "$2"
}
if [ ! -f "$macos_conf" ]; then
  fail "$macos_conf is missing"
elif ! python3 -c 'import json, sys; json.load(open(sys.argv[1]))' "$macos_conf" 2>/dev/null; then
  fail "$macos_conf is not valid JSON"
else
  [ "$(json_value "$macos_conf" identifier)" = "app.lingopop.tauri.macos" ] \
    || fail "$macos_conf: identifier must be app.lingopop.tauri.macos"
  [ "$(json_value "$macos_conf" bundle.macOS.minimumSystemVersion)" = "13.0" ] \
    || fail "$macos_conf: bundle.macOS.minimumSystemVersion must be 13.0"
  if grep -n -E '"version"[[:space:]]*:' "$macos_conf" >&2; then
    fail "$macos_conf must not contain a version key"
  fi
fi

# (6) A release or packaging build never turns on every cargo feature: that would also enable
# `dev-plaintext` (and any future dev-only feature) in the shipped binary. Discovery is by what
# a file DOES, not by its name (`packaging.yml` is not `*package*`): every workflow and script is
# scanned command by command. Before splitting, a workflow `run:` is unfolded the way YAML reads
# it (`run: >` and a plain multi-line value fold their lines into ONE command; `run: |` keeps
# its lines), backslash continuations are joined and `#` comments (outside quotes) are dropped;
# `&&`, `||`, `;` and `|` then start a new command. A command fails the gate only when it is a
# build invocation carrying `--all-features`: `cargo [+toolchain] [global options] build|test|
# clippy|run|check|bench|doc|rustc|install|bundle|tauri ...` (`cargo --color always build` is a
# build; the option values of `--color`, `-C`, `-Z`, `--config` are skipped), `tauri build|
# bundle`, `npm run tauri`, `npx tauri`. Read-only commands such as `cargo tree --all-features`
# or `cargo metadata --all-features` never fail it, even in a file that also builds; in a file
# whose NAME says it releases, packages, bundles, signs or notarizes, any other command carrying
# `--all-features` fails too.
commands_with_all_features() {
  python3 -I - "$1" "$2" <<'PY'
import re
import sys

path, named = sys.argv[1], sys.argv[2] == "1"
BUILD = {"build", "test", "clippy", "run", "check", "bench", "doc", "rustc", "install", "bundle", "tauri"}
READ_ONLY = {"tree", "metadata", "pkgid", "read-manifest", "locate-project", "vendor", "fetch"}
VALUE_OPTIONS = {"--color", "-C", "-Z", "--config"}
RUN_KEY = re.compile(r"^(\s*)(?:-\s+)?run\s*:\s*(.*)$")


def strip_comment(line):
    """Cuts a `#` comment that starts a word outside single and double quotes."""
    quote = None
    for i, ch in enumerate(line):
        if quote:
            if ch == quote:
                quote = None
        elif ch in "'\"":
            quote = ch
        elif ch == "#" and (i == 0 or line[i - 1].isspace()):
            return line[:i]
    return line


def indent_of(line):
    return len(line) - len(line.lstrip())


def logical_lines(text):
    """(first line number, line) with YAML `run:` scalars unfolded and `\\` continuations joined."""
    lines = text.split("\n")
    out, i = [], 0
    while i < len(lines):
        line = lines[i]
        m = RUN_KEY.match(line)
        if not m:
            out.append((i + 1, line))
            i += 1
            continue
        key_indent, value = len(m.group(1)), m.group(2).strip()
        if value.startswith("#"):
            value = ""
        block = re.match(r"^([>|])[-+0-9]*\s*(#.*)?$", value)
        j = i + 1
        body = []
        while j < len(lines) and (not lines[j].strip() or indent_of(lines[j]) > key_indent):
            body.append(lines[j])
            j += 1
        while body and not body[-1].strip():
            body.pop()
        if block and block.group(1) == "|":
            out.append((i + 1, ""))
            out.extend((i + 2 + k, b) for k, b in enumerate(body))
        elif block:
            # Folded: the lines are ONE command (a more-indented line is kept apart in YAML,
            # which for a shell command only means a line break: treated as a separator).
            folded, number = [], i + 2
            for b in body:
                if b.strip():
                    folded.append(b.strip())
            out.append((number, " ".join(folded)))
        elif value and not value.startswith(("'", '"')) and body:
            # A plain multi-line scalar also folds into one line.
            out.append((i + 1, " ".join([value] + [b.strip() for b in body if b.strip()])))
        else:
            out.append((i + 1, line))
            out.extend((i + 2 + k, b) for k, b in enumerate(body))
        i = j
    return out


def commands(text):
    acc, start = "", 0
    for number, raw in logical_lines(text):
        line = strip_comment(raw)
        if not line.strip():
            if acc:
                yield start, acc
                acc = ""
            continue
        if not acc:
            start = number
        cont = re.search(r"\\\s*$", line) is not None
        acc += re.sub(r"\\\s*$", " ", line)
        if not cont:
            yield start, acc
            acc = ""
    if acc:
        yield start, acc


def cargo_subcommand(tokens):
    for k, tok in enumerate(tokens):
        if tok.rsplit("/", 1)[-1] != "cargo":
            continue
        j = k + 1
        while j < len(tokens):
            t = tokens[j]
            if t.startswith("+"):
                j += 1
            elif t in VALUE_OPTIONS:
                j += 2
            elif t.startswith("-"):
                j += 1
            else:
                return t
        return None
    return None


def is_build(command):
    tokens = command.split()
    sub = cargo_subcommand(tokens)
    if sub in BUILD:
        return True
    for k, tok in enumerate(tokens):
        nxt = tokens[k + 1] if k + 1 < len(tokens) else ""
        if tok == "tauri" and nxt in ("build", "bundle"):
            return True
        if tok == "npm" and nxt == "run" and k + 2 < len(tokens) and tokens[k + 2] == "tauri":
            return True
        if tok == "npx" and nxt == "tauri":
            return True
    return False


def is_read_only(command):
    return cargo_subcommand(command.split()) in READ_ONLY


with open(path, encoding="utf-8", errors="replace") as handle:
    text = handle.read()
for number, command in commands(text):
    for segment in re.split(r"&&|\|\||;|\|", command):
        if "--all-features" not in segment:
            continue
        if is_build(segment) or (named and not is_read_only(segment)):
            print(f"{number}:{segment.strip()}")
PY
}
while IFS= read -r file; do
  named=0
  if printf '%s' "$(basename "$file")" | grep -q -i -E 'release|packag|bundle|dist|dmg|notar|sign'; then
    named=1
  fi
  hits=$(commands_with_all_features "$file" "$named")
  [ -n "$hits" ] || continue
  printf '%s: %s\n' "$file" "$hits" >&2
  fail "$file builds with --all-features"
done < <(
  {
    find .github/workflows -type f \( -name '*.yml' -o -name '*.yaml' \) 2>/dev/null
    find scripts -type f \( -name '*.sh' -o -name '*.bash' -o -name '*.zsh' -o -name '*.py' -o -name '*.swift' \) \
      -not -path 'scripts/tests/*' -not -name 'check-macos-production.sh' 2>/dev/null
  } | sort -u
)

if [ "$status" -eq 0 ]; then
  echo "macOS production readiness: ok"
fi
exit "$status"
