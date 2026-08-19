# Secure updater design

QuiverDL's direct-download updater is deliberately disabled in normal builds until a maintainer has
generated, backed up, and configured a stable Tauri updater key. Store-distributed builds use the
Microsoft Store or Mac App Store update channel and never bypass marketplace review with the direct
updater.

## Trust and release boundary

- Tauri's updater signature is mandatory and cannot be disabled. The public key is compiled into
  the direct-download app; the encrypted private key exists only in the protected `release`
  environment and an offline maintainer backup.
- The manifest endpoint is fixed to
  `https://github.com/Sheekovic/QuiverDL/releases/latest/download/latest.json`. Production builds do
  not allow HTTP, a user-configurable endpoint, unsigned fallback, or runtime public-key replacement.
- `latest.json` names immutable assets under the matching `v<version>` GitHub release. The manifest
  contains the exact contents of each Tauri `.sig` file, not a path to it, and includes every
  supported direct-download platform so partial publication fails closed.
- Authenticode and Apple code signing remain required in addition to the updater signature. The
  updater signature authenticates QuiverDL's update channel; operating-system signatures
  authenticate the publisher and package on platforms that support them.
- The application asks before download and again before restart. It never installs while active
  downloads, verification, merge, or queue persistence are in progress. Failed checks leave the
  current installation untouched and retain no executable temporary file.

The tracked `tauri.updater.conf.json.template` enables signed updater artifacts and the passive
Windows installer only for a protected direct-release build. `scripts/prepare-updater-config.mjs`
materializes an ignored config after decoding and validating `TAURI_UPDATER_PUBLIC_KEY` as the
canonical Tauri Minisign public-key format, including its packet type and key identifier. It rejects
private-key material and refuses to overwrite an existing config. Pull-request workflows never
receive the public/private key pair or enable the updater.

## Manifest generation

`scripts/generate-update-manifest.mjs` accepts exactly the four supported platform artifacts, parses
each sibling `.sig` as a canonical Tauri Minisign signature, and emits deterministic Tauri static
JSON. It requires the same `TAURI_UPDATER_PUBLIC_KEY` used by the app, streams every artifact through
BLAKE2b-512, and cryptographically verifies both its Ed25519 artifact signature and trusted-comment
signature before writing a manifest. Modern prehashed `ED` packets are mandatory so verification
remains memory-bounded. It also requires distinct signed-input identities and signatures from the
configured key identifier across all platforms. It rejects symlinks, empty or oversized
artifacts/signatures, unexpected platforms, unsafe filenames, non-SemVer versions, tag mismatches,
credentials, HTTP, query strings, fragments, duplicate platform paths/URLs, existing or
artifact-aliased output paths, and any release URL outside this repository. Stable manifests reject
prerelease versions until a separately named, tested, and published prerelease channel exists.

Example after all signed artifacts have reached draft release `v0.2.0`:

```powershell
$env:TAURI_UPDATER_PUBLIC_KEY = (Get-Content C:\secure\quiverdl-updater.key.pub -Raw).Trim()
node scripts/generate-update-manifest.mjs `
  --version 0.2.0 `
  --base-url https://github.com/Sheekovic/QuiverDL/releases/download/v0.2.0 `
  --output dist/latest.json `
  --artifact windows-x86_64=dist/QuiverDL-0.2.0-setup.exe `
  --artifact linux-x86_64=dist/QuiverDL-0.2.0.AppImage `
  --artifact darwin-x86_64=dist/QuiverDL-0.2.0-x86_64.app.tar.gz `
  --artifact darwin-aarch64=dist/QuiverDL-0.2.0-aarch64.app.tar.gz
```

Upload the generated manifest only after verifying every updater signature, OS signature,
notarization result, checksum, and clean-machine upgrade. Publishing `latest.json` is the final
release action; a manifest must never point at a draft, mutable, or unsigned asset.

## Version and rollback policy

Normal update comparison is strictly monotonic SemVer. The app persists the highest successfully
started version in protected local state; network responses cannot authorize a downgrade, even when
an older artifact still has a valid historical signature. Release tags and attached assets are
immutable.

Before installation, QuiverDL atomically records the current version, the verified previous package
identity, queue/settings schema versions, and a pending-start marker. The new version clears that
marker only after settings and queue migrations load, the engine initializes, and a bounded startup
health check succeeds. Migrations must retain a backwards-readable backup and must not delete the
last known-good state.

Rollback uses only the immediately previous package already cached and verified before installation;
it never downloads a lower version from the network. A platform-specific recovery helper must:

1. notice that the pending-start marker survived a failed launch or bounded health timeout;
2. re-check the cached package's Tauri signature with the version-scoped key recorded before the
   update, plus its recorded hash and OS signature where available;
3. restore the backwards-readable settings/queue backup without touching user downloads or partials;
4. reinstall the previous package atomically, record the reason locally, and disable automatic
   retries of the rejected version; and
5. show a recovery report and require explicit consent before any later update attempt.

Until that helper passes clean-machine crash, power-loss, migration, and disk-full tests on Windows,
Linux, Intel macOS, and Apple Silicon, QuiverDL may offer a manual verified reinstall but must not
claim automatic rollback. A bad release is withdrawn by removing `latest.json`; maintainers then
publish a higher fixed version rather than changing an existing release or serving a network
downgrade.

## Key lifecycle

Generate the updater key offline with `tauri signer generate`, store the encrypted private key in the
protected GitHub environment, and keep a separately tested offline backup. Never put it in a pull
request, artifact, log, `.env`, or issue. Losing the key prevents updates to installed clients.

Rotation is not transparently safe with Tauri's single embedded updater key. First publish a bridge
release signed by the old key that embeds the new public key, keep its OS-signed installers and
checksums immutable, and wait through a documented adoption window before switching `latest.json`
to new-key signatures. Clients that miss the bridge cannot authenticate a new-key release; they must
install that retained bridge or a later OS-signed package manually from the repository's verified
release page. Never claim that a one-release rotation reaches offline clients. A future seamless
rotation requires a reviewed multi-key verifier or a version-aware endpoint that permanently serves
an old-key-signed bridge to old clients.

The bridge and the first new-key release must also retain the old public key in a rollback-only key
ring. That verifier is scoped to the exact cached prior version, artifact hash, and pending-update
record; it cannot authorize a network update. Keep each prior key until two later versions have
started successfully, so both a failed bridge launch and a failed first new-key launch can
authenticate their immediate cached predecessor.

If the old private key is compromised, disable the update endpoint and direct users to a freshly
OS-signed installer; do not silently trust a new updater key delivered by the compromised channel.
