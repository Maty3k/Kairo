# SPDX-License-Identifier: GPL-3.0-or-later
# pc logs [id]: without an id, list the newest job and build results in C:\kairo\logs; with an
# id, print <id>.result.json and the ends of <id>.out.log, <id>.err.log and <id>.log (for example
# hello.log, which kairo-hello writes). Refuses while the PC is busy unless --force.

Assert-PcIdle 'logs'

$logs = $Kairo.Logs
if (-not [IO.Directory]::Exists($logs)) { Fail ($logs + ' does not exist yet') }
$tail = 200
if ($Pc['Tail']) { $tail = [int]$Pc['Tail'] }

if (-not $Pc['Id']) {
    $results = @(Get-ChildItem -LiteralPath $logs -Filter '*.result.json' -File -ErrorAction SilentlyContinue |
        Sort-Object -Property LastWriteTimeUtc -Descending | Select-Object -First 20)
    if ($results.Count -eq 0) {
        Say ('no job results in ' + $logs + ' yet')
        Exit-Pc 0
    }
    Say ('newest results in ' + $logs + ' (details: tools/mac/pc logs <id>):')
    foreach ($file in $results) {
        $data = Read-JsonFile $file.FullName
        $id = $file.Name.Substring(0, $file.Name.Length - '.result.json'.Length)
        if ($null -eq $data) {
            Say ('  ' + $id + '  (unreadable)')
            continue
        }
        $ended = [string](Get-Field $data 'ended_utc')
        $exitCode = Get-Field $data 'exit_code'
        if ($null -eq $exitCode) { $exitCode = 'none' }
        $line = '  ' + $ended.PadRight(25) + ' ' + $id.PadRight(28) + ' exit=' + $exitCode + ' reason=' + [string](Get-Field $data 'reason')
        if ((Get-Field $data 'aborted') -eq $true) { $line += ' (aborted)' }
        Say $line
    }
    Exit-Pc 0
}

$id = [string]$Pc['Id']
if ($id -cnotmatch $JobIdRegex) { Fail 'the id must match ^[A-Za-z0-9_-]{1,64}$' 2 }
$found = $false
foreach ($name in @(($id + '.result.json'), ($id + '.out.log'), ($id + '.err.log'), ($id + '.log'))) {
    $path = Join-Path $logs $name
    if (-not [IO.File]::Exists($path)) { continue }
    $found = $true
    if ($name.EndsWith('.result.json')) {
        Say ('--- ' + $path + ' ---')
        try {
            Say-Indented (Read-SharedText $path).Trim()
        } catch {
            Say ('  (not read: ' + $_.Exception.Message + ')')
        }
    } else {
        Show-Tail $path $tail
    }
}
if (-not $found) { Fail ('there are no logs for "' + $id + '" in ' + $logs) }
Exit-Pc 0
