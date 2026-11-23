# SPDX-License-Identifier: GPL-3.0-or-later
#
# tools/pc/bench/bench-rules.ps1 - which bench runs count, how many more runs a workload gets at the
# job's one decision point, and whether the environment changed during a session.
# Project: https://github.com/Maty3k/Kairo
# Design: docs/research/17-loadgen-bench/README.md section 4.7 (with 4.6, 4.8 and 5.4).
# Contract: docs/milestones/M1.md, Formats.
#
# Command line (each prints one JSON line):
#   pwsh -NoProfile -File bench-rules.ps1 -Bench <bench.json> -Stats <pm-stats lines> [-Encoder <encoder-usage lines>]
#   pwsh -NoProfile -File bench-rules.ps1 -Decide -Valid <n> [-AaStats <ab-stats A/A line>] [-Workload <name>]
#   pwsh -NoProfile -File bench-rules.ps1 -CompareEnv -Before <env.json> -After <env.json> [-Bench <bench.json>]
# A value is JSON text when it starts with '{', otherwise a file: bench.json and env.json hold one
# JSON object, a -Stats or -Encoder file one JSON line per run or per sampling. -Stats and -Encoder
# take one or more values. Exit codes: 0 printed (-CompareEnv: nothing changed); 1 -CompareEnv only:
# a compared field changed, so the session doesn't count (M1:217); 2 bad command line (a missing or
# unknown argument, a -Valid that isn't a whole number, an A/A line over another number of runs);
# 3 can't evaluate (a file missing, unreadable or of the wrong kind, a tag that two lines share).
#
# Validity (section 4.7). Every run bench.json lists is judged with the pm-stats line of its tag:
#   1 PresentMon exited 0 after both recording lines and wasn't stopped early; the CSV exists and
#     ends with a newline; loadgen was still running when PresentMon ended;
#   2 no ETW loss;
#   3 a pm-stats line with bad_rows 0 and swapchains 1;
#   4 |duration_s - plan.timed_s| <= 1.0;
#   5 frame_types holds only Application;
#   6 one present mode holds at least 99% of the run's frames, and it is the mode of the workload:
#     the one most of its otherwise valid runs have, extra runs included. A run with another mode
#     fails, and when two modes tie for the most, every run with either fails;
#   7 long_frames 0;   8 dropped_pct <= 1.0.
# The job sees rules 1 and 2 itself and re-runs such a slot (job_retry), except after PresentMon's
# exit 6, the access problem that stops a block. Flags are reported and never invalidate a run:
# near_refresh (avg_fps within 1% of the refresh rate, a cap), not_gpu_limited (gpu_busy_ms * avg_fps
# / 1000 < 0.9), few_frames (< 1000), sync_interval_not_0, allows_tearing_not_1, present_flags_not_512;
# and for the session, other_encoder_use: how many programs other than Kairo's (kairo*, testbed) used
# a video engine above 1% on average in the encoder-usage lines (null without them). Output:
#   {"bench_rules":1,"mode":"validity","run_id":"bench-20261123-101500","timed_s":60,"refresh_mhz":180000,
#    "workloads":[{"workload":"alu","runs":6,"valid":6,"valid_tags":["alu-01",..],
#      "present_mode":"Hardware: Independent Flip","present_mode_share_pct":100.0,"flags":[],
#      "results":[{"tag":"alu-01","n":1,"pass":1,"valid":true,"failed":[],"reasons":[],"job_retry":false,
#        "present_mode":"Hardware: Independent Flip","present_mode_frames":7500,"frames":7500,
#        "present_mode_share_pct":100.0,"flags":[]},..]}],"session_flags":{"other_encoder_use":0}}
# A workload's share and flags are over its valid runs. A run on the elevated fallback's route
# (section 4.2, "route":"elevated_stdout") can't be evaluated yet: its rule 1 is built with it.
#
# -Decide (section 4.7, at the job's one decision point), for one workload:
#   6 valid runs and a CV of 1.00% or less: none; 6 valid runs and a CV above 1.00%: 6 more, the
#   extension (n = 12); 3 to 5 valid runs: 6 - valid make-up runs, noting a CV above 1.00%; fewer
#   than 3: none, and "insufficient data: fix the setup". The CV comes from ab-stats' A/A line over
#   the same valid runs, needed at 6 or more; -Valid from this script's own "valid". Output:
#   {"bench_rules":1,"mode":"decide","workload":"alu","valid":6,"cv_pct":0.59..,"cv_high":false,
#    "extra":0,"kind":"none","note":"..."}   kind: none, extension, make_up or insufficient.
#
# -CompareEnv (sections 4.6 and 5.4): env.json's windows.build, reboot_pending, gpus[].driver_version,
# amd_software.version, hags, power.scheme, power.overlay_ac, lock, amd_hints, mpo, dx_user and
# kairo.build_id, before and after the session; with -Bench, the displays at its start and end
# (bench.json's displays_start and displays_end: each one's role, refresh_mhz and hdr). Lists compare
# as sets, objects field by field, numbers by value. A field in one file only has changed; one in
# neither, or displays without both lists, is "not_compared". Output, names only, never values:
#   {"bench_rules":1,"mode":"compare_env","same":false,"changed":["gpus[].driver_version"],"not_compared":[]}
#
# As a library (dot-sourced; the elevated session does this with the text it received on stdin):
#   $r = Get-BenchRules -Bench $benchText -Stats $pmStatsLines -EncoderUsage $encoderLines   # $r.Rules
#   $d = Get-BenchDecision -Valid 4 -AaStats $abStatsAaLine -Workload 'alu'                 # $d.Decision
#   $c = Compare-BenchEnv -Before $envBefore -After $envAfter -Bench $benchText              # $c.Comparison
#   Each also has .Json, the line, and .ExitCode. Inputs may be JSON text, parsed objects or
#   dictionaries (pm-stats' .Stats). Test-BenchRun and Test-BenchWorkload judge single runs.
# Functions only return or throw. A thrown exception carries its exit code in .Data['KairoExit'].
# Dot-sourcing defines functions and nothing else; the command line at the bottom runs only when
# the file itself is run, and it never prompts.
#
# Rules: ASCII only; Windows PowerShell 5.1 syntax and .NET Framework 4.x APIs (it runs on
# PowerShell 7); no Add-Type; clean under Set-StrictMode -Version 3.0. It reads, and writes no file.
# Output names no folder, file, program or PC detail: tags, workload names, rule numbers and
# PresentMon's present-mode names only. Errors name a file by its name alone.

function New-BenchRulesError([int] $Code, [string] $Message) {
    $exception = [Exception]::new($Message)
    $exception.Data['KairoExit'] = $Code
    return $exception
}

function Get-BenchRulesKairoException($ErrorRecord) {
    # The exception that carries an exit code, the error's own or one it wraps; $null for a bug.
    $exception = $ErrorRecord.Exception
    while ($null -ne $exception) {
        if ($exception.Data.Contains('KairoExit')) { return $exception }
        $exception = $exception.InnerException
    }
    return $null
}

function Get-BenchRulesExitCode($ErrorRecord) {
    # The exit code a caught error stands for: its own, or 3 (can't evaluate).
    $exception = Get-BenchRulesKairoException $ErrorRecord
    if ($null -ne $exception) { return [int]$exception.Data['KairoExit'] }
    return 3
}

