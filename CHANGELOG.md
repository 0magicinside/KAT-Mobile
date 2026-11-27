# Changelog

All notable changes to KAT Mobile are documented here.

## [0.8.0] - 2026-11-26

The first release from this repository: CooKie's KAT Mobile RX 0.8.0-en, with KAT 1.29.0's
receive core, now built and tested in CI.

### Changed

- The application ID is `io.karazajac.katmobile` (CooKie's builds were `app.katmobile`), so it
  installs beside those builds instead of conflicting with them. The QA build is
  `io.karazajac.katmobile.qa`.
- The APK is built on Linux, macOS or Windows: Gradle finds cargo on its own and links the receive
  core with the Android SDK's NDK 27.2.12479018, instead of fixed Windows paths.
- The emulator build of the receive core is x86_64 (it was 32-bit x86). The release APK is
  arm64-v8a, as before.
- Releases are debug-signed with one fixed key, so later releases can update this one.

### Fixed

- The Stop button in the reception notification was not translated.
