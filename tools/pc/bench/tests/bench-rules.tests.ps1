# SPDX-License-Identifier: GPL-3.0-or-later
#
# tools/pc/bench/tests/bench-rules.tests.ps1 - bench-rules.ps1's vectors V1-V8 and D1-D5, the rules'
# and flags' edges, and the before/after comparison.
# Project: https://github.com/Maty3k/Kairo
# Design: docs/research/17-loadgen-bench/README.md sections 4.7, 5.4 and 6.
#
#   pwsh -NoProfile -File tools/pc/bench/tests/bench-rules.tests.ps1
#
# It prints one line per case and exits 0 when every case passed, 1 when one failed.
# Every vector runs both ways: through the functions after dot-sourcing bench-rules.ps1 under
# Set-StrictMode -Version 3.0 and $ErrorActionPreference = 'Stop' (testlib.ps1 sets both), and
# through pwsh -File, which must exit with the vector's code and print the same verdict.
# Each V vector is one run record and pm-stats line that break one rule and nothing else (section
# 6); the D vectors are -Decide over a workload's valid runs, with ab-stats' A/A line built here
# from the runs' avg_fps (sample SD, as ab-stats computes it), so these tests don't need ab-stats.
# Fixtures are built in code and written into a temporary folder; synthetic names and values only
# (kairo-loadgen.exe, other.exe).
#
# Rules: ASCII only; Windows PowerShell 5.1 syntax (CI parses tools/** there); no Add-Type.

. (Join-Path $PSScriptRoot 'testlib.ps1')
. (Join-Path $BenchDir 'bench-rules.ps1')

$script:BrDir = New-BenchTempDir
$script:BrAa1 = [double[]]@(120.0, 121.2, 119.4, 120.6, 119.8, 121.0)
$script:BrAa2 = [double[]]@(118.0, 120.4, 121.9, 119.1, 122.3, 117.6)

# --- Fixtures -------------------------------------------------------------------------------

function New-BrRun {
    # A run record as bench.json holds it (research 17 section 4.8), one that passes rules 1 and 2.
    # -Set overrides fields by name; -Drop removes them.
    param([string] $Tag, [int] $N = 1, [hashtable] $Set = @{}, [string[]] $Drop = @())
    $run = [ordered]@{
        n                       = $N
        tag                     = $Tag
        pid                     = 4242
        csv                     = $Tag + '.csv'
        csv_bytes               = 2025000
        csv_ends_newline        = $true
        pm_exit                 = 0
        pm_started              = $true
        pm_stopped              = $true
        etw_loss                = $false
        stopped_early           = $false
        loadgen_exit            = 0
        loadgen_alive_at_pm_end = $true
        retry_of                = $null
        loadgen                 = [ordered]@{ fps = 125.0; gpu_ms_p50 = 7.9; gpu_ms_p99 = 8.1; gpu_ms_max = 9.0; gpu_unknown = 0; gpu_disjoint = 0; occluded = $false }
        elapsed_s               = 69.2
    }
    foreach ($key in $Set.Keys) { $run[$key] = $Set[$key] }
    foreach ($key in $Drop) { $run.Remove($key) }
    return $run
}

function New-BrStats {
    # A pm-stats line (research 17 section 5.1) for a clean 60 s run at 125 fps, one that passes
    # rules 3 to 8 and raises no flag. -Set overrides keys by name.
    param([string] $Tag, [hashtable] $Set = @{})
    $stats = [ordered]@{
        pm_stats              = 1
        tag                   = $Tag
        frames                = 7500
        avg_fps               = 125.0
        low1_fps              = 119.05
        low01_fps             = 112.36
        gpu_busy_ms           = 7.9
        gpu_wait_ms           = 0.1
        present_modes         = [ordered]@{ 'Hardware: Independent Flip' = 7500 }
        frame_types           = [ordered]@{ 'Application' = 7500 }
        low_def               = 'p99_nearest_rank'
        gpu_time_ms           = 8.0
        gpu_busy_p99_ms       = 8.1
        gpu_wait_p99_ms       = 0.2
        gpu_time_p99_ms       = 8.3
        display_change_ms     = 8.0
        display_change_p99_ms = 8.4
        max_frame_ms          = 9.1
        long_frames           = 0
        sync_interval         = [ordered]@{ '0' = 7500 }
        allows_tearing        = [ordered]@{ '1' = 7500 }
        present_flags         = [ordered]@{ '512' = 7500 }
        duration_s            = 60.0
        dropped_pct           = 0.0
        swapchains            = 1
        rows                  = 7500
        bad_rows              = 0
    }
    foreach ($key in $Set.Keys) { $stats[$key] = $Set[$key] }
    return $stats
}

function New-BrDisplays([object[]] $Changes = @()) {
    # bench.json's displays: the 180 Hz bench monitor and one covered display, synthetic.
    return ,@(
        [ordered]@{ role = 'bench'; refresh_mhz = 180000; hdr = $false; size = '1920x1080'; adapter_luid = '0x0000000000001111' },
        [ordered]@{ role = 'cover'; refresh_mhz = 60000; hdr = $false; size = '1920x1080'; adapter_luid = '0x0000000000002222' }
    )
}

function New-BrBlock([string] $Workload, [int] $Pass, [object[]] $Runs) {
    return [ordered]@{ workload = $Workload; pass = $Pass; warmup = [ordered]@{ ok = $true; seconds = 120 }; runs = $Runs }
}

function New-BrBench {
    # A bench.json (research 17 section 4.8) holding the given blocks.
    param([object[]] $Blocks, [double] $Timed = 60, $Refresh = 180000)
    return [ordered]@{
        kairo_bench        = 1
        run_id             = 'bench-20261123-101500'
        started_utc        = '2026-11-23T10:15:00Z'
        ended_utc          = '2026-11-23T10:41:00Z'
        phase              = 'report'
        outcome            = 'complete'
        exit_code          = 0
        decision           = 'none'
        foreign_presentmon = $false
        plan               = [ordered]@{ workloads = @('alu', 'bw'); runs = 6; retries = 3; warmup_s = 120; delay_s = 5; timed_s = $Timed; gap_s = 30; decide_wait_s = 300 }
        monitor            = [ordered]@{ refresh_mhz = $Refresh; hdr = $false; size = '1920x1080'; adapter_luid = '0x0000000000001111'; hags = $true; fingerprint = 'f00d' }
        displays_start     = New-BrDisplays
        displays_end       = New-BrDisplays
        presentmon         = [ordered]@{ file = 'PresentMon-2.6.0-x64.exe'; session = 'KairoBench'; leftover_stopped = $false }
        covers             = 1
        blocks             = $Blocks
    }
}

function ConvertTo-BrJson($Value) {
    return (ConvertTo-Json -InputObject $Value -Compress -Depth 12)
}

function Write-BrFile([string] $Name, [string] $Text) {
    # A fixture file in the test folder, UTF-8 without a byte-order mark; returns its full name.
    $file = Join-Path $script:BrDir $Name
    [IO.File]::WriteAllText($file, $Text, [Text.UTF8Encoding]::new($false))
    return $file
}

function Get-BrCv([double[]] $Fps) {
    # CV = 100 s / mean with the sample SD (n - 1), as ab-stats and research 17 section 4.7 define it.
    $n = $Fps.Length
    $sum = 0.0
    foreach ($f in $Fps) { $sum += $f }
    $mean = $sum / $n
    $squares = 0.0
    foreach ($f in $Fps) { $squares += ($f - $mean) * ($f - $mean) }
    return (100.0 * [Math]::Sqrt($squares / ($n - 1)) / $mean)
}

function New-BrAaLine([double[]] $Fps) {
    # ab-stats' A/A line (research 17 section 5.2) over these avg_fps values, built here.
    $n = $Fps.Length
    $sum = 0.0
    foreach ($f in $Fps) { $sum += $f }
    $mean = $sum / $n
    $squares = 0.0
    foreach ($f in $Fps) { $squares += ($f - $mean) * ($f - $mean) }
    $sd = [Math]::Sqrt($squares / ($n - 1))
    $cv = 100.0 * $sd / $mean
    $xMean = ($n + 1) / 2.0
    $num = 0.0
    $den = 0.0
    for ($i = 0; $i -lt $n; $i++) {
        $num += (($i + 1) - $xMean) * ($Fps[$i] - $mean)
        $den += (($i + 1) - $xMean) * (($i + 1) - $xMean)
    }
    $line = [ordered]@{
        ab_stats          = 1
        mode              = 'aa'
        n                 = $n
        mean_fps          = $mean
        sd_fps            = $sd
        cv_pct            = $cv
        low1_cv_pct       = $null
        drift_pct_per_run = 100.0 * ($num / $den) / $mean
        noise_ok          = ($cv -le 1.0)
    }
    return (ConvertTo-BrJson $line)
}

function Get-BrFirst([double[]] $Values, [int] $Count) {
    $out = [double[]]::new($Count)
    [Array]::Copy($Values, $out, $Count)
    return ,$out
}