# --- JSON values ------------------------------------------------------------------------------

function Test-BenchRulesObject($Value) {
    # A JSON object: parsed by ConvertFrom-Json, or a dictionary such as pm-stats' .Stats.
    return ($Value -is [Collections.IDictionary] -or $Value -is [Management.Automation.PSCustomObject])
}

function Test-BenchRulesList($Value) {
    return ($Value -is [Collections.IList] -and -not (Test-BenchRulesObject $Value))
}

function Get-BenchRulesKeys($Object) {
    # A JSON object's keys, in its own order.
    $keys = [Collections.Generic.List[string]]::new()
    if ($Object -is [Collections.IDictionary]) {
        foreach ($key in $Object.Keys) { $keys.Add([string]$key) }
    } elseif ($Object -is [Management.Automation.PSCustomObject]) {
        foreach ($property in $Object.PSObject.Properties) { $keys.Add($property.Name) }
    }
    return ,$keys
}

function Test-BenchRulesField($Object, [string] $Name) {
    # Whether a JSON object has the key, compared exactly as JSON compares keys.
    foreach ($key in (Get-BenchRulesKeys $Object)) {
        if ([string]::Equals($key, $Name, [StringComparison]::Ordinal)) { return $true }
    }
    return $false
}

function Get-BenchRulesField($Object, [string] $Name) {
    # A field of a JSON object, a list kept whole; $null when it is missing.
    if ($Object -is [Collections.IDictionary]) {
        foreach ($key in $Object.Keys) {
            if ($key -is [string] -and [string]::Equals($key, $Name, [StringComparison]::Ordinal)) { return ,$Object[$key] }
        }
        return $null
    }
    if ($Object -is [Management.Automation.PSCustomObject]) {
        foreach ($property in $Object.PSObject.Properties) {
            if ([string]::Equals($property.Name, $Name, [StringComparison]::Ordinal)) { return ,$property.Value }
        }
    }
    return $null
}

function ConvertTo-BenchRulesNumber($Value) {
    # A JSON number as a double (Windows PowerShell 5.1 parses ints and decimals, PowerShell 7 longs
    # and doubles); $null for anything else, a bool or a numeric string included.
    if ($null -eq $Value -or $Value -isnot [ValueType] -or $Value -is [bool] -or $Value -is [char] -or $Value -is [Enum]) { return $null }
    $code = [string][Type]::GetTypeCode($Value.GetType())
    if (@('Byte', 'SByte', 'Int16', 'UInt16', 'Int32', 'UInt32', 'Int64', 'UInt64', 'Single', 'Double', 'Decimal') -notcontains $code) { return $null }
    $number = [double]$Value
    if ([double]::IsNaN($number) -or [double]::IsInfinity($number)) { return $null }
    return $number
}

function ConvertTo-BenchRulesInteger($Value) {
    # A JSON number that is whole, as a long; $null otherwise.
    $number = ConvertTo-BenchRulesNumber $Value
    if ($null -eq $number -or [Math]::Floor($number) -ne $number -or [Math]::Abs($number) -gt 9007199254740992.0) { return $null }
    return [long]$number
}

function ConvertFrom-BenchRulesJson($Value, [string] $What) {
    # One JSON object: text is parsed, and a parsed object or a dictionary is taken as it is.
    if ($Value -is [string]) {
        $text = $Value.Trim([char[]]@([char]0xFEFF, ' ', [char]9, [char]10, [char]13))
        if ($text.Length -eq 0 -or $text[0] -ne [char]'{') { throw (New-BenchRulesError 3 ($What + ' is not a JSON object')) }
        $parsed = $null
        try {
            $parsed = ConvertFrom-Json -InputObject $text -ErrorAction Stop
        } catch {
            throw (New-BenchRulesError 3 ($What + ' is not valid JSON'))
        }
        $Value = $parsed
    }
    if (-not (Test-BenchRulesObject $Value)) { throw (New-BenchRulesError 3 ($What + ' is not a JSON object')) }
    return $Value
}

function Get-BenchRulesCounts($Value) {
    # A pm-stats count object, {"name": count, ...}, as Key/Count pairs in its own order; $null when
    # it isn't one (pm-stats writes null for a column the CSV lacks).
    if (-not (Test-BenchRulesObject $Value)) { return $null }
    $list = [Collections.Generic.List[object]]::new()
    foreach ($key in (Get-BenchRulesKeys $Value)) {
        $count = ConvertTo-BenchRulesInteger (Get-BenchRulesField $Value $key)
        if ($null -eq $count -or $count -lt 0) { return $null }
        $list.Add([pscustomobject]@{ Key = $key; Count = $count })
    }
    return ,$list
}

function Format-BenchRulesNumber($Value) {
    if ($null -eq $Value) { return 'none' }
    return ([double]$Value).ToString('R', [Globalization.CultureInfo]::InvariantCulture)
}

function Get-BenchRulesRounded([double] $Value, [int] $Digits) {
    # AwayFromZero, as the other bench scripts round; -0 prints as 0.
    $rounded = [Math]::Round($Value, $Digits, [MidpointRounding]::AwayFromZero)
    if ($rounded -eq 0) { $rounded = 0.0 }
    return $rounded
}

function Test-BenchRulesTag($Value) {
    # A run's tag, as pm-stats takes it, so no path or PC detail can ride in one.
    return ($Value -is [string] -and $Value -cmatch '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}\z')
}

function Test-BenchRulesWorkload($Value) {
    return ($Value -is [string] -and $Value -cmatch '^[A-Za-z0-9][A-Za-z0-9_-]{0,31}\z')
}

function Test-BenchRulesKairoName($Name) {
    # Kairo's own programs, as pc deploy names them (kairo*.exe, testbed.exe): encoder-usage gives
    # process names, without .exe.
    return ($Name -is [string] -and $Name -match '^(kairo[A-Za-z0-9._-]*|testbed)(\.exe)?\z')
}

function Get-BenchRulesFlagNames {
    # The run flags, in the order they are reported.
    return ,[string[]]@('near_refresh', 'not_gpu_limited', 'few_frames', 'sync_interval_not_0', 'allows_tearing_not_1', 'present_flags_not_512')
}

# --- Validity (section 4.7) -------------------------------------------------------------------

function Get-BenchRulesBool($Run, [string] $Name) {
    # A recorded true or false; $null when the field is missing or isn't a bool.
    $value = Get-BenchRulesField $Run $Name
    if ($value -is [bool]) { return $value }
    return $null
}

