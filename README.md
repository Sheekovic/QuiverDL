# QuiverDL

**Your downloads, right on target.**

[![CI](https://github.com/Sheekovic/QuiverDL/actions/workflows/ci.yml/badge.svg)](https://github.com/Sheekovic/QuiverDL/actions/workflows/ci.yml)
[![License: GPL v3 or later](https://img.shields.io/badge/License-GPL_v3%2B-blue.svg)](LICENSE)
[![Contributions welcome](https://img.shields.io/badge/contributions-welcome-brightgreen.svg)](CONTRIBUTING.md)
[![Get it from Microsoft](https://img.shields.io/badge/Get_it_from-Microsoft_Store-0078D4?logo=microsoft&logoColor=white)](https://apps.microsoft.com/detail/9MVB2DD54NF4)

[Website](https://sheekovic.github.io/QuiverDL/) ·
[Microsoft Store](https://apps.microsoft.com/detail/9MVB2DD54NF4) ·
[Privacy Policy](https://sheekovic.github.io/QuiverDL/privacy/) ·
[Roadmap](ROADMAP.md) ·
[Contributing](CONTRIBUTING.md)

QuiverDL is a performance-first, privacy-respecting open-source download manager. It combines a
native Rust engine with a lightweight Tauri and React desktop application.

> [!TIP]
> QuiverDL is now available from the
> [Microsoft Store](https://apps.microsoft.com/detail/9MVB2DD54NF4). Install it there to receive a
> Microsoft-signed package and Store-managed updates.

## Install on Windows

[Download QuiverDL from the Microsoft Store](https://apps.microsoft.com/detail/9MVB2DD54NF4)

The Microsoft Store is currently the recommended installation method. Direct installer and
portable downloads from GitHub remain pending independent code-signing credentials and final
clean-machine validation.

## Why QuiverDL?

- Reliability before feature count
- Native performance with modest memory usage
- Safe pause and resume across application restarts
- No accounts, advertising, analytics, or telemetry
- Clear behavior: never silently overwrite a user's file
- One reusable engine for the desktop app, browser bridge, and future CLI

## Current status

The native engine supports persistent validator-safe resume, bounded retries, trusted filename
discovery, adaptive parallel range transfers, exact merge and SHA-256 verification, per-host
connection policies, and speed limits. The desktop app persists its queue and settings atomically,
recovers interrupted work, provides adaptive themes, English/Arabic direction support, tray and
notification behavior, supports durable scheduled starts and parallel or sequential FIFO queues,
supports direct, system, and credential-safe custom proxy routing, and never silently overwrites a
destination.

Optional Chromium and Firefox companions communicate through an authenticated native host. Manual
capture is the default; automatic interception is opt-in, local, and constrained by explicit rules.
See the [roadmap](ROADMAP.md), [threat model](docs/THREAT_MODEL.md), and
[release process](docs/RELEASE.md) for the remaining direct-distribution signing step. The
[safe pause and resume guide](docs/RESUME.md) explains when interrupted bytes are reused, restarted,
or preserved for another retry. The [proxy guide](docs/PROXY.md) explains routing, authentication,
and privacy boundaries.
The [queue guide](docs/QUEUES.md) documents scheduling, ordering, restart recovery, and controls.
The [secure updater design](docs/UPDATER.md) defines signature, manifest, downgrade, recovery, and
key-rotation gates before direct-download updates can be enabled.
The [distributed source evaluation](docs/DISTRIBUTED_SOURCES.md) records why bounded HTTP-only
Metalink support may proceed later while BitTorrent remains behind separate isolation and consent
gates.

## Get involved

QuiverDL is being designed in public, and contributions of every size are welcome:

- Browse [`good first issue`](https://github.com/Sheekovic/QuiverDL/labels/good%20first%20issue)
  tasks for a guided starting point.
- Read the [contribution guide](CONTRIBUTING.md) before opening a pull request.
- Propose ideas and ask questions in
  [GitHub Discussions](https://github.com/Sheekovic/QuiverDL/discussions).
- Report bugs with the structured
  [bug form](https://github.com/Sheekovic/QuiverDL/issues/new?template=bug_report.yml).

You do not need to be a Rust expert. Documentation, accessibility, tests, design feedback, and
careful bug reports are valuable contributions.

## Repository layout

```text
crates/quiver-core/   Native download engine and domain model
crates/quiver-native-host/ Authenticated browser native-messaging bridge
apps/desktop/         Tauri 2 and React desktop application
extensions/           Chromium, Firefox, and native-host installation assets
docs/                 Architecture and product decisions
```

## Development quick start

Prerequisites:

- Stable Rust with `rustfmt` and `clippy`
- Node.js 20.19 or newer
- Tauri's platform prerequisites

```powershell
git clone https://github.com/Sheekovic/QuiverDL.git
cd QuiverDL
npm ci --prefix apps/desktop
cargo test --workspace
npm run build --prefix apps/desktop
```

Run all required checks before submitting a change:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm run build --prefix apps/desktop
```

## Community

Participation is governed by our [Code of Conduct](CODE_OF_CONDUCT.md). For usage questions, read
[Support](SUPPORT.md). Please report vulnerabilities privately according to the
[Security Policy](SECURITY.md).

## License

QuiverDL is free software licensed under the [GNU General Public License v3.0 or later](LICENSE).
