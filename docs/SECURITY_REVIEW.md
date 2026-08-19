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
- Native bridge directories and token files are private before any secret bytes are written.
- Browser interception remains off by default and does not forward cookies, credentials, history, or page content.
- Distributed metadata is bounded before parsing and cannot select paths outside the confirmed
  destination root. Metalink requires a complete preview before listed-mirror requests. Magnet
  networking remains deferred; offline inspection cannot resolve metadata or enqueue a transfer.
- Metalink mirrors cannot inherit origin credentials; every completed file requires its confirmed
  size and a SHA-256-or-stronger whole-file digest before promotion. Phase one ignores RFC 6249
  response metadata and fails closed when a proxy cannot prove destination-address binding.
- BitTorrent changes stay outside the HTTP engine and separately review peer discovery, uploading,
  proxy coverage, private torrents, parser limits, path containment, strong v2 integrity, and stop
  semantics. V1-only inputs remain offline-inspector data and cannot start a transfer; recovered v2
  pieces are rehashed before verified status is restored. Initial network support is allowlisted to
  HTTPS trackers/web seeds and outbound TCP peers.
- New dependencies are justified, locked, maintained, and checked for advisories and license compatibility.
- Release artifacts are produced only by the protected tag workflow, signed where required, checksummed, and manually inspected before a draft is published.
- Updater manifests use the fixed HTTPS origin, immutable release assets, exact embedded signatures,
  complete platform coverage, increasing SemVer, and no unsigned or network-downgrade fallback.
- A rollback change preserves queue/download state, re-verifies the cached previous package, and
  passes bounded failed-start, migration, power-loss, and disk-full recovery tests on every platform.

Current verification commands:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm run build --prefix apps/desktop
node --check extensions/chromium/background.js
node --check extensions/firefox/background.js
node scripts/validate-updater-design.mjs
node --test scripts/test-generate-update-manifest.mjs
```

Report unresolved security findings privately according to `SECURITY.md`; never paste real tokens or private URLs into issues or test fixtures.
