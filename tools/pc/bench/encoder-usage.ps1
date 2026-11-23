# SPDX-License-Identifier: GPL-3.0-or-later
#
# tools/pc/bench/encoder-usage.ps1 - which programs use the video engines: use per process, adapter
# LUID and engine type, from PDH's "GPU Engine" counters, with file names only.
# Project: https://github.com/Maty3k/Kairo
# Design: docs/research/17-loadgen-bench/README.md section 5.3. Contract: docs/milestones/M1.md, Formats.
#
# Command line:
#   pwsh -NoProfile -File encoder-usage.ps1 -Seconds <1-600> [-Json]
# -Json prints one line:
#   {"kairo_encoder_usage":1,"seconds":3,"samples":3,"invalid_samples":0,"elevated":true,
#    "types":{"3d":24,"copy":12,"videocodec":4},"video":[{"name":"...","pid":5120,
#    "luid":"0x00000000000123ab","engtype":"videocodec","eng":4,"avg_pct":3.1,"max_pct":7.5,"seen":3}]}
# Without -Json it prints a summary and a table of the rows whose max_pct is above 0. Warnings go to
# stderr, one line each. Exit codes: 0 sampled; 2 bad command line; 3 the counter set can't be read
# (missing, as on a CI runner or off Windows, or under a localized name).
# It only reads. Sampling costs CPU, so the scripts that call it check the busy state first, and in
# M1 it never runs during a recorded run (research 17 sections 4.6 and 5.3).
#
# Sampling: one Get-Counter -MaxSamples 1 call a second, N times. Each call expands the wildcard
# again, so a program that starts using an engine is seen from the next second, and Get-Counter drops
# its own priming read, so every set it returns is a rate over its second. A sample whose Status isn't
# 0 (a program exited mid-sample) is dropped and counted.
#
# The result:
# - samples counts the sets Get-Counter returned, invalid_samples the samples dropped; elevated says
#   whether this session has an administrator's full token (null off Windows);
# - types is a census of every engine type seen, lower-cased with spaces removed: how many engine
#   instances (process, adapter, engine) have that type, each counted once however often it was seen;
# - video has a row per video-engine instance (its type starts with "video"; AMD's videocodec counts
#   decoding too), sorted by process id, LUID and engine. avg_pct and max_pct are over the samples
#   that held the instance, and seen is their number. The LUID is in kairo-display's form, 0x and 16
#   hex digits, high part first. name is the process's file name from the second snapshot, or
#   "(exited)" when its id is gone by then or names another program than in the first (a reused id).
#
# House rule (CLAUDE.md, Never 3): process names come from one [Diagnostics.Process]::GetProcesses()
# snapshot before sampling and one after, reading Id and ProcessName only, in Get-ProcessIdAndName,
# the one helper that may call it. ProcessName is the snapshot's name: no process is opened. A
# process's Path, MainModule, Modules, Company, FileVersion, ProductVersion, Description or Product
# goes through MainModule, which opens the process, a game included, with PROCESS_QUERY_INFORMATION |
# PROCESS_VM_READ and lists its modules; CommandLine is a WMI query for its command line. None is read,
# and no Process object is ever formatted or serialized whole.
#
# Privacy: the output names every program that uses a video engine, by file name, so it stays in
# .dogfood/ (CLAUDE.md). A counter sample's own path starts with the computer's name and is never
# read, and Get-Counter's error messages are never printed (PDH's can name the computer): only their
# ids are. An unexpected error prints its type and line, never its message.
#
# As a library (dot-sourced; the elevated session does this with the text it received on stdin):
#   $r = Get-EncoderUsage -Seconds 3   # $r.Usage (an ordered dictionary), $r.Json, $r.Table, $r.Warnings
#   Measure-EncoderUsage turns given sample sets and snapshots into the same result; the tests use it.
# Functions only return or throw. A thrown exception carries its exit code in .Data['KairoExit'].
# Dot-sourcing defines functions and nothing else; the command line at the bottom runs only when the
# file itself is run, and it never prompts. Load the text with '.', from the file or through
# [scriptblock]::Create: Invoke-Expression reports the caller's name, so the command line would run.
#
# Rules: ASCII only; Windows PowerShell 5.1 syntax and .NET Framework 4.x APIs (it runs on
# PowerShell 7); no Add-Type; clean under Set-StrictMode -Version 3.0.

