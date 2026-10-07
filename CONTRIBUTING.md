# Contributing to StateFS

Thank you for your interest in contributing to StateFS.

## Principles & Engineering Philosophy

StateFS is built on strict systems engineering principles:
1. **Mechanical Sympathy**: Design for CPU cache lines (64 bytes), branch predictors, and flat contiguous memory layouts.
2. **Zero Involuntary Allocations**: Hot paths (lookups, resolution, reads) must not allocate heap memory unnecessarily.
3. **Explicit Failure**: Never swallow errors or return dummy defaults on unhandled states.
4. **Zero Emojis**: In accordance with project standards, emojis are strictly prohibited across all source code, comments, documentation, commit messages, and PR descriptions.

## Development Workflow

1. Fork the repository and create your feature branch from `main`.
2. Follow [Conventional Commits](https://www.conventionalcommits.org/):
   - `feat(scope): ...`
   - `fix(scope): ...`
   - `perf(scope): ...`
   - `docs(scope): ...`
   - `test(scope): ...`
3. Ensure all local verifications pass:
   ```bash
   cargo test --workspace
   cargo clippy --workspace --all-targets -- -D warnings
   cargo fmt --all -- --check
   ```
4. Submit a Pull Request targeting `main`. Ensure all CI checks pass.

## Unsafe Code Policy

Every `unsafe` block must be accompanied by a clear, audited `// SAFETY:` comment detailing invariants and bounds guarantees.
Panics must never cross C-ABI or foreign language boundaries.
