# SPDX-License-Identifier: GPL-3.0-or-later
# pc build: cargo build --release --locked of the workspace in C:\kairo\src, as a hidden \Kairo\Run job
# in the owner's session at below-normal priority. So every build script and proc-macro of every
# dependency runs with the owner's limited token, never with this elevated session's (M1 contract,
# Builds; the owner agreed on 2026-10-24). It builds into C:\kairo\build\target, which the owner's
# account can write; nothing elevated ever runs what lands there: pc deploy copies it into the
# admin-owned C:\kairo\bin, and only the \Kairo\ tasks, as the owner, run it.
# The job stops at the owner's first keyboard or mouse input (kairo-devrun), and this script stops it
# while it is connected if a fullscreen window comes to the front (neither with --force). Build jobs
# may run while the owner is away (the owner agreed on 2026-10-31).
# Logged like any job, by kairo-devrun as the owner (C:\kairo\logs\build-<utc>.{out,err}.log and
# .result.json, so "pc logs" lists it; kairo-devrun keeps the logs of the ten builds before the current
# one), and recorded in the admin-only C:\kairo\bin\last-build.json for "pc deploy": first as running,
# so a build that never finishes can't leave the last success in place. This session writes and
# deletes nothing in logs, state or build (lib.ps1).
# Exit code: 0 built, 1 failed (a time-out included), 3 stopped because the owner became active.

Assert-PcIdle 'build'

function Format-Utc([DateTime]$Time) {
    return $Time.ToUniversalTime().ToString("yyyy-MM-dd'T'HH:mm:ss.fff'Z'", [Globalization.CultureInfo]::InvariantCulture)
}

$Admins = @('S-1-5-32-544', 'S-1-5-18')

# Any right to add, change or delete what is inside, including the generic write and all bits.
$WriteRights = [int][Security.AccessControl.FileSystemRights]'WriteData, AppendData, WriteExtendedAttributes, WriteAttributes, DeleteSubdirectoriesAndFiles, Delete, ChangePermissions, TakeOwnership'
$WriteRights = $WriteRights -bor 0x50000000

function Find-OtherWriter($Acl, [string[]]$Allowed) {
    # The first SID outside $Allowed that an Allow entry, explicit or inherited, lets change the folder.
    foreach ($rule in @($Acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier]))) {
        if (([string]$rule.AccessControlType -ne 'Allow') -or ($Allowed -contains $rule.IdentityReference.Value)) { continue }
        if (([int]$rule.FileSystemRights -band $WriteRights) -ne 0) { return $rule.IdentityReference.Value }
    }
    return $null
}

function Initialize-BuildFolder {
    # C:\kairo\build is the owner's to write, like logs and state (tools/pc/bootstrap.ps1 makes all three):
    # owned by Administrators, inheriting C:\kairo's entries, plus Modify for the owner's account (the
    # account of this session). A PC bootstrapped before builds moved has none, so it is made here, as the
    # bootstrap makes it: first owned by Administrators with an empty list, so only administrators can touch
    # it; then, still empty, it inherits C:\kairo's entries and gets the owner's Modify entry.
    $dir = $Kairo.Build
    if (-not [IO.Directory]::Exists($dir)) {
        $security = [Security.AccessControl.DirectorySecurity]::new()
        $security.SetSecurityDescriptorSddlForm('O:BAD:')
        [void][IO.FileSystemAclExtensions]::Create([IO.DirectoryInfo]::new($dir), $security)
        foreach ($change in @(@('/inheritance:e'), @('/grant', ('*{0}:(OI)(CI)M' -f $OwnerSid)))) {
            $set = Invoke-Native -Exe $IcaclsExe -Arguments (@($dir) + $change + @('/C', '/Q'))
            if ($set.Code -ne 0) { Fail ('could not set the permissions of ' + $dir + ': ' + $set.Text) }
        }
    }
    $item = Get-Item -LiteralPath $dir -Force
    if (([int]$item.Attributes -band [int][IO.FileAttributes]::ReparsePoint) -ne 0) { Fail ($dir + ' is a link; refusing to build into it') }
    $acl = Get-Acl -LiteralPath $dir
    if ($acl.GetOwner([Security.Principal.SecurityIdentifier]).Value -cne 'S-1-5-32-544') {
        Fail ($dir + ' is not owned by Administrators; run the bootstrap again (tools/pc/bootstrap.ps1 sets it up)')
    }
    $modify = [int][Security.AccessControl.FileSystemRights]::Modify
    $writable = @($acl.GetAccessRules($true, $false, [Security.Principal.SecurityIdentifier]) | Where-Object {
            $_.IdentityReference.Value -ceq $OwnerSid -and [string]$_.AccessControlType -eq 'Allow' -and
            (([int]$_.FileSystemRights -band $modify) -eq $modify) -and ([int]$_.InheritanceFlags -eq 3) -and ([int]$_.PropagationFlags -eq 0)
        })
    if ($writable.Count -eq 0) { Fail ($dir + ' does not let the owner''s account write it; run the bootstrap again') }
    # Only the owner's account may change it: another account could otherwise swap a build script cargo
    # runs as the owner, or a program between the build and pc deploy.
    $other = Find-OtherWriter $acl ($Admins + $OwnerSid)
    if ($other) { Fail ($dir + ' lets ' + $other + ' change it; refusing to build (run the bootstrap again)') }
}

