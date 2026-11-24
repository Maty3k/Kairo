# SPDX-License-Identifier: GPL-3.0-or-later
#
# tools/pc/bench/pm-stats.ps1 - one PresentMon 2.6.0 CSV, one process -> one JSON line of statistics.
# Project: https://github.com/Maty3k/Kairo
# Design: docs/research/17-loadgen-bench/README.md section 5.1. Contract: docs/milestones/M1.md, Formats.
#
# Command line:
#   pwsh -NoProfile -File pm-stats.ps1 -Csv <file> -Exe <file name> -Tag <tag> [-ProcessId <pid>] [-Strict]
# Exit codes: 0 the line is printed; 2 bad command line (a missing argument, a bad -Tag, a -ProcessId
# that isn't a positive integer); 3 the CSV can't be evaluated (no file, unreadable or empty, a
# required column missing, no row for the process, several processes, no valid frame).
# Warnings go to stderr, one line each. It never judges a run: bench-rules.ps1 does.
#
# As a library (the elevated session dot-sources the text it received on stdin; so do the tests):
#   $text = ConvertFrom-PmCsvBytes $bytes        # the CSV's bytes, decoded by their byte-order mark
#   $r = Get-PmStats -Text $text -Exe 'kairo-loadgen.exe' -Tag 'alu-01' -ProcessId 4242 -Strict
#   $r.Json is the line, $r.Stats the same as an ordered dictionary, $r.Warnings the warnings.
# Functions only return or throw. A thrown exception carries its exit code in .Data['KairoExit'].
# Dot-sourcing defines functions and nothing else; the command line at the bottom runs only when
# the file itself is run, and it never prompts. Load the text with '.', from the file or through
# [scriptblock]::Create: Invoke-Expression reports the caller's name, so the command line would run.
#
# Rules: ASCII only; Windows PowerShell 5.1 syntax and .NET Framework 4.x APIs (it runs on
# PowerShell 7); no Add-Type; clean under Set-StrictMode -Version 3.0. Output names files, never
# folders, and never another program: rows of other processes leave this file only as counts.

function New-PmStatsError([int] $Code, [string] $Message) {
    $exception = [Exception]::new($Message)
    $exception.Data['KairoExit'] = $Code
    return $exception
}

function Get-PmStatsExitCode($ErrorRecord) {
    # The exit code a caught error stands for: its own, or 3 (can't evaluate).
    $exception = $ErrorRecord.Exception
    while ($null -ne $exception) {
        if ($exception.Data.Contains('KairoExit')) { return [int]$exception.Data['KairoExit'] }
        $exception = $exception.InnerException
    }
    return 3
}

function ConvertTo-PmStatsProcessId($Value) {
    # -ProcessId: a positive integer that fits a Windows process id, or $null when not given.
    if ($null -eq $Value) { return $null }
    $text = ([string]$Value).Trim()
    if ($text -notmatch '^[0-9]{1,10}\z') { throw (New-PmStatsError 2 '-ProcessId must be a positive integer') }
    $id = [long]$text
    if ($id -lt 1 -or $id -gt 4294967295) { throw (New-PmStatsError 2 '-ProcessId must be a positive integer') }
    return $id
}

