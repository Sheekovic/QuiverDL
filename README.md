# QuiverDL

**Your downloads, right on target.**

QuiverDL is a performance-first, privacy-respecting open-source download manager. It is being
built around a native Rust engine with a lightweight cross-platform desktop interface.

> [!IMPORTANT]
> QuiverDL is in early development. There are no official binaries yet.

## Product principles

- Reliability before feature count
- Native performance with modest memory usage
- Safe pause and resume across application restarts
- No accounts, advertising, analytics, or telemetry
- Clear behavior: never silently overwrite a user's file
- A reusable engine shared by the desktop app, browser bridge, and future CLI

## Current status

The first engine slice supports HTTP/HTTPS transfer, safe partial files, conservative resume,
pause/cancel controls, progress events, and SHA-256 verification. Multi-connection downloading
and the desktop shell are the next milestones.

## Repository layout

```text
crates/quiver-core/   Native download engine and domain model
apps/desktop/         Tauri 2 and React desktop application
docs/                 Architecture and product decisions
```

## Development

Install the stable Rust toolchain, then run:

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## License

QuiverDL is licensed under `GPL-3.0-or-later`. This is the working license for development and
will be reviewed before the first public release.
