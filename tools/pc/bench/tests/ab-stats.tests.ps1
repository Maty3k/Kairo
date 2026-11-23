# SPDX-License-Identifier: GPL-3.0-or-later
#
# tools/pc/bench/tests/ab-stats.tests.ps1 - ab-stats.ps1's vectors (research 17 section 6).
# Project: https://github.com/Maty3k/Kairo
#
# Run: pwsh -NoProfile -File tools/pc/bench/tests/ab-stats.tests.ps1. It exits 1 if a case fails.
# Every vector runs both ways: through the functions, with the script dot-sourced under testlib's
# Set-StrictMode -Version 3.0 and $ErrorActionPreference = 'Stop' (as the elevated session runs
# it), and through pwsh -File with its exit codes. Vectors: AB1-AB5, AE1-AE3, AA1-AA2, L1 and S1-S2,
# and the t table. Full-precision expectations were recomputed with IEEE doubles from research 17's
# formulas and agree with its [Mac] figures. Fixtures are written into temporary folders and use
# synthetic names only.
#
# Rules: ASCII only; Windows PowerShell 5.1 syntax and .NET Framework 4.x APIs; no Add-Type.

. (Join-Path $PSScriptRoot 'testlib.ps1')
. (Join-Path $BenchDir 'ab-stats.ps1')

# --- Helpers this file needs beyond testlib.ps1 ------------------------------------------------

function Join-AbTestNumbers([double[]] $Values) {
    # Bare avg_fps numbers as one command-line value, comma-separated, in the invariant culture.
    $texts = @($Values | ForEach-Object { $_.ToString('R', [Globalization.CultureInfo]::InvariantCulture) })
    return ($texts -join ',')
}

function New-AbTestCounts([string] $Key, [int] $Count) {
    $counts = [ordered]@{}
    $counts[$Key] = $Count
    return $counts
}

function New-AbTestPmStats([string] $Tag, [double] $AvgFps, [double] $Low1Fps) {
    # A run as Get-PmStats returns it in .Stats (research 17 section 5.1), with synthetic values.
    $s = [ordered]@{}
    $s['pm_stats'] = 1
    $s['tag'] = $Tag
    $s['frames'] = 7500
    $s['avg_fps'] = $AvgFps
    $s['low1_fps'] = $Low1Fps
    $s['low01_fps'] = 40.0
    $s['gpu_busy_ms'] = 7.545
    $s['gpu_wait_ms'] = 0.05
    $s['present_modes'] = New-AbTestCounts 'Hardware: Independent Flip' 7500
    $s['frame_types'] = New-AbTestCounts 'Application' 7500
    $s['low_def'] = 'p99_nearest_rank'
    $s['gpu_time_ms'] = 7.595
    $s['gpu_busy_p99_ms'] = 7.59
    $s['gpu_wait_p99_ms'] = 0.1
    $s['gpu_time_p99_ms'] = 7.69
    $s['display_change_ms'] = 8.564
    $s['display_change_p99_ms'] = 9.0
    $s['max_frame_ms'] = 25.0
    $s['long_frames'] = 0
    $s['sync_interval'] = New-AbTestCounts '0' 7500
    $s['allows_tearing'] = New-AbTestCounts '1' 7500
    $s['present_flags'] = New-AbTestCounts '512' 7500
    $s['duration_s'] = 64.23
    $s['dropped_pct'] = 0.0
    $s['swapchains'] = 1
    $s['rows'] = 7500
    $s['bad_rows'] = 0
    return $s
}

function New-AbTestPmLines([string] $Prefix, [double[]] $AvgFps, [double[]] $Low1Fps) {
    # One pm-stats JSON line per run, as pm-stats.ps1 prints them.
    $lines = [Collections.Generic.List[string]]::new()
    for ($i = 0; $i -lt $AvgFps.Length; $i++) {
        $tag = $Prefix + '-' + ($i + 1).ToString('00', [Globalization.CultureInfo]::InvariantCulture)
        $lines.Add((ConvertTo-Json -InputObject (New-AbTestPmStats $tag $AvgFps[$i] $Low1Fps[$i]) -Compress -Depth 5))
    }
    return ,$lines.ToArray()
}

function Write-AbTestFile([string] $Dir, [string] $Name, [string[]] $Lines, [Text.Encoding] $Encoding) {
    # A file of runs: the encoding's byte-order mark (if any), CRLF line ends and a blank last line.
    $path = Join-Path $Dir $Name
    $body = $Encoding.GetBytes((($Lines -join "`r`n") + "`r`n`r`n"))
    $preamble = $Encoding.GetPreamble()
    $bytes = [byte[]]::new($preamble.Length + $body.Length)
    [Array]::Copy($preamble, 0, $bytes, 0, $preamble.Length)
    [Array]::Copy($body, 0, $bytes, $preamble.Length, $body.Length)
    [IO.File]::WriteAllBytes($path, $bytes)
    return $path
}

function Get-AbTestError([scriptblock] $Body) {
    # The exit code the error $Body throws stands for, as the command line would exit.
    $record = Assert-Throws $Body 'an ab-stats error'
    return (Get-AbStatsExitCode $record)
}

function Assert-AbTestRun($Run, [int] $Exit, [string] $StdOut, [string] $What) {
    # A pwsh -File run: its exit code, its one stdout line, and an empty stderr.
    Assert-Equal $Exit $Run.ExitCode ($What + ': exit code (stderr: ' + $Run.StdErr.Trim() + ')')
    Assert-Equal $StdOut $Run.StdOut.Trim() ($What + ': stdout')
    Assert-Equal '' $Run.StdErr.Trim() ($What + ': stderr')
}

function Assert-AbTestRefused($Run, [int] $Exit, [string] $Pattern, [string] $What) {
    # A refused pwsh -File run: its exit code, nothing on stdout, and a reason on stderr.
    Assert-Equal $Exit $Run.ExitCode ($What + ': exit code (stderr: ' + $Run.StdErr.Trim() + ')')
    Assert-Equal '' $Run.StdOut.Trim() ($What + ': stdout')
    Assert-Match ('^ab-stats: ' + $Pattern) $Run.StdErr.Trim() ($What + ': stderr')
}

