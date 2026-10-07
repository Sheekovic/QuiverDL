# Threat model

## Security goals

QuiverDL must not write outside a user-selected destination, silently replace an existing file, append bytes from a changed remote object, expose private download data, or let an unauthenticated browser process enqueue work. A completed file must contain exactly the validated response bytes and, when supplied, match the expected SHA-256 digest.

## Trust boundaries and threats

### Remote download servers and networks

Servers, redirects, headers, lengths, range responses, and filenames are untrusted. The engine accepts only HTTP(S), limits redirects, parses byte counts as `u64`, sanitizes suggested filenames, and never treats a filename as a path. Resume requires range support, a matching ETag or Last-Modified validator, a matching stored URL and total, and an exact returned `Content-Range`. Parallel segments additionally require a known size and validator; every start, end, total, and merged byte count is checked before hashing and atomic promotion.

TLS protects HTTPS transport but does not make a server trustworthy. SHA-256 verifies content only when the user or publisher supplies an expected digest through a trusted channel.

### Proxy routing and credentials

Direct routing is the default. System proxy mode delegates discovery and credential handling to the
operating system and HTTP client. Custom mode accepts only an HTTP(S) endpoint without embedded
credentials, paths, queries, or fragments. A bounded bypass list is applied by the HTTP client.

Custom proxy passwords are stored under a fixed QuiverDL service entry in Windows Credential
Manager, macOS Keychain, or Linux Secret Service. Passwords are never serialized into `state.json`,
recovery sidecars, logs, errors, or URLs, and the backend does not return a stored password to the
webview. Usernames and proxy endpoints are not secrets and remain in application settings. The UI
must explicitly replace or remove the stored credential. Each credential is bound to the normalized
proxy endpoint and username so it cannot be forwarded to a different proxy after an edit.

An HTTP proxy can observe and modify plain HTTP traffic. For HTTPS destinations, a proxy observes
connection metadata and can deny or disrupt connections, but destination TLS still protects content
unless the operating system trusts a proxy-controlled interception certificate. Proxy compromise,
malicious local root certificates, PAC scripts, SOCKS routing, and credential-store compromise are
outside the current proxy feature's guarantees.

### Local filesystem and other processes

Destinations must be absolute, parents are canonicalized, recovery sidecars are reserved against concurrent transfers, and completed files use no-replace atomic promotion. A local process running as the same user can still modify application files, partials, settings, or the destination; defending against a fully compromised user account is out of scope. Queue and bridge state use bounded JSON and atomic replacement. Release signing helps detect distribution tampering but does not protect a compromised running account.

### Desktop webview and IPC

The bundled UI is trusted application code. Tauri capabilities expose only the required dialog, notification, opener, and command surface to the main window. Backend commands revalidate identifiers, URLs, destinations, settings, and byte counts rather than trusting TypeScript types. Remote content is never rendered in the webview.

### Browser extension and native messaging

Native messages are length-prefixed and capped at 1 MiB. Firefox's host manifest permits only `quiverdl@quiverdl.app`; the host additionally checks Firefox's extension-ID argument before accepting tokenless enqueue/ping messages. Other invocations still require a random 256-bit pairing token compared in constant time. HTTP(S) and magnet requests become bounded inbox files with generated identifiers. Tokens are never written to inbox items or logs. An accepted Firefox request starts only the desktop executable registered locally by QuiverDL; browser messages cannot choose a program or pass shell arguments.

New Firefox installations capture downloads automatically, with a visible opt-out and optional size/domain filters. Existing saved settings are respected. Chromium keeps its prior opt-in policy. Browser cancellation happens only after an acknowledged queue write; failed desktop launch retains the browser download. Regular Firefox requests start through the durable queue, while torrents require file preview and confirmation. The extension does not transmit cookies, authorization headers, page contents, history, or telemetry. URLs themselves can contain secrets; the local queue remains private.

### Store browser transport

MSIX instances use an automatic IPv4-loopback HTTP endpoint instead of installing a native
host. Firefox's declared localhost permission allows the background extension to establish
an in-memory session without user-entered codes. Sessions expire after one hour and rotate
on restart. This is not exclusive addon authentication: other localhost-permitted extensions
and local programs can connect or impersonate the service. No browser cookies or authorization
headers are forwarded. The API exposes only session establishment and ping/enqueue.

