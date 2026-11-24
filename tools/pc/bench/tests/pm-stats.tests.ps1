# SPDX-License-Identifier: GPL-3.0-or-later
#
# tools/pc/bench/tests/pm-stats.tests.ps1 - pm-stats.ps1's vectors P1-P13 and E1-E6, and the edges
# of section 5.1's rules that no vector sits on.
# Project: https://github.com/Maty3k/Kairo
# Design: docs/research/17-loadgen-bench/README.md sections 5.1 and 6.
#
#   pwsh -NoProfile -File tools/pc/bench/tests/pm-stats.tests.ps1
#
# It prints one line per case and exits 0 when every case passed, 1 when one failed.
# Every vector runs both ways: through Get-PmStats after dot-sourcing pm-stats.ps1 under
# Set-StrictMode -Version 3.0 and $ErrorActionPreference = 'Stop' (testlib.ps1 sets both), over the
# file's bytes decoded by their byte-order mark as the elevated session passes them; and through
# pwsh -File, which must exit with the vector's code and print the same line and warnings. Numbers
# compare as numbers, since ConvertTo-Json may print 40.0 or 40.
# The CSVs are written here, into a temporary folder, from section 5.1's 28-column header with
# section 6's defaults; synthetic names only (kairo-loadgen.exe, other.exe).
#
# Rules: ASCII only; Windows PowerShell 5.1 syntax (CI parses tools/** there); no Add-Type.

. (Join-Path $PSScriptRoot 'testlib.ps1')
. (Join-Path $BenchDir 'pm-stats.ps1')

$script:PmDir = New-BenchTempDir

# PresentMon 2.6.0's header with the bench's flags (research 17 section 5.1).
$script:PmColumns = @(
    'Application', 'ProcessID', 'SwapChainAddress', 'PresentRuntime', 'SyncInterval', 'PresentFlags',
    'AllowsTearing', 'PresentMode', 'FrameType', 'TimeInQPC', 'MsBetweenSimulationStart', 'MsBetweenPresents',
    'MsBetweenDisplayChange', 'MsInPresentAPI', 'MsRenderPresentLatency', 'MsUntilDisplayed', 'CPUStartQPC',
    'MsBetweenAppStart', 'MsCPUBusy', 'MsCPUWait', 'MsGPULatency', 'MsGPUTime', 'MsGPUBusy', 'MsGPUWait',
    'MsVideoBusy', 'MsAnimationError', 'AnimationTime', 'MsFlipDelay'
)

# Section 6's P1 line, verbatim.
$script:P1Line = '{"pm_stats":1,"tag":"aa-alu-1","frames":10,"avg_fps":113.64,"low1_fps":62.5,"low01_fps":62.5,"gpu_busy_ms":7.45,"gpu_wait_ms":0.45,"present_modes":{"Hardware: Independent Flip":10},"frame_types":{"Application":10},"low_def":"p99_nearest_rank","gpu_time_ms":7.9,"gpu_busy_p99_ms":7.9,"gpu_wait_p99_ms":0.9,"gpu_time_p99_ms":8.8,"display_change_ms":8.8,"display_change_p99_ms":16.0,"max_frame_ms":16.0,"long_frames":0,"sync_interval":{"0":10},"allows_tearing":{"1":10},"present_flags":{"512":10},"duration_s":0.088,"dropped_pct":0.0,"swapchains":1,"rows":10,"bad_rows":0}'

# --- Fixtures -------------------------------------------------------------------------------

function New-PmRow {
    # One row as column -> text, with section 6's defaults. Present $I's TimeInQPC is
    # 1,000,000 + 80,000 * $I; MsBetweenPresents is $Ms with 14 decimals and MsBetweenDisplayChange
    # the same; -Dropped makes both MsBetweenDisplayChange and MsUntilDisplayed NA; GPU busy and
    # wait have 4 decimals and MsGPUTime is their sum. -Set overrides columns by name.
    param([long] $I, [double] $Ms, [double] $Busy = 7.9, [double] $Wait = 0.1, [switch] $Dropped, [hashtable] $Set = @{})
    $inv = [Globalization.CultureInfo]::InvariantCulture
    $msText = $Ms.ToString('F14', $inv)
    $display = $msText
    $until = '2.5000'
    if ($Dropped) {
        $display = 'NA'
        $until = 'NA'
    }
    $row = [ordered]@{
        Application              = 'kairo-loadgen.exe'
        ProcessID                = '4242'
        SwapChainAddress         = '0x1A2B3C4D5E0'
        PresentRuntime           = 'DXGI'
        SyncInterval             = '0'
        PresentFlags             = '512'
        AllowsTearing            = '1'
        PresentMode              = 'Hardware: Independent Flip'
        FrameType                = 'Application'
        TimeInQPC                = (1000000 + 80000 * $I).ToString($inv)
        MsBetweenSimulationStart = 'NA'
        MsBetweenPresents        = $msText
        MsBetweenDisplayChange   = $display
        MsInPresentAPI           = '0'
        MsRenderPresentLatency   = '0'
        MsUntilDisplayed         = $until
        CPUStartQPC              = '0'
        MsBetweenAppStart        = '0'
        MsCPUBusy                = '0'
        MsCPUWait                = '0'
        MsGPULatency             = '0'
        MsGPUTime                = ($Busy + $Wait).ToString('F4', $inv)
        MsGPUBusy                = $Busy.ToString('F4', $inv)
        MsGPUWait                = $Wait.ToString('F4', $inv)
        MsVideoBusy              = '0'
        MsAnimationError         = 'NA'
        AnimationTime            = 'NA'
        MsFlipDelay              = 'NA'
    }
    foreach ($key in $Set.Keys) {
        if (-not $row.Contains($key)) { throw ('the fixture has no column ' + $key) }
        $row[$key] = [string]$Set[$key]
    }
    return $row
}

function ConvertTo-PmLine($Row, [string[]] $Columns = $script:PmColumns) {
    $fields = [Collections.Generic.List[string]]::new()
    foreach ($name in $Columns) { $fields.Add([string]$Row[$name]) }
    return [string]::Join(',', $fields)
}