function Assert-PmStatsArguments([string] $Exe, [string] $Tag) {
    if ([string]::IsNullOrEmpty($Exe)) { throw (New-PmStatsError 2 'missing -Exe') }
    if ($Exe.IndexOfAny([char[]]@('\', '/', ':')) -ge 0) { throw (New-PmStatsError 2 '-Exe takes a file name, not a path') }
    if ([string]::IsNullOrEmpty($Tag)) { throw (New-PmStatsError 2 'missing -Tag') }
    # No path or PC detail can ride in a tag.
    if ($Tag -cnotmatch '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}\z') {
        throw (New-PmStatsError 2 'bad -Tag: letters, digits, dot, underscore and hyphen, at most 64, starting with a letter or digit')
    }
}

function ConvertFrom-PmCsvBytes([byte[]] $Bytes) {
    # PresentMon writes a disk file as UTF-8 with a byte-order mark; UTF-16 (either order) is
    # accepted too, and bytes without a mark are read as UTF-8.
    if ($null -eq $Bytes -or $Bytes.Length -eq 0) { return '' }
    if ($Bytes.Length -ge 3 -and $Bytes[0] -eq 0xEF -and $Bytes[1] -eq 0xBB -and $Bytes[2] -eq 0xBF) {
        return [Text.Encoding]::UTF8.GetString($Bytes, 3, $Bytes.Length - 3)
    }
    if ($Bytes.Length -ge 2 -and $Bytes[0] -eq 0xFF -and $Bytes[1] -eq 0xFE) {
        return [Text.Encoding]::Unicode.GetString($Bytes, 2, $Bytes.Length - 2)
    }
    if ($Bytes.Length -ge 2 -and $Bytes[0] -eq 0xFE -and $Bytes[1] -eq 0xFF) {
        return [Text.Encoding]::BigEndianUnicode.GetString($Bytes, 2, $Bytes.Length - 2)
    }
    return [Text.Encoding]::UTF8.GetString($Bytes)
}

function Get-PmStatsRank([long] $Count, [long] $Num, [long] $Den) {
    # Nearest rank in integers: k = (Num * n + Num) div Den, which is ceil(Num * n / Den).
    $x = $Num * $Count + $Num
    return [long](($x - ($x % $Den)) / $Den)
}

function Get-PmStatsRounded($Value, [int] $Digits) {
    # AwayFromZero, as the design asks; $null stays $null, and -0 prints as 0.
    if ($null -eq $Value) { return $null }
    $rounded = [Math]::Round([double]$Value, $Digits, [MidpointRounding]::AwayFromZero)
    if ($rounded -eq 0) { $rounded = 0.0 }
    return $rounded
}

function Get-PmStatsMean($Values) {
    if ($Values.Count -eq 0) { return $null }
    $sum = 0.0
    foreach ($v in $Values) { $sum += $v }
    return ($sum / $Values.Count)
}

function Get-PmStatsP99($Values) {
    # Nearest-rank P99 over the values; $null when there are none.
    if ($Values.Count -eq 0) { return $null }
    $sorted = $Values.ToArray()
    [Array]::Sort($sorted)
    return $sorted[(Get-PmStatsRank $sorted.Length 99 100) - 1]
}

function Test-PmStatsNumber([string] $Text, [ref] $Value) {
    # A finite number in the C locale; NA, empty and non-finite values are missing.
    $parsed = 0.0
    if (-not [double]::TryParse($Text, [Globalization.NumberStyles]::Float, [Globalization.CultureInfo]::InvariantCulture, [ref] $parsed)) { return $false }
    if ([double]::IsNaN($parsed) -or [double]::IsInfinity($parsed)) { return $false }
    $Value.Value = $parsed
    return $true
}

function Add-PmStatsCount($Counts, [string] $Key) {
    if ($Counts.ContainsKey($Key)) { $Counts[$Key] = $Counts[$Key] + 1 } else { $Counts[$Key] = 1 }
}

function ConvertTo-PmStatsCounts($Counts) {
    # A count object with its keys in ordinal order; $null for a column the CSV doesn't have.
    if ($null -eq $Counts) { return $null }
    $keys = [Collections.Generic.List[string]]::new()
    foreach ($key in $Counts.Keys) { $keys.Add($key) }
    $keys.Sort([StringComparer]::Ordinal)
    $out = [Collections.Specialized.OrderedDictionary]::new()
    foreach ($key in $keys) { $out.Add($key, $Counts[$key]) }
    return $out
}

function New-PmStatsCounter([bool] $HasColumn) {
    if (-not $HasColumn) { return $null }
    return [Collections.Generic.Dictionary[string, int]]::new([StringComparer]::Ordinal)
}

function Get-PmStats {
    # One CSV's text -> one process's statistics (research 17 section 5.1).
    param(
        [string] $Text,
        [string] $Exe,
        [string] $Tag,
        [object] $ProcessId = $null,
        [switch] $Strict
    )
    Assert-PmStatsArguments $Exe $Tag
    $wantId = ConvertTo-PmStatsProcessId $ProcessId
    $inv = [Globalization.CultureInfo]::InvariantCulture

    if ($null -eq $Text) { $Text = '' }
    if ($Text.Length -gt 0 -and $Text[0] -eq [char]0xFEFF) { $Text = $Text.Substring(1) }
    if ($Text.Length -eq 0) { throw (New-PmStatsError 3 'the CSV is empty') }

    # --- The header: columns are read by name. Under --qpc_time the time column is TimeInQPC
    # (research 13's TimeInMs is the default), and the README's v2 names are --v2_metrics' own.
    $lines = $Text.Split([char]10)
    $headerLine = $lines[0].TrimEnd([char]13)
    if ($headerLine.Length -eq 0) { throw (New-PmStatsError 3 'no header line') }
    $header = $headerLine.Split([char]',')
    $column = [Collections.Generic.Dictionary[string, int]]::new([StringComparer]::Ordinal)
    for ($c = 0; $c -lt $header.Length; $c++) {
        if (-not $column.ContainsKey($header[$c])) { $column[$header[$c]] = $c }
    }
    if (-not $column.ContainsKey('MsBetweenPresents')) {
        throw (New-PmStatsError 3 'missing MsBetweenPresents (a --v2_metrics CSV?)')
    }
    foreach ($name in @('Application', 'ProcessID', 'SwapChainAddress', 'PresentMode', 'MsUntilDisplayed', 'MsGPUBusy', 'MsGPUWait')) {
        if (-not $column.ContainsKey($name)) { throw (New-PmStatsError 3 ('missing ' + $name)) }
    }
    $timeName = $null
    foreach ($name in @('TimeInQPC', 'TimeInMs', 'TimeInSeconds', 'TimeInDateTime')) {
        if ($null -eq $timeName -and $column.ContainsKey($name)) { $timeName = $name }
    }
    if ($null -eq $timeName) {
        throw (New-PmStatsError 3 'missing a time column (TimeInQPC, TimeInMs, TimeInSeconds or TimeInDateTime)')
    }
    if ($Strict) {
        # The bench's flags (M1:74) make these, so a CSV made without them can't pass.
        foreach ($name in @('TimeInQPC', 'FrameType', 'MsVideoBusy')) {
            if (-not $column.ContainsKey($name)) { throw (New-PmStatsError 3 ('missing ' + $name + ' (-Strict)')) }
        }
    }
    $iApp = $column['Application']
    $iId = $column['ProcessID']
    $iChain = $column['SwapChainAddress']
    $iMode = $column['PresentMode']
    $iMs = $column['MsBetweenPresents']
    $iUntil = $column['MsUntilDisplayed']
    $iBusy = $column['MsGPUBusy']
    $iWait = $column['MsGPUWait']
    $iTime = $column[$timeName]
    $iType = -1
    if ($column.ContainsKey('FrameType')) { $iType = $column['FrameType'] }
    $iGpuTime = -1
    if ($column.ContainsKey('MsGPUTime')) { $iGpuTime = $column['MsGPUTime'] }
    $iDisplay = -1
    if ($column.ContainsKey('MsBetweenDisplayChange')) { $iDisplay = $column['MsBetweenDisplayChange'] }
    $iSync = -1
    if ($column.ContainsKey('SyncInterval')) { $iSync = $column['SyncInterval'] }
    $iTearing = -1
    if ($column.ContainsKey('AllowsTearing')) { $iTearing = $column['AllowsTearing'] }
    $iFlags = -1
    if ($column.ContainsKey('PresentFlags')) { $iFlags = $column['PresentFlags'] }

    # --- Rows. Never quoted, so a name with a comma shifts its row: a different field count is a
    # bad row, and so is a last line without a newline (a killed PresentMon's cut tail).
    $fieldCount = $header.Length
    $badRows = 0
    $otherRows = 0
    $otherIdRows = 0
    $rows = [Collections.Generic.List[object]]::new()
    $ids = [Collections.Generic.List[long]]::new()
    $last = $lines.Length - 1
    for ($li = 1; $li -le $last; $li++) {
        $line = $lines[$li]
        if ($li -eq $last) {
            if ($line.Length -gt 0) { $badRows++ }
            break
        }
        $fields = $line.TrimEnd([char]13).Split([char]',')
        if ($fields.Length -ne $fieldCount) { $badRows++; continue }
        if (-not [string]::Equals($fields[$iApp], $Exe, [StringComparison]::OrdinalIgnoreCase)) { $otherRows++; continue }
        $rowId = [long]0
        if (-not [long]::TryParse($fields[$iId], [Globalization.NumberStyles]::None, $inv, [ref] $rowId)) { $badRows++; continue }
        if ($null -ne $wantId -and $rowId -ne $wantId) { $otherIdRows++; continue }
        if (-not $ids.Contains($rowId)) { $ids.Add($rowId) }
        $rows.Add($fields)
    }
    # Counts only: another program's name never leaves this function.
    if ($rows.Count -eq 0) {
        if ($null -ne $wantId -and $otherIdRows -gt 0) {
            throw (New-PmStatsError 3 ('no row for process id ' + $wantId + ' (' + $otherIdRows + ' rows of ' + $Exe + ' with other ids)'))
        }
        throw (New-PmStatsError 3 ('no row for ' + $Exe + ' (' + $otherRows + ' rows of other processes, ' + $badRows + ' bad rows)'))
    }
    if ($ids.Count -gt 1) {
        throw (New-PmStatsError 3 ($ids.Count.ToString($inv) + ' processes named ' + $Exe + ': give -ProcessId'))
    }

    # --- The main swap chain: the most rows; a tie goes to the chain seen first.
    $chainRows = [Collections.Generic.Dictionary[string, int]]::new([StringComparer]::Ordinal)
    $chainOrder = [Collections.Generic.List[string]]::new()
    foreach ($fields in $rows) {
        $chain = $fields[$iChain]
        if ($chainRows.ContainsKey($chain)) { $chainRows[$chain] = $chainRows[$chain] + 1 }
        else { $chainRows[$chain] = 1; $chainOrder.Add($chain) }
    }
    $main = $chainOrder[0]
    foreach ($chain in $chainOrder) {
        if ($chainRows[$chain] -gt $chainRows[$main]) { $main = $chain }
    }

    # --- Presents: the main chain's rows grouped by the time column. Frame generation gives one
    # row per displayed instance with the same time; only the application instance carries the
    # GPU and CPU numbers, so a present's numbers come from its first Application row.
    $presentAt = [Collections.Generic.Dictionary[string, int]]::new([StringComparer]::Ordinal)
    $presents = [Collections.Generic.List[object]]::new()
    foreach ($fields in $rows) {
        if (-not [string]::Equals($fields[$iChain], $main, [StringComparison]::Ordinal)) { continue }
        $key = $fields[$iTime]
        $at = 0
        if ($presentAt.TryGetValue($key, [ref] $at)) {
            $presents[$at].Add($fields)
        } else {
            $group = [Collections.Generic.List[object]]::new()
            $group.Add($fields)
            $presentAt[$key] = $presents.Count
            $presents.Add($group)
        }
    }

    $frameMs = [Collections.Generic.List[double]]::new()
    $busy = [Collections.Generic.List[double]]::new()
    $wait = [Collections.Generic.List[double]]::new()
    $gpuTime = [Collections.Generic.List[double]]::new()
    $display = [Collections.Generic.List[double]]::new()
    $dropped = 0
    $modes = New-PmStatsCounter $true
    $types = New-PmStatsCounter ($iType -ge 0)
    $sync = New-PmStatsCounter ($iSync -ge 0)
    $tearing = New-PmStatsCounter ($iTearing -ge 0)
    $flags = New-PmStatsCounter ($iFlags -ge 0)
    $v = 0.0
    foreach ($group in $presents) {
        $chosen = $group[0]
        if ($iType -ge 0) {
            foreach ($candidate in $group) {
                if ([string]::Equals($candidate[$iType], 'Application', [StringComparison]::Ordinal)) { $chosen = $candidate; break }
            }
        }
        # Frames are presents whose MsBetweenPresents is above 0; the others are bad rows.
        if (-not (Test-PmStatsNumber $chosen[$iMs] ([ref] $v)) -or $v -le 0) {
            $badRows++
            continue
        }
        $frameMs.Add($v)
        # Missing values (NA, empty, not finite) are skipped; zeros are kept.
        if (Test-PmStatsNumber $chosen[$iBusy] ([ref] $v)) { $busy.Add($v) }
        if (Test-PmStatsNumber $chosen[$iWait] ([ref] $v)) { $wait.Add($v) }
        if ($iGpuTime -ge 0 -and (Test-PmStatsNumber $chosen[$iGpuTime] ([ref] $v))) { $gpuTime.Add($v) }
        if ($iDisplay -ge 0 -and (Test-PmStatsNumber $chosen[$iDisplay] ([ref] $v))) { $display.Add($v) }
        if (-not (Test-PmStatsNumber $chosen[$iUntil] ([ref] $v))) { $dropped++ }
        Add-PmStatsCount $modes $chosen[$iMode]
        if ($null -ne $types) { foreach ($instance in $group) { Add-PmStatsCount $types $instance[$iType] } }
        if ($null -ne $sync) { Add-PmStatsCount $sync $chosen[$iSync] }
        if ($null -ne $tearing) { Add-PmStatsCount $tearing $chosen[$iTearing] }
        if ($null -ne $flags) { Add-PmStatsCount $flags $chosen[$iFlags] }
    }
    $n = $frameMs.Count
    if ($n -eq 0) { throw (New-PmStatsError 3 'no valid frame') }

    # --- Statistics. avg_fps is time-weighted and keeps dropped presents (the app made them);
    # the lows are nearest-rank in integers; nothing is trimmed, by time or otherwise.
    $sum = 0.0
    $max = 0.0
    $long = 0
    foreach ($f in $frameMs) {
        $sum += $f
        if ($f -gt $max) { $max = $f }
        if ($f -gt 250) { $long++ }
    }
    $sorted = $frameMs.ToArray()
    [Array]::Sort($sorted)
    $p99 = $sorted[(Get-PmStatsRank $n 99 100) - 1]
    $p999 = $sorted[(Get-PmStatsRank $n 999 1000) - 1]

    $stats = [ordered]@{}
    $stats['pm_stats'] = 1
    $stats['tag'] = $Tag
    $stats['frames'] = $n
    $stats['avg_fps'] = Get-PmStatsRounded (1000.0 * $n / $sum) 2
    $stats['low1_fps'] = Get-PmStatsRounded (1000.0 / $p99) 2
    $stats['low01_fps'] = Get-PmStatsRounded (1000.0 / $p999) 2
    $stats['gpu_busy_ms'] = Get-PmStatsRounded (Get-PmStatsMean $busy) 3
    $stats['gpu_wait_ms'] = Get-PmStatsRounded (Get-PmStatsMean $wait) 3
    $stats['present_modes'] = ConvertTo-PmStatsCounts $modes
    $stats['frame_types'] = ConvertTo-PmStatsCounts $types
    $stats['low_def'] = 'p99_nearest_rank'
    $stats['gpu_time_ms'] = Get-PmStatsRounded (Get-PmStatsMean $gpuTime) 3
    $stats['gpu_busy_p99_ms'] = Get-PmStatsRounded (Get-PmStatsP99 $busy) 3
    $stats['gpu_wait_p99_ms'] = Get-PmStatsRounded (Get-PmStatsP99 $wait) 3
    $stats['gpu_time_p99_ms'] = Get-PmStatsRounded (Get-PmStatsP99 $gpuTime) 3
    $stats['display_change_ms'] = Get-PmStatsRounded (Get-PmStatsMean $display) 3
    $stats['display_change_p99_ms'] = Get-PmStatsRounded (Get-PmStatsP99 $display) 3
    $stats['max_frame_ms'] = Get-PmStatsRounded $max 3
    $stats['long_frames'] = $long
    $stats['sync_interval'] = ConvertTo-PmStatsCounts $sync
    $stats['allows_tearing'] = ConvertTo-PmStatsCounts $tearing
    $stats['present_flags'] = ConvertTo-PmStatsCounts $flags
    $stats['duration_s'] = Get-PmStatsRounded ($sum / 1000.0) 3
    $stats['dropped_pct'] = Get-PmStatsRounded (100.0 * $dropped / $n) 2
    $stats['swapchains'] = $chainOrder.Count
    $stats['rows'] = $rows.Count
    $stats['bad_rows'] = $badRows

    $warnings = [Collections.Generic.List[string]]::new()
    if ($chainOrder.Count -gt 1) {
        $warnings.Add('swap chains: ' + $chainOrder.Count + '; the statistics use the one with ' + $chainRows[$main] + ' of ' + $rows.Count + ' rows')
    }
    if ($badRows -gt 0) { $warnings.Add('bad rows: ' + $badRows) }
    if ($n -lt 1000) { $warnings.Add('frames: ' + $n + ', fewer than 1000') }
    if ($long -gt 0) {
        $warnings.Add('frames over 250 ms: ' + $long + ', the longest ' + $stats['max_frame_ms'].ToString($inv) + ' ms')
    }
    if ($null -eq $types) {
        $warnings.Add('no FrameType column: frame_types is null')
    } else {
        $otherTypes = [Collections.Generic.List[string]]::new()
        foreach ($type in $stats['frame_types'].Keys) { if ($type -cne 'Application') { $otherTypes.Add($type) } }
        if ($otherTypes.Count -gt 0) { $warnings.Add('frame types other than Application: ' + ($otherTypes -join ', ')) }
    }

    return [pscustomobject]@{
        Stats    = $stats
        Json     = (ConvertTo-Json -InputObject $stats -Compress -Depth 5)
        Warnings = $warnings.ToArray()
    }
}

function Get-PmStatsLeafName([string] $File) {
    # The file name alone: output never names a folder.
    if ($null -eq $File) { return '' }
    $cut = $File.LastIndexOfAny([char[]]@('\', '/'))
    if ($cut -ge 0) { return $File.Substring($cut + 1) }
    return $File
}

function Read-PmStatsFile([string] $File) {
    # The CSV's bytes, at most 64 MiB (research 17 section 4.6), with errors that name the file
    # only. A relative name is taken from the current folder.
    $leaf = Get-PmStatsLeafName $File
    try {
        $full = [IO.Path]::GetFullPath([IO.Path]::Combine((Get-Location -PSProvider FileSystem).ProviderPath, $File))
        $info = [IO.FileInfo]::new($full)
        if (-not $info.Exists) { throw (New-PmStatsError 3 ('no such file: ' + $leaf)) }
        if ($info.Length -gt 67108864) { throw (New-PmStatsError 3 ($leaf + ' is larger than 64 MiB')) }
        return ,[IO.File]::ReadAllBytes($full)
    } catch {
        if ($_.Exception.Data.Contains('KairoExit')) { throw }
        throw (New-PmStatsError 3 ('can''t read ' + $leaf + ' (' + $_.Exception.GetType().Name + ')'))
    }
}

function ConvertFrom-PmStatsArguments([object[]] $Arguments) {
    # pwsh -File passes every argument as text: -Name:value arrives as -Name and value, and
    # -Name:$false as -Name and a bool. & passes -Name: and the value, a switch's as a bool, and
    # numbers as numbers. Names are matched whole, ignoring case; anything else is refused.
    $values = @('Csv', 'Exe', 'Tag', 'ProcessId')
    $switches = @('Strict')
    $parsed = @{}
    $i = 0
    while ($i -lt $Arguments.Count) {
        $token = $Arguments[$i]
        $i++
        $match = $null
        if ($token -is [string]) { $match = [regex]::Match($token, '^-([A-Za-z]+):?\z') }
        if ($null -eq $match -or -not $match.Success) {
            throw (New-PmStatsError 2 ('unexpected argument at position ' + $i))
        }
        $name = $null
        foreach ($known in ($values + $switches)) {
            if ([string]::Equals($known, $match.Groups[1].Value, [StringComparison]::OrdinalIgnoreCase)) { $name = $known }
        }
        if ($null -eq $name) { throw (New-PmStatsError 2 ('unknown parameter -' + $match.Groups[1].Value)) }
        if ($parsed.ContainsKey($name)) { throw (New-PmStatsError 2 ('-' + $name + ' given twice')) }
        if ($switches -contains $name) {
            $parsed[$name] = $true
            if ($i -lt $Arguments.Count -and $Arguments[$i] -is [bool]) {
                $parsed[$name] = $Arguments[$i]
                $i++
            }
            continue
        }
        if ($i -ge $Arguments.Count) { throw (New-PmStatsError 2 ('-' + $name + ' needs a value')) }
        $parsed[$name] = $Arguments[$i]
        $i++
    }
    return $parsed
}

# --- Command line: only when the file is run, never when it is dot-sourced. -----------------
if ($MyInvocation.InvocationName -ne '.') {
    Set-StrictMode -Version 3.0
    $ErrorActionPreference = 'Stop'
    $pmStatsCode = 0
    $pmStatsCsv = ''
    try {
        $pmStatsArgs = ConvertFrom-PmStatsArguments $args
        foreach ($required in @('Csv', 'Exe', 'Tag')) {
            if (-not $pmStatsArgs.ContainsKey($required) -or [string]::IsNullOrEmpty([string]$pmStatsArgs[$required])) {
                throw (New-PmStatsError 2 ('missing -' + $required))
            }
        }
        $pmStatsExe = [string]$pmStatsArgs['Exe']
        $pmStatsTag = [string]$pmStatsArgs['Tag']
        $pmStatsId = $null
        if ($pmStatsArgs.ContainsKey('ProcessId')) { $pmStatsId = $pmStatsArgs['ProcessId'] }
        # The command line is checked before the file is read.
        Assert-PmStatsArguments $pmStatsExe $pmStatsTag
        $null = ConvertTo-PmStatsProcessId $pmStatsId
        $pmStatsStrict = $pmStatsArgs.ContainsKey('Strict') -and [bool]$pmStatsArgs['Strict']
        $pmStatsCsv = [string]$pmStatsArgs['Csv']
        $pmStatsText = ConvertFrom-PmCsvBytes (Read-PmStatsFile $pmStatsCsv)
        $pmStatsResult = Get-PmStats -Text $pmStatsText -Exe $pmStatsExe -Tag $pmStatsTag -ProcessId $pmStatsId -Strict:$pmStatsStrict
        foreach ($warning in $pmStatsResult.Warnings) { [Console]::Error.WriteLine('pm-stats: warning: ' + $warning) }
        [Console]::Out.WriteLine($pmStatsResult.Json)
    } catch {
        $pmStatsCode = Get-PmStatsExitCode $_
        $message = $_.Exception.Message
        if (-not $_.Exception.Data.Contains('KairoExit')) {
            # A bug. Its message could quote a path or a field of another program's row, so only
            # its type and line are printed.
            $line = 0
            if ($null -ne $_.InvocationInfo) { $line = $_.InvocationInfo.ScriptLineNumber }
            $message = 'internal error: ' + $_.Exception.GetType().Name + ' at line ' + $line
        }
        [Console]::Error.WriteLine('pm-stats: ' + $message)
    }
    exit $pmStatsCode
}
