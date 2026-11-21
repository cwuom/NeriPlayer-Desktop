[English](./README_EN.md) | [中文](./README.md)

# Quality checks

These tools check module boundaries, the IPC contract, file capacity, and per-function risk scores (CRAP). CI runs them in the `Quality gates` and frontend jobs.

- Only Python 3.11+ is required (standard-library `tomllib`); no third-party packages
- Rules and baselines live in [`config/quality/`](../../config/quality/), not in the scripts
- Module responsibilities and dependency directions are described in [Implementation Notes](../../docs/implementation_EN.md#dependency-rules)

| Check | Entry point | Purpose |
| --- | --- | --- |
| Module boundaries | `module_boundaries.py` | Rust crate registration and dependency direction, Tauri isolation, file and directory capacity; frontend layering, UI IPC baseline, file capacity |
| IPC contract | `ipc_contract.py` | `#[tauri::command]` definitions, `generate_handler!` registrations, and frontend `invoke()` calls stay consistent |
| CRAP | `crap_report.py`, `ts_complexity.mjs` | Risk scores from function complexity and Vitest coverage, with the same formula and thresholds as the Android repository |
| Maintainability metrics | `maintainability.py` | Collects raw metrics from the checks above, tests, coverage, docs, and CI, and scores them with a published rubric |

## Running

From the repository root:

```bash
python3 -B tools/quality/module_boundaries.py
python3 -B tools/quality/ipc_contract.py
python3 -B -m unittest discover -s tools/quality -p 'test_*.py'
node --test tools/quality/*.test.mjs
pnpm crap   # generates Vitest coverage, then runs crap_report.py
```

Exit codes: `0` passes, `1` means violations (each one lists the reason and how to fix it), `2` means invalid input or configuration.

## Module boundaries

Configuration: [`module-boundaries.json`](../../config/quality/module-boundaries.json)

### Rust

| Rule | Details |
| --- | --- |
| Crate registration | Every crate under `src-tauri/crates/` must be registered, its package name must match its directory, and it needs a `README.md` |
| Dependency allow-list | A crate may depend only on project crates listed in `allowed`; `dependencies`, `dev-dependencies`, `build-dependencies`, and `target.*` tables are all checked |
| No cycles | The project dependency graph, including the app shell, must be acyclic |
| Tauri isolation | Domain crates must not depend on `tauri*` and their sources must not call `tauri::` / `tauri_plugin_*::` |
| File capacity | ≤ 2000 production lines per `.rs` file; content after an inline `#[cfg(test)] mod tests` and `tests.rs` files are excluded |
| Directory capacity | ≤ 16 `.rs` files directly in one directory; introduce a submodule beyond that |

### Frontend

| Rule | Details |
| --- | --- |
| Layered imports | `@/…`, relative, and dynamic `import()` paths are checked against the `layers` allow-list; commented-out imports are ignored |
| Unregistered layers | New top-level directories under `src/` must be registered first |
| File capacity | ≤ 2000 lines per `.ts` / `.vue`; historical oversized files are listed in `legacy_oversized` and may only shrink |
| UI IPC | Direct `invoke()` calls in `components` / `views` are allowed only in `ui_invoke_baseline`, and the counts may only go down |
| Directory capacity | ≤ 24 source files directly in one directory; historical exceptions are recorded in `legacy_crowded_directories` and may only shrink |

Test files (`__tests__/`) and declaration files (`*.d.ts`) are excluded from frontend checks.

## IPC contract

Configuration: [`ipc-contract.json`](../../config/quality/ipc-contract.json)

- Every `#[tauri::command]` must be registered in `main.rs`; every registration needs a definition and may appear only once
- Command names invoked with string literals must be registered
- Non-literal calls (such as `invoke(command)`) must declare candidates in `dynamic_invocations`, and each candidate must appear as a literal in that file
- Wrappers that forward a command name (such as `persistUserData`) are registered in `invoke_wrappers`: literal command names at their call sites count as references; `scope: "file"` only counts calls inside the defining file; only the wrapper's own file may forward a non-literal command name
- Commands never referenced by the frontend are allowed only in `unreferenced_baseline`; remove an entry once it is referenced again or deleted

## CRAP

Configuration: [`crap-scope.json`](../../config/quality/crap-scope.json). The formula and thresholds match
`tools_pub/quality/crap_report.py` in the Android repository:

```text
CRAP = C² × (1 − cov)³ + C
```

| Metric | Definition |
| --- | --- |
| `C` | Function cyclomatic complexity, counted by `ts_complexity.mjs` with the decision points of ESLint's built-in `complexity` rule (a test compares every function with ESLint) |
| `cov` | The lower of the function's own statement coverage and v8 block coverage (nested functions excluded), from `coverage/coverage-final.json` |
| Full coverage | `CRAP = C` when `cov = 100%` |

- `measure_patterns`: measured `.ts` files under `src/modules`, `src/utils`, and `src/stores`; every function with CRAP > 8 is written to `coverage/crap/above-8.md`
- `source_patterns`: the gate scope; any function above 9 fails. It may only grow; add files once they comply
- `unscoped_failure_baseline`: functions above 9 outside the scope; it may only decrease
- `cov` approximates path coverage; `scripts/test-*.mjs` regression scripts do not count toward v8 coverage, and Vue components and Rust are not measured

## Baselines (ratchets)

`legacy_oversized`, `legacy_crowded_directories`, `ui_invoke_baseline`, `unreferenced_baseline`, and `unscoped_failure_baseline` record historical debt:

- New files, calls, or commands cannot enter a baseline
- When the code improves, the checks ask you to tighten the baseline so it never drifts
- Never loosen thresholds, grow baselines, or delete meaningful tests just to pass; explain any necessary adjustment in the PR

## Tool regression tests

`test_module_boundaries.py`, `test_ipc_contract.py`, `test_crap_report.py`, and `test_maintainability.py` build fixtures in a temporary directory, `ts_complexity.test.mjs` compares complexity with ESLint, and together they verify that
every class of violation is rejected and compliant layouts pass. They test the gates themselves and do not replace product tests.