function ConvertTo-PmLines($Rows, [string[]] $Columns = $script:PmColumns) {
    # The header, then one line per row.
    $lines = [Collections.Generic.List[string]]::new()
    $lines.Add([string]::Join(',', $Columns))
    foreach ($row in $Rows) { $lines.Add((ConvertTo-PmLine $row $Columns)) }
    return ,$lines.ToArray()
}

function Write-PmCsv {
    # Writes the lines as PresentMon does, UTF-8 with a byte-order mark and CRLF after every line,
    # unless told otherwise; -Tail is a last line with no line end. Returns the file's path.
    param([string] $Name, [string[]] $Lines, [string] $Encoding = 'utf8-bom', [string] $Eol = "`r`n", [string] $Tail = '')
    $text = [Text.StringBuilder]::new()
    foreach ($line in $Lines) {
        [void]$text.Append($line)
        [void]$text.Append($Eol)
    }
    [void]$text.Append($Tail)
    $stream = [IO.MemoryStream]::new()
    if ($Encoding -eq 'utf8-bom') {
        $stream.Write([byte[]](0xEF, 0xBB, 0xBF), 0, 3)
        $body = [Text.UTF8Encoding]::new($false).GetBytes($text.ToString())
    } elseif ($Encoding -eq 'utf8') {
        $body = [Text.UTF8Encoding]::new($false).GetBytes($text.ToString())
    } elseif ($Encoding -eq 'utf16le-bom') {
        $stream.Write([byte[]](0xFF, 0xFE), 0, 2)
        $body = [Text.UnicodeEncoding]::new($false, $false).GetBytes($text.ToString())
    } else {
        throw ('unknown encoding ' + $Encoding)
    }
    $stream.Write($body, 0, $body.Length)
    $path = Join-Path $script:PmDir $Name
    [IO.File]::WriteAllBytes($path, $stream.ToArray())
    return $path
}

function New-PmP1Rows {
    # P1: 10 presents; MsBetweenPresents 8 x9, then 16; busy 7.0, 7.1 ... 7.9; wait 0.0, 0.1 ... 0.9.
    # $Set, if given, maps a present's index (0-based) to the columns it overrides (P13).
    param([hashtable] $Set = @{})
    $rows = [Collections.Generic.List[object]]::new()
    for ($i = 0; $i -lt 10; $i++) {
        $ms = 8.0
        if ($i -eq 9) { $ms = 16.0 }
        $override = @{}
        if ($Set.ContainsKey($i)) { $override = $Set[$i] }
        $rows.Add((New-PmRow -I $i -Ms $ms -Busy (7.0 + 0.1 * $i) -Wait (0.1 * $i) -Set $override))
    }
    return ,$rows
}

# --- Running a vector both ways -------------------------------------------------------------

function New-PmArguments([string] $File, [string] $Exe, [string] $Tag, $ProcessId, [bool] $Strict) {
    $list = [Collections.Generic.List[string]]::new()
    foreach ($word in @('-Csv', $File, '-Exe', $Exe, '-Tag', $Tag)) { $list.Add($word) }
    if ($null -ne $ProcessId) {
        $list.Add('-ProcessId')
        $list.Add([string]$ProcessId)
    }
    if ($Strict) { $list.Add('-Strict') }
    return ,$list.ToArray()
}

