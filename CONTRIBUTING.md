# Contributing

Thanks for your interest in contributing to sheetpack.

## Development

1. Fork / clone the repository
2. Install a stable Rust toolchain (https://rustup.rs)
3. Create a branch: `git switch -c feat/your-change`
4. Make your change and verify locally:
   ```bash
   cargo fmt --all -- --check
   cargo clippy --all-targets -- -D warnings
   cargo test --all
   cargo build --release
   ```
5. Commit and open a pull request

## Ground rules

- Determinism is the product. Any change that lets timestamps, hash-map
  iteration order, randomness or platform differences reach the output PNG,
  a manifest or the fingerprint is a bug, however convenient. New code paths
  need a test proving two runs produce identical bytes where that applies.
- The fingerprint contract is public. If a change alters what goes into the
  fingerprint (settings line, hashing scheme), say so in the PR and in the
  changelog: every user's next `--check` will report stale once, and that
  must be on purpose, not by accident.
- Manifest dialects are added against a primary source. A new `--format`
  needs a link to the consuming engine's loader code or format documentation
  and the date it was read, in the module comment -- not a blog post, not a
  model's memory.
- No rotation, no polygon packing, unless the manifest story for every
  existing consumer is solved first. Density is worth less than loaders that
  keep working.

## Tests

The suite is fixture-free: sprites are generated in code, atlases are decoded
and asserted on pixel counts, exit codes and manifest text. Follow that
pattern; a checked-in binary fixture needs a very good reason.