function Assert-CargoConfig {
    # cargo reads .cargo\config.toml (and the older .cargo\config) in every parent folder of C:\kairo\src
    # (doc.rust-lang.org/cargo/reference/config.html), and the build runs as the owner, so a config that
    # another account planted would run its programs as the owner. C:\ lets any signed-in user make
    # folders, so the bootstrap makes C:\.cargo empty and admin-only: refuse unless it still is, and refuse
    # a config there or in C:\kairo\.cargo (which only an administrator can make).
    $guard = Join-Path $env:SystemDrive '.cargo'
    if (-not [IO.Directory]::Exists($guard)) {
        Fail ($guard + ' is missing: the bootstrap makes it, empty and admin-only, so that no other account can plant a Cargo config; run tools/mac/pc doctor')
    }
    $item = Get-Item -LiteralPath $guard -Force
    if (([int]$item.Attributes -band [int][IO.FileAttributes]::ReparsePoint) -ne 0) { Fail ($guard + ' is a link; refusing to build') }
    $acl = Get-Acl -LiteralPath $guard
    if ($Admins -notcontains $acl.GetOwner([Security.Principal.SecurityIdentifier]).Value) {
        Fail ($guard + ' is not owned by Administrators or SYSTEM; refusing to build (run tools/mac/pc doctor)')
    }
    $other = Find-OtherWriter $acl $Admins
    if ($other) { Fail ($guard + ' lets ' + $other + ' change it; refusing to build (run the bootstrap again)') }
    foreach ($dir in @($guard, (Join-Path $Kairo.Root '.cargo'))) {
        if (-not [IO.Directory]::Exists($dir)) { continue }
        try {
            $names = @([IO.Directory]::GetFileSystemEntries($dir) | ForEach-Object { [IO.Path]::GetFileName($_) })
        } catch {
            Fail ('cannot list ' + $dir + ': ' + $_.Exception.Message)
        }
        $configs = @($names | Where-Object { $_ -like 'config*' })
        if ($configs.Count -gt 0) {
            Fail ($dir + ' holds a Cargo config (' + ($configs -join ', ') + '), which the build would read; refusing. Ask the owner what it is.')
        }
    }
}

# Builds run as kairo-devrun jobs, so they need a deployed kairo-devrun. A new or wiped PC has none:
# its first build needs the one-time step in the M1 contract (Builds), with the owner's OK.
if (-not [IO.File]::Exists((Join-Path $Kairo.Current 'kairo-devrun.exe'))) {
    Fail 'no kairo-devrun is deployed (C:\kairo\bin\current), so builds cannot run as the owner yet; a new or wiped PC needs the one-time first build in the M1 contract (Builds): ask the owner'
}
$repo = $Kairo.Src
if (-not [IO.File]::Exists((Join-Path $repo 'Cargo.toml'))) {
    Fail ('there is no Cargo.toml in ' + $repo + ': run tools/mac/pc sync first')
}
# Only checked for, never run from here: a program in the owner's profile runs as the owner.
$cargo = $CargoExe
if (-not [IO.File]::Exists($cargo)) { Fail ('cargo is not installed on the PC (' + $cargo + '): see SETUP.md, or run tools/mac/pc doctor') }
Assert-CargoConfig

