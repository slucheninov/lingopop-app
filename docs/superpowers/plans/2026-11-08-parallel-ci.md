# Parallel CI Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement task-by-task.

**Goal:** Run frontend and general checks alongside platform Rust jobs while retaining the `build-and-test` required check.

**Architecture:** Three independently scheduled Windows/macOS matrices share the existing classification decision. A fail-closed aggregator accepts only successful required matrices, or all skipped matrices for documentation-only changes.

**Tech Stack:** GitHub Actions, Bash, Python stdlib, Rust, Svelte.

**Spec:** `docs/superpowers/specs/2026-11-08-parallel-ci-design.md`

## Global Constraints

Keep OS coverage, platform gate, desktop test behavior, pinned actions and no-secret PR boundary. Build frontend in each Rust job for embedded Tauri assets. Keep `build-and-test` exact name and documentation-only behavior. Do not alter release workflow.

## Review Focus

- Classification failure, empty output or unexpected output cannot pass.
- Required matrix failure, cancellation or partial skip cannot pass.
- Documentation-only changes pass only when all code matrices skipped.
- Matrix coverage remains both Windows and macOS for frontend and general checks.
- Rust jobs retain the embedded frontend and required Windows desktop tests.

## Tasks

1. Add deterministic aggregator truth-table tests in `scripts/tests/test_ci_results.py` against `scripts/check-ci-results.sh`. Cover success, docs skip, every failure/cancel/skip, invalid classification; run RED. Implement the shell script and run GREEN.
2. Split `.github/workflows/ci.yml` into frontend, general and Rust platform matrices; have the aggregator execute the tested script with explicit result env vars. Keep all existing checks and correct OS-specific behavior; run actionlint and fixture tests.
3. Update `AGENTS.md` and validate production gate, all Python tests, shellcheck, formatting and change classification. Compare actual remote elapsed time only after an appropriately authorized PR run.