function Invoke-BrTool([string[]] $Arguments) {
    # bench-rules.ps1 through pwsh -File; .Json is its line parsed, or $null when it printed none.
    $run = Invoke-BenchTool 'bench-rules.ps1' $Arguments
    $json = $null
    $text = $run.StdOut.Trim()
    if ($text.Length -gt 0 -and $text[0] -eq [char]'{') { $json = ConvertFrom-Json -InputObject $text }
    return [pscustomobject]@{ ExitCode = $run.ExitCode; StdOut = $run.StdOut; StdErr = $run.StdErr; Json = $json }
}

function Get-BrList($Value) {
    # A list from a result, a dictionary's List or a parsed JSON array, as text "a,b".
    return (@($Value | ForEach-Object { [string]$_ }) -join ',')
}

function Get-BrWorkload($Rules, [string] $Name) {
    # One workload's entry from Get-BenchRules' .Rules (dictionaries) or the parsed line.
    foreach ($w in $Rules.workloads) {
        if ([string]$w.workload -ceq $Name) { return $w }
    }
    throw ('no workload ' + $Name)
}

function Assert-BrInvalidFor($Result, [int] $Rule, [string] $What) {
    # A run judged invalid by exactly one rule, with one reason that names it.
    Assert-Equal $false $Result.valid ($What + ': valid')
    Assert-Equal ([string]$Rule) (Get-BrList $Result.failed) ($What + ': failed rules')
    $reasons = @($Result.reasons)
    Assert-Equal 1 $reasons.Count ($What + ': reasons')
    Assert-Match ('^rule ' + $Rule + ': ') ([string]$reasons[0]) ($What + ': reason')
}

function Assert-BrExit([int] $Code, $Run, [string] $What) {
    if ($Run.ExitCode -ne $Code) {
        throw ($What + ': expected exit ' + $Code + ', got ' + $Run.ExitCode + ' (stderr: ' + $Run.StdErr.Trim() + ')')
    }
}

# --- V1-V8: each breaks one rule and nothing else -----------------------------------------------

# Each vector: its rule, the run record's changes and the pm-stats line's changes.
$script:BrVectors = @(
    @{ Id = 'V1'; Rule = 1; Run = @{ pm_stopped = $false }; Stats = @{} },
    @{ Id = 'V2'; Rule = 2; Run = @{ etw_loss = $true }; Stats = @{} },
    @{ Id = 'V3'; Rule = 3; Run = @{}; Stats = @{ swapchains = 2 } },
    @{ Id = 'V4'; Rule = 4; Run = @{}; Stats = @{ duration_s = 58.7 } },
    @{ Id = 'V5'; Rule = 5; Run = @{}; Stats = @{ frame_types = [ordered]@{ 'Application' = 7400; 'AMD AFMF' = 7400 } } },
    @{ Id = 'V6'; Rule = 6; Run = @{}; Stats = @{ present_modes = [ordered]@{ 'Hardware: Independent Flip' = 7350; 'Composed: Flip' = 150 } } },
    @{ Id = 'V7'; Rule = 7; Run = @{}; Stats = @{ long_frames = 1 } },
    @{ Id = 'V8'; Rule = 8; Run = @{}; Stats = @{ dropped_pct = 1.5 } }
)

Test-Case 'V0 the base run passes every rule and raises no flag' {
    $r = Test-BenchRun -Run (New-BrRun 'alu-01') -Stats (New-BrStats 'alu-01') -TimedS 60 -RefreshMhz 180000 -Pass 1
    Assert-Equal $true $r['valid'] 'valid'
    Assert-Equal '' (Get-BrList $r['failed']) 'failed'
    Assert-Equal '' (Get-BrList $r['flags']) 'flags'
    Assert-Equal $false $r['job_retry'] 'job_retry'
    Assert-Equal 'Hardware: Independent Flip' $r['present_mode'] 'present_mode'
    Assert-Equal 100.0 $r['present_mode_share_pct'] 'present_mode_share_pct'
    Assert-Equal 1 $r['n'] 'n'
    Assert-Equal 1 $r['pass'] 'pass'
}

foreach ($vector in $script:BrVectors) {
    Test-Case ($vector.Id + ' breaks rule ' + $vector.Rule + ' alone (functions)') {
        $run = New-BrRun 'alu-01' -Set $vector.Run
        $stats = New-BrStats 'alu-01' -Set $vector.Stats
        $r = Test-BenchRun -Run $run -Stats $stats -TimedS 60 -RefreshMhz 180000 -Pass 1
        Assert-BrInvalidFor $r $vector.Rule $vector.Id
        # Rules 1 and 2 are the job's own: it re-runs such a slot.
        Assert-Equal ($vector.Rule -le 2) $r['job_retry'] ($vector.Id + ': job_retry')
        # The same through Get-BenchRules, with bench.json and the line as JSON text.
        $bench = ConvertTo-BrJson (New-BrBench @((New-BrBlock 'alu' 1 @($run))))
        $rules = (Get-BenchRules -Bench $bench -Stats @((ConvertTo-BrJson $stats))).Rules
        $w = Get-BrWorkload $rules 'alu'
        Assert-Equal 0 $w.valid ($vector.Id + ': the workload''s valid runs')
        Assert-BrInvalidFor $w.results[0] $vector.Rule ($vector.Id + ' through Get-BenchRules')
    }
}

Test-Case 'V1-V8 through pwsh -File: one workload per vector, exit 0' {
    $blocks = [Collections.Generic.List[object]]::new()
    $lines = [Collections.Generic.List[string]]::new()
    $blocks.Add((New-BrBlock 'v0' 1 @((New-BrRun 'v0-01'))))
    $lines.Add((ConvertTo-BrJson (New-BrStats 'v0-01')))
    foreach ($vector in $script:BrVectors) {
        $tag = $vector.Id.ToLowerInvariant() + '-01'
        $blocks.Add((New-BrBlock $vector.Id.ToLowerInvariant() 1 @((New-BrRun $tag -Set $vector.Run))))
        $lines.Add((ConvertTo-BrJson (New-BrStats $tag -Set $vector.Stats)))
    }
    $benchFile = Write-BrFile 'v-bench.json' (ConvertTo-BrJson (New-BrBench $blocks.ToArray()))
    $statsFile = Write-BrFile 'v-stats.jsonl' (($lines -join "`r`n") + "`r`n")
    $run = Invoke-BrTool @('-Bench', $benchFile, '-Stats', $statsFile)
    Assert-BrExit 0 $run 'exit'
    Assert-Equal 'validity' $run.Json.mode 'mode'
    Assert-Equal 'bench-20261123-101500' $run.Json.run_id 'run_id'
    Assert-Equal 60 $run.Json.timed_s 'timed_s'
    Assert-Equal 180000 $run.Json.refresh_mhz 'refresh_mhz'
    $v0 = Get-BrWorkload $run.Json 'v0'
    Assert-Equal 1 $v0.valid 'v0 valid runs'
    Assert-Equal 'v0-01' (Get-BrList $v0.valid_tags) 'v0 valid_tags'
    Assert-Equal 'Hardware: Independent Flip' $v0.present_mode 'v0 present_mode'
    foreach ($vector in $script:BrVectors) {
        $w = Get-BrWorkload $run.Json $vector.Id.ToLowerInvariant()
        Assert-Equal 1 $w.runs ($vector.Id + ': runs')
        Assert-Equal 0 $w.valid ($vector.Id + ': valid runs')
        Assert-BrInvalidFor $w.results[0] $vector.Rule ($vector.Id + ' through pwsh -File')
        Assert-Equal ($vector.Rule -le 2) $w.results[0].job_retry ($vector.Id + ': job_retry')
    }
    Assert-Equal $null $run.Json.session_flags.other_encoder_use 'other_encoder_use without -Encoder'
}

# --- The rules' edges ---------------------------------------------------------------------------

Test-Case 'rule 4 at its edges: 59.0 and 61.0 s count, 61.001 and 58.999 s do not' {
    foreach ($ok in @(59.0, 61.0, 60.4)) {
        $r = Test-BenchRun -Run (New-BrRun 'alu-01') -Stats (New-BrStats 'alu-01' -Set @{ duration_s = $ok }) -TimedS 60
        Assert-Equal $true $r['valid'] ('duration ' + $ok)
    }
    foreach ($bad in @(61.001, 58.999)) {
        $r = Test-BenchRun -Run (New-BrRun 'alu-01') -Stats (New-BrStats 'alu-01' -Set @{ duration_s = $bad }) -TimedS 60
        Assert-BrInvalidFor $r 4 ('duration ' + $bad)
    }
    # A rehearsal's --timed 20.
    $r = Test-BenchRun -Run (New-BrRun 'alu-01') -Stats (New-BrStats 'alu-01' -Set @{ duration_s = 20.2 }) -TimedS 20
    Assert-Equal $true $r['valid'] 'a 20 s rehearsal run'
}

Test-Case 'rule 6 at 99% counts; rule 8 at 1.0 counts and 1.01 does not' {
    $modes = [ordered]@{ 'Hardware: Independent Flip' = 7425; 'Composed: Flip' = 75 }
    $r = Test-BenchRun -Run (New-BrRun 'alu-01') -Stats (New-BrStats 'alu-01' -Set @{ present_modes = $modes }) -TimedS 60
    Assert-Equal $true $r['valid'] '99.00% in one mode'
    Assert-Equal 99.0 $r['present_mode_share_pct'] 'share'
    $r = Test-BenchRun -Run (New-BrRun 'alu-01') -Stats (New-BrStats 'alu-01' -Set @{ dropped_pct = 1.0 }) -TimedS 60
    Assert-Equal $true $r['valid'] 'dropped_pct 1.0'
    $r = Test-BenchRun -Run (New-BrRun 'alu-01') -Stats (New-BrStats 'alu-01' -Set @{ dropped_pct = 1.01 }) -TimedS 60
    Assert-BrInvalidFor $r 8 'dropped_pct 1.01'
}