function Test-BenchRunRecord($Run) {
    # Rule 1's facts, from the run's record in bench.json; the problems found, in order.
    $problems = [Collections.Generic.List[string]]::new()
    $pmExit = ConvertTo-BenchRulesInteger (Get-BenchRulesField $Run 'pm_exit')
    if ($null -eq $pmExit) { $problems.Add('no PresentMon exit code') }
    elseif ($pmExit -ne 0) { $problems.Add('PresentMon exit ' + $pmExit) }
    $started = Get-BenchRulesBool $Run 'pm_started'
    if ($null -eq $started) { $problems.Add('pm_started not recorded') }
    elseif (-not $started) { $problems.Add('no Started recording. line') }
    $stopped = Get-BenchRulesBool $Run 'pm_stopped'
    if ($null -eq $stopped) { $problems.Add('pm_stopped not recorded') }
    elseif (-not $stopped) { $problems.Add('no Stopped recording. line') }
    $early = Get-BenchRulesBool $Run 'stopped_early'
    if ($null -eq $early) { $problems.Add('stopped_early not recorded') }
    elseif ($early) { $problems.Add('stopped early') }
    $csv = Get-BenchRulesField $Run 'csv'
    $csvBytes = ConvertTo-BenchRulesInteger (Get-BenchRulesField $Run 'csv_bytes')
    if (-not ($csv -is [string] -and $csv.Length -gt 0) -or $null -eq $csvBytes -or $csvBytes -le 0) {
        $problems.Add('no CSV')
    } else {
        $newline = Get-BenchRulesBool $Run 'csv_ends_newline'
        if ($null -eq $newline) { $problems.Add('csv_ends_newline not recorded') }
        elseif (-not $newline) { $problems.Add('the CSV does not end with a newline (cut)') }
    }
    $alive = Get-BenchRulesBool $Run 'loadgen_alive_at_pm_end'
    if ($null -eq $alive) { $problems.Add('loadgen_alive_at_pm_end not recorded') }
    elseif (-not $alive) { $problems.Add('loadgen ended before PresentMon') }
    return ,$problems
}

function Add-BenchRulesFailure($Result, [int] $Rule, [string] $Reason) {
    # Marks a run invalid for a rule, keeping the rule numbers in order.
    $failed = $Result['failed']
    if (-not $failed.Contains($Rule)) {
        $failed.Add($Rule)
        $failed.Sort()
    }
    $Result['reasons'].Add($Reason)
    $Result['valid'] = $false
}

function Test-BenchRun {
    # One run against section 4.7's rules 1 to 8 and its flags. $Run is the run's record in
    # bench.json; $Stats its pm-stats line (text, parsed, or pm-stats' .Stats), or $null when
    # pm-stats gave none; $TimedS the plan's timed_s; $RefreshMhz the monitor's refresh rate, or
    # $null. Rule 6's second half, one present mode across the workload, is Test-BenchWorkload's.
    param(
        $Run,
        $Stats = $null,
        $TimedS = $null,
        $RefreshMhz = $null,
        $Pass = $null
    )
    $timed = ConvertTo-BenchRulesNumber $TimedS
    if ($null -eq $timed -or $timed -le 0) { throw (New-BenchRulesError 2 '-TimedS must be a positive number of seconds') }
    if (-not (Test-BenchRulesObject $Run)) { throw (New-BenchRulesError 3 'a run that is not a JSON object') }
    $tag = Get-BenchRulesField $Run 'tag'
    if (-not (Test-BenchRulesTag $tag)) { throw (New-BenchRulesError 3 'a run without a valid tag') }
    $route = Get-BenchRulesField $Run 'route'
    if ($null -ne $route -and -not ($route -is [string] -and $route -ceq 'job')) {
        if ($route -is [string] -and $route -ceq 'elevated_stdout') {
            throw (New-BenchRulesError 3 ('run ' + $tag + ': rule 1 for the elevated fallback (route elevated_stdout, research 17 section 4.2) is not built yet'))
        }
        throw (New-BenchRulesError 3 ('run ' + $tag + ': an unknown route'))
    }
    $statsLine = $null
    if ($null -ne $Stats) {
        $statsLine = ConvertFrom-BenchRulesJson $Stats 'a pm-stats line'
        if ((ConvertTo-BenchRulesInteger (Get-BenchRulesField $statsLine 'pm_stats')) -ne 1) { throw (New-BenchRulesError 3 'not a pm-stats line (pm_stats 1)') }
        $statsTag = Get-BenchRulesField $statsLine 'tag'
        if (-not ($statsTag -is [string] -and $statsTag -ceq $tag)) { throw (New-BenchRulesError 3 ('the pm-stats line given for run ' + $tag + ' has another tag')) }
    }

    $result = [ordered]@{}
    $result['tag'] = $tag
    $result['n'] = ConvertTo-BenchRulesInteger (Get-BenchRulesField $Run 'n')
    $result['pass'] = ConvertTo-BenchRulesInteger $Pass
    $result['valid'] = $true
    $result['failed'] = [Collections.Generic.List[int]]::new()
    $result['reasons'] = [Collections.Generic.List[string]]::new()
    $result['job_retry'] = $false
    $result['present_mode'] = $null
    $result['present_mode_frames'] = $null
    $result['frames'] = $null
    $result['present_mode_share_pct'] = $null
    $result['flags'] = [Collections.Generic.List[string]]::new()

    # Rule 1: what the job saw of PresentMon, the CSV and loadgen.
    $problems = Test-BenchRunRecord $Run
    if ($problems.Count -gt 0) { Add-BenchRulesFailure $result 1 ('rule 1: ' + ($problems -join '; ')) }
    # Rule 2: no ETW loss.
    $loss = Get-BenchRulesBool $Run 'etw_loss'
    if ($null -eq $loss) { Add-BenchRulesFailure $result 2 'rule 2: etw_loss not recorded' }
    elseif ($loss) { Add-BenchRulesFailure $result 2 'rule 2: ETW events or buffers were lost' }
    # The job re-runs a slot that fails rule 1 or 2, except after PresentMon's exit 6 (section 4.9).
    $pmExit = ConvertTo-BenchRulesInteger (Get-BenchRulesField $Run 'pm_exit')
    $result['job_retry'] = ($result['failed'].Count -gt 0 -and $pmExit -ne 6)

    if ($null -eq $statsLine) {
        Add-BenchRulesFailure $result 3 'rule 3: no pm-stats line'
        return $result
    }
    # Rule 3: pm-stats read every row, and the run had one swap chain.
    $badRows = ConvertTo-BenchRulesInteger (Get-BenchRulesField $statsLine 'bad_rows')
    $chains = ConvertTo-BenchRulesInteger (Get-BenchRulesField $statsLine 'swapchains')
    if ($null -eq $badRows -or $badRows -ne 0 -or $null -eq $chains -or $chains -ne 1) {
        Add-BenchRulesFailure $result 3 ('rule 3: bad_rows ' + (Format-BenchRulesNumber $badRows) + ', swapchains ' + (Format-BenchRulesNumber $chains))
    }
    # Rule 4: the recording holds the timed length within 1 s.
    $duration = ConvertTo-BenchRulesNumber (Get-BenchRulesField $statsLine 'duration_s')
    if ($null -eq $duration -or [Math]::Abs($duration - $timed) -gt 1.0) {
        Add-BenchRulesFailure $result 4 ('rule 4: duration_s ' + (Format-BenchRulesNumber $duration) + ' at timed ' + (Format-BenchRulesNumber $timed))
    }
    # Rule 5: every frame is an Application frame (AMD AFMF means AFMF is on).
    $types = Get-BenchRulesCounts (Get-BenchRulesField $statsLine 'frame_types')
    if ($null -eq $types -or $types.Count -eq 0) {
        Add-BenchRulesFailure $result 5 'rule 5: no frame types recorded'
    } else {
        $others = [Collections.Generic.List[string]]::new()
        foreach ($entry in $types) { if ($entry.Key -cne 'Application') { $others.Add($entry.Key) } }
        if ($others.Count -gt 0) { Add-BenchRulesFailure $result 5 ('rule 5: frame types other than Application: ' + ($others -join ', ')) }
    }
    # Rule 6, first half: one present mode holds at least 99% of the frames.
    $modes = Get-BenchRulesCounts (Get-BenchRulesField $statsLine 'present_modes')
    $top = $null
    $total = [long]0
    if ($null -ne $modes) {
        foreach ($entry in $modes) {
            $total += $entry.Count
            if ($null -eq $top -or $entry.Count -gt $top.Count) { $top = $entry }
        }
    }
    if ($null -eq $top -or $total -le 0) {
        Add-BenchRulesFailure $result 6 'rule 6: no present mode recorded'
    } else {
        $share = Get-BenchRulesRounded (100.0 * $top.Count / $total) 2
        $result['present_mode'] = $top.Key
        $result['present_mode_frames'] = $top.Count
        $result['frames'] = $total
        $result['present_mode_share_pct'] = $share
        if (100 * $top.Count -lt 99 * $total) {
            Add-BenchRulesFailure $result 6 ('rule 6: the most common present mode holds ' + (Format-BenchRulesNumber $share) + '% of frames, below 99%')
        }
    }
    # Rule 7: no frame over 250 ms (a stall stays in the CSV as one long frame).
    $long = ConvertTo-BenchRulesInteger (Get-BenchRulesField $statsLine 'long_frames')
    if ($null -eq $long -or $long -ne 0) { Add-BenchRulesFailure $result 7 ('rule 7: long_frames ' + (Format-BenchRulesNumber $long)) }
    # Rule 8: at most 1% of presents dropped.
    $dropped = ConvertTo-BenchRulesNumber (Get-BenchRulesField $statsLine 'dropped_pct')
    if ($null -eq $dropped -or $dropped -gt 1.0) { Add-BenchRulesFailure $result 8 ('rule 8: dropped_pct ' + (Format-BenchRulesNumber $dropped)) }

    # Flags: reported, never invalidating.
    $flags = $result['flags']
    $fps = ConvertTo-BenchRulesNumber (Get-BenchRulesField $statsLine 'avg_fps')
    $refresh = ConvertTo-BenchRulesNumber $RefreshMhz
    if ($null -ne $fps -and $null -ne $refresh -and $refresh -gt 0) {
        $hz = $refresh / 1000.0
        if ([Math]::Abs($fps - $hz) -le 0.01 * $hz) { $flags.Add('near_refresh') }
    }
    $busy = ConvertTo-BenchRulesNumber (Get-BenchRulesField $statsLine 'gpu_busy_ms')
    if ($null -ne $fps -and $null -ne $busy -and ($busy * $fps / 1000.0) -lt 0.9) { $flags.Add('not_gpu_limited') }
    $frames = ConvertTo-BenchRulesInteger (Get-BenchRulesField $statsLine 'frames')
    if ($null -ne $frames -and $frames -lt 1000) { $flags.Add('few_frames') }
    foreach ($check in @(@('sync_interval', '0', 'sync_interval_not_0'), @('allows_tearing', '1', 'allows_tearing_not_1'), @('present_flags', '512', 'present_flags_not_512'))) {
        $counts = Get-BenchRulesCounts (Get-BenchRulesField $statsLine $check[0])
        if ($null -eq $counts) { continue }
        foreach ($entry in $counts) {
            if ($entry.Key -cne $check[1] -and $entry.Count -gt 0) {
                $flags.Add($check[2])
                break
            }
        }
    }
    return $result
}

