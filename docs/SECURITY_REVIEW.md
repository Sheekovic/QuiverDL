# Security review checklist

Review this checklist for changes touching transfers, paths, persistence, browser integration, IPC, or releases.

- URLs are parsed once, restricted to HTTP(S), and never logged with query secrets.
- Redirect, retry, segment, connection, queue, and message limits remain bounded.
- Resume and segmented responses validate status, validator, exact range, total, and received length.
- No destination can be silently replaced; equivalent paths and all recovery files share reservations.
- Server filenames are suggestions only, sanitized, length-limited, and cannot choose directories.
- Cancellation interrupts network waits, pause checkpoints, hashing, and merge boundaries safely.
- Persistent JSON is size/shape/range validated and atomically replaced.
- Tauri commands validate at the Rust boundary and capabilities remain least privilege.
- Native messages are framed, size-limited, versioned, authenticated, and never persist the token in queue items.
- Browser interception remains off by default and does not forward cookies, credentials, history, or page content.
- New dependencies are justified, locked, maintained, and checked for advisories and license compatibility.
- Release artifacts are produced only by the protected tag workflow, signed where required, checksummed, and manually inspected before a draft is published.

Current verification commands:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm run build --prefix apps/desktop
node --check extensions/chromium/background.js
node --check extensions/firefox/background.js
```

Report unresolved security findings privately according to `SECURITY.md`; never paste real tokens or private URLs into issues or test fixtures.
