# SPDX-License-Identifier: GPL-3.0-or-later
#
# tools/pc/bench/ab-stats.ps1 - the paired A/B loss with its 95% interval, and the A/A noise floor.
# Project: https://github.com/Maty3k/Kairo
# Design: docs/research/17-loadgen-bench/README.md section 5.2. Contract: docs/milestones/M1.md, Formats.
#
# Command line:
#   pwsh -NoProfile -File ab-stats.ps1 -A <runs> [-B <runs> [-Reference <ab-stats line>]] [-Json]
#   pwsh -NoProfile -File ab-stats.ps1 -A <runs> -SelfTest [-Json]
# <runs> are pm-stats JSON lines, files of them (one run per line, in run order), or bare avg_fps
# numbers (separated by commas or spaces); one kind per call. -A alone is the A/A noise floor; -A
# and -B pair the runs by order (A = Kairo off, B = Kairo on); -SelfTest pairs -A's own runs as
# (1,2), (3,4), (5,6) and so on, leaving an odd last run out. -Reference is the paired ab-stats
# line of Instant Replay alone, from the same session and workload. A file is UTF-8 or UTF-16 with
# a byte-order mark, or UTF-8 without one; its blank lines are skipped.
# Output, in the invariant culture:
#   6 pairs: loss 2.44% (95% CI 2.25..2.63%) PASS=True
#   6 runs: avg_fps 120.33 sd 0.71 (CV 0.59%) NOISE=ok
# or with -Json one line at full precision:
#   {"ab_stats":1,"mode":"ab","n":6,"loss_pct":..,"sd_pp":..,"t":..,"ci_lo":..,"ci_hi":..,"clauses":
#    {"mean_le_3":..,"upper_le_4":..,"low1_le_5":..,"present_mode":null,"ir_plus_1pp":..},"pass":..}
#   {"ab_stats":1,"mode":"aa","n":6,"mean_fps":..,"sd_fps":..,"cv_pct":..,"low1_cv_pct":..,
#    "drift_pct_per_run":..,"noise_ok":..}
# PASS is M1's two clauses: mean loss <= 3% and upper 95% bound <= 4%. The plan's other clauses
# join PASS before SP1; until then they are reported only, and null means not evaluated:
# low1_le_5 (the same pairs on low1_fps, mean <= 5%) needs pm-stats lines, ir_plus_1pp (this loss
# minus -Reference's <= 1 pp) needs -Reference, and present_mode isn't built yet. NOISE is ok at a
# CV of 1.00% or less (sample SD, n - 1); low1_cv_pct (pm-stats lines only) and drift_pct_per_run
# (the least-squares slope of avg_fps over the run order, as % of the mean) are diagnostics.
# Exit codes: 0 PASS or noise ok; 1 FAIL or noise high; 2 bad command line (unequal counts, n < 2,
# mixed kinds); 3 can't evaluate (an A run's avg_fps <= 0, an unparsable line or number, an
# unreadable file or one without runs).
#
# As a library (the elevated session dot-sources the text it received on stdin; so do the tests):
#   $r = Get-AbStats -A $runsA -B $runsB [-Reference $line]   # -A alone: A/A
#   $r = Get-AbStatsSelfTest -Runs $runs
#   $lines = ConvertFrom-AbStatsText $text                     # a file's text -> its runs, in order
#   $r.Text and $r.Json are the two outputs, $r.Stats the JSON as an ordered dictionary, $r.ExitCode
#   0 or 1, $r.Low1LossPct the 1%-low loss behind low1_le_5 (or $null), $r.Warnings the warnings.
# Runs may be numbers, pm-stats JSON lines, or the .Stats dictionaries Get-PmStats returns.
# Functions only return or throw. A thrown exception carries its exit code in .Data['KairoExit'].
# Dot-sourcing defines functions and nothing else; the command line at the bottom runs only when
# the file itself is run, and it never prompts. Load the text with '.', from the file or through
# [scriptblock]::Create: Invoke-Expression reports the caller's name, so the command line would run.
#
# Rules: ASCII only; Windows PowerShell 5.1 syntax and .NET Framework 4.x APIs (it runs on
# PowerShell 7); no Add-Type; clean under Set-StrictMode -Version 3.0. Output names files, never
# folders, and never echoes a run's line.

