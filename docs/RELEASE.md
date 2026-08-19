# Release process

QuiverDL releases are built from annotated `v*` tags by `.github/workflows/release.yml`. The workflow uses locked Cargo and npm dependency graphs, builds on each target operating system, creates Windows NSIS/MSI installers plus a portable archive, Linux DEB/RPM/AppImage packages, macOS app/DMG bundles for Intel and Apple Silicon, and native-host archives with portable SHA-256 checksum files. Standalone macOS host archives are submitted to Apple notarization after the host is signed. Releases begin as drafts so a maintainer can inspect every asset before publishing.

The same tagged workflow creates deterministic unsigned Chrome Web Store and Firefox AMO submission
archives. Microsoft Store and Mac App Store overlays, sandbox entitlements, validation, and owner
submission commands are documented in [store packaging](STORE_PACKAGING.md). Store accounts,
provisioning profiles, and marketplace signatures remain owner-controlled release inputs.

Windows users who install the NSIS or MSI desktop package and want browser integration must also download the matching `native-host-x86_64-pc-windows-msvc.zip`, then run its `extensions/native-host/install-windows.ps1` script against the included signed host. The portable archive already contains the same host and registration assets.

## One-time repository setup

Create a protected GitHub environment named `release`. Add `WINDOWS_CERTIFICATE` (base64 PFX) and `WINDOWS_CERTIFICATE_PASSWORD`. Add `APPLE_CERTIFICATE` (base64 Developer ID Application P12), `APPLE_CERTIFICATE_PASSWORD`, `KEYCHAIN_PASSWORD`, `APPLE_ID`, `APPLE_PASSWORD` (app-specific password), and `APPLE_TEAM_ID`. Require maintainer approval for the environment.

The workflow deliberately fails instead of publishing unsigned Windows or unnotarized macOS release artifacts. Certificates and account credentials cannot be supplied by source code; the repository owner must obtain them from an appropriate certificate authority and Apple Developer account.

## Cutting a release

1. Confirm CI is green and `ROADMAP.md`, `CHANGELOG.md`, Cargo, npm, and Tauri versions agree.
2. Run the development checks in `README.md` and `cargo bench -p quiver-core --bench transfer_baseline`.
3. Create and push an annotated tag, for example `git tag -a v0.1.0 -m "QuiverDL 0.1.0"` and `git push origin v0.1.0`.
4. Approve the protected release environment. Inspect signatures, notarization, checksums, install/uninstall behavior, extension pairing, and a clean-machine download test.
5. Add human-readable release notes and publish the draft.

Never test signing with production keys on pull requests or upload certificates as artifacts.
