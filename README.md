# KAT Mobile RX 0.8.0-en

English Android adaptation of the KAT 1.29.0 receive core.
See BUILD.txt for Windows build instructions and THANK-YOU.txt for a personal note.

app/: Kotlin/Compose UI, Android USB receiver, recording, exports, storage and tests.
native/: prepared Rust/JNI receive core and integration tests.
gradle/: Gradle wrapper.

167 registered native receive decoders, an English interface, grouped capture
history, pulse-preserving export, fixed-dwell frequency hopping and a thank-you
to Kara with an About / What's new screen.
Android is receive-only; desktop TX/replay, recovery tools and additional radio
backends are not included. Registration does not establish compatibility with
every physical device. Android IQ file analysis currently supports CS8/CU8.
The prepared native core builds without a desktop-source checkout.

Application ID: app.katmobile. Version code: 9.
The included release configuration uses the local Android debug signing key;
a different machine's key cannot update the original signed installation.
No signing keys or personal data are included.

Original KAT BSD-3-Clause notices are retained in LICENSE, native/LICENSE
and app/src/main/assets/KAT-LICENSE.txt.
