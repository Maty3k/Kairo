# SPDX-License-Identifier: GPL-3.0-or-later
#
# tools/pc/bench/tests/encoder-usage.tests.ps1 - encoder-usage.ps1's tests (research 17 section 6, PS row).
# Project: https://github.com/Maty3k/Kairo
#
# Run: pwsh -NoProfile -File tools/pc/bench/tests/encoder-usage.tests.ps1 (exits 1 if a case fails).
#
# The script runs both ways: dot-sourced under testlib's Set-StrictMode -Version 3.0 and
# $ErrorActionPreference = 'Stop', as the elevated session runs it, and through -File with its exit
# codes. Counter reads exist only on Windows, so most cases feed synthetic sample sets: to the
# functions directly (U, N, M), to Get-EncoderUsage through a stand-in Get-Counter defined inside the
# case, and to the command line (C) through a wrapper script that defines the stand-in and then runs
# encoder-usage.ps1 with &, which runs its command line as -File does. C4 alone reads the real
# counter set, for one second, and accepts exit 0 or 3 (3 off Windows and where the set is missing).
# The S cases scan the script's syntax tree for the Never list and its own rules.
#
# Fixtures are built in code; process names are synthetic (kairo-loadgen, other). Each stand-in
# sample has a Path, and each stand-in error a message, that names a made-up computer, secret-pc,
# as PDH's do; it must never reach the output.
#
# Helpers testlib.ps1 lacks, kept here: Assert-NoSecret (text that must not name secret-pc) and
# ConvertFrom-EuLine (one JSON line from a run's stdout), plus the fixture builders below.
#
# Rules: ASCII only; Windows PowerShell 5.1 syntax and .NET Framework 4.x APIs; no Add-Type.

. (Join-Path $PSScriptRoot 'testlib.ps1')
. (Join-Path $BenchDir 'encoder-usage.ps1')

$EuScript = Join-Path $BenchDir 'encoder-usage.ps1'
$EuTokens = $null
$EuParseErrors = $null
$EuAst = [Management.Automation.Language.Parser]::ParseFile($EuScript, [ref] $EuTokens, [ref] $EuParseErrors)

# Two adapters, as LUID halves in an instance name, and as kairo-display prints them.
$EuLuidA = @('00000000', '000123ab')
$EuLuidB = @('00000001', '0000d1a5')
$EuLuidAText = '0x00000000000123ab'
$EuLuidBText = '0x000000010000d1a5'

function New-EuInstance([long] $ProcessId, [string[]] $Luid, [int] $Eng, [string] $Type) {
    # A GPU Engine instance name as Get-Counter returns it.
    return ('pid_' + $ProcessId + '_luid_0x' + $Luid[0] + '_0x' + $Luid[1] + '_phys_0_eng_' + $Eng + '_engtype_' + $Type)
}

function New-EuSample([string] $Instance, [double] $Value, [long] $Status = 0) {
    # A PerformanceCounterSample's fields that encoder-usage may meet. Path names the computer, as
    # PDH's does, and must never be read.
    return [pscustomobject]@{
        Path         = '\\secret-pc\gpu engine(' + $Instance + ')\utilization percentage'
        InstanceName = $Instance
        CookedValue  = $Value
        Status       = [uint32]$Status
    }
}

function New-EuSet([object[]] $Samples = @()) {
    # A PerformanceCounterSampleSet: what one Get-Counter -MaxSamples 1 call returns.
    return [pscustomobject]@{ Timestamp = [datetime]::MinValue; CounterSamples = $Samples }
}

function New-EuProcess([long] $Id, [string] $Name) {
    # One snapshot entry, as Get-ProcessIdAndName returns it.
    return [pscustomobject]@{ Id = $Id; Name = $Name }
}

function Assert-NoSecret([string] $Text, [string] $What) {
    if ($null -ne $Text -and $Text.IndexOf('secret-pc', [StringComparison]::OrdinalIgnoreCase) -ge 0) {
        throw ($What + ': names the computer')
    }
}

function ConvertFrom-EuLine([string] $Text) {
    # A -Json run's stdout: exactly one line, which parses as JSON.
    $lines = @($Text.Split([char]10) | ForEach-Object { $_.TrimEnd([char]13) } | Where-Object { $_.Length -gt 0 })
    if ($lines.Count -ne 1) { throw ('expected one line on stdout, got ' + $lines.Count) }
    return (ConvertFrom-Json -InputObject $lines[0])
}

function Get-EuLines([string] $Text) {
    return @($Text.Split([char]10) | ForEach-Object { $_.TrimEnd([char]13) } | Where-Object { $_.Length -gt 0 })
}

function Get-EuRowText($Usage) {
    # The video rows as "pid=name" in output order.
    return (@($Usage['video'] | ForEach-Object { [string]$_['pid'] + '=' + $_['name'] }) -join ' ')
}

function New-EuThreeSets {
    # Section 5.3's example in miniature: process 200 uses the video engine of adapter A and holds an
    # idle one on adapter B; process 100 draws on both. Three sets, all valid.
    $video200A = New-EuInstance 200 $EuLuidA 4 'videocodec'
    $video200B = New-EuInstance 200 $EuLuidB 4 'videocodec'
    $d3d100A = New-EuInstance 100 $EuLuidA 0 '3d'
    $d3d100B = New-EuInstance 100 $EuLuidB 0 '3d'
    $copy100A = New-EuInstance 100 $EuLuidA 2 'copy'
    $sets = [Collections.Generic.List[object]]::new()
    foreach ($v in @(@(50, 1.2), @(60, 7.5), @(70, 0.6))) {
        $sets.Add((New-EuSet @((New-EuSample $d3d100A $v[0]), (New-EuSample $copy100A 0), (New-EuSample $video200A $v[1]), (New-EuSample $video200B 0), (New-EuSample $d3d100B 0))))
    }
    return ,$sets.ToArray()
}

# This PowerShell's own process name, which a child of the same PowerShell also has.
$EuSelfName = $null
foreach ($entry in (Get-ProcessIdAndName)) { if ($entry.Id -eq $PID) { $EuSelfName = $entry.Name } }

# --- The parser -----------------------------------------------------------------------------

Test-Case 'U1 an AMD instance name' {
    $i = ConvertFrom-GpuEngineInstance 'pid_5120_luid_0x00000000_0x000123ab_phys_0_eng_4_engtype_videocodec'
    Assert-Equal 5120 $i.Pid 'pid'
    Assert-True ($i.Pid -is [long]) 'pid is a long'
    Assert-Equal '0x00000000000123ab' $i.Luid 'luid'
    Assert-Equal 0 $i.Phys 'phys'
    Assert-Equal 4 $i.Eng 'eng'
    Assert-Equal 'videocodec' $i.EngType 'engtype'
}

Test-Case 'U2 the LUID is 0x and 16 hex digits, high part first, as kairo-display prints it' {
    $i = ConvertFrom-GpuEngineInstance 'pid_8_luid_0x00000001_0xdeadbeef_phys_1_eng_12_engtype_3d'
    Assert-Equal '0x00000001deadbeef' $i.Luid 'luid (display/lib.rs test value)'
    Assert-Equal 1 $i.Phys 'phys'
    Assert-Equal 12 $i.Eng 'eng'
    Assert-Equal '3d' $i.EngType 'engtype'
}

