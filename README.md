# KAT Mobile

The Android edition of [KAT](https://github.com/KaraZajac/KAT), the Keyfob Analysis Toolkit. A phone
with a **HackRF** on USB OTG receives, records and identifies car key fob, gate, garage door and
sensor signals with KAT's own receive core, without a computer. It is receive-only: no
transmission, replay or key recovery.

KAT Mobile was written by **CooKie** as an Android adaptation of KAT, and is maintained here as
KAT's official Android app.

---

## Security and legal disclaimer

**Use KAT Mobile only on systems and vehicles you own or have explicit, written permission to
test.** Capturing or decoding keyfob and vehicle-access signals without authorization may be
illegal in your jurisdiction (e.g. computer misuse, unauthorized access, or radio regulations). You
are solely responsible for ensuring your use complies with all applicable laws. KAT Mobile is
intended for security research, authorized penetration testing, education, and legitimate testing
on your own equipment. The authors and contributors assume no liability for misuse or damage
arising from use of this software.

---

## Features

- **HackRF over USB OTG** — a HackRF One, Jawbreaker or rad1o through Android's USB host API: no
  root and no driver app. Frequency, sample rate (2–20 MS/s), LNA and VGA gains, the RF amplifier,
  the baseband filter and AM, FM or both (AUTO).
- **KAT's receive core** — KAT 1.29.0's receive pipeline (burst detection, channel filtering, the
  AM and FM slicers) and its protocol decoders, built from KAT's Rust source and called over JNI.
  A signal is listed with its protocol, bit count and whether its checksum holds; frames that
  fail their checksum are dropped. Serials, counters and keys stay inside the core.
- **A few more formats** — Nexus/TFA and Rubicson/Solight weather sensors, and Princeton, CAME and
  Nice FLO fixed codes when the core reads nothing, from a small recognizer in Kotlin.
- **Live view** — spectrum, waterfall, signal level and data rate, the latest signal received, and
  the history grouped by protocol, with repeats of the same signal counted instead of listed.
- **Frequency hopping** — a fixed time on each frequency (Flipper's hopper list by default), which
  does not wait for a signal to end.
- **IQ recording and analysis** — record raw CS8 IQ at one frequency (with a storage quota), and
  analyze CS8 or CU8 recordings later, from the phone or from KAT on a computer.
- **Import and export** — KAT `.fob` files (every saved pulse, readable by desktop KAT), Flipper RAW
  `.sub` files, JSON, whole sessions, and KAT's `config.ini` (its radio, hopping and history
  settings).
- **Private by default** — no internet permission. The session is stored encrypted (AES-GCM, with
  a key in the Android Keystore), Android backups are off, and files leave the app only through the
  system's file picker.

## Installing

Download `KAT-Mobile-<version>.apk` from the [releases page](https://github.com/KaraZajac/KAT-Mobile/releases)
(each has a `.sha256` checksum), allow your browser or file manager to install apps, and open it.
It needs a 64-bit ARM phone with Android 8.0 or later and USB OTG.

Connect the HackRF with an OTG adapter or cable and press **Start RX**; Android asks for USB access
the first time. A HackRF draws up to 500 mA: if the phone cannot power it, use a powered USB hub.

Releases are debug-signed for now. When KAT Mobile moves to a release key, the new build will not
install over the old one: export your session first (Signals, Export session), uninstall, install
the new build and import the session.

## Building

You need JDK 17, the Android SDK with platform 36 and NDK 27.2.12479018, and Rust:

```sh
sdkmanager "platforms;android-36" "ndk;27.2.12479018"
rustup target add aarch64-linux-android x86_64-linux-android
```

Then, on Linux, macOS or Windows:

```sh
./gradlew assembleDebug                         # the QA app (io.karazajac.katmobile.qa), arm64 and x86_64
./gradlew -PkatAbis=arm64-v8a assembleRelease   # app/build/outputs/apk/release/app-release.apk
```

Gradle builds the receive core in `native/` with cargo for each ABI (`-PkatAbis` picks them) and
links it with the NDK's clang. It finds cargo through `$CARGO`, rustup's `~/.cargo/bin` or the
`PATH`, and the NDK in the SDK or `$ANDROID_NDK_HOME`.

On Windows, `Build.ps1` (or `BUILD-APK.cmd`) also runs the tests and checks the APK; `-Install
-Serial <device>` installs it and `-DeviceTests -Serial <device>` runs the instrumentation tests.

### Tests

```sh
./gradlew testDebugUnitTest            # first: it writes the .fob that fob_interop reads
(cd native && cargo test --locked)     # the receive core, on the computer
```

The instrumentation tests reset the saved session, so they run only in the QA app. With a phone or
emulator connected:

```sh
./gradlew assembleDebug assembleDebugAndroidTest
adb install -r app/build/outputs/apk/debug/app-debug.apk
adb install -r app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk
adb shell pm grant io.karazajac.katmobile.qa android.permission.POST_NOTIFICATIONS
adb shell am instrument -w -e class app.katmobile.AppTest io.karazajac.katmobile.qa.test/androidx.test.runner.AndroidJUnitRunner
```

CI runs all of them on every push, the instrumentation tests on an Android 14 emulator.

## How it is built

- `app/` — the Kotlin and Jetpack Compose app: the USB receiver (`HackRfUsb.kt`, receive requests
  only), the radio service, storage, import and export, and the tests.
- `native/src/` — KAT 1.29.0's receive code. `radio/` and 37 other files are KAT's unchanged; in
  the rest, encoders, Flipper key files and key recovery are stubs, so the core cannot build a
  transmission. `lib.rs` and `mobile_rx.rs` are the JNI layer, and `interop_fob.rs` is KAT's `.fob`
  reader, used by the tests.

## Credits

- **CooKie** — the Android app and its adaptation of KAT's receive core.
- **KAT** — [github.com/KaraZajac/KAT](https://github.com/KaraZajac/KAT), whose decoders follow
  ProtoPirate, Flipper-ARF, doctor-firmware, Momentum, RogueMaster, FoxFW and rtl_433 (see KAT's
  credits).

## License

BSD 3-Clause, like KAT: see [LICENSE](LICENSE).
