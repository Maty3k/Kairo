# SPDX-License-Identifier: GPL-3.0-or-later
#
# tools/pc/bench/tests/testlib.ps1 - helpers for the bench scripts' tests.
# Project: https://github.com/Maty3k/Kairo
#
# Each <script>.tests.ps1 dot-sources this file, runs its cases with Test-Case and ends with
# Complete-BenchTests, which prints a summary and exits 1 if any case failed, else 0:
#
#   . (Join-Path $PSScriptRoot 'testlib.ps1')
#   . (Join-Path $BenchDir 'pm-stats.ps1')          # the script as a library, under the strict mode below
#   Test-Case 'P1 ten presents' { Assert-Equal 10 $r.Stats['frames'] 'frames' }
#   $run = Invoke-BenchTool 'pm-stats.ps1' @('-Csv', $file, '-Exe', 'kairo-loadgen.exe', '-Tag', 't')
#   Test-Case 'E1 no file' { Assert-Equal 3 $run.ExitCode 'exit code' }
#   Complete-BenchTests
#
# Invoke-BenchTool runs a script in a child process of the same PowerShell (pwsh, or Windows
# PowerShell 5.1's powershell.exe) with -NoProfile -NonInteractive -File, as the command line and
# the elevated session run it, and returns its exit code, standard output and standard error.
#
# Rules: ASCII only; Windows PowerShell 5.1 syntax and .NET Framework 4.x APIs; no Add-Type.

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

$BenchDir = Split-Path -Parent $PSScriptRoot
$script:BenchTestCount = 0
$script:BenchTestFailures = [Collections.Generic.List[string]]::new()
$script:BenchTestDirs = [Collections.Generic.List[string]]::new()

function Test-Case([string] $Name, [scriptblock] $Body) {
    # Runs one case: any exception, a failed assertion included, fails it with its message.
    $script:BenchTestCount++
    try {
        & $Body
        [Console]::Out.WriteLine('ok   ' + $Name)
    } catch {
        $script:BenchTestFailures.Add($Name)
        [Console]::Out.WriteLine('FAIL ' + $Name + ': ' + $_.Exception.Message)
    }
}

function Assert-True($Condition, [string] $What) {
    if (-not $Condition) { throw ('expected true: ' + $What) }
}

function Assert-Equal($Expected, $Actual, [string] $What) {
    # Exact equality; strings compare case-sensitively. Numbers of different types (40 and 40.0)
    # are equal when their values are. A bool equals only a bool: PowerShell's -eq would make
    # $true equal 1 (or any number but 0), so a JSON boolean printed as a number would pass.
    $same = $false
    if ($null -eq $Expected -or $null -eq $Actual) {
        $same = ($null -eq $Expected -and $null -eq $Actual)
    } elseif ($Expected -is [bool] -or $Actual -is [bool]) {
        $same = ($Expected -is [bool] -and $Actual -is [bool] -and $Expected -eq $Actual)
    } elseif ($Expected -is [string] -or $Actual -is [string]) {
        $same = ([string]$Expected -ceq [string]$Actual)
    } elseif ((Test-BenchNumber $Expected) -and (Test-BenchNumber $Actual)) {
        $same = ([double]$Expected -eq [double]$Actual)
    } else {
        $same = ($Expected -eq $Actual)
    }
    if (-not $same) { throw ($What + ': expected <' + (Format-BenchValue $Expected) + '>, got <' + (Format-BenchValue $Actual) + '>') }
}

function Assert-Near([double] $Expected, $Actual, [double] $Tolerance, [string] $What) {
    if ($null -eq $Actual -or -not (Test-BenchNumber $Actual)) { throw ($What + ': expected a number near ' + $Expected + ', got <' + (Format-BenchValue $Actual) + '>') }
    if ([Math]::Abs([double]$Actual - $Expected) -gt $Tolerance) { throw ($What + ': expected ' + $Expected + ' +/- ' + $Tolerance + ', got ' + [double]$Actual) }
}

function Assert-Match([string] $Pattern, [string] $Text, [string] $What) {
    if ($Text -cnotmatch $Pattern) { throw ($What + ': <' + $Text + '> does not match ' + $Pattern) }
}