$others = @(Get-Process -Name 'cargo' -ErrorAction SilentlyContinue)
if ($others.Count -gt 0 -and -not $Pc['Force']) {
    Fail ('cargo is already running on the PC (pid ' + (($others | ForEach-Object { $_.Id }) -join ', ') + '); wait for it, or re-run with --force')
}

$commit = Get-FirstLine (Invoke-Git @('-C', $repo, 'rev-parse', '--verify', '-q', 'HEAD'))
$porcelain = Invoke-Git @('-C', $repo, 'status', '--porcelain', '--untracked-files=normal')
$dirty = (@($porcelain.Lines | Where-Object { $_ }).Count -gt 0)
$lockTracked = ((Invoke-Git @('-C', $repo, 'ls-files', '--error-unmatch', '--', 'Cargo.lock')).Code -eq 0)
if (-not $lockTracked) {
    Fail 'Cargo.lock is not in the snapshot; builds use only a committed Cargo.lock (commit it, then sync again)'
}

Initialize-BuildFolder
$targetDir = $Kairo.Target
$cargoArgs = @('build', '--release', '--locked', '--target-dir', $targetDir)
$packages = @($Pc['Packages'] | Where-Object { $_ })
if ($packages.Count -gt 0) {
    foreach ($package in $packages) { $cargoArgs += @('-p', $package) }
} else {
    $cargoArgs += '--workspace'
}

