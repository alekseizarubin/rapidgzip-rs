# Security Policy

This policy applies to the `rapidgzip-rs` repository.

## Reporting

If you find a vulnerability that could lead to memory corruption, arbitrary
code execution, data exposure, or unsafe archive handling, do not open a public
issue with full exploit details.

Use GitHub private vulnerability reporting for this repository.

## Scope

Security-relevant reports for this repository include:

- FFI unsafety
- memory safety bugs in the native integration
- archive parsing bugs that can crash or corrupt process state
- path handling issues in the Rust library surface
- unsafe temporary object or callback lifecycle handling
- vendored upstream vulnerabilities affecting the shipped library

## Out of Scope

The companion CLI and benchmark repositories should handle their own
repository-specific security reports.