Test-Case 'rule 1 names each fact the job recorded' {
    $cases = @(
        @{ Set = @{ pm_exit = 1 }; Drop = @(); Text = 'PresentMon exit 1'; Retry = $true },
        @{ Set = @{ pm_exit = 6 }; Drop = @(); Text = 'PresentMon exit 6'; Retry = $false },
        @{ Set = @{ pm_started = $false }; Drop = @(); Text = 'no Started recording. line'; Retry = $true },
        @{ Set = @{ stopped_early = $true }; Drop = @(); Text = 'stopped early'; Retry = $true },
        @{ Set = @{ csv = $null; csv_bytes = 0 }; Drop = @(); Text = 'no CSV'; Retry = $true },
        @{ Set = @{ csv_ends_newline = $false }; Drop = @(); Text = 'the CSV does not end with a newline (cut)'; Retry = $true },
        @{ Set = @{ loadgen_alive_at_pm_end = $false }; Drop = @(); Text = 'loadgen ended before PresentMon'; Retry = $true },
        @{ Set = @{}; Drop = @('pm_stopped'); Text = 'pm_stopped not recorded'; Retry = $true },
        @{ Set = @{ pm_started = 'yes' }; Drop = @(); Text = 'pm_started not recorded'; Retry = $true }
    )
    foreach ($case in $cases) {
        $r = Test-BenchRun -Run (New-BrRun 'alu-01' -Set $case.Set -Drop $case.Drop) -Stats (New-BrStats 'alu-01') -TimedS 60
        Assert-BrInvalidFor $r 1 $case.Text
        Assert-Equal ('rule 1: ' + $case.Text) ([string]$r['reasons'][0]) 'reason'
        Assert-Equal $case.Retry $r['job_retry'] ($case.Text + ': job_retry')
    }
    # A ready file that never came leaves no PresentMon run and no CSV: rule 1, and no pm-stats line.
    $run = New-BrRun 'alu-01' -Set @{ pm_exit = $null; pm_started = $false; pm_stopped = $false; csv = $null; csv_bytes = $null; csv_ends_newline = $null; loadgen_alive_at_pm_end = $false }
    $r = Test-BenchRun -Run $run -Stats $null -TimedS 60
    Assert-Equal '1,3' (Get-BrList $r['failed']) 'a ready time-out'
    Assert-Equal $true $r['job_retry'] 'a ready time-out: job_retry'
}

Test-Case 'rules 2, 3, 5 and 7 when a value is missing or off' {
    $r = Test-BenchRun -Run (New-BrRun 'alu-01' -Drop @('etw_loss')) -Stats (New-BrStats 'alu-01') -TimedS 60
    Assert-BrInvalidFor $r 2 'etw_loss not recorded'
    $r = Test-BenchRun -Run (New-BrRun 'alu-01') -Stats $null -TimedS 60
    Assert-BrInvalidFor $r 3 'no pm-stats line'
    Assert-Equal $false $r['job_retry'] 'no pm-stats line: job_retry'
    $r = Test-BenchRun -Run (New-BrRun 'alu-01') -Stats (New-BrStats 'alu-01' -Set @{ bad_rows = 3 }) -TimedS 60
    Assert-BrInvalidFor $r 3 'bad_rows 3'
    $r = Test-BenchRun -Run (New-BrRun 'alu-01') -Stats (New-BrStats 'alu-01' -Set @{ frame_types = $null }) -TimedS 60
    Assert-BrInvalidFor $r 5 'frame_types null (no FrameType column)'
    $r = Test-BenchRun -Run (New-BrRun 'alu-01') -Stats (New-BrStats 'alu-01' -Set @{ long_frames = $null }) -TimedS 60
    Assert-BrInvalidFor $r 7 'long_frames missing'
    # P12's stall: one 4.5 s frame stays in the CSV, so rule 7 drops the run (section 5.1).
    $r = Test-BenchRun -Run (New-BrRun 'alu-01') -Stats (New-BrStats 'alu-01' -Set @{ long_frames = 1; max_frame_ms = 4500.0 }) -TimedS 60
    Assert-BrInvalidFor $r 7 'a stall'
}

Test-Case 'a run breaking several rules lists each once, in order' {
    $stats = New-BrStats 'alu-01' -Set @{ dropped_pct = 50.0; long_frames = 2; duration_s = 30.0 }
    $r = Test-BenchRun -Run (New-BrRun 'alu-01' -Set @{ etw_loss = $true }) -Stats $stats -TimedS 60
    Assert-Equal '2,4,7,8' (Get-BrList $r['failed']) 'failed'
    Assert-Equal 4 @($r['reasons']).Count 'reasons'
}

# --- Rule 6 across a workload, and both passes ---------------------------------------------------

function New-BrModeRun([string] $Tag, [int] $N, [string] $Mode, [hashtable] $Set = @{}) {
    $stats = New-BrStats $Tag -Set $Set
    $stats['present_modes'] = [ordered]@{}
    $stats['present_modes'][$Mode] = 7500
    return @{ Run = (New-BrRun $Tag -N $N); Stats = $stats }
}

function Invoke-BrModes([object[]] $Items) {
    # A one-workload bench of these runs, judged through Get-BenchRules.
    $runs = @($Items | ForEach-Object { $_.Run })
    $lines = @($Items | ForEach-Object { ConvertTo-BrJson $_.Stats })
    $bench = New-BrBench @((New-BrBlock 'alu' 1 $runs))
    return (Get-BrWorkload (Get-BenchRules -Bench $bench -Stats $lines).Rules 'alu')
}

Test-Case 'rule 6: a run in another present mode than most valid runs fails' {
    $items = @()
    for ($i = 1; $i -le 6; $i++) {
        $mode = 'Hardware: Independent Flip'
        if ($i -eq 4) { $mode = 'Composed: Flip' }
        $items += New-BrModeRun ('alu-0' + $i) $i $mode
    }
    $w = Invoke-BrModes $items
    Assert-Equal 6 $w.runs 'runs'
    Assert-Equal 5 $w.valid 'valid'
    Assert-Equal 'alu-01,alu-02,alu-03,alu-05,alu-06' (Get-BrList $w.valid_tags) 'valid_tags'
    Assert-Equal 'Hardware: Independent Flip' $w.present_mode 'present_mode'
    Assert-Equal 100.0 $w.present_mode_share_pct 'present_mode_share_pct'
    Assert-BrInvalidFor $w.results[3] 6 'alu-04'
    Assert-Match 'Composed: Flip' ([string]$w.results[3].reasons[0]) 'reason'
}

Test-Case 'rule 6: valid runs split evenly between two present modes all fail' {
    $items = @()
    for ($i = 1; $i -le 6; $i++) {
        $mode = 'Hardware: Independent Flip'
        if ($i -gt 3) { $mode = 'Composed: Flip' }
        $items += New-BrModeRun ('alu-0' + $i) $i $mode
    }
    $w = Invoke-BrModes $items
    Assert-Equal 0 $w.valid 'valid'
    Assert-Equal $null $w.present_mode 'present_mode'
    Assert-Equal $null $w.present_mode_share_pct 'present_mode_share_pct'
    foreach ($result in $w.results) { Assert-BrInvalidFor $result 6 $result.tag }
}

Test-Case 'rule 6: a run invalid for another rule does not vote' {
    $items = @()
    for ($i = 1; $i -le 6; $i++) {
        if ($i -le 3) { $items += New-BrModeRun ('alu-0' + $i) $i 'Hardware: Independent Flip' }
        else { $items += New-BrModeRun ('alu-0' + $i) $i 'Composed: Flip' @{ long_frames = 1 } }
    }
    $w = Invoke-BrModes $items
    Assert-Equal 3 $w.valid 'valid'
    Assert-Equal 'Hardware: Independent Flip' $w.present_mode 'present_mode'
    for ($i = 3; $i -le 5; $i++) { Assert-BrInvalidFor $w.results[$i] 7 $w.results[$i].tag }
}

Test-Case 'the workload pools both passes: extra runs keep the first pass''s mode' {
    $first = @()
    $second = @()
    $lines = @()
    for ($i = 1; $i -le 6; $i++) {
        $tag = 'bw-{0:D2}' -f $i
        $first += New-BrRun $tag -N $i
        $lines += ConvertTo-BrJson (New-BrStats $tag)
    }
    for ($i = 7; $i -le 12; $i++) {
        $tag = 'bw-{0:D2}' -f $i
        $second += New-BrRun $tag -N $i
        $set = @{}
        if ($i -eq 12) { $set = @{ present_modes = [ordered]@{ 'Composed: Flip' = 7500 } } }
        $lines += ConvertTo-BrJson (New-BrStats $tag -Set $set)
    }
    $bench = New-BrBench @((New-BrBlock 'bw' 1 $first), (New-BrBlock 'bw' 2 $second))
    $w = Get-BrWorkload (Get-BenchRules -Bench $bench -Stats $lines).Rules 'bw'
    Assert-Equal 12 $w.runs 'runs'
    Assert-Equal 11 $w.valid 'valid'
    Assert-Equal 1 $w.results[0]['pass'] 'first pass'
    Assert-Equal 2 $w.results[11]['pass'] 'second pass'
    Assert-BrInvalidFor $w.results[11] 6 'bw-12'
}

