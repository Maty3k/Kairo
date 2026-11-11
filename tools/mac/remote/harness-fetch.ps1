# SPDX-License-Identifier: GPL-3.0-or-later
# pc harness: fetch, read-only, what a harness job wrote in C:\kairo\logs, as Base64 in @@KV lines
# (research 16 README section 8.7), so no scp path is involved. Three modes:
#  - File: one file in the logs folder, harness-<name>.json (pc harness displays: it names the
#    owner's monitors and GPU, so the Mac keeps it in .dogfood/ only).
#  - Files: a run folder's reports, from a fixed list (harness.json, verify.json, verify.txt),
#    with the job's result (exit code, reason, seconds).
#  - Clip: the run folder's capture.mp4 in 48 KiB blocks, with its size and SHA-256.
# A copy older than the job's start (from C:\kairo\logs\<Id>.result.json) is refused, so a failed
# job never passes off an earlier run's file. Nothing in the logs folder is created, renamed or
# deleted. Refuses while the PC is busy unless --force.

Assert-PcIdle 'harness'

$id = [string]$Pc['Id']
if ($id -cnotmatch $JobIdRegex) { Fail 'the id must match ^[A-Za-z0-9_-]{1,64}$' 2 }
$result = Read-JsonFile (Join-Path $Kairo.Logs ($id + '.result.json'))
if ($null -eq $result) { Fail ('the job ' + $id + ' has no result') }
$started = [DateTime]::MinValue
$startedText = [string](Get-Field $result 'started_utc')
$styles = [Globalization.DateTimeStyles]::AdjustToUniversal -bor [Globalization.DateTimeStyles]::AssumeUniversal
if (-not [DateTime]::TryParse($startedText, [Globalization.CultureInfo]::InvariantCulture, $styles, [ref]$started)) {
    Fail ('the job ' + $id + ' has no start time')
}

function Assert-Fresh([string]$Path, [string]$Name) {
    # The file exists and the job wrote it, not an earlier run.
    if (-not [IO.File]::Exists($Path)) { return $false }
    if ([IO.FileInfo]::new($Path).LastWriteTimeUtc -lt $started) {
        Fail ($Name + ' is older than the job ' + $id + ', which did not write it')
    }
    return $true
}

if ($Pc.ContainsKey('File')) {
    $name = [string]$Pc['File']
    if ($name -cnotmatch '^harness-[a-z0-9-]{1,40}\.json\z') { Fail 'the file must match ^harness-[a-z0-9-]{1,40}\.json$' 2 }
    $path = Join-Path $Kairo.Logs $name
    if (-not (Assert-Fresh $path $name)) { Fail ('the job wrote no ' + $name) }
    try {
        $text = Read-SharedText $path 1048576
    } catch {
        Fail ($name + ' not read: ' + $_.Exception.Message)
    }
    $bytes = $Utf8NoBom.GetBytes($text)
    Send-KV 'out' ([Convert]::ToBase64String($bytes))
    Say ('fetched ' + $name + ' (' + $bytes.Length + ' bytes)')
    Exit-Pc 0
}

# A run folder: the job's own id names it.
if ($id -cnotmatch '^harness-[0-9]{8}-[0-9]{6}\z') { Fail 'a run folder''s id must match ^harness-<yyyyMMdd>-<HHmmss>$' 2 }
$folder = Join-Path $Kairo.Logs $id
Send-KV 'result_exit' ([string](Get-Field $result 'exit_code'))
Send-KV 'result_reason' ([string](Get-Field $result 'reason'))
$ended = [DateTime]::MinValue
if ([DateTime]::TryParse([string](Get-Field $result 'ended_utc'), [Globalization.CultureInfo]::InvariantCulture, $styles, [ref]$ended)) {
    Send-KV 'result_seconds' ([string][Math]::Round(($ended - $started).TotalSeconds, 1))
}

if ($Pc['Clip']) {
    $path = Join-Path $folder 'capture.mp4'
    if (-not (Assert-Fresh $path 'capture.mp4')) { Fail 'the run wrote no capture.mp4' }
    $limit = 512MB
    $stream = $null
    try {
        $stream = Open-ReadFile $path
        if ($stream.Length -gt $limit) { Fail 'capture.mp4 is larger than 512 MiB' }
        $sha = [Security.Cryptography.SHA256]::Create()
        $buffer = [byte[]]::new(49152)
        $total = 0
        while ($true) {
            $count = 0
            while ($count -lt $buffer.Length) {
                $n = $stream.Read($buffer, $count, $buffer.Length - $count)
                if ($n -le 0) { break }
                $count += $n
            }
            if ($count -le 0) { break }
            $total += $count
            if ($total -gt $limit) { Fail 'capture.mp4 grew past 512 MiB while it was read' }
            [void]$sha.TransformBlock($buffer, 0, $count, $null, 0)
            Send-KV 'clip' ([Convert]::ToBase64String($buffer, 0, $count))
            if ($count -lt $buffer.Length) { break }
        }
        [void]$sha.TransformFinalBlock([byte[]]::new(0), 0, 0)
        Send-KV 'clip_sha256' (([BitConverter]::ToString($sha.Hash)).Replace('-', '').ToLowerInvariant())
        Send-KV 'clip_bytes' ([string]$total)
        Say ('fetched capture.mp4 (' + $total + ' bytes)')
    } catch {
        Fail ('capture.mp4 not read: ' + $_.Exception.Message)
    } finally {
        if ($null -ne $stream) { $stream.Dispose() }
    }
    Exit-Pc 0
}

$allowed = @('harness.json', 'verify.json', 'verify.txt')
$fetched = 0
foreach ($name in @($Pc['Files'])) {
    if ($allowed -cnotcontains [string]$name) { Fail ('not a file a run folder holds: ' + $name) 2 }
    $path = Join-Path $folder $name
    if (-not (Assert-Fresh $path $name)) {
        Say ('missing: ' + $name)
        continue
    }
    try {
        $text = Read-SharedText $path 1048576
    } catch {
        Say ('not read: ' + $name + ' (' + $_.Exception.Message + ')')
        continue
    }
    $key = $name.Replace('.', '_')
    Send-KV $key ([Convert]::ToBase64String($Utf8NoBom.GetBytes($text)))
    $fetched++
}
Say ('fetched ' + $fetched + ' of ' + @($Pc['Files']).Count + ' files from ' + $id)
Exit-Pc 0
