# Contributing

This repository contains the Rust library layer for the `rapidgzip` stack.

## Scope

Changes that belong here:

- safe Rust API behavior and ergonomics
- raw FFI bindings and native build glue
- native C/C++ bridge changes used by the Rust library
- vendored upstream refreshes and performance tuning
- integration and regression tests for the library layer

Changes that belong in companion repositories:

- end-user CLI packaging and installation flows
- benchmark harnesses and published benchmark results

## Workflow

- Prefer small reviewable commits.
- Keep vendored upstream refreshes separate from Rust API changes whenever you can.
- Do not mix benchmark artifact churn with library code changes.
- Record significant hardening or compatibility changes in git with enough context to audit them later.

## Validation

Before proposing changes, run the smallest relevant validation set you can justify.

Typical commands:

```bash
cargo test -p rapidgzip
cargo test -p rapidgzip-tests
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

For performance-sensitive changes, include enough context to reproduce the
claim: dataset type, command line, machine description, and whether the result
was measured on `.gz`, `.bgz`, or indexed paths.