function Test-BenchWorkload {
    # Rule 6's second half: the same present mode in every valid run of the workload in the
    # session, extra runs included. The workload's mode is the one most of its otherwise valid
    # runs have; a valid run with another fails rule 6, and when two or more modes tie for the
    # most, no mode is the workload's and every valid run fails it. $Results are Test-BenchRun's.
    param(
        [string] $Workload,
        [object[]] $Results = @()
    )
    if ($null -eq $Results) { $Results = @() }
    $votes = [Collections.Generic.Dictionary[string, int]]::new([StringComparer]::Ordinal)
    $order = [Collections.Generic.List[string]]::new()
    foreach ($result in $Results) {
        if (-not $result['valid'] -or $result['present_mode'] -isnot [string]) { continue }
        $mode = [string]$result['present_mode']
        if ($votes.ContainsKey($mode)) { $votes[$mode] = $votes[$mode] + 1 } else { $votes[$mode] = 1; $order.Add($mode) }
    }
    $most = 0
    foreach ($mode in $order) { if ($votes[$mode] -gt $most) { $most = $votes[$mode] } }
    $leaders = [Collections.Generic.List[string]]::new()
    foreach ($mode in $order) { if ($votes[$mode] -eq $most) { $leaders.Add($mode) } }
    $sessionMode = $null
    if ($leaders.Count -eq 1) { $sessionMode = $leaders[0] }
    foreach ($result in $Results) {
        if (-not $result['valid']) { continue }
        if ($null -eq $sessionMode) {
            Add-BenchRulesFailure $result 6 ('rule 6: the valid runs split evenly between present modes: ' + ($leaders -join ', '))
        } elseif ([string]$result['present_mode'] -cne $sessionMode) {
            Add-BenchRulesFailure $result 6 ('rule 6: present mode ' + [string]$result['present_mode'] + ', where most valid runs have ' + $sessionMode)
        }
    }

    $validTags = [Collections.Generic.List[string]]::new()
    $modeFrames = [long]0
    $allFrames = [long]0
    $seenFlags = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($result in $Results) {
        if (-not $result['valid']) { continue }
        $validTags.Add([string]$result['tag'])
        $modeFrames += [long]$result['present_mode_frames']
        $allFrames += [long]$result['frames']
        foreach ($flag in $result['flags']) { [void]$seenFlags.Add($flag) }
    }
    $flags = [Collections.Generic.List[string]]::new()
    foreach ($flag in (Get-BenchRulesFlagNames)) { if ($seenFlags.Contains($flag)) { $flags.Add($flag) } }
    $share = $null
    if ($allFrames -gt 0) { $share = Get-BenchRulesRounded (100.0 * $modeFrames / $allFrames) 2 }
    $resultList = [Collections.Generic.List[object]]::new()
    foreach ($result in $Results) { $resultList.Add($result) }

    $out = [ordered]@{}
    $out['workload'] = $Workload
    $out['runs'] = $resultList.Count
    $out['valid'] = $validTags.Count
    $out['valid_tags'] = $validTags
    $out['present_mode'] = $sessionMode
    $out['present_mode_share_pct'] = $share
    $out['flags'] = $flags
    $out['results'] = $resultList
    return $out
}