$invariant = [Globalization.CultureInfo]::InvariantCulture
$id = 'build-' + [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss', $invariant)
$job = [ordered]@{
    id             = $id
    cmd            = @($cargo) + $cargoArgs
    cwd            = $repo
    max_seconds    = 3600
    abort_on_input = (-not $Pc['Force'])
    window         = 'hidden'
    priority       = 'below_normal'
}
$what = $commit
if (-not $what) { $what = '(no commit)' }
if ($dirty) { $what += ' with uncommitted changes on the PC' }
Say ('building ' + $what + ' as job ' + $id + ' in the owner''s session: cargo ' + ($cargoArgs -join ' '))

# A fullscreen window means the owner is back (a game, a video): stop, as builds always have. This
# runs only while this session is connected; kairo-devrun's own input stop doesn't need it.
$stopWhen = {
    if ($Pc['Force']) { return '' }
    $watch = Get-KairoWatch
    if ($watch.State -eq 'busy') { return $watch.Reason }
    return ''
}
function Write-BuildRecord($Id, $Fields) {
    $record = [ordered]@{ id = $Id; commit = $commit; dirty = $dirty; locked = $lockTracked; packages = $packages }
    foreach ($key in $Fields.Keys) { $record[$key] = $Fields[$key] }
    Write-TextFileAtomic $Kairo.LastBuild ($record | ConvertTo-Json -Compress -Depth 4)
}
# Running until this script records how it ended: pc deploy refuses anything but a finished success.
Write-BuildRecord $id ([ordered]@{ exit_code = $null; aborted = $false; reason = 'running' })
$run = Start-KairoJob ($job | ConvertTo-Json -Compress)
$waited = Wait-KairoJob $run ($job.max_seconds + 30) $stopWhen
$result = $waited.Result
if (($null -ne $result) -and ([string](Get-Field $result 'reason') -ceq 'invalid_job')) {
    $why = ''
    try { $why = Read-SharedText $run.ErrFile } catch { }
    if ($why -notmatch 'priority') {
        Say 'error: kairo-devrun refused the build job:'
        Say-Indented $why.Trim()
        Exit-Pc 1
    }
    # The deployed kairo-devrun predates the job's priority field: build once at normal priority; the
    # deploy that follows brings a kairo-devrun that knows it. \Kairo\Run may still be ending the refusal.
    Say 'note: the deployed kairo-devrun does not know "priority" yet, so this one build runs at normal priority'
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    while (((Get-KairoTask 'Run').State -eq 'running') -and ([DateTime]::UtcNow -lt $deadline)) { Start-Sleep -Milliseconds 500 }
    $job.Remove('priority')
    # Its own id, so the refused attempt's result can't pass for this one's.
    $id = $id + '-n'
    $job.id = $id
    Write-BuildRecord $id ([ordered]@{ exit_code = $null; aborted = $false; reason = 'running' })
    $run = Start-KairoJob ($job | ConvertTo-Json -Compress)
    $waited = Wait-KairoJob $run ($job.max_seconds + 30) $stopWhen
    $result = $waited.Result
}
$ended = [DateTime]::UtcNow
$stoppedFor = $waited.StoppedFor
if ($stoppedFor) {
    # This session ended kairo-devrun with the job, so no result file was written; the record below
    # (in the admin-only bin\) says why.
    $result = [ordered]@{ exit_code = $null; aborted = $true; reason = 'busy' }
}
if ($null -eq $result) {
    $info = Get-KairoTask 'Run'
    if ($info.State -eq 'running') {
        Stop-KairoRun
        Fail ('the build ran past ' + $job.max_seconds + ' s and was stopped')
    }
    Say ('error: \Kairo\Run ended without writing ' + $run.ResultFile + ' (task last run ' + $info.LastRun + ', last result ' + $info.LastResult + ').')
    Say 'The task only runs while the owner is signed in (interactive logon). kairo-devrun''s own log:'
    Show-Tail (Join-Path $Kairo.Logs 'devrun.log') 20
    Exit-Pc 1
}
$exitCode = Get-Field $result 'exit_code'
$aborted = ((Get-Field $result 'aborted') -eq $true)
$reason = [string](Get-Field $result 'reason')

# Where did the executables go? target\release, or target\<triple>\release if the workspace
# sets build.target; pick the folder with the newest .exe.
$artifacts = ''
$newest = [DateTime]::MinValue
foreach ($dir in @((Join-Path $targetDir 'release'), (Join-Path $targetDir 'x86_64-pc-windows-msvc\release'))) {
    if (-not [IO.Directory]::Exists($dir)) { continue }
    $exe = Get-ChildItem -LiteralPath $dir -Filter '*.exe' -File -ErrorAction SilentlyContinue |
        Sort-Object -Property LastWriteTimeUtc -Descending | Select-Object -First 1
    if ($exe -and $exe.LastWriteTimeUtc -gt $newest) {
        $newest = $exe.LastWriteTimeUtc
        $artifacts = $dir
    }
}
Write-BuildRecord $id ([ordered]@{
        exit_code     = $exitCode
        aborted       = $aborted
        reason        = $reason
        artifacts_dir = $artifacts
        started_utc   = (Format-Utc $run.Started)
        ended_utc     = (Format-Utc $ended)
    })

$seconds = [int]($ended - $run.Started).TotalSeconds
$tail = 60
if ($Pc['Tail']) { $tail = [int]$Pc['Tail'] }
if ($stoppedFor -or ($aborted -and $reason -ceq 'input')) {
    $why = $reason
    if ($stoppedFor) { $why = $stoppedFor }
    Say ('STOPPED after ' + $seconds + ' s: the owner became active (' + $why + '). Nothing was deployed.')
    Show-Tail $run.ErrFile 10 -Since $run.Started
    Exit-Pc 3
}
if ($aborted) {
    Say ('BUILD STOPPED by kairo-devrun after ' + $seconds + ' s (' + $reason + '; the limit is ' + $job.max_seconds + ' s). Nothing was deployed.')
    Show-Tail $run.OutFile $tail -Since $run.Started
    Show-Tail $run.ErrFile $tail -Since $run.Started
    Exit-Pc 1
}
if (($reason -cne 'exited') -or ($exitCode -ne 0)) {
    Say ('BUILD FAILED (' + $reason + ', cargo exit ' + $exitCode + ') after ' + $seconds + ' s')
    Show-Tail $run.OutFile $tail -Since $run.Started
    Show-Tail $run.ErrFile $tail -Since $run.Started
    Exit-Pc 1
}
Show-Tail $run.ErrFile ([Math]::Min($tail, 12)) -Since $run.Started
Say ('built ' + $commit + ' in ' + $seconds + ' s; executables in ' + $artifacts)
Say 'next: tools/mac/pc deploy'
Exit-Pc 0
