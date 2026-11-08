#!/usr/bin/env bash
# GitHub's required check: only the deliberate docs-only skip may omit code jobs.
set -euo pipefail
changes=${CHANGES:-}
code=${CODE:-}
frontend=${FRONTEND:-}
general=${GENERAL:-}
core=${CORE:-}
printf 'changes=%s code=%s frontend=%s general=%s core=%s\n' "$changes" "$code" "$frontend" "$general" "$core"
if [ "$changes" != success ]; then
  echo 'change classification did not succeed' >&2
  exit 1
fi
case "$code" in
  true) expected=success ;;
  false) expected=skipped ;;
  *) echo 'invalid change classification' >&2; exit 1 ;;
esac
if [ "$frontend" != "$expected" ] || [ "$general" != "$expected" ] || [ "$core" != "$expected" ]; then
  echo "required jobs must all be $expected" >&2
  exit 1
fi
