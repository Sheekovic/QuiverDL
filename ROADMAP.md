# QuiverDL Roadmap

This roadmap communicates direction rather than fixed deadlines. Priorities may change as testing and contributor feedback reveal better solutions.

## Foundation

- [x] Native Rust HTTP and HTTPS transfer engine
- [x] Partial-file staging and validator-gated resume decisions
- [x] Validate returned `Content-Range` values before appending resumed bytes
- [x] Pause, cancel, progress, and SHA-256 primitives
- [x] Tauri and React desktop application
- [x] Local HTTP integration testing
- [x] Persistent queue and restart recovery
- [x] Retry policy with bounded exponential backoff and clear failure states
- [x] Filename discovery from URL and `Content-Disposition`

## Fast transfers

- [x] Bounded multi-segment downloads
- [x] Shared per-host connection policy
- [x] Adaptive segment sizing and merge verification
- [x] Global and per-download speed limits
- [x] Reproducible throughput, CPU, memory, and disk benchmark procedure

## Desktop experience

- [x] Adaptive light and dark themes
- [x] Accessible keyboard-first queue controls
- [x] Destination picker and collision handling
- [x] Completion notifications and system tray
- [x] Atomic settings persistence
- [x] English/Arabic localization foundation with RTL direction support

## Browser integration

- [x] Authenticated native-messaging bridge
- [x] Chromium and Firefox extension foundations
- [x] Explicit, privacy-preserving interception rules that are disabled by default
- [x] Native framing, authentication, inbox-contract, and extension syntax tests

## Release readiness

- [ ] Publish a signed Windows installer and portable archive (automation is complete; repository-owner certificate credentials are required)
- [x] Linux DEB, RPM, and AppImage automation
- [x] macOS Intel/Apple Silicon app, signing, and notarization plan
- [x] Draft GitHub release automation with checksummed artifacts
- [x] Threat model and security review checklist

## Next cycle

- [x] Proxy configuration and credential-safe proxy authentication
- [x] Scheduled and sequential queues
- [x] Metalink and BitTorrent evaluation behind separate threat models
- [x] Store packaging for browser extensions and operating-system app stores
- [x] Automated updater design with rollback and signature verification
- [x] Signed Linux AppImage update checks and reviewed automatic release PRs

## Good first contributions

Beginner-friendly work should be small, testable, and avoid changing core safety behavior without review. Good candidates include documentation examples, deterministic parsing tests, accessible UI labels, additional translations, theme tokens, and local test-server improvements. Look for the [`good first issue`](https://github.com/Sheekovic/QuiverDL/labels/good%20first%20issue) label.
