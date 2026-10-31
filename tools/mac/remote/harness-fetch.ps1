# SPDX-License-Identifier: GPL-3.0-or-later
# pc harness: fetch, read-only, a file a harness job wrote in C:\kairo\logs, as Base64 in one @@KV
# line (research 16 README section 8.7), so no scp path is involved. For `pc harness displays`
# that is harness-displays.json, which names the owner's monitors and GPU: the Mac keeps it in
# .dogfood/ only, and the job's own output holds only a count. A copy older than the job's start
# (from C:\kairo\logs\<Id>.result.json) is refused, so a failed job never passes off the last
# run's file. Refuses while the PC is busy unless --force.

Assert-PcIdle 'harness'

$id = [string]$Pc['Id']
if ($id -cnotmatch $JobIdRegex) { Fail 'the id must match ^[A-Za-z0-9_-]{1,64}$' 2 }
$name = [string]$Pc['File']
if ($name -cnotmatch '^harness-[a-z0-9-]{1,40}\.json\z') { Fail 'the file must match ^harness-[a-z0-9-]{1,40}\.json$' 2 }
$result = Read-JsonFile (Join-Path $Kairo.Logs ($id + '.result.json'))
if ($null -eq $result) { Fail ('the job ' + $id + ' has no result') }
$started = [DateTime]::MinValue
$startedText = [string](Get-Field $result 'started_utc')
$styles = [Globalization.DateTimeStyles]::AdjustToUniversal -bor [Globalization.DateTimeStyles]::AssumeUniversal
if (-not [DateTime]::TryParse($startedText, [Globalization.CultureInfo]::InvariantCulture, $styles, [ref]$started)) {
    Fail ('the job ' + $id + ' has no start time')
}
$path = Join-Path $Kairo.Logs $name
if (-not [IO.File]::Exists($path)) { Fail ('the job wrote no ' + $name) }
$info = [IO.FileInfo]::new($path)
if ($info.LastWriteTimeUtc -lt $started) { Fail ($name + ' is older than the job ' + $id + ', which did not write it') }
try {
    $text = Read-SharedText $path 1048576
} catch {
    Fail ($name + ' not read: ' + $_.Exception.Message)
}
$bytes = $Utf8NoBom.GetBytes($text)
Send-KV 'out' ([Convert]::ToBase64String($bytes))
Say ('fetched ' + $name + ' (' + $bytes.Length + ' bytes)')
Exit-Pc 0
