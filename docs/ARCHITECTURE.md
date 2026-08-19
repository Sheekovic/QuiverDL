# Architecture

QuiverDL separates transfer behavior from presentation:

```text
Desktop UI / future CLI / browser bridge
                  |
             quiver-core
        HTTP, state, files, integrity
```

## Safety invariants

1. Bytes are written to a `.quiver-part` file until validation succeeds.
2. Existing destination files are never overwritten unless the caller explicitly opts in.
3. A partial file is resumed only when the remote validator still matches. Without an ETag or
   Last-Modified value, QuiverDL restarts rather than risking a mixed or corrupted file.
4. A completed transfer is length-checked when the server provides a total size.
5. SHA-256 is always calculated; an expected digest can be supplied for strict verification.

## Planned stages

1. Reliable single-stream HTTP/HTTPS engine
2. Bounded multi-segment transfers with per-host connection policy
3. Persistent queue and crash recovery
4. Tauri desktop application
5. Authenticated native-messaging browser bridge
6. Explicit proxy routing with operating-system credential storage
