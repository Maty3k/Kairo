# SPDX-License-Identifier: GPL-3.0-or-later
#
# tools/pc/bench/tests/chain.tests.ps1 - the bench scripts read each other's lines.
# Project: https://github.com/Maty3k/Kairo
# Design: docs/research/17-loadgen-bench/README.md sections 4.6 (step 8), 4.7 and 5.
#
#   pwsh -NoProfile -File tools/pc/bench/tests/chain.tests.ps1
#
# The other test files build each script's input by hand. This one feeds the real outputs
# through, as bench-fetch will: pm-stats' lines into bench-rules (validity) and ab-stats (the A/A
# noise floor and the self-test), ab-stats' A/A line into bench-rules -Decide, and encoder-usage's
# line into bench-rules' session flag. It runs the chain twice, with the scripts dot-sourced
# together under testlib's Set-StrictMode -Version 3.0 and $ErrorActionPreference = 'Stop', as the
# elevated session loads them, and through pwsh -File with files between the steps; both must
# give the same lines. It prints one line per case and exits 0 when every case passed, 1 when one
# failed. Fixtures are built in code; synthetic names only (kairo-loadgen.exe, recorder).
#
# Rules: ASCII only; Windows PowerShell 5.1 syntax (CI parses tools/** there); no Add-Type.

. (Join-Path $PSScriptRoot 'testlib.ps1')
foreach ($chainScript in @('pm-stats.ps1', 'ab-stats.ps1', 'bench-rules.ps1', 'encoder-usage.ps1')) {
    . (Join-Path $BenchDir $chainScript)
}

$script:ChainDir = New-BenchTempDir
$script:ChainColumns = @(
    'Application', 'ProcessID', 'SwapChainAddress', 'PresentRuntime', 'SyncInterval', 'PresentFlags',
    'AllowsTearing', 'PresentMode', 'FrameType', 'TimeInQPC', 'MsBetweenSimulationStart', 'MsBetweenPresents',
    'MsBetweenDisplayChange', 'MsInPresentAPI', 'MsRenderPresentLatency', 'MsUntilDisplayed', 'CPUStartQPC',
    'MsBetweenAppStart', 'MsCPUBusy', 'MsCPUWait', 'MsGPULatency', 'MsGPUTime', 'MsGPUBusy', 'MsGPUWait',
    'MsVideoBusy', 'MsAnimationError', 'AnimationTime', 'MsFlipDelay'
)

function New-ChainCsv([double] $Ms, [int] $Presents, [string] $Mode) {
    # PresentMon 2.6.0's CSV with the bench's flags (research 17 section 5.1), as text: $Presents
    # presents of $Ms each, one swap chain, every one displayed, CRLF line ends.
    $inv = [Globalization.CultureInfo]::InvariantCulture
    $lines = [Collections.Generic.List[string]]::new()
    $lines.Add([string]::Join(',', $script:ChainColumns))
    $msText = $Ms.ToString('F14', $inv)
    for ($i = 0; $i -lt $Presents; $i++) {
        $fields = @('kairo-loadgen.exe', '4242', '0x1A2B3C4D5E0', 'DXGI', '0', '512', '1', $Mode, 'Application',
            (1000000 + 80000 * $i).ToString($inv), 'NA', $msText, $msText, '0', '0', '2.5000', '0', '0', '0', '0', '0',
            '8.0000', '7.9000', '0.1000', '0', 'NA', 'NA', 'NA')
        $lines.Add([string]::Join(',', $fields))
    }
    return ([string]::Join("`r`n", $lines) + "`r`n")
}

function New-ChainRunRecord([string] $Tag, [int] $N) {
    # A run as bench.json records it (research 17 section 4.8), one the job saw end well.
    return [ordered]@{
        n = $N; tag = $Tag; pid = 4242; csv = $Tag + '.csv'; csv_bytes = 30000; csv_ends_newline = $true
        pm_exit = 0; pm_started = $true; pm_stopped = $true; etw_loss = $false; stopped_early = $false
        loadgen_exit = 0; loadgen_alive_at_pm_end = $true; retry_of = $null; elapsed_s = 37.0
    }
}

# Seven 1-second runs of the ALU workload: frame times 8.1 to 8.7 ms, so the CV is above 1%; run 4
# presents composed, so rule 6 drops it and six runs stay valid.
$script:ChainRuns = @()
for ($r = 1; $r -le 7; $r++) {
    $mode = 'Hardware: Independent Flip'
    if ($r -eq 4) { $mode = 'Composed: Flip' }
    $script:ChainRuns += [pscustomobject]@{ Tag = ('alu-0' + $r); N = $r; Csv = (New-ChainCsv (8.0 + 0.1 * $r) 123 $mode) }
}
$script:ChainBench = [ordered]@{
    kairo_bench = 1; run_id = 'bench-20261123-101500'; started_utc = '2026-11-23T10:15:00Z'; phase = 'awaiting_decision'
    plan = [ordered]@{ workloads = @('alu'); runs = 6; retries = 3; warmup_s = 20; delay_s = 5; timed_s = 1; gap_s = 10; decide_wait_s = 300 }
    monitor = [ordered]@{ refresh_mhz = 180000; hdr = $false }
    blocks = @([ordered]@{ workload = 'alu'; pass = 1; warmup = [ordered]@{ ok = $true; seconds = 20 }; runs = @($script:ChainRuns | ForEach-Object { New-ChainRunRecord $_.Tag $_.N }) })
}
$script:ChainBenchJson = ConvertTo-Json -InputObject $script:ChainBench -Compress -Depth 10

