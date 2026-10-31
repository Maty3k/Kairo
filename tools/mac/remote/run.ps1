# SPDX-License-Identifier: GPL-3.0-or-later
# pc run: hand one job to \Kairo\Run, which runs "kairo-devrun.exe run --job C:\kairo\jobs\next.json"
# in the owner's session (interactive, unelevated). Then wait for C:\kairo\logs\<id>.result.json
# and print it with the job's output. Refuses while the PC is busy unless --force.
# Exit code: 0 ok, 1 the job failed or could not run, 2 bad job, 3/4 refused, 5 still running
# when the wait ended, 6 kairo-devrun stopped the job (time limit or owner input).

Assert-PcIdle 'run'

$jobText = $Utf8NoBom.GetString([Convert]::FromBase64String([string]$Pc['JobB64']))
$run = Start-KairoJob $jobText
$job = $run.Job
$abortText = 'no'
if ($job.AbortOnInput) { $abortText = 'yes' }
Say ('started job ' + $job.Id + ': ' + $job.Program + ' (window ' + $job.Window + ', priority ' + $job.Priority + ', at most ' + $job.MaxSeconds + ' s, stops on owner input: ' + $abortText + ')')

$waitSeconds = $job.MaxSeconds + 30
if ($Pc.ContainsKey('Wait')) { $waitSeconds = [int]$Pc['Wait'] }
if ($waitSeconds -le 0) {
    Say ('not waiting; read the result later with: tools/mac/pc logs ' + $job.Id)
    Exit-Pc 0
}
$result = (Wait-KairoJob $run $waitSeconds).Result

$tail = 200
if ($Pc['Tail']) { $tail = [int]$Pc['Tail'] }
if ($null -eq $result) {
    $info = Get-KairoTask 'Run'
    if ($info.State -eq 'running') {
        Say ('job ' + $job.Id + ' is still running after ' + $waitSeconds + ' s; read the result later with: tools/mac/pc logs ' + $job.Id)
        Exit-Pc 5
    }
    Say ('error: \Kairo\Run ended without writing ' + $run.ResultFile + ' (task last run ' + $info.LastRun + ', last result ' + $info.LastResult + ').')
    Say 'The task only runs while the owner is signed in (interactive logon). kairo-devrun''s own log:'
    Show-Tail (Join-Path $Kairo.Logs 'devrun.log') 20
    Show-Tail $run.ErrFile $tail -Since $run.Started
    Exit-Pc 1
}

Say 'result:'
try {
    Say-Indented (Read-SharedText $run.ResultFile).Trim()
} catch {
    Say ('  (not read: ' + $_.Exception.Message + ')')
}
Show-Tail $run.OutFile $tail -Since $run.Started
Show-Tail $run.ErrFile $tail -Since $run.Started
Show-Tail $run.OwnLog 40 -Since $run.Started
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