function New-AbStatsError([int] $Code, [string] $Message) {
    $exception = [Exception]::new($Message)
    $exception.Data['KairoExit'] = $Code
    return $exception
}

function Get-AbStatsExitCode($ErrorRecord) {
    # The exit code a caught error stands for: its own, or 3 (can't evaluate).
    $exception = $ErrorRecord.Exception
    while ($null -ne $exception) {
        if ($exception.Data.Contains('KairoExit')) { return [int]$exception.Data['KairoExit'] }
        $exception = $exception.InnerException
    }
    return 3
}

function Get-AbStatsT975([int] $Df) {
    # t(0.975, df). df 1-30: the published table to 4 decimals. Beyond 30: the Cornish-Fisher
    # series, whose error is 3.1e-8 at df 30 and falls from there (research 17 section 5.2).
    # Research 13's table stopped at n = 10, 12 and 16, so any other n lost its interval.
    if ($Df -lt 1) { throw (New-AbStatsError 2 'the t quantile needs at least 1 degree of freedom') }
    $table = @(12.7062, 4.3027, 3.1824, 2.7764, 2.5706, 2.4469, 2.3646, 2.3060, 2.2622, 2.2281,
        2.2010, 2.1788, 2.1604, 2.1448, 2.1314, 2.1199, 2.1098, 2.1009, 2.0930, 2.0860,
        2.0796, 2.0739, 2.0687, 2.0639, 2.0595, 2.0555, 2.0518, 2.0484, 2.0452, 2.0423)
    if ($Df -le 30) { return [double]$table[$Df - 1] }
    $z = 1.959963984540054
    $v = [double]$Df
    $z2 = $z * $z
    $z3 = $z2 * $z
    $z5 = $z3 * $z2
    $z7 = $z5 * $z2
    $z9 = $z7 * $z2
    return $z + ($z3 + $z) / (4 * $v) +
        (5 * $z5 + 16 * $z3 + 3 * $z) / (96 * $v * $v) +
        (3 * $z7 + 19 * $z5 + 17 * $z3 - 15 * $z) / (384 * $v * $v * $v) +
        (79 * $z9 + 776 * $z7 + 1482 * $z5 - 1920 * $z3 - 945 * $z) / (92160 * $v * $v * $v * $v)
}

function Format-AbStatsNumber([double] $Value) {
    # Two decimals in the invariant culture, rounded AwayFromZero first, so PowerShell 5.1 and 7
    # agree; -0 prints as 0.
    $rounded = [Math]::Round($Value, 2, [MidpointRounding]::AwayFromZero)
    if ($rounded -eq 0) { $rounded = 0.0 }
    return $rounded.ToString('F2', [Globalization.CultureInfo]::InvariantCulture)
}

function Get-AbStatsField($Object, [string] $Name) {
    # A field of a parsed JSON object or a dictionary; $null when it is missing.
    if ($null -eq $Object) { return $null }
    if ($Object -is [Collections.IDictionary]) {
        if ($Object.Contains($Name)) { return $Object[$Name] }
        return $null
    }
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) { return $null }
    return $property.Value
}

function ConvertTo-AbStatsDouble($Value) {
    # A finite number as a double (JSON numbers arrive as [decimal] on 5.1 and as [double] on 7);
    # $null for anything else, strings and booleans included.
    if ($null -eq $Value) { return $null }
    if ($Value -is [double] -or $Value -is [int] -or $Value -is [long] -or $Value -is [decimal] -or $Value -is [single] -or
        $Value -is [int16] -or $Value -is [byte] -or $Value -is [uint16] -or $Value -is [uint32] -or $Value -is [uint64] -or $Value -is [sbyte]) {
        $d = [double]$Value
        if ([double]::IsNaN($d) -or [double]::IsInfinity($d)) { return $null }
        return $d
    }
    return $null
}