function New-EncoderUsageError([int] $Code, [string] $Message) {
    $exception = [Exception]::new($Message)
    $exception.Data['KairoExit'] = $Code
    return $exception
}

function Get-EncoderUsageExitCode($ErrorRecord) {
    # The exit code a caught error stands for: its own, or 3 (can't sample).
    $exception = $ErrorRecord.Exception
    while ($null -ne $exception) {
        if ($exception.Data.Contains('KairoExit')) { return [int]$exception.Data['KairoExit'] }
        $exception = $exception.InnerException
    }
    return 3
}

function ConvertTo-EncoderUsageSeconds($Value) {
    # -Seconds: a whole number from 1 to 600, given as a number or as text.
    $text = ''
    if ($null -ne $Value -and -not ($Value -is [bool])) { $text = ([string]$Value).Trim() }
    if ($text -cnotmatch '^[0-9]{1,3}\z') { throw (New-EncoderUsageError 2 '-Seconds must be a whole number from 1 to 600') }
    $seconds = [int]::Parse($text, [Globalization.CultureInfo]::InvariantCulture)
    if ($seconds -lt 1 -or $seconds -gt 600) { throw (New-EncoderUsageError 2 '-Seconds must be a whole number from 1 to 600') }
    return $seconds
}

function Get-ProcessIdAndName {
    # The one helper that may call GetProcesses (research 17 sections 5.3 and 6): one
    # [pscustomobject]@{Id; Name} per process. GetProcesses builds every object from one
    # NtQuerySystemInformation snapshot, and Id and ProcessName come from that snapshot without
    # opening the process. Nothing else of a process is read, and no Process object leaves here.
    $list = [Collections.Generic.List[object]]::new()
    foreach ($process in [Diagnostics.Process]::GetProcesses()) {
        try {
            $list.Add([pscustomobject]@{ Id = [long]$process.Id; Name = [string]$process.ProcessName })
        } catch {
            # A process that can't be named (gone already, or off Windows) is left out.
        } finally {
            $process.Dispose()
        }
    }
    return ,$list.ToArray()
}

function ConvertFrom-GpuEngineInstance([string] $Name) {
    # pid_<PID>_luid_0x<HIGH>_0x<LOW>_phys_<N>_eng_<N>_engtype_<Type>, which Get-Counter lower-cases
    # -> Pid, Luid in kairo-display's form (0x and 16 hex digits, high part first, lower case), Phys,
    # Eng and EngType (lower-cased, spaces removed); $null when the name doesn't match. Case is
    # ignored, so a name that isn't lower-cased reads the same.
    if ([string]::IsNullOrEmpty($Name)) { return $null }
    $options = [Text.RegularExpressions.RegexOptions]::IgnoreCase -bor [Text.RegularExpressions.RegexOptions]::CultureInvariant
    $match = [regex]::Match($Name, '^pid_([0-9]+)_luid_0x([0-9a-f]{8})_0x([0-9a-f]{8})_phys_([0-9]+)_eng_([0-9]+)_engtype_(.+)\z', $options)
    if (-not $match.Success) { return $null }
    $inv = [Globalization.CultureInfo]::InvariantCulture
    $digits = [Globalization.NumberStyles]::None
    $id = [long]0
    $phys = 0
    $eng = 0
    if (-not [long]::TryParse($match.Groups[1].Value, $digits, $inv, [ref] $id)) { return $null }
    if (-not [int]::TryParse($match.Groups[4].Value, $digits, $inv, [ref] $phys)) { return $null }
    if (-not [int]::TryParse($match.Groups[5].Value, $digits, $inv, [ref] $eng)) { return $null }
    $type = $match.Groups[6].Value.Replace(' ', '').ToLowerInvariant()
    if ($type.Length -eq 0) { return $null }
    return [pscustomobject]@{
        Pid     = $id
        Luid    = ('0x' + $match.Groups[2].Value + $match.Groups[3].Value).ToLowerInvariant()
        Phys    = $phys
        Eng     = $eng
        EngType = $type
    }
}

