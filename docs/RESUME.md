# Safe pause and resume

QuiverDL keeps interrupted downloads recoverable, but it resumes them only when the remote file can
be shown to be the same file. If that cannot be established, QuiverDL starts again from byte zero
instead of risking a completed file made from two different versions.

## Recovery files

While downloading `archive.zip`, QuiverDL may place these files beside the selected destination:

- `archive.zip.quiver-part` contains bytes that have downloaded but have not yet been promoted to
  the final destination.
- `archive.zip.quiver.json` records the source URL, expected size, remote validator, and any
  parallel segment layout needed to decide whether those bytes are safe to reuse.
- `archive.zip.quiver-part.segment-N` files may hold validated pieces of a parallel download until
  QuiverDL merges them.

These files are implementation details. Do not rename, edit, or share them while a download is
active. URLs and filenames can contain private information.

## When QuiverDL resumes

Before transferring more data, QuiverDL probes the server and compares the response with the saved
state. Existing bytes are reused only when all relevant checks pass:

- the saved URL still matches the requested URL;
- the server still supports byte-range requests;
- the saved and current total sizes match;
- the partial file is shorter than the expected completed file; and
- a strong `ETag`, or otherwise a `Last-Modified` value, matches the saved validator.

QuiverDL then sends both a `Range` request for the missing bytes and an `If-Range` condition using
that validator. The server must return `206 Partial Content` with a valid `Content-Range` beginning
at exactly the requested byte. The returned end, total, and received byte count are checked before
the partial file can become the completed destination.

## When QuiverDL restarts

QuiverDL discards the old partial bytes and starts from byte zero when the URL, size, range support,
or validator no longer matches. It also restarts when no trustworthy validator is available. This
can use more bandwidth, but it prevents old and new remote content from being combined.

If a server ignores a validated resume request and returns a normal full response, QuiverDL rejects
that response rather than appending it. The existing recovery files remain available for a later
retry.

## Cleanup and failures

After verification succeeds, QuiverDL atomically promotes the partial file to the selected
destination and removes its state and segment files. An ordinary cancellation or transfer error
keeps valid recovery data when possible. A checksum mismatch removes the partial and recovery state
because those bytes failed the requested integrity check.

QuiverDL never silently replaces an existing completed destination unless the user explicitly
allows overwrite behavior.
