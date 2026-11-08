# Parallel CI checks

## Goal

Shorten pull-request CI elapsed time by running frontend and general checks in parallel with platform Rust checks, following the MyPass approach while retaining existing validation and the required `build-and-test` status.

## Approaches

1. Split frontend and general checks into separate jobs, keeping Rust and desktop checks on Windows and macOS. Selected: isolates independent work without duplicating Rust compilation.
2. Split Rust checks and tests into further jobs. Not selected: each additional runner may recompile the workspace and increase runner usage; measure the first split before considering it.

## Design

- Keep change classification and the documentation-only skip behavior.
- Move frontend tests and Svelte checks to an independently scheduled Windows/macOS matrix, preserving both platforms. Each Rust job retains its own frontend installation and build for Tauri's compile-time `app/dist` embedding.
- Move localization, version and release-script checks into independent Windows/macOS jobs, retaining their current per-platform coverage. Keep the macOS production gate and its fixtures on the macOS general-check leg.
- Retain formatting, Clippy, workspace tests, dependency guard, required Windows desktop smoke and non-blocking Windows input-driven tests in the platform Rust matrix. Preserve current commands and locked dependencies.
- The always-run `build-and-test` aggregator must fail if classification fails or if any required matrix fails, is cancelled, or is unexpectedly skipped; only documentation-only changes may skip all code jobs. Reject missing or invalid classification values and any partial skip. Keep its required check name unchanged.
- Keep action pins, permissions and pull-request security boundaries. Do not change release packaging or publication.
- Add deterministic tests for aggregator outcomes, update project documentation, run actionlint and relevant tests, and compare real CI timings after the change. Multiple jobs can increase runner startup time and minutes; actual improvement must be measured.
