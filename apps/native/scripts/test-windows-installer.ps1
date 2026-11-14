param([Parameter(Mandatory)][string]$Installer, [ValidateSet('rust')][string]$Backend = 'rust')
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = (Resolve-Path "$PSScriptRoot/../../..").Path
$installerPath = (Resolve-Path $Installer).Path
$stage = Join-Path $root 'apps/native/target/windows-package'
$harness = Join-Path $root 'apps/native/target/release/native-harness.exe'
$testRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("native-install-" + [guid]::NewGuid())
# Spaces exercise installer command-line quoting and sibling binary resolution.
$product = 'Workspacer Native'
$appId = 'Workspacer.Native'
# The identity the app shipped under as a preview; an install over it upgrades in place.
$legacyProduct = 'Workspacer Native Rust Preview'
$legacyKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$legacyProduct"
$legacyInstallKey = "HKCU:\Software\$legacyProduct"
$legacyShortcutPath = Join-Path ([Environment]::GetFolderPath('Programs')) "$legacyProduct.lnk"
$legacyData = Join-Path $testRoot "data-$legacyProduct"
$installDir = Join-Path $testRoot $product
$notificationKey = "HKCU:\Software\Classes\AppUserModelId\$appId"
$key = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$product"
if (Test-Path $key) { throw 'Smoke test requires a user without Workspacer Native installed' }
$shortcutPath = Join-Path ([Environment]::GetFolderPath('Programs')) "$product.lnk"
if (Test-Path $shortcutPath) { throw 'Smoke test refuses to replace an existing shortcut' }
if ((Test-Path $legacyKey) -or (Test-Path $legacyInstallKey) -or (Test-Path $legacyShortcutPath)) {
    throw 'Smoke test requires a user without the Workspacer Native Rust Preview installed'
}
$simulatedLegacy = $false
New-Item -ItemType Directory -Path $testRoot | Out-Null
$uninstalled = $false
$savedEnv = @{}
foreach ($name in @('APPDATA', 'LOCALAPPDATA', 'XDG_CONFIG_HOME', 'XDG_DATA_HOME', 'WORKSPACER_USAGE_POLL_ON_BOOT', 'PATH', 'WKS_DESKTOP_HOST')) {
    $savedEnv[$name] = [Environment]::GetEnvironmentVariable($name)
}
function Invoke-Installer {
    # NSIS requires /D last, without quotes, even when its value contains spaces.
    $process = Start-Process $installerPath -ArgumentList "/S /D=$installDir" -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Install failed: $($process.ExitCode)" }
}
function Invoke-Uninstaller {
    $uninstaller = Join-Path $installDir 'Uninstall.exe'
    if (Test-Path $uninstaller) {
        # _?= runs synchronously in place, so -Wait really waits for removal.
        $process = Start-Process $uninstaller -ArgumentList "/S _?=$installDir" -Wait -PassThru
        if ($process.ExitCode -ne 0) { throw "Uninstall failed: $($process.ExitCode)" }
    }
}
try {
    Invoke-Installer
    $shortcut = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcutPath)
    if ($shortcut.TargetPath -ne (Join-Path $installDir 'wks-native.exe') -or $shortcut.Arguments -ne '--local') {
        throw 'Start menu shortcut must launch the installed GUI in local mode'
    }
    if (!(Test-Path $key)) { throw 'Windows uninstall registration missing' }
    if (!(Test-Path $notificationKey) -or (Get-ItemProperty $notificationKey).DisplayName -ne $product) { throw 'Native notification identity missing' }
    $expectedUninstall = '"' + (Join-Path $installDir 'Uninstall.exe') + '"'
    $registration = Get-ItemProperty $key
    if ($registration.UninstallString -ne $expectedUninstall -or $registration.QuietUninstallString -ne "$expectedUninstall /S") {
        throw 'Windows uninstall commands must quote the installed path without literal dollar signs'
    }
    $version = (Get-Content (Join-Path $stage 'build-stamp.json') | ConvertFrom-Json).version
    if ((Get-ItemProperty $key).DisplayVersion -ne $version) { throw 'Installed version does not match the release' }
    # Upgrade in place must preserve data and reproduce the complete payload.
    Set-Content (Join-Path $installDir 'user-file.txt') 'retain me'
    # Simulate a preview install in this folder: only its registration exists,
    # and the installer is run WITHOUT /D=, as a user double-clicking it would.
    # It must find the preview's folder, upgrade it in place and leave exactly
    # one registration, without touching the preview's data folder.
    $simulatedLegacy = $true
    Remove-Item "HKCU:\Software\$product" -Recurse -Force
    Remove-Item $key -Recurse -Force
    New-Item $legacyInstallKey -Force | Out-Null
    Set-ItemProperty $legacyInstallKey -Name InstallDir -Value $installDir
    New-Item $legacyKey -Force | Out-Null
    Set-ItemProperty $legacyKey -Name DisplayName -Value $legacyProduct
    Set-ItemProperty $legacyKey -Name InstallLocation -Value $installDir
    Set-ItemProperty $legacyKey -Name UninstallString -Value ('"' + (Join-Path $installDir 'Uninstall.exe') + '"')
    $legacyShortcut = (New-Object -ComObject WScript.Shell).CreateShortcut($legacyShortcutPath)
    $legacyShortcut.TargetPath = Join-Path $installDir 'wks-native.exe'
    $legacyShortcut.Save()
    New-Item -ItemType Directory -Path $legacyData | Out-Null
    Set-Content (Join-Path $legacyData 'state.db') 'preview data'
    Set-Content (Join-Path $installDir 'build-stamp.json') '{"version":"preview"}'
    $process = Start-Process $installerPath -ArgumentList '/S' -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Install over a preview failed: $($process.ExitCode)" }
    if ((Get-Content (Join-Path $installDir 'build-stamp.json') | ConvertFrom-Json).version -ne $version) {
        throw 'Install without /D= did not upgrade the preview folder in place'
    }
    if ((Test-Path $legacyKey) -or (Test-Path $legacyInstallKey)) { throw 'Preview registration survived the upgrade' }
    if (Test-Path $legacyShortcutPath) { throw 'Preview Start menu shortcut survived the upgrade' }
    if (!(Test-Path $key) -or (Get-ItemProperty $key).InstallLocation -ne $installDir) { throw 'Upgrade over a preview did not register Workspacer Native in place' }
    if ((Get-ItemProperty "HKCU:\Software\$product").InstallDir -ne $installDir) { throw 'Upgrade over a preview did not record its install folder' }
    if ((Get-Content (Join-Path $legacyData 'state.db')) -ne 'preview data') { throw 'Upgrade over a preview touched user data' }
    if ((Get-Content (Join-Path $installDir 'user-file.txt')) -ne 'retain me') { throw 'Upgrade over a preview removed user files' }
    $simulatedLegacy = $false
    foreach ($file in Get-ChildItem $stage -File -Recurse) {
        $relative = [System.IO.Path]::GetRelativePath($stage, $file.FullName)
        $installed = Join-Path $installDir $relative
        if ((Get-FileHash $installed).Hash -ne (Get-FileHash $file.FullName).Hash) {
            throw "Installed payload differs: $relative"
        }
    }
    # The in-app updater trusts the silent installer's exit code. A payload
    # file it cannot replace (NSIS skips it silently) must fail the install.
    $locked = [System.IO.File]::Open((Join-Path $installDir 'build-stamp.json'), 'Open', 'Read', 'None')
    try {
        $process = Start-Process $installerPath -ArgumentList "/S /D=$installDir" -Wait -PassThru
        if ($process.ExitCode -eq 0) { throw 'Silent install reported success over a locked payload file' }
    } finally {
        $locked.Dispose()
    }
    # A locked main binary must fail the install AND leave the previous stamp,
    # or the app reports itself current while still running the old build.
    $stampPath = Join-Path $installDir 'build-stamp.json'
    Set-Content $stampPath '{"version":"previous"}'
    $locked = [System.IO.File]::Open((Join-Path $installDir 'wks-native.exe'), 'Open', 'Read', 'None')
    try {
        $process = Start-Process $installerPath -ArgumentList "/S /D=$installDir" -Wait -PassThru
        if ($process.ExitCode -eq 0) { throw 'Silent install reported success over a locked wks-native.exe' }
    } finally {
        $locked.Dispose()
    }
    if ((Get-Content $stampPath -Raw).Trim() -ne '{"version":"previous"}') {
        throw 'A failed install must not advance build-stamp.json'
    }
    # A lock that clears shortly after the app exits (the updater's case) must
    # be retried, not failed.
    $locked = [System.IO.File]::Open((Join-Path $installDir 'wks-native.exe'), 'Open', 'Read', 'None')
    $process = Start-Process $installerPath -ArgumentList "/S /D=$installDir" -PassThru
    # Without a cached handle, ExitCode reads null after a non -Wait start.
    $null = $process.Handle
    try {
        Start-Sleep -Seconds 3
    } finally {
        $locked.Dispose()
    }
    $process.WaitForExit()
    if ($process.ExitCode -ne 0) { throw "Install did not retry a transiently locked wks-native.exe: $($process.ExitCode)" }
    if ((Get-Content $stampPath | ConvertFrom-Json).version -ne $version) { throw 'Retried install did not write the new stamp' }
    Invoke-Installer
    # Isolate backend state without changing the actual user's home directory.
    $env:APPDATA = Join-Path $testRoot 'config'
    $env:LOCALAPPDATA = Join-Path $testRoot 'data'
    $env:XDG_CONFIG_HOME = $env:APPDATA
    $env:XDG_DATA_HOME = $env:LOCALAPPDATA
    $env:WORKSPACER_USAGE_POLL_ON_BOOT = '0'
    Remove-Item Env:WKS_DESKTOP_HOST -ErrorAction SilentlyContinue
    # Verify the installed backend runs with Node absent from PATH.
    $env:PATH = "$env:SystemRoot\System32;$env:SystemRoot"
    foreach ($forbidden in @('node.exe','workspacer.exe','hub.exe','brain.exe','mcp.exe','desktop-host.cjs','NODE-LICENSE.txt')) {
        if (Test-Path (Join-Path $installDir $forbidden)) { throw "Rust payload contains legacy companion: $forbidden" }
    }
    & (Join-Path $installDir 'workspacer-rust.exe') --help
    if ($LASTEXITCODE -ne 0) { throw 'Installed Rust standalone CLI failed' }
    & $harness rust-probe --directory $testRoot
    if ($LASTEXITCODE -ne 0) { throw 'Installed Rust backend probe failed' }
    # Restore the shell paths before invoking the uninstall machinery.
    foreach ($name in $savedEnv.Keys) { [Environment]::SetEnvironmentVariable($name, $savedEnv[$name]) }
    Invoke-Uninstaller
    $uninstalled = $true
    if (Test-Path $key) { throw 'Uninstall registration was not removed' }
    if (Test-Path $notificationKey) { throw 'Native notification identity was not removed' }
    if (Test-Path $shortcutPath) { throw 'Start menu shortcut was not removed' }
    foreach ($file in Get-ChildItem $stage -File -Recurse) {
        $relative = [System.IO.Path]::GetRelativePath($stage, $file.FullName)
        if (Test-Path (Join-Path $installDir $relative)) { throw "Uninstall left packaged file: $relative" }
    }
    if ((Get-Content (Join-Path $installDir 'user-file.txt')) -ne 'retain me') { throw 'Uninstall removed user files' }
    if (!(Test-Path (Join-Path $testRoot 'state.db'))) { throw 'Uninstall removed session data' }
    if ((Get-Content (Join-Path $legacyData 'state.db')) -ne 'preview data') { throw 'Uninstall removed preview data' }
    Write-Host 'Native install, upgrade, preview migration, embedded backend, shutdown and uninstall passed.'
} finally {
    foreach ($name in $savedEnv.Keys) { [Environment]::SetEnvironmentVariable($name, $savedEnv[$name]) }
    if (!$uninstalled) { Invoke-Uninstaller }
    if ($simulatedLegacy) {
        # Only what this script created, when the scenario failed part-way.
        Remove-Item $legacyKey, $legacyInstallKey -Recurse -Force -ErrorAction SilentlyContinue
        Remove-Item $legacyShortcutPath -Force -ErrorAction SilentlyContinue
    }
    Remove-Item $testRoot -Recurse -Force -ErrorAction SilentlyContinue
}
