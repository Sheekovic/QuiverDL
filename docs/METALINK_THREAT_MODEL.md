# Metalink threat model and adoption decision

## Status

**Approved for a future, bounded HTTP-only implementation.** This evaluation does not enable
Metalink handling in the current application. The first implementation must satisfy every gate in
this document before `.meta4` files or Metalink response metadata are accepted.

[RFC 5854](https://www.rfc-editor.org/rfc/rfc5854) defines XML metadata containing filenames,
sizes, hashes, piece hashes, and alternate URLs. [RFC 6249](https://www.rfc-editor.org/rfc/rfc6249)
defines related mirror and digest metadata in HTTP response headers. Both specifications require
downloaded bytes to be checked against the described size and cryptographic hash.

## Security goals

- A Metalink cannot cause requests to its listed mirrors, writes, or directory creation before the
  user sees and confirms the parsed plan. An explicitly requested metadata URL may be fetched only
  under the bounded scheme, redirect, address-classification, DNS-binding, proxy, and cancellation
  rules below in order to build that plan.
- Every completed file has a declared size and a valid SHA-256 digest from the confirmed Metalink.
- No path can escape the destination root or replace an existing file.
- A mirror cannot make QuiverDL forward origin credentials, cookies, authorization headers, proxy
  credentials, or private headers to another host.
- A malformed document cannot consume unbounded CPU, memory, disk space, connections, or queue
  entries.

## Inputs and trust boundaries

The Metalink document, its XML structure, extensions, text, filenames, sizes, hashes, priorities,
locations, mirror URLs, and referenced metadata are untrusted. HTTPS authenticates the server that
delivered a document; it does not prove that the publisher or listed files are trustworthy. A
digest carried by the same malicious document cannot establish publisher authenticity.

The user-selected destination root and an expected digest obtained independently from a trusted
publisher are stronger trust inputs. XML signatures are outside the first implementation. If a
document contains a signature QuiverDL cannot validate, the UI must label it **not verified** and
must never imply authenticity from its presence.

## Required parser boundary

The parser must be a UI-independent Rust component with deterministic tests and fuzz coverage.

- Read at most 4 MiB of Metalink XML and reject trailing data beyond that bound.
- Parse as a stream; reject DTDs, entity declarations, external entities, XInclude, and network or
  filesystem resolution from XML.
- Accept only the RFC 5854 namespace and known core fields. Ignore bounded extension elements
  without executing or dereferencing them.
- Limit a document to 256 files and 32 HTTP(S) mirror URLs per file. Bound XML depth, text length,
  decoded hash length, and total piece-hash entries before allocation.
- Require exactly one non-negative size representable as `u64` and at least one correctly sized
  SHA-256 whole-file hash for every accepted file. MD5 and SHA-1 never satisfy the integrity gate.
- Reject duplicate or contradictory size, filename, hash, and piece-set declarations rather than
  selecting one silently.
- Reject `metaurl`, `origin`, dynamic refresh, FTP, peer-to-peer, and unknown URL schemes in the
  first implementation. Nested metadata is never followed automatically.

## Filesystem boundary

RFC 5854 allows relative directory components in a file name and explicitly forbids traversal.
QuiverDL applies stricter platform-aware containment:

- Treat every name component as untrusted. Reject empty components, `.`, `..`, absolute paths,
  drive or UNC prefixes, NUL/control characters, alternate separators, and Windows reserved names.
- Join only sanitized relative components beneath a destination root chosen after preview.
- Canonicalize the existing root, reject symlink traversal, and verify containment again before
  every create and final promotion.
- Conservatively reject case-folded or Unicode-normalized path collisions on every platform before
  starting any file. Do not infer destination filesystem semantics from the operating-system name;
  Linux destinations can also be case-insensitive or normalization-aware.
- Reserve the destination, partial, state, temporary, and segment paths for the full batch. Use the
  existing no-replace promotion and preserve recoverable partials after ordinary interruption.
- Do not implement Metalink-declared symbolic links, hard links, permissions, or executable bits.

## Network and integrity boundary

- Fetching a remote Metalink document is an explicit user action and may retrieve only the bounded
  metadata needed for preview. Classify its initial address and every redirect before connecting,
  apply the same DNS-binding rules as mirror requests, and require separate approval before a public
  metadata URL can enter a local or special-use address class.
- The confirmation screen lists all destination-relative paths, total bytes, mirror hosts, and
  whether publisher authenticity is unverified. Confirmation precedes inspection or download
  requests to listed mirrors.
- Only HTTP and HTTPS mirrors are eligible. HTTPS is preferred; use of an HTTP mirror requires the
  same explicit insecure-transport warning used for a direct HTTP download.
- Apply the existing retry, per-host connection, global connection, proxy, speed, cancellation, and
  redirect-count policies independently to every mirror. Metalink support must additionally resolve
  and classify the initial target and every redirect target before connecting; the current
  scheme-only redirect validation is not sufficient for this feature.
- A mirror confirmed as public cannot redirect to loopback, link-local, private, or otherwise local
  addresses without a second explicit confirmation. Mixed public/private DNS answers fail closed.
  Resolution and the actual socket destination must remain bound to the approved address class so a
  DNS change cannot bypass the decision.
- Never copy request credentials or private headers between mirror origins. Stored proxy
  credentials remain inside the existing backend boundary and are not exposed to Metalink data.
- Prevent mirror and metadata loops. Bound attempted mirrors and do not retry a failed mirror
  indefinitely.
- Phase one downloads a file from one mirror at a time and falls back only after a validated
  failure. It does not combine ranges from different mirrors.
- Check the declared size while streaming and verify SHA-256 before atomic promotion. On mismatch,
  retain only explicitly recoverable partial state and record the failing mirror locally without
  exposing its URL in public diagnostics.
- Piece hashes may later support repair or cross-mirror ranges, but only SHA-256-or-stronger piece
  sets with exact count, ordering, and length validation are eligible. A final whole-file digest
  remains mandatory.

## Availability and abuse cases

A malicious publisher can list slow servers, unrelated victims, looped mirrors, enormous files, or
many failing URLs. Preview and confirmation prevent silent requests, while document, file, mirror,
redirect, retry, connection, and disk limits contain amplification. The application must re-check
available space before allocation and keep all writes streaming.

Private, loopback, link-local, and local-network metadata or mirror targets require an additional
per-download confirmation because a remotely supplied URL can otherwise turn the desktop into a
request agent for internal services. DNS results must be revalidated on every connection to limit
rebinding.

## Implementation acceptance gates

- Bounded parser tests cover malformed XML, entity expansion attempts, numeric overflow, duplicate
  fields, unsupported hashes and schemes, deep nesting, and limit edges.
- Cross-platform path tests cover traversal, separators, drive/UNC inputs, reserved names,
  Unicode/case collisions, symlink races, and destination no-replace behavior.
- Local HTTP tests cover mirror fallback, redirect loops, size mismatch, digest mismatch,
  cancellation, proxy routing, resume policy, public-to-private redirects, mixed-address DNS
  answers, DNS rebinding, and failure without public-internet access.
- The persistent schema records the confirmed metadata identity, selected paths, expected sizes and
  hashes, and per-file state atomically.
- UI review covers keyboard access, RTL layout, adaptive themes, warnings, and complete pre-network
  consent.
- Security review confirms that no existing HTTP download or browser-interception default changes.

Only after these gates pass may the roadmap gain a separate **Metalink implementation** item.