function Assert-PmNoFolder($Run) {
    # Output names files, never folders.
    $root = [IO.Path]::GetTempPath().TrimEnd([char]'\', [char]'/')
    foreach ($folder in @($script:PmDir, $root)) {
        Assert-True (-not $Run.StdOut.Contains($folder)) ('stdout names no folder: ' + $Run.StdOut)
        Assert-True (-not $Run.StdErr.Contains($folder)) ('stderr names no folder: ' + $Run.StdErr)
    }
}

function Invoke-PmStats {
    # One vector both ways (see the top). Returns the line parsed as JSON (Stats), the library's
    # ordered dictionary (Dictionary), its warnings and the line itself.
    param(
        [string] $File,
        [string] $Exe = 'kairo-loadgen.exe',
        [string] $Tag = 'aa-alu-1',
        $ProcessId = $null,
        [switch] $Strict
    )
    $csvText = ConvertFrom-PmCsvBytes ([IO.File]::ReadAllBytes($File))
    $lib = Get-PmStats -Text $csvText -Exe $Exe -Tag $Tag -ProcessId $ProcessId -Strict:$Strict
    $cli = Invoke-BenchTool 'pm-stats.ps1' (New-PmArguments $File $Exe $Tag $ProcessId $Strict.IsPresent)
    Assert-Equal 0 $cli.ExitCode ('exit code (stderr: ' + $cli.StdErr.Trim() + ')')
    Assert-Equal ($lib.Json + [Environment]::NewLine) $cli.StdOut 'stdout, the library''s line alone'
    $expectedErr = [Text.StringBuilder]::new()
    foreach ($warning in $lib.Warnings) { [void]$expectedErr.Append('pm-stats: warning: ' + $warning + [Environment]::NewLine) }
    Assert-Equal $expectedErr.ToString() $cli.StdErr 'stderr, the library''s warnings one line each'
    Assert-PmNoFolder $cli
    return [pscustomobject]@{
        Stats      = (ConvertFrom-Json $lib.Json)
        Dictionary = $lib.Stats
        Warnings   = @($lib.Warnings)
        Json       = $lib.Json
    }
}

function Assert-PmFails {
    # An error vector both ways. The library (Get-PmStats over the file's text, or with -Missing
    # Read-PmStatsFile) throws carrying exit code $Code, with a message matching $Pattern; the
    # command line exits $Code and prints nothing on stdout and "pm-stats: <that message>" on stderr.
    # Returns the command line's run.
    param(
        [int] $Code,
        [string] $Pattern,
        [string] $File,
        [string] $Exe = 'kairo-loadgen.exe',
        [string] $Tag = 'aa-alu-1',
        $ProcessId = $null,
        [switch] $Strict,
        [switch] $Missing
    )
    if ($Missing) {
        $failure = Assert-Throws { [void](Read-PmStatsFile $File) } 'Read-PmStatsFile throws'
    } else {
        $csvText = ConvertFrom-PmCsvBytes ([IO.File]::ReadAllBytes($File))
        $failure = Assert-Throws { [void](Get-PmStats -Text $csvText -Exe $Exe -Tag $Tag -ProcessId $ProcessId -Strict:$Strict) } 'Get-PmStats throws'
    }
    Assert-Equal $Code (Get-PmStatsExitCode $failure) ('the library''s exit code (' + $failure.Exception.Message + ')')
    Assert-Match $Pattern $failure.Exception.Message 'the library''s message'
    $cli = Invoke-BenchTool 'pm-stats.ps1' (New-PmArguments $File $Exe $Tag $ProcessId $Strict.IsPresent)
    Assert-Equal $Code $cli.ExitCode ('exit code (stderr: ' + $cli.StdErr.Trim() + ')')
    Assert-Equal '' $cli.StdOut 'stdout'
    Assert-Equal ('pm-stats: ' + $failure.Exception.Message + [Environment]::NewLine) $cli.StdErr 'stderr'
    Assert-PmNoFolder $cli
    return $cli
}

function Assert-PmSame($Expected, $Actual, [string] $What) {
    # JSON values: objects key by key, in order; numbers as numbers.
    if ($Expected -is [Management.Automation.PSCustomObject]) {
        if (-not ($Actual -is [Management.Automation.PSCustomObject])) {
            throw ($What + ': expected an object, got <' + (Format-BenchValue $Actual) + '>')
        }
        $expectedNames = @($Expected.PSObject.Properties | ForEach-Object { $_.Name })
        $actualNames = @($Actual.PSObject.Properties | ForEach-Object { $_.Name })
        Assert-Equal ($expectedNames -join ', ') ($actualNames -join ', ') ($What + ' keys')
        foreach ($name in $expectedNames) {
            Assert-PmSame $Expected.PSObject.Properties[$name].Value $Actual.PSObject.Properties[$name].Value ($What + '.' + $name)
        }
        return
    }
    Assert-Equal $Expected $Actual $What
}

function Assert-PmSubset([string] $ExpectedJson, $Actual, [string] $What) {
    # Only the keys $ExpectedJson names; their values as Assert-PmSame compares them.
    $expected = ConvertFrom-Json $ExpectedJson
    foreach ($property in $expected.PSObject.Properties) {
        $found = $Actual.PSObject.Properties[$property.Name]
        if ($null -eq $found) { throw ($What + ': no key ' + $property.Name) }
        Assert-PmSame $property.Value $found.Value ($What + '.' + $property.Name)
    }
}

function Get-PmWarnings($Result, [string] $Pattern) {
    # The warnings matching $Pattern, as an array even when there is one.
    return ,@($Result.Warnings | Where-Object { $_ -cmatch $Pattern })
}

# --- pm-stats vectors (section 6) -----------------------------------------------------------

$script:P1File = Write-PmCsv 'p1.csv' (ConvertTo-PmLines (New-PmP1Rows))

Test-Case 'P1 ten presents: section 6''s line, key for key and in order' {
    $r = Invoke-PmStats $script:P1File
    Assert-PmSame (ConvertFrom-Json $script:P1Line) $r.Stats 'P1'
    $names = @($r.Dictionary.Keys)
    Assert-Equal 27 $names.Count 'the dictionary''s keys'
    Assert-Equal 10 $r.Dictionary['frames'] 'the dictionary''s frames'
    Assert-Equal 1 $r.Warnings.Count ('warnings: ' + ($r.Warnings -join ' | '))
    Assert-Equal 1 (Get-PmWarnings $r 'fewer than 1000').Count 'the frame-count warning'
}

Test-Case 'P2 frame generation: one frame per present, GPU numbers from its Application row' {
    $rows = [Collections.Generic.List[object]]::new()
    $ms = @(10.0, 10.0, 10.0, 20.0, 20.0, 20.0)
    for ($i = 0; $i -lt 6; $i++) {
        if ($i -lt 3) { $rows.Add((New-PmRow -I $i -Ms $ms[$i] -Busy 0 -Wait 0 -Set @{ FrameType = 'AMD AFMF' })) }
        $rows.Add((New-PmRow -I $i -Ms $ms[$i] -Busy 6.0 -Wait 0.5))
    }
    $r = Invoke-PmStats (Write-PmCsv 'p2.csv' (ConvertTo-PmLines $rows))
    Assert-PmSubset '{"frames":6,"avg_fps":66.67,"low1_fps":50.0,"low01_fps":50.0,"gpu_busy_ms":6.0,"gpu_wait_ms":0.5,"present_modes":{"Hardware: Independent Flip":6},"frame_types":{"AMD AFMF":3,"Application":6},"gpu_time_ms":6.5,"duration_s":0.09,"swapchains":1,"rows":9,"bad_rows":0}' $r.Stats 'P2'
    Assert-Equal 1 (Get-PmWarnings $r 'AMD AFMF').Count ('the frame-type warning: ' + ($r.Warnings -join ' | '))
}

Test-Case 'P3 dropped presents count as frames and in dropped_pct' {
    $rows = [Collections.Generic.List[object]]::new()
    for ($i = 0; $i -lt 8; $i++) {
        $rows.Add((New-PmRow -I $i -Ms 5.0 -Busy 4.2 -Wait 0.3 -Dropped:($i -eq 2 -or $i -eq 5) -Set @{ PresentMode = 'Composed: Flip' }))
    }
    $r = Invoke-PmStats (Write-PmCsv 'p3.csv' (ConvertTo-PmLines $rows))
    Assert-PmSubset '{"frames":8,"avg_fps":200.0,"gpu_busy_ms":4.2,"gpu_wait_ms":0.3,"present_modes":{"Composed: Flip":8},"display_change_ms":5.0,"dropped_pct":25.0,"rows":8,"bad_rows":0}' $r.Stats 'P3'
}

Test-Case 'P4 the main chain, -Exe ignoring case, and another program as counts only' {
    $rows = [Collections.Generic.List[object]]::new()
    for ($i = 0; $i -lt 5; $i++) { $rows.Add((New-PmRow -I $i -Ms 10.0 -Set @{ SwapChainAddress = '0xAAAA0' })) }
    for ($i = 5; $i -lt 8; $i++) { $rows.Add((New-PmRow -I $i -Ms 30.0 -Set @{ SwapChainAddress = '0xCCCC0' })) }
    for ($i = 8; $i -lt 13; $i++) { $rows.Add((New-PmRow -I $i -Ms 12.0 -Set @{ Application = 'other.exe'; ProcessID = '5151'; SwapChainAddress = '0x9F9F0' })) }
    $r = Invoke-PmStats (Write-PmCsv 'p4.csv' (ConvertTo-PmLines $rows)) -Exe 'Kairo-Loadgen.EXE'
    Assert-PmSubset '{"frames":5,"avg_fps":100.0,"swapchains":2,"rows":8,"bad_rows":0}' $r.Stats 'P4'
    Assert-Equal 1 (Get-PmWarnings $r '^swap chains: 2;').Count ('the swap-chain warning: ' + ($r.Warnings -join ' | '))
    Assert-True (-not $r.Json.Contains('other.exe')) 'the line names no other program'
    Assert-True (-not ($r.Warnings -join ' ').Contains('other.exe')) 'the warnings name no other program'
}

Test-Case 'P5 bad rows: a shifted row, an NA frame time and a cut last line' {
    $rows = [Collections.Generic.List[object]]::new()
    for ($i = 0; $i -lt 3; $i++) { $rows.Add((New-PmRow -I $i -Ms 10.0)) }
    $rows.Add((New-PmRow -I 3 -Ms 10.0 -Set @{ Application = 'kairo,loadgen.exe' }))
    for ($i = 4; $i -lt 7; $i++) { $rows.Add((New-PmRow -I $i -Ms 10.0)) }
    $rows.Add((New-PmRow -I 7 -Ms 10.0 -Set @{ MsBetweenPresents = 'NA'; MsBetweenDisplayChange = 'NA' }))
    $tail = (ConvertTo-PmLine (New-PmRow -I 8 -Ms 10.0)).Substring(0, 40)
    $r = Invoke-PmStats (Write-PmCsv 'p5.csv' (ConvertTo-PmLines $rows) -Tail $tail)
    Assert-PmSubset '{"frames":6,"avg_fps":100.0,"frame_types":{"Application":6},"rows":7,"bad_rows":3}' $r.Stats 'P5'
    Assert-Equal 1 (Get-PmWarnings $r '^bad rows: 3').Count ('the bad-row warning: ' + ($r.Warnings -join ' | '))
}

Test-Case 'P6 no byte-order mark, LF line ends: as P1' {
    $r = Invoke-PmStats (Write-PmCsv 'p6.csv' (ConvertTo-PmLines (New-PmP1Rows)) -Encoding 'utf8' -Eol "`n")
    Assert-PmSame (ConvertFrom-Json $script:P1Line) $r.Stats 'P6'
}

$script:P7File = Write-PmCsv 'p7.csv' (ConvertTo-PmLines (New-PmP1Rows) @($script:PmColumns | Where-Object { $_ -ne 'FrameType' }))

Test-Case 'P7 no FrameType column: P1''s numbers, frame_types null, its warning' {
    $r = Invoke-PmStats $script:P7File
    Assert-PmSame (ConvertFrom-Json $script:P1Line.Replace('"frame_types":{"Application":10}', '"frame_types":null')) $r.Stats 'P7'
    Assert-Equal 1 (Get-PmWarnings $r 'FrameType').Count ('the FrameType warning: ' + ($r.Warnings -join ' | '))
}

Test-Case 'P7 no FrameType column with -Strict: exit 3' {
    [void](Assert-PmFails -Code 3 -Pattern '^missing FrameType \(-Strict\)\z' -File $script:P7File -Strict)
}

Test-Case 'P8 UTF-16LE with a byte-order mark, CRLF: as P1' {
    $r = Invoke-PmStats (Write-PmCsv 'p8.csv' (ConvertTo-PmLines (New-PmP1Rows)) -Encoding 'utf16le-bom')
    Assert-PmSame (ConvertFrom-Json $script:P1Line) $r.Stats 'P8'
}

Test-Case 'P9 two chains tie: the one seen first' {
    $rows = [Collections.Generic.List[object]]::new()
    for ($i = 0; $i -lt 4; $i++) { $rows.Add((New-PmRow -I $i -Ms 10.0 -Set @{ SwapChainAddress = '0xAAAA0' })) }
    for ($i = 4; $i -lt 8; $i++) { $rows.Add((New-PmRow -I $i -Ms 20.0 -Set @{ SwapChainAddress = '0xCCCC0' })) }
    $r = Invoke-PmStats (Write-PmCsv 'p9.csv' (ConvertTo-PmLines $rows))
    Assert-PmSubset '{"frames":4,"avg_fps":100.0,"duration_s":0.04,"swapchains":2,"rows":8}' $r.Stats 'P9'
}

Test-Case 'P10 nearest rank: 200 presents of 1 to 200 ms' {
    $rows = [Collections.Generic.List[object]]::new()
    for ($i = 0; $i -lt 200; $i++) { $rows.Add((New-PmRow -I $i -Ms ([double]($i + 1)))) }
    $r = Invoke-PmStats (Write-PmCsv 'p10.csv' (ConvertTo-PmLines $rows))
    Assert-PmSubset '{"frames":200,"avg_fps":9.95,"low1_fps":5.05,"low01_fps":5.0,"duration_s":20.1}' $r.Stats 'P10'
    # The display cadence's P99 is rank 198 too, below the largest value: at 100 frames or more a
    # P99 is not the maximum.
    Assert-PmSubset '{"display_change_ms":100.5,"display_change_p99_ms":198.0,"max_frame_ms":200.0}' $r.Stats 'P10 display cadence'
}

Test-Case 'P11 7,500 presents: the 30 hitches miss the 1% low and set the 0.1% low' {
    $lines = [Collections.Generic.List[string]]::new()
    $lines.Add([string]::Join(',', $script:PmColumns))
    for ($i = 0; $i -lt 7500; $i++) {
        $ms = 8.0 + 0.25 * ($i % 5)
        if ($i % 250 -eq 249) { $ms = 25.0 }
        $lines.Add((ConvertTo-PmLine (New-PmRow -I $i -Ms $ms -Busy (7.5 + 0.01 * ($i % 10)) -Wait (0.05 * ($i % 3)))))
    }
    $r = Invoke-PmStats (Write-PmCsv 'p11.csv' $lines.ToArray())
    Assert-PmSubset '{"frames":7500,"avg_fps":116.77,"low1_fps":111.11,"low01_fps":40.0,"gpu_busy_ms":7.545,"gpu_wait_ms":0.05,"duration_s":64.23}' $r.Stats 'P11'
    Assert-Equal 0 $r.Warnings.Count ('warnings: ' + ($r.Warnings -join ' | '))
}

Test-Case 'P12 a stall stays in: one frame of 4,500 ms' {
    $rows = [Collections.Generic.List[object]]::new()
    for ($i = 0; $i -lt 201; $i++) {
        $ms = 10.0
        if ($i -eq 100) { $ms = 4500.0 }
        $rows.Add((New-PmRow -I $i -Ms $ms))
    }
    $r = Invoke-PmStats (Write-PmCsv 'p12.csv' (ConvertTo-PmLines $rows))
    Assert-PmSubset '{"frames":201,"avg_fps":30.92,"low1_fps":100.0,"low01_fps":0.22,"max_frame_ms":4500.0,"long_frames":1,"duration_s":6.5}' $r.Stats 'P12'
    Assert-Equal 1 (Get-PmWarnings $r '^frames over 250 ms: 1,').Count ('the long-frame warning: ' + ($r.Warnings -join ' | '))
}

Test-Case 'P13 SyncInterval, AllowsTearing and PresentFlags counted per present' {
    $set = @{
        2 = @{ SyncInterval = '1' }
        3 = @{ SyncInterval = '1' }
        4 = @{ AllowsTearing = '0' }
        5 = @{ AllowsTearing = '0' }
        6 = @{ PresentFlags = '0' }
        7 = @{ PresentFlags = '0' }
    }
    $r = Invoke-PmStats (Write-PmCsv 'p13.csv' (ConvertTo-PmLines (New-PmP1Rows -Set $set)))
    $expected = $script:P1Line.Replace('"sync_interval":{"0":10}', '"sync_interval":{"0":8,"1":2}')
    $expected = $expected.Replace('"allows_tearing":{"1":10}', '"allows_tearing":{"0":2,"1":8}')
    $expected = $expected.Replace('"present_flags":{"512":10}', '"present_flags":{"0":2,"512":8}')
    Assert-PmSame (ConvertFrom-Json $expected) $r.Stats 'P13'
}

# --- Section 5.1's edges and refusals, beyond section 6's vectors ---------------------------------

Test-Case 'A present of 0 ms is a bad row, not a frame' {
    $rows = [Collections.Generic.List[object]]::new()
    for ($i = 0; $i -lt 6; $i++) { $rows.Add((New-PmRow -I $i -Ms 10.0)) }
    $rows.Add((New-PmRow -I 6 -Ms 0.0))
    $r = Invoke-PmStats (Write-PmCsv 'zero-ms.csv' (ConvertTo-PmLines $rows))
    Assert-PmSubset '{"frames":6,"avg_fps":100.0,"duration_s":0.06,"rows":7,"bad_rows":1}' $r.Stats 'a 0 ms present'
    Assert-Equal 1 (Get-PmWarnings $r '^bad rows: 1$').Count ('the bad-row warning: ' + ($r.Warnings -join ' | '))
}

Test-Case 'A frame of exactly 250 ms is not a long frame' {
    $rows = [Collections.Generic.List[object]]::new()
    for ($i = 0; $i -lt 4; $i++) { $rows.Add((New-PmRow -I $i -Ms 250.0)) }
    $r = Invoke-PmStats (Write-PmCsv 'frames-250.csv' (ConvertTo-PmLines $rows))
    Assert-PmSubset '{"frames":4,"avg_fps":4.0,"max_frame_ms":250.0,"long_frames":0,"duration_s":1.0}' $r.Stats '250 ms frames'
    Assert-Equal 0 (Get-PmWarnings $r '^frames over 250 ms').Count ('no long-frame warning: ' + ($r.Warnings -join ' | '))
}

Test-Case 'dropped_pct rounds away from zero: 1 dropped present of 800 is 0.13' {
    # 100 / 800 = 0.125 exactly, which rounding to even would print as 0.12.
    $rows = [Collections.Generic.List[object]]::new()
    for ($i = 0; $i -lt 800; $i++) { $rows.Add((New-PmRow -I $i -Ms 8.0 -Dropped:($i -eq 400))) }
    $r = Invoke-PmStats (Write-PmCsv 'dropped-800.csv' (ConvertTo-PmLines $rows))
    Assert-PmSubset '{"frames":800,"avg_fps":125.0,"duration_s":6.4,"dropped_pct":0.13,"rows":800,"bad_rows":0}' $r.Stats '800 presents'
}

Test-Case 'dropped_pct counts MsUntilDisplayed over frames: a dropped present beside a bad one' {
    # 9 frames of 10 ms and a present with no frame time, a bad row: 1 dropped of 9 frames, not of
    # 10 presents. The dropped present keeps its MsBetweenDisplayChange here (PresentMon writes NA
    # in both), so only section 5.1's column, MsUntilDisplayed, counts it.
    $rows = [Collections.Generic.List[object]]::new()
    for ($i = 0; $i -lt 9; $i++) {
        $set = @{}
        if ($i -eq 2) { $set = @{ MsUntilDisplayed = 'NA' } }
        $rows.Add((New-PmRow -I $i -Ms 10.0 -Set $set))
    }
    $rows.Add((New-PmRow -I 9 -Ms 10.0 -Set @{ MsBetweenPresents = 'NA'; MsBetweenDisplayChange = 'NA' }))
    $r = Invoke-PmStats (Write-PmCsv 'dropped-bad.csv' (ConvertTo-PmLines $rows))
    Assert-PmSubset '{"frames":9,"avg_fps":100.0,"display_change_ms":10.0,"dropped_pct":11.11,"rows":10,"bad_rows":1}' $r.Stats 'a dropped present beside a bad one'
}

Test-Case '-Strict refuses a CSV made without --qpc_time (TimeInMs): exit 3' {
    $lines = ConvertTo-PmLines (New-PmP1Rows)
    $lines[0] = $lines[0].Replace('TimeInQPC', 'TimeInMs')
    $file = Write-PmCsv 'time-in-ms.csv' $lines
    $r = Invoke-PmStats $file
    Assert-PmSame (ConvertFrom-Json $script:P1Line) $r.Stats 'without -Strict: as P1'
    [void](Assert-PmFails -Code 3 -Pattern '^missing TimeInQPC \(-Strict\)\z' -File $file -Strict)
}

Test-Case '-Strict refuses a CSV made without --track_gpu_video (no MsVideoBusy): exit 3' {
    $file = Write-PmCsv 'no-video-busy.csv' (ConvertTo-PmLines (New-PmP1Rows) @($script:PmColumns | Where-Object { $_ -ne 'MsVideoBusy' }))
    $r = Invoke-PmStats $file
    Assert-PmSame (ConvertFrom-Json $script:P1Line) $r.Stats 'without -Strict: as P1'
    [void](Assert-PmFails -Code 3 -Pattern '^missing MsVideoBusy \(-Strict\)\z' -File $file -Strict)
}

Test-Case 'Each required column, and a time column, is required: exit 3, naming it' {
    foreach ($name in @('Application', 'ProcessID', 'SwapChainAddress', 'PresentMode', 'MsUntilDisplayed', 'MsGPUBusy', 'MsGPUWait')) {
        $file = Write-PmCsv ('no-' + $name + '.csv') (ConvertTo-PmLines (New-PmP1Rows) @($script:PmColumns | Where-Object { $_ -ne $name }))
        [void](Assert-PmFails -Code 3 -Pattern ('^missing ' + $name + '\z') -File $file)
    }
    $file = Write-PmCsv 'no-time.csv' (ConvertTo-PmLines (New-PmP1Rows) @($script:PmColumns | Where-Object { $_ -ne 'TimeInQPC' }))
    [void](Assert-PmFails -Code 3 -Pattern '^missing a time column \(TimeInQPC, TimeInMs, TimeInSeconds or TimeInDateTime\)\z' -File $file)
}

# --- pm-stats error vectors (section 6) -----------------------------------------------------

Test-Case 'E1 no file: exit 3, naming the file only' {
    [void](Assert-PmFails -Code 3 -Pattern '^no such file: missing\.csv\z' -File (Join-Path $script:PmDir 'missing.csv') -Missing)
}

Test-Case 'E2 a header only: exit 3' {
    $file = Write-PmCsv 'e2.csv' @([string]::Join(',', $script:PmColumns))
    [void](Assert-PmFails -Code 3 -Pattern '^no row for kairo-loadgen\.exe \(0 rows of other processes, 0 bad rows\)\z' -File $file)
}

Test-Case 'E3 a --v2_metrics header: exit 3' {
    $header = 'Application,ProcessID,SwapChainAddress,PresentRuntime,SyncInterval,PresentFlags,AllowsTearing,PresentMode,FrameType,CPUStartQPC,FrameTime,CPUBusy,CPUWait,GPULatency,GPUTime,GPUBusy,GPUWait,VideoBusy,DisplayLatency,DisplayedTime,AnimationError,AnimationTime,MsFlipDelay'
    $row = 'kairo-loadgen.exe,4242,0x1A2B3C4D5E0,DXGI,0,512,1,Hardware: Independent Flip,Application,1000000,8.0000,0.0000,0.0000,0.0000,8.0000,7.9000,0.1000,0.0000,2.5000,8.0000,NA,NA,NA'
    $file = Write-PmCsv 'e3.csv' @($header, $row, $row.Replace(',1000000,', ',1080000,'))
    [void](Assert-PmFails -Code 3 -Pattern '^missing MsBetweenPresents \(a --v2_metrics CSV\?\)\z' -File $file)
}

Test-Case 'E4 rows only for another program: exit 3, counts only' {
    $rows = [Collections.Generic.List[object]]::new()
    for ($i = 0; $i -lt 5; $i++) { $rows.Add((New-PmRow -I $i -Ms 10.0 -Set @{ Application = 'other.exe'; ProcessID = '5151' })) }
    $file = Write-PmCsv 'e4.csv' (ConvertTo-PmLines $rows)
    $cli = Assert-PmFails -Code 3 -Pattern '^no row for kairo-loadgen\.exe \(5 rows of other processes, 0 bad rows\)\z' -File $file
    Assert-True (-not $cli.StdErr.Contains('other.exe')) 'stderr names no other program'
}

$script:E5Rows = [Collections.Generic.List[object]]::new()
for ($i = 0; $i -lt 10; $i++) {
    if ($i % 2 -eq 0) {
        $script:E5Rows.Add((New-PmRow -I $i -Ms 10.0))
    } else {
        $script:E5Rows.Add((New-PmRow -I $i -Ms 20.0 -Set @{ ProcessID = '4343'; SwapChainAddress = '0x2B3C4D5E6F0' }))
    }
}
$script:E5File = Write-PmCsv 'e5.csv' (ConvertTo-PmLines $script:E5Rows)

Test-Case 'E5 two processes of kairo-loadgen.exe without -ProcessId: exit 3' {
    [void](Assert-PmFails -Code 3 -Pattern '^2 processes named kairo-loadgen\.exe: give -ProcessId\z' -File $script:E5File)
}

Test-Case 'E5 with -ProcessId: that process''s statistics' {
    $r = Invoke-PmStats $script:E5File -ProcessId 4343
    Assert-PmSubset '{"frames":5,"avg_fps":50.0,"duration_s":0.1,"swapchains":1,"rows":5,"bad_rows":0}' $r.Stats 'E5 -ProcessId 4343'
    $r = Invoke-PmStats $script:E5File -ProcessId 4242
    Assert-PmSubset '{"frames":5,"avg_fps":100.0,"duration_s":0.05,"swapchains":1,"rows":5,"bad_rows":0}' $r.Stats 'E5 -ProcessId 4242'
}

Test-Case 'E5 with a -ProcessId that has no row: exit 3' {
    [void](Assert-PmFails -Code 3 -Pattern '^no row for process id 999 \(10 rows of kairo-loadgen\.exe with other ids\)\z' -File $script:E5File -ProcessId 999)
}

Test-Case 'E6 -Tag ../x: exit 2' {
    [void](Assert-PmFails -Code 2 -Pattern '^bad -Tag: ' -File $script:P1File -Tag '../x')
}

# --- The command line and the library around the vectors ------------------------------------

Test-Case 'A bad command line exits 2 before the file is read' {
    $missing = Join-Path $script:PmDir 'never-read.csv'
    $cases = @(
        @{ Args = @(); Says = '^pm-stats: missing -Csv' },
        @{ Args = @('-Csv', $script:P1File, '-Tag', 't'); Says = '^pm-stats: missing -Exe' },
        @{ Args = @('-Csv', $script:P1File, '-Exe', 'kairo-loadgen.exe'); Says = '^pm-stats: missing -Tag' },
        @{ Args = @('-Csv', $missing, '-Exe', 'kairo-loadgen.exe', '-Tag', '../x'); Says = '^pm-stats: bad -Tag' },
        @{ Args = @('-Csv', $missing, '-Exe', 'kairo-loadgen.exe', '-Tag', 't', '-ProcessId', '0'); Says = '^pm-stats: -ProcessId must be a positive integer' },
        @{ Args = @('-Csv', $missing, '-Exe', 'kairo-loadgen.exe', '-Tag', 't', '-ProcessId', '-1'); Says = '^pm-stats: -ProcessId must be a positive integer' },
        @{ Args = @('-Csv', $missing, '-Exe', 'kairo-loadgen.exe', '-Tag', 't', '-ProcessId', 'abc'); Says = '^pm-stats: -ProcessId must be a positive integer' },
        @{ Args = @('-Csv', $missing, '-Exe', 'C:\games\kairo-loadgen.exe', '-Tag', 't'); Says = '^pm-stats: -Exe takes a file name' },
        @{ Args = @('-Csv', $missing, '-Exe', 'kairo-loadgen.exe', '-Tag', 't', '-Bogus', '1'); Says = '^pm-stats: unknown parameter -Bogus' },
        @{ Args = @('-Csv', $missing, '-Exe', 'kairo-loadgen.exe', '-Tag', 't', 'stray'); Says = '^pm-stats: unexpected argument' }
    )
    foreach ($case in $cases) {
        $run = Invoke-BenchTool 'pm-stats.ps1' $case.Args
        $what = '[' + ($case.Args -join ' ') + ']'
        Assert-Equal 2 $run.ExitCode ($what + ' exit code (stderr: ' + $run.StdErr.Trim() + ')')
        Assert-Equal '' $run.StdOut ($what + ' stdout')
        Assert-Match $case.Says $run.StdErr ($what + ' stderr')
        Assert-PmNoFolder $run
    }
}

Test-Case '-Strict:$false from the command line runs without -Strict' {
    $run = Invoke-BenchTool 'pm-stats.ps1' @('-Csv', $script:P7File, '-Exe', 'kairo-loadgen.exe', '-Tag', 'aa-alu-1', '-Strict:$false')
    Assert-Equal 0 $run.ExitCode ('exit code (stderr: ' + $run.StdErr.Trim() + ')')
    Assert-PmSubset '{"frames":10,"frame_types":null}' (ConvertFrom-Json $run.StdOut) '-Strict:$false'
    $run = Invoke-BenchTool 'pm-stats.ps1' @('-Csv', $script:P7File, '-Exe', 'kairo-loadgen.exe', '-Tag', 'aa-alu-1', '-Strict:$true')
    Assert-Equal 3 $run.ExitCode ('-Strict:$true exit code (stderr: ' + $run.StdErr.Trim() + ')')
}

Test-Case '-Tag takes section 5.1''s pattern and nothing else' {
    foreach ($tag in @('a', '7', 'aa-alu-1', 'A.b_c-9', ('t' * 64))) {
        Assert-PmStatsArguments 'kairo-loadgen.exe' $tag
    }
    foreach ($tag in @('', '../x', 'a/b', 'a\b', 'C:x', '-x', '.x', '_x', 'a b', ('t' * 65), "a`n", ('caf' + [char]0xE9))) {
        $failure = Assert-Throws { Assert-PmStatsArguments 'kairo-loadgen.exe' $tag } ('tag <' + $tag + '>')
        Assert-Equal 2 (Get-PmStatsExitCode $failure) ('tag <' + $tag + '> exit code')
    }
}

Test-Case '-ProcessId: a positive integer or nothing' {
    Assert-Equal $null (ConvertTo-PmStatsProcessId $null) 'not given'
    Assert-Equal 4242 (ConvertTo-PmStatsProcessId 4242) 'a number'
    Assert-Equal 4242 (ConvertTo-PmStatsProcessId '4242') 'text'
    Assert-Equal 4294967295 (ConvertTo-PmStatsProcessId '4294967295') 'the largest process id'
    foreach ($bad in @(0, -1, '0', '-1', 'abc', '1.5', '', '4294967296', '0x10')) {
        $failure = Assert-Throws { [void](ConvertTo-PmStatsProcessId $bad) } ('-ProcessId <' + $bad + '>')
        Assert-Equal 2 (Get-PmStatsExitCode $failure) ('-ProcessId <' + $bad + '> exit code')
    }
}

Test-Case 'Nearest rank in integers' {
    # k = ceil(p * n): (99n + 99) div 100 for P99 and (999n + 999) div 1000 for P99.9.
    $expected = @(
        @(1, 1, 1), @(99, 99, 99), @(100, 99, 100), @(101, 100, 101), @(200, 198, 200),
        @(201, 199, 201), @(1000, 990, 999), @(1001, 991, 1000), @(7500, 7425, 7493)
    )
    foreach ($case in $expected) {
        Assert-Equal $case[1] (Get-PmStatsRank $case[0] 99 100) ('P99 rank at n = ' + $case[0])
        Assert-Equal $case[2] (Get-PmStatsRank $case[0] 999 1000) ('P99.9 rank at n = ' + $case[0])
    }
}

Test-Case 'The line is the same in a comma-decimal culture' {
    $csvText = ConvertFrom-PmCsvBytes ([IO.File]::ReadAllBytes($script:P1File))
    $thread = [Threading.Thread]::CurrentThread
    $saved = $thread.CurrentCulture
    try {
        $thread.CurrentCulture = [Globalization.CultureInfo]::GetCultureInfo('de-DE')
        $line = (Get-PmStats -Text $csvText -Exe 'kairo-loadgen.exe' -Tag 'aa-alu-1').Json
    } finally {
        $thread.CurrentCulture = $saved
    }
    Assert-PmSame (ConvertFrom-Json $script:P1Line) (ConvertFrom-Json $line) 'de-DE'
    Assert-True (-not $line.Contains('113,64')) 'no decimal comma'
}

Test-Case 'Text with its byte-order mark still in it reads as P1' {
    $csvText = [string][char]0xFEFF + [IO.File]::ReadAllText($script:P1File).TrimStart([char]0xFEFF)
    $r = Get-PmStats -Text $csvText -Exe 'kairo-loadgen.exe' -Tag 'aa-alu-1'
    Assert-PmSame (ConvertFrom-Json $script:P1Line) (ConvertFrom-Json $r.Json) 'U+FEFF first'
}

Test-Case 'Empty text and an empty file: exit 3' {
    $failure = Assert-Throws { [void](Get-PmStats -Text '' -Exe 'kairo-loadgen.exe' -Tag 't') } 'empty text'
    Assert-Equal 3 (Get-PmStatsExitCode $failure) 'empty text exit code'
    $empty = Join-Path $script:PmDir 'empty.csv'
    [IO.File]::WriteAllBytes($empty, [byte[]]@())
    [void](Assert-PmFails -Code 3 -Pattern '^the CSV is empty\z' -File $empty)
}

Test-Case 'Loaded from text as the elevated session loads it: functions only, and they throw' {
    # The elevated session dot-sources the text it received through [scriptblock]::Create under
    # StrictMode 3.0 and Stop. Loading must run no command line, leave no variable behind, and an
    # error must come back as an exception carrying its exit code, never as an exit.
    $driver = Join-Path $script:PmDir 'load-as-text.ps1'
    $lines = @(
        'param([string] $Tool, [string] $Csv)',
        'Set-StrictMode -Version 3.0',
        '$ErrorActionPreference = ''Stop''',
        '$before = @(Get-Variable -Scope 0 | ForEach-Object { $_.Name })',
        '. ([scriptblock]::Create([IO.File]::ReadAllText($Tool)))',
        '$new = @(Get-Variable -Scope 0 | ForEach-Object { $_.Name } | Where-Object { $before -notcontains $_ -and $_ -ne ''before'' })',
        '[Console]::Out.WriteLine(''variables:'' + ($new -join '',''))',
        '$text = ConvertFrom-PmCsvBytes ([IO.File]::ReadAllBytes($Csv))',
        '[Console]::Out.WriteLine((Get-PmStats -Text $text -Exe ''kairo-loadgen.exe'' -Tag ''aa-alu-1'').Json)',
        'try { [void](Get-PmStats -Text $text -Exe ''kairo-loadgen.exe'' -Tag ''../x'') } catch { [Console]::Out.WriteLine(''caught '' + (Get-PmStatsExitCode $_)) }',
        'try { [void](Get-PmStats -Text $text -Exe ''other.exe'' -Tag ''t'') } catch { [Console]::Out.WriteLine(''caught '' + (Get-PmStatsExitCode $_)) }',
        '[Console]::Out.WriteLine(''still here'')',
        'exit 0'
    )
    [IO.File]::WriteAllText($driver, ($lines -join "`n") + "`n")
    $run = Invoke-BenchTool $driver @('-Tool', (Join-Path $BenchDir 'pm-stats.ps1'), '-Csv', $script:P1File)
    Assert-Equal 0 $run.ExitCode ('exit code (stderr: ' + $run.StdErr.Trim() + ')')
    Assert-Equal '' $run.StdErr 'stderr'
    $out = @($run.StdOut.Split([char]10) | ForEach-Object { $_.TrimEnd([char]13) } | Where-Object { $_.Length -gt 0 })
    Assert-Equal 5 $out.Count ('stdout lines: ' + ($out -join ' | '))
    Assert-Equal 'variables:' $out[0] 'no variable is left behind'
    Assert-PmSame (ConvertFrom-Json $script:P1Line) (ConvertFrom-Json $out[1]) 'the line'
    Assert-Equal 'caught 2' $out[2] 'a bad tag'
    Assert-Equal 'caught 3' $out[3] 'no row for the process'
    Assert-Equal 'still here' $out[4] 'the session goes on'
}

Complete-BenchTests
