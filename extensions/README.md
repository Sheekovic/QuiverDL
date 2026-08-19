# QuiverDL browser companions

The Chromium and Firefox folders are unpacked-development extensions. Both default to manual, user-initiated downloads through the link context menu. Automatic interception is disabled until the user opts in and is constrained by a minimum size and optional exact-domain allowlist.

## Local development

1. Build the host with `cargo build -p quiver-native-host --release`.
2. Load `extensions/chromium` as an unpacked extension or `extensions/firefox` as a temporary add-on.
3. Install the native manifest with the script for your platform: `native-host/install-windows.ps1`, `native-host/install-linux.sh`, or `native-host/install-macos.sh`. Chromium requires the generated extension ID.
4. In QuiverDL Settings, open **Browser extension setup** and copy the pairing token into the extension options.

The extension sends only the selected download URL and an optional filename. It never forwards cookies, authorization headers, page contents, browsing history, or telemetry. Browser requests remain in a local inbox until the user reviews them in QuiverDL.

Production store packages must replace the Chromium extension ID in the native-host allowlist. The macOS installer registers the host for per-user Chrome, Chrome for Testing, Chromium, and Firefox profiles.

## Store submission archives

Run `node scripts/package-extensions.mjs` from the repository root to create deterministic Chromium
and Firefox ZIP files under `dist/store`. The script rejects symlinks, validates the store-facing
manifest fields, fixes archive timestamps and permissions, and places `manifest.json` at the archive
root. CI builds the archives twice and compares their bytes.

The ZIP files are submission inputs, not directly installable production extensions. Chrome Web
Store applies its own signature. Firefox packages must be submitted to and signed by Mozilla before
installation. Marketplace credentials remain with the repository owner and are never accepted by
pull-request workflows. See [store packaging](../docs/STORE_PACKAGING.md) for the release procedure.
