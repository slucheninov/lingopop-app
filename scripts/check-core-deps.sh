#!/usr/bin/env bash
# Guard: lingopop-core must stay OS-agnostic. Exit 0 = clean, exit 1 = offending dependency.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

status=0

# Dependency trees are resolved once per target (Windows, macOS, Linux), with all features, so a
# platform-specific dependency cannot hide from the host-only default tree. `--target all` is not
# used: it unifies the Windows-only `system-proxy` feature into the macOS edges and reports
# system-configuration, which no real macOS build pulls in.
core_tree() {
  local target
  for target in x86_64-pc-windows-msvc aarch64-apple-darwin x86_64-unknown-linux-gnu; do
    cargo tree -p lingopop-core --target "$target" --all-features "$@" || return $?
  done
}

# (a) Direct dependencies: no OS / UI crates by name prefix, and no tokio (it may only be a
# dev-dependency or come in transitively via reqwest).
direct=$(core_tree -e normal,build --depth 1 --prefix none)
bad_direct=$(printf '%s\n' "$direct" | awk '
  { name = $1 }
  name ~ /^(windows|winapi|tauri|objc|cocoa|core-foundation)/ || name == "core-graphics" || name == "tokio" { print }
')
if [ -n "$bad_direct" ]; then
  echo "error: lingopop-core has forbidden direct dependencies:" >&2
  printf '%s\n' "$bad_direct" >&2
  status=1
fi

# (b) Whole tree: no tauri*, objc*, core-foundation*, system-configuration*, windows, winapi,
# cocoa, nor a platform TLS stack (native-tls, openssl*, aws-lc-*, rustls-native-certs,
# rustls-platform-verifier).
# A transitive windows-sys (e.g. via getrandom or tempfile on Windows) stays legitimate.
full=$(core_tree -e normal,build --prefix none)
bad_tree=$(printf '%s\n' "$full" | awk '
  { name = $1 }
  name ~ /^(tauri|objc|core-foundation|system-configuration)/ || name == "windows" || name == "winapi" || name == "cocoa" || name == "lingopop-platform" { print }
  name == "native-tls" || name == "openssl" || name == "openssl-sys" { print }
  name == "aws-lc-rs" || name == "aws-lc-sys" { print }
  name == "rustls-native-certs" || name == "rustls-platform-verifier" { print }
')
if [ -n "$bad_tree" ]; then
  echo "error: lingopop-core dependency tree contains forbidden crates:" >&2
  printf '%s\n' "$bad_tree" >&2
  status=1
fi

# (c) A dev-dependency cycle core -> platform is legal for cargo, so this is the real guard.
with_dev=$(core_tree -e normal,build,dev --prefix none)
if printf '%s\n' "$with_dev" | awk '{ if ($1 == "lingopop-platform") found = 1 } END { exit !found }'; then
  echo "error: lingopop-core depends (even as a dev-dependency) on lingopop-platform" >&2
  status=1
fi

exit "$status"
