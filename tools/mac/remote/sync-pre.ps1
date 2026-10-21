# SPDX-License-Identifier: GPL-3.0-or-later
# pc sync, step 1: refuse when the PC is busy, then report the state of the clone C:\kairo\src
# (@@KV lines) so the Mac can skip a no-op sync and build a small bundle if it has to.

Assert-PcIdle 'sync'

$repo = $Kairo.Src
if (-not $GitExe) { Fail 'git is not installed on the PC: run the bootstrap (SETUP.md)' }
if (-not [IO.Directory]::Exists((Join-Path $repo '.git'))) {
    Fail ($repo + ' is not a git clone yet: run the bootstrap (SETUP.md) first')
}

$head = Get-FirstLine (Invoke-Git @('-C', $repo, 'rev-parse', '--verify', '-q', 'HEAD'))
$branch = Get-FirstLine (Invoke-Git @('-C', $repo, 'symbolic-ref', '-q', 'HEAD'))
# Tracked changes only: updateInstead refuses those, while an untracked file in the way just makes
# the push fail and the Mac fall back to the bundle (sync-apply.ps1 then cleans it away).
$status = Invoke-Git @('-C', $repo, 'status', '--porcelain', '--untracked-files=no')
if ($status.Code -ne 0) { Fail ('git status failed in ' + $repo + ': ' + $status.Text) }
$dirty = '0'
if (@($status.Lines | Where-Object { $_ }).Count -gt 0) { $dirty = '1' }
$deny = Get-FirstLine (Invoke-Git @('-C', $repo, 'config', '--get', 'receive.denyCurrentBranch'))

Send-KV 'head' $head
Send-KV 'branch' $branch
Send-KV 'dirty' $dirty
Send-KV 'deny' $deny
Send-KV 'git' $GitExe
Exit-Pc 0