function Get-BenchOtherEncoderUse {
    # Section 4.7's session flag: how many programs other than Kairo's used a video engine above
    # 1% on average in the encoder-usage lines given (taken before and after the job), such as a
    # recorder; $null when none is given. A count only: other programs' names stay in this function.
    param([object[]] $EncoderUsage = $null)
    if ($null -eq $EncoderUsage -or $EncoderUsage.Count -eq 0) { return $null }
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($item in $EncoderUsage) {
        $usage = ConvertFrom-BenchRulesJson $item 'an encoder-usage line'
        if ((ConvertTo-BenchRulesInteger (Get-BenchRulesField $usage 'kairo_encoder_usage')) -ne 1) {
            throw (New-BenchRulesError 3 'not an encoder-usage line (kairo_encoder_usage 1)')
        }
        $video = Get-BenchRulesField $usage 'video'
        if ($null -eq $video) { continue }
        if (-not (Test-BenchRulesList $video)) { throw (New-BenchRulesError 3 'an encoder-usage line whose video is not a list') }
        foreach ($row in $video) {
            if (-not (Test-BenchRulesObject $row)) { throw (New-BenchRulesError 3 'an encoder-usage row that is not a JSON object') }
            $average = ConvertTo-BenchRulesNumber (Get-BenchRulesField $row 'avg_pct')
            if ($null -eq $average) { throw (New-BenchRulesError 3 'an encoder-usage row without avg_pct') }
            if ($average -le 1.0) { continue }
            $name = Get-BenchRulesField $row 'name'
            if (Test-BenchRulesKairoName $name) { continue }
            # A program is its name; one that exited before encoder-usage named it is its process id.
            if ($name -is [string] -and $name.Length -gt 0 -and $name -cne '(exited)') {
                [void]$seen.Add('name:' + $name)
            } else {
                [void]$seen.Add('pid:' + (Format-BenchRulesNumber (ConvertTo-BenchRulesInteger (Get-BenchRulesField $row 'pid'))))
            }
        }
    }
    return $seen.Count
}

function Get-BenchRules {
    # Every run bench.json lists, judged by section 4.7's rules with the pm-stats line of its tag,
    # grouped by workload across both passes; with encoder-usage lines, the session's flag too.
    param(
        $Bench,
        [object[]] $Stats = @(),
        [object[]] $EncoderUsage = $null
    )
    $benchObject = ConvertFrom-BenchRulesJson $Bench 'bench.json'
    if ((ConvertTo-BenchRulesInteger (Get-BenchRulesField $benchObject 'kairo_bench')) -ne 1) { throw (New-BenchRulesError 3 'not a bench.json (kairo_bench 1)') }
    $runId = Get-BenchRulesField $benchObject 'run_id'
    if (-not ($runId -is [string] -and $runId -cmatch '^bench-[0-9]{8}-[0-9]{6}\z')) { throw (New-BenchRulesError 3 'bench.json has no run_id of the form bench-yyyyMMdd-HHmmss') }
    $timed = ConvertTo-BenchRulesNumber (Get-BenchRulesField (Get-BenchRulesField $benchObject 'plan') 'timed_s')
    if ($null -eq $timed -or $timed -le 0) { throw (New-BenchRulesError 3 'bench.json has no plan.timed_s') }
    $refresh = ConvertTo-BenchRulesNumber (Get-BenchRulesField (Get-BenchRulesField $benchObject 'monitor') 'refresh_mhz')
    if ($null -ne $refresh -and $refresh -le 0) { $refresh = $null }

    $byTag = [Collections.Generic.Dictionary[string, object]]::new([StringComparer]::Ordinal)
    if ($null -ne $Stats) {
        foreach ($item in $Stats) {
            if ($null -eq $item) { continue }
            $line = ConvertFrom-BenchRulesJson $item 'a pm-stats line'
            if ((ConvertTo-BenchRulesInteger (Get-BenchRulesField $line 'pm_stats')) -ne 1) { throw (New-BenchRulesError 3 'not a pm-stats line (pm_stats 1)') }
            $tag = Get-BenchRulesField $line 'tag'
            if (-not (Test-BenchRulesTag $tag)) { throw (New-BenchRulesError 3 'a pm-stats line without a valid tag') }
            if ($byTag.ContainsKey($tag)) { throw (New-BenchRulesError 3 ('two pm-stats lines for ' + $tag)) }
            $byTag[$tag] = $line
        }
    }

    $blocks = Get-BenchRulesField $benchObject 'blocks'
    if (-not (Test-BenchRulesList $blocks)) { throw (New-BenchRulesError 3 'bench.json has no blocks list') }
    $groups = [Collections.Specialized.OrderedDictionary]::new([StringComparer]::Ordinal)
    $runTags = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($block in $blocks) {
        if (-not (Test-BenchRulesObject $block)) { throw (New-BenchRulesError 3 'a block that is not a JSON object') }
        $workload = Get-BenchRulesField $block 'workload'
        if (-not (Test-BenchRulesWorkload $workload)) { throw (New-BenchRulesError 3 'a block without a valid workload name') }
        if (-not $groups.Contains($workload)) { $groups.Add($workload, [Collections.Generic.List[object]]::new()) }
        $pass = Get-BenchRulesField $block 'pass'
        $runs = Get-BenchRulesField $block 'runs'
        if ($null -eq $runs) { continue }
        if (-not (Test-BenchRulesList $runs)) { throw (New-BenchRulesError 3 ('a ' + $workload + ' block whose runs are not a list')) }
        foreach ($run in $runs) {
            if (-not (Test-BenchRulesObject $run)) { throw (New-BenchRulesError 3 'a run that is not a JSON object') }
            $tag = Get-BenchRulesField $run 'tag'
            if (-not (Test-BenchRulesTag $tag)) { throw (New-BenchRulesError 3 'a run without a valid tag') }
            if (-not $runTags.Add($tag)) { throw (New-BenchRulesError 3 ('two runs tagged ' + $tag)) }
            $runStats = $null
            if ($byTag.ContainsKey($tag)) { $runStats = $byTag[$tag] }
            $groups[$workload].Add((Test-BenchRun -Run $run -Stats $runStats -TimedS $timed -RefreshMhz $refresh -Pass $pass))
        }
    }
    foreach ($tag in $byTag.Keys) {
        if (-not $runTags.Contains($tag)) { throw (New-BenchRulesError 3 ('a pm-stats line for ' + $tag + ', which bench.json does not list')) }
    }

    $workloads = [Collections.Generic.List[object]]::new()
    foreach ($workload in $groups.Keys) {
        $workloads.Add((Test-BenchWorkload -Workload $workload -Results $groups[$workload].ToArray()))
    }
    $sessionFlags = [ordered]@{}
    $sessionFlags['other_encoder_use'] = Get-BenchOtherEncoderUse -EncoderUsage $EncoderUsage

    $out = [ordered]@{}
    $out['bench_rules'] = 1
    $out['mode'] = 'validity'
    $out['run_id'] = $runId
    $out['timed_s'] = $timed
    $out['refresh_mhz'] = $refresh
    $out['workloads'] = $workloads
    $out['session_flags'] = $sessionFlags
    return [pscustomobject]@{ Rules = $out; Json = (ConvertTo-Json -InputObject $out -Compress -Depth 10); ExitCode = 0 }
}

# --- Extra runs at the decision point (section 4.7) ------------------------------------------