$script:Chain = $null

function Get-Chain {
    # The chain through the functions, run once; the cases check it, and the command line against
    # it. A step that throws fails each case that needs it, with its message.
    if ($null -eq $script:Chain) {
        $lines = @($script:ChainRuns | ForEach-Object { (Get-PmStats -Text $_.Csv -Exe 'kairo-loadgen.exe' -Tag $_.Tag -ProcessId 4242 -Strict).Json })
        $rules = Get-BenchRules -Bench $script:ChainBenchJson -Stats $lines
        $workload = $rules.Rules['workloads'][0]
        $valid = @($lines | Where-Object { $workload['valid_tags'] -contains (ConvertFrom-Json $_).tag })
        $aa = Get-AbStats -A $valid
        $decision = Get-BenchDecision -Valid $workload['valid'] -AaStats $aa.Json -Workload 'alu'
        $script:Chain = [pscustomobject]@{ Lines = $lines; Rules = $rules; Workload = $workload; Valid = $valid; Aa = $aa; Decision = $decision }
    }
    return $script:Chain
}

Test-Case 'the four scripts dot-source into one session: no function defined twice, nothing printed' {
    $owners = @{}
    $twice = [Collections.Generic.List[string]]::new()
    foreach ($name in @('pm-stats.ps1', 'ab-stats.ps1', 'bench-rules.ps1', 'encoder-usage.ps1')) {
        $tokens = $null
        $errors = $null
        $ast = [Management.Automation.Language.Parser]::ParseFile((Join-Path $BenchDir $name), [ref] $tokens, [ref] $errors)
        foreach ($function in $ast.FindAll({ param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] }, $true)) {
            if ($owners.ContainsKey($function.Name)) { $twice.Add($function.Name + ' in ' + $owners[$function.Name] + ' and ' + $name) }
            else { $owners[$function.Name] = $name }
        }
    }
    Assert-Equal '' ($twice -join '; ') 'functions defined twice'
    $printed = @(& {
            foreach ($name in @('pm-stats.ps1', 'ab-stats.ps1', 'bench-rules.ps1', 'encoder-usage.ps1')) {
                . ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $BenchDir $name))))
            }
        })
    Assert-Equal 0 $printed.Count 'objects printed'
}

Test-Case 'pm-stats lines are what bench-rules judges: rule 6 drops the composed run, six stay valid' {
    $w = (Get-Chain).Workload
    Assert-Equal 'alu' $w['workload'] 'workload'
    Assert-Equal 7 $w['runs'] 'runs'
    Assert-Equal 'alu-01,alu-02,alu-03,alu-05,alu-06,alu-07' ($w['valid_tags'] -join ',') 'valid_tags'
    Assert-Equal 'Hardware: Independent Flip' $w['present_mode'] 'the workload''s present mode'
    Assert-Equal 100.0 $w['present_mode_share_pct'] 'its share'
    # 123 frames: fewer than 1,000, a flag that leaves the runs valid.
    Assert-Equal 'few_frames' ($w['flags'] -join ',') 'flags'
    $dropped = $w['results'][3]
    Assert-Equal 'alu-04' $dropped['tag'] 'the dropped run'
    Assert-Equal '6' ($dropped['failed'] -join ',') 'its rule'
    foreach ($result in $w['results']) {
        if ($result['tag'] -ne 'alu-04') { Assert-Equal '' ($result['failed'] -join ',') ($result['tag'] + ' failed rules') }
    }
}

Test-Case 'the valid runs'' pm-stats lines are what ab-stats reads: the A/A floor and the self-test' {
    $c = Get-Chain
    $fps = @($c.Valid | ForEach-Object { [double](ConvertFrom-Json $_).avg_fps })
    $mean = 0.0
    foreach ($f in $fps) { $mean += $f / $fps.Count }
    $aa = $c.Aa.Stats
    Assert-Equal 'aa' $aa['mode'] 'mode'
    Assert-Equal 6 $aa['n'] 'n'
    Assert-Near $mean $aa['mean_fps'] 1e-9 'mean_fps is the mean of the lines'' avg_fps'
    Assert-True ($null -ne $aa['low1_cv_pct']) 'low1_cv_pct, from the lines'' low1_fps'
    Assert-True ($aa['cv_pct'] -gt 1.0) ('a CV above 1%: ' + $aa['cv_pct'])
    Assert-Equal 1 $c.Aa.ExitCode 'NOISE=high exits 1'
    $self = Get-AbStatsSelfTest -Runs $c.Valid
    Assert-Equal 3 $self.Stats['n'] 'self-test pairs'
    Assert-True ($null -ne $self.Stats['clauses']['low1_le_5']) 'the 1%-low clause, evaluated from pm-stats lines'
}

