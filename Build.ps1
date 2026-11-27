param([switch]$Install,[string]$Serial="",[switch]$DeviceTests)
$ErrorActionPreference="Stop"
Set-StrictMode -Version Latest
$projectRoot=$PSScriptRoot
Set-Location -LiteralPath $projectRoot
if(!$env:JAVA_HOME -or !(Test-Path -LiteralPath "$env:JAVA_HOME/bin/java.exe")){
    $taskJdk=Get-ChildItem -LiteralPath "$env:ProgramFiles/Eclipse Adoptium" -Directory -ErrorAction SilentlyContinue | Where-Object Name -like "jdk-17*" | Sort-Object Name -Descending | Select-Object -First 1
    if(!$taskJdk){throw "Install JDK 17 and set JAVA_HOME, then build again."}
    $env:JAVA_HOME=$taskJdk.FullName
}
$taskSdk=$env:ANDROID_HOME
if(!$taskSdk){$taskSdk=Join-Path $env:LOCALAPPDATA "Android/Sdk"}
if(!(Test-Path -LiteralPath "$taskSdk/platform-tools/adb.exe")){throw "Android SDK not found. Set ANDROID_HOME."}
[IO.File]::WriteAllText((Join-Path $projectRoot "local.properties"),"sdk.dir="+$taskSdk.Replace('\','/')+"`n")
$taskCargo=Join-Path $env:USERPROFILE ".cargo/bin/cargo.exe"
if(!(Test-Path -LiteralPath $taskCargo)){throw "Install Rust using rustup (cargo is missing)."}
$env:CARGO=$taskCargo
if(!(Test-Path -LiteralPath "$projectRoot/.tools/android-ndk-r27c/toolchains/llvm/prebuilt/windows-x86_64/bin/clang.exe")){throw "NDK r27c is missing from the .tools directory."}
$taskTargets=& (Join-Path $env:USERPROFILE ".cargo/bin/rustup.exe") target list --installed
foreach($taskTarget in @("aarch64-linux-android","i686-linux-android")){
    if($taskTargets -notcontains $taskTarget){throw "Missing Rust target $taskTarget. Run: rustup target add $taskTarget"}
}
$taskList=@(":app:testDebugUnitTest",":app:lintDebug",":app:assembleRelease")
if($DeviceTests){$taskList+=@(':app:assembleDebug',':app:assembleDebugAndroidTest')}
Write-Host "Building KAT Mobile RX from $projectRoot ..." -ForegroundColor Cyan
& "$projectRoot/gradlew.bat" --no-daemon @taskList --console=plain
if($LASTEXITCODE -ne 0){throw "Build failed. Review the output above."}
New-Item -ItemType Directory -Force -Path "$projectRoot/dist" | Out-Null
Copy-Item -LiteralPath "$projectRoot/app/build/outputs/apk/release/app-release.apk" -Destination "$projectRoot/dist/KAT-Mobile-RX.apk"
$taskHash=(Get-FileHash -LiteralPath "$projectRoot/dist/KAT-Mobile-RX.apk" -Algorithm SHA256).Hash
Set-Content -LiteralPath "$projectRoot/dist/KAT-Mobile-RX.apk.sha256" -Value "$taskHash  KAT-Mobile-RX.apk"
$taskBuildTools=Get-ChildItem -LiteralPath "$taskSdk/build-tools" -Directory | Where-Object {Test-Path -LiteralPath (Join-Path $_.FullName 'apksigner.bat')} | Sort-Object Name -Descending | Select-Object -First 1
& (Join-Path $taskBuildTools.FullName 'apksigner.bat') verify --verbose "$projectRoot/dist/KAT-Mobile-RX.apk"
if($LASTEXITCODE -ne 0){throw "APK signature verification failed."}
& (Join-Path $taskBuildTools.FullName 'zipalign.exe') -c -P 16 4 "$projectRoot/dist/KAT-Mobile-RX.apk"
if($LASTEXITCODE -ne 0){throw "APK alignment verification failed."}
if($Install -or $DeviceTests){
    if(!$Serial){throw "Specify the device using -Serial, for example .\Build.ps1 -Install -Serial YOUR_DEVICE_SERIAL"}
    $taskAdb=Join-Path $taskSdk 'platform-tools/adb.exe'
    if($DeviceTests){
        & $taskAdb -s $Serial install -r "$projectRoot/app/build/outputs/apk/debug/app-debug.apk"
        if($LASTEXITCODE -ne 0){throw 'QA installation failed.'}
        & $taskAdb -s $Serial install -r "$projectRoot/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk"
        if($LASTEXITCODE -ne 0){throw 'Test installation failed.'}
        $deviceApi=[int](& $taskAdb -s $Serial shell getprop ro.build.version.sdk)
        if($deviceApi -ge 33){& $taskAdb -s $Serial shell pm grant app.katmobile.qa android.permission.POST_NOTIFICATIONS}
        $taskTestOutput=& $taskAdb -s $Serial shell am instrument -w -e class app.katmobile.AppTest app.katmobile.qa.test/androidx.test.runner.AndroidJUnitRunner
        $taskTestOutput | Tee-Object -FilePath "$projectRoot/dist/DEVICE-TESTOVI.txt"
        if(($taskTestOutput -join "`n") -notmatch 'OK \(\d+ tests\)'){throw 'Device tests failed.'}
    }
    if($Install){
        & $taskAdb -s $Serial install -r "$projectRoot/dist/KAT-Mobile-RX.apk"
        if($LASTEXITCODE -ne 0){throw 'Release APK installation failed.'}
        & $taskAdb -s $Serial shell am start -n app.katmobile/.MainActivity
    }
}
Write-Host "Ready: $projectRoot/dist/KAT-Mobile-RX.apk" -ForegroundColor Green
Write-Host "Build instructions: $projectRoot/BUILD.txt"
