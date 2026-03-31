# Offerings Testing Guide

## Purpose

This document defines the testable build flow to run before cutting a production release.

## Fast Validation

```bash
cargo test --color never
cargo run --quiet -- --self-test
```

`--self-test` is a headless diagnostic mode. It does not create the Slint window. It prints a JSON report with:

- app version
- database path in use
- cached package count
- metadata catalog entry count
- default export-catalog path
- IPC socket path
- per-source availability status

This is the preferred smoke test for CI, SSH sessions, and headless packaging environments.

## Pre-Release Validation

Run the repository script:

```bash
./test_offerings.sh
```

That performs:

1. `cargo fmt --check`
2. `cargo test --color never`
3. `cargo run --quiet -- --self-test`
4. `cargo build --release`
5. `./target/release/offerings --self-test`

## Release Candidate Checks

Before packaging an AppImage or tagging a release, verify:

- `cargo test` passes
- release build completes
- `--self-test` succeeds in both dev and release builds
- `--refresh` and `--refresh-catalog` run successfully on a machine with the target package-manager stack installed
- the GUI still opens correctly in a real desktop session
- at least one install, update, and uninstall flow is exercised on live sources

## Notes

- `--refresh`, `--export-catalog`, `--import-catalog`, `--refresh-catalog`, and `--self-test` are all headless-safe CLI modes.
- Headless commands intentionally skip Slint window creation so they can be used in CI and packaging scripts.