# A JSON number, and the two -Json forms with their keys in the design's order.
$Num = '-?[0-9]+(?:\.[0-9]+)?(?:[Ee][+-]?[0-9]+)?'
$PairedJsonPattern = '^\{"ab_stats":1,"mode":"ab","n":[0-9]+,"loss_pct":' + $Num + ',"sd_pp":' + $Num + ',"t":' + $Num +
    ',"ci_lo":' + $Num + ',"ci_hi":' + $Num + ',"clauses":\{"mean_le_3":(true|false),"upper_le_4":(true|false),' +
    '"low1_le_5":(true|false|null),"present_mode":null,"ir_plus_1pp":(true|false|null)\},"pass":(true|false)\}$'
$AaJsonPattern = '^\{"ab_stats":1,"mode":"aa","n":[0-9]+,"mean_fps":' + $Num + ',"sd_fps":' + $Num + ',"cv_pct":' + $Num +
    ',"low1_cv_pct":(' + $Num + '|null),"drift_pct_per_run":' + $Num + ',"noise_ok":(true|false)\}$'

# --- The vectors (research 17 section 6) --------------------------------------------------------

$AA1 = @(120.0, 121.2, 119.4, 120.6, 119.8, 121.0)
$AA2 = @(118.0, 120.4, 121.9, 119.1, 122.3, 117.6)
$L1A = @(111.1, 110.8, 111.3, 111.0, 110.9, 111.2)
$L1B = @(106.0, 105.2, 106.4, 105.9, 105.1, 106.2)

$Paired = @(
    [pscustomobject]@{ Id = 'AB1'; Exit = 0; MeanOk = $true; UpperOk = $true; T = 2.5706
        A = @(125.40, 124.90, 125.10, 125.60, 124.80, 125.20); B = @(122.10, 121.95, 122.40, 122.30, 121.70, 122.25)
        Text = '6 pairs: loss 2.44% (95% CI 2.25..2.63%) PASS=True'
        Loss = 2.436555794371293; Sd = 0.18230088792970991; Lo = 2.2452413935280995; Hi = 2.6278701952144865 },
    [pscustomobject]@{ Id = 'AB2'; Exit = 0; MeanOk = $true; UpperOk = $true; T = 2.2281
        A = @(100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0)
        B = @(97.0, 97.5, 96.8, 97.2, 97.1, 96.9, 97.3, 97.0, 96.7, 97.4, 97.1)
        Text = '11 pairs: loss 2.91% (95% CI 2.74..3.07%) PASS=True'
        Loss = 2.909090909090909; Sd = 0.24679767200905867; Lo = 2.7432928680817366; Hi = 3.074888950100082 },
    [pscustomobject]@{ Id = 'AB3'; Exit = 0; MeanOk = $true; UpperOk = $true; T = 2.5706
        A = @(100.0, 100.0, 100.0, 100.0, 100.0, 100.0); B = @(98.0, 98.0, 98.0, 98.0, 98.0, 98.0)
        Text = '6 pairs: loss 2.00% (95% CI 2.00..2.00%) PASS=True'
        Loss = 2.0; Sd = 0.0; Lo = 2.0; Hi = 2.0 },
    [pscustomobject]@{ Id = 'AB4'; Exit = 1; MeanOk = $true; UpperOk = $false; T = 2.5706
        A = @(100.0, 100.0, 100.0, 100.0, 100.0, 100.0); B = @(96.0, 99.0, 97.5, 95.5, 98.5, 97.0)
        Text = '6 pairs: loss 2.75% (95% CI 1.31..4.19%) PASS=False'
        Loss = 2.75; Sd = 1.3693063937629153; Lo = 1.31299091425976; Hi = 4.1870090857402396 },
    [pscustomobject]@{ Id = 'AB5'; Exit = 0; MeanOk = $true; UpperOk = $true; T = 4.3027
        A = @(100.0, 100.0, 100.0); B = @(101.0, 100.5, 101.5)
        Text = '3 pairs: loss -1.00% (95% CI -2.24..0.24%) PASS=True'
        Loss = -1.0; Sd = 0.5; Lo = -2.2420825016211015; Hi = 0.24208250162110145 }
)
$AB1 = $Paired[0]
$S1Text = '3 pairs: loss -1.00% (95% CI -1.01..-1.00%) PASS=True'
$S2Text = '3 pairs: loss 1.37% (95% CI -6.20..8.94%) PASS=False'
$AA1Text = '6 runs: avg_fps 120.33 sd 0.71 (CV 0.59%) NOISE=ok'
$AA2Text = '6 runs: avg_fps 119.88 sd 1.98 (CV 1.65%) NOISE=high'
$Dir = New-BenchTempDir

# --- The t table -------------------------------------------------------------------------------

$DesignT = @(12.7062, 4.3027, 3.1824, 2.7764, 2.5706, 2.4469, 2.3646, 2.3060, 2.2622, 2.2281,
    2.2010, 2.1788, 2.1604, 2.1448, 2.1314, 2.1199, 2.1098, 2.1009, 2.0930, 2.0860,
    2.0796, 2.0739, 2.0687, 2.0639, 2.0595, 2.0555, 2.0518, 2.0484, 2.0452, 2.0423)

Test-Case 't table: df 1-30 are the published values' {
    for ($df = 1; $df -le 30; $df++) { Assert-Equal $DesignT[$df - 1] (Get-AbStatsT975 $df) ('t(0.975, ' + $df + ')') }
}

Test-Case 't table: beyond 30 the Cornish-Fisher series is within the design''s error of the exact quantile' {
    # Exact t(0.975, df) by bisection on the regularised incomplete beta, in IEEE doubles. The
    # design gives the series' error as 3.1e-8 at df 30, 7.3e-9 at 40 and 3.0e-11 at 120.
    $exact = @(
        @(31, 2.0395134463964064, 3.1e-8), @(35, 2.030107928250346, 3.1e-8), @(40, 2.0210753903062697, 7.4e-9),
        @(60, 2.0002978220142573, 7.4e-9), @(120, 1.979930405082441, 3.1e-11), @(1000, 1.9623390808264358, 3.1e-11))
    foreach ($row in $exact) { Assert-Near $row[1] (Get-AbStatsT975 $row[0]) $row[2] ('t(0.975, ' + $row[0] + ')') }
}

