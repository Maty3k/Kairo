# SPDX-License-Identifier: GPL-3.0-or-later
# pc rollback [build-id]: point C:\kairo\bin\current back at the build before the current one
# (or at the given build) and restart the watch agent from it. Refuses while the PC is busy or a
# \Kairo\Run job is running. "pc deploy" keeps the newest 3 builds, so up to 2 steps back exist.

Assert-PcIdle 'rollback'
if ((Test-RunTaskBusy) -and -not $Pc['Force']) {
    Fail 'a \Kairo\Run job is running right now; wait for it (tools/mac/pc logs) or re-run with --force' 3
}

$dirs = @(Get-BuildDirs)
if ($dirs.Count -eq 0) { Fail ('there are no builds in ' + $Kairo.Bin) }
$current = Get-CurrentBuildId
$names = @($dirs | ForEach-Object { $_.Name })

if ($Pc['To']) {
    $target = [string]$Pc['To']
    if ($names -cnotcontains $target) { Fail ('there is no build ' + $target + '; builds: ' + ($names -join ', ')) }
} else {
    if (-not $current) { Fail 'C:\kairo\bin\current does not point at a build, so there is nothing to roll back from; name a build: pc rollback <build-id>' }
    $older = @($names | Where-Object { [string]::CompareOrdinal($_, $current) -lt 0 })
    if ($older.Count -eq 0) { Fail ('there is no build older than ' + $current + '; builds: ' + ($names -join ', ')) }
    $target = $older[-1]
}

if ($target -ceq $current) {
    Say ('C:\kairo\bin\current already points at ' + $target)
    Exit-Pc 0
}
Set-CurrentBuild $target
$was = 'none'
if ($current) { $was = $current }
Say ('rolled back: C:\kairo\bin\current -> ' + $target + ' (was ' + $was + ')')
Restart-WatchAgent
foreach ($name in $names) {
    $mark = '  '
    if ($name -ceq $target) { $mark = '* ' }
    $commit = Get-BuildCommit $name
    Say ($mark + $name + '  commit ' + $commit)
}
Exit-Pc 0
