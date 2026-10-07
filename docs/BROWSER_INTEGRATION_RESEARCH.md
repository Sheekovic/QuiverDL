# Automatic Firefox integration for the MSIX edition

Research date: 2026-10-07. Recommendation: keep MSIX and the tray app, replace manual code pairing with automatic localhost discovery and a narrowly scoped download handoff API. This is a researched design supported by isolated experiments, not a completed production implementation or a Store certification result.

## Why the rejected capability was added

The certification report rejected `unvirtualizedResources` under policy 10.6.3. The native messaging integration needed Firefox to discover a registry entry and launch a helper whose manifest and shared files were outside MSIX virtualization. The previous packaging change requested exclusions to make those resources visible.

This is a real interoperability problem, not a need for users to copy codes. Mozilla tracks [MSIX native messaging discovery in bug 1901373](https://bugzilla.mozilla.org/show_bug.cgi?id=1901373). Firefox's current [NativeManifests implementation](https://searchfox.org/firefox-main/source/toolkit/components/extensions/NativeManifests.sys.mjs) looks up Windows native hosts through the registry. Microsoft documents that [flexible virtualization exclusions require the restricted capability](https://learn.microsoft.com/en-us/windows/msix/desktop/flexible-virtualization).

Manual pairing was a design choice in the initial localhost patch, not an MSIX requirement. This research supersedes that user-facing pairing proposal. The existing unpublished patch still contains the pairing UI and must be revised before release.

## Proposed user experience

1. Install QuiverDL and its Firefox extension, accepting the normal browser installation permissions.
2. Run QuiverDL; it remains available in the tray as already agreed.
3. The extension detects the local service and establishes a session automatically. No code, account, settings entry, or separate Connect step.
4. A download selected for QuiverDL is handed over. Firefox retains responsibility until QuiverDL acknowledges that the request has been durably queued.

Keep automatic capture subject to the user's capture setting. Reconnect automatically after an app restart. If the app is unavailable, preserve the original browser download and explain the unavailable state without asking for a pairing code.

Declare the specific localhost host permission in the extension manifest. Firefox gives background extension requests privileges unavailable to normal webpages. Installation prompts, revoked permissions, and existing-profile upgrade behavior still need handling; automatic connection does not mean bypassing Firefox consent. See [Mozilla's host permission documentation](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/manifest.json/host_permissions).

## How the connection should work

The packaged desktop process listens only on numeric `127.0.0.1`, using the existing proposed port 47831. Require the exact expected Host header and reject mismatches. Never bind to all interfaces.

The extension background context sends a JSON POST carrying a non-safelisted connector header. The service grants no CORS access, denies preflight requests, and rejects requests missing that header or the expected content type before performing any action. A normal webpage cannot send the required combination without a successful preflight. A `no-cors` request cannot retain the required custom header. Reject ordinary HTTP(S) origins as another check; an extension-shaped origin alone is not authentication. [MDN explains these browser-enforced CORS rules](https://developer.mozilla.org/en-US/docs/Web/HTTP/Guides/CORS).

Do not rely on missing CORS response headers alone: simple cross-origin requests can still reach and mutate a server. Do not implement mutation through GET, form submissions, a permissive fallback endpoint, or a CORS-enabled WebSocket substitute.

The protected bootstrap endpoint can return a random, short-lived session credential which the extension uses automatically. Rotate it on service restart and keep it out of logs. This is session management, not proof of exclusive extension identity. Do not use a public secret embedded in the extension, or trust whichever client connects first.

There is a maintained precedent: Zotero's [connector HTTP server documentation](https://www.zotero.org/support/dev/client_coding/connector_http_server), [server implementation](https://github.com/zotero/zotero/blob/main/chrome/content/zotero/xpcom/server/server.js), and [browser connector](https://github.com/zotero/zotero-connectors/blob/master/src/common/connector.js) show localhost handoff with connector headers and server-side request checks. This is architectural evidence, not a proposal to copy its licensed source or assume all its endpoints fit QuiverDL.

## Security boundary and API limits

This design separates ordinary websites from extension background requests. It does **not** authenticate the exact QuiverDL extension against other extensions with localhost access or native programs running as the user. A malicious local process could also occupy the expected port and impersonate the app. A public connector header, claimed addon ID, or automatically obtained token cannot solve that identity problem.

Consequently the initial API should expose only protocol health, session establishment, and enqueue acknowledgements. It must not expose download history, file contents, filesystem browsing, shell execution, credential retrieval, arbitrary destination paths, or general application control. Limit accepted schemes to supported HTTP(S) and magnet inputs; validate inputs, cap request size and queue rate, and keep destinations under app control. Preserve torrent confirmation behavior. Do not send browser cookies or authorization headers in the initial design; authenticated download forwarding needs a separately reviewed trust design. Never log signed URLs or session credentials.

Use client-generated request IDs and durable deduplication. A retry after a lost acknowledgement must not create a second download. Acknowledge only after queue persistence. Do not switch to a different installed transport after an ambiguous acknowledgement. Test redirects and download interception so that external page content cannot accidentally gain a general bridge into privileged requests.

If the product requires Firefox-enforced **exact-addon identity**, prefer native messaging through a conventional installer instead. Native manifests support an [allowed_extensions list](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/Native_manifests). Automatic localhost bootstrap cannot reproduce that identity guarantee.

## Experiments performed

### Real Firefox, isolated profile

Used Firefox 157.0.1, geckodriver 0.37.1, a fresh temporary WebDriver profile, temporary test extensions, and an original local HTTP fixture. No normal browsing profile, real download, saved credential, or existing extension was used.

| Test | Observed result |
| --- | --- |
| Test extension with localhost host permission | Automatic bootstrap and session-authenticated enqueue succeeded without a code |
| Different test extension with the same permission | Also succeeded, confirming the exact-addon identity limitation |
| Ordinary webpage, custom-header CORS request | Preflight denied; no accepted command |
| Ordinary webpage, no-cors request | Required header absent; command rejected |
| Sandboxed iframe with opaque/null origin | Both request variants failed to submit an accepted command |

Total accepted commands from the website tests: zero. Firefox supplied a distinct `moz-extension://` UUID for each extension; it was not the stable public addon ID.

Local reproduction material is in `dist/browser-research/probe.py`; observed output is in `dist/browser-research/results.json`. These are ignored research artifacts, not committed test coverage. The fixture proves the tested browser mechanics, not the production QuiverDL download lifecycle or security against every browser attack.

### Registered MSIX process

Built a minimal Rust listener and temporarily registered a separate development package named `QuiverDL.BrowserResearch.20261007`. Its manifest used QuiverDL's `packagedClassicApp` / `mediumIL` settings and only `runFullTrust`. Activated it through the Windows app activation API, verified package identity inside the process, and connected from an unpackaged local client.

Observed: package identity confirmed, localhost connection succeeded, and no loopback exemption was used. The temporary package was removed afterward and removal verified. No test certificate, firewall exception, magnet association, or changes to the normal QuiverDL installation were needed.

Local evidence: `dist/browser-research/msix_probe.rs`, `test-msix.ps1`, and `msix-results.json`.

This matters because a medium-integrity packaged desktop process must not be assumed to have the same network restrictions as an AppContainer process. See Microsoft's [packaged desktop application behavior](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-behind-the-scenes). The test covers the current Windows environment; it does not substitute for testing the signed production package or Store review.

## Alternatives considered

| Approach | Assessment |
| --- | --- |
| MSIX registry exclusions for native messaging | Reintroduces the capability Microsoft rejected; not the preferred resubmission path |
| MSIX package registry or app-extension declaration alone | Current Firefox registry lookup does not establish discovery through an MSIX app-extension catalog |
| Automatic localhost connector | Recommended with the limited trust boundary above; keeps MSIX and avoids manual codes |
| Custom URL scheme | Useful for explicit Open in app actions, but not a silent authenticated bridge; Firefox added external protocol prompts for security ([Mozilla bug 1792138](https://bugzilla.mozilla.org/show_bug.cgi?id=1792138)) |
| Static secret shipped inside our extension | Publicly recoverable and therefore not exclusive authentication |
| Separate helper installer | Adds installation and update lifecycle work; undermines the desired single Store installation flow |
| Store-distributed EXE/MSI with native messaging | Supported fallback when exact-addon identity matters more than retaining MSIX |

Microsoft explicitly supports [Store distribution of conventional Win32 EXE/MSI applications](https://learn.microsoft.com/en-us/windows/apps/distribute-through-store/how-to-distribute-your-win32-app-through-microsoft-store). That route can register the native host during ordinary installation. It brings publisher-managed hosting and updates, and [package requirements](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msi/app-package-requirements) including trusted signing, versioned installer URLs, and silent offline installation. Signing readiness and certification have not been verified for that alternative.

## Production changes and remaining validation

Revise the existing local patch rather than shipping its manual pairing experience:

- Replace copy/reset-code UI and extension pairing settings with automatic connection status.
- Replace the optional localhost permission flow with an appropriate declared host permission, handling existing installations and revocation explicitly.
- Add guarded automatic bootstrap, short-lived sessions, strict request checks, and the limited enqueue contract above.
- Keep the removal of `unvirtualizedResources` and the package-private data location. Preserve the existing native transport for conventional installations where applicable.
- Verify the actual signed MSIX plus the actual Firefox extension completes a real download before claiming integration works. Cover ordinary and Store-installed Firefox, restart/reconnect, unavailable app, permission revocation, port conflict, duplicate retries, and direct/Store installations coexisting.

The research answers feasibility: user-entered pairing codes are unnecessary for a browser-protected, download-only localhost connector. The implementation still needs revision and production validation, and Microsoft alone determines certification acceptance.

## Implementation update

The local patch now removes manual pairing, adds guarded automatic sessions, remembers the
selected Store transport, limits message rate and response size, and suppresses repeated
Firefox events after an attempted handoff. The research above records the original design.
The first implementation deliberately does not retry ambiguous enqueues; general durable
request-ID deduplication remains deferred. A lost acknowledgment can leave both the accepted
QuiverDL transfer and Firefox's retained copy. Only a pre-mutation 401 is retried automatically.

### Full application validation

A subsequent end-to-end test used the actual desktop binary with embedded frontend assets,
a separate temporary registered MSIX identity, and Firefox 157.0.1 in a fresh profile. The
production extension scripts were unmodified; an additional test-only script initiated the
fixture download and reported browser state. The app ran with a hidden window and an isolated
application identifier/settings directory. A 2,000,000-byte HTTP fixture was captured
automatically, saved by QuiverDL, and matched the expected SHA-256. Firefox reported
`interrupted` / `USER_CANCELED` after the bridge acknowledgement and stored the automatic
Store transport selection. The temporary package was removed after the test.

Local evidence: `dist/browser-research/e2e.py`, `test-e2e.ps1`, and `e2e-results.json`. This
was a developer-registered debug package, not the signed Store release or Store-installed
Firefox. Those release-environment checks and Store certification remain outstanding.

### Final local checks

Passed: 109 Rust tests across the workspace; six extension transport tests; browser capture
policy tests (including ambiguous replies and background restart); deterministic extension
packaging; frontend dependency install/build; desktop cargo check; Rust formatting; strict
workspace Clippy using the already-installed Rust 1.98.0 toolchain; and MakeAppx pack/unpack
with payload hash validation. The default stable toolchain lacks clippy-driver.exe, so its
Clippy invocation could not run; no default toolchain setting was changed.

Preview artifacts are under `dist/automatic-connector-preview`. The MSIX is unsigned and
contains a debug binary; the extension ZIP is unsigned. These are validation artifacts, not
release-ready uploads. No Store submission or remote publication was performed.
