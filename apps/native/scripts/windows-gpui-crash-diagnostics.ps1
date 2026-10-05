# Evidence for a crashed serial GPUI suite on Windows (run after the full
# suite failed; it never replaces it). Writes everything to $Out:
#   serial.log    the whole wks-native unit binary, serially, under cdb when the
#                 runner has it: the faulting thread's stack and every thread's
#   isolated.tsv  every test alone in its own process, with its exit code
#   repeat.log    the test that was running when the serial binary died, alone
#                 and after its predecessor, $Repeat times each
#   dumps/        WER minidumps of anything that crashed, plus the binary + PDB
param([string]$Out = 'gpui-crash', [int]$Repeat = 5, [int]$TestSeconds = 180)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$Out = (New-Item -ItemType Directory -Force -Path $Out).FullName
$dumps = (New-Item -ItemType Directory -Force -Path (Join-Path $Out 'dumps')).FullName

# The bin's unit-test executable (the lib's shares its wks_native-*.exe name).
$exe = cargo test --locked --features ui-tests --bin wks-native --no-run --message-format=json |
    ForEach-Object { $_ | ConvertFrom-Json } |
    Where-Object { $_.reason -eq 'compiler-artifact' -and $_.executable -and $_.target.kind -contains 'bin' } |
    Select-Object -Last 1 -ExpandProperty executable
if (-not $exe) { throw 'wks-native unit-test executable not found' }
"executable: $exe"

$wer = 'HKLM:\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps'
New-Item -Force -Path $wer | Out-Null
New-ItemProperty -Force -Path $wer -Name DumpFolder -PropertyType ExpandString -Value $dumps | Out-Null
New-ItemProperty -Force -Path $wer -Name DumpType -PropertyType DWord -Value 1 | Out-Null
New-ItemProperty -Force -Path $wer -Name DumpCount -PropertyType DWord -Value 20 | Out-Null

function Invoke-Bounded([string[]]$Arguments, [string]$Log) {
    $p = Start-Process -FilePath $exe -ArgumentList $Arguments -NoNewWindow -PassThru `
        -RedirectStandardOutput "$Log.out" -RedirectStandardError "$Log.err"
    $null = $p.Handle  # keeps ExitCode readable after exit
    if (-not $p.WaitForExit($TestSeconds * 1000)) {
        $p.Kill()
        $p.WaitForExit()
        return 'timeout'
    }
    return ('0x{0:x8}' -f $p.ExitCode)
}

$serial = Join-Path $Out 'serial.log'
$cdb = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\Debuggers\x64\cdb.exe'
if (Test-Path $cdb) {
    $symbols = "$(Split-Path $exe);srv*$env:RUNNER_TEMP\symbols*https://msdl.microsoft.com/download/symbols"
    # -g/-G skip the start and exit breakpoints, so the commands run only at
    # the first exception the binary does not survive on its own (an access
    # violation breaks first-chance; Rust panics are C++ EH, second-chance).
    & $cdb -g -G -lines -y $symbols `
        -c '.lastevent; r; .ecxr; kpn 100; ~*kn 40; q' `
        $exe --test-threads=1 *> $serial
    "cdb exit: $LASTEXITCODE"
} else {
    "cdb not installed; serial run without a debugger (WER dumps only)"
    $code = Invoke-Bounded @('--test-threads=1') $serial
    Get-Content "$serial.out", "$serial.err" | Set-Content $serial
    "serial exit: $code"
}

# libtest flushes `test NAME ... ` before running a serial test and the result
# after it, so a name without a result is the test the process died in (the
# debugger's or the next writer's output may share its line).
$lines = @(Get-Content $serial)
$order = @($lines | Select-String -Pattern '^test (\S+) \.\.\. ' | ForEach-Object { $_.Matches[0].Groups[1].Value })
$finished = @($lines | Select-String -Pattern '^test (\S+) \.\.\. (ok|FAILED|ignored)$' | ForEach-Object { $_.Matches[0].Groups[1].Value })
$crashed = $order | Where-Object { $finished -notcontains $_ } | Select-Object -Last 1
"crashed in: $(if ($crashed) { $crashed } else { '(no unfinished test in the serial run)' })"
$lines | Select-String -Pattern 'Access violation|c0000005|ExceptionCode|^\s*\d+ [0-9a-f`]+ [0-9a-f`]+ ' -Context 0, 0 |
    Select-Object -First 120 | ForEach-Object { $_.Line }

$tests = @(& $exe --list --format terse | Where-Object { $_ -match ': test$' } | ForEach-Object { $_ -replace ': test$', '' })
$table = Join-Path $Out 'isolated.tsv'
"test`texit" | Set-Content $table
foreach ($name in $tests) {
    $code = Invoke-Bounded @('--exact', $name, '--test-threads=1') (Join-Path $Out 'isolated-last')
    "$name`t$code" | Add-Content $table
    if ($code -ne '0x00000000') { "isolated $name -> $code" }
}
"isolated: $($tests.Count) tests, one process each"

if ($crashed) {
    $repeatLog = Join-Path $Out 'repeat.log'
    $index = [array]::IndexOf($order, $crashed)
    $previous = if ($index -gt 0) { $order[$index - 1] } else { $null }
    for ($i = 1; $i -le $Repeat; $i++) {
        $alone = Invoke-Bounded @('--exact', $crashed, '--test-threads=1') (Join-Path $Out 'repeat-alone')
        "alone #$i $crashed -> $alone" | Tee-Object -Append $repeatLog
        if ($previous) {
            $pair = Invoke-Bounded @('--exact', $previous, $crashed, '--test-threads=1') (Join-Path $Out 'repeat-pair')
            "after $previous #$i -> $pair" | Tee-Object -Append $repeatLog
        }
    }
}

Copy-Item $exe $dumps
$pdb = [IO.Path]::ChangeExtension($exe, 'pdb')
if (Test-Path $pdb) { Copy-Item $pdb $dumps }
Get-ChildItem $dumps -Filter *.dmp | ForEach-Object { "dump: $($_.Name) $($_.Length) bytes" }
