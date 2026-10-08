# Store packaging

QuiverDL keeps store preparation reproducible while leaving publication, identity verification, and
private signing material with the repository owner. Pull requests can validate every tracked input,
but they cannot publish a marketplace listing or mint a store signature.

## Browser stores

`node scripts/package-extensions.mjs` creates deterministic `quiverdl-chromium-<version>.zip` and
`quiverdl-firefox-<version>.zip` archives in `dist/store`. It packages only the corresponding source
directory, rejects symbolic links, sorts file names, writes fixed timestamps and permissions, and
validates Manifest V3, descriptions, versions, Firefox's stable Gecko ID, and its mandatory
declaration that the companion collects and transmits no data. Run:

```powershell
node scripts/validate-store-packaging.mjs
node --test scripts/test-package-extensions.mjs
node scripts/package-extensions.mjs
```

Upload the Chromium ZIP through the Chrome Web Store developer dashboard. Upload the Firefox ZIP
through addons.mozilla.org or sign it with `web-ext sign`; an unsigned ZIP is not a production XPI.
The owner must also supply the marketplace listing assets outside the ZIP, including the required
Chrome screenshot and promotional tile, and keep the homepage and privacy-policy URLs current.
Before publishing, set the final Chromium store ID in the native-host allowlist, rebuild the native
host packages, test automatic connection, and confirm that the store privacy disclosures match
`docs/privacy/index.html`. QuiverDL never supplies cookies, headers, page contents, history, or
telemetry to the companion.

The tagged-release workflow attaches both unsigned submission archives and portable SHA-256 sums to
the draft release. Store signatures and owner credentials are deliberately not reusable from pull
requests.

## Microsoft Store

QuiverDL has a dedicated full-trust x64 MSIX package for Microsoft Store submission. Its manifest is
bound to the public Partner Center identity. The Store package requires Windows 11 because WebView2
is part of that operating system; Windows 10 users remain supported by the direct EXE/MSI packages,
which provision the Evergreen WebView2 Runtime:

- Package identity: `SHEEKOVIC.QuiverDL`
- Publisher: `CN=BC484461-F987-4E7B-82B4-47D7995725CA`
- Publisher display name: `SHEEKOVIC`
- Package family name: `SHEEKOVIC.QuiverDL_x7yre5s1hmnca`
- Store ID: `9MVB2DD54NF4`

Install the locked frontend dependencies once, then build and validate the package on Windows:

```powershell
npm ci --prefix apps/desktop
.\scripts\package-msix.ps1
```

The script builds the Tauri executable, renders the reserved four-part Store version, packages it
with the required assets, asks MakeAppx to perform semantic validation, unpacks the result, and
checks that the packaged executable has the same SHA-256 hash. The result is written to `dist/store`.
Microsoft requires a nonzero package major version, so the monotonic mapping adds one to QuiverDL's
SemVer major: desktop `0.1.0` becomes Store package `1.1.0.0`, while `1.0.0` becomes `2.0.0.0`.
It is intentionally unsigned: Partner Center accepts an unsigned MSIX and Microsoft signs the
certified package for Store distribution. Upload the `.msix` under **Packages** in the app submission;
do not use the PFN, Package SID, or Store ID as signing secrets.

The **Microsoft Store MSIX** GitHub Actions workflow accepts only an existing annotated version tag
whose version matches every package and whose commit is contained in `main`. It builds the same
validated package, records and rechecks its SHA-256 digest, and retains both files as a private
workflow artifact for 30 days. A version-tag event authenticates with the dedicated Partner Center
application and submits the package to Microsoft certification automatically. A manual run verifies
the package, credentials, and product access without changing the Store unless its explicit
**Publish** input is enabled. Immediately before either publication path, the workflow fetches the
remote annotated tag again and refuses submission if it was deleted or no longer resolves to the
commit that passed preflight. Concurrency keys include both the release tag and operation, so a
duplicate run can replace only the same tag's same intent; a manual verification or newer release
cannot silently discard a queued Store submission.

The publishing job uses the `microsoft-store` GitHub environment and these encrypted secrets:

- `AZURE_AD_TENANT_ID`
- `SELLER_ID`
- `AZURE_AD_APPLICATION_CLIENT_ID`
- `AZURE_AD_APPLICATION_SECRET`

The Entra application must have only the Partner Center **Manager (Windows)** role. Protect the
environment so only `main` and `v*` tags can deploy, rotate the client secret before it expires, and
never place any credential in a workflow input, artifact, log, pull request, or repository file.
Before creating a release tag, run the Windows App Certification Kit and test Store installation,
launch, download, resume, and uninstall behavior on a clean Windows 11 VM. Microsoft still performs
certification and controls when an accepted update becomes available to Store users.