# --- Flags ----------------------------------------------------------------------------------------

Test-Case 'flags are reported and never invalidate a run' {
    $stats = New-BrStats 'alu-01' -Set @{
        avg_fps        = 179.0
        gpu_busy_ms    = 4.0
        frames         = 900
        sync_interval  = [ordered]@{ '0' = 899; '1' = 1 }
        allows_tearing = [ordered]@{ '0' = 2; '1' = 898 }
        present_flags  = [ordered]@{ '0' = 1; '512' = 899 }
    }
    $r = Test-BenchRun -Run (New-BrRun 'alu-01') -Stats $stats -TimedS 60 -RefreshMhz 180000
    Assert-Equal $true $r['valid'] 'valid'
    Assert-Equal 'near_refresh,not_gpu_limited,few_frames,sync_interval_not_0,allows_tearing_not_1,present_flags_not_512' (Get-BrList $r['flags']) 'flags'
    # 125 fps at 7.9 ms is GPU-limited (0.99); at 7.1 ms it is not (0.89). Without a refresh rate,
    # near_refresh can't be judged.
    $r = Test-BenchRun -Run (New-BrRun 'alu-01') -Stats (New-BrStats 'alu-01' -Set @{ gpu_busy_ms = 7.1 }) -TimedS 60
    Assert-Equal 'not_gpu_limited' (Get-BrList $r['flags']) 'gpu_busy_ms 7.1'
    $r = Test-BenchRun -Run (New-BrRun 'alu-01') -Stats (New-BrStats 'alu-01' -Set @{ avg_fps = 178.3; gpu_busy_ms = 5.6 }) -TimedS 60 -RefreshMhz 180000
    Assert-Equal 'near_refresh' (Get-BrList $r['flags']) 'avg_fps 178.3 at 180 Hz'
    $r = Test-BenchRun -Run (New-BrRun 'alu-01') -Stats (New-BrStats 'alu-01' -Set @{ avg_fps = 178.1; gpu_busy_ms = 5.6 }) -TimedS 60 -RefreshMhz 180000
    Assert-Equal '' (Get-BrList $r['flags']) 'avg_fps 178.1 at 180 Hz'
}

Test-Case 'flags at their edges: near_refresh includes 1%, not_gpu_limited and few_frames exclude theirs' {
    # |99.0 - 100| is 1.0, exactly 1% of 100 Hz: a cap. At 98.99 it is 1.01. Both are GPU-limited
    # (9.5 ms x 99 fps / 1000 = 0.94).
    $r = Test-BenchRun -Run (New-BrRun 'alu-01') -Stats (New-BrStats 'alu-01' -Set @{ avg_fps = 99.0; gpu_busy_ms = 9.5 }) -TimedS 60 -RefreshMhz 100000
    Assert-Equal 'near_refresh' (Get-BrList $r['flags']) 'avg_fps 99.0 at 100 Hz'
    $r = Test-BenchRun -Run (New-BrRun 'alu-01') -Stats (New-BrStats 'alu-01' -Set @{ avg_fps = 98.99; gpu_busy_ms = 9.5 }) -TimedS 60 -RefreshMhz 100000
    Assert-Equal '' (Get-BrList $r['flags']) 'avg_fps 98.99 at 100 Hz'
    # 7.2 ms x 125 fps / 1000 is 0.9 exactly: still GPU-limited.
    $r = Test-BenchRun -Run (New-BrRun 'alu-01') -Stats (New-BrStats 'alu-01' -Set @{ gpu_busy_ms = 7.2 }) -TimedS 60 -RefreshMhz 180000
    Assert-Equal '' (Get-BrList $r['flags']) 'gpu_busy_ms 7.2 at 125 fps'
    # 1,000 frames are enough; 999 are few.
    $r = Test-BenchRun -Run (New-BrRun 'alu-01') -Stats (New-BrStats 'alu-01' -Set @{ frames = 1000 }) -TimedS 60 -RefreshMhz 180000
    Assert-Equal '' (Get-BrList $r['flags']) 'frames 1000'
    $r = Test-BenchRun -Run (New-BrRun 'alu-01') -Stats (New-BrStats 'alu-01' -Set @{ frames = 999 }) -TimedS 60 -RefreshMhz 180000
    Assert-Equal 'few_frames' (Get-BrList $r['flags']) 'frames 999'
}

Test-Case 'a workload reports its valid runs'' flags; the session counts other encoders, never naming them' {
    $runs = @((New-BrRun 'alu-01' -N 1), (New-BrRun 'alu-02' -N 2), (New-BrRun 'alu-03' -N 3))
    $lines = @(
        (ConvertTo-BrJson (New-BrStats 'alu-01' -Set @{ frames = 900 })),
        (ConvertTo-BrJson (New-BrStats 'alu-02')),
        (ConvertTo-BrJson (New-BrStats 'alu-03' -Set @{ gpu_busy_ms = 1.0; long_frames = 1 }))
    )
    $before = [ordered]@{
        kairo_encoder_usage = 1; seconds = 3; samples = 3; invalid_samples = 0; elevated = $true
        types = [ordered]@{ '3d' = 24; 'videocodec' = 4 }
        video = @(
            [ordered]@{ name = 'kairo-loadgen'; pid = 4242; luid = '0x0000000000001111'; engtype = 'videocodec'; eng = 4; avg_pct = 5.0; max_pct = 6.0; seen = 3 },
            [ordered]@{ name = 'other'; pid = 5120; luid = '0x0000000000001111'; engtype = 'videocodec'; eng = 4; avg_pct = 3.1; max_pct = 7.5; seen = 3 },
            [ordered]@{ name = 'other'; pid = 5120; luid = '0x0000000000002222'; engtype = 'videocodec'; eng = 4; avg_pct = 2.0; max_pct = 2.5; seen = 3 },
            [ordered]@{ name = 'quiet'; pid = 6000; luid = '0x0000000000001111'; engtype = 'videocodec'; eng = 4; avg_pct = 1.0; max_pct = 9.0; seen = 3 },
            [ordered]@{ name = '(exited)'; pid = 7000; luid = '0x0000000000001111'; engtype = 'videocodec'; eng = 4; avg_pct = 2.5; max_pct = 3.0; seen = 1 }
        )
    }
    $after = [ordered]@{ kairo_encoder_usage = 1; seconds = 3; samples = 3; invalid_samples = 0; elevated = $true; types = [ordered]@{ '3d' = 24 }; video = @(
            [ordered]@{ name = 'OTHER'; pid = 5121; luid = '0x0000000000001111'; engtype = 'videocodec'; eng = 4; avg_pct = 4.0; max_pct = 4.0; seen = 3 },
            [ordered]@{ name = 'testbed'; pid = 8000; luid = '0x0000000000001111'; engtype = 'videocodec'; eng = 4; avg_pct = 9.0; max_pct = 9.0; seen = 3 }
        )
    }
    $bench = New-BrBench @((New-BrBlock 'alu' 1 $runs))
    $r = Get-BenchRules -Bench $bench -Stats $lines -EncoderUsage @((ConvertTo-BrJson $before), (ConvertTo-BrJson $after))
    $w = Get-BrWorkload $r.Rules 'alu'
    Assert-Equal 2 $w.valid 'valid'
    # alu-03's not_gpu_limited flag isn't the workload's: that run is invalid (rule 7).
    Assert-Equal 'few_frames' (Get-BrList $w.flags) 'workload flags'
    # other (one program over both lines) and the exited process 7000; not quiet (1.0%), not Kairo's.
    Assert-Equal 2 $r.Rules['session_flags']['other_encoder_use'] 'other_encoder_use'
    Assert-True ($r.Json -cnotmatch '"(other|OTHER|quiet|\(exited\))"') 'no program name in the line'
    # The same through pwsh -File, the encoder lines in two files.
    $benchFile = Write-BrFile 'enc-bench.json' (ConvertTo-BrJson $bench)
    $statsFile = Write-BrFile 'enc-stats.jsonl' (($lines -join "`n") + "`n")
    $beforeFile = Write-BrFile 'encoder-before.json' (ConvertTo-BrJson $before)
    $afterFile = Write-BrFile 'encoder-after.json' (ConvertTo-BrJson $after)
    $run = Invoke-BrTool @('-Bench', $benchFile, '-Stats', $statsFile, '-Encoder', $beforeFile, $afterFile)
    Assert-BrExit 0 $run 'exit'
    Assert-Equal 2 $run.Json.session_flags.other_encoder_use 'other_encoder_use through pwsh -File'
    Assert-Equal 'few_frames' (Get-BrList (Get-BrWorkload $run.Json 'alu').flags) 'workload flags through pwsh -File'
    Assert-True ($run.StdOut -cnotmatch '"(other|OTHER|quiet|\(exited\))"') 'no program name on stdout'
}

