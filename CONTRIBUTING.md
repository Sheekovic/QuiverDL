# Contributing to QuiverDL

QuiverDL is at an early architectural stage. Before implementing a large feature, open an issue
describing the user problem, expected behavior, and a testing strategy.

## Local checks

Every change must pass:

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Keep the engine independent of the desktop interface. Network and filesystem behavior should be
covered by tests, and partial downloads must never be presented as completed files.