function Test-AbStatsVersion($Object, [string] $Key) {
    # True when the object's $Key field is the number 1 (pm_stats 1, ab_stats 1).
    $version = ConvertTo-AbStatsDouble (Get-AbStatsField $Object $Key)
    return ($null -ne $version -and $version -eq 1)
}

function ConvertFrom-AbStatsLine([string] $Text, [string] $What) {
    # One JSON line; a line that doesn't parse can't be evaluated (exit 3). The line itself is
    # never echoed.
    try {
        return (ConvertFrom-Json -InputObject $Text -ErrorAction Stop)
    } catch {
        throw (New-AbStatsError 3 ('an unparsable ' + $What))
    }
}

function ConvertTo-AbStatsRun($Item) {
    # One run as [pscustomobject]@{ Kind = 'number' or 'pm_stats'; Fps; Low1 }. A string is a number
    # in the invariant culture or a pm-stats line; an object is a parsed line or a dictionary.
    if ($null -eq $Item) { throw (New-AbStatsError 3 'an empty run') }
    $number = ConvertTo-AbStatsDouble $Item
    if ($null -ne $number) { return [pscustomobject]@{ Kind = 'number'; Fps = $number; Low1 = $null } }
    $object = $Item
    if ($Item -is [string]) {
        $text = $Item.Trim()
        if ($text.Length -eq 0 -or $text[0] -ne [char]'{') {
            $value = 0.0
            if ([double]::TryParse($text, [Globalization.NumberStyles]::Float, [Globalization.CultureInfo]::InvariantCulture, [ref] $value) -and
                -not ([double]::IsNaN($value) -or [double]::IsInfinity($value))) {
                return [pscustomobject]@{ Kind = 'number'; Fps = $value; Low1 = $null }
            }
            throw (New-AbStatsError 3 'a run that is neither a finite number nor a pm-stats line')
        }
        $object = ConvertFrom-AbStatsLine $text 'pm-stats line'
    }
    if (-not (Test-AbStatsVersion $object 'pm_stats')) { throw (New-AbStatsError 3 'not a pm-stats line (pm_stats 1)') }
    $fps = ConvertTo-AbStatsDouble (Get-AbStatsField $object 'avg_fps')
    if ($null -eq $fps) { throw (New-AbStatsError 3 'a pm-stats line without a numeric avg_fps') }
    $low1 = ConvertTo-AbStatsDouble (Get-AbStatsField $object 'low1_fps')
    return [pscustomobject]@{ Kind = 'pm_stats'; Fps = $fps; Low1 = $low1 }
}

function Get-AbStatsRunList([object[]] $Items) {
    # The runs in order; arrays inside the list are flattened, so @(1, 2) and a list of lines both work.
    $runs = [Collections.Generic.List[object]]::new()
    if ($null -eq $Items) { return ,$runs }
    foreach ($item in $Items) {
        if ($item -is [Array]) {
            foreach ($inner in $item) { $runs.Add((ConvertTo-AbStatsRun $inner)) }
        } else {
            $runs.Add((ConvertTo-AbStatsRun $item))
        }
    }
    return ,$runs
}

function ConvertTo-AbStatsReference($Item) {
    # -Reference: the paired line of Instant Replay alone; its loss_pct is m(IR). $null when absent.
    if ($null -eq $Item) { return $null }
    $object = $Item
    if ($Item -is [string]) {
        $text = $Item.Trim()
        if ($text.Length -eq 0 -or $text[0] -ne [char]'{') { throw (New-AbStatsError 3 '-Reference is not an ab-stats line') }
        $object = ConvertFrom-AbStatsLine $text '-Reference line'
    }
    $mode = Get-AbStatsField $object 'mode'
    if (-not (Test-AbStatsVersion $object 'ab_stats') -or -not ($mode -is [string]) -or $mode -cne 'ab') {
        throw (New-AbStatsError 3 '-Reference is not a paired ab-stats line (ab_stats 1, mode ab)')
    }
    $loss = ConvertTo-AbStatsDouble (Get-AbStatsField $object 'loss_pct')
    if ($null -eq $loss) { throw (New-AbStatsError 3 '-Reference has no numeric loss_pct') }
    return $loss
}