# --- Validity's refusals ------------------------------------------------------------------------

Test-Case 'validity refuses inputs it can''t judge (exit 3), naming no folder' {
    $good = New-BrBench @((New-BrBlock 'alu' 1 @((New-BrRun 'alu-01'))))
    $line = ConvertTo-BrJson (New-BrStats 'alu-01')
    $cases = @(
        @{ What = 'not a bench.json'; Bench = (ConvertTo-BrJson ([ordered]@{ kairo_bench = 2; run_id = 'bench-20261123-101500' })); Stats = @($line); Text = 'not a bench.json' },
        @{ What = 'a top-level list'; Bench = '[' + (ConvertTo-BrJson $good) + ']'; Stats = @($line); Text = 'not a JSON object' },
        @{ What = 'two lines for one tag'; Bench = (ConvertTo-BrJson $good); Stats = @($line, $line); Text = 'two pm-stats lines for alu-01' },
        @{ What = 'a line for a run bench.json lacks'; Bench = (ConvertTo-BrJson $good); Stats = @($line, (ConvertTo-BrJson (New-BrStats 'alu-09'))); Text = 'alu-09, which bench.json does not list' },
        @{ What = 'a line that isn''t pm-stats'; Bench = (ConvertTo-BrJson $good); Stats = @('{"ab_stats":1,"tag":"alu-01"}'); Text = 'not a pm-stats line' },
        @{ What = 'unparsable JSON'; Bench = (ConvertTo-BrJson $good); Stats = @('{"pm_stats":1,'); Text = 'not valid JSON' },
        @{ What = 'a tag with a path in it'; Bench = (ConvertTo-BrJson (New-BrBench @((New-BrBlock 'alu' 1 @((New-BrRun 'C:\x')))))); Stats = @(); Text = 'a run without a valid tag' },
        @{ What = 'a tag ending in a line end'; Bench = (ConvertTo-BrJson (New-BrBench @((New-BrBlock 'alu' 1 @((New-BrRun ('alu-01' + [char]10))))))); Stats = @(); Text = 'a run without a valid tag' },
        @{ What = 'the elevated fallback'; Bench = (ConvertTo-BrJson (New-BrBench @((New-BrBlock 'alu' 1 @((New-BrRun 'alu-01' -Set @{ route = 'elevated_stdout' })))))); Stats = @($line); Text = 'not built yet' }
    )
    foreach ($case in $cases) {
        $e = Assert-Throws { Get-BenchRules -Bench $case.Bench -Stats $case.Stats } $case.What
        Assert-Equal 3 (Get-BenchRulesExitCode $e) ($case.What + ': exit code')
        Assert-Match ([regex]::Escape($case.Text)) $e.Exception.Message ($case.What + ': message')
    }
    # Through pwsh -File: a missing file and a bad bench.json exit 3 and name the file alone.
    $statsFile = Write-BrFile 'refuse-stats.jsonl' $line
    $missing = Join-Path $script:BrDir 'no-such-bench.json'
    $run = Invoke-BrTool @('-Bench', $missing, '-Stats', $statsFile)
    Assert-BrExit 3 $run 'a missing bench.json'
    Assert-Match '^bench-rules: no such file: no-such-bench\.json\s*$' $run.StdErr 'stderr'
    Assert-True ($run.StdErr -cnotmatch [regex]::Escape($script:BrDir)) 'stderr names no folder'
    Assert-Equal '' $run.StdOut.Trim() 'stdout'
    $badFile = Write-BrFile 'refuse-bench.json' '{"kairo_bench":1}'
    $run = Invoke-BrTool @('-Bench', $badFile, '-Stats', $statsFile)
    Assert-BrExit 3 $run 'a bench.json without run_id'
}

Test-Case 'the command line refuses a bad one (exit 2)' {
    $benchFile = Write-BrFile 'cl-bench.json' (ConvertTo-BrJson (New-BrBench @((New-BrBlock 'alu' 1 @((New-BrRun 'alu-01'))))))
    $statsFile = Write-BrFile 'cl-stats.jsonl' (ConvertTo-BrJson (New-BrStats 'alu-01'))
    $cases = @(
        @{ What = 'no -Stats'; Args = @('-Bench', $benchFile); Text = 'missing -Stats' },
        @{ What = 'no -Bench'; Args = @('-Stats', $statsFile); Text = 'missing -Bench' },
        @{ What = 'an unknown parameter'; Args = @('-Bench', $benchFile, '-Stats', $statsFile, '-Verbose'); Text = 'unknown parameter -Verbose' },
        @{ What = 'a stray argument'; Args = @($benchFile); Text = 'unexpected argument at position 1' },
        @{ What = 'a parameter twice'; Args = @('-Bench', $benchFile, '-bench', $benchFile, '-Stats', $statsFile); Text = '-Bench given twice' },
        @{ What = 'both -Decide and -CompareEnv'; Args = @('-Decide', '-CompareEnv', '-Valid', '6'); Text = 'not both' },
        @{ What = '-Valid in validity'; Args = @('-Bench', $benchFile, '-Stats', $statsFile, '-Valid', '6'); Text = '-Valid does not go with this mode' },
        @{ What = '-Stats with no value'; Args = @('-Bench', $benchFile, '-Stats'); Text = '-Stats needs at least one value' }
    )
    foreach ($case in $cases) {
        $run = Invoke-BrTool $case.Args
        Assert-BrExit 2 $run $case.What
        Assert-Match ([regex]::Escape($case.Text)) $run.StdErr ($case.What + ': stderr')
    }
    # -Name:value and -Switch:$false work as pwsh passes them.
    $run = Invoke-BrTool @('-Bench:', $benchFile, '-Stats', $statsFile, '-Decide:$false')
    Assert-BrExit 0 $run '-Bench: and -Decide:$false'
    Assert-Equal 'validity' $run.Json.mode 'mode'
}

# --- D1-D5: the extra runs at the decision point ---------------------------------------------------

Test-Case 'the fixtures'' CVs are section 6''s (0.59%, 1.65%, 0.64%, 1.40%)' {
    Assert-Equal '0.59' (Get-BrCv $script:BrAa1).ToString('F2', [Globalization.CultureInfo]::InvariantCulture) 'AA1'
    Assert-Equal '1.65' (Get-BrCv $script:BrAa2).ToString('F2', [Globalization.CultureInfo]::InvariantCulture) 'AA2'
    Assert-Equal '0.64' (Get-BrCv (Get-BrFirst $script:BrAa1 4)).ToString('F2', [Globalization.CultureInfo]::InvariantCulture) 'AA1 first 4'
    Assert-Equal '1.40' (Get-BrCv (Get-BrFirst $script:BrAa2 4)).ToString('F2', [Globalization.CultureInfo]::InvariantCulture) 'AA2 first 4'
}

# Each D vector: the valid runs' avg_fps (or $null: no A/A line), and what -Decide proposes.
$script:BrDecisions = @(
    @{ Id = 'D1'; Valid = 6; Fps = $script:BrAa1; Extra = 0; Kind = 'none'; High = $false; Note = '^CV 0\.59% over 6 valid runs is 1\.00% or less: no extra runs$' },
    @{ Id = 'D2'; Valid = 6; Fps = $script:BrAa2; Extra = 6; Kind = 'extension'; High = $true; Note = '^CV 1\.65% over 6 valid runs is above 1\.00%: 6 more runs' },
    @{ Id = 'D3'; Valid = 4; Fps = (Get-BrFirst $script:BrAa1 4); Extra = 2; Kind = 'make_up'; High = $false; Note = '^2 make-up runs to reach 6 valid runs$' },
    @{ Id = 'D4'; Valid = 4; Fps = (Get-BrFirst $script:BrAa2 4); Extra = 2; Kind = 'make_up'; High = $true; Note = '^2 make-up runs to reach 6 valid runs; CV 1\.40% over the 4 valid runs is above 1\.00%' },
    @{ Id = 'D5'; Valid = 2; Fps = (Get-BrFirst $script:BrAa1 2); Extra = 0; Kind = 'insufficient'; High = $false; Note = '^insufficient data: fix the setup' }
)

foreach ($vector in $script:BrDecisions) {
    Test-Case ($vector.Id + ' ' + $vector.Valid + ' valid runs -> ' + $vector.Extra + ' extra (' + $vector.Kind + '), both ways') {
        $line = New-BrAaLine $vector.Fps
        $d = (Get-BenchDecision -Valid $vector.Valid -AaStats $line -Workload 'alu').Decision
        Assert-Equal $vector.Extra $d['extra'] 'extra'
        Assert-Equal $vector.Kind $d['kind'] 'kind'
        Assert-Equal $vector.High $d['cv_high'] 'cv_high'
        Assert-Equal $vector.Valid $d['valid'] 'valid'
        Assert-Equal 'alu' $d['workload'] 'workload'
        Assert-Near (Get-BrCv $vector.Fps) $d['cv_pct'] 1e-12 'cv_pct'
        Assert-Match $vector.Note $d['note'] 'note'
        # The A/A line as a file, and as text on the command line.
        $file = Write-BrFile ('aa-' + $vector.Id + '.json') ($line + "`r`n")
        foreach ($given in @($file, $line)) {
            $run = Invoke-BrTool @('-Decide', '-Valid', ([string]$vector.Valid), '-AaStats', $given, '-Workload', 'alu')
            Assert-BrExit 0 $run 'exit'
            Assert-Equal 'decide' $run.Json.mode 'mode'
            Assert-Equal $vector.Extra $run.Json.extra 'extra through pwsh -File'
            Assert-Equal $vector.Kind $run.Json.kind 'kind through pwsh -File'
            Assert-Equal $vector.High $run.Json.cv_high 'cv_high through pwsh -File'
            Assert-Match $vector.Note $run.Json.note 'note through pwsh -File'
        }
    }
}