Test-Case 't table: falls with df, stays above z, and refuses df 0' {
    $previous = Get-AbStatsT975 1
    for ($df = 2; $df -le 400; $df++) {
        $t = Get-AbStatsT975 $df
        Assert-True ($t -lt $previous) ('t(0.975, ' + $df + ') < t(0.975, ' + ($df - 1) + ')')
        Assert-True ($t -gt 1.959963984540054) ('t(0.975, ' + $df + ') > z')
        $previous = $t
    }
    Assert-Equal 2 (Get-AbTestError { Get-AbStatsT975 0 }) 'df 0'
}

Test-Case 't table: n outside research 13''s table keeps its interval (13 and 40 pairs)' {
    foreach ($n in @(13, 40)) {
        $a = @(1..$n | ForEach-Object { 100.0 })
        $b = @(1..$n | ForEach-Object { 97.0 + 0.1 * ($_ % 7) })
        $r = Get-AbStats -A $a -B $b
        Assert-Equal (Get-AbStatsT975 ($n - 1)) $r.Stats['t'] ('t at n ' + $n)
        Assert-True ($r.Stats['ci_hi'] - $r.Stats['ci_lo'] -gt 0.01) ('an interval at n ' + $n)
    }
    Assert-Equal 2.1788 (Get-AbStatsT975 12) 't(0.975, 12)'
}

Test-Case 't table: through pwsh -File, 40 pairs use the series' {
    $a = @(1..40 | ForEach-Object { 100.0 })
    $b = @(1..40 | ForEach-Object { 97.0 + 0.1 * ($_ % 7) })
    $run = Invoke-BenchTool 'ab-stats.ps1' @('-A', (Join-AbTestNumbers $a), '-B', (Join-AbTestNumbers $b), '-Json')
    Assert-Equal 0 $run.ExitCode 'exit code'
    $json = ConvertFrom-Json $run.StdOut.Trim()
    Assert-Equal 40 $json.n 'n'
    Assert-Near (Get-AbStatsT975 39) $json.t 1e-15 't'
    Assert-Near 2.0226909200367587 $json.t 3.1e-8 't(0.975, 39), exact'
    Assert-True ($json.ci_hi - $json.ci_lo -gt 0.01) 'an interval'
}

# --- AB1-AB5: the paired loss --------------------------------------------------------------------

foreach ($v in $Paired) {
    Test-Case ($v.Id + ' through the functions') {
        $r = Get-AbStats -A $v.A -B $v.B
        Assert-Equal $v.Text $r.Text 'text'
        Assert-Equal $v.Exit $r.ExitCode 'exit code'
        Assert-Equal 1 $r.Stats['ab_stats'] 'ab_stats'
        Assert-Equal 'ab' $r.Stats['mode'] 'mode'
        Assert-Equal $v.A.Count $r.Stats['n'] 'n'
        Assert-Near $v.Loss $r.Stats['loss_pct'] 1e-9 'loss_pct'
        Assert-Near $v.Sd $r.Stats['sd_pp'] 1e-9 'sd_pp'
        Assert-Equal $v.T $r.Stats['t'] 't'
        Assert-Near $v.Lo $r.Stats['ci_lo'] 1e-9 'ci_lo'
        Assert-Near $v.Hi $r.Stats['ci_hi'] 1e-9 'ci_hi'
        Assert-Equal $v.MeanOk $r.Stats['clauses']['mean_le_3'] 'mean_le_3'
        Assert-Equal $v.UpperOk $r.Stats['clauses']['upper_le_4'] 'upper_le_4'
        Assert-Equal ($v.Exit -eq 0) $r.Stats['pass'] 'pass'
        # Bare numbers evaluate only the mean and upper-bound clauses.
        Assert-Equal $null $r.Stats['clauses']['low1_le_5'] 'low1_le_5'
        Assert-Equal $null $r.Stats['clauses']['present_mode'] 'present_mode'
        Assert-Equal $null $r.Stats['clauses']['ir_plus_1pp'] 'ir_plus_1pp'
        Assert-Equal $null $r.Low1LossPct 'Low1LossPct'
        Assert-Match $PairedJsonPattern $r.Json 'JSON form'
    }
    Test-Case ($v.Id + ' through pwsh -File') {
        $run = Invoke-BenchTool 'ab-stats.ps1' @('-A', (Join-AbTestNumbers $v.A), '-B', (Join-AbTestNumbers $v.B))
        Assert-AbTestRun $run $v.Exit $v.Text $v.Id
    }
}

Test-Case 'AB1: s, t and h as the design states them (0.1823, 2.5706, 0.1913)' {
    $r = Get-AbStats -A $AB1.A -B $AB1.B
    Assert-Near 0.1823 $r.Stats['sd_pp'] 0.00005 's'
    Assert-Equal 2.5706 $r.Stats['t'] 't'
    Assert-Near 0.1913 (($r.Stats['ci_hi'] - $r.Stats['ci_lo']) / 2) 0.00005 'h'
}

Test-Case 'AB1 -Json through pwsh -File: the function''s line, keys in the design''s order' {
    $r = Get-AbStats -A $AB1.A -B $AB1.B
    $run = Invoke-BenchTool 'ab-stats.ps1' @('-A', (Join-AbTestNumbers $AB1.A), '-B', (Join-AbTestNumbers $AB1.B), '-Json')
    Assert-AbTestRun $run 0 $r.Json 'AB1 -Json'
    Assert-Match $PairedJsonPattern $run.StdOut.Trim() 'JSON form'
    $json = ConvertFrom-Json $run.StdOut.Trim()
    Assert-Near $AB1.Loss $json.loss_pct 1e-12 'loss_pct'
    Assert-Equal $true $json.pass 'pass'
    Assert-Equal $null $json.clauses.low1_le_5 'low1_le_5'
}

