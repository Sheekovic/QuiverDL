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
Before publishing, set the final Chromium store ID in the native-host allowlist, rebuild the native
host packages, test explicit pairing, and confirm that the store privacy disclosures match
`docs/privacy/index.html`. QuiverDL never supplies cookies, headers, page contents, history, or
telemetry to the companion.

The tagged-release workflow attaches both unsigned submission archives and portable SHA-256 sums to
the draft release. Store signatures and owner credentials are deliberately not reusable from pull
requests.

## Microsoft Store

Tauri currently submits a linked offline EXE/MSI installer rather than generating an MSIX. The
overlay `apps/desktop/src-tauri/tauri.microsoftstore.conf.json` selects the required offline WebView2
installer and a publisher name distinct from the product name. After importing the owner's
Authenticode certificate, generate the ignored combined Store/signing overlay and build on Windows:

```powershell
npm ci --prefix apps/desktop
$thumbprint = 'REPLACE_WITH_THE_40_HEX_CERTIFICATE_THUMBPRINT'
.\scripts\prepare-microsoft-store-config.ps1 -CertificateThumbprint $thumbprint
npm run tauri --prefix apps/desktop -- build --no-bundle
npm run tauri --prefix apps/desktop -- bundle --bundles nsis,msi --config src-tauri/tauri.microsoftstore.release.conf.json
```

The preparation script validates the certificate's private key, code-signing usage, and expiry, then
injects its thumbprint, SHA-256 digest, and timestamp service without modifying the tracked overlay.
Verify every generated executable and MSI with `signtool verify /pa`, test offline
install/update/uninstall on a clean Windows 11 VM, run the Windows App Certification Kit, upload the
immutable installer to stable HTTPS hosting, and link that exact URL in Partner Center. MSI/EXE
Store submissions are not re-signed by Microsoft, so publication remains blocked until the
repository owner supplies a trusted signing certificate.

## Snap Store

`snap/snapcraft.yaml` builds the locked Tauri desktop project, unpacks its Debian bundle into a
strictly confined Snap, and declares only desktop integration, outbound network, home-directory,
removable-media, and single-instance D-Bus access. Removable-media access is not silently granted by
the package and may require the user to connect that interface. Browser native-messaging
registration remains a direct-package feature because strict confinement cannot write arbitrary
browser profile locations.

Build and test on Ubuntu before owner publication:

```bash
snapcraft
sudo snap install --dangerous ./quiverdl_0.1.0_amd64.snap
snap connections quiverdl
snap run quiverdl
```

Exercise downloads to the home directory, denied destinations, interruption/restart, notifications,
single-instance behavior, and an explicitly connected removable drive. After registering the name,
the owner can upload the tested artifact with `snapcraft upload --release=stable <artifact>.snap`.

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
working direct-download release.
