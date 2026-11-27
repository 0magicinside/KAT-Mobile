import java.io.File

plugins { id("com.android.application"); id("org.jetbrains.kotlin.android"); id("org.jetbrains.kotlin.plugin.compose") }
android {
 namespace="app.katmobile";compileSdk=36
 defaultConfig { applicationId="app.katmobile";minSdk=26;targetSdk=35;versionCode=9;versionName="0.8.0-en";testInstrumentationRunner="androidx.test.runner.AndroidJUnitRunner" }
 buildTypes {
  debug { if(providers.gradleProperty("inspectPhone").orNull!="true")applicationIdSuffix=".qa" }
  release { isMinifyEnabled=false;signingConfig=signingConfigs.getByName("debug");ndk { abiFilters += "arm64-v8a" } }
 }
 compileOptions { sourceCompatibility=JavaVersion.VERSION_17;targetCompatibility=JavaVersion.VERSION_17 }
 kotlinOptions { jvmTarget="17" }
 buildFeatures { compose=true;buildConfig=true }
 testOptions { unitTests.isReturnDefaultValues=true }
 sourceSets["main"].jniLibs.srcDir(layout.buildDirectory.dir("generated/katJni"))
 packaging { jniLibs.useLegacyPackaging=false }
}

val nativeRoot=rootProject.file("native")
val cargoPath=System.getenv("CARGO")?:File(System.getProperty("user.home"),".cargo/bin/cargo.exe").absolutePath
val rustArm=tasks.register<Exec>("buildKatArm64") {
 workingDir(nativeRoot);commandLine(cargoPath,"build","--release","--target","aarch64-linux-android","--lib")
 inputs.dir(nativeRoot.resolve("src"));inputs.file(nativeRoot.resolve("Cargo.toml"));inputs.file(nativeRoot.resolve("Cargo.lock"));inputs.file(nativeRoot.resolve(".cargo/config.toml"))
 outputs.file(nativeRoot.resolve("target/aarch64-linux-android/release/libkat_rx.so"))
}
val rustX86=tasks.register<Exec>("buildKatX86") {
 dependsOn(rustArm);workingDir(nativeRoot);commandLine(cargoPath,"build","--release","--target","i686-linux-android","--lib")
 inputs.dir(nativeRoot.resolve("src"));inputs.file(nativeRoot.resolve("Cargo.toml"));inputs.file(nativeRoot.resolve("Cargo.lock"));inputs.file(nativeRoot.resolve(".cargo/config.toml"))
 outputs.file(nativeRoot.resolve("target/i686-linux-android/release/libkat_rx.so"))
}
val copyKat=tasks.register<Copy>("copyKatLibraries") {
 dependsOn(rustArm,rustX86)
 from(nativeRoot.resolve("target/aarch64-linux-android/release/libkat_rx.so")){into("arm64-v8a")}
 from(nativeRoot.resolve("target/i686-linux-android/release/libkat_rx.so")){into("x86")}
 into(layout.buildDirectory.dir("generated/katJni"))
}
tasks.named("preBuild"){dependsOn(copyKat)}
dependencies {
 implementation(platform("androidx.compose:compose-bom:2024.12.01"))
 implementation("androidx.activity:activity-compose:1.9.3")
 implementation("androidx.compose.material3:material3")
 implementation("androidx.compose.material:material-icons-extended")
 implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.7")
 implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.7")
 implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")
 implementation("com.google.code.gson:gson:2.11.0")
 testImplementation("junit:junit:4.13.2")
 androidTestImplementation(platform("androidx.compose:compose-bom:2024.12.01"))
 androidTestImplementation("androidx.test:runner:1.7.0")
 androidTestImplementation("androidx.test.ext:junit:1.3.0")
 androidTestImplementation("androidx.test.espresso:espresso-core:3.7.0")
 androidTestImplementation("androidx.compose.ui:ui-test-junit4")
 debugImplementation("androidx.compose.ui:ui-test-manifest")
}
