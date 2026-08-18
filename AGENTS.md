# QuiverDL agent guide

QuiverDL is a privacy-respecting, cross-platform download manager built around a reusable Rust
engine and a Tauri 2 + React desktop application.

## Repository map

- `crates/quiver-core`: HTTP probing, streaming downloads, pause/cancel control, resume state, and
  integrity verification.
- `apps/desktop/src-tauri`: Tauri commands and the desktop Rust boundary.
- `apps/desktop/src`: React and TypeScript user interface.
- `docs`: architecture and product decisions.

## Working expectations

- Keep engine behavior independent from the desktop UI.
- Prefer focused changes with deterministic tests.
- Do not add accounts, advertising, analytics, or telemetry.
- Never commit credentials, cookies, authorization headers, private download URLs, or downloaded
  content.
- Preserve cross-platform behavior on Windows, Linux, and macOS.

## Validation

Run the checks relevant to the changed code. Before declaring a repository-wide change complete,
run all of them:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm ci --prefix apps/desktop
npm run build --prefix apps/desktop
cargo check -p quiver-desktop
```

## Code Review Rules

### Download integrity and recovery

- Flag any path that can overwrite a completed destination without explicit user permission.
- Resume is safe only when the URL and total size still match, the server supports ranges, and a
  strong remote validator such as ETag or Last-Modified still matches. A server that ignores a
  validated range request must not silently append a full response to partial content.
- Partial data and its state must remain recoverable after ordinary interruption. Corrupt or stale
  partial data must never be promoted to the final destination.
- File-size accounting and progress arithmetic must handle overflow, missing lengths, short
  responses, and servers that return malformed range headers.

### Security and privacy

- Treat URLs, redirects, response headers, filenames, sidecar state, and downloaded bytes as
  untrusted input.
- Network downloads must remain limited to explicitly supported schemes and bounded redirect
  behavior.
- Never expose secrets or private URLs in logs, errors, telemetry, fixtures, screenshots, or issue
  output.
- Downloads must stream to disk; flag designs that buffer an unbounded response in memory.

### Cross-platform behavior

- Flag filesystem, path, rename, locking, or process assumptions that only work on one operating
  system.
- Core behavior changes need deterministic coverage that can run on Windows, Linux, and macOS
  without the public internet.

### Desktop experience

- Preserve keyboard access, visible focus, readable contrast, and complete adaptive light/dark
  styling for user-visible changes.
- Keep Tauri commands narrow and validate data crossing the JavaScript/Rust boundary.

### Review signal

- Report consequential defects introduced by the pull request, with a concrete failure scenario
  and the smallest safe correction.
- Leave formatting and other fully deterministic mechanical checks to CI.
- Do not block a change for speculative redesign or unrelated pre-existing issues.
