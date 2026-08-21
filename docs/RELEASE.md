# Release process

QuiverDL releases are built from annotated `v*` tags by `.github/workflows/release.yml`. Linux is an independent release path: the workflow uses locked Cargo and npm dependency graphs, creates DEB, RPM, and AppImage packages plus a native-host archive, attaches SHA-256 checksums, and publishes the validated draft. Pull requests that change the release workflow perform a complete Linux bundle build before merging.

Signed direct-download Windows and macOS packages remain optional jobs. They run only when the repository variable `ENABLE_SIGNED_RELEASES` is set to `true`; Windows certificate and Apple Developer credentials are then required in the protected `release` environment. The Linux release does not wait for those credentials. Standalone macOS host archives are submitted to Apple notarization after the host is signed.

The same tagged workflow creates deterministic unsigned Chrome Web Store and Firefox AMO submission
archives. Microsoft Store and strict-confined Snap Store packaging, validation, and owner submission
commands are documented in [store packaging](STORE_PACKAGING.md). Store accounts and marketplace
signatures remain owner-controlled release inputs. Mac App Store packaging remains gated on durable
security-scoped bookmark support for restart-safe destinations.

Windows users who install the NSIS or MSI desktop package and want browser integration must also download the matching `native-host-x86_64-pc-windows-msvc.zip`, then run its `extensions/native-host/install-windows.ps1` script against the included signed host. The portable archive already contains the same host and registration assets.

## One-time repository setup

Create a protected GitHub environment named `release`. Add `WINDOWS_CERTIFICATE` (base64 PFX) and `WINDOWS_CERTIFICATE_PASSWORD`. Add `APPLE_CERTIFICATE` (base64 Developer ID Application P12), `APPLE_CERTIFICATE_PASSWORD`, `KEYCHAIN_PASSWORD`, `APPLE_ID`, `APPLE_PASSWORD` (app-specific password), and `APPLE_TEAM_ID`. Require maintainer approval for the environment.

Set the repository variable `ENABLE_SIGNED_RELEASES` to `true` only after those credentials are ready. The signed jobs deliberately fail instead of publishing unsigned Windows or unnotarized macOS release artifacts. Certificates and account credentials cannot be supplied by source code; the repository owner must obtain them from an appropriate certificate authority and Apple Developer account.

Direct-download update signing is a separate trust root from operating-system code signing. Follow
the [secure updater design](UPDATER.md): generate and back up the Tauri key offline, add the encrypted
private key only to the protected release environment, compile the public key into direct builds,
and publish `latest.json` only after every immutable artifact and signature passes release QA. Store
builds use their marketplace update channel.

## Cutting a release

1. Confirm CI is green and `ROADMAP.md`, `CHANGELOG.md`, Cargo, npm, and Tauri versions agree.
2. Run the development checks in `README.md` and `cargo bench -p quiver-core --bench transfer_baseline`.
3. Create and push an annotated tag, for example `git tag -a v0.1.0 -m "QuiverDL 0.1.0"` and `git push origin v0.1.0`.
4. Confirm the Linux workflow publishes the release with AppImage, DEB, RPM, native-host, and checksum assets.
5. Perform a clean-machine Linux download and launch test, then add any additional human-readable release notes.
6. When signed direct downloads are enabled, approve the protected release environment and inspect signatures, notarization, install/uninstall behavior, extension pairing, and clean-machine tests.

Never test signing with production keys on pull requests or upload certificates as artifacts.