Test-Case 'AB1 through pwsh -File: runs as separate values, and separated by spaces' {
    $words = @($AB1.A | ForEach-Object { $_.ToString('R', [Globalization.CultureInfo]::InvariantCulture) })
    $run = Invoke-BenchTool 'ab-stats.ps1' (@('-A') + $words + @('-B', ((Join-AbTestNumbers $AB1.B) -replace ',', ' ')))
    Assert-AbTestRun $run 0 $AB1.Text 'AB1 as words'
}

# --- AE1-AE3: refusals ---------------------------------------------------------------------------

$SixA = @(100.0, 100.0, 100.0, 100.0, 100.0, 100.0)
$FiveB = @(98.0, 98.0, 98.0, 98.0, 98.0)

Test-Case 'AE1 6 A runs vs 5 B: exit 2, through the functions' {
    Assert-Equal 2 (Get-AbTestError { Get-AbStats -A $SixA -B $FiveB }) 'exit code'
}
Test-Case 'AE1 6 A runs vs 5 B: exit 2, through pwsh -File' {
    $run = Invoke-BenchTool 'ab-stats.ps1' @('-A', (Join-AbTestNumbers $SixA), '-B', (Join-AbTestNumbers $FiveB))
    Assert-AbTestRefused $run 2 'unequal counts' 'AE1'
}

Test-Case 'AE2 n = 1: exit 2, paired and A/A, through the functions' {
    Assert-Equal 2 (Get-AbTestError { Get-AbStats -A @(100.0) -B @(98.0) }) 'one pair'
    Assert-Equal 2 (Get-AbTestError { Get-AbStats -A @(120.0) }) 'one A/A run'
    Assert-Equal 2 (Get-AbTestError { Get-AbStats -A @() }) 'no run'
}
Test-Case 'AE2 n = 1: exit 2, paired and A/A, through pwsh -File' {
    Assert-AbTestRefused (Invoke-BenchTool 'ab-stats.ps1' @('-A', '100', '-B', '98')) 2 'at least 2 pairs' 'one pair'
    Assert-AbTestRefused (Invoke-BenchTool 'ab-stats.ps1' @('-A', '120')) 2 'at least 2 runs' 'one A/A run'
}

Test-Case 'AE3 an A run of 0: exit 3, paired and A/A, through the functions' {
    $zero = @(100.0, 100.0, 0.0, 100.0, 100.0, 100.0)
    Assert-Equal 3 (Get-AbTestError { Get-AbStats -A $zero -B @(98.0, 98.0, 98.0, 98.0, 98.0, 98.0) }) 'paired'
    Assert-Equal 3 (Get-AbTestError { Get-AbStats -A $zero }) 'A/A'
    Assert-Equal 3 (Get-AbTestError { Get-AbStats -A @(100.0, -100.0) -B @(98.0, 98.0) }) 'a negative A run'
}
Test-Case 'AE3 an A run of 0: exit 3, paired and A/A, through pwsh -File' {
    $zero = '100,100,0,100,100,100'
    Assert-AbTestRefused (Invoke-BenchTool 'ab-stats.ps1' @('-A', $zero, '-B', '98,98,98,98,98,98')) 3 'an A run with an avg_fps of 0' 'paired'
    Assert-AbTestRefused (Invoke-BenchTool 'ab-stats.ps1' @('-A', $zero)) 3 'an A run with an avg_fps of 0' 'A/A'
}

# --- AA1-AA2: the A/A noise floor ----------------------------------------------------------------

Test-Case 'AA1 through the functions' {
    $r = Get-AbStats -A $AA1
    Assert-Equal $AA1Text $r.Text 'text'
    Assert-Equal 0 $r.ExitCode 'exit code'
    Assert-Equal 'aa' $r.Stats['mode'] 'mode'
    Assert-Equal 6 $r.Stats['n'] 'n'
    Assert-Near 120.33333333333333 $r.Stats['mean_fps'] 1e-9 'mean_fps'
    Assert-Near 0.7118052168020866 $r.Stats['sd_fps'] 1e-12 'sd_fps'
    Assert-Near 0.591527880998964 $r.Stats['cv_pct'] 1e-12 'cv_pct'
    Assert-Equal $null $r.Stats['low1_cv_pct'] 'low1_cv_pct (bare numbers)'
    Assert-Near 0.047487138899880606 $r.Stats['drift_pct_per_run'] 1e-12 'drift_pct_per_run'
    Assert-Equal 0.0475 ([Math]::Round($r.Stats['drift_pct_per_run'], 4)) 'drift +0.0475% per run'
    Assert-Equal $true $r.Stats['noise_ok'] 'noise_ok'
    Assert-Match $AaJsonPattern $r.Json 'JSON form'
}

Test-Case 'AA1: the CV uses the sample SD (0.59%), not the population SD (0.54%)' {
    $r = Get-AbStats -A $AA1
    $mean = 0.0
    foreach ($x in $AA1) { $mean += $x / $AA1.Count }
    $squares = 0.0
    foreach ($x in $AA1) { $squares += ($x - $mean) * ($x - $mean) }
    $populationCv = 100.0 * [Math]::Sqrt($squares / $AA1.Count) / $mean
    Assert-Equal 0.54 ([Math]::Round($populationCv, 2)) 'population CV'
    Assert-Equal 0.59 ([Math]::Round($r.Stats['cv_pct'], 2)) 'reported CV'
    # Measure-Object -StandardDeviation (PowerShell 7) is the design's reference: 1.58113883008419 for 1..5.
    $meanSd = Get-AbStatsMeanSd @(1.0, 2.0, 3.0, 4.0, 5.0)
    Assert-Near 1.58113883008419 $meanSd[1] 1e-14 'sample SD of 1..5'
    if ((Get-Command Measure-Object).Parameters.ContainsKey('StandardDeviation')) {
        Assert-Near (($AA1 | Measure-Object -StandardDeviation).StandardDeviation) $r.Stats['sd_fps'] 1e-12 'Measure-Object'
    }
}