Test-Case 'U3 engine types are lower-cased with spaces removed' {
    Assert-Equal 'videodecode' (ConvertFrom-GpuEngineInstance 'pid_42_luid_0x00000000_0x0000abcd_phys_0_eng_2_engtype_video decode').EngType 'video decode'
    Assert-Equal 'videoencode' (ConvertFrom-GpuEngineInstance 'pid_42_luid_0x00000000_0x0000abcd_phys_0_eng_3_engtype_Video Encode').EngType 'Video Encode'
    Assert-Equal 'legacyoverlay' (ConvertFrom-GpuEngineInstance 'pid_42_luid_0x00000000_0x0000abcd_phys_0_eng_9_engtype_legacy overlay').EngType 'legacy overlay'
    Assert-Equal '3d_extra' (ConvertFrom-GpuEngineInstance 'pid_42_luid_0x00000000_0x0000abcd_phys_0_eng_0_engtype_3d_extra').EngType 'the rest of the name is the type'
}

Test-Case 'U4 a name that is not lower-cased reads the same' {
    $i = ConvertFrom-GpuEngineInstance 'PID_7_LUID_0X0000ABCD_0X0000EF01_PHYS_0_ENG_3_ENGTYPE_VideoCodec'
    Assert-Equal 7 $i.Pid 'pid'
    Assert-Equal '0x0000abcd0000ef01' $i.Luid 'luid, lower case'
    Assert-Equal 'videocodec' $i.EngType 'engtype'
}

Test-Case 'U5 names that are not GPU Engine instances give nothing' {
    $tail = '_luid_0x00000000_0x00000001_phys_0_eng_0_engtype_3d'
    $names = @(
        '',
        '_total',
        'gpu engine',
        ('pid_' + $tail),
        ('pid_x' + $tail),
        ('pid_-1' + $tail),
        (' pid_1' + $tail),
        ('pid_1' + $tail + [char]10),
        ('pid_' + [char]0x0661 + [char]0x0662 + $tail),
        ('pid_99999999999999999999' + $tail),
        'pid_1_luid_0x0000000_0x00000001_phys_0_eng_0_engtype_3d',
        'pid_1_luid_0x000000000_0x00000001_phys_0_eng_0_engtype_3d',
        'pid_1_luid_0x0000000g_0x00000001_phys_0_eng_0_engtype_3d',
        'pid_1_luid_00000000_00000001_phys_0_eng_0_engtype_3d',
        'pid_1_luid_0x00000000_0x00000001_phys__eng_0_engtype_3d',
        'pid_1_luid_0x00000000_0x00000001_phys_0_eng_99999999999_engtype_3d',
        'pid_1_luid_0x00000000_0x00000001_phys_0_eng_0_engtype_',
        'pid_1_luid_0x00000000_0x00000001_phys_0_eng_0_engtype_   ',
        'pid_1_luid_0x00000000_0x00000001_phys_0_eng_0'
    )
    foreach ($name in $names) {
        Assert-Equal $null (ConvertFrom-GpuEngineInstance $name) ('<' + $name + '>')
    }
    Assert-Equal $null (ConvertFrom-GpuEngineInstance $null) 'null'
}

Test-Case 'U6 a video engine is one whose type starts with video' {
    foreach ($type in @('videocodec', 'videoencode', 'videodecode', 'videoprocessing', 'video')) {
        Assert-True (Test-EncoderUsageVideoType $type) $type
    }
    foreach ($type in @('3d', 'copy', 'compute_0', 'security', 'legacyoverlay', 'vr', 'xvideo', 'vide', '')) {
        Assert-True (-not (Test-EncoderUsageVideoType $type)) ('not ' + $type)
    }
}

# --- Names ------------------------------------------------------------------------------------

