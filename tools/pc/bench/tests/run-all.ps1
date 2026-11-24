# SPDX-License-Identifier: GPL-3.0-or-later
#
# tools/pc/bench/tests/run-all.ps1 - runs every *.tests.ps1 beside it, each in a child process of
# this same PowerShell, and prints each file's summary line.
# Project: https://github.com/Maty3k/Kairo
# Design: docs/research/17-loadgen-bench/README.md section 6 (the PS row).
#
#   pwsh -NoProfile -NonInteractive -File tools/pc/bench/tests/run-all.ps1
#
# A file fails when its process exits with a code other than 0, when it prints no
# "<n> cases, <m> failed" line, or when that line counts no case or a failed one; a failing file's
# whole output is printed. It exits 1 if a file failed or none was found, else 0. CI runs it in
# Windows CI's powershell job under pwsh 7 and in Linux CI's preinstalled pwsh.
#
# Rules: ASCII only; Windows PowerShell 5.1 syntax (CI parses tools/** there); no Add-Type.

. (Join-Path $PSScriptRoot 'testlib.ps1')

$runAllFiles = @(Get-ChildItem -LiteralPath $PSScriptRoot -File -Filter '*.tests.ps1' | Sort-Object Name)
[Console]::Out.WriteLine('PowerShell ' + $PSVersionTable.PSVersion.ToString() + ', ' + $runAllFiles.Count + ' test files')
$runAllFailed = [Collections.Generic.List[string]]::new()
$runAllCases = 0
foreach ($runAllFile in $runAllFiles) {
    $run = Invoke-BenchTool $runAllFile.FullName @()
    $summary = $null
    foreach ($line in $run.StdOut.Split([char]10)) {
        $match = [regex]::Match($line.TrimEnd([char]13), '^([0-9]+) cases, ([0-9]+) failed\z')
        if ($match.Success) { $summary = $match }
    }
    $ok = ($run.ExitCode -eq 0 -and $null -ne $summary)
    if ($ok) { $ok = ([int]$summary.Groups[1].Value -gt 0 -and [int]$summary.Groups[2].Value -eq 0) }
    if (-not $ok) {
        $runAllFailed.Add($runAllFile.Name)
        [Console]::Out.WriteLine('--- ' + $runAllFile.Name + ' (exit ' + $run.ExitCode + ')')
        [Console]::Out.Write($run.StdOut)
        if ($run.StdErr.Length -gt 0) {
            [Console]::Out.WriteLine('--- its stderr')
            [Console]::Out.Write($run.StdErr)
        }
    }
    $text = 'no summary line'
    if ($null -ne $summary) {
        $runAllCases += [int]$summary.Groups[1].Value
        $text = $summary.Value
    }
    if ($ok) {
        [Console]::Out.WriteLine('ok   ' + $runAllFile.Name + ': ' + $text)
    } else {
        [Console]::Out.WriteLine('FAIL ' + $runAllFile.Name + ': ' + $text + ', exit ' + $run.ExitCode)
    }
}
if ($runAllFiles.Count -eq 0) {
    [Console]::Out.WriteLine('no *.tests.ps1 file found')
    exit 1
}
if ($runAllFailed.Count -gt 0) {
    [Console]::Out.WriteLine('FAILED: ' + ($runAllFailed -join ', '))
    exit 1
}
[Console]::Out.WriteLine('all ' + $runAllFiles.Count + ' test files passed, ' + $runAllCases + ' cases')
exit 0
