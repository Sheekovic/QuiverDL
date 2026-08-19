# Performance benchmarks

Run `cargo bench -p quiver-core --bench transfer_baseline` on an otherwise idle machine. The baseline reports SHA-256 CPU throughput, durable sequential writes, sequential reads, and the fixed memory footprint used by the benchmark. Record the commit, operating system, CPU, memory, storage device, filesystem, and power mode beside results.

For end-to-end network measurements, use a local HTTP server with a file larger than 1 GiB, then compare one and four segments with identical speed-limit settings. Measure wall time, peak working set, CPU time, and bytes written. Remote internet tests are useful observations but are not regression gates because server load and routing are uncontrolled.

Performance changes must preserve all response-range, validator, destination-collision, checksum, and cancellation tests. A throughput win never justifies weakening those invariants.