function Get-BenchDecision {
    # The extra runs one workload gets at the job's one decision point. $Valid is the workload's
    # valid runs (Get-BenchRules' "valid"); $AaStats ab-stats' A/A line over those same runs, whose
    # cv_pct (sample SD, n - 1) decides at 6 valid runs or more and is noted below that.
    param(
        $Valid,
        $AaStats = $null,
        [string] $Workload = ''
    )
    $inv = [Globalization.CultureInfo]::InvariantCulture
    if ($null -eq $Valid -or ([string]$Valid).Trim() -cnotmatch '^[0-9]{1,4}\z') { throw (New-BenchRulesError 2 '-Valid must be a whole number of runs') }
    $n = [int](([string]$Valid).Trim())
    if ($Workload.Length -gt 0 -and -not (Test-BenchRulesWorkload $Workload)) { throw (New-BenchRulesError 2 'bad -Workload: letters, digits, underscore and hyphen, at most 32') }
    $cv = $null
    if ($null -ne $AaStats) {
        $aa = ConvertFrom-BenchRulesJson $AaStats 'the ab-stats line'
        $mode = Get-BenchRulesField $aa 'mode'
        if ((ConvertTo-BenchRulesInteger (Get-BenchRulesField $aa 'ab_stats')) -ne 1 -or -not ($mode -is [string] -and $mode -ceq 'aa')) {
            throw (New-BenchRulesError 3 'not an ab-stats A/A line (ab_stats 1, mode aa)')
        }
        $aaN = ConvertTo-BenchRulesInteger (Get-BenchRulesField $aa 'n')
        if ($null -eq $aaN -or $aaN -ne $n) {
            throw (New-BenchRulesError 2 ('the ab-stats line covers ' + (Format-BenchRulesNumber $aaN) + ' runs, not the ' + $n + ' valid ones'))
        }
        $cv = ConvertTo-BenchRulesNumber (Get-BenchRulesField $aa 'cv_pct')
        if ($null -eq $cv -or $cv -lt 0) { throw (New-BenchRulesError 3 'the ab-stats line has no cv_pct') }
        $noiseOk = Get-BenchRulesField $aa 'noise_ok'
        if ($noiseOk -is [bool] -and $noiseOk -ne ($cv -le 1.0)) { throw (New-BenchRulesError 3 'the ab-stats line has a noise_ok that disagrees with its cv_pct') }
    }
    $cvHigh = $null
    $cvText = 'unknown'
    if ($null -ne $cv) {
        $cvHigh = ($cv -gt 1.0)
        $cvText = (Get-BenchRulesRounded $cv 2).ToString('F2', $inv) + '%'
    }
    $runsText = $n.ToString($inv) + ' valid runs'
    if ($n -ge 6) {
        if ($null -eq $cv) { throw (New-BenchRulesError 2 (([string]$n) + ' valid runs need -AaStats, the A/A line that gives their CV')) }
        if ($cvHigh) {
            $extra = 6
            $kind = 'extension'
            $note = 'CV ' + $cvText + ' over ' + $runsText + ' is above 1.00%: 6 more runs, then the CV over all of them'
        } else {
            $extra = 0
            $kind = 'none'
            $note = 'CV ' + $cvText + ' over ' + $runsText + ' is 1.00% or less: no extra runs'
        }
    } elseif ($n -ge 3) {
        $extra = 6 - $n
        $kind = 'make_up'
        $note = $extra.ToString($inv) + ' make-up runs to reach 6 valid runs'
        if ($cvHigh) {
            $note += '; CV ' + $cvText + ' over the ' + $runsText + ' is above 1.00%: if the 6 stay above 1%, the report says so and a later session extends the workload'
        }
    } else {
        $extra = 0
        $kind = 'insufficient'
        $note = 'insufficient data: fix the setup (' + $runsText + '; the rules name what failed)'
    }
    $out = [ordered]@{}
    $out['bench_rules'] = 1
    $out['mode'] = 'decide'
    $out['workload'] = $null
    if ($Workload.Length -gt 0) { $out['workload'] = $Workload }
    $out['valid'] = $n
    $out['cv_pct'] = $cv
    $out['cv_high'] = $cvHigh
    $out['extra'] = $extra
    $out['kind'] = $kind
    $out['note'] = $note
    return [pscustomobject]@{ Decision = $out; Json = (ConvertTo-Json -InputObject $out -Compress -Depth 5); ExitCode = 0 }
}

# --- The environment before and after (sections 4.6 and 5.4) ---------------------------------

function Get-BenchEnvFields {
    # env.json's fields compared before and after a session. a.b is a field of an object; list[].x
    # is the x of each element of list, the elements compared as a set.
    return ,[string[]]@('windows.build', 'reboot_pending', 'gpus[].driver_version', 'amd_software.version', 'hags',
        'power.scheme', 'power.overlay_ac', 'lock', 'amd_hints', 'mpo', 'dx_user', 'kairo.build_id')
}