Test-Case 'D5 without an A/A line, and the counts between' {
    foreach ($n in @(0, 1, 2)) {
        $d = (Get-BenchDecision -Valid $n).Decision
        Assert-Equal 'insufficient' $d['kind'] ('-Valid ' + $n)
        Assert-Equal 0 $d['extra'] ('-Valid ' + $n + ': extra')
        Assert-Equal $null $d['cv_pct'] ('-Valid ' + $n + ': cv_pct')
        Assert-Equal $null $d['workload'] ('-Valid ' + $n + ': workload')
    }
    foreach ($n in @(3, 4, 5)) {
        $d = (Get-BenchDecision -Valid $n).Decision
        Assert-Equal 'make_up' $d['kind'] ('-Valid ' + $n)
        Assert-Equal (6 - $n) $d['extra'] ('-Valid ' + $n + ': extra')
        Assert-Equal $null $d['cv_high'] ('-Valid ' + $n + ': cv_high')
    }
    $run = Invoke-BrTool @('-Decide', '-Valid', '2')
    Assert-BrExit 0 $run 'D5 through pwsh -File'
    Assert-Equal 'insufficient' $run.Json.kind 'kind'
    Assert-Equal 0 $run.Json.extra 'extra'
}

Test-Case 'a CV of exactly 1.00% needs no extra runs' {
    $line = '{"ab_stats":1,"mode":"aa","n":6,"mean_fps":120.0,"sd_fps":1.2,"cv_pct":1.0,"low1_cv_pct":null,"drift_pct_per_run":0.0,"noise_ok":true}'
    $d = (Get-BenchDecision -Valid 6 -AaStats $line).Decision
    Assert-Equal 'none' $d['kind'] 'kind'
    Assert-Equal 'CV 1.00% over 6 valid runs is 1.00% or less: no extra runs' $d['note'] 'note'
    $line = $line.Replace('"cv_pct":1.0', '"cv_pct":1.0000001').Replace('"noise_ok":true', '"noise_ok":false')
    $d = (Get-BenchDecision -Valid 6 -AaStats $line).Decision
    Assert-Equal 'extension' $d['kind'] 'kind just above 1%'
    # Three decimals, at least 1.001, where two would print 1.00 beside "above 1.00%".
    Assert-Equal 'CV 1.001% over 6 valid runs is above 1.00%: 6 more runs, then the CV over all of them' $d['note'] 'note just above 1%'
}

Test-Case 'the note prints a CV just above 1% with 3 decimals, as ab-stats does' {
    # ab-stats' A/A line for a CV of 1.0030%, built here: 6 more runs, and the note says 1.003%.
    $line = New-BrAaLine ([double[]]@(119.768, 121.803, 118.751, 120.785, 119.429, 121.464))
    $d = (Get-BenchDecision -Valid 6 -AaStats $line -Workload 'alu').Decision
    Assert-Equal 'extension' $d['kind'] 'kind'
    Assert-Equal $true $d['cv_high'] 'cv_high'
    Assert-Match '^CV 1\.003% over 6 valid runs is above 1\.00%: 6 more runs' $d['note'] 'note'
    $run = Invoke-BrTool @('-Decide', '-Valid', '6', '-AaStats', $line, '-Workload', 'alu')
    Assert-BrExit 0 $run 'through pwsh -File'
    Assert-Match '^CV 1\.003% over 6 valid runs is above 1\.00%: 6 more runs' $run.Json.note 'note through pwsh -File'
    # The make-up note too.
    $line = '{"ab_stats":1,"mode":"aa","n":4,"mean_fps":120.0,"sd_fps":1.2,"cv_pct":1.0042,"low1_cv_pct":null,"drift_pct_per_run":0.0,"noise_ok":false}'
    $d = (Get-BenchDecision -Valid 4 -AaStats $line).Decision
    Assert-Match '^2 make-up runs to reach 6 valid runs; CV 1\.004% over the 4 valid runs is above 1\.00%' $d['note'] 'make-up note'
    # The rule alone: two decimals unless a CV above 1.00 would print as 1.00.
    $cases = @(@(0.59, '0.59'), @(1.0, '1.00'), @(1.0000001, '1.001'), @(1.0049, '1.005'), @(1.0051, '1.01'), @(1.65, '1.65'))
    foreach ($case in $cases) { Assert-Equal $case[1] (Format-BenchRulesCv $case[0]) ('CV ' + $case[0]) }
}

Test-Case '-Decide refuses what it can''t decide' {
    $aa1 = New-BrAaLine $script:BrAa1
    $cases = @(
        @{ What = '6 valid runs without an A/A line'; Valid = 6; Aa = $null; Code = 2 },
        @{ What = 'an A/A line over another number of runs'; Valid = 5; Aa = $aa1; Code = 2 },
        @{ What = 'a -Valid that is not a whole number'; Valid = '4.5'; Aa = $null; Code = 2 },
        @{ What = 'a negative -Valid'; Valid = '-1'; Aa = $null; Code = 2 },
        @{ What = 'a paired ab-stats line'; Valid = 6; Aa = '{"ab_stats":1,"mode":"ab","n":6,"loss_pct":2.0}'; Code = 3 },
        @{ What = 'a noise_ok that disagrees with cv_pct'; Valid = 6; Aa = $aa1.Replace('"noise_ok":true', '"noise_ok":false'); Code = 3 }
    )
    foreach ($case in $cases) {
        $e = Assert-Throws { Get-BenchDecision -Valid $case.Valid -AaStats $case.Aa } $case.What
        Assert-Equal $case.Code (Get-BenchRulesExitCode $e) ($case.What + ': exit code')
        $arguments = @('-Decide', '-Valid', [string]$case.Valid)
        if ($null -ne $case.Aa) { $arguments += @('-AaStats', $case.Aa) }
        $run = Invoke-BrTool $arguments
        Assert-BrExit $case.Code $run ($case.What + ' through pwsh -File')
    }
    $run = Invoke-BrTool @('-Decide', '-Valid', '4', '-Workload', '../x')
    Assert-BrExit 2 $run 'a bad -Workload'
    $run = Invoke-BrTool @('-Decide')
    Assert-BrExit 2 $run 'no -Valid'
}

Test-Case 'the chain: validity gives the valid runs, ab-stats their CV, -Decide the make-up runs' {
    # Six runs at AA1's avg_fps; runs 2 and 5 fail rules 3 and 7, so 4 remain (AA1's 1, 3, 4, 6).
    $runs = @()
    $lines = @()
    for ($i = 1; $i -le 6; $i++) {
        $tag = 'alu-0' + $i
        $set = @{ avg_fps = $script:BrAa1[$i - 1] }
        if ($i -eq 2) { $set['bad_rows'] = 1 }
        if ($i -eq 5) { $set['long_frames'] = 1 }
        $runs += New-BrRun $tag -N $i
        $lines += ConvertTo-BrJson (New-BrStats $tag -Set $set)
    }
    $w = Get-BrWorkload (Get-BenchRules -Bench (New-BrBench @((New-BrBlock 'alu' 1 $runs))) -Stats $lines).Rules 'alu'
    Assert-Equal 'alu-01,alu-03,alu-04,alu-06' (Get-BrList $w.valid_tags) 'valid_tags'
    $fps = [double[]]@(120.0, 119.4, 120.6, 121.0)
    $d = (Get-BenchDecision -Valid $w.valid -AaStats (New-BrAaLine $fps) -Workload $w.workload).Decision
    Assert-Equal 'make_up' $d['kind'] 'kind'
    Assert-Equal 2 $d['extra'] 'extra'
}

# --- The before/after comparison (-CompareEnv) -----------------------------------------------------