Test-Case 'AA1 through pwsh -File, text and -Json' {
    Assert-AbTestRun (Invoke-BenchTool 'ab-stats.ps1' @('-A', (Join-AbTestNumbers $AA1))) 0 $AA1Text 'AA1'
    $r = Get-AbStats -A $AA1
    $run = Invoke-BenchTool 'ab-stats.ps1' @('-A', (Join-AbTestNumbers $AA1), '-Json')
    Assert-AbTestRun $run 0 $r.Json 'AA1 -Json'
    Assert-Match $AaJsonPattern $run.StdOut.Trim() 'JSON form'
}

Test-Case 'AA2 through the functions: noise high' {
    $r = Get-AbStats -A $AA2
    Assert-Equal $AA2Text $r.Text 'text'
    Assert-Equal 1 $r.ExitCode 'exit code'
    Assert-Near 1.6493432820040126 $r.Stats['cv_pct'] 1e-12 'cv_pct'
    Assert-Near 0.021449425036244 $r.Stats['drift_pct_per_run'] 1e-12 'drift_pct_per_run'
    Assert-Equal $false $r.Stats['noise_ok'] 'noise_ok'
}

Test-Case 'AA2 through pwsh -File: exit 1' {
    Assert-AbTestRun (Invoke-BenchTool 'ab-stats.ps1' @('-A', (Join-AbTestNumbers $AA2))) 1 $AA2Text 'AA2'
}

Test-Case 'A/A on pm-stats lines adds the CV of low1_fps' {
    $r = Get-AbStats -A (New-AbTestPmLines 'aa-alu' $AA1 $L1A)
    Assert-Equal $AA1Text $r.Text 'text'
    Assert-Near 0.16846723938648925 $r.Stats['low1_cv_pct'] 1e-12 'low1_cv_pct'
    Assert-Match $AaJsonPattern $r.Json 'JSON form'
}

# --- L1: the 1%-low clause (reported, not part of M1's PASS) ------------------------------------

$L1LinesA = New-AbTestPmLines 'ab-alu-a' $AB1.A $L1A
$L1LinesB = New-AbTestPmLines 'ab-alu-b' $AB1.B $L1B

Test-Case 'L1 through the functions: mean 1%-low loss 4.73% -> low1_le_5 true' {
    $r = Get-AbStats -A $L1LinesA -B $L1LinesB
    Assert-Equal $AB1.Text $r.Text 'text (the avg_fps are AB1''s)'
    Assert-Near 4.728010124348055 $r.Low1LossPct 1e-9 'mean 1%-low loss'
    Assert-Equal 4.73 ([Math]::Round($r.Low1LossPct, 2)) 'mean 1%-low loss, 2 decimals'
    Assert-Equal $true $r.Stats['clauses']['low1_le_5'] 'low1_le_5'
    Assert-Equal $null $r.Stats['clauses']['present_mode'] 'present_mode'
    Assert-Equal $true $r.Stats['pass'] 'pass'
    Assert-Equal 0 $r.ExitCode 'exit code'
}

Test-Case 'L1 through the functions: Get-PmStats-style dictionaries give the same' {
    $a = @(for ($i = 0; $i -lt 6; $i++) { New-AbTestPmStats ('ab-alu-a-' + $i) $AB1.A[$i] $L1A[$i] })
    $b = @(for ($i = 0; $i -lt 6; $i++) { New-AbTestPmStats ('ab-alu-b-' + $i) $AB1.B[$i] $L1B[$i] })
    $r = Get-AbStats -A $a -B $b
    Assert-Equal $AB1.Text $r.Text 'text'
    Assert-Equal $true $r.Stats['clauses']['low1_le_5'] 'low1_le_5'
}

Test-Case 'L1 variant: a 1%-low loss of 6.20% sets low1_le_5 false and leaves PASS alone' {
    $b = New-AbTestPmLines 'ab-alu-b' $AB1.B @(104.0, 104.2, 104.4, 104.1, 104.0, 104.3)
    $r = Get-AbStats -A $L1LinesA -B $b
    Assert-Near 6.1983087214873605 $r.Low1LossPct 1e-9 'mean 1%-low loss'
    Assert-Equal $false $r.Stats['clauses']['low1_le_5'] 'low1_le_5'
    Assert-Equal $true $r.Stats['pass'] 'pass'
    Assert-Equal 0 $r.ExitCode 'exit code'
}

Test-Case 'L1 through pwsh -File: files of pm-stats lines (UTF-8 BOM, CRLF), -Json' {
    $fileA = Write-AbTestFile $Dir 'l1-a.jsonl' $L1LinesA ([Text.UTF8Encoding]::new($true))
    $fileB = Write-AbTestFile $Dir 'l1-b.jsonl' $L1LinesB ([Text.UTF8Encoding]::new($true))
    $r = Get-AbStats -A $L1LinesA -B $L1LinesB
    $run = Invoke-BenchTool 'ab-stats.ps1' @('-A', $fileA, '-B', $fileB, '-Json')
    Assert-AbTestRun $run 0 $r.Json 'L1 files'
    $json = ConvertFrom-Json $run.StdOut.Trim()
    Assert-Equal $true $json.clauses.low1_le_5 'low1_le_5'
    Assert-Equal 6 $json.n 'n'
}

Test-Case 'L1 through pwsh -File: UTF-16LE files, and lines given as values' {
    $fileA = Write-AbTestFile $Dir 'l1-a-utf16.jsonl' $L1LinesA ([Text.UnicodeEncoding]::new($false, $true))
    $fileB = Write-AbTestFile $Dir 'l1-b-utf16.jsonl' $L1LinesB ([Text.UnicodeEncoding]::new($false, $true))
    Assert-AbTestRun (Invoke-BenchTool 'ab-stats.ps1' @('-A', $fileA, '-B', $fileB)) 0 $AB1.Text 'UTF-16LE files'
    $values = @('-A') + $L1LinesA + @('-B') + $L1LinesB
    Assert-AbTestRun (Invoke-BenchTool 'ab-stats.ps1' $values) 0 $AB1.Text 'lines as values'
}

