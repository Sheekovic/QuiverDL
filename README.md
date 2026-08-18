# QuiverDL

**Your downloads, right on target.**

[![CI](https://github.com/Sheekovic/QuiverDL/actions/workflows/ci.yml/badge.svg)](https://github.com/Sheekovic/QuiverDL/actions/workflows/ci.yml)
[![License: GPL v3 or later](https://img.shields.io/badge/License-GPL_v3%2B-blue.svg)](LICENSE)
[![Contributions welcome](https://img.shields.io/badge/contributions-welcome-brightgreen.svg)](CONTRIBUTING.md)

QuiverDL is a performance-first, privacy-respecting open-source download manager. It combines a
native Rust engine with a lightweight Tauri and React desktop application.

> [!IMPORTANT]
> QuiverDL is in early development. The engine and desktop foundation are real and tested, but
> there are no official end-user releases yet.

## Why QuiverDL?

- Reliability before feature count
- Native performance with modest memory usage
- Safe pause and resume across application restarts
- No accounts, advertising, analytics, or telemetry
- Clear behavior: never silently overwrite a user's file
- One reusable engine for the desktop app, browser bridge, and future CLI

## Current status

The first engine slice supports HTTP and HTTPS transfers, staged partial files, resume decisions
gated by matching remote validators, pause and cancel controls, progress events, and SHA-256
verification. Response-range validation is still being hardened before resume is considered
complete. The desktop app can inspect a URL through the Rust engine and report its size, resume
support, and remote validators.

The next major milestones are wiring transfers into the desktop queue, persistent crash recovery,
and bounded multi-segment downloading. See the [roadmap](ROADMAP.md) for approachable tasks and
longer-term direction.

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
apps/desktop/         Tauri 2 and React desktop application
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
