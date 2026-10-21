# SPDX-License-Identifier: GPL-3.0-or-later
# pc run: hand one job to \Kairo\Run, which runs "kairo-devrun.exe run --job C:\kairo\jobs\next.json"
# in the owner's session (interactive, unelevated). Then wait for C:\kairo\logs\<id>.result.json
# and print it with the job's output. Refuses while the PC is busy unless --force.
# Exit code: 0 ok, 1 the job failed or could not run, 2 bad job, 3/4 refused, 5 still running
# when the wait ended, 6 kairo-devrun stopped the job (time limit or owner input).

Assert-PcIdle 'run'

function Test-KairoJob([string]$Text) {
    # The same rules as crates/kairo-core/src/job.rs, checked again here so that a bad file
    # never reaches C:\kairo\jobs\next.json.
    try { $job = Read-JsonText $Text } catch { Fail ('the job is not valid JSON: ' + $_.Exception.Message) 2 }
    if (-not ($job -is [Collections.IDictionary])) { Fail 'the job must be a JSON object' 2 }
    $known = @('id', 'cmd', 'cwd', 'max_seconds', 'abort_on_input', 'window')
    foreach ($key in @($job.Keys)) {
        if ($known -cnotcontains $key) { Fail ('the job has an unknown field "' + $key + '" (kairo-devrun rejects unknown fields)') 2 }
    }
    foreach ($key in @('id', 'cmd', 'max_seconds')) {
        if (-not $job.Contains($key)) { Fail ('the job has no "' + $key + '"') 2 }
    }
    $id = $job['id']
    if (-not ($id -is [string]) -or ($id -cnotmatch $JobIdRegex)) { Fail 'the job "id" must match ^[A-Za-z0-9_-]{1,64}$' 2 }
    $cmd = $job['cmd']
    if (-not ($cmd -is [object[]]) -or $cmd.Count -eq 0) { Fail 'the job "cmd" must be a non-empty list of strings' 2 }
    foreach ($part in $cmd) {
        if (-not ($part -is [string]) -or ($part -match '[\x00-\x1f]')) { Fail 'every "cmd" entry must be a string without control characters' 2 }
    }
    if ([string]$cmd[0] -eq '') { Fail 'the job "cmd" starts with an empty program' 2 }
    $cwd = $null
    if ($job.Contains('cwd') -and ($null -ne $job['cwd'])) {
        $cwd = $job['cwd']
        if (-not ($cwd -is [string]) -or ($cwd -eq '') -or ($cwd -match '[\x00-\x1f]')) { Fail 'the job "cwd" must be null or a folder' 2 }
    }
    $max = $job['max_seconds']
    if (-not ($max -is [long]) -or ($max -lt 1) -or ($max -gt 86400)) { Fail 'the job "max_seconds" must be a whole number from 1 to 86400' 2 }
    $abort = $true
    if ($job.Contains('abort_on_input')) {
        $abort = $job['abort_on_input']
        if (-not ($abort -is [bool])) { Fail 'the job "abort_on_input" must be true or false' 2 }
    }
    $window = 'hidden'
    if ($job.Contains('window')) {
        $window = $job['window']
        if (-not ($window -is [string]) -or (@('visible', 'hidden') -cnotcontains $window)) { Fail 'the job "window" must be "visible" or "hidden"' 2 }
    }
    return [pscustomobject]@{
        Id           = $id
        Program      = [string]$cmd[0]
        MaxSeconds   = [int]$max
        AbortOnInput = $abort
        Window       = $window
        Cwd          = $cwd
    }
}

$jobText = $Utf8NoBom.GetString([Convert]::FromBase64String([string]$Pc['JobB64']))
$job = Test-KairoJob $jobText

$devrun = Join-Path $Kairo.Current 'kairo-devrun.exe'
if (-not [IO.File]::Exists($devrun)) {
    Fail ('there is no ' + $devrun + ' yet: build and deploy first (tools/mac/pc build, then tools/mac/pc deploy)')
}
$task = Get-KairoTask 'Run'
if (-not $task.Exists) { Fail 'the scheduled task \Kairo\Run is not registered (the bootstrap creates it)' }
if ($task.State -eq 'disabled') { Fail 'the scheduled task \Kairo\Run is disabled' }
if ($task.State -eq 'running') {
    Fail 'another \Kairo\Run job is still running, and \Kairo\Run ignores new starts until it ends; wait for it (tools/mac/pc logs) and try again'
}
if ([IO.Path]::IsPathRooted($job.Program) -and -not [IO.File]::Exists($job.Program)) {
    Say ('warning: ' + $job.Program + ' does not exist on the PC')
}
if ($job.AbortOnInput -and $Pc['Force'] -and ((Get-KairoWatch).State -eq 'busy')) {
    Say 'note: abort_on_input is true and the owner is active, so kairo-devrun will probably stop the job at the next input'
}