Test-Case 'F2 output: AwayFromZero on exact ties, -0 as 0, no group separator' {
    # No vector sits on a tie, so these pin the rounding: 0.125 is exact in binary. 2.675 is 2.67499...
    # and 1.005 is 1.00499..., but Math.Round scales by 100 first (267.5 and 100.49999...), so both
    # PowerShells print 2.68 and 1.00, where F2 alone prints 2.68 and 1.01 on 5.1 but 2.67 and 1.00
    # on 7. -0.001 must not print -0.00.
    Assert-Equal '0.13' (Format-AbStatsNumber 0.125) '0.125'
    Assert-Equal '-0.13' (Format-AbStatsNumber -0.125) '-0.125'
    Assert-Equal '2.68' (Format-AbStatsNumber 2.675) '2.675'
    Assert-Equal '1.00' (Format-AbStatsNumber 1.005) '1.005'
    Assert-Equal '0.00' (Format-AbStatsNumber -0.001) '-0.001'
    Assert-Equal '1234.50' (Format-AbStatsNumber 1234.5) '1234.5'
}

Test-Case 'ConvertFrom-AbStatsText: one run per line, in order; blank lines and a leftover BOM skipped' {
    $text = [string][char]0xFEFF + "120.0`r`n`r`n121.2`n  119.4  `r`n"
    $runs = ConvertFrom-AbStatsText $text
    Assert-Equal 3 $runs.Length 'runs'
    Assert-Equal '120.0' $runs[0] 'first'
    Assert-Equal '119.4' $runs[2] 'last'
    Assert-Equal 0 (ConvertFrom-AbStatsText '').Length 'empty text'
}

# --- One kind per call, unparsable lines ---------------------------------------------------------

Test-Case 'mixed kinds: exit 2, through the functions and pwsh -File' {
    Assert-Equal 2 (Get-AbTestError { Get-AbStats -A $L1LinesA -B $AB1.B }) 'lines and numbers'
    Assert-Equal 2 (Get-AbTestError { Get-AbStats -A @($L1LinesA[0], 120.5, 121.0) }) 'within -A'
    $fileA = Write-AbTestFile $Dir 'mixed-a.jsonl' $L1LinesA ([Text.UTF8Encoding]::new($false))
    $run = Invoke-BenchTool 'ab-stats.ps1' @('-A', $fileA, '-B', (Join-AbTestNumbers $AB1.B))
    Assert-AbTestRefused $run 2 'mixed kinds' 'mixed kinds'
}

Test-Case 'an unparsable or foreign line: exit 3, and the line is never echoed' {
    $cut = '{"pm_stats":1,"tag":"marker-7f3a","avg_fps":12'
    Assert-Equal 3 (Get-AbTestError { Get-AbStats -A @($cut, $L1LinesA[1]) }) 'a cut line'
    Assert-Equal 3 (Get-AbTestError { Get-AbStats -A @('{"ab_stats":1,"mode":"aa"}', $L1LinesA[1]) }) 'not a pm-stats line'
    Assert-Equal 3 (Get-AbTestError { Get-AbStats -A @('{"pm_stats":1,"tag":"x"}', $L1LinesA[1]) }) 'no avg_fps'
    Assert-Equal 3 (Get-AbTestError { Get-AbStats -A @('{"pm_stats":1,"avg_fps":"120"}', $L1LinesA[1]) }) 'a text avg_fps'
    Assert-Equal 3 (Get-AbTestError { Get-AbStats -A @('120.5', 'abc') }) 'not a number'
    Assert-Equal 3 (Get-AbTestError { Get-AbStats -A @('120.5', 'NaN') }) 'NaN'
    Assert-Equal 3 (Get-AbTestError { Get-AbStats -A @('120.5', 'Infinity') }) 'Infinity'
    $file = Write-AbTestFile $Dir 'cut.jsonl' @($L1LinesA[0], $cut) ([Text.UTF8Encoding]::new($true))
    $run = Invoke-BenchTool 'ab-stats.ps1' @('-A', $file)
    Assert-AbTestRefused $run 3 'an unparsable pm-stats line' 'a cut line in a file'
    Assert-True (-not $run.StdErr.Contains('marker-7f3a')) 'stderr never echoes the line'
}

# --- S1-S2: the self-test pairs A/A runs (1,2), (3,4), (5,6) ------------------------------------

Test-Case 'S1 through the functions' {
    $r = Get-AbStatsSelfTest -Runs $AA1
    Assert-Equal $S1Text $r.Text 'text'
    Assert-Equal 0 $r.ExitCode 'exit code'
    Assert-Equal 3 $r.Stats['n'] 'n'
    Assert-Equal 4.3027 $r.Stats['t'] 't'
    Assert-Near -1.002231524903313 $r.Stats['loss_pct'] 1e-9 'loss_pct'
    Assert-Near -1.0085892009928967 $r.Stats['ci_lo'] 1e-9 'ci_lo'
    Assert-Near -0.9958738488137291 $r.Stats['ci_hi'] 1e-9 'ci_hi'
    Assert-Equal 0 $r.Warnings.Count 'warnings'
    $split = Get-AbStats -A @($AA1[0], $AA1[2], $AA1[4]) -B @($AA1[1], $AA1[3], $AA1[5])
    Assert-Equal $r.Json $split.Json 'the same as -A (1,3,5) -B (2,4,6)'
}

Test-Case 'S2 through the functions' {
    $r = Get-AbStatsSelfTest -Runs $AA2
    Assert-Equal $S2Text $r.Text 'text'
    Assert-Equal 1 $r.ExitCode 'exit code'
    Assert-Near 1.3686918047920695 $r.Stats['loss_pct'] 1e-9 'loss_pct'
    Assert-Near -6.199160137655524 $r.Stats['ci_lo'] 1e-9 'ci_lo'
    Assert-Near 8.936543747239664 $r.Stats['ci_hi'] 1e-9 'ci_hi'
    Assert-Equal $true $r.Stats['clauses']['mean_le_3'] 'mean_le_3'
    Assert-Equal $false $r.Stats['clauses']['upper_le_4'] 'upper_le_4'
}

