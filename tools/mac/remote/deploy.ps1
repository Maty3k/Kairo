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
# A build that never recorded its end may have left anything in the build folder: never deployed,
# --force included.
if ([string](Get-Field $last 'reason') -ceq 'running') {
    Fail ('the last build (' + $buildLog + ') never recorded how it ended (stopped, or the session dropped); run tools/mac/pc build again')
}
if (($code -ne 0) -and -not $Pc['Force']) {
    Fail ('the last build (' + $buildLog + ') did not succeed (exit ' + $code + '): fix it and run tools/mac/pc build again')
}
$commit = [string](Get-Field $last 'commit')
if ($commit -cnotmatch '^[0-9a-f]{40}\z') { Fail ('the last build has no commit id (' + $commit + '): run tools/mac/pc sync and build') }
$artifacts = [string](Get-Field $last 'artifacts_dir')
if (-not $artifacts -or -not [IO.Directory]::Exists($artifacts)) {
    Fail ('the folder with the built executables is missing (' + $artifacts + '): run tools/mac/pc build')
}
# Only build output from C:\kairo\build\target, where pc build puts it, ever reaches C:\kairo\bin.
$targetRoot = [IO.Path]::GetFullPath($Kairo.Target).TrimEnd([char]'\') + '\'
$artifactsFull = [IO.Path]::GetFullPath($artifacts).TrimEnd([char]'\') + '\'
if (-not $artifactsFull.StartsWith($targetRoot, [StringComparison]::OrdinalIgnoreCase)) {
    Fail ('the last build record names ' + $artifacts + ', which is not under ' + $Kairo.Target + '; run tools/mac/pc build again')
}

$head = Get-FirstLine (Invoke-Git @('-C', $Kairo.Src, 'rev-parse', '--verify', '-q', 'HEAD'))
if ($head -and ($head -cne $commit)) {
    Say ('warning: C:\kairo\src is now at ' + $head.Substring(0, 12) + ', but the last build is of ' + $commit.Substring(0, 12) + '; deploying that build (run pc build to deploy the newer code)')
}

# Kairo's own programs only: kairo*.exe and testbed.exe (the M1 contract's names), with their .pdb and
# .dll files. Anything else in the build folder is listed, never copied.
$all = @(Get-ChildItem -LiteralPath $artifacts -File | Where-Object { @('.exe', '.pdb', '.dll') -contains $_.Extension.ToLowerInvariant() })
$files = @($all | Where-Object { $_.Name -match '^(kairo[A-Za-z0-9_-]*|testbed)\.(exe|pdb|dll)\z' })
$skipped = @($all | Where-Object { $files -notcontains $_ } | ForEach-Object { $_.Name })
if ($skipped.Count -gt 0) { Say ('warning: not deployed, as not Kairo''s: ' + ($skipped -join ', ')) }
$exes = @($files | Where-Object { $_.Extension -ieq '.exe' })
if ($exes.Count -eq 0) { Fail ('there are no Kairo programs in ' + $artifacts) }
if (-not ($exes | Where-Object { $_.Name -ieq 'kairo-devrun.exe' })) {
    # \Kairo\Run, \Kairo\Ensure and \Kairo\Engine all start bin\current\kairo-devrun.exe, and so do pc build,
    # run and cli: without one, nothing could run, and no build could make one.
    if (-not $Pc['Force']) {
        Fail 'this build has no kairo-devrun.exe, which every \Kairo\ task and pc build, run and cli need; build it too (pc build, or -p kairo-devrun), or deploy with --force'
    }
    Say 'warning: this build has no kairo-devrun.exe, so the \Kairo\ tasks, pc build, run and cli stop working, and no build can bring one back: run tools/mac/pc rollback --force to get back'
}

# The owner's account can write C:\kairo\build (build.ps1), so any name there could be a link to a file only
# Administrators may read, such as the SSH host key. Each file is opened once, checked through that same
# handle and copied from it, and only if the owner's account owns it, so it is nothing the owner couldn't
# read anyway. (.NET opens files with anonymous impersonation: a link to a named pipe gets no token either.)
$ownerSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
function Copy-OwnedFile([string]$Source, [string]$Destination) {
    # '' once copied; otherwise why not.
    try {
        $in = [IO.File]::Open($Source, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    } catch {
        return ('could not open ' + $Source + ': ' + $_.Exception.Message)
    }
    try {
        $owner = ''
        if ($in.CanSeek) { $owner = [IO.FileSystemAclExtensions]::GetAccessControl($in).GetOwner([Security.Principal.SecurityIdentifier]).Value }
        if ($owner -cne $ownerSid) { return ($Source + ' is not a file the owner''s account made (owner: ' + $owner + ')') }
        $out = [IO.File]::Open($Destination, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        try { $in.CopyTo($out) } finally { $out.Dispose() }
        return ''
    } catch {
        return ('could not copy ' + $Source + ': ' + $_.Exception.Message)
    } finally {
        $in.Dispose()
    }
}

$invariant = [Globalization.CultureInfo]::InvariantCulture
$now = [DateTime]::UtcNow
$id = $now.ToString('yyyyMMdd-HHmmss', $invariant) + '-' + $commit.Substring(0, 12)
if ($id -cnotmatch $BuildIdRegex) { Fail ('internal: bad build id ' + $id) }
$destination = Join-Path $Kairo.Bin $id
if ([IO.Directory]::Exists($destination)) { Fail ($destination + ' already exists; try again in a second') }
[void](New-Item -ItemType Directory -Force -Path $destination)
foreach ($file in $files) {
    $problem = Copy-OwnedFile $file.FullName (Join-Path $destination $file.Name)
    if ($problem) {
        Remove-Item -LiteralPath $destination -Recurse -Force -ErrorAction SilentlyContinue
        Fail ($problem + '; nothing was deployed (run tools/mac/pc build again)')
    }
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
# Only the \Kairo\ tasks run these, as the owner (nothing elevated runs a build: build.ps1); Administrators
# own the deployed copy so that programs running as the owner can't swap a file under a running task.
Set-AdminOwner $destination

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
