# Scheduled and sequential queues

QuiverDL can start a download immediately or at a future local date and time. The desktop converts
that choice to a bounded UTC timestamp before it crosses the Tauri command boundary. Queue entries,
including a unique FIFO sequence and their enqueue and scheduled times, are written atomically
before network work starts.

## Queue modes

- **Parallel** starts each due download independently. Existing per-host and global connection
  limits still apply.
- **Sequential** admits one due download at a time through a fair Rust semaphore. Downloads enter
  that gate in arrival order, so an active transfer releases the next waiting transfer when it
  finishes or fails. A scheduled download does not occupy the gate before its start time.

The selected mode is captured when a running desktop session submits the download. Changing the
setting affects downloads submitted afterward. After an application restart, pending entries use
the restored queue setting.

## Restart and controls

Queued and scheduled entries remain pending across a normal quit or an interrupted shutdown. On
the next launch, the desktop validates the saved queue and registers every pending entry with the
Rust coordinator in persisted enqueue order before it accepts new work. Due tickets remain ordered
even when path preparation finishes out of order. Future tickets do not block ready work; they join
the ready set at their scheduled time, with their persisted ticket breaking ties. A scheduled time
that passed while QuiverDL was closed becomes due immediately.

Queued and scheduled entries can be cancelled before any request is sent. Cancellation requested
while a ticket is being registered is retained and applied as soon as registration completes.
Destination and recovery sidecar paths are reserved while an entry waits, preventing two active
queue items from writing the same files. Network-active downloads keep the existing pause, resume,
cancel, retry, and validator-safe recovery behavior described in [RESUME.md](RESUME.md).

Queue state is local application data. It can include private URLs, destination paths, and timing
information, so it must not be attached to public bug reports without redaction.
