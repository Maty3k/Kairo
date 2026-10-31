# SPDX-License-Identifier: GPL-3.0-or-later
# pc cli ping|keys [--pipe NAME]: run C:\kairo\bin\current\kairo-cli.exe as a short, hidden \Kairo\Run
# job in the owner's session. It talks to a pipe server (later the engine, started by \Kairo\Engine)
# as the same user. Builds run as the owner (build.ps1), so nothing they produce runs in this elevated
# session. One \Kairo\Run job runs at a time, so it can't reach a kairo-hello that `pc run hello` started:
# that job holds \Kairo\Run (read hello's keys with tools/mac/pc logs hello). Refuses while busy unless
# --force. kairo-devrun keeps only the last 3 cli runs' logs.

Assert-PcIdle 'cli'

$cli = Join-Path $Kairo.Current 'kairo-cli.exe'
if (-not [IO.File]::Exists($cli)) {
    Fail ('there is no ' + $cli + ' yet: build and deploy first (tools/mac/pc build, then tools/mac/pc deploy)')
}
if (Test-RunTaskBusy) {
    Fail 'a \Kairo\Run job is running (pc run hello, a build or another job), and kairo-cli runs as one too, so it would have to wait; read hello''s keys with tools/mac/pc logs hello, or wait for the job (tools/mac/pc logs)'
}
$arguments = @($Pc['CliArgs'] | Where-Object { $null -ne $_ })
$invariant = [Globalization.CultureInfo]::InvariantCulture
$job = [ordered]@{
    id             = 'cli-' + [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss', $invariant)
    cmd            = @($cli) + $arguments
    cwd            = $null
    max_seconds    = 30
    abort_on_input = $true
    window         = 'hidden'
}
$run = Start-KairoJob ($job | ConvertTo-Json -Compress)
$result = (Wait-KairoJob $run 60).Result
if ($null -eq $result) {
    Say ('error: the kairo-cli job wrote no result (see tools/mac/pc logs ' + $job.id + ')')
    Exit-Pc 1
}
foreach ($file in @($run.OutFile, $run.ErrFile)) {
    if (-not [IO.File]::Exists($file)) { continue }
    try {
        $text = (Read-SharedText $file).Trim()
    } catch {
        Say ('  (' + $file + ' not read: ' + $_.Exception.Message + ')')
        continue
    }
    if ($text) { Say-Indented $text }
}
$exitCode = Get-Field $result 'exit_code'
$reason = [string](Get-Field $result 'reason')
if (($reason -cne 'exited') -or ($exitCode -ne 0)) {
    Say ('kairo-cli did not succeed (' + $reason + ', exit code ' + $exitCode + ')')
    Exit-Pc 1
}
Exit-Pc 0