function Assert-Throws([scriptblock] $Body, [string] $What) {
    # Runs $Body, which must throw; returns the error record.
    try {
        & $Body
    } catch {
        return $_
    }
    throw ('expected an error: ' + $What)
}

function Test-BenchNumber($Value) {
    return ($Value -is [int] -or $Value -is [long] -or $Value -is [double] -or $Value -is [decimal] -or $Value -is [single] -or $Value -is [int16] -or $Value -is [byte])
}

function Format-BenchValue($Value) {
    # A bool prints as $true or $false, so a failure tells it from the text 'True' and from 1.
    if ($null -eq $Value) { return '$null' }
    if ($Value -is [bool]) {
        if ($Value) { return '$true' }
        return '$false'
    }
    if (Test-BenchNumber $Value) { return ([double]$Value).ToString('R', [Globalization.CultureInfo]::InvariantCulture) }
    return [string]$Value
}

function New-BenchTempDir {
    # A new empty folder, removed by Complete-BenchTests.
    $dir = Join-Path ([IO.Path]::GetTempPath()) ('kairo-bench-test-' + [guid]::NewGuid().ToString('N'))
    [void][IO.Directory]::CreateDirectory($dir)
    $script:BenchTestDirs.Add($dir)
    return $dir
}

function ConvertTo-BenchArgument([string] $Text) {
    # One argument quoted for a Windows command line (CommandLineToArgvW's rules).
    if ($Text.Length -gt 0 -and $Text -notmatch '[\s"]') { return $Text }
    $out = [Text.StringBuilder]::new()
    [void]$out.Append('"')
    $slashes = 0
    foreach ($c in $Text.ToCharArray()) {
        if ($c -eq [char]'\') {
            $slashes++
            continue
        }
        if ($c -eq [char]'"') {
            [void]$out.Append([char]'\', 2 * $slashes + 1)
        } elseif ($slashes -gt 0) {
            [void]$out.Append([char]'\', $slashes)
        }
        $slashes = 0
        [void]$out.Append($c)
    }
    if ($slashes -gt 0) { [void]$out.Append([char]'\', 2 * $slashes) }
    [void]$out.Append('"')
    return $out.ToString()
}

function Get-BenchShell {
    # The PowerShell running the tests, so 5.1 tests 5.1 and 7 tests 7.
    if ($PSVersionTable.PSEdition -eq 'Core') {
        $name = 'pwsh'
        if ($IsWindows) { $name = 'pwsh.exe' }
        return (Join-Path $PSHOME $name)
    }
    return (Join-Path $PSHOME 'powershell.exe')
}

function Invoke-BenchTool([string] $Script, [string[]] $Arguments = @(), [string] $StdIn = $null) {
    # Runs tools/pc/bench/<Script> with -File; returns ExitCode, StdOut and StdErr.
    $path = $Script
    if (-not [IO.Path]::IsPathRooted($path)) { $path = Join-Path $BenchDir $Script }
    $parts = @('-NoProfile', '-NonInteractive', '-File', $path) + @($Arguments)
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = Get-BenchShell
    $info.Arguments = (@($parts | ForEach-Object { ConvertTo-BenchArgument ([string]$_) }) -join ' ')
    $info.UseShellExecute = $false
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $info.RedirectStandardInput = $true
    $info.CreateNoWindow = $true
    $process = [Diagnostics.Process]::Start($info)
    try {
        $errTask = $process.StandardError.ReadToEndAsync()
        if ($null -ne $StdIn) { $process.StandardInput.Write($StdIn) }
        $process.StandardInput.Close()
        $out = $process.StandardOutput.ReadToEnd()
        $process.WaitForExit()
        return [pscustomobject]@{ ExitCode = $process.ExitCode; StdOut = $out; StdErr = $errTask.Result }
    } finally {
        $process.Dispose()
    }
}

function Complete-BenchTests {
    foreach ($dir in $script:BenchTestDirs) {
        try { [IO.Directory]::Delete($dir, $true) } catch { [Console]::Out.WriteLine('note: could not remove a test folder: ' + $_.Exception.Message) }
    }
    $failed = $script:BenchTestFailures.Count
    [Console]::Out.WriteLine(([string]$script:BenchTestCount) + ' cases, ' + $failed + ' failed')
    if ($failed -gt 0) { exit 1 }
    exit 0
}