$resultFile = Join-Path $Kairo.Logs ($job.Id + '.result.json')
$outFile = Join-Path $Kairo.Logs ($job.Id + '.out.log')
$errFile = Join-Path $Kairo.Logs ($job.Id + '.err.log')
$ownLog = Join-Path $Kairo.Logs ($job.Id + '.log')
foreach ($stale in @($resultFile, $outFile, $errFile)) {
    if ([IO.File]::Exists($stale)) {
        try { Remove-Item -LiteralPath $stale -Force -ErrorAction Stop } catch { Fail ('cannot remove the old ' + $stale + ': ' + $_.Exception.Message) }
    }
}
[void](New-Item -ItemType Directory -Force -Path $Kairo.Jobs)
[void](New-Item -ItemType Directory -Force -Path $Kairo.Logs)
Write-TextFileAtomic $Kairo.NextJob $jobText

$started = [DateTime]::UtcNow
# TODO(verify): schtasks /run takes the folder form \Kairo\Run (the M0 contract; documented for /create).
$trigger = Invoke-Native -Exe $SchtasksExe -Arguments @('/run', '/tn', '\Kairo\Run')
if ($trigger.Code -ne 0) { Fail ('schtasks /run /tn \Kairo\Run failed: ' + $trigger.Text) }
$abortText = 'no'
if ($job.AbortOnInput) { $abortText = 'yes' }
Say ('started job ' + $job.Id + ': ' + $job.Program + ' (window ' + $job.Window + ', at most ' + $job.MaxSeconds + ' s, stops on owner input: ' + $abortText + ')')

$waitSeconds = $job.MaxSeconds + 30
if ($Pc.ContainsKey('Wait')) { $waitSeconds = [int]$Pc['Wait'] }
if ($waitSeconds -le 0) {
    Say ('not waiting; read the result later with: tools/mac/pc logs ' + $job.Id)
    Exit-Pc 0
}

$deadline = $started.AddSeconds($waitSeconds)
$sawRunning = $false
$idleSince = $null
$nextNote = 60
$result = $null
while ([DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 1000
    if ([IO.File]::Exists($resultFile)) {
        $result = Read-JsonFile $resultFile
        if ($null -ne $result) { break }
    }
    if ((Get-KairoTask 'Run').State -eq 'running') {
        $sawRunning = $true
        $idleSince = $null
    } else {
        if ($null -eq $idleSince) { $idleSince = [DateTime]::UtcNow }
        $grace = 5
        if (-not $sawRunning) { $grace = 20 }   # the task may take a moment to start
        if (([DateTime]::UtcNow - $idleSince).TotalSeconds -gt $grace) { break }
    }
    $elapsed = ([DateTime]::UtcNow - $started).TotalSeconds
    if ($elapsed -ge $nextNote) {
        Say ('... job ' + $job.Id + ' is still running (' + [int]$elapsed + ' s)')
        $nextNote += 60
    }
}

$tail = 200
if ($Pc['Tail']) { $tail = [int]$Pc['Tail'] }
if ($null -eq $result) {
    $info = Get-KairoTask 'Run'
    if ($info.State -eq 'running') {
        Say ('job ' + $job.Id + ' is still running after ' + $waitSeconds + ' s; read the result later with: tools/mac/pc logs ' + $job.Id)
        Exit-Pc 5
    }
    Say ('error: \Kairo\Run ended without writing ' + $resultFile + ' (task last run ' + $info.LastRun + ', last result ' + $info.LastResult + ').')
    Say 'The task only runs while the owner is signed in (interactive logon). kairo-devrun''s own log:'
    Show-Tail (Join-Path $Kairo.Logs 'devrun.log') 20
    Show-Tail $errFile $tail
    Exit-Pc 1
}

Say 'result:'
Say-Indented (Read-SharedText $resultFile).Trim()
Show-Tail $outFile $tail
Show-Tail $errFile $tail
Show-Tail $ownLog 40
$exitCode = Get-Field $result 'exit_code'
$reason = [string](Get-Field $result 'reason')
if ((Get-Field $result 'aborted') -eq $true) {
    Say ('job ' + $job.Id + ' was stopped by kairo-devrun (' + $reason + ')')
    Exit-Pc 6
}
if ($reason -and ($reason -cne 'exited')) {
    Say ('job ' + $job.Id + ' did not run normally (' + $reason + ')')
    Exit-Pc 1
}
if ($exitCode -ne 0) {
    Say ('job ' + $job.Id + ' exited with code ' + $exitCode)
    Exit-Pc 1
}
Say ('job ' + $job.Id + ' finished OK')
Exit-Pc 0