function New-BrEnv {
    # An env.json (research 17 section 5.4) with synthetic values. -Set replaces top-level fields.
    param([hashtable] $Set = @{}, [string[]] $Drop = @())
    $env = [ordered]@{
        windows        = [ordered]@{ display_version = '00H0'; build = '10000.1'; edition = 'Synthetic' }
        reboot_pending = $false
        uptime_s       = 3600
        gpus           = @(
            [ordered]@{ driver_version = '1.0.0.1'; driver_date = '2026-10-01'; pci = 'VEN_0000:DEV_0001'; radeon_software_version = 'absent' },
            [ordered]@{ driver_version = '1.0.0.2'; driver_date = '2026-10-01'; pci = 'VEN_0000:DEV_0002'; radeon_software_version = 'absent' }
        )
        amd_software   = [ordered]@{ installed = $true; version = '0.0.1'; basis = 'uninstall' }
        amd_hints      = [ordered]@{ InstantReplayEnabled = 0; InGameReplayEnabled = 'absent'; DvrEnabled = 1; DesktopRecordingEnabled = 0; EnableMetricsOverlay = 'absent' }
        hags           = [ordered]@{ hw_sch_mode = 2; effective = $true }
        dx_user        = [ordered]@{ VRROptimizeEnable = 'absent'; SwapEffectUpgradeEnable = 1; AutoHDREnable = 'absent' }
        vrr            = 'unknown'
        power          = [ordered]@{ scheme = 'balanced'; overlay_ac = 'none'; display_off_ac_s = 600 }
        lock           = [ordered]@{ screen_saver = [ordered]@{ active = $false; timeout_s = 600; secure = $false; set = $false }; dynamic_lock = $false; inactivity_s = 0 }
        tdr            = [ordered]@{ events_4101_7d = 0; live_kernel_reports = 0 }
        mpo            = 'absent'
        kairo          = [ordered]@{ build_id = '2026-11-20T10:10:10Z'; commit = 'abc1234' }
        game           = $null
        kairo_engine   = $null
    }
    foreach ($key in $Set.Keys) { $env[$key] = $Set[$key] }
    foreach ($key in $Drop) { $env.Remove($key) }
    return $env
}

function Invoke-BrCompare($Before, $After, $Bench = $null) {
    # -CompareEnv both ways: through Compare-BenchEnv with the JSON text, and through pwsh -File
    # with files; both must agree. Returns the function's result.
    $beforeText = ConvertTo-BrJson $Before
    $afterText = ConvertTo-BrJson $After
    $benchText = $null
    if ($null -ne $Bench) { $benchText = ConvertTo-BrJson $Bench }
    $c = Compare-BenchEnv -Before $beforeText -After $afterText -Bench $benchText
    $arguments = @('-CompareEnv', '-Before', (Write-BrFile 'env-before.json' $beforeText), '-After', (Write-BrFile 'env-after.json' $afterText))
    if ($null -ne $benchText) { $arguments += @('-Bench', (Write-BrFile 'env-bench.json' $benchText)) }
    $run = Invoke-BrTool $arguments
    Assert-BrExit $c.ExitCode $run 'pwsh -File exits as Compare-BenchEnv returns'
    Assert-Equal $c.Json $run.StdOut.Trim() 'pwsh -File prints the same line'
    return $c
}

Test-Case 'C1 the same environment: exit 0' {
    $c = Invoke-BrCompare (New-BrEnv) (New-BrEnv)
    Assert-Equal 0 $c.ExitCode 'exit code'
    Assert-Equal $true $c.Comparison['same'] 'same'
    Assert-Equal '' (Get-BrList $c.Comparison['changed']) 'changed'
    Assert-Equal 'displays' (Get-BrList $c.Comparison['not_compared']) 'not_compared without -Bench'
}

Test-Case 'C2 a driver update during the session: exit 1, the field named and no value printed' {
    $after = New-BrEnv
    $after['gpus'][1]['driver_version'] = '1.0.0.9'
    $c = Invoke-BrCompare (New-BrEnv) $after
    Assert-Equal 1 $c.ExitCode 'exit code'
    Assert-Equal $false $c.Comparison['same'] 'same'
    Assert-Equal 'gpus[].driver_version' (Get-BrList $c.Comparison['changed']) 'changed'
    Assert-True ($c.Json -cnotmatch '1\.0\.0\.|10000|VEN_|abc1234') 'no value in the line'
}

Test-Case 'C3 each compared field is seen; uncompared fields and order are not' {
    $changes = @(
        @{ Field = 'windows.build'; Set = @{ windows = [ordered]@{ display_version = '00H0'; build = '10000.2'; edition = 'Synthetic' } } },
        @{ Field = 'reboot_pending'; Set = @{ reboot_pending = $true } },
        @{ Field = 'amd_software.version'; Set = @{ amd_software = [ordered]@{ installed = $true; version = '0.0.2'; basis = 'uninstall' } } },
        @{ Field = 'hags'; Set = @{ hags = [ordered]@{ hw_sch_mode = 1; effective = $true } } },
        @{ Field = 'power.scheme'; Set = @{ power = [ordered]@{ scheme = 'high_performance'; overlay_ac = 'none'; display_off_ac_s = 600 } } },
        @{ Field = 'power.overlay_ac'; Set = @{ power = [ordered]@{ scheme = 'balanced'; overlay_ac = 'best_performance'; display_off_ac_s = 600 } } },
        @{ Field = 'lock'; Set = @{ lock = [ordered]@{ screen_saver = [ordered]@{ active = $true; timeout_s = 600; secure = $false; set = $true }; dynamic_lock = $false; inactivity_s = 0 } } },
        @{ Field = 'amd_hints'; Set = @{ amd_hints = [ordered]@{ InstantReplayEnabled = 1; InGameReplayEnabled = 'absent'; DvrEnabled = 1; DesktopRecordingEnabled = 0; EnableMetricsOverlay = 'absent' } } },
        @{ Field = 'mpo'; Set = @{ mpo = 5 } },
        @{ Field = 'dx_user'; Set = @{ dx_user = [ordered]@{ VRROptimizeEnable = 1; SwapEffectUpgradeEnable = 1; AutoHDREnable = 'absent' } } },
        @{ Field = 'kairo.build_id'; Set = @{ kairo = [ordered]@{ build_id = '2026-11-21T10:10:10Z'; commit = 'abc1234' } } }
    )
    foreach ($change in $changes) {
        $c = Compare-BenchEnv -Before (New-BrEnv) -After (New-BrEnv -Set $change.Set)
        Assert-Equal $change.Field (Get-BrList $c.Comparison['changed']) $change.Field
        Assert-Equal 1 $c.ExitCode ($change.Field + ': exit code')
    }
    # Not compared: the uptime, the driver date, the display-off timer, the TDR counts.
    $after = New-BrEnv -Set @{ uptime_s = 9999; power = [ordered]@{ scheme = 'balanced'; overlay_ac = 'none'; display_off_ac_s = 0 }; tdr = [ordered]@{ events_4101_7d = 1; live_kernel_reports = 1 } }
    $after['gpus'][0]['driver_date'] = '2026-11-01'
    $c = Compare-BenchEnv -Before (New-BrEnv) -After $after
    Assert-Equal $true $c.Comparison['same'] 'uncompared fields'
    # The GPUs in another order, the lock's keys in another order, 600 written as 600.0.
    $after = New-BrEnv -Set @{ lock = [ordered]@{ inactivity_s = 0; dynamic_lock = $false; screen_saver = [ordered]@{ set = $false; secure = $false; timeout_s = 600.0; active = $false } } }
    $after['gpus'] = @($after['gpus'][1], $after['gpus'][0])
    $c = Invoke-BrCompare (New-BrEnv) $after
    Assert-Equal $true $c.Comparison['same'] 'order and number forms'
}

Test-Case 'C4 a field in one file only has changed; in neither, it is not compared' {
    $c = Invoke-BrCompare (New-BrEnv -Drop @('mpo')) (New-BrEnv -Drop @('mpo'))
    Assert-Equal $true $c.Comparison['same'] 'mpo in neither'
    Assert-Equal 'mpo,displays' (Get-BrList $c.Comparison['not_compared']) 'not_compared'
    $c = Invoke-BrCompare (New-BrEnv) (New-BrEnv -Drop @('mpo'))
    Assert-Equal 'mpo' (Get-BrList $c.Comparison['changed']) 'mpo after only'
    $c = Invoke-BrCompare (New-BrEnv) (New-BrEnv -Set @{ mpo = $null })
    Assert-Equal 'mpo' (Get-BrList $c.Comparison['changed']) 'mpo null after'
    $after = New-BrEnv
    $after['gpus'][0].Remove('driver_version')
    $c = Compare-BenchEnv -Before (New-BrEnv) -After $after
    Assert-Equal 'gpus[].driver_version' (Get-BrList $c.Comparison['changed']) 'a GPU without its driver version'
}

Test-Case 'C5 with -Bench, the displays at the start and the end' {
    $bench = New-BrBench @((New-BrBlock 'alu' 1 @((New-BrRun 'alu-01'))))
    $bench['displays_end'] = @($bench['displays_end'][1], $bench['displays_end'][0])
    $c = Invoke-BrCompare (New-BrEnv) (New-BrEnv) $bench
    Assert-Equal $true $c.Comparison['same'] 'the same displays in another order'
    Assert-Equal '' (Get-BrList $c.Comparison['not_compared']) 'not_compared'
    $bench['displays_end'][0]['size'] = '1280x720'
    $c = Compare-BenchEnv -Before (New-BrEnv) -After (New-BrEnv) -Bench $bench
    Assert-Equal $true $c.Comparison['same'] 'a size change alone is not compared'
    $bench = New-BrBench @()
    $bench['displays_end'][0]['refresh_mhz'] = 144000
    $c = Invoke-BrCompare (New-BrEnv) (New-BrEnv) $bench
    Assert-Equal 'displays' (Get-BrList $c.Comparison['changed']) 'a refresh change'
    Assert-Equal 1 $c.ExitCode 'a refresh change: exit code'
    Assert-True ($c.Json -cnotmatch '144000') 'no refresh rate in the line'
    $bench = New-BrBench @()
    $bench['displays_end'][1]['hdr'] = $true
    $c = Compare-BenchEnv -Before (New-BrEnv) -After (New-BrEnv) -Bench $bench
    Assert-Equal 'displays' (Get-BrList $c.Comparison['changed']) 'HDR turned on'
    $bench = New-BrBench @()
    $bench['displays_end'] = @($bench['displays_end'][0])
    $c = Compare-BenchEnv -Before (New-BrEnv) -After (New-BrEnv) -Bench $bench
    Assert-Equal 'displays' (Get-BrList $c.Comparison['changed']) 'a display gone'
    # A job ended by your input writes no displays_end: not compared, not a change.
    $bench = New-BrBench @()
    $bench.Remove('displays_end')
    $c = Invoke-BrCompare (New-BrEnv) (New-BrEnv) $bench
    Assert-Equal $true $c.Comparison['same'] 'no displays_end'
    Assert-Equal 'displays' (Get-BrList $c.Comparison['not_compared']) 'no displays_end: not_compared'
}