function Test-EncoderUsageVideoType([string] $EngType) {
    # A video engine: its type, lower-cased with spaces removed, starts with "video" (cookbook 9.6).
    # AMD reports videocodec, which counts decoding too (research 13 section 5.3).
    if ($null -eq $EngType) { return $false }
    return $EngType.StartsWith('video', [StringComparison]::Ordinal)
}

function Get-EncoderUsageErrorId($Record) {
    # An error's id without its message, such as CounterApiError: PDH's messages can name the computer.
    $id = ''
    if ($Record -is [Management.Automation.ErrorRecord]) {
        $id = [string]$Record.FullyQualifiedErrorId
    } elseif ($Record -is [Management.Automation.IContainsErrorRecord]) {
        $id = [string]$Record.ErrorRecord.FullyQualifiedErrorId
    } elseif ($null -ne $Record) {
        $id = $Record.GetType().Name
    }
    $cut = $id.IndexOf([char]',')
    if ($cut -ge 0) { $id = $id.Substring(0, $cut) }
    if ($id -cnotmatch '^[A-Za-z0-9_.]{1,64}\z') { $id = 'unknown' }
    return $id
}

function Invoke-EncoderUsageCounter {
    # One Get-Counter call: one sample set over about a second. A program that exits mid-sample leaves
    # a sample whose Status isn't 0, and Get-Counter then writes a non-terminating error, which would
    # throw under the callers' $ErrorActionPreference = 'Stop'. So errors are silenced here and the set
    # still comes back; a terminating one (a missing Get-Counter included) leaves this second empty.
    # -> Sets (none when the read failed) and Errors (the errors' ids, never their messages).
    $sets = @()
    $pdhErrors = @()
    $ids = [Collections.Generic.List[string]]::new()
    try {
        $sets = @(Get-Counter -Counter '\GPU Engine(*)\Utilization Percentage' -MaxSamples 1 -ErrorAction SilentlyContinue -ErrorVariable pdhErrors)
    } catch {
        $sets = @()
        $ids.Add((Get-EncoderUsageErrorId $_))
    }
    foreach ($record in @($pdhErrors)) {
        if ($null -eq $record) { continue }
        $id = Get-EncoderUsageErrorId $record
        if (-not $ids.Contains($id)) { $ids.Add($id) }
    }
    return [pscustomobject]@{ Sets = $sets; Errors = $ids.ToArray() }
}