function Assert-AbStatsOneKind($Runs) {
    # One kind per call: pm-stats lines or bare numbers.
    $kinds = @{}
    foreach ($run in $Runs) { $kinds[$run.Kind] = $true }
    if ($kinds.Count -gt 1) { throw (New-AbStatsError 2 'mixed kinds: give pm-stats lines or bare numbers, not both') }
}

function Assert-AbStatsFinite([double[]] $Values) {
    # Finite inputs can still overflow; such a result can't be evaluated.
    foreach ($value in $Values) {
        if ([double]::IsNaN($value) -or [double]::IsInfinity($value)) { throw (New-AbStatsError 3 'the runs give a result that is not finite') }
    }
}

function Get-AbStatsMeanSd([double[]] $Values) {
    # Mean and sample SD (n - 1), as Measure-Object -StandardDeviation computes it.
    $n = $Values.Length
    $sum = 0.0
    foreach ($x in $Values) { $sum += $x }
    $mean = $sum / $n
    $squares = 0.0
    foreach ($x in $Values) { $squares += ($x - $mean) * ($x - $mean) }
    return @($mean, [Math]::Sqrt($squares / ($n - 1)))
}

function Get-AbStatsPaired($RunsA, $RunsB, $ReferenceLoss) {
    # d_i = 100 (A_i - B_i) / A_i from avg_fps, positive a loss; m their mean, s their sample SD,
    # h = t(0.975, n - 1) s / sqrt(n), CI = [m - h, m + h]; PASS = m <= 3 and m + h <= 4.
    $inv = [Globalization.CultureInfo]::InvariantCulture
    $n = $RunsA.Count
    $loss = [double[]]::new($n)
    for ($i = 0; $i -lt $n; $i++) { $loss[$i] = 100.0 * ($RunsA[$i].Fps - $RunsB[$i].Fps) / $RunsA[$i].Fps }
    Assert-AbStatsFinite $loss
    $meanSd = Get-AbStatsMeanSd $loss
    $m = $meanSd[0]
    $s = $meanSd[1]
    $t = Get-AbStatsT975 ($n - 1)
    $h = $t * $s / [Math]::Sqrt($n)
    Assert-AbStatsFinite @($m, $s, $h, ($m - $h), ($m + $h))
    $meanOk = ($m -le 3.0)
    $upperOk = (($m + $h) -le 4.0)
    $pass = ($meanOk -and $upperOk)

    # The 1%-low clause (PLAN:353): the same pairs on low1_fps, mean <= 5, no interval. It needs
    # every pair's low1_fps, above 0 on the A side, so bare numbers leave it null.
    $low1Loss = $null
    $low1Ok = $null
    $haveLow1 = $true
    for ($i = 0; $i -lt $n; $i++) {
        if ($null -eq $RunsA[$i].Low1 -or $null -eq $RunsB[$i].Low1 -or $RunsA[$i].Low1 -le 0) { $haveLow1 = $false }
    }
    if ($haveLow1) {
        $sum = 0.0
        for ($i = 0; $i -lt $n; $i++) { $sum += 100.0 * ($RunsA[$i].Low1 - $RunsB[$i].Low1) / $RunsA[$i].Low1 }
        $low1Loss = $sum / $n
        Assert-AbStatsFinite @($low1Loss)
        $low1Ok = ($low1Loss -le 5.0)
    }
    # No worse than Instant Replay alone + 1 pp (PLAN:353, PLAN:427): m - m(IR) <= 1.0.
    $irOk = $null
    if ($null -ne $ReferenceLoss) { $irOk = (($m - $ReferenceLoss) -le 1.0) }

    $clauses = [ordered]@{}
    $clauses['mean_le_3'] = $meanOk
    $clauses['upper_le_4'] = $upperOk
    $clauses['low1_le_5'] = $low1Ok
    $clauses['present_mode'] = $null
    $clauses['ir_plus_1pp'] = $irOk
    $stats = [ordered]@{}
    $stats['ab_stats'] = 1
    $stats['mode'] = 'ab'
    $stats['n'] = $n
    $stats['loss_pct'] = $m
    $stats['sd_pp'] = $s
    $stats['t'] = $t
    $stats['ci_lo'] = $m - $h
    $stats['ci_hi'] = $m + $h
    $stats['clauses'] = $clauses
    $stats['pass'] = $pass
    $text = $n.ToString($inv) + ' pairs: loss ' + (Format-AbStatsNumber $m) + '% (95% CI ' + (Format-AbStatsNumber ($m - $h)) +
        '..' + (Format-AbStatsNumber ($m + $h)) + '%) PASS=' + $pass
    $code = 1
    if ($pass) { $code = 0 }
    return [pscustomobject]@{
        Stats       = $stats
        Text        = $text
        Json        = (ConvertTo-Json -InputObject $stats -Compress -Depth 5)
        ExitCode    = $code
        Low1LossPct = $low1Loss
        Warnings    = [string[]]@()
    }
}

