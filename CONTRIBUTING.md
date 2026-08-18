# Contributing to QuiverDL

Thank you for helping build QuiverDL. We welcome code, tests, documentation, design feedback,
accessibility improvements, and reproducible bug reports.

By participating, you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Find the right starting point

- **First contribution:** choose an issue labeled
  [`good first issue`](https://github.com/Sheekovic/QuiverDL/labels/good%20first%20issue).
- **Ready for a larger task:** look for
  [`help wanted`](https://github.com/Sheekovic/QuiverDL/labels/help%20wanted).
- **Found a bug:** search existing issues, then use the bug report form.
- **Have a feature idea:** start a Discussion before investing in a large implementation.
- **Security concern:** do not open a public issue; follow [SECURITY.md](SECURITY.md).

Comment on an issue before starting substantial work. This prevents duplicated effort and gives
maintainers a chance to clarify requirements. Small documentation and test fixes can go directly
to a pull request.

## Development setup

QuiverDL currently uses:

- Rust for `quiver-core` and the Tauri backend
- React and TypeScript for the desktop interface
- npm for frontend dependencies

Install stable Rust, Node.js 20.19 or newer, and the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your operating system.

```powershell
git clone https://github.com/Sheekovic/QuiverDL.git
cd QuiverDL
npm ci --prefix apps/desktop
cargo test --workspace
npm run build --prefix apps/desktop
```

To run the desktop app during development:

```powershell
npm run tauri --prefix apps/desktop -- dev
```

## Make a focused change

1. Fork the repository and create a short-lived branch from `main`.
2. Keep each pull request focused on one problem.
3. Add or update tests for observable behavior.
4. Update documentation when behavior or contributor workflows change.
5. Run the required checks locally.
6. Open a pull request and complete the template.

Recommended branch names include `fix/resume-validation`, `feat/download-queue`, and
`docs/http-behavior`.

## Required checks

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm run build --prefix apps/desktop
```

CI repeats these checks. If a platform-specific check cannot run locally, explain that clearly in
the pull request.

## Engineering expectations

### Download safety

The invariants in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) are release gates:

- Partial data must never appear as a completed destination file.
- Existing destination files must not be overwritten without explicit consent.
- Resume must not combine bytes from different remote versions.
- Network and filesystem errors must preserve recoverable state when safe.
- Tests must not depend on public internet services.

### Rust

- Keep `quiver-core` independent of Tauri and presentation code.
- Prefer explicit error types and bounded resource use.
- Avoid `unsafe` code unless an issue documents the need and review plan.
- Add deterministic unit or local integration tests for network behavior.

### React and TypeScript

- Preserve keyboard navigation, visible focus, and semantic labels.
- Support both light and dark system themes.
- Do not add analytics, tracking, remote fonts, or unnecessary network requests.
- Avoid large dependencies when a small local implementation is clear and maintainable.

## Commits and pull requests

Use short, descriptive commit messages such as:

```text
test resume after validated partial transfer
fix destination collision handling
docs explain local HTTP fixtures
```

Draft pull requests are welcome. Maintainers will review correctness, tests, scope, security,
accessibility, and consistency with the roadmap. Reviews should be respectful and focused on the
change, never the contributor.

## Licensing contributions

By submitting a contribution, you agree that it is your original work or that you have the right
to submit it, and that it may be distributed under QuiverDL's
[GPL-3.0-or-later license](LICENSE).

