# BitTorrent threat model and adoption decision

## Status

**Evaluated and deferred.** BitTorrent is not approved for implementation in QuiverDL's desktop
process or HTTP engine. A future proposal may proceed only after satisfying the architecture,
privacy, parser, filesystem, and test entry gates in this document.

[BEP 3](https://www.bittorrent.org/beps/bep_0003.html) defines v1 metainfo, trackers, piece hashes,
and a peer protocol in which downloaders also upload. [BEP 52](https://www.bittorrent.org/beps/bep_0052.html)
defines v2 metainfo and SHA-256 Merkle roots. Magnet support obtains metadata from peers under
[BEP 9](https://www.bittorrent.org/beps/bep_0009.html), while public peer discovery can use the UDP
DHT described by [BEP 5](https://www.bittorrent.org/beps/bep_0005.html).

## Why this is a separate boundary

BitTorrent is not an alternate HTTP mirror. Joining a swarm can disclose the user's IP address and
content identifier to trackers, peers, and DHT nodes; receive connections; send downloaded pieces
to strangers; and keep network activity alive after the local file completes. It also adds
untrusted bencoded metadata, multi-file layouts, peer-wire messages, TCP and UDP transports, peer
selection, and upload accounting.

QuiverDL's direct/system/custom HTTP proxy setting cannot safely be presented as covering this
traffic. An HTTP proxy does not automatically route peer TCP, uTP, UDP trackers, DHT, local peer
discovery, or port mapping. BitTorrent must fail closed when the selected privacy policy cannot be
applied to every enabled transport.

## Assets and adversaries

Assets include destination files, unrelated local files, IP address and network location, tracker
passkeys embedded in URLs, magnet/torrent history, bandwidth, disk capacity, connection slots, and
the user's legal or policy expectations about uploading.

Untrusted parties include torrent publishers, trackers, DHT nodes, peers, web pages offering
magnets, and local processes racing filesystem operations. Any of them can send malformed lengths,
deep metadata, unsafe paths, inconsistent hashes, unsolicited messages, slow streams, duplicate
peers, or addresses aimed at local services.

## Metadata and parser requirements

- Parse bencoding with explicit limits on source bytes, nesting, collection entries, integer width,
  string length, file count, tracker count, piece count, and decoded total size. Reject non-canonical
  encodings rather than limiting that check to info-hash calculation. Require raw byte-string keys
  to be strictly increasing in every metainfo, tracker, and peer-message dictionary, which rejects
  duplicates at every nesting level. Preview and transfer code consume the same validated typed
  representation instead of reparsing with different duplicate semantics.
- Require `name` and every path component to be strict UTF-8 before preview. Preserve that one
  validated Unicode representation through normalization, collision checks, IPC, persistence, and
  handle-relative creation; never display replacement characters for bytes that would be written.
  Normalize to NFC once and reject Unicode format characters (`General_Category=Cf`), including
  bidirectional marks, embeddings, overrides, and isolates. The exact previewed normalized string
  is the only representation eligible for filesystem creation.
- For v2/hybrid input, require `piece length` to be a power of two from 16 KiB through 16 MiB.
  Compute per-file piece counts with checked ceiling division, validate every pieces root and exact
  piece-layer hash count against the declared file lengths, and reject missing, extra, or
  inconsistent hashes before allocation or verification.
- Treat names and paths as advisory. Reject absolute paths, traversal, empty components, drive/UNC
  prefixes, control characters, `/`, `\`, `<`, `>`, `:`, `"`, `|`, `?`, `*`,
  alternate-data-stream syntax, components ending in a dot or space, Windows device names even with
  extensions, and normalized or case-folded collisions on every platform.
- Bound every path component to at most 240 UTF-8 bytes and 240 UTF-16 code units, relative depth to
  32 components, and each encoded relative path to 2,048 bytes/code units before filesystem access.
- Reject duplicate normalized paths and any file path that is a strict component-prefix of another
  file path before allocation or reservation.
- Resolve every file beneath one canonical user-selected root. Reject symbolic-link semantics,
  padding-file surprises, special files, and any write that escapes or aliases another target.
- Retain a trusted root directory handle and perform no-follow, handle-relative traversal and
  creation on every platform. Rechecking a path before creation is not sufficient against a local
  symlink or reparse-point swap. Compare stable identity for existing components so short-name or
  filesystem-specific aliases cannot make selected paths share a target.
- Convert every file length losslessly to a non-negative `u64` before aggregation. Accumulate all
  file and selected-file sizes with checked arithmetic, reject totals above `u64::MAX`, and preserve
  byte counts losslessly across backend, IPC, persistence, and UI layers.
- For a local `.torrent` file, present the full file tree and exact selected byte total before
  allocation or network discovery.
- Magnet URIs may be parsed and displayed offline but cannot resolve metadata or enter the transfer
  queue in the first networked backend. Reject an encoded URI above 16 KiB, more than 128 query
  parameters, a decoded key above 64 bytes, or a decoded value above 4 KiB. Accept at most eight
  exact-topic values, 32 tracker values, and 32 web-seed values; reject malformed percent encoding,
  invalid UTF-8 text fields, and duplicate key/value pairs. Perform checked decoding into bounded
  buffers and add deterministic boundary tests. Because the private flag is unavailable before
  metadata retrieval, DHT, peer exchange, or public tracker discovery could disclose a private
  torrent too early. Magnet networking requires a later, separate threat model; private magnets
  remain unsupported until privacy can be established before discovery.
- Unselected files must not be materialized except for bounded piece-overlap staging that is clearly
  accounted for and removed safely.

## Content integrity

V1 torrents use SHA-1 piece hashes. Those hashes detect ordinary corruption but are not publisher
authentication and do not meet QuiverDL's strong-digest policy. A v1-only input may be parsed by the
offline inspector but cannot start discovery, allocate content, or download. The first eligible
networked backend requires v2 or a hybrid torrent with a valid v2 SHA-256 representation; hybrid
torrents must validate both representations and reject disagreement.

Every v2 piece must be verified against its SHA-256 Merkle data before it becomes eligible for
upload or final assembly. Failed piece bytes are discarded, peers repeatedly supplying bad data are
bounded and evicted, and completed files still use no-replace promotion. If a trusted independent
whole-file SHA-256 is available, QuiverDL also verifies it before completion. The UI must
distinguish **piece-verified** from **publisher-authenticated**.

Persisted verification flags are hints, not trust. After restart or crash recovery, hash every
piece's current bytes against the v2 Merkle data before restoring verified status or allowing those
bytes to be uploaded or assembled. Mark missing pieces incomplete and discard inconsistent state or
bytes without promoting them.

## Network, privacy, and consent requirements

- Before any network discovery or content request, an initial confirmation must explain that the
  user's IP address and torrent identifier can be visible, that downloading normally uploads pieces,
  which trackers and discovery mechanisms will run, the sanitized origin of every web seed, and when
  network activity will stop.
- No tracker or web-seed contact, DNS lookup, DHT lookup, local discovery, peer connection, listener,
  port mapping, or content request occurs before confirmation. Browser interception for `.torrent`
  and magnet links remains disabled until a separate reviewed integration milestone.
- The first networked backend accepts only HTTPS tracker URLs, optional HTTPS web-seed URLs, and
  outbound TCP peer connections learned from an approved tracker. Reject HTTP, FTP, file,
  WebSocket, UDP tracker, uTP, and every unknown or unreviewed scheme/transport before dispatch.
  Each additional tracker, web-seed, or peer transport requires its own later review.
- Resolve and classify every tracker, web-seed, direct-peer, DHT, peer-exchange, and local-discovery
  address before connecting. Loopback, link-local, private, and other special-use destinations are
  denied by default; metadata cannot grant access to them. Any local-network exception is an
  explicit per-torrent approval, and mixed public/private DNS answers fail closed.
- HTTP tracker and web-seed redirects are bounded and re-enter address classification on every hop.
  They must remain HTTPS and same-origin on every hop; cross-origin redirects fail closed so an
  unconfirmed tracker or seed never receives the torrent identifier or request. Resolution and the
  actual socket destination remain bound to the approved address class so DNS rebinding cannot
  bypass the decision.
- Incoming listeners, DHT, peer exchange, local service discovery, UPnP/NAT-PMP/PCP, UDP trackers,
  and seeding after completion are individually modeled features, not implicit defaults.
- Connections, peers, pending requests, message sizes, metadata bytes, retries, timeouts, upload
  rate, download rate, and share duration are bounded globally and per torrent.
- Tracker, web-seed, and magnet URLs may contain private passkeys. Redact them from errors and logs, never
  include them in public diagnostics, and do not persist an offline-inspected magnet. The
  confirmation UI displays only a sanitized tracker or web-seed identity (scheme and host, plus a
  non-default port); it strips userinfo, path, query, and fragment so passkeys cannot appear on screen
  or in screenshots. Store full tracker and web-seed URLs only in protected local state.
- Private torrents must follow [BEP 27](https://www.bittorrent.org/beps/bep_0027.html): contact only
  the private tracker and peers it returns, with DHT, peer exchange, and local discovery disabled.
- Pause and quit semantics must say whether announcing, uploading, and listeners stop. **Stopped**
  means no torrent network activity remains.

## Architecture entry gates

Before implementation begins, a proposal must:

1. Select a maintained, license-compatible Rust backend or justify a new one with dependency,
   advisory, fuzzing, and maintenance evidence.
2. Keep torrent state and networking outside `quiver-core`'s HTTP engine, behind a narrow backend
   interface with cancellation and bounded event delivery. Process isolation is preferred if the
   selected library cannot enforce the required limits itself.
3. Define a versioned persistence format that never confuses torrent pieces with HTTP recovery
   sidecars, rehashes recovered piece bytes before trusting them, and can recover safely after
   crashes.
4. Specify transport-by-transport proxy behavior. Any enabled transport that bypasses the chosen
   route must be disabled or startup fails closed; user consent does not turn a routing bypass into
   enforcement of the selected privacy policy.
5. Provide deterministic parser and filesystem tests plus local fake tracker/peer integration tests
   for malicious messages, duplicate/unsorted keys at every dictionary layer, per-file and aggregate
   overflow, negative lengths, zero/out-of-range piece lengths, piece-count and Merkle-layer
   mismatches, invalid UTF-8 paths, Unicode format/bidirectional controls, recovered-piece
   tampering, all Windows-invalid characters, component/path length edges, file/directory prefix
   conflicts, adversarial symlink/reparse-point swaps, trailing-dot/space and
   alternate-data-stream aliases, scheme rejection, hash failure, cancellation, private torrents,
   special-use addresses, mixed-address DNS answers, downgrade/cross-origin redirects, DNS
   rebinding, and resource exhaustion.
6. Complete dependency, privacy, legal-disclosure, accessibility, and cross-platform security
   review before enabling the backend in production builds.

The first acceptable experiment is an offline, bounded `.torrent` inspector that performs no DNS
or network activity. A networked client, magnet resolution, DHT, peer exchange, local discovery,
incoming ports, and automatic browser capture are later and separately reviewed milestones.