function Get-AbStatsNoise($Runs) {
    # The A/A noise floor: CV = 100 s / mean over avg_fps, s the sample SD; noise_ok = CV <= 1.00.
    $inv = [Globalization.CultureInfo]::InvariantCulture
    $n = $Runs.Count
    $fps = [double[]]::new($n)
    for ($i = 0; $i -lt $n; $i++) { $fps[$i] = $Runs[$i].Fps }
    $meanSd = Get-AbStatsMeanSd $fps
    $mean = $meanSd[0]
    $sd = $meanSd[1]
    $cv = 100.0 * $sd / $mean

    # Diagnostics, not gating (research 17 section 4.7): the CV of low1_fps, and the least-squares
    # drift of avg_fps per run as a percentage of the mean, where thermal creep shows.
    $low1Cv = $null
    $haveLow1 = $true
    foreach ($run in $Runs) { if ($null -eq $run.Low1 -or $run.Low1 -le 0) { $haveLow1 = $false } }
    if ($haveLow1) {
        $low1 = [double[]]::new($n)
        for ($i = 0; $i -lt $n; $i++) { $low1[$i] = $Runs[$i].Low1 }
        $low1MeanSd = Get-AbStatsMeanSd $low1
        $low1Cv = 100.0 * $low1MeanSd[1] / $low1MeanSd[0]
        Assert-AbStatsFinite @($low1Cv)
    }
    $xMean = ($n + 1) / 2.0
    $num = 0.0
    $den = 0.0
    for ($i = 0; $i -lt $n; $i++) {
        $num += (($i + 1) - $xMean) * ($fps[$i] - $mean)
        $den += (($i + 1) - $xMean) * (($i + 1) - $xMean)
    }
    $drift = 100.0 * ($num / $den) / $mean
    Assert-AbStatsFinite @($mean, $sd, $cv, $drift)
    $ok = ($cv -le 1.0)

    $stats = [ordered]@{}
    $stats['ab_stats'] = 1
    $stats['mode'] = 'aa'
    $stats['n'] = $n
    $stats['mean_fps'] = $mean
    $stats['sd_fps'] = $sd
    $stats['cv_pct'] = $cv
    $stats['low1_cv_pct'] = $low1Cv
    $stats['drift_pct_per_run'] = $drift
    $stats['noise_ok'] = $ok
    $word = 'high'
    $code = 1
    if ($ok) { $word = 'ok'; $code = 0 }
    $text = $n.ToString($inv) + ' runs: avg_fps ' + (Format-AbStatsNumber $mean) + ' sd ' + (Format-AbStatsNumber $sd) +
        ' (CV ' + (Format-AbStatsNumber $cv) + '%) NOISE=' + $word
    return [pscustomobject]@{
        Stats       = $stats
        Text        = $text
        Json        = (ConvertTo-Json -InputObject $stats -Compress -Depth 5)
        ExitCode    = $code
        Low1LossPct = $null
        Warnings    = [string[]]@()
    }
}

