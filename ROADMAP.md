# QuiverDL Roadmap

This roadmap communicates direction rather than fixed deadlines. Priorities may change as testing
and contributor feedback reveal better solutions.

## Foundation — in progress

- [x] Native Rust HTTP and HTTPS transfer engine
- [x] Partial-file staging and validator-gated resume decisions
- [x] Validate the returned `Content-Range` start before appending resumed bytes
- [x] Pause, cancel, progress, and SHA-256 primitives
- [x] Tauri and React desktop foundation
- [x] Local HTTP integration testing
- [ ] Persistent queue and restart recovery
- [x] Wire real transfers into the desktop interface
- [ ] Retry policy with backoff and clear failure states
- [ ] Filename discovery from URL and `Content-Disposition`

## Fast transfers

- [ ] Bounded multi-segment downloads
- [ ] Per-host connection policy
- [ ] Dynamic segment sizing and merge verification
- [ ] Global and per-download speed limits
- [ ] Benchmarks for throughput, CPU, memory, and disk behavior

## Desktop experience

- [x] Adaptive light and dark themes
- [x] Accessible keyboard-first queue controls
- [x] Destination picker and collision handling
- [ ] Notifications and system-tray behavior
- [ ] Settings persistence
- [ ] Localization foundation

## Browser integration

- [ ] Authenticated native-messaging bridge
- [ ] Chromium and Firefox extension foundation
- [ ] Explicit, privacy-preserving download interception rules
- [ ] Browser-to-desktop integration tests

## Release readiness

- [ ] Signed Windows installer and portable build
- [ ] Linux packages
- [ ] macOS application and signing plan
- [ ] Reproducible GitHub release automation
- [ ] Threat model and security review

## Good first contributions

Beginner-friendly work should be small, testable, and avoid changing core safety behavior without
review. Good candidates include documentation examples, deterministic parsing tests, accessible UI
labels, theme tokens, and local test-server improvements. Look for the
[`good first issue`](https://github.com/Sheekovic/QuiverDL/labels/good%20first%20issue) label.