Test-Case 'C6 -CompareEnv refuses what it can''t compare' {
    $e = Assert-Throws { Compare-BenchEnv -Before '[1]' -After (ConvertTo-BrJson (New-BrEnv)) } 'a list'
    Assert-Equal 3 (Get-BenchRulesExitCode $e) 'a list: exit code'
    $e = Assert-Throws { Compare-BenchEnv -Before '{}' -After '{"uptime_s":1}' } 'no compared field'
    Assert-Equal 3 (Get-BenchRulesExitCode $e) 'no compared field: exit code'
    $e = Assert-Throws { Compare-BenchEnv -Before (New-BrEnv) -After (New-BrEnv) -Bench '{"kairo_bench":2}' } 'not a bench.json'
    Assert-Equal 3 (Get-BenchRulesExitCode $e) 'not a bench.json: exit code'
    $envFile = Write-BrFile 'c6-env.json' (ConvertTo-BrJson (New-BrEnv))
    $run = Invoke-BrTool @('-CompareEnv', '-Before', $envFile)
    Assert-BrExit 2 $run 'no -After'
    $run = Invoke-BrTool @('-CompareEnv', '-Before', $envFile, '-After', (Join-Path $script:BrDir 'gone.json'))
    Assert-BrExit 3 $run 'a missing -After file'
    Assert-Match '^bench-rules: no such file: gone\.json\s*$' $run.StdErr 'stderr names the file alone'
    $run = Invoke-BrTool @('-CompareEnv', '-Before', $envFile, '-After', $envFile, '-Stats', $envFile)
    Assert-BrExit 2 $run '-Stats with -CompareEnv'
    # A UTF-16 file with a byte-order mark, as Windows PowerShell 5.1 writes one, reads the same.
    $utf16 = Join-Path $script:BrDir 'c6-env-utf16.json'
    [IO.File]::WriteAllText($utf16, (ConvertTo-BrJson (New-BrEnv)), [Text.Encoding]::Unicode)
    $run = Invoke-BrTool @('-CompareEnv', '-Before', $envFile, '-After', $utf16)
    Assert-BrExit 0 $run 'a UTF-16 env.json'
}

# --- House rules over bench-rules.ps1 (research 17 sections 5 and 6) ------------------------------

Test-Case 'bench-rules.ps1 is ASCII and parses' {
    $file = Join-Path $BenchDir 'bench-rules.ps1'
    $bytes = [IO.File]::ReadAllBytes($file)
    for ($i = 0; $i -lt $bytes.Length; $i++) {
        if ($bytes[$i] -gt 127) { throw ('a byte above 127 at offset ' + $i) }
    }
    $tokens = $null
    $errors = $null
    [void][Management.Automation.Language.Parser]::ParseFile($file, [ref] $tokens, [ref] $errors)
    Assert-Equal 0 @($errors).Count 'parse errors'
}

Test-Case 'bench-rules.ps1 keeps the Never list and the privacy rules' {
    $file = Join-Path $BenchDir 'bench-rules.ps1'
    $tokens = $null
    $errors = $null
    $ast = [Management.Automation.Language.Parser]::ParseFile($file, [ref] $tokens, [ref] $errors)
    $problems = [Collections.Generic.List[string]]::new()
    # Commands: no Add-Type, no CSV import, no process list, no program started, no file written.
    $bannedCommands = @('Add-Type', 'Import-Csv', 'Get-Process', 'Start-Process', 'Invoke-Expression', 'Read-Host',
        'Set-Content', 'Add-Content', 'Out-File', 'Export-Csv', 'New-Item', 'Remove-Item', 'Copy-Item', 'Move-Item',
        'Rename-Item', 'Set-ItemProperty', 'New-ItemProperty', 'Get-WmiObject', 'Get-CimInstance', 'Start-Job')
    foreach ($command in $ast.FindAll({ param($n) $n -is [Management.Automation.Language.CommandAst] }, $true)) {
        $name = $command.GetCommandName()
        if ($null -ne $name -and $bannedCommands -contains $name) { $problems.Add('line ' + $command.Extent.StartLineNumber + ': ' + $name) }
    }
    # Members: none that lists a process's modules or reads its command line; no process list, no
    # SendInput; and [IO.File] or [IO.Directory] only to read.
    $moduleMembers = @('Path', 'MainModule', 'Modules', 'Company', 'FileVersion', 'ProductVersion', 'Description', 'Product', 'CommandLine')
    foreach ($member in $ast.FindAll({ param($n) $n -is [Management.Automation.Language.MemberExpressionAst] }, $true)) {
        $memberName = $member.Member.Extent.Text.Trim([char[]]@([char]39, [char]34))
        $onType = $member.Expression -is [Management.Automation.Language.TypeExpressionAst]
        if (-not $onType -and $moduleMembers -contains $memberName) { $problems.Add('line ' + $member.Extent.StartLineNumber + ': .' + $memberName) }
        if (@('GetProcesses', 'GetProcessById', 'SendInput') -contains $memberName) { $problems.Add('line ' + $member.Extent.StartLineNumber + ': ' + $memberName) }
        if ($onType -and $member.Expression.TypeName.FullName -match '^(System\.)?IO\.(File|Directory)\z' -and @('ReadAllText', 'ReadAllBytes', 'Exists') -notcontains $memberName) {
            $problems.Add('line ' + $member.Extent.StartLineNumber + ': ' + $member.Expression.TypeName.FullName + '::' + $memberName)
        }
    }
    # Dot-sourcing defines functions and nothing else: the top level holds function definitions and
    # the guarded command line, and nothing exits or prompts outside it.
    $statements = @($ast.EndBlock.Statements)
    $guard = $null
    foreach ($statement in $statements) {
        if ($statement -is [Management.Automation.Language.FunctionDefinitionAst]) { continue }
        if ($null -eq $guard -and $statement -is [Management.Automation.Language.IfStatementAst] -and $statement.Clauses[0].Item1.Extent.Text -ceq '$MyInvocation.InvocationName -ne ''.''') {
            $guard = $statement
            continue
        }
        $problems.Add('line ' + $statement.Extent.StartLineNumber + ': a top-level statement outside the guarded command line')
    }
    if ($null -eq $guard) { $problems.Add('no guarded command line') }
    foreach ($exit in $ast.FindAll({ param($n) $n -is [Management.Automation.Language.ExitStatementAst] }, $true)) {
        if ($null -eq $guard -or $exit.Extent.StartOffset -lt $guard.Extent.StartOffset -or $exit.Extent.EndOffset -gt $guard.Extent.EndOffset) {
            $problems.Add('line ' + $exit.Extent.StartLineNumber + ': exit outside the command line')
        }
    }
    foreach ($attribute in $ast.FindAll({ param($n) $n -is [Management.Automation.Language.NamedAttributeArgumentAst] }, $true)) {
        if ($attribute.ArgumentName -eq 'Mandatory') { $problems.Add('line ' + $attribute.Extent.StartLineNumber + ': a Mandatory parameter') }
    }
    if ($problems.Count -gt 0) { throw ($problems -join '; ') }
}

Test-Case 'dot-sourcing defines functions only, prints nothing and runs no command line' {
    $file = Join-Path $BenchDir 'bench-rules.ps1'
    $seen = & {
        $varsBefore = @(Get-Variable -Scope 0 | ForEach-Object { $_.Name })
        $dotOutput = @(. $file)
        $varsAfter = @(Get-Variable -Scope 0 | ForEach-Object { $_.Name })
        [pscustomobject]@{
            Output = $dotOutput.Count
            New    = @($varsAfter | Where-Object { $varsBefore -notcontains $_ -and $_ -ne 'varsBefore' -and $_ -ne 'dotOutput' })
        }
    }
    Assert-Equal 0 $seen.Output 'output'
    Assert-Equal '' ($seen.New -join ',') 'new variables'
    # Loaded as text through [scriptblock]::Create, as the elevated session does.
    $text = [IO.File]::ReadAllText($file)
    $output = @(. ([scriptblock]::Create($text)))
    Assert-Equal 0 $output.Count 'output from the text'
}

Complete-BenchTests