function Assert-AbStatsPositive($Runs, [string] $What) {
    # A run whose avg_fps divides the loss or the CV must be above 0.
    foreach ($run in $Runs) {
        if ($run.Fps -le 0) { throw (New-AbStatsError 3 ($What + ' with an avg_fps of 0 or less')) }
    }
}

function Get-AbStatsResult($RunsA, $RunsB, $ReferenceLoss) {
    # The checks every mode shares, in the order of their exit codes, then the statistics.
    # $RunsB $null is the A/A mode.
    $all = [Collections.Generic.List[object]]::new()
    foreach ($run in $RunsA) { $all.Add($run) }
    if ($null -ne $RunsB) { foreach ($run in $RunsB) { $all.Add($run) } }
    Assert-AbStatsOneKind $all
    if ($null -ne $RunsB) {
        if ($RunsA.Count -ne $RunsB.Count) {
            throw (New-AbStatsError 2 ('unequal counts: ' + $RunsA.Count + ' A runs and ' + $RunsB.Count + ' B runs'))
        }
        if ($RunsA.Count -lt 2) { throw (New-AbStatsError 2 ('at least 2 pairs are needed, not ' + $RunsA.Count)) }
    } elseif ($RunsA.Count -lt 2) {
        throw (New-AbStatsError 2 ('at least 2 runs are needed, not ' + $RunsA.Count))
    }
    Assert-AbStatsPositive $RunsA 'an A run'
    if ($null -eq $RunsB) { return (Get-AbStatsNoise $RunsA) }
    return (Get-AbStatsPaired $RunsA $RunsB $ReferenceLoss)
}

function Get-AbStats {
    # -A and -B: the paired loss; -A alone: the A/A noise floor (research 17 section 5.2).
    param(
        [object[]] $A,
        [object[]] $B,
        [object] $Reference = $null
    )
    $paired = $PSBoundParameters.ContainsKey('B') -and $null -ne $B
    if ($null -ne $Reference -and -not $paired) { throw (New-AbStatsError 2 '-Reference needs -B: it compares two paired losses') }
    $runsA = Get-AbStatsRunList $A
    $runsB = $null
    if ($paired) { $runsB = Get-AbStatsRunList $B }
    $referenceLoss = ConvertTo-AbStatsReference $Reference
    return (Get-AbStatsResult $runsA $runsB $referenceLoss)
}

function Get-AbStatsSelfTest {
    # A/A runs paired as (1,2), (3,4), (5,6) and so on: an interval that should include 0, and whose
    # half-width shows what M2's pairs can resolve (research 17 section 5.2). Every run is an A/A
    # run, so every one is checked, an odd last run included; that run is then left out, with a
    # warning.
    param([object[]] $Runs)
    $all = Get-AbStatsRunList $Runs
    Assert-AbStatsOneKind $all
    if ($all.Count -lt 4) { throw (New-AbStatsError 2 ('the self-test needs at least 4 runs (2 pairs), not ' + $all.Count)) }
    Assert-AbStatsPositive $all 'a run'
    $first = [Collections.Generic.List[object]]::new()
    $second = [Collections.Generic.List[object]]::new()
    for ($i = 0; $i + 1 -lt $all.Count; $i += 2) {
        $first.Add($all[$i])
        $second.Add($all[$i + 1])
    }
    $result = Get-AbStatsResult $first $second $null
    if ($all.Count % 2 -eq 1) {
        $result.Warnings = [string[]]@('run ' + $all.Count + ' left out: the self-test pairs runs (1,2), (3,4) and so on')
    }
    return $result
}

function ConvertFrom-AbStatsText([string] $Text) {
    # A file's text -> its runs, one per line in run order: lines split on CRLF or LF, trimmed,
    # blank lines skipped. A byte-order mark left at the start is dropped too.
    $runs = [Collections.Generic.List[string]]::new()
    if ([string]::IsNullOrEmpty($Text)) { return ,$runs.ToArray() }
    foreach ($line in $Text.Split([char[]]@([char]10))) {
        $trimmed = $line.Trim([char[]]@([char]0xFEFF, ' ', [char]9, [char]13))
        if ($trimmed.Length -gt 0) { $runs.Add($trimmed) }
    }
    return ,$runs.ToArray()
}

