#!/usr/bin/env python3
"""Decide whether a CI run needs the full build (Python stdlib, 3.9+).

Reads changed paths (one per line) on stdin and prints `code=true` or
`code=false`. Only a change set made entirely of Markdown files skips the
build; `docs/INSTALL.md` is the release-notes template a test renders, so it
counts as code. An empty or unknown change set always builds.
"""
import sys

# Markdown files that tests or the release read, so they need the full run.
CODE_MARKDOWN = frozenset({"docs/INSTALL.md"})


def needs_build(paths):
    paths = [p.strip() for p in paths if p.strip()]
    if not paths:
        return True
    return any(not p.lower().endswith(".md") or p in CODE_MARKDOWN for p in paths)


def main():
    print(f"code={'true' if needs_build(sys.stdin.read().splitlines()) else 'false'}")


if __name__ == "__main__":
    main()
