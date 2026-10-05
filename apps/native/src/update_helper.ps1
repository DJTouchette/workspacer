# Workspacer Native update helper (Windows PowerShell 5.1+).
#
# Started hidden by the app (src/updates.rs) with the path of a JSON plan in
# WKS_UPDATE_PLAN. This text is constant: every path and argument comes from
# the plan, so nothing user-controlled is ever parsed as PowerShell. Avoid
# double quotes here; the script travels on a Windows command line.
#
# Steps, each recorded in the plan's state file and log:
#   waiting    holding a handle to the app, until it exits
#   installing no process runs from the install folder; the silent per-user
#              installer runs in place (/S /D=<install folder>)
#   succeeded  installer exit code 0 and build-stamp.json reports the
#              expected version
#   failed     anything else, with the reason
# The app is relaunched with its original arguments and working directory
# whenever it has exited, whether or not the install succeeded.
$ErrorActionPreference = 'Stop'
$planPath = $env:WKS_UPDATE_PLAN
Remove-Item Env:WKS_UPDATE_PLAN -ErrorAction SilentlyContinue
$plan = [IO.File]::ReadAllText($planPath) | ConvertFrom-Json

function Write-Log([string]$text) {
    [IO.File]::AppendAllText($plan.log, (Get-Date).ToString('o') + ' ' + $text + [Environment]::NewLine)
}

function Set-State([string]$name, [string]$detail) {
    $record = [ordered]@{
        nonce = $plan.nonce
        state = $name
        detail = $detail
        expected = $plan.expected
        installer = $plan.installer
        log = $plan.log
        parent = $plan.parentId
        helper = $PID
        time = (Get-Date).ToUniversalTime().ToString('o')
    }
    $temporary = $plan.state + '.tmp'
    Write-Log ('state ' + $name + ' ' + $detail)
    [IO.File]::WriteAllText($temporary, ($record | ConvertTo-Json -Compress))
    if ([IO.File]::Exists($plan.state)) { [IO.File]::Delete($plan.state) }
    [IO.File]::Move($temporary, $plan.state)
}

function Start-App {
    try {
        $start = New-Object Diagnostics.ProcessStartInfo
        $start.FileName = $plan.exe
        $start.Arguments = $plan.arguments
        $start.WorkingDirectory = $plan.cwd
        $start.UseShellExecute = $false
        $app = [Diagnostics.Process]::Start($start)
        Write-Log ('relaunched ' + $plan.exe + ' as ' + $app.Id)
    } catch {
        Set-State 'failed' ('Workspacer could not restart: ' + $_.Exception.Message)
        Write-Log ('relaunch failed: ' + $_.Exception.Message)
    }
}

function Normalize-ImagePath([string]$path) {
    if ($path.StartsWith('\\?\UNC\', [StringComparison]::OrdinalIgnoreCase)) { return '\\' + $path.Substring(8) }
    if ($path.StartsWith('\\?\', [StringComparison]::OrdinalIgnoreCase)) { return $path.Substring(4) }
    return $path
}

function Get-ImagePath($process) {
    try {
        $path = $process.Path
        if ($path) { return Normalize-ImagePath ([string]$path) }
    } catch {}
    return ''
}

$exited = $false
$ownsLease = $false
$installDir = Normalize-ImagePath ([string]$plan.installDir)
# A second app/helper must not race an already accepted installation. Plans
# are per nonce; only the lease owner may replace the shared outcome record.
$hasher = [Security.Cryptography.SHA256]::Create()
$key = [BitConverter]::ToString($hasher.ComputeHash([Text.Encoding]::UTF8.GetBytes((Normalize-ImagePath ([IO.Path]::GetDirectoryName([string]$plan.state))).ToUpperInvariant()))).Replace('-', '')
$hasher.Dispose()
$lease = New-Object Threading.Mutex($false, ('Local\WorkspacerNativeUpdate-' + $key))
try {
    try { $ownsLease = $lease.WaitOne(0) } catch [Threading.AbandonedMutexException] { $ownsLease = $true }
    if (-not $ownsLease) {
        [Console]::Error.WriteLine('Another Workspacer update helper already owns this installation. Nothing was installed.')
        exit 8
    }
    Write-Log ('helper ' + $PID + ' started for Workspacer ' + $plan.parentId + ', installing ' + $plan.expected)
    $parent = [Diagnostics.Process]::GetProcessById([int]$plan.parentId)
    # Keep a handle so a reused process ID can never satisfy the wait.
    $null = $parent.Handle
    Set-State 'waiting' ''
    if (-not $parent.WaitForExit([int]$plan.appExitMs)) {
        Set-State 'failed' 'Workspacer did not close, so nothing was installed.'
        exit 3
    }
    $exited = $true
    Write-Log 'Workspacer exited'

    # The installer cannot replace files that are still running.
    $prefix = $installDir.TrimEnd('\') + '\'
    $deadline = [DateTime]::UtcNow.AddMilliseconds([int]$plan.siblingMs)
    while ($true) {
        $busy = @(Get-Process | Where-Object {
            $_.Id -ne $PID -and (Get-ImagePath $_).StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)
        })
        if ($busy.Count -eq 0) { break }
        if ([DateTime]::UtcNow -ge $deadline) {
            $names = ($busy | ForEach-Object { $_.ProcessName + ' (' + $_.Id + ')' }) -join ', '
            Set-State 'failed' ('Still running from the install folder, so nothing was installed: ' + $names)
            Start-App
            exit 4
        }
        Start-Sleep -Milliseconds 250
    }

    Set-State 'installing' ''
    $setup = New-Object Diagnostics.ProcessStartInfo
    $setup.FileName = $plan.installer
    # NSIS: /D must be last and unquoted, even with spaces.
    $setup.Arguments = '/S /D=' + $plan.installDir
    $setup.UseShellExecute = $false
    $installer = [Diagnostics.Process]::Start($setup)
    if (-not $installer.WaitForExit([int]$plan.installMs)) {
        Set-State 'failed' ('The installer is still running after the timeout. Workspacer was not restarted while files may be changing. Wait for the installer to finish before reopening Workspacer or retrying. Log: ' + $plan.log)
        # Keep the update lease until the installer actually exits. Releasing
        # it here would allow another helper to overwrite a live installation.
        $installer.WaitForExit()
    }
    $code = $installer.ExitCode
    Write-Log ('installer exited with code ' + $code)
    if ($code -ne 0) {
        Set-State 'failed' ('The installer exited with code ' + $code + '. Run it manually: ' + $plan.installer)
        Start-App
        exit 6
    }
    $installed = ''
    try {
        $stamp = [IO.File]::ReadAllText([IO.Path]::Combine($plan.installDir, 'build-stamp.json')) | ConvertFrom-Json
        $installed = [string]$stamp.version
    } catch {}
    if ($installed -ne $plan.expected) {
        Set-State 'failed' ('The installer finished, but the installed version is ' + $installed + ' instead of ' + $plan.expected + '.')
        Start-App
        exit 7
    }
    Set-State 'succeeded' ''
    try { [IO.File]::Delete($plan.installer) } catch {}
    Start-App
    exit 0
} catch {
    $message = $_.Exception.Message
    if ($ownsLease) { try { Set-State 'failed' ('The update helper stopped: ' + $message) } catch {} }
    [Console]::Error.WriteLine($message)
    if ($exited) { Start-App }
    exit 1
} finally {
    if ($ownsLease) { $lease.ReleaseMutex() }
    $lease.Dispose()
    try { [IO.File]::Delete($planPath) } catch {}
}
