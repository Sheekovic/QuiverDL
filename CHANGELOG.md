# Changelog

All notable changes will be documented here. QuiverDL follows semantic versioning once public releases begin.

## [0.3.0](https://github.com/Sheekovic/QuiverDL/compare/v0.2.0...v0.3.0) (2026-08-25)


### Features

* add unified media, torrent, clipboard, and routing flows ([#36](https://github.com/Sheekovic/QuiverDL/issues/36)) ([37a9079](https://github.com/Sheekovic/QuiverDL/commit/37a9079ba4546d125bb36e38c6eb6f11099c139d))


### Bug Fixes

* accept Tauri updater signatures ([#35](https://github.com/Sheekovic/QuiverDL/issues/35)) ([d4ba891](https://github.com/Sheekovic/QuiverDL/commit/d4ba891a0a6a4915e1ee14c2881aa5f75b32db6e))
* preserve tag context in release recovery ([#34](https://github.com/Sheekovic/QuiverDL/issues/34)) ([0d418e9](https://github.com/Sheekovic/QuiverDL/commit/0d418e9dee154bfae370eae21673ef5269d143e3))
* unblock automatic release preparation ([#38](https://github.com/Sheekovic/QuiverDL/issues/38)) ([8e2af1d](https://github.com/Sheekovic/QuiverDL/commit/8e2af1deca79f6b73c5f546c2a9403eca7f992fc))

## [0.2.0](https://github.com/Sheekovic/QuiverDL/compare/v0.1.0...v0.2.0) (2026-08-21)


### Features

* add private, searchable download history ([#26](https://github.com/Sheekovic/QuiverDL/issues/26)) ([62e14b1](https://github.com/Sheekovic/QuiverDL/commit/62e14b1d473556a0803187184decd134697aa15c))
* add signed automatic Linux updates ([#27](https://github.com/Sheekovic/QuiverDL/issues/27)) ([6aa499f](https://github.com/Sheekovic/QuiverDL/commit/6aa499ff9cddddbbd926019fdca7680750b798c3))


### Bug Fixes

* accept normalized Snap version scalars ([#32](https://github.com/Sheekovic/QuiverDL/issues/32)) ([371c796](https://github.com/Sheekovic/QuiverDL/commit/371c79610f0e9353c86da6016e732d8ce1f54372))
* handle release PR output and token setup safely ([#30](https://github.com/Sheekovic/QuiverDL/issues/30)) ([2cb350d](https://github.com/Sheekovic/QuiverDL/commit/2cb350d6b15bff8944b56b1a7f2de39f5d1c30df))
* make automatic release PRs workspace-safe ([#29](https://github.com/Sheekovic/QuiverDL/issues/29)) ([99753b6](https://github.com/Sheekovic/QuiverDL/commit/99753b6e46d498822e28e10e4e31752fdf9e2fa1))
* restore automatic release PR generation ([#28](https://github.com/Sheekovic/QuiverDL/issues/28)) ([0776a5e](https://github.com/Sheekovic/QuiverDL/commit/0776a5ec0178f8bb60aa01991e1a559401a15802))

## [0.1.0](https://github.com/Sheekovic/QuiverDL/releases/tag/v0.1.0) (2026-08-21)

- Added persistent queue recovery, settings, bounded retries, and trusted filename discovery.
- Added validator-gated parallel range transfers, adaptive segment sizing, per-host connection caps, speed limits, and merge verification.
- Added system tray behavior, completion notifications, adaptive themes, and an English/Arabic localization foundation.
- Added an authenticated native-messaging host and privacy-first Chromium and Firefox companion extensions.
- Added cross-platform draft release automation, benchmark guidance, a threat model, and a security review checklist.
