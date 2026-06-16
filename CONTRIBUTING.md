# Contributing to rustchm

Thanks for your interest in improving rustchm!

## Building

```
cargo build --release
```

Produces `target/release/rustchm`.

## Testing

```
python test/run_tests.py
```

The harness compiles every project under `test/`, asserts the expected internal
CHM files, and round-trips through rustchm's own `--extract`.

## Guidelines

- Format with `cargo fmt` (rustfmt defaults).
- Keep the tool **dependency-free at runtime** — please don't add runtime crates.
- Keep changes focused; one logical change per pull request.
- If you change behavior, describe it in the PR (and update the README if it is
  user-facing).

## Reporting issues

Open an issue with the exact command you ran and, if possible, a minimal input
file that reproduces the problem.