Test-Case 'S1 and S2 through pwsh -File, with -SelfTest and as explicit pairs' {
    Assert-AbTestRun (Invoke-BenchTool 'ab-stats.ps1' @('-A', (Join-AbTestNumbers $AA1), '-SelfTest')) 0 $S1Text 'S1 -SelfTest'
    Assert-AbTestRun (Invoke-BenchTool 'ab-stats.ps1' @('-A', (Join-AbTestNumbers $AA2), '-SelfTest')) 1 $S2Text 'S2 -SelfTest'
    $run = Invoke-BenchTool 'ab-stats.ps1' @('-A', (Join-AbTestNumbers @($AA1[0], $AA1[2], $AA1[4])), '-B', (Join-AbTestNumbers @($AA1[1], $AA1[3], $AA1[5])))
    Assert-AbTestRun $run 0 $S1Text 'S1 as -A and -B'
}

Test-Case 'self-test: an odd last run is left out with a warning' {
    $seven = $AA1 + @(120.5)
    $r = Get-AbStatsSelfTest -Runs $seven
    Assert-Equal $S1Text $r.Text 'text'
    Assert-Equal 1 $r.Warnings.Count 'warnings'
    $run = Invoke-BenchTool 'ab-stats.ps1' @('-A', (Join-AbTestNumbers $seven), '-SelfTest')
    Assert-Equal 0 $run.ExitCode 'exit code'
    Assert-Equal $S1Text $run.StdOut.Trim() 'stdout'
    Assert-Match '^ab-stats: warning: run 7 left out' $run.StdErr.Trim() 'stderr'
}

Test-Case 'self-test refusals: fewer than 4 runs, a run of 0 on either side, -B' {
    Assert-Equal 2 (Get-AbTestError { Get-AbStatsSelfTest -Runs @(120.0, 121.2, 119.4) }) 'three runs'
    Assert-Equal 3 (Get-AbTestError { Get-AbStatsSelfTest -Runs @(120.0, 0.0, 119.4, 120.6) }) 'a 0 in run 2'
    Assert-Equal 3 (Get-AbTestError { Get-AbStatsSelfTest -Runs @(120.0, 121.2, 119.4, 120.6, 0.0) }) 'a 0 in the odd run'
    Assert-AbTestRefused (Invoke-BenchTool 'ab-stats.ps1' @('-A', '120,121.2,119.4', '-SelfTest')) 2 'the self-test needs at least 4 runs' 'three runs'
    Assert-AbTestRefused (Invoke-BenchTool 'ab-stats.ps1' @('-A', '120,0,119.4,120.6', '-SelfTest')) 3 'a run with an avg_fps of 0' 'a 0 in run 2'
    Assert-AbTestRefused (Invoke-BenchTool 'ab-stats.ps1' @('-A', '120,121', '-B', '119,118', '-SelfTest')) 2 '-SelfTest pairs' '-SelfTest with -B'
}

# --- -Reference: Instant Replay alone + 1 pp (reported, not part of M1's PASS) -------------------

Test-Case '-Reference through the functions: ir_plus_1pp, and PASS unchanged' {
    $ab3 = Get-AbStats -A $Paired[2].A -B $Paired[2].B
    $ab5 = Get-AbStats -A $Paired[4].A -B $Paired[4].B
    $near = Get-AbStats -A $AB1.A -B $AB1.B -Reference $ab3.Json
    Assert-Equal $true $near.Stats['clauses']['ir_plus_1pp'] '2.44 - 2.00 <= 1'
    $far = Get-AbStats -A $AB1.A -B $AB1.B -Reference $ab5.Json
    Assert-Equal $false $far.Stats['clauses']['ir_plus_1pp'] '2.44 - (-1.00) > 1'
    Assert-Equal $true $far.Stats['pass'] 'pass'
    Assert-Equal 0 $far.ExitCode 'exit code'
    Assert-Equal $AB1.Text $far.Text 'text'
    Assert-Equal 2 (Get-AbTestError { Get-AbStats -A $AB1.A -Reference $ab3.Json }) 'without -B'
    $aa = Get-AbStats -A $AA1
    Assert-Equal 3 (Get-AbTestError { Get-AbStats -A $AB1.A -B $AB1.B -Reference $aa.Json }) 'an A/A line'
    Assert-Equal 3 (Get-AbTestError { Get-AbStats -A $AB1.A -B $AB1.B -Reference '2.0' }) 'not a line'
}

Test-Case '-Reference through pwsh -File' {
    $ab3 = Get-AbStats -A $Paired[2].A -B $Paired[2].B
    $run = Invoke-BenchTool 'ab-stats.ps1' @('-A', (Join-AbTestNumbers $AB1.A), '-B', (Join-AbTestNumbers $AB1.B), '-Reference', $ab3.Json, '-Json')
    Assert-Equal 0 $run.ExitCode ('exit code (stderr: ' + $run.StdErr.Trim() + ')')
    Assert-Equal $true (ConvertFrom-Json $run.StdOut.Trim()).clauses.ir_plus_1pp 'ir_plus_1pp'
    Assert-AbTestRefused (Invoke-BenchTool 'ab-stats.ps1' @('-A', '120,121', '-Reference', $ab3.Json)) 2 '-Reference needs -B' 'without -B'
    Assert-AbTestRefused (Invoke-BenchTool 'ab-stats.ps1' @('-A', '120,121', '-B', '119,118', '-Reference', 'x', 'y')) 2 '-Reference takes one' 'two values'
}

# --- The command line ----------------------------------------------------------------------------

Test-Case 'command line: bad arguments exit 2' {
    Assert-AbTestRefused (Invoke-BenchTool 'ab-stats.ps1' @()) 2 'missing -A' 'no arguments'
    Assert-AbTestRefused (Invoke-BenchTool 'ab-stats.ps1' @('-A', '120,121', '-Bogus')) 2 'unknown parameter -Bogus' 'unknown parameter'
    Assert-AbTestRefused (Invoke-BenchTool 'ab-stats.ps1' @('-A', '120,121', '-A', '119,118')) 2 '-A given twice' '-A twice'
    Assert-AbTestRefused (Invoke-BenchTool 'ab-stats.ps1' @('120,121')) 2 'unexpected argument at position 1' 'a value without a name'
    Assert-AbTestRefused (Invoke-BenchTool 'ab-stats.ps1' @('-A')) 2 '-A needs a value' '-A without a value'
}