function Get-AbStatsLeafName([string] $File) {
    # The file name alone: output never names a folder.
    if ($null -eq $File) { return '' }
    $cut = $File.LastIndexOfAny([char[]]@('\', '/'))
    if ($cut -ge 0) { return $File.Substring($cut + 1) }
    return $File
}

function Read-AbStatsArgument([string] $Value) {
    # One command-line value: a pm-stats line; numbers separated by commas or spaces; or a file of
    # pm-stats lines (or numbers), one run per line in run order, at most 16 MiB. Errors name the
    # file only. A relative name is taken from the current folder.
    $text = $Value.Trim()
    if ($text.Length -eq 0) { throw (New-AbStatsError 2 'an empty value') }
    if ($text[0] -eq [char]'{') { return ,@($text) }
    $parts = @($text.Split([char[]]@(',', ' ', [char]9), [StringSplitOptions]::RemoveEmptyEntries))
    $numbers = [Collections.Generic.List[object]]::new()
    $parsedNumber = 0.0
    foreach ($part in $parts) {
        if ([double]::TryParse($part, [Globalization.NumberStyles]::Float, [Globalization.CultureInfo]::InvariantCulture, [ref] $parsedNumber)) {
            $numbers.Add($part)
        }
    }
    if ($parts.Count -gt 0 -and $numbers.Count -eq $parts.Count) { return ,$numbers.ToArray() }
    $leaf = Get-AbStatsLeafName $Value
    try {
        $full = [IO.Path]::GetFullPath([IO.Path]::Combine((Get-Location -PSProvider FileSystem).ProviderPath, $Value))
        $info = [IO.FileInfo]::new($full)
        if (-not $info.Exists) { throw (New-AbStatsError 3 ('neither numbers, a pm-stats line nor a file: ' + $leaf)) }
        if ($info.Length -gt 16777216) { throw (New-AbStatsError 3 ($leaf + ' is larger than 16 MiB')) }
        # ReadAllText decodes by the byte-order mark (UTF-8 or UTF-16), else as UTF-8.
        $content = [IO.File]::ReadAllText($full)
    } catch {
        if ($_.Exception.Data.Contains('KairoExit')) { throw }
        throw (New-AbStatsError 3 ('can''t read ' + $leaf + ' (' + $_.Exception.GetType().Name + ')'))
    }
    $runs = ConvertFrom-AbStatsText $content
    if ($runs.Length -eq 0) { throw (New-AbStatsError 3 ('no runs in ' + $leaf)) }
    return ,$runs
}

function ConvertFrom-AbStatsArguments([object[]] $Arguments) {
    # -A and -B take one or more values each, -Reference one; -SelfTest and -Json are switches.
    # pwsh -File passes every value as text, and -Name:$false as -Name and a bool; & passes -Name:
    # and the value. Names are matched whole, ignoring case; anything else is refused.
    $lists = @('A', 'B', 'Reference')
    $switches = @('SelfTest', 'Json')
    $parsed = @{}
    $i = 0
    while ($i -lt $Arguments.Count) {
        $token = $Arguments[$i]
        $i++
        $match = $null
        if ($token -is [string]) { $match = [regex]::Match($token, '^-([A-Za-z]+):?\z') }
        if ($null -eq $match -or -not $match.Success) { throw (New-AbStatsError 2 ('unexpected argument at position ' + $i)) }
        $name = $null
        foreach ($known in ($lists + $switches)) {
            if ([string]::Equals($known, $match.Groups[1].Value, [StringComparison]::OrdinalIgnoreCase)) { $name = $known }
        }
        if ($null -eq $name) { throw (New-AbStatsError 2 ('unknown parameter -' + $match.Groups[1].Value)) }
        if ($parsed.ContainsKey($name)) { throw (New-AbStatsError 2 ('-' + $name + ' given twice')) }
        if ($switches -contains $name) {
            $parsed[$name] = $true
            if ($i -lt $Arguments.Count -and $Arguments[$i] -is [bool]) {
                $parsed[$name] = $Arguments[$i]
                $i++
            }
            continue
        }
        $values = [Collections.Generic.List[object]]::new()
        while ($i -lt $Arguments.Count) {
            $next = $Arguments[$i]
            if ($next -is [string] -and [regex]::IsMatch($next, '^-[A-Za-z]+:?\z')) { break }
            if ($next -is [Array]) { foreach ($inner in $next) { $values.Add($inner) } } else { $values.Add($next) }
            $i++
        }
        if ($values.Count -eq 0) { throw (New-AbStatsError 2 ('-' + $name + ' needs a value')) }
        if ($name -eq 'Reference' -and $values.Count -ne 1) { throw (New-AbStatsError 2 '-Reference takes one ab-stats line') }
        $parsed[$name] = $values
    }
    return $parsed
}

function Expand-AbStatsValues($Values) {
    # The runs behind -A's or -B's values, in order: a string may hold several runs or name a file.
    $runs = [Collections.Generic.List[object]]::new()
    foreach ($value in $Values) {
        if ($value -is [string]) { foreach ($run in (Read-AbStatsArgument $value)) { $runs.Add($run) } } else { $runs.Add($value) }
    }
    return ,$runs.ToArray()
}

# --- Command line: only when the file is run, never when it is dot-sourced. -----------------
if ($MyInvocation.InvocationName -ne '.') {
    Set-StrictMode -Version 3.0
    $ErrorActionPreference = 'Stop'
    $abStatsCode = 0
    try {
        $abStatsArgs = ConvertFrom-AbStatsArguments $args
        if (-not $abStatsArgs.ContainsKey('A')) { throw (New-AbStatsError 2 'missing -A') }
        $abStatsSelf = $abStatsArgs.ContainsKey('SelfTest') -and [bool]$abStatsArgs['SelfTest']
        $abStatsJson = $abStatsArgs.ContainsKey('Json') -and [bool]$abStatsArgs['Json']
        $abStatsPaired = $abStatsArgs.ContainsKey('B')
        $abStatsReference = $null
        if ($abStatsArgs.ContainsKey('Reference')) { $abStatsReference = $abStatsArgs['Reference'][0] }
        # The command line is checked before any file is read.
        if ($abStatsSelf -and $abStatsPaired) { throw (New-AbStatsError 2 '-SelfTest pairs -A''s own runs: no -B') }
        if ($null -ne $abStatsReference -and -not $abStatsPaired) { throw (New-AbStatsError 2 '-Reference needs -B: it compares two paired losses') }
        $abStatsA = Expand-AbStatsValues $abStatsArgs['A']
        if ($abStatsSelf) {
            $abStatsResult = Get-AbStatsSelfTest -Runs $abStatsA
        } elseif ($abStatsPaired) {
            $abStatsResult = Get-AbStats -A $abStatsA -B (Expand-AbStatsValues $abStatsArgs['B']) -Reference $abStatsReference
        } else {
            $abStatsResult = Get-AbStats -A $abStatsA
        }
        foreach ($warning in $abStatsResult.Warnings) { [Console]::Error.WriteLine('ab-stats: warning: ' + $warning) }
        if ($abStatsJson) {
            [Console]::Out.WriteLine($abStatsResult.Json)
        } else {
            [Console]::Out.WriteLine($abStatsResult.Text)
        }
        $abStatsCode = $abStatsResult.ExitCode
    } catch {
        $abStatsCode = Get-AbStatsExitCode $_
        $message = $_.Exception.Message
        if (-not $_.Exception.Data.Contains('KairoExit')) {
            # A bug. Its message could quote a path or a run's line, so only its type and line
            # are printed.
            $line = 0
            if ($null -ne $_.InvocationInfo) { $line = $_.InvocationInfo.ScriptLineNumber }
            $message = 'internal error: ' + $_.Exception.GetType().Name + ' at line ' + $line
        }
        [Console]::Error.WriteLine('ab-stats: ' + $message)
    }
    exit $abStatsCode
}
