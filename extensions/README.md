# QuiverDL browser companions

The Chromium and Firefox folders are unpacked-development extensions. Firefox connects automatically after QuiverDL is opened once from a package containing the native helper. New Firefox installations capture downloads by default, including downloads whose size is not yet known. The extension's **Download through QuiverDL** switch disables capture; saved opt-outs and custom size/domain rules are preserved. Firefox keeps a download if the native handoff fails. Chromium retains its manual pairing workflow.

## Local development

1. Build the host with `cargo build -p quiver-native-host --release`.
2. Load `extensions/chromium` as an unpacked extension or `extensions/firefox` as a temporary add-on.
3. Install the native manifest with the script for your platform: `native-host/install-windows.ps1`, `native-host/install-linux.sh`, or `native-host/install-macos.sh`. Chromium requires the generated extension ID.
4. For Firefox, open QuiverDL once with `quiver-native-host` beside the desktop executable. No pairing token is needed. Chromium development installations continue to use the token from the private native bridge configuration.

The extension sends only a download URL, an optional filename, and capture intent. It never forwards cookies, authorization headers, page contents, browsing history, or telemetry. Accepted Firefox requests launch QuiverDL and regular files enter its durable queue automatically. Torrents open a file-selection preview. Legacy manual requests remain available for review in the browser inbox.

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