Test-Case 'ab-stats'' A/A line is what bench-rules -Decide reads: 6 more runs at a CV above 1%' {
    $c = Get-Chain
    $d = $c.Decision.Decision
    Assert-Equal 'extension' $d['kind'] 'kind'
    Assert-Equal 6 $d['extra'] 'extra'
    Assert-Equal 6 $d['valid'] 'valid'
    Assert-Equal $c.Aa.Stats['cv_pct'] $d['cv_pct'] 'cv_pct, carried over'
    Assert-Equal $true $d['cv_high'] 'cv_high'
}

Test-Case 'the same chain through pwsh -File, with files between the steps, gives the same lines' {
    $c = Get-Chain
    $lines = [Collections.Generic.List[string]]::new()
    foreach ($run in $script:ChainRuns) {
        $csv = Join-Path $script:ChainDir ($run.Tag + '.csv')
        [IO.File]::WriteAllBytes($csv, [byte[]](@(0xEF, 0xBB, 0xBF) + [Text.UTF8Encoding]::new($false).GetBytes($run.Csv)))
        $out = Invoke-BenchTool 'pm-stats.ps1' @('-Csv', $csv, '-Exe', 'kairo-loadgen.exe', '-Tag', $run.Tag, '-ProcessId', '4242', '-Strict')
        Assert-Equal 0 $out.ExitCode ('pm-stats ' + $run.Tag + ' exit code (stderr: ' + $out.StdErr.Trim() + ')')
        $lines.Add($out.StdOut.Trim())
    }
    Assert-Equal ($c.Lines -join '|') ($lines -join '|') 'pm-stats lines'
    $statsFile = Join-Path $script:ChainDir 'stats.jsonl'
    [IO.File]::WriteAllText($statsFile, ($lines -join "`r`n") + "`r`n")
    $benchFile = Join-Path $script:ChainDir 'bench.json'
    [IO.File]::WriteAllText($benchFile, $script:ChainBenchJson)
    $out = Invoke-BenchTool 'bench-rules.ps1' @('-Bench', $benchFile, '-Stats', $statsFile)
    Assert-Equal 0 $out.ExitCode ('bench-rules exit code (stderr: ' + $out.StdErr.Trim() + ')')
    Assert-Equal $c.Rules.Json $out.StdOut.Trim() 'bench-rules line'
    $valid = @((ConvertFrom-Json $out.StdOut.Trim()).workloads[0].valid_tags)
    $validFile = Join-Path $script:ChainDir 'valid.jsonl'
    [IO.File]::WriteAllText($validFile, (@($lines | Where-Object { $valid -contains (ConvertFrom-Json $_).tag }) -join "`n") + "`n")
    $out = Invoke-BenchTool 'ab-stats.ps1' @('-A', $validFile, '-Json')
    Assert-Equal 1 $out.ExitCode ('ab-stats exit code, NOISE=high (stderr: ' + $out.StdErr.Trim() + ')')
    Assert-Equal $c.Aa.Json $out.StdOut.Trim() 'ab-stats line'
    $out = Invoke-BenchTool 'bench-rules.ps1' @('-Decide', '-Valid', ([string]$valid.Count), '-AaStats', $out.StdOut.Trim(), '-Workload', 'alu')
    Assert-Equal 0 $out.ExitCode ('bench-rules -Decide exit code (stderr: ' + $out.StdErr.Trim() + ')')
    Assert-Equal $c.Decision.Json $out.StdOut.Trim() 'the decision'
}

Test-Case 'encoder-usage''s line is what bench-rules counts other encoders from' {
    $luid = '_luid_0x00000000_0x000123ab_phys_0_eng_4_engtype_videocodec'
    $sample = {
        param([long] $ProcessId, [double] $Value)
        [pscustomobject]@{ InstanceName = 'pid_' + $ProcessId + $luid; CookedValue = $Value; Status = [uint32]0 }
    }
    $sets = @(
        [pscustomobject]@{ CounterSamples = @((& $sample 100 10.0), (& $sample 200 3.0), (& $sample 300 0.5)) },
        [pscustomobject]@{ CounterSamples = @((& $sample 100 12.0), (& $sample 200 5.0), (& $sample 300 0.5)) }
    )
    $snapshot = @([pscustomobject]@{ Id = 100; Name = 'kairo-loadgen' }, [pscustomobject]@{ Id = 200; Name = 'recorder' }, [pscustomobject]@{ Id = 300; Name = 'player' })
    $usage = Measure-EncoderUsage -Sets $sets -Before $snapshot -After $snapshot -Seconds 2 -Elevated $true
    $rules = Get-BenchRules -Bench $script:ChainBenchJson -Stats (Get-Chain).Lines -EncoderUsage @($usage.Json)
    # The recorder at 4% counts; Kairo's loadgen and the player at 0.5% don't.
    Assert-Equal 1 $rules.Rules['session_flags']['other_encoder_use'] 'other_encoder_use'
    Assert-True (-not $rules.Json.Contains('recorder')) 'no program''s name in the line'
}

Complete-BenchTests
