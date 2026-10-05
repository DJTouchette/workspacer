param([Parameter(Mandatory)][string]$Installer, [ValidateSet('rust')][string]$Backend = 'rust')
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = (Resolve-Path "$PSScriptRoot/../../..").Path
$installerPath = (Resolve-Path $Installer).Path
$stage = Join-Path $root 'apps/native/target/windows-package'
$harness = Join-Path $root 'apps/native/target/release/native-harness.exe'
$testRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("native-install-" + [guid]::NewGuid())
# Spaces exercise installer command-line quoting and sibling binary resolution.
$product = 'Workspacer Native Rust Preview'
$appId = 'Workspacer.Native.RustPreview'
$installDir = Join-Path $testRoot $product
$notificationKey = "HKCU:\Software\Classes\AppUserModelId\$appId"
$key = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$product"
if (Test-Path $key) { throw 'Smoke test requires a user without Workspacer Native installed' }
$shortcutPath = Join-Path ([Environment]::GetFolderPath('Programs')) "$product.lnk"
if (Test-Path $shortcutPath) { throw 'Smoke test refuses to replace an existing shortcut' }
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
    Invoke-Installer
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
    Write-Host 'Native install, upgrade, embedded backend, shutdown and uninstall passed.'
} finally {
    foreach ($name in $savedEnv.Keys) { [Environment]::SetEnvironmentVariable($name, $savedEnv[$name]) }
    if (!$uninstalled) { Invoke-Uninstaller }
    Remove-Item $testRoot -Recurse -Force -ErrorAction SilentlyContinue
}
