# SPDX-License-Identifier: GPL-3.0-or-later
# pc sync, step 2: make C:\kairo\src match the snapshot exactly: branch 'snapshot' checked out
# at $Pc['Sha'] with a clean working tree. After a push into the updateInstead clone this is only a
# check. After the fallback (a git bundle copied with scp) it fetches the bundle first.
# The clone is Claude's workspace: anything changed there by hand is discarded (the Mac has it).

$repo = $Kairo.Src
$sha = [string]$Pc['Sha']
if ($sha -cnotmatch '^[0-9a-f]{40}\z') { Fail 'internal: the snapshot id is not a full commit id' 2 }
if (-not $GitExe) { Fail 'git is not installed on the PC' }

if ($Pc['Bundle']) {
    $bundle = [string]$Pc['Bundle']
    if (-not [IO.File]::Exists($bundle)) { Fail ('the bundle did not arrive at ' + $bundle) }
    $verify = Invoke-Git @('-C', $repo, 'bundle', 'verify', '--quiet', $bundle)
    if ($verify.Code -ne 0) { Fail ('the bundle does not fit ' + $repo + ': ' + $verify.Text) }
    $fetch = Invoke-Git @('-C', $repo, 'fetch', '--quiet', '--no-tags', $bundle, 'refs/heads/snapshot')
    Remove-Item -LiteralPath $bundle -Force -ErrorAction SilentlyContinue
    if ($fetch.Code -ne 0) { Fail ('git fetch from the bundle failed: ' + $fetch.Text) }
}

$present = Invoke-Git @('-C', $repo, 'cat-file', '-e', ($sha + '^{commit}'))
if ($present.Code -ne 0) { Fail ('commit ' + $sha + ' is not in ' + $repo + ' after the transfer') }

function Get-CloneState {
    $status = Invoke-Git @('-C', $repo, 'status', '--porcelain', '--untracked-files=normal')
    if ($status.Code -ne 0) { Fail ('git status failed in ' + $repo + ': ' + $status.Text) }
    return [pscustomobject]@{
        Head    = Get-FirstLine (Invoke-Git @('-C', $repo, 'rev-parse', '--verify', '-q', 'HEAD'))
        Branch  = Get-FirstLine (Invoke-Git @('-C', $repo, 'symbolic-ref', '-q', 'HEAD'))
        Changes = @($status.Lines | Where-Object { $_ })
    }
}

$state = Get-CloneState
$inSync = ($state.Head -ceq $sha) -and ($state.Branch -ceq 'refs/heads/snapshot') -and ($state.Changes.Count -eq 0)
if (-not $inSync) {
    if ($state.Changes.Count -gt 0) {
        Say ('discarding ' + $state.Changes.Count + ' local change(s) in ' + $repo + ':')
        $state.Changes | Select-Object -First 20 | ForEach-Object { Say ('  ' + $_) }
    }
    $checkout = Invoke-Git @('-C', $repo, 'checkout', '--quiet', '--force', '-B', 'snapshot', $sha)
    if ($checkout.Code -ne 0) { Fail ('git checkout of the snapshot failed: ' + $checkout.Text) }
    $clean = Invoke-Git @('-C', $repo, 'clean', '-d', '--force', '--quiet')
    if ($clean.Code -ne 0) { Fail ('git clean failed: ' + $clean.Text) }
    $state = Get-CloneState
    $inSync = ($state.Head -ceq $sha) -and ($state.Branch -ceq 'refs/heads/snapshot') -and ($state.Changes.Count -eq 0)
    if (-not $inSync) {
        Fail ('C:\kairo\src still differs from the snapshot (HEAD ' + $state.Head + ', ' + $state.Changes.Count + ' change(s))')
    }
}

Say ('C:\kairo\src is at ' + $sha.Substring(0, 12) + ' on branch snapshot, working tree clean')
Send-KV 'head' $sha
Exit-Pc 0
