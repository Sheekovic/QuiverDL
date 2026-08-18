# Copilot review guidance

Act as an independent second reviewer for QuiverDL. Follow the repository context and review rules
in the root `AGENTS.md` file.

- Focus on defects introduced by the pull request: data loss, unsafe resume behavior, corrupted
  output, privacy leaks, unbounded resource use, cross-platform regressions, accessibility failures,
  and missing behavioral tests.
- Treat remote HTTP metadata, URLs, filenames, state files, and downloaded content as untrusted.
- Check that Rust engine changes remain UI-independent and that Tauri commands validate the
  JavaScript/Rust boundary.
- For visible UI changes, consider keyboard behavior and both adaptive color themes.
- Do not repeat formatting, lint, compilation, or dependency findings already enforced by CI.
- Make each finding actionable: identify the affected code, describe a realistic failure scenario,
  and suggest the smallest safe direction for correction.
- Avoid speculative redesigns and unrelated pre-existing issues.