function ConvertTo-BenchRulesCanonical($Value) {
    # A parsed JSON value as text that is equal exactly when the values are: an object's keys in
    # ordinal order, numbers by value (5.1 and 7 parse 60 and 60.0 into different types).
    if ($null -eq $Value) { return 'null' }
    if ($Value -is [bool]) {
        if ($Value) { return 'true' }
        return 'false'
    }
    if ($Value -is [string]) { return '"' + $Value.Replace('\', '\\').Replace('"', '\"') + '"' }
    $number = ConvertTo-BenchRulesNumber $Value
    if ($null -ne $number) { return $number.ToString('R', [Globalization.CultureInfo]::InvariantCulture) }
    # PowerShell 7 turns ISO 8601 strings into dates; both sides go through the same conversion.
    if ($Value -is [DateTime] -or $Value -is [DateTimeOffset]) { return '"' + $Value.ToString('o', [Globalization.CultureInfo]::InvariantCulture) + '"' }
    if (Test-BenchRulesObject $Value) {
        $keys = Get-BenchRulesKeys $Value
        $keys.Sort([StringComparer]::Ordinal)
        $parts = [Collections.Generic.List[string]]::new()
        foreach ($key in $keys) { $parts.Add((ConvertTo-BenchRulesCanonical $key) + ':' + (ConvertTo-BenchRulesCanonical (Get-BenchRulesField $Value $key))) }
        return '{' + ($parts -join ',') + '}'
    }
    if ($Value -is [Collections.IList]) {
        $parts = [Collections.Generic.List[string]]::new()
        foreach ($item in $Value) { $parts.Add((ConvertTo-BenchRulesCanonical $item)) }
        return '[' + ($parts -join ',') + ']'
    }
    return '?' + $Value.GetType().FullName + ':' + [string]$Value
}

function Get-BenchEnvCanonical($Object, [string] $Field) {
    # A compared field's canonical text, or $null when the file lacks it. A list path gives its
    # elements' texts sorted, so the elements' order doesn't matter; an element that lacks the
    # field counts as "missing".
    $parts = $Field.Split('.')
    $current = $Object
    for ($i = 0; $i -lt $parts.Length; $i++) {
        $part = $parts[$i]
        if ($part.EndsWith('[]', [StringComparison]::Ordinal)) {
            $name = $part.Substring(0, $part.Length - 2)
            if (-not (Test-BenchRulesField $current $name)) { return $null }
            $items = Get-BenchRulesField $current $name
            if (-not (Test-BenchRulesList $items)) { return ('not a list: ' + (ConvertTo-BenchRulesCanonical $items)) }
            $rest = ''
            if ($i + 1 -lt $parts.Length) { $rest = [string]::Join('.', $parts, $i + 1, $parts.Length - $i - 1) }
            $texts = [Collections.Generic.List[string]]::new()
            foreach ($item in $items) {
                if ($rest.Length -eq 0) {
                    $texts.Add((ConvertTo-BenchRulesCanonical $item))
                } else {
                    $text = Get-BenchEnvCanonical $item $rest
                    if ($null -eq $text) { $text = 'missing' }
                    $texts.Add($text)
                }
            }
            $texts.Sort([StringComparer]::Ordinal)
            return '[' + ($texts -join ',') + ']'
        }
        if (-not (Test-BenchRulesField $current $part)) { return $null }
        $current = Get-BenchRulesField $current $part
    }
    return (ConvertTo-BenchRulesCanonical $current)
}

function Get-BenchDisplaysCanonical($Displays) {
    # bench.json's displays_start or displays_end as each display's role, refresh_mhz and hdr,
    # sorted, so their order doesn't matter; $null when it isn't a list.
    if (-not (Test-BenchRulesList $Displays)) { return $null }
    $texts = [Collections.Generic.List[string]]::new()
    foreach ($display in $Displays) {
        $parts = [Collections.Generic.List[string]]::new()
        foreach ($name in @('role', 'refresh_mhz', 'hdr')) {
            $value = 'missing'
            if (Test-BenchRulesField $display $name) { $value = ConvertTo-BenchRulesCanonical (Get-BenchRulesField $display $name) }
            $parts.Add('"' + $name + '":' + $value)
        }
        $texts.Add('{' + ($parts -join ',') + '}')
    }
    $texts.Sort([StringComparer]::Ordinal)
    return '[' + ($texts -join ',') + ']'
}

function Compare-BenchEnv {
    # env.json before and after a session, and with $Bench the displays at its start and end: any
    # change means the session doesn't count (M1:217). Output names the fields, never their values,
    # which hold PC details and stay in the env.json files in .dogfood/.
    param(
        $Before,
        $After,
        $Bench = $null
    )
    $beforeObject = ConvertFrom-BenchRulesJson $Before 'env-before'
    $afterObject = ConvertFrom-BenchRulesJson $After 'env-after'
    $changed = [Collections.Generic.List[string]]::new()
    $notCompared = [Collections.Generic.List[string]]::new()
    $compared = 0
    foreach ($field in (Get-BenchEnvFields)) {
        $old = Get-BenchEnvCanonical $beforeObject $field
        $new = Get-BenchEnvCanonical $afterObject $field
        if ($null -eq $old -and $null -eq $new) {
            $notCompared.Add($field)
            continue
        }
        $compared++
        if ($null -eq $old -or $null -eq $new -or -not [string]::Equals($old, $new, [StringComparison]::Ordinal)) { $changed.Add($field) }
    }
    # The displays come from the session's bench.json (section 4.8). A job ended by your input
    # never writes displays_end, so a missing list is "not compared", not a change.
    if ($null -eq $Bench) {
        $notCompared.Add('displays')
    } else {
        $benchObject = ConvertFrom-BenchRulesJson $Bench 'bench.json'
        if ((ConvertTo-BenchRulesInteger (Get-BenchRulesField $benchObject 'kairo_bench')) -ne 1) { throw (New-BenchRulesError 3 'not a bench.json (kairo_bench 1)') }
        $start = Get-BenchDisplaysCanonical (Get-BenchRulesField $benchObject 'displays_start')
        $end = Get-BenchDisplaysCanonical (Get-BenchRulesField $benchObject 'displays_end')
        if ($null -eq $start -or $null -eq $end) {
            $notCompared.Add('displays')
        } else {
            $compared++
            if (-not [string]::Equals($start, $end, [StringComparison]::Ordinal)) { $changed.Add('displays') }
        }
    }
    if ($compared -eq 0) { throw (New-BenchRulesError 3 'nothing to compare: neither file holds a compared field (env.json files?)') }

    $out = [ordered]@{}
    $out['bench_rules'] = 1
    $out['mode'] = 'compare_env'
    $out['same'] = ($changed.Count -eq 0)
    $out['changed'] = $changed
    $out['not_compared'] = $notCompared
    $code = 0
    if ($changed.Count -gt 0) { $code = 1 }
    return [pscustomobject]@{ Comparison = $out; Json = (ConvertTo-Json -InputObject $out -Compress -Depth 5); ExitCode = $code }
}

# --- The command line -------------------------------------------------------------------------

function Get-BenchRulesLeafName([string] $File) {
    # The file name alone: output never names a folder.
    if ($null -eq $File) { return '' }
    $cut = $File.LastIndexOfAny([char[]]@('\', '/'))
    if ($cut -ge 0) { return $File.Substring($cut + 1) }
    return $File
}

function Read-BenchRulesInput([string] $Value, [long] $MaxBytes, [switch] $Lines) {
    # A command-line value: JSON text when it starts with '{', otherwise a file, read whole (at most
    # $MaxBytes) or, with -Lines, as one JSON value per line that isn't blank. A relative name is
    # taken from the current folder. Errors name the file only.
    $text = $Value.Trim()
    if ($text.Length -eq 0) { throw (New-BenchRulesError 2 'an empty value') }
    if ($text[0] -eq [char]'{') { return ,[string[]]@($text) }
    $leaf = Get-BenchRulesLeafName $Value
    try {
        $full = [IO.Path]::GetFullPath([IO.Path]::Combine((Get-Location -PSProvider FileSystem).ProviderPath, $Value))
        $info = [IO.FileInfo]::new($full)
        if (-not $info.Exists) { throw (New-BenchRulesError 3 ('no such file: ' + $leaf)) }
        if ($info.Length -gt $MaxBytes) { throw (New-BenchRulesError 3 ($leaf + ' is larger than ' + $MaxBytes + ' bytes')) }
        $content = [IO.File]::ReadAllText($full)
    } catch {
        if ($_.Exception.Data.Contains('KairoExit')) { throw }
        throw (New-BenchRulesError 3 ('can''t read ' + $leaf + ' (' + $_.Exception.GetType().Name + ')'))
    }
    if (-not $Lines) { return ,[string[]]@($content) }
    $values = [Collections.Generic.List[string]]::new()
    foreach ($line in $content.Split([char]10)) {
        $trimmed = $line.Trim([char[]]@([char]0xFEFF, ' ', [char]9, [char]13))
        if ($trimmed.Length -gt 0) { $values.Add($trimmed) }
    }
    return ,$values.ToArray()
}

function ConvertFrom-BenchRulesArguments([object[]] $Arguments) {
    # pwsh -File passes every argument as text: -Name:value arrives as -Name and value, and
    # -Name:$false as -Name and a bool. & passes -Name: and the value, a switch's as a bool, and
    # numbers as numbers. Names are matched whole, ignoring case; -Stats and -Encoder take every
    # value up to the next name; anything else is refused.
    $values = @('Bench', 'Valid', 'AaStats', 'Workload', 'Before', 'After')
    $lists = @('Stats', 'Encoder')
    $switches = @('Decide', 'CompareEnv')
    $parsed = @{}
    $i = 0
    while ($i -lt $Arguments.Count) {
        $token = $Arguments[$i]
        $i++
        $match = $null
        if ($token -is [string]) { $match = [regex]::Match($token, '^-([A-Za-z]+):?\z') }
        if ($null -eq $match -or -not $match.Success) { throw (New-BenchRulesError 2 ('unexpected argument at position ' + $i)) }
        $name = $null
        foreach ($known in ($values + $lists + $switches)) {
            if ([string]::Equals($known, $match.Groups[1].Value, [StringComparison]::OrdinalIgnoreCase)) { $name = $known }
        }
        if ($null -eq $name) { throw (New-BenchRulesError 2 ('unknown parameter -' + $match.Groups[1].Value)) }
        if ($parsed.ContainsKey($name)) { throw (New-BenchRulesError 2 ('-' + $name + ' given twice')) }
        if ($switches -contains $name) {
            $parsed[$name] = $true
            if ($i -lt $Arguments.Count -and $Arguments[$i] -is [bool]) {
                $parsed[$name] = $Arguments[$i]
                $i++
            }
            continue
        }
        if ($values -contains $name) {
            if ($i -ge $Arguments.Count) { throw (New-BenchRulesError 2 ('-' + $name + ' needs a value')) }
            $parsed[$name] = $Arguments[$i]
            $i++
            continue
        }
        $list = [Collections.Generic.List[object]]::new()
        while ($i -lt $Arguments.Count) {
            $next = $Arguments[$i]
            if ($next -is [string] -and $next -match '^-[A-Za-z]+:?\z') { break }
            $list.Add($next)
            $i++
        }
        if ($list.Count -eq 0) { throw (New-BenchRulesError 2 ('-' + $name + ' needs at least one value')) }
        $parsed[$name] = $list
    }
    return $parsed
}

# --- Command line: only when the file is run, never when it is dot-sourced. -----------------
if ($MyInvocation.InvocationName -ne '.') {
    Set-StrictMode -Version 3.0
    $ErrorActionPreference = 'Stop'
    $benchRulesCode = 0
    try {
        $benchRulesArgs = ConvertFrom-BenchRulesArguments $args
        $benchRulesDecide = $benchRulesArgs.ContainsKey('Decide') -and [bool]$benchRulesArgs['Decide']
        $benchRulesCompare = $benchRulesArgs.ContainsKey('CompareEnv') -and [bool]$benchRulesArgs['CompareEnv']
        if ($benchRulesDecide -and $benchRulesCompare) { throw (New-BenchRulesError 2 'give -Decide or -CompareEnv, not both') }
        $benchRulesMode = 'validity'
        $benchRulesAllowed = @('Bench', 'Stats', 'Encoder')
        if ($benchRulesDecide) {
            $benchRulesMode = 'decide'
            $benchRulesAllowed = @('Decide', 'Valid', 'AaStats', 'Workload')
        }
        if ($benchRulesCompare) {
            $benchRulesMode = 'compare'
            $benchRulesAllowed = @('CompareEnv', 'Before', 'After', 'Bench')
        }
        foreach ($benchRulesGiven in $benchRulesArgs.Keys) {
            if ($benchRulesAllowed -notcontains $benchRulesGiven) {
                if ($benchRulesGiven -eq 'Decide' -or $benchRulesGiven -eq 'CompareEnv') { continue }
                throw (New-BenchRulesError 2 ('-' + $benchRulesGiven + ' does not go with this mode'))
            }
        }
        if ($benchRulesMode -eq 'validity') {
            foreach ($benchRulesNeeded in @('Bench', 'Stats')) {
                if (-not $benchRulesArgs.ContainsKey($benchRulesNeeded)) { throw (New-BenchRulesError 2 ('missing -' + $benchRulesNeeded)) }
            }
            $benchRulesBench = (Read-BenchRulesInput ([string]$benchRulesArgs['Bench']) 1048576)[0]
            $benchRulesStats = [Collections.Generic.List[object]]::new()
            foreach ($benchRulesValue in $benchRulesArgs['Stats']) {
                foreach ($benchRulesLine in (Read-BenchRulesInput ([string]$benchRulesValue) 16777216 -Lines)) { $benchRulesStats.Add($benchRulesLine) }
            }
            $benchRulesEncoder = $null
            if ($benchRulesArgs.ContainsKey('Encoder')) {
                $benchRulesEncoder = [Collections.Generic.List[object]]::new()
                foreach ($benchRulesValue in $benchRulesArgs['Encoder']) {
                    foreach ($benchRulesLine in (Read-BenchRulesInput ([string]$benchRulesValue) 1048576 -Lines)) { $benchRulesEncoder.Add($benchRulesLine) }
                }
                if ($benchRulesEncoder.Count -eq 0) { throw (New-BenchRulesError 3 'no encoder-usage line in -Encoder') }
                $benchRulesEncoder = $benchRulesEncoder.ToArray()
            }
            $benchRulesResult = Get-BenchRules -Bench $benchRulesBench -Stats $benchRulesStats.ToArray() -EncoderUsage $benchRulesEncoder
        } elseif ($benchRulesMode -eq 'decide') {
            if (-not $benchRulesArgs.ContainsKey('Valid')) { throw (New-BenchRulesError 2 'missing -Valid') }
            $benchRulesAa = $null
            if ($benchRulesArgs.ContainsKey('AaStats')) {
                $benchRulesAaLines = Read-BenchRulesInput ([string]$benchRulesArgs['AaStats']) 1048576 -Lines
                if ($benchRulesAaLines.Length -ne 1) { throw (New-BenchRulesError 3 '-AaStats must hold one ab-stats line') }
                $benchRulesAa = $benchRulesAaLines[0]
            }
            $benchRulesWorkload = ''
            if ($benchRulesArgs.ContainsKey('Workload')) { $benchRulesWorkload = [string]$benchRulesArgs['Workload'] }
            $benchRulesResult = Get-BenchDecision -Valid $benchRulesArgs['Valid'] -AaStats $benchRulesAa -Workload $benchRulesWorkload
        } else {
            foreach ($benchRulesNeeded in @('Before', 'After')) {
                if (-not $benchRulesArgs.ContainsKey($benchRulesNeeded)) { throw (New-BenchRulesError 2 ('missing -' + $benchRulesNeeded)) }
            }
            $benchRulesBefore = (Read-BenchRulesInput ([string]$benchRulesArgs['Before']) 1048576)[0]
            $benchRulesAfter = (Read-BenchRulesInput ([string]$benchRulesArgs['After']) 1048576)[0]
            $benchRulesBench = $null
            if ($benchRulesArgs.ContainsKey('Bench')) { $benchRulesBench = (Read-BenchRulesInput ([string]$benchRulesArgs['Bench']) 1048576)[0] }
            $benchRulesResult = Compare-BenchEnv -Before $benchRulesBefore -After $benchRulesAfter -Bench $benchRulesBench
        }
        [Console]::Out.WriteLine($benchRulesResult.Json)
        $benchRulesCode = $benchRulesResult.ExitCode
    } catch {
        $benchRulesCode = Get-BenchRulesExitCode $_
        $benchRulesError = Get-BenchRulesKairoException $_
        if ($null -ne $benchRulesError) {
            $benchRulesMessage = $benchRulesError.Message
        } else {
            # A bug. Its message could quote a path or a value from the files, so only its type
            # and line are printed.
            $benchRulesLine = 0
            if ($null -ne $_.InvocationInfo) { $benchRulesLine = $_.InvocationInfo.ScriptLineNumber }
            $benchRulesMessage = 'internal error: ' + $_.Exception.GetType().Name + ' at line ' + $benchRulesLine
        }
        [Console]::Error.WriteLine('bench-rules: ' + $benchRulesMessage)
    }
    exit $benchRulesCode
}
