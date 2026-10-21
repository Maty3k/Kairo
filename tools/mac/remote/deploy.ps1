# SPDX-License-Identifier: GPL-3.0-or-later
# pc deploy: copy the executables of the last successful "pc build" into C:\kairo\bin\<build-id>,
# point the junction C:\kairo\bin\current at it, restart the watch agent from the new build and
# keep the newest 3 builds. Refuses while the PC is busy or a \Kairo\Run job is running.
# <build-id> is <UTC yyyyMMdd-HHmmss>-<first 12 hex digits of the built commit>.

Assert-PcIdle 'deploy'
if ((Test-RunTaskBusy) -and -not $Pc['Force']) {
    Fail 'a \Kairo\Run job is running right now; wait for it (tools/mac/pc logs) or re-run with --force' 3
}

$last = Read-JsonFile $Kairo.LastBuild
if ($null -eq $last) { Fail ('there is no record of a build (' + $Kairo.LastBuild + '): run tools/mac/pc build first') }
$buildLog = [string](Get-Field $last 'id')
$code = Get-Field $last 'exit_code'
if (($code -ne 0) -and -not $Pc['Force']) {
    Fail ('the last build (' + $buildLog + ') did not succeed (exit ' + $code + '): fix it and run tools/mac/pc build again')
}
$commit = [string](Get-Field $last 'commit')
if ($commit -cnotmatch '^[0-9a-f]{40}\z') { Fail ('the last build has no commit id (' + $commit + '): run tools/mac/pc sync and build') }
$artifacts = [string](Get-Field $last 'artifacts_dir')
if (-not $artifacts -or -not [IO.Directory]::Exists($artifacts)) {
    Fail ('the folder with the built executables is missing (' + $artifacts + '): run tools/mac/pc build')
}
# Only build output from C:\kairo\src\target, where pc build puts it, ever reaches C:\kairo\bin.
$targetRoot = [IO.Path]::GetFullPath($Kairo.Target).TrimEnd([char]'\') + '\'
$artifactsFull = [IO.Path]::GetFullPath($artifacts).TrimEnd([char]'\') + '\'
if (-not $artifactsFull.StartsWith($targetRoot, [StringComparison]::OrdinalIgnoreCase)) {
    Fail ('the last build record names ' + $artifacts + ', which is not under ' + $Kairo.Target + '; run tools/mac/pc build again')
}

$head = Get-FirstLine (Invoke-Git @('-C', $Kairo.Src, 'rev-parse', '--verify', '-q', 'HEAD'))
if ($head -and ($head -cne $commit)) {
    Say ('warning: C:\kairo\src is now at ' + $head.Substring(0, 12) + ', but the last build is of ' + $commit.Substring(0, 12) + '; deploying that build (run pc build to deploy the newer code)')
}

$files = @(Get-ChildItem -LiteralPath $artifacts -File | Where-Object { @('.exe', '.pdb', '.dll') -contains $_.Extension.ToLowerInvariant() })
$exes = @($files | Where-Object { $_.Extension -ieq '.exe' })
if ($exes.Count -eq 0) { Fail ('there are no .exe files in ' + $artifacts) }
if (-not ($exes | Where-Object { $_.Name -ieq 'kairo-devrun.exe' })) {
    Say 'warning: this build has no kairo-devrun.exe; \Kairo\Run, \Kairo\Ensure and \Kairo\Engine all start C:\kairo\bin\current\kairo-devrun.exe'
}

$invariant = [Globalization.CultureInfo]::InvariantCulture
$now = [DateTime]::UtcNow
$id = $now.ToString('yyyyMMdd-HHmmss', $invariant) + '-' + $commit.Substring(0, 12)
if ($id -cnotmatch $BuildIdRegex) { Fail ('internal: bad build id ' + $id) }
$destination = Join-Path $Kairo.Bin $id
if ([IO.Directory]::Exists($destination)) { Fail ($destination + ' already exists; try again in a second') }
[void](New-Item -ItemType Directory -Force -Path $destination)
foreach ($file in $files) {
    Copy-Item -LiteralPath $file.FullName -Destination (Join-Path $destination $file.Name) -Force
}
$info = [ordered]@{
    build_id      = $id
    commit        = $commit
    build_log     = $buildLog
    built_utc     = (Get-Field $last 'ended_utc')
    deployed_utc  = $now.ToString("yyyy-MM-dd'T'HH:mm:ss'Z'", $invariant)
    files         = @($files | ForEach-Object { $_.Name })
}
Write-TextFileAtomic (Join-Path $destination 'build.json') ($info | ConvertTo-Json -Compress -Depth 4)

$previous = Get-CurrentBuildId
Set-CurrentBuild $id
$was = 'none'
if ($previous) { $was = $previous }
Say ('deployed ' + $id + ' (' + $files.Count + ' files: ' + (@($exes | ForEach-Object { $_.Name }) -join ', ') + ')')
Say ('C:\kairo\bin\current -> ' + $id + ' (was ' + $was + ')')
Restart-WatchAgent
Remove-OldBuilds

$engine = Get-KairoTask 'Engine'
if ($engine.State -eq 'running') {
    Say 'note: \Kairo\Engine is still running the previous build; it picks up this one the next time it starts'
}
Exit-Pc 0