Test-Case 'command line: names ignore case; under pwsh, -Json:$false prints the text line' {
    Assert-AbTestRun (Invoke-BenchTool 'ab-stats.ps1' @('-a', (Join-AbTestNumbers $AA1), '-JSON')) 0 (Get-AbStats -A $AA1).Json '-a and -JSON'
    # pwsh -File passes -Json:$false as -Json and a bool; Windows PowerShell 5.1's -File doesn't.
    if ($PSVersionTable.PSEdition -eq 'Core') {
        Assert-AbTestRun (Invoke-BenchTool 'ab-stats.ps1' @('-A', (Join-AbTestNumbers $AA1), '-Json:$false')) 0 $AA1Text '-Json:$false'
    }
}

Test-Case 'command line: a missing or empty file exits 3 and names the file, never its folder' {
    $folder = Split-Path -Leaf $Dir
    $run = Invoke-BenchTool 'ab-stats.ps1' @('-A', (Join-Path $Dir 'missing-runs.jsonl'))
    Assert-AbTestRefused $run 3 'neither numbers, a pm-stats line nor a file: missing-runs\.jsonl$' 'missing file'
    Assert-True (-not $run.StdErr.Contains($folder)) 'stderr names no folder'
    $empty = Write-AbTestFile $Dir 'empty-runs.jsonl' @() ([Text.UTF8Encoding]::new($true))
    $run = Invoke-BenchTool 'ab-stats.ps1' @('-A', $empty)
    Assert-AbTestRefused $run 3 'no runs in empty-runs\.jsonl$' 'empty file'
    Assert-True (-not $run.StdErr.Contains($folder)) 'stderr names no folder'
}

# --- Invariance, dot-sourcing and the file itself ------------------------------------------------

Test-Case 'the text and the JSON are invariant under a German culture' {
    $culture = $null
    try { $culture = [Globalization.CultureInfo]::GetCultureInfo('de-DE') } catch { $culture = $null }
    if ($null -eq $culture) {
        [Console]::Out.WriteLine('note: de-DE is not available here; the culture case checks nothing')
        return
    }
    $saved = [Threading.Thread]::CurrentThread.CurrentCulture
    $inEffect = $false
    try {
        [Threading.Thread]::CurrentThread.CurrentCulture = $culture
        # Windows PowerShell 5.1 may put the culture back between statements; pwsh keeps it.
        $inEffect = ((2.5).ToString() -eq '2,5')
        $r = Get-AbStats -A $AB1.A -B $AB1.B
        $aa = Get-AbStats -A (New-AbTestPmLines 'aa-alu' $AA1 $L1A)
        $parsed = Get-AbStats -A @('120.0', '121.2', '119.4', '120.6', '119.8', '121.0')
        $inEffect = ($inEffect -and (2.5).ToString() -eq '2,5')
    } finally {
        [Threading.Thread]::CurrentThread.CurrentCulture = $saved
    }
    if ($PSVersionTable.PSEdition -eq 'Core') { Assert-True $inEffect 'the German culture was in effect' }
    if (-not $inEffect) { [Console]::Out.WriteLine('note: the German culture did not stay in effect here') }
    Assert-Equal $AB1.Text $r.Text 'paired text'
    Assert-Match '"loss_pct":2\.4365' $r.Json 'paired JSON'
    Assert-Equal $AA1Text $aa.Text 'A/A text'
    Assert-Match '"cv_pct":0\.5915' $aa.Json 'A/A JSON'
    Assert-Equal $AA1Text $parsed.Text 'numbers given as text'
}

Test-Case 'dot-sourcing defines functions only: no output, no variables, settings untouched' {
    $path = Join-Path $BenchDir 'ab-stats.ps1'
    $result = & {
        Set-StrictMode -Off
        $ErrorActionPreference = 'Continue'
        $emitted = @(. $path)
        $leaked = @(Get-Variable -Name 'abStats*' -Scope 0 -ErrorAction SilentlyContinue)
        # Strict mode is still off: an unset variable reads as $null.
        $unset = $variableThatIsNeverSet
        [pscustomobject]@{
            Emitted  = $emitted.Count
            Leaked   = $leaked.Count
            Eap      = $ErrorActionPreference
            Unset    = ($null -eq $unset)
            Function = ($null -ne (Get-Command -Name 'Get-AbStats' -CommandType Function -ErrorAction SilentlyContinue))
        }
    }
    Assert-Equal 0 $result.Emitted 'objects emitted'
    Assert-Equal 0 $result.Leaked 'command-line variables'
    Assert-Equal 'Continue' $result.Eap 'ErrorActionPreference'
    Assert-Equal $true $result.Unset 'strict mode left off'
    Assert-Equal $true $result.Function 'Get-AbStats defined'
}

Test-Case 'dot-sourcing the text through [scriptblock]::Create, as the elevated session does' {
    $text = [IO.File]::ReadAllText((Join-Path $BenchDir 'ab-stats.ps1'))
    $emitted = @(& { . ([scriptblock]::Create($text)); (Get-AbStats -A $AA1).Text })
    Assert-Equal 1 $emitted.Count 'objects emitted'
    Assert-Equal $AA1Text $emitted[0] 'the functions work'
}

Test-Case 'ab-stats.ps1 and this file are ASCII' {
    foreach ($file in @((Join-Path $BenchDir 'ab-stats.ps1'), $PSCommandPath)) {
        $bytes = [IO.File]::ReadAllBytes($file)
        $bad = 0
        foreach ($b in $bytes) { if ($b -gt 0x7E -or ($b -lt 0x20 -and $b -ne 9 -and $b -ne 10 -and $b -ne 13)) { $bad++ } }
        Assert-Equal 0 $bad ((Split-Path -Leaf $file) + ': bytes outside printable ASCII')
    }
}

Complete-BenchTests
