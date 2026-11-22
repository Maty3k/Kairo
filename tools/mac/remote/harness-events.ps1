# SPDX-License-Identifier: GPL-3.0-or-later
# pc harness capture, after the job: did the display driver reset (System event 4101, provider
# Display) or the PC reboot unexpectedly (event 41, Kernel-Power) since the job started? Either is
# a P0 (M1:204): the Mac stops all PC work and reports it to the owner. Read-only: it prints each
# event's id, provider and seconds from the start, never its message. Never refuses. It covers the
# job from its start to its end (the result's ended_utc) and a minute after; while the job has no
# result and \Kairo\Run still runs, the check can't cover it yet.
# Exit codes: 0 none, 7 found, 5 the job still runs (check again once it has ended), 1 the System
# log can't be read (check it by hand first).

$id = [string]$Pc['Id']
if ($id -cnotmatch '^harness-[0-9]{8}-[0-9]{6}\z') { Fail 'the id must match ^harness-<yyyyMMdd>-<HHmmss>$' 2 }
$sinceText = [string]$Pc['Since']
if ($sinceText -cnotmatch '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{3}Z\z') { Fail 'Since must be a UTC time like 2026-11-11T20:30:00.000Z' 2 }
$since = [DateTime]::MinValue
$styles = [Globalization.DateTimeStyles]::AdjustToUniversal -bor [Globalization.DateTimeStyles]::AssumeUniversal
if (-not [DateTime]::TryParse($sinceText, [Globalization.CultureInfo]::InvariantCulture, $styles, [ref]$since)) { Fail 'Since does not read as a time' 2 }
$invariant = [Globalization.CultureInfo]::InvariantCulture
$from = $since.AddSeconds(-5).ToString("yyyy-MM-dd'T'HH:mm:ss.fff'Z'", $invariant)

# The job's end, from its result; without one, the job is either still running or died without it.
$range = "TimeCreated[@SystemTime>='" + $from + "']"
$result = Read-JsonFile (Join-Path $Kairo.Logs ($id + '.result.json'))
$ended = [DateTime]::MinValue
if ($null -ne $result -and [DateTime]::TryParse([string](Get-Field $result 'ended_utc'), $invariant, $styles, [ref]$ended)) {
    $until = $ended.AddSeconds(60).ToString("yyyy-MM-dd'T'HH:mm:ss.fff'Z'", $invariant)
    $range = "TimeCreated[@SystemTime>='" + $from + "' and @SystemTime<='" + $until + "']"
} elseif (Test-RunTaskBusy) {
    Say ('the job ' + $id + ' is still running: check again once it has ended')
    Exit-Pc 5
}

# TODO(verify): the provider names on the owner's Windows build, and NoMatchingEventsFound's id.
$wanted = @{ 4101 = 'Display'; 41 = 'Microsoft-Windows-Kernel-Power' }
$events = @()
try {
    $xpath = "*[System[(EventID=4101 or EventID=41) and " + $range + "]]"
    $events = @(Get-WinEvent -LogName System -FilterXPath $xpath -ErrorAction Stop)
} catch {
    if ([string]$_.FullyQualifiedErrorId -notlike 'NoMatchingEventsFound*') {
        Say ('error: the System log could not be read (' + $_.Exception.Message + '); check it by hand before the next job')
        Exit-Pc 1
    }
}
$found = 0
foreach ($event in $events) {
    $provider = [string]$event.ProviderName
    if ($wanted[[int]$event.Id] -cne $provider) { continue }
    $offset = [Math]::Round(($event.TimeCreated.ToUniversalTime() - $since).TotalSeconds, 1)
    Say ('found: event ' + $event.Id + ' (' + $provider + ') at ' + $offset + ' s from the job''s start')
    $found++
}
if ($found -gt 0) {
    Say ('P0: ' + $found + ' display-driver reset or unexpected reboot event(s) during ' + $id)
    Exit-Pc 7
}
Say ('no display-driver reset or unexpected reboot since ' + $id + ' started')
Exit-Pc 0
