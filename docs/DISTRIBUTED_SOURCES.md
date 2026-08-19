# Distributed source evaluation

Metalink and BitTorrent can both describe more than one source for content, but they do not share a
trust boundary. QuiverDL will treat them as separate backends with separate user consent, state,
network policy, and security review.

| Area | Metalink | BitTorrent |
| --- | --- | --- |
| Primary input | RFC 5854 XML; RFC 6249 is evaluated but deferred | `.torrent` metainfo or a magnet URI |
| Network model | Known HTTP(S) mirrors | Trackers and many untrusted peers over additional protocols |
| Integrity | Publisher-provided size and SHA-256 | V2 SHA-256 Merkle data; v1-only is inspector-only |
| Privacy change | Mirror operators learn requests | Trackers and peers can learn the user's IP address and swarm identifier |
| QuiverDL decision | Approved for a bounded HTTP-only implementation | Deferred until the isolation and consent gates below are met |

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
  Metalink requires a complete plan before mirror requests. Magnets require explicit metadata-only
  discovery consent followed by a second full-tree confirmation before content transfer.
- A protocol backend may not weaken destination reservations, no-replace promotion, cancellation,
  resource limits, or final integrity verification.

## Sequencing decision

Metalink is the next eligible implementation because a first release can remain inside the current
HTTP(S) and filesystem boundaries. That release is limited to failover between mirrors; concurrent
cross-mirror range mixing remains a later optimization that requires its own validator and
piece-hash tests.

BitTorrent is not approved for implementation in the desktop process yet. A future proposal must
first select and review a maintained backend, define process or crate isolation, expose the privacy
and upload behavior before joining a swarm, and prove bounded parsing and filesystem containment.
This deferral is a security decision, not a claim that BitTorrent itself is unsafe.

## References

- [RFC 5854: The Metalink Download Description Format](https://www.rfc-editor.org/rfc/rfc5854)
- [RFC 6249: Metalink/HTTP: Mirrors and Hashes](https://www.rfc-editor.org/rfc/rfc6249)
- [BEP 3: The BitTorrent Protocol Specification](https://www.bittorrent.org/beps/bep_0003.html)
- [BEP 52: The BitTorrent Protocol Specification v2](https://www.bittorrent.org/beps/bep_0052.html)