Every endpoint requires an exact numeric Host, JSON content type, and a non-safelisted
`X-QuiverDL-Connector` header. Ordinary website and null origins are rejected; no CORS
permission is granted. Session authorization uses constant-time comparison. Body reads
are bounded and timed; message rate and pending queue size are capped. Requests cannot
launch executables, read files/history, or choose arbitrary destination paths. Queue writes
are durable before acknowledgment. Extension responses are bounded while streaming.

The extension remembers a selected Store transport, rejects redirects, and never switches
to native messaging after a Store failure. Only initial discovery can select native messaging
when the localhost service is unreachable. An explicit pre-mutation 401 permits one automatic
session refresh; ambiguous handoffs are never retried. Download IDs and creation times already attempted are
remembered (bounded to the latest 1000) to suppress repeat Firefox change events, including
a background restart. There is no general durable idempotency protocol: after a lost
acknowledgment Firefox retains its copy and QuiverDL may already have accepted the request.
QuiverDL must remain running in the tray. See [the research](BROWSER_INTEGRATION_RESEARCH.md).

### Distributed source metadata

Metalink and BitTorrent use separate trust boundaries, documented in
[DISTRIBUTED_SOURCES.md](DISTRIBUTED_SOURCES.md).

A future Metalink implementation may provide HTTP(S) mirror fallback only after bounded XML
parsing, complete preview before listed-mirror requests, strong whole-file digest requirements,
path containment, and extended redirect/address policies are enforced. Metalink metadata and its
hashes do not authenticate the publisher by themselves. See
[METALINK_THREAT_MODEL.md](METALINK_THREAT_MODEL.md).

The constrained BitTorrent adapter requires per-transfer consent and Direct connection mode. It
disables DHT, local discovery, incoming listeners, uploading, and seeding after completion; tracker
and outbound peer traffic can still reveal the user's IP address and swarm identifier. HTTP proxy
settings are never presented as covering that traffic. See
[BITTORRENT_THREAT_MODEL.md](BITTORRENT_THREAT_MODEL.md).

### Updates, dependencies, and release pipeline

Pull requests run formatting, linting, tests on major desktop platforms, frontend builds, extension syntax checks, and dependency lockfiles. Tag releases require protected-environment signing secrets, Windows Authenticode signing, macOS Developer ID signing/notarization, and checksummed artifacts. Maintainer accounts, GitHub Actions dependencies, certificate authorities, and package registries remain supply-chain dependencies. Branch protection, least-privilege workflow permissions, review, Dependabot, and draft release inspection reduce that risk.

The direct updater remains disabled until its separate key and recovery gates are satisfied. Its
design requires a fixed HTTPS manifest endpoint, immutable release URLs, mandatory Tauri signatures
plus platform signatures, strictly increasing versions, no unsigned fallback, and a locally cached
previous package for rollback. A valid historical signature cannot authorize a network downgrade.
Store builds use store-managed updates. See [the updater design](UPDATER.md) for the full trust,
rollback, and key-rotation boundary.

## Availability and resource limits

Retries, redirects, segments, per-host connections, queue length, message size, and settings are bounded. Speed limits are cooperative schedulers. A malicious server can remain slow, consume the configured partial-file disk space, or repeatedly return transient failures; users can cancel, and QuiverDL preserves resumable data when safe. Disk exhaustion and operating-system termination cannot be fully prevented.

## Out of scope

- Malware intentionally downloaded and executed by the user
- A compromised operating system, browser, GitHub maintainer, or user account
- DRM, paywall, authentication bypass, or copying content without authorization
- Anonymity from servers, networks, DNS providers, or the user’s ISP
- Guaranteed recovery when a server changes content without supplying validators

Security assumptions and mitigations must be revisited before adding origin credential forwarding,
proxy scripting, remote control APIs, enabling the designed automatic updater, plugin execution,
Metalink parsing, or any peer-to-peer transport.