function Test-EncoderUsageElevated {
    # Whether this session has an administrator's full token; $null off Windows, or when unknown.
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { return $null }
    try {
        $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
        try {
            return ([Security.Principal.WindowsPrincipal]::new($identity)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
        } finally {
            $identity.Dispose()
        }
    } catch {
        return $null
    }
}

function Get-EncoderUsageRounded([double] $Value) {
    # Percentages to 2 decimals, AwayFromZero, as pm-stats rounds them; -0 prints as 0.
    $rounded = [Math]::Round($Value, 2, [MidpointRounding]::AwayFromZero)
    if ($rounded -eq 0) { $rounded = 0.0 }
    return $rounded
}

function Get-EncoderUsageNames($Snapshot) {
    # A snapshot from Get-ProcessIdAndName -> a dictionary from process id to name.
    $names = [Collections.Generic.Dictionary[long, string]]::new()
    if ($null -eq $Snapshot) { return ,$names }
    foreach ($entry in $Snapshot) {
        if ($null -eq $entry) { continue }
        $names[[long]$entry.Id] = [string]$entry.Name
    }
    return ,$names
}

function Get-EncoderUsageName([long] $Id, $First, $Second) {
    # A sampled process id's name: the second snapshot's; "(exited)" for an id gone by then, or for
    # one whose name changed between the snapshots (a reused id: the program sampled has exited).
    $name = $null
    if (-not $Second.TryGetValue($Id, [ref] $name)) { return '(exited)' }
    $earlier = $null
    if ($First.TryGetValue($Id, [ref] $earlier) -and -not [string]::Equals($earlier, $name, [StringComparison]::Ordinal)) {
        return '(exited)'
    }
    return $name
}

function Measure-EncoderUsage {
    # Sample sets and the two snapshots -> the result (research 17 section 5.3):
    # - samples counts the sets returned; invalid_samples the samples whose Status isn't 0, which are
    #   dropped;
    # - types is a census of every engine type seen, each engine instance counted once;
    # - video has a row per video-engine instance: avg_pct over the samples that held it, max_pct, and
    #   seen, their number. Rows are sorted by process id, LUID, physical adapter, engine and type.
    # No set at all means the counter set can't be read: exit 3, with Get-Counter's error ids.
    param(
        [object[]] $Sets,
        [object[]] $Before,
        [object[]] $After,
        [int] $Seconds,
        $Elevated = $null,
        [string[]] $Errors = @()
    )
    $inv = [Globalization.CultureInfo]::InvariantCulture
    $errorIds = @($Errors | Where-Object { -not [string]::IsNullOrEmpty($_) })
    $census = [Collections.Generic.SortedDictionary[string, object]]::new([StringComparer]::Ordinal)
    $rows = [Collections.Generic.SortedDictionary[string, object]]::new([StringComparer]::Ordinal)
    $setCount = 0
    $invalid = 0
    $unknown = 0
    foreach ($set in @($Sets)) {
        if ($null -eq $set) { continue }
        $setCount++
        $property = $set.PSObject.Properties['CounterSamples']
        if ($null -eq $property -or $null -eq $property.Value) { continue }
        foreach ($sample in @($property.Value)) {
            if ($null -eq $sample) { continue }
            if ([long]$sample.Status -ne 0) { $invalid++; continue }
            $instanceName = [string]$sample.InstanceName
            $instance = ConvertFrom-GpuEngineInstance $instanceName
            if ($null -eq $instance) {
                if (-not [string]::Equals($instanceName, '_total', [StringComparison]::OrdinalIgnoreCase)) { $unknown++ }
                continue
            }
            # The key sorts as the rows do: numbers padded, text compared ordinally.
            $key = $instance.Pid.ToString('D20', $inv) + '|' + $instance.Luid + '|' + $instance.Phys.ToString('D10', $inv) + '|' + $instance.Eng.ToString('D10', $inv) + '|' + $instance.EngType
            $ofType = $null
            if (-not $census.TryGetValue($instance.EngType, [ref] $ofType)) {
                $ofType = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
                $census.Add($instance.EngType, $ofType)
            }
            [void]$ofType.Add($key)
            if (-not (Test-EncoderUsageVideoType $instance.EngType)) { continue }
            $value = [double]$sample.CookedValue
            if ([double]::IsNaN($value) -or [double]::IsInfinity($value)) { continue }
            $row = $null
            if (-not $rows.TryGetValue($key, [ref] $row)) {
                $row = [pscustomobject]@{ Instance = $instance; Sum = 0.0; Max = 0.0; Seen = 0 }
                $rows.Add($key, $row)
            }
            $row.Sum += $value
            if ($row.Seen -eq 0 -or $value -gt $row.Max) { $row.Max = $value }
            $row.Seen++
        }
    }
    if ($setCount -eq 0) {
        $message = 'the GPU Engine counter set can''t be read (missing here, or under a localized name)'
        if ($errorIds.Count -gt 0) { $message += '; Get-Counter: ' + ($errorIds -join ', ') }
        throw (New-EncoderUsageError 3 $message)
    }

    $first = Get-EncoderUsageNames $Before
    $second = Get-EncoderUsageNames $After
    $video = [Collections.Generic.List[object]]::new()
    foreach ($row in $rows.Values) {
        $entry = [ordered]@{}
        $entry['name'] = Get-EncoderUsageName $row.Instance.Pid $first $second
        $entry['pid'] = $row.Instance.Pid
        $entry['luid'] = $row.Instance.Luid
        $entry['engtype'] = $row.Instance.EngType
        $entry['eng'] = $row.Instance.Eng
        $entry['avg_pct'] = Get-EncoderUsageRounded ($row.Sum / $row.Seen)
        $entry['max_pct'] = Get-EncoderUsageRounded $row.Max
        $entry['seen'] = $row.Seen
        $video.Add($entry)
    }
    $types = [Collections.Specialized.OrderedDictionary]::new()
    foreach ($pair in $census.GetEnumerator()) { $types.Add($pair.Key, $pair.Value.Count) }

    $usage = [ordered]@{}
    $usage['kairo_encoder_usage'] = 1
    $usage['seconds'] = $Seconds
    $usage['samples'] = $setCount
    $usage['invalid_samples'] = $invalid
    $usage['elevated'] = $null
    if ($Elevated -is [bool]) { $usage['elevated'] = $Elevated }
    $usage['types'] = $types
    $usage['video'] = $video

    $warnings = [Collections.Generic.List[string]]::new()
    if ($setCount -lt $Seconds) {
        $text = ($Seconds - $setCount).ToString($inv) + ' of ' + $Seconds.ToString($inv) + ' seconds returned no sample set'
        if ($errorIds.Count -gt 0) { $text += ' (Get-Counter: ' + ($errorIds -join ', ') + ')' }
        $warnings.Add($text)
    }
    if ($unknown -gt 0) {
        $warnings.Add($unknown.ToString($inv) + ' samples skipped: their instance name isn''t pid_<id>_luid_0x<high>_0x<low>_phys_<n>_eng_<n>_engtype_<type>')
    }
    return [pscustomobject]@{
        Usage    = $usage
        Json     = (ConvertTo-Json -InputObject $usage -Compress -Depth 5)
        Table    = (Format-EncoderUsageTable $usage)
        Warnings = $warnings.ToArray()
    }
}

function Format-EncoderUsageTable($Usage) {
    # The terminal's view: a summary, the census, and the rows whose max_pct is above 0.
    $inv = [Globalization.CultureInfo]::InvariantCulture
    $lines = [Collections.Generic.List[string]]::new()
    $elevated = 'unknown'
    if ($Usage['elevated'] -is [bool]) {
        if ($Usage['elevated']) { $elevated = 'yes' } else { $elevated = 'no' }
    }
    $lines.Add([string]::Format($inv, 'encoder use: {0} s, {1} samples ({2} invalid), elevated: {3}', $Usage['seconds'], $Usage['samples'], $Usage['invalid_samples'], $elevated))
    $census = [Collections.Generic.List[string]]::new()
    foreach ($pair in $Usage['types'].GetEnumerator()) { $census.Add([string]::Format($inv, '{0} {1}', $pair.Key, $pair.Value)) }
    if ($census.Count -eq 0) { $census.Add('none') }
    $lines.Add('engine types seen: ' + ($census -join ', '))
    $used = [Collections.Generic.List[object]]::new()
    foreach ($row in $Usage['video']) {
        if ([double]$row['max_pct'] -gt 0) { $used.Add($row) }
    }
    if ($used.Count -eq 0) {
        $lines.Add('no video-engine use in these seconds')
    } else {
        $lines.Add([string]::Format($inv, '{0,-24} {1,10} {2,-18} {3,-12} {4,3} {5,7} {6,7} {7,4}', 'name', 'pid', 'luid', 'engtype', 'eng', 'avg %', 'max %', 'seen'))
        foreach ($row in $used) {
            $lines.Add([string]::Format($inv, '{0,-24} {1,10} {2,-18} {3,-12} {4,3} {5,7:F2} {6,7:F2} {7,4}',
                    $row['name'], $row['pid'], $row['luid'], $row['engtype'], $row['eng'], $row['avg_pct'], $row['max_pct'], $row['seen']))
        }
    }
    return ,$lines.ToArray()
}

function Get-EncoderUsage {
    # One snapshot, one Get-Counter sample a second for -Seconds, one snapshot (section 5.3).
    param($Seconds)
    $count = ConvertTo-EncoderUsageSeconds $Seconds
    $before = Get-ProcessIdAndName
    $sets = [Collections.Generic.List[object]]::new()
    $errors = [Collections.Generic.List[string]]::new()
    for ($i = 0; $i -lt $count; $i++) {
        $read = Invoke-EncoderUsageCounter
        foreach ($set in $read.Sets) { $sets.Add($set) }
        foreach ($id in $read.Errors) { if (-not $errors.Contains($id)) { $errors.Add($id) } }
    }
    $after = Get-ProcessIdAndName
    return (Measure-EncoderUsage -Sets $sets.ToArray() -Before $before -After $after -Seconds $count -Elevated (Test-EncoderUsageElevated) -Errors $errors.ToArray())
}

function ConvertFrom-EncoderUsageArguments([object[]] $Arguments) {
    # pwsh -File passes every argument as text: -Name:value arrives as -Name and value, and
    # -Name:$false as -Name and a bool. & passes -Name: and the value, a switch's as a bool, and
    # numbers as numbers. Names are matched whole, ignoring case; anything else is refused.
    $values = @('Seconds')
    $switches = @('Json')
    $parsed = @{}
    $i = 0
    while ($i -lt $Arguments.Count) {
        $token = $Arguments[$i]
        $i++
        $match = $null
        if ($token -is [string]) { $match = [regex]::Match($token, '^-([A-Za-z]+):?\z') }
        if ($null -eq $match -or -not $match.Success) {
            throw (New-EncoderUsageError 2 ('unexpected argument at position ' + $i))
        }
        $name = $null
        foreach ($known in ($values + $switches)) {
            if ([string]::Equals($known, $match.Groups[1].Value, [StringComparison]::OrdinalIgnoreCase)) { $name = $known }
        }
        if ($null -eq $name) { throw (New-EncoderUsageError 2 ('unknown parameter -' + $match.Groups[1].Value)) }
        if ($parsed.ContainsKey($name)) { throw (New-EncoderUsageError 2 ('-' + $name + ' given twice')) }
        if ($switches -contains $name) {
            $parsed[$name] = $true
            if ($i -lt $Arguments.Count -and $Arguments[$i] -is [bool]) {
                $parsed[$name] = $Arguments[$i]
                $i++
            }
            continue
        }
        if ($i -ge $Arguments.Count) { throw (New-EncoderUsageError 2 ('-' + $name + ' needs a value')) }
        $parsed[$name] = $Arguments[$i]
        $i++
    }
    return $parsed
}

# --- Command line: only when the file is run, never when it is dot-sourced. -----------------
if ($MyInvocation.InvocationName -ne '.') {
    Set-StrictMode -Version 3.0
    $ErrorActionPreference = 'Stop'
    $encoderUsageCode = 0
    try {
        $encoderUsageArgs = ConvertFrom-EncoderUsageArguments $args
        if (-not $encoderUsageArgs.ContainsKey('Seconds')) { throw (New-EncoderUsageError 2 'missing -Seconds') }
        $encoderUsageResult = Get-EncoderUsage -Seconds $encoderUsageArgs['Seconds']
        foreach ($warning in $encoderUsageResult.Warnings) { [Console]::Error.WriteLine('encoder-usage: warning: ' + $warning) }
        if ($encoderUsageArgs.ContainsKey('Json') -and [bool]$encoderUsageArgs['Json']) {
            [Console]::Out.WriteLine($encoderUsageResult.Json)
        } else {
            foreach ($line in $encoderUsageResult.Table) { [Console]::Out.WriteLine($line) }
        }
    } catch {
        $encoderUsageCode = Get-EncoderUsageExitCode $_
        $message = $_.Exception.Message
        if (-not $_.Exception.Data.Contains('KairoExit')) {
            # A bug. Its message could quote what it failed on, so only its type and line are
            # printed, as pm-stats does.
            $line = 0
            if ($null -ne $_.InvocationInfo) { $line = $_.InvocationInfo.ScriptLineNumber }
            $message = 'internal error: ' + $_.Exception.GetType().Name + ' at line ' + $line
        }
        [Console]::Error.WriteLine('encoder-usage: ' + $message)
    }
    exit $encoderUsageCode
}
