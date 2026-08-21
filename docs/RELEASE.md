# Release process

QuiverDL releases are built from annotated `v*` tags by `.github/workflows/release.yml`. Release
Please opens a draft version PR after reviewed changes reach `main`; that PR must pass normal review
and CI. Merging the reviewed version PR updates the release manifest, and `version-tag.yml` creates
the matching annotated tag exactly once. The workspace-safe release strategy updates every declared
version file, then `sync-release-lockfile.mjs` deterministically updates the three local Cargo.lock
package entries in a follow-up release-branch commit; CI must pass on that final PR head. The
synchronizer always executes from the immutable triggering `main` commit without push credentials;
the credential is exposed only to the final hook-disabled push. Linux then uses locked Cargo and npm dependency graphs,
creates DEB, RPM, and a signed updater-enabled AppImage plus a native-host archive, verifies the
updater signature, attaches SHA-256 checksums, uploads `latest.json` last, and publishes the
validated draft. Pull requests that change the release workflow perform a complete unsigned Linux
bundle build without receiving signing credentials.

Signed direct-download Windows and macOS packages remain optional jobs. They run only when the repository variable `ENABLE_SIGNED_RELEASES` is set to `true`; Windows certificate and Apple Developer credentials are then required in the protected `release` environment. The Linux release does not wait for those credentials. Standalone macOS host archives are submitted to Apple notarization after the host is signed.

Release PR creation requires either a fine-grained `RELEASE_PLEASE_TOKEN` repository secret with
Contents, Pull requests, and Issues write access (recommended, because its commits can trigger follow-up CI),
or the repository's **Allow GitHub Actions to create and approve pull requests** setting. The
workflow falls back to `GITHUB_TOKEN` only when the dedicated secret is absent.

The same tagged workflow creates deterministic unsigned Chrome Web Store and Firefox AMO submission
archives. Microsoft Store and strict-confined Snap Store packaging, validation, and owner submission
commands are documented in [store packaging](STORE_PACKAGING.md). Store accounts and marketplace
signatures remain owner-controlled release inputs. Mac App Store packaging remains gated on durable
security-scoped bookmark support for restart-safe destinations.

Windows users who install the NSIS or MSI desktop package and want browser integration must also download the matching `native-host-x86_64-pc-windows-msvc.zip`, then run its `extensions/native-host/install-windows.ps1` script against the included signed host. The portable archive already contains the same host and registration assets.

## One-time repository setup

Create a protected GitHub environment named `release`. Add `WINDOWS_CERTIFICATE` (base64 PFX) and `WINDOWS_CERTIFICATE_PASSWORD`. Add `APPLE_CERTIFICATE` (base64 Developer ID Application P12), `APPLE_CERTIFICATE_PASSWORD`, `KEYCHAIN_PASSWORD`, `APPLE_ID`, `APPLE_PASSWORD` (app-specific password), and `APPLE_TEAM_ID`. Require maintainer approval for the environment.

Generate the permanent Tauri updater key offline with
`npm --prefix apps/desktop run tauri -- signer generate -w C:\secure\quiverdl-updater.key` and
choose a strong password interactively. Back up the private key and password separately. In the
protected `release` environment, add the private key contents as `TAURI_SIGNING_PRIVATE_KEY`, its
password as `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, and the public `.pub` file contents as the
environment variable `TAURI_UPDATER_PUBLIC_KEY`. Never paste the private key into an issue, pull
request, command log, repository variable, or artifact.

Add a fine-grained bot token as repository secret `RELEASE_PLEASE_TOKEN` with access only to this
repository and permission to write contents, pull requests, and issues. The Issues permission lets
Release Please manage its PR labels. This token also lets release PRs trigger normal CI; the workflow
never merges those PRs automatically.

Set the repository variable `ENABLE_SIGNED_RELEASES` to `true` only after those credentials are ready. The signed jobs deliberately fail instead of publishing unsigned Windows or unnotarized macOS release artifacts. Certificates and account credentials cannot be supplied by source code; the repository owner must obtain them from an appropriate certificate authority and Apple Developer account.

Direct-download update signing is a separate trust root from operating-system code signing. Follow
the [secure updater design](UPDATER.md): generate and back up the Tauri key offline, add the encrypted
private key only to the protected release environment, compile the public key into direct builds,
and publish `latest.json` only after every immutable artifact and signature passes release QA. Store
builds use their marketplace update channel.

## Cutting a release

1. Review and merge the draft Release Please PR only after CI and code review pass.
2. Confirm `version-tag.yml` creates the exact synchronized `v*` tag and the release workflow enters
   the protected environment.
3. Confirm the Linux workflow publishes AppImage, DEB, RPM, native-host, signature, checksum, and
   `latest.json` assets.
4. Perform a clean-machine Linux launch and AppImage upgrade test, then add any additional
   human-readable release notes.
5. When signed Windows or macOS direct downloads are enabled, inspect their OS signatures,
   notarization, install/uninstall behavior, extension pairing, and clean-machine tests.

Never test signing with production keys on pull requests or upload certificates as artifacts.
