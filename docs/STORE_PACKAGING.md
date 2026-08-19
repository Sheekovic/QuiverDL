# Store packaging

QuiverDL keeps store preparation reproducible while leaving publication, identity verification, and
private signing material with the repository owner. Pull requests can validate every tracked input,
but they cannot publish a marketplace listing or mint a store signature.

## Browser stores

`node scripts/package-extensions.mjs` creates deterministic `quiverdl-chromium-<version>.zip` and
`quiverdl-firefox-<version>.zip` archives in `dist/store`. It packages only the corresponding source
directory, rejects symbolic links, sorts file names, writes fixed timestamps and permissions, and
validates Manifest V3, descriptions, versions, and Firefox's stable Gecko ID. Run:

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
installer and a publisher name distinct from the product name. Build on Windows after importing the
owner's Authenticode certificate:

```powershell
npm ci --prefix apps/desktop
npm run tauri --prefix apps/desktop -- build --no-bundle
npm run tauri --prefix apps/desktop -- bundle --bundles nsis,msi --config src-tauri/tauri.microsoftstore.conf.json
```

Verify the signatures with `signtool verify /pa`, test offline install/update/uninstall on a clean
Windows 11 VM, run the Windows App Certification Kit, upload the immutable installer to stable HTTPS
hosting, and link that exact URL in Partner Center. MSI/EXE Store submissions are not re-signed by
Microsoft, so publication remains blocked until the repository owner supplies a trusted signing
certificate.

## Mac App Store

The App Store overlay enables the Utility category, embeds a provisioning profile, and uses a
generated entitlement file. The template grants only the App Sandbox, outbound network access, and
read/write access to user-selected destinations. Browser native-messaging installation remains a
separate direct-download feature and is not smuggled into the sandboxed Store bundle.

Set `APPLE_TEAM_ID` and point `APPLE_PROVISIONING_PROFILE` at a Mac App Store Connect profile, then
materialize the ignored signing inputs and build on macOS:

```bash
node scripts/prepare-app-store.mjs
npm ci --prefix apps/desktop
npm run tauri --prefix apps/desktop -- build --no-bundle --target universal-apple-darwin
npm run tauri --prefix apps/desktop -- bundle --bundles app --target universal-apple-darwin --config src-tauri/tauri.appstore.conf.json
xcrun productbuild --sign "<Mac Installer Distribution identity>" \
  --component "target/universal-apple-darwin/release/bundle/macos/QuiverDL.app" /Applications QuiverDL.pkg
```

Confirm the built application is sandboxed, exercise user-selected destination access, inspect its
entitlements and signature, and upload the signed PKG with the owner's App Store Connect API key.
The generated entitlements and provisioning profile are ignored so neither can enter a commit.

## Release gate

For every marketplace, keep desktop, Chromium, and Firefox versions equal; validate the submitted
hash against the draft release; test a clean install and an upgrade from the previous public
version; and retain the marketplace review result. A rejected or unsigned artifact never replaces a
working direct-download release.
