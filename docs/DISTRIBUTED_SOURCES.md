# Distributed source evaluation

Metalink and BitTorrent can both describe more than one source for content, but they do not share a
trust boundary. QuiverDL will treat them as separate backends with separate user consent, state,
network policy, and security review.

| Area | Metalink | BitTorrent |
| --- | --- | --- |
| Primary input | RFC 5854 XML; RFC 6249 is evaluated but deferred | Remote `.torrent` URLs and magnet links after explicit consent |
| Network model | Known HTTP(S) mirrors | Trackers and many untrusted peers over additional protocols |
| Integrity | Publisher-provided size and SHA-256 | BitTorrent piece hashes detect corruption but do not authenticate the publisher |
| Privacy change | Mirror operators learn requests | Trackers and peers can learn the user's IP address and swarm identifier |
| QuiverDL decision | Approved for a bounded HTTP-only implementation | Constrained direct-only adapter; advanced transports remain deferred |

The detailed decisions live in [METALINK_THREAT_MODEL.md](METALINK_THREAT_MODEL.md) and
[BITTORRENT_THREAT_MODEL.md](BITTORRENT_THREAT_MODEL.md).

## Shared rules

- Metadata is untrusted even when it contains hashes. A hash from the same untrusted document
  detects inconsistent bytes; it does not authenticate the publisher.
- Metadata must be bounded before parsing and must not choose an absolute destination, escape a
  user-selected root, overwrite an existing file, or create symbolic or hard links.
- URLs, tracker addresses, peer addresses, filenames, hashes, and failure messages must not be
  logged or included in telemetry. QuiverDL has no telemetry.
- New protocols must have explicit queue states and resumable state formats. An HTTP recovery
  sidecar must never be interpreted as peer-to-peer state, or the reverse.
- Browser integration must remain manual until the relevant backend has shipped and been reviewed.
  Metalink requires a complete plan before mirror requests. Clipboard detection may offer a magnet
  for review, but it cannot start torrent networking or bypass the confirmation screen.
- A protocol backend may not weaken destination reservations, no-replace promotion, cancellation,
  resource limits, or final integrity verification.

## Sequencing decision

Metalink is the next eligible implementation because a first release can remain inside the current
HTTP(S) and filesystem boundaries. That release is limited to failover between mirrors; concurrent
cross-mirror range mixing remains a later optimization that requires its own validator and
piece-hash tests.

The first BitTorrent adapter uses maintained `librqbit` behind a narrow desktop boundary. Every
transfer requires an explicit privacy confirmation and an isolated task directory. It runs only in
Direct connection mode, with DHT, local discovery, incoming listeners, uploading, and
post-completion seeding disabled. Tracker contact, outbound TCP peers, and peer exchange remain
visible in the disclosure. SOCKS routing, incoming connections, DHT, uTP, port mapping, automatic
capture, and background seeding remain deferred behind separate review.

## References

- [RFC 5854: The Metalink Download Description Format](https://www.rfc-editor.org/rfc/rfc5854)
- [RFC 6249: Metalink/HTTP: Mirrors and Hashes](https://www.rfc-editor.org/rfc/rfc6249)
- [BEP 3: The BitTorrent Protocol Specification](https://www.bittorrent.org/beps/bep_0003.html)
- [BEP 52: The BitTorrent Protocol Specification v2](https://www.bittorrent.org/beps/bep_0052.html)
