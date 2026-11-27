# Changelog

All notable changes to KAT Mobile are documented here.

## [1.30.0] - 2026-11-26

KAT Mobile transmits, as KAT does, and its version now follows KAT's: its core is KAT 1.30.0's.

### Added

- **Transmit** through the HackRF, from a signal's details, as desktop KAT's signal menu offers it:
  Replay (the recorded pulses); TX Lock, Unlock, Trunk and Panic for a car remote; Send next code
  and the remote's own buttons for a gate, garage, barrier or alarm remote. The frames come from
  KAT's encoders with the copies KAT sends, and a rolling code's capture keeps the frame sent, so
  the next press sends the code after (desktop KAT's `app/tx.rs`). The HackRF sends KAT's waveform
  (OOK or 2-FSK at the protocol's deviation, 2 MS/s, 500 kHz above the LO) the way desktop KAT's
  `transmit_signal_hackrf` does, and receiving pauses for it. TX power (desktop KAT's steps, 47 dB
  + amp by default) is in Settings; `tx_vga_gain` and `tx_amp` import from KAT's `config.ini`. A
  transmission is refused during an IQ recording and above 10 s on air.
- Captures show their serial, button and counter, as KAT lists them, and `.fob` files carry the
  decode both ways, so a remote's codes continue from desktop KAT to the phone and back.

### Changed

- The core is KAT 1.30.0's. 219 of its files are KAT's unchanged, with `radio/tx.rs` and "Add
  manually" (`add_manually.rs`, which the tests use) among them; before, it was KAT 1.29.0's with
  the encoders and Flipper key files cut out. The other four leave out key recovery, the user's key
  files and the vulnerability database.
- The core loads KAT's embedded keys at start, as desktop KAT does: the Kia V3/V4, V5 and V6, VAG,
  PSA, Ford V1, BMW V3, Audi V3 and AUT64 decoders decrypt with them. Before, the phone had none.
- The version is KAT's, 1.30.0 (version code 13000, from major × 10000 + minor × 100 + patch). The
  About screen and What's new show the core's KAT version from `native/Cargo.toml`.
- `tools/sync-kat-core.sh` merges a KAT release into the core, keeping KAT Mobile's edits.

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