Test-Case 'N1 Get-ProcessIdAndName returns Id and Name only, this process included' {
    $snapshot = Get-ProcessIdAndName
    Assert-True ($snapshot.Count -gt 0) 'a snapshot with processes'
    $self = $null
    foreach ($entry in $snapshot) {
        Assert-Equal 'Id,Name' (@($entry.PSObject.Properties | ForEach-Object { $_.Name }) -join ',') 'properties'
        Assert-True ($entry.Id -is [long]) 'Id is a long'
        Assert-True ($entry.Name -is [string]) 'Name is a string'
        if ($entry.Id -eq $PID) { $self = $entry }
    }
    Assert-True ($null -ne $self) 'this process is in it'
    Assert-True ($self.Name.Length -gt 0) 'with a name'
    Assert-True ($self.Name.IndexOfAny([char[]]@('\', '/')) -lt 0) 'a file name, never a folder'
}

Test-Case 'N2 a sampled id is named from the second snapshot, or (exited)' {
    $first = Get-EncoderUsageNames @((New-EuProcess 100 'kairo-loadgen'), (New-EuProcess 200 'other'), (New-EuProcess 500 'other'))
    $second = Get-EncoderUsageNames @((New-EuProcess 100 'kairo-loadgen'), (New-EuProcess 300 'other'), (New-EuProcess 500 'other-new'))
    Assert-Equal 'kairo-loadgen' (Get-EncoderUsageName 100 $first $second) 'in both snapshots'
    Assert-Equal '(exited)' (Get-EncoderUsageName 200 $first $second) 'gone by the second snapshot'
    Assert-Equal 'other' (Get-EncoderUsageName 300 $first $second) 'started during the window'
    Assert-Equal '(exited)' (Get-EncoderUsageName 400 $first $second) 'in neither snapshot'
    Assert-Equal '(exited)' (Get-EncoderUsageName 500 $first $second) 'an id another program took'
    $none = Get-EncoderUsageNames $null
    Assert-Equal '(exited)' (Get-EncoderUsageName 100 $none $none) 'no snapshot'
}

# --- The result, from synthetic sets (dot-sourced) --------------------------------------------

Test-Case 'M1 one JSON line in section 5.3''s form' {
    $snapshot = @((New-EuProcess 100 'kairo-loadgen'), (New-EuProcess 200 'other'))
    $r = Measure-EncoderUsage -Sets (New-EuThreeSets) -Before $snapshot -After $snapshot -Seconds 3 -Elevated $true
    Assert-Equal 'kairo_encoder_usage,seconds,samples,invalid_samples,elevated,types,video' (@($r.Usage.Keys) -join ',') 'keys'
    Assert-Equal 'name,pid,luid,engtype,eng,avg_pct,max_pct,seen' (@($r.Usage['video'][0].Keys) -join ',') 'row keys'
    # Zeros print as 0.0 on PowerShell 7 and 0 on 5.1; every other number prints the same on both.
    $expected = '^\{"kairo_encoder_usage":1,"seconds":3,"samples":3,"invalid_samples":0,"elevated":true,' +
        '"types":\{"3d":2,"copy":1,"videocodec":2\},' +
        '"video":\[\{"name":"other","pid":200,"luid":"0x00000000000123ab","engtype":"videocodec","eng":4,"avg_pct":3\.1,"max_pct":7\.5,"seen":3\},' +
        '\{"name":"other","pid":200,"luid":"0x000000010000d1a5","engtype":"videocodec","eng":4,"avg_pct":0(\.0)?,"max_pct":0(\.0)?,"seen":3\}\]\}$'
    Assert-Match $expected $r.Json 'the line'
    Assert-Equal 0 $r.Warnings.Count 'warnings'
    Assert-NoSecret $r.Json 'json'
}

Test-Case 'M2 the (exited) path: a process gone by the second snapshot, or never in one' {
    $samples = @(100, 200, 300, 400, 500 | ForEach-Object { New-EuSample (New-EuInstance $_ $EuLuidA 4 'videocodec') 1.0 })
    $before = @((New-EuProcess 100 'kairo-loadgen'), (New-EuProcess 200 'other'), (New-EuProcess 500 'other'))
    $after = @((New-EuProcess 100 'kairo-loadgen'), (New-EuProcess 300 'other'), (New-EuProcess 500 'other-new'))
    $r = Measure-EncoderUsage -Sets @(New-EuSet $samples) -Before $before -After $after -Seconds 1
    Assert-Equal '100=kairo-loadgen 200=(exited) 300=other 400=(exited) 500=(exited)' (Get-EuRowText $r.Usage) 'names'
    $parsed = ConvertFrom-Json -InputObject $r.Json
    Assert-Equal '(exited)' $parsed.video[1].name 'in the JSON'
}

Test-Case 'M3 samples with a non-zero Status are dropped and counted' {
    $a = New-EuInstance 100 $EuLuidA 4 'videocodec'
    $b = New-EuInstance 200 $EuLuidA 4 'videocodec'
    $c = New-EuInstance 200 $EuLuidA 0 '3d'
    # A program that exited mid-sample: PDH_CSTATUS_INVALID_DATA (0xC0000BBA), PDH_CSTATUS_NO_INSTANCE (0x800007D1).
    $sets = @(
        (New-EuSet @((New-EuSample $a 5.0), (New-EuSample $b 0 3221228474), (New-EuSample $c 0 2147485649))),
        (New-EuSet @((New-EuSample $a 15.0))),
        (New-EuSet @((New-EuSample $b 0 3221228474)))
    )
    $snapshot = @((New-EuProcess 100 'kairo-loadgen'))
    $r = Measure-EncoderUsage -Sets $sets -Before $snapshot -After $snapshot -Seconds 3
    Assert-Equal 3 $r.Usage['samples'] 'samples: every set returned'
    Assert-Equal 3 $r.Usage['invalid_samples'] 'invalid_samples'
    Assert-Equal 'videocodec=1' (@($r.Usage['types'].GetEnumerator() | ForEach-Object { $_.Key + '=' + $_.Value }) -join ',') 'types: dropped samples are not seen'
    Assert-Equal '100=kairo-loadgen' (Get-EuRowText $r.Usage) 'rows'
    $row = $r.Usage['video'][0]
    Assert-Equal 10 $row['avg_pct'] 'avg_pct'
    Assert-Equal 15 $row['max_pct'] 'max_pct'
    Assert-Equal 2 $row['seen'] 'seen'
    Assert-Equal 0 $r.Warnings.Count 'warnings'
}

Test-Case 'M4 no sample set at all: exit 3, with Get-Counter''s error ids' {
    $err = Assert-Throws { Measure-EncoderUsage -Sets @() -Before @() -After @() -Seconds 3 -Errors @('CounterApiError', 'CommandNotFoundException') } 'no set'
    Assert-Equal 3 (Get-EncoderUsageExitCode $err) 'exit code'
    Assert-Equal 'the GPU Engine counter set can''t be read (missing here, or under a localized name); Get-Counter: CounterApiError, CommandNotFoundException' $err.Exception.Message 'message'
    $err = Assert-Throws { Measure-EncoderUsage -Sets $null -Seconds 1 } 'null sets'
    Assert-Equal 3 (Get-EncoderUsageExitCode $err) 'exit code, no ids'
    Assert-Equal 'the GPU Engine counter set can''t be read (missing here, or under a localized name)' $err.Exception.Message 'message, no ids'
}

Test-Case 'M5 a second without a set is a warning, with the ids' {
    $x = New-EuInstance 100 $EuLuidA 4 'videocodec'
    $r = Measure-EncoderUsage -Sets @((New-EuSet @(New-EuSample $x 1.0)), (New-EuSet @(New-EuSample $x 3.0))) -Seconds 3 -Errors @('CounterApiError')
    Assert-Equal 2 $r.Usage['samples'] 'samples'
    Assert-Equal 3 $r.Usage['seconds'] 'seconds'
    Assert-Equal 1 $r.Warnings.Count 'warnings'
    Assert-Equal '1 of 3 seconds returned no sample set (Get-Counter: CounterApiError)' $r.Warnings[0] 'warning'
}

Test-Case 'M6 other instance names are skipped with a warning; _total quietly' {
    $x = New-EuInstance 100 $EuLuidA 4 'videocodec'
    $set = New-EuSet @((New-EuSample $x 2.0), (New-EuSample '_total' 9.0), (New-EuSample 'not-an-engine' 9.0), (New-EuSample 'pid_1_luid_0x1_0x2_phys_0_eng_0_engtype_3d' 9.0))
    $r = Measure-EncoderUsage -Sets @($set) -Seconds 1
    Assert-Equal 1 $r.Warnings.Count 'warnings'
    Assert-Equal '2 samples skipped: their instance name isn''t pid_<id>_luid_0x<high>_0x<low>_phys_<n>_eng_<n>_engtype_<type>' $r.Warnings[0] 'warning'
    Assert-Equal 'videocodec=1' (@($r.Usage['types'].GetEnumerator() | ForEach-Object { $_.Key + '=' + $_.Value }) -join ',') 'types'
    Assert-Equal 1 $r.Usage['video'].Count 'rows'
}

Test-Case 'M7 the census counts every type, each engine instance once, in ordinal order' {
    $sets = [Collections.Generic.List[object]]::new()
    for ($s = 0; $s -lt 3; $s++) {
        $samples = [Collections.Generic.List[object]]::new()
        $samples.Add((New-EuSample (New-EuInstance 100 $EuLuidA 0 '3d') 1.0))
        $samples.Add((New-EuSample (New-EuInstance 100 $EuLuidB 0 '3d') 1.0))
        $samples.Add((New-EuSample (New-EuInstance 200 $EuLuidA 0 '3d') 1.0))
        $samples.Add((New-EuSample (New-EuInstance 100 $EuLuidA 1 'compute_0') 1.0))
        $samples.Add((New-EuSample (New-EuInstance 100 $EuLuidA 2 'copy') 1.0))
        $samples.Add((New-EuSample (New-EuInstance 200 $EuLuidA 5 'video decode') 1.0))
        if ($s -eq 2) { $samples.Add((New-EuSample (New-EuInstance 300 $EuLuidB 7 'Security') 0.0)) }
        $sets.Add((New-EuSet $samples.ToArray()))
    }
    $r = Measure-EncoderUsage -Sets $sets.ToArray() -Seconds 3
    Assert-Equal '3d=3,compute_0=1,copy=1,security=1,videodecode=1' (@($r.Usage['types'].GetEnumerator() | ForEach-Object { $_.Key + '=' + $_.Value }) -join ',') 'census'
    Assert-Match '"types":\{"3d":3,"compute_0":1,"copy":1,"security":1,"videodecode":1\}' $r.Json 'in the JSON'
    Assert-Equal 1 $r.Usage['video'].Count 'one video row: video decode'
    Assert-Equal 'videodecode' $r.Usage['video'][0]['engtype'] 'its type'
}

Test-Case 'M8 rows sort by process id as a number, then LUID and engine' {
    $order = @(@(1000, $EuLuidA, 4), @(999, $EuLuidB, 4), @(999, $EuLuidA, 10), @(999, $EuLuidA, 4), @(8, $EuLuidB, 4))
    $samples = @($order | ForEach-Object { New-EuSample (New-EuInstance $_[0] $_[1] $_[2] 'videocodec') 1.0 })
    $r = Measure-EncoderUsage -Sets @(New-EuSet $samples) -Seconds 1
    $got = @($r.Usage['video'] | ForEach-Object { [string]$_['pid'] + '/' + $_['luid'] + '/' + $_['eng'] }) -join ' '
    Assert-Equal ('8/' + $EuLuidBText + '/4 999/' + $EuLuidAText + '/4 999/' + $EuLuidAText + '/10 999/' + $EuLuidBText + '/4 1000/' + $EuLuidAText + '/4') $got 'order'
}

Test-Case 'M9 avg_pct and max_pct: 2 decimals away from zero, over the samples that held the instance' {
    $x = New-EuInstance 100 $EuLuidA 4 'videocodec'
    $y = New-EuInstance 200 $EuLuidA 4 'videocodec'
    $z = New-EuInstance 300 $EuLuidA 4 'videocodec'
    $sets = @(
        (New-EuSet @((New-EuSample $x 0.125), (New-EuSample $y 1))),
        (New-EuSet @((New-EuSample $y 2), (New-EuSample $z 10))),
        (New-EuSet @((New-EuSample $y 4), (New-EuSample $z 20)))
    )
    $r = Measure-EncoderUsage -Sets $sets -Seconds 3
    $rows = $r.Usage['video']
    Assert-Equal 0.13 $rows[0]['avg_pct'] 'x avg: 0.125 rounds away from zero'
    Assert-Equal 0.13 $rows[0]['max_pct'] 'x max'
    Assert-Equal 1 $rows[0]['seen'] 'x seen in one set'
    Assert-Equal 2.33 $rows[1]['avg_pct'] 'y avg: 7 / 3'
    Assert-Equal 4 $rows[1]['max_pct'] 'y max'
    Assert-Equal 3 $rows[1]['seen'] 'y seen'
    Assert-Equal 15 $rows[2]['avg_pct'] 'z avg over its 2 samples, not the 3 sets'
    Assert-Equal 2 $rows[2]['seen'] 'z seen'
}

Test-Case 'M10 a value that is not a finite number is skipped; its instance still counts in the census' {
    $x = New-EuInstance 100 $EuLuidA 4 'videocodec'
    $y = New-EuInstance 200 $EuLuidA 4 'videocodec'
    $sets = @(
        (New-EuSet @((New-EuSample $x ([double]::NaN)), (New-EuSample $y ([double]::PositiveInfinity)))),
        (New-EuSet @((New-EuSample $x 5.0)))
    )
    $r = Measure-EncoderUsage -Sets $sets -Seconds 2
    Assert-Equal 'videocodec=2' (@($r.Usage['types'].GetEnumerator() | ForEach-Object { $_.Key + '=' + $_.Value }) -join ',') 'census'
    Assert-Equal 1 $r.Usage['video'].Count 'rows'
    Assert-Equal 5 $r.Usage['video'][0]['avg_pct'] 'avg'
    Assert-Equal 1 $r.Usage['video'][0]['seen'] 'seen'
    Assert-True (-not $r.Json.Contains('NaN')) 'no NaN in the JSON'
}

Test-Case 'M11 elevated is true, false or null' {
    foreach ($case in @(@($true, 'true'), @($false, 'false'), @($null, 'null'), @('yes', 'null'))) {
        $r = Measure-EncoderUsage -Sets @(New-EuSet) -Seconds 1 -Elevated $case[0]
        Assert-Match ('"elevated":' + $case[1] + ',') $r.Json ('elevated ' + $case[1])
    }
}

Test-Case 'M12 the table: a summary, the census, and the rows used' {
    $snapshot = @((New-EuProcess 100 'kairo-loadgen'), (New-EuProcess 200 'other'))
    $r = Measure-EncoderUsage -Sets (New-EuThreeSets) -Before $snapshot -After $snapshot -Seconds 3 -Elevated $true
    $lines = $r.Table
    Assert-Equal 4 $lines.Count 'lines: the idle row is left out'
    Assert-Equal 'encoder use: 3 s, 3 samples (0 invalid), elevated: yes' $lines[0] 'summary'
    Assert-Equal 'engine types seen: 3d 2, copy 1, videocodec 2' $lines[1] 'census'
    Assert-Match '^name +pid luid +engtype +eng +avg % +max % +seen$' $lines[2] 'header'
    Assert-Match '^other +200 0x00000000000123ab videocodec +4 +3\.10 +7\.50 +3$' $lines[3] 'row'
    $idle = Measure-EncoderUsage -Sets @(New-EuSet) -Seconds 1 -Elevated $false
    Assert-Equal 'encoder use: 1 s, 1 samples (0 invalid), elevated: no' $idle.Table[0] 'summary, not elevated'
    Assert-Equal 'engine types seen: none' $idle.Table[1] 'no census'
    Assert-Equal 'no video-engine use in these seconds' $idle.Table[2] 'no row'
    $unknown = Measure-EncoderUsage -Sets @(New-EuSet) -Seconds 1
    Assert-Match 'elevated: unknown$' $unknown.Table[0] 'summary, unknown'
    Assert-NoSecret ($r.Table -join "`n") 'table'
}

Test-Case 'M13 Get-EncoderUsage: a snapshot, one Get-Counter call a second with section 5.3''s arguments, a snapshot' {
    $log = [Collections.Generic.List[string]]::new()
    $state = @{ Calls = 0 }
    function Get-Counter {
        [CmdletBinding()]
        param([string[]] $Counter, [long] $MaxSamples)
        $state.Calls++
        $log.Add('counter ' + ($Counter -join ';') + ' max ' + $MaxSamples + ' ' + [string]$PSBoundParameters['ErrorAction'] + ' ' + [string]$PSBoundParameters['ErrorVariable'])
        if ($state.Calls -eq 2) {
            # The non-terminating error Get-Counter writes beside a set with an invalid sample.
            Write-Error -Message 'The data is not valid on \\SECRET-PC' -ErrorId 'CounterApiError' -Category InvalidResult
        }
        if ($state.Calls -eq 3) {
            # A terminating one: this second has no set.
            $PSCmdlet.ThrowTerminatingError([Management.Automation.ErrorRecord]::new([Exception]::new('Unable to connect to \\SECRET-PC'), 'CounterApiError', [Management.Automation.ErrorCategory]::InvalidResult, $null))
        }
        New-EuSet @(New-EuSample (New-EuInstance 100 $EuLuidA 4 'videocodec') (10.0 * $state.Calls))
    }
    function Get-ProcessIdAndName {
        $log.Add('snapshot')
        return ,@(New-EuProcess 100 'kairo-loadgen')
    }
    $r = Get-EncoderUsage -Seconds 4
    $call = 'counter \GPU Engine(*)\Utilization Percentage max 1 SilentlyContinue pdhErrors'
    Assert-Equal ('snapshot|' + $call + '|' + $call + '|' + $call + '|' + $call + '|snapshot') ($log -join '|') 'calls'
    Assert-Equal 4 $r.Usage['seconds'] 'seconds'
    Assert-Equal 3 $r.Usage['samples'] 'samples'
    $row = $r.Usage['video'][0]
    Assert-Equal 'kairo-loadgen' $row['name'] 'name'
    Assert-Equal 23.33 $row['avg_pct'] 'avg_pct: (10 + 20 + 40) / 3'
    Assert-Equal 40 $row['max_pct'] 'max_pct'
    Assert-Equal 3 $row['seen'] 'seen'
    Assert-Equal '1 of 4 seconds returned no sample set (Get-Counter: CounterApiError)' ($r.Warnings -join ' / ') 'warnings'
    Assert-NoSecret ($r.Json + ($r.Table -join "`n") + ($r.Warnings -join "`n")) 'output'
}

Test-Case 'M14 Get-EncoderUsage: no set in any second is exit 3, with the ids only' {
    function Get-Counter {
        [CmdletBinding()]
        param([string[]] $Counter, [long] $MaxSamples)
        Write-Error -Message 'Unable to connect to the specified computer or the computer is offline: \\SECRET-PC' -ErrorId 'CounterApiError' -Category InvalidResult
    }
    function Get-ProcessIdAndName { return ,@() }
    $err = Assert-Throws { Get-EncoderUsage -Seconds 2 } 'no set'
    Assert-Equal 3 (Get-EncoderUsageExitCode $err) 'exit code'
    Assert-Equal 'the GPU Engine counter set can''t be read (missing here, or under a localized name); Get-Counter: CounterApiError' $err.Exception.Message 'message'
}

Test-Case 'M15 a bad -Seconds is exit 2, before any snapshot or sample' {
    $log = [Collections.Generic.List[string]]::new()
    function Get-Counter {
        [CmdletBinding()]
        param([string[]] $Counter, [long] $MaxSamples)
        $log.Add('counter')
    }
    function Get-ProcessIdAndName { $log.Add('snapshot'); return ,@() }
    foreach ($bad in @(0, 601, '0', '601', '1000', 'abc', '1.5', 1.5, '-1', '+5', '', ' ', $true, $false, $null, @(1, 2))) {
        $err = Assert-Throws { Get-EncoderUsage -Seconds $bad } ('-Seconds <' + [string]$bad + '>')
        Assert-Equal 2 (Get-EncoderUsageExitCode $err) ('exit code for <' + [string]$bad + '>')
        Assert-Equal '-Seconds must be a whole number from 1 to 600' $err.Exception.Message 'message'
    }
    Assert-Equal 0 $log.Count 'snapshots and samples taken'
    foreach ($good in @(@('5', 5), @(5, 5), @(' 7 ', 7), @('600', 600), @(1, 1), @([long]3, 3), @(5.0, 5), @('007', 7))) {
        Assert-Equal $good[1] (ConvertTo-EncoderUsageSeconds $good[0]) ('-Seconds <' + [string]$good[0] + '>')
    }
}

Test-Case 'M16 the command line''s parser, as -File and & pass arguments' {
    $p = ConvertFrom-EncoderUsageArguments @('-Seconds', '5', '-Json')
    Assert-Equal '5' $p['Seconds'] '-Seconds'
    Assert-Equal $true $p['Json'] '-Json'
    $p = ConvertFrom-EncoderUsageArguments @('-seconds', '5', '-JSON', $false)
    Assert-Equal $false $p['Json'] '-Json:$false as -File passes it, names in any case'
    $p = ConvertFrom-EncoderUsageArguments @('-Seconds:', 7, '-Json:', $true)
    Assert-Equal 7 $p['Seconds'] '-Seconds:7 as & passes it'
    Assert-Equal $true $p['Json'] '-Json:$true as & passes it'
    Assert-Equal 0 (ConvertFrom-EncoderUsageArguments @()).Count 'no arguments'
    $bad = @(
        @('5'),
        @('-Foo', '1'),
        @('-Sec', '1'),
        @('--Seconds', '1'),
        @('-Seconds'),
        @('-Seconds', '1', '-Seconds', '2'),
        @('-Json', '-Json'),
        @('-Json', 'yes'),
        @('-Seconds', '1', 5)
    )
    foreach ($arguments in $bad) {
        $err = Assert-Throws { ConvertFrom-EncoderUsageArguments $arguments } ($arguments -join ' ')
        Assert-Equal 2 (Get-EncoderUsageExitCode $err) ('exit code for ' + ($arguments -join ' '))
    }
}

Test-Case 'M17 an error is kept as its id, never its message' {
    $record = [Management.Automation.ErrorRecord]::new([Exception]::new('Unable to connect to \\SECRET-PC'), 'CounterApiError,Microsoft.PowerShell.Commands.GetCounterCommand', [Management.Automation.ErrorCategory]::InvalidResult, $null)
    Assert-Equal 'CounterApiError' (Get-EncoderUsageErrorId $record) 'an error record'
    $carrier = [Management.Automation.RuntimeException]::new('Unable to connect to \\SECRET-PC', $null, $record)
    Assert-Equal 'CounterApiError' (Get-EncoderUsageErrorId $carrier) 'an exception that carries one'
    $odd = [Management.Automation.ErrorRecord]::new([Exception]::new('x'), 'not an id \\SECRET-PC', [Management.Automation.ErrorCategory]::InvalidResult, $null)
    Assert-Equal 'unknown' (Get-EncoderUsageErrorId $odd) 'an id that is not one word'
    Assert-Equal 'InvalidOperationException' (Get-EncoderUsageErrorId ([InvalidOperationException]::new('\\SECRET-PC'))) 'a bare exception: its type'
    Assert-Equal 'unknown' (Get-EncoderUsageErrorId $null) 'nothing'
}

Test-Case 'M18 elevated reads this session''s own token: a bool on Windows, null elsewhere' {
    $elevated = Test-EncoderUsageElevated
    if ([Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT) {
        Assert-True ($elevated -is [bool]) 'a bool on Windows'
    } else {
        Assert-Equal $null $elevated 'null off Windows'
    }
}

# --- Dot-sourcing ---------------------------------------------------------------------------

Test-Case 'G1 dot-sourcing the file defines functions and nothing else' {
    $result = & {
        $names0 = @((Get-Variable -Scope 0).Name)
        $output = @(. $EuScript)
        $names1 = @((Get-Variable -Scope 0).Name)
        $new = @($names1 | Where-Object { $names0 -notcontains $_ -and $_ -cne 'names0' -and $_ -cne 'output' })
        [pscustomobject]@{ Output = $output.Count; New = ($new -join ','); Defined = [bool](Get-Command 'Get-EncoderUsage' -CommandType Function -ErrorAction SilentlyContinue) }
    }
    Assert-Equal 0 $result.Output 'output'
    Assert-Equal '' $result.New 'new variables'
    Assert-True $result.Defined 'Get-EncoderUsage is defined'
}

Test-Case 'G2 dot-sourcing its text, as the elevated session does, runs no command line' {
    $text = [IO.File]::ReadAllText($EuScript)
    $result = & {
        $output = @(. ([scriptblock]::Create($text)))
        [pscustomobject]@{ Output = $output.Count; Defined = [bool](Get-Command 'Measure-EncoderUsage' -CommandType Function -ErrorAction SilentlyContinue) }
    }
    Assert-Equal 0 $result.Output 'output'
    Assert-True $result.Defined 'Measure-EncoderUsage is defined'
}

# --- The house rules, over the script's syntax tree -----------------------------------------

Test-Case 'S1 ASCII only, and it parses' {
    foreach ($file in @($EuScript, $PSCommandPath)) {
        $high = 0
        foreach ($b in [IO.File]::ReadAllBytes($file)) { if ($b -gt 127) { $high++ } }
        Assert-Equal 0 $high ('bytes above 127 in ' + [IO.Path]::GetFileName($file))
    }
    Assert-Equal 0 @($EuParseErrors).Count 'parse errors'
}

Test-Case 'S2 no member that lists modules or reads a command line (CLAUDE.md Never 3), except on a type' {
    $banned = @('Path', 'MainModule', 'Modules', 'Company', 'FileVersion', 'ProductVersion', 'Description', 'Product', 'CommandLine')
    $hits = [Collections.Generic.List[string]]::new()
    foreach ($node in $EuAst.FindAll({ param($n) $n -is [Management.Automation.Language.MemberExpressionAst] }, $true)) {
        if ($node.Expression -is [Management.Automation.Language.TypeExpressionAst]) { continue }
        if (-not ($node.Member -is [Management.Automation.Language.StringConstantExpressionAst])) {
            $hits.Add('a computed member: ' + $node.Extent.Text)
        } elseif ($banned -contains $node.Member.Value) {
            $hits.Add($node.Extent.Text)
        }
    }
    Assert-Equal '' ($hits -join '; ') 'members'
}

Test-Case 'S3 GetProcesses only in Get-ProcessIdAndName, which reads Id and ProcessName only' {
    $calls = @($EuAst.FindAll({ param($n) $n -is [Management.Automation.Language.InvokeMemberExpressionAst] -and $n.Member -is [Management.Automation.Language.StringConstantExpressionAst] -and $n.Member.Value -eq 'GetProcesses' }, $true))
    Assert-Equal 1 $calls.Count 'GetProcesses calls'
    $helper = $calls[0].Parent
    while ($null -ne $helper -and -not ($helper -is [Management.Automation.Language.FunctionDefinitionAst])) { $helper = $helper.Parent }
    Assert-True ($null -ne $helper) 'inside a function'
    Assert-Equal 'Get-ProcessIdAndName' $helper.Name 'the helper'
    $members = @($helper.FindAll({ param($n) $n -is [Management.Automation.Language.MemberExpressionAst] -and $n.Expression -is [Management.Automation.Language.VariableExpressionAst] -and $n.Expression.VariablePath.UserPath -eq 'process' }, $true) | ForEach-Object { $_.Member.Value } | Sort-Object -Unique)
    Assert-Equal 'Dispose,Id,ProcessName' ($members -join ',') 'what it reads of a process'
    $processes = @($EuAst.FindAll({ param($n) $n -is [Management.Automation.Language.TypeExpressionAst] -and $n.TypeName.Name -match 'Process$' }, $true))
    Assert-Equal 1 $processes.Count 'one use of the Process type'
}

Test-Case 'S4 it calls only its own functions, Get-Counter, Where-Object, ConvertTo-Json and Set-StrictMode' {
    $own = @($EuAst.FindAll({ param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] }, $true) | ForEach-Object { $_.Name })
    $allowed = $own + @('Get-Counter', 'Where-Object', 'ConvertTo-Json', 'Set-StrictMode')
    $other = [Collections.Generic.List[string]]::new()
    foreach ($command in $EuAst.FindAll({ param($n) $n -is [Management.Automation.Language.CommandAst] }, $true)) {
        $name = $command.GetCommandName()
        if ($null -eq $name -or $allowed -notcontains $name) { $other.Add($command.Extent.Text) }
    }
    Assert-Equal '' ($other -join '; ') 'other commands (Get-Process, Add-Type, Import-Csv, writes and the like)'
    foreach ($json in $EuAst.FindAll({ param($n) $n -is [Management.Automation.Language.CommandAst] -and $n.GetCommandName() -eq 'ConvertTo-Json' }, $true)) {
        Assert-Equal 'ConvertTo-Json -InputObject $usage -Compress -Depth 5' $json.Extent.Text 'ConvertTo-Json serializes the result only'
    }
    # Add-Type, Import-Csv and Invoke-Expression are commands, so the list above refuses them; the
    # header names them only in comments. SendInput would be a Windows call, so it is looked for as text.
    Assert-True ([IO.File]::ReadAllText($EuScript).IndexOf('SendInput', [StringComparison]::OrdinalIgnoreCase) -lt 0) 'no SendInput'
}

Test-Case 'S5 functions only return or throw; the one exit is in the guarded command line' {
    Assert-Equal $null $EuAst.ParamBlock 'a script-level param block'
    Assert-Equal $null $EuAst.BeginBlock 'a begin block'
    Assert-Equal $null $EuAst.ProcessBlock 'a process block'
    $guards = [Collections.Generic.List[object]]::new()
    foreach ($statement in $EuAst.EndBlock.Statements) {
        if ($statement -is [Management.Automation.Language.FunctionDefinitionAst]) { continue }
        Assert-True ($statement -is [Management.Automation.Language.IfStatementAst]) ('a top-level statement: ' + $statement.Extent.Text)
        $guards.Add($statement)
    }
    Assert-Equal 1 $guards.Count 'guarded blocks'
    Assert-Equal '$MyInvocation.InvocationName -ne ''.''' $guards[0].Clauses[0].Item1.Extent.Text 'the guard'
    $exits = @($EuAst.FindAll({ param($n) $n -is [Management.Automation.Language.ExitStatementAst] }, $true))
    Assert-Equal 1 $exits.Count 'exit statements'
    Assert-True ($guards[0].Extent.StartOffset -le $exits[0].Extent.StartOffset -and $exits[0].Extent.EndOffset -le $guards[0].Extent.EndOffset) 'the exit is inside the guard'
    $mandatory = @($EuAst.FindAll({ param($n) $n -is [Management.Automation.Language.NamedAttributeArgumentAst] -and $n.ArgumentName -eq 'Mandatory' }, $true))
    Assert-Equal 0 $mandatory.Count 'Mandatory parameters'
}

Test-Case 'S6 Get-Counter: one call, with section 5.3''s arguments' {
    $calls = @($EuAst.FindAll({ param($n) $n -is [Management.Automation.Language.CommandAst] -and $n.GetCommandName() -eq 'Get-Counter' }, $true))
    Assert-Equal 1 $calls.Count 'Get-Counter calls'
    Assert-Equal 'Get-Counter -Counter ''\GPU Engine(*)\Utilization Percentage'' -MaxSamples 1 -ErrorAction SilentlyContinue -ErrorVariable pdhErrors' $calls[0].Extent.Text 'the call'
}

# --- The command line (-File), with its exit codes -----------------------------------------

# What each wrapper starts with: a counter of Get-Counter calls, and a sample maker like the one above.
$EuPrelude = @'
$global:EuCalls = 0
function New-EuSample([string] $Instance, [double] $Value, [long] $Status) {
    [pscustomobject]@{ Path = '\\secret-pc\gpu engine(' + $Instance + ')\utilization percentage'; InstanceName = $Instance; CookedValue = $Value; Status = [uint32]$Status }
}
'@

# What Get-Counter does where the counter set doesn't exist: a non-terminating CounterApiError, whose
# message can name the computer, and no set.
$EuCounterMissing = @'
function Get-Counter {
    [CmdletBinding()]
    param([string[]] $Counter, [long] $MaxSamples)
    $global:EuCalls++
    Write-Error -Message 'Unable to connect to the specified computer or the computer is offline: \\SECRET-PC' -ErrorId 'CounterApiError' -Category InvalidResult
}
'@

# A terminating error in every second.
$EuCounterThrows = @'
function Get-Counter {
    [CmdletBinding()]
    param([string[]] $Counter, [long] $MaxSamples)
    $record = [Management.Automation.ErrorRecord]::new([Exception]::new('Unable to connect to \\SECRET-PC'), 'CounterApiError', [Management.Automation.ErrorCategory]::InvalidResult, $null)
    $PSCmdlet.ThrowTerminatingError($record)
}
'@

# Two sets: this process on a video engine (12.5 then 7.5) and the 3D engine; process 4000000001,
# which no snapshot holds, on a video engine (3 then 5) and idle on another adapter's encoder; in the
# first set also a sample of a program that exited mid-sample, and Get-Counter's error for it.
$EuCounterTwoSets = @'
function Get-Counter {
    [CmdletBinding()]
    param([string[]] $Counter, [long] $MaxSamples)
    $global:EuCalls++
    $engine = '_luid_0x00000000_0x0000d1a5_phys_0_eng_'
    $self = 'pid_' + $PID + $engine
    $gone = 'pid_4000000001' + $engine
    $v = @(7.5, 5.0, 60.0)
    if ($global:EuCalls -eq 1) { $v = @(12.5, 3.0, 40.0) }
    $samples = [Collections.Generic.List[object]]::new()
    $samples.Add((New-EuSample ($self + '4_engtype_videocodec') $v[0] 0))
    $samples.Add((New-EuSample ($gone + '4_engtype_videocodec') $v[1] 0))
    $samples.Add((New-EuSample ($self + '0_engtype_3d') $v[2] 0))
    $samples.Add((New-EuSample 'pid_4000000001_luid_0x00000001_0x00000002_phys_0_eng_5_engtype_videoencode' 0 0))
    if ($global:EuCalls -eq 1) {
        $samples.Add((New-EuSample 'pid_4000000003_luid_0x00000000_0x0000d1a5_phys_0_eng_4_engtype_videocodec' 0 3221228474))
        Write-Error -Message 'The data in one of the performance counter samples is not valid on \\SECRET-PC' -ErrorId 'CounterApiError' -Category InvalidResult
    }
    [pscustomobject]@{ Timestamp = [datetime]::Now; CounterSamples = $samples.ToArray() }
}
'@

# A sample without a Status: an internal error under Set-StrictMode.
$EuCounterBroken = @'
function Get-Counter {
    [CmdletBinding()]
    param([string[]] $Counter, [long] $MaxSamples)
    [pscustomobject]@{ CounterSamples = @([pscustomobject]@{ InstanceName = 'pid_1_luid_0x00000000_0x00000001_phys_0_eng_4_engtype_videocodec'; CookedValue = 1.0 }) }
}
'@

function New-EuWrapper([string] $StandIn, [string] $Arguments) {
    # A script that defines a stand-in Get-Counter, runs encoder-usage.ps1 with & and these
    # arguments, and exits with its exit code. & runs the command line as -File does
    # ($MyInvocation.InvocationName is '&', not '.'), and the stand-in, a function, comes before the
    # cmdlet; the snapshots stay real.
    $dir = New-BenchTempDir
    $target = $EuScript.Replace("'", "''")
    $text = $EuPrelude + "`r`n" + $StandIn + "`r`n" + "& '" + $target + "' " + $Arguments + "`r`n" + 'exit $LASTEXITCODE' + "`r`n"
    $file = Join-Path $dir 'wrapper.ps1'
    [IO.File]::WriteAllText($file, $text, [Text.UTF8Encoding]::new($true))
    return $file
}

Test-Case 'C1 no arguments: exit 2' {
    $run = Invoke-BenchTool 'encoder-usage.ps1' @()
    Assert-Equal 2 $run.ExitCode 'exit code'
    Assert-Equal '' $run.StdOut 'stdout'
    Assert-Equal 'encoder-usage: missing -Seconds' $run.StdErr.TrimEnd() 'stderr'
}

Test-Case 'C2 a bad -Seconds: exit 2' {
    foreach ($bad in @('0', '601', 'abc', '1.5')) {
        $run = Invoke-BenchTool 'encoder-usage.ps1' @('-Seconds', $bad, '-Json')
        Assert-Equal 2 $run.ExitCode ('exit code for ' + $bad)
        Assert-Equal '' $run.StdOut ('stdout for ' + $bad)
        Assert-Equal 'encoder-usage: -Seconds must be a whole number from 1 to 600' $run.StdErr.TrimEnd() ('stderr for ' + $bad)
    }
}

Test-Case 'C3 other bad command lines: exit 2' {
    $cases = @(
        @(@('-Seconds', '1', '-Foo'), 'encoder-usage: unknown parameter -Foo'),
        @(@('-Seconds', '1', '-Seconds', '2'), 'encoder-usage: -Seconds given twice'),
        @(@('5'), 'encoder-usage: unexpected argument at position 1'),
        @(@('-Seconds'), 'encoder-usage: -Seconds needs a value')
    )
    foreach ($case in $cases) {
        $run = Invoke-BenchTool 'encoder-usage.ps1' $case[0]
        Assert-Equal 2 $run.ExitCode ('exit code for ' + ($case[0] -join ' '))
        Assert-Equal '' $run.StdOut 'stdout'
        Assert-Equal $case[1] $run.StdErr.TrimEnd() 'stderr'
    }
}

Test-Case 'C4 the real counter set for a second: one JSON line, or exit 3 where it is missing' {
    $run = Invoke-BenchTool 'encoder-usage.ps1' @('-Seconds', '1', '-Json')
    [Console]::Out.WriteLine('     (the real counter set: exit ' + $run.ExitCode + ')')
    if ($run.ExitCode -eq 3) {
        Assert-Equal '' $run.StdOut 'stdout'
        Assert-Match '^encoder-usage: the GPU Engine counter set can''t be read \(missing here, or under a localized name\)(; Get-Counter: [A-Za-z0-9_.]+(, [A-Za-z0-9_.]+)*)?$' $run.StdErr.TrimEnd() 'stderr'
    } else {
        Assert-Equal 0 $run.ExitCode 'exit code'
        $usage = ConvertFrom-EuLine $run.StdOut
        Assert-Equal 1 $usage.kairo_encoder_usage 'kairo_encoder_usage'
        Assert-Equal 1 $usage.seconds 'seconds'
        Assert-Equal 1 $usage.samples 'samples'
        foreach ($row in @($usage.video)) {
            Assert-Match '^0x[0-9a-f]{16}$' $row.luid 'luid'
            Assert-True ($row.name.IndexOfAny([char[]]@('\', '/')) -lt 0) 'a file name'
        }
    }
}

Test-Case 'C5 exit 3 where GPU Engine is missing, with the error id and never its message' {
    $run = Invoke-BenchTool (New-EuWrapper $EuCounterMissing '-Seconds 2 -Json') @()
    Assert-Equal 3 $run.ExitCode 'exit code'
    Assert-Equal '' $run.StdOut 'stdout'
    Assert-Equal 'encoder-usage: the GPU Engine counter set can''t be read (missing here, or under a localized name); Get-Counter: CounterApiError' $run.StdErr.TrimEnd() 'stderr'
    Assert-NoSecret ($run.StdOut + $run.StdErr) 'output'
}

Test-Case 'C6 the command line over two sets: this process by name, an unknown id (exited), an invalid sample counted' {
    $run = Invoke-BenchTool (New-EuWrapper $EuCounterTwoSets '-Seconds 2 -Json') @()
    Assert-Equal 0 $run.ExitCode ('exit code; stderr: ' + $run.StdErr)
    Assert-Equal '' $run.StdErr 'stderr'
    Assert-NoSecret $run.StdOut 'stdout'
    $usage = ConvertFrom-EuLine $run.StdOut
    Assert-Equal 'kairo_encoder_usage,seconds,samples,invalid_samples,elevated,types,video' (@($usage.PSObject.Properties | ForEach-Object { $_.Name }) -join ',') 'keys'
    Assert-Equal 1 $usage.kairo_encoder_usage 'kairo_encoder_usage'
    Assert-Equal 2 $usage.seconds 'seconds'
    Assert-Equal 2 $usage.samples 'samples'
    Assert-Equal 1 $usage.invalid_samples 'invalid_samples'
    if ([Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT) {
        Assert-True ($usage.elevated -is [bool]) 'elevated, on Windows'
    } else {
        Assert-Equal $null $usage.elevated 'elevated, off Windows'
    }
    Assert-Equal '3d=1,videocodec=2,videoencode=1' (@($usage.types.PSObject.Properties | ForEach-Object { $_.Name + '=' + $_.Value }) -join ',') 'types'
    $rows = @($usage.video)
    Assert-Equal 3 $rows.Count 'rows'
    Assert-Equal $EuSelfName $rows[0].name 'this process, from the snapshots'
    Assert-True ($rows[0].pid -gt 0 -and $rows[0].pid -ne 4000000001) 'its id'
    Assert-Equal '0x000000000000d1a5' $rows[0].luid 'its luid'
    Assert-Equal 'videocodec' $rows[0].engtype 'its type'
    Assert-Equal 4 $rows[0].eng 'its engine'
    Assert-Equal 10 $rows[0].avg_pct 'its avg_pct'
    Assert-Equal 12.5 $rows[0].max_pct 'its max_pct'
    Assert-Equal 2 $rows[0].seen 'its seen'
    Assert-Equal '(exited)' $rows[1].name 'an id no snapshot holds'
    Assert-Equal 4000000001 $rows[1].pid 'its id'
    Assert-Equal 4 $rows[1].avg_pct 'its avg_pct'
    Assert-Equal 5 $rows[1].max_pct 'its max_pct'
    Assert-Equal '(exited)' $rows[2].name 'the idle encoder'
    Assert-Equal '0x0000000100000002' $rows[2].luid 'on the other adapter'
    Assert-Equal 'videoencode' $rows[2].engtype 'its type'
    Assert-Equal 0 $rows[2].max_pct 'idle'
}

Test-Case 'C7 without -Json, or with -Json:$false, a table of the rows used' {
    foreach ($arguments in @('-Seconds 2', '-Seconds 2 -Json:$false')) {
        $run = Invoke-BenchTool (New-EuWrapper $EuCounterTwoSets $arguments) @()
        Assert-Equal 0 $run.ExitCode ('exit code for ' + $arguments)
        Assert-Equal '' $run.StdErr 'stderr'
        $lines = Get-EuLines $run.StdOut
        Assert-Equal 5 $lines.Count ('lines for ' + $arguments + ': the idle row is left out')
        Assert-Match '^encoder use: 2 s, 2 samples \(1 invalid\), elevated: (yes|no|unknown)$' $lines[0] 'summary'
        Assert-Equal 'engine types seen: 3d 1, videocodec 2, videoencode 1' $lines[1] 'census'
        Assert-Match '^name +pid luid +engtype +eng +avg % +max % +seen$' $lines[2] 'header'
        Assert-Match ('^' + [regex]::Escape($EuSelfName) + ' +[0-9]+ 0x000000000000d1a5 videocodec +4 +10\.00 +12\.50 +2$') $lines[3] 'this process'
        Assert-Match '^\(exited\) +4000000001 0x000000000000d1a5 videocodec +4 +4\.00 +5\.00 +2$' $lines[4] 'the unknown id'
        Assert-NoSecret $run.StdOut 'stdout'
    }
}

Test-Case 'C8 Get-Counter failing in every second: exit 3, with its id only' {
    $run = Invoke-BenchTool (New-EuWrapper $EuCounterThrows '-Seconds 2 -Json') @()
    Assert-Equal 3 $run.ExitCode 'exit code'
    Assert-Equal '' $run.StdOut 'stdout'
    Assert-Equal 'encoder-usage: the GPU Engine counter set can''t be read (missing here, or under a localized name); Get-Counter: CounterApiError' $run.StdErr.TrimEnd() 'stderr'
    Assert-NoSecret $run.StdErr 'stderr'
}

Test-Case 'C9 an unexpected error prints its type and line, never its message' {
    $run = Invoke-BenchTool (New-EuWrapper $EuCounterBroken '-Seconds 1 -Json') @()
    Assert-Equal 3 $run.ExitCode 'exit code'
    Assert-Equal '' $run.StdOut 'stdout'
    Assert-Match '^encoder-usage: internal error: [A-Za-z]+ at line [0-9]+$' $run.StdErr.TrimEnd() 'stderr'
}

Complete-BenchTests