The existing `tauri.microsoftstore.conf.json` and `prepare-microsoft-store-config.ps1` remain only
for the alternative linked EXE/MSI Store route. That route still needs the owner's Authenticode
certificate because Microsoft does not re-sign linked installers. Browser native-messaging
registration is not included in the MSIX: Store installations use the automatic localhost
connector described below, while direct installations retain native messaging.

## Snap Store

`snap/snapcraft.yaml` builds the locked Tauri desktop project, unpacks its Debian bundle into a
strictly confined Snap, and declares only desktop integration, outbound network, home-directory,
removable-media, and single-instance D-Bus access. Removable-media access is not silently granted by
the package and may require the user to connect that interface. Browser native-messaging
registration remains a direct-package feature because strict confinement cannot write arbitrary
browser profile locations. Authenticated proxy credentials use Secret Service through the declared
`password-manager-service` interface; if the Store does not auto-connect it, the user must approve
that connection before saving a proxy password.

Build and test on Ubuntu before owner publication:

```bash
snapcraft
sudo snap install --dangerous ./quiverdl_0.1.0_amd64.snap
sudo snap connect quiverdl:password-manager-service
snap connections quiverdl
snap run quiverdl
```

Exercise downloads to the home directory, denied destinations, interruption/restart, notifications,
single-instance behavior, authenticated-proxy credential save/load, and an explicitly connected
removable drive. After registering the name, the owner can upload the tested artifact with
`snapcraft upload --release=stable <artifact>.snap`.

## Mac App Store status

Mac App Store packaging is intentionally not emitted yet. The App Sandbox gives a save panel only
temporary access to arbitrary user-selected destinations, while QuiverDL persists paths and resumes
them after relaunch. A Store build would therefore lose access to queued destinations unless it
persisted and reacquired security-scoped bookmarks. Add and test that platform boundary before
creating App Sandbox entitlements or a Mac App Store package; direct signed/notarized DMG builds
remain supported.

## Release gate

For every marketplace, keep desktop, Chromium, and Firefox versions equal; validate the submitted
hash against the draft release; test a clean install and an upgrade from the previous public
version; and retain the marketplace review result. A rejected or unsigned artifact never replaces a
working direct-download release. The tagged workflow also rejects a tag whose normalized version
does not exactly match the validated desktop and extension versions.
# Packaging recovery and desktop integration

Manual Store runs on `main` build application code from the validated immutable release
tag, and use the packaging script and MSIX manifest from the immutable workflow commit.
This allows a packaging-only correction without changing an already published release tag.
The automatic browser transport requires a new application release and a matching Firefox
extension release. It cannot be applied to the old v0.4.0 binary as a packaging-only fix.
The packager always builds the selected source and does not accept external executables or
a skip-build option. It rejects source trees without the automatic transport, includes configured Tauri
resources, and verifies their hashes after unpacking. It does not ship a native browser helper.

The Store manifest declares `.torrent` and `magnet:` handlers with quoted command-line
arguments. It requests `runFullTrust` for the desktop application but does not request
`unvirtualizedResources`, exclude AppData or registry locations from virtualization, or
register Firefox native messaging. Package identity is detected with
`GetCurrentPackageFullName`; packaged instances use the automatic transport, and unpackaged
instances retain their native host integration. Store browser state lives under
`%LOCALAPPDATA%\Packages\<package-family>\LocalState\BrowserBridge`, separately from
any old shared `%APPDATA%\QuiverDL` files.

Firefox connects automatically with its declared IPv4 localhost host permission. No code
copying or Connect step is needed. Keep QuiverDL running, including hidden in the tray.
It listens only on `127.0.0.1:47831`; settings report port conflicts. No loopback exemption
utility is invoked. Explicitly quitting QuiverDL leaves new downloads in Firefox. A revoked
or missing permission can be restored through Check connection in the extension settings.

A guarded bootstrap establishes a session in memory. Custom headers, Host, Origin, content
type, body size, message rate, URLs, and queue limits are checked; no CORS access is granted.
This blocks ordinary websites, not other privileged extensions or local programs. The
limited API does not expose files, history, cookies, credentials, or arbitrary commands.
An acknowledgment follows the durable inbox write. Once Store transport is selected,
failures never send the download to a different native installation. Session expiration or
an app restart reconnects automatically; ambiguous enqueue failures are not retried.

Before submission, test a clean Windows 11 MSIX install with normal Firefox and Store
Firefox: connect automatically, enqueue with the window open and hidden, quit and verify Firefox retains its
download, restart and confirm automatic reconnection, test upgrade and uninstall, and verify the Firefox key
and shared AppData helper are not created. Check the final package manifest and submitted
artifact hash. Local unit/integration tests do not establish Store approval or replace this
installed-package verification. Old unvirtualized files from previous builds can remain;
this change does not delete user data or modify a direct installation's registration.

For migration or coexistence, Firefox settings provide an explicit installation choice.
Direct installation selects native messaging; selecting automatic detection again clears the
previous Store selection. Ordinary connection failures never change this choice automatically.
