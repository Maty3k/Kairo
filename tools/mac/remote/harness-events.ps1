# SPDX-License-Identifier: GPL-3.0-or-later
# pc harness capture, after the job: did the display driver reset (System event 4101, provider
# Display) or the PC reboot unexpectedly (event 41, Kernel-Power) since the job started? Either is
# a P0 (M1:204): the Mac stops all PC work and reports it to the owner. Read-only: it prints each
# event's id, provider and seconds from the start, never its message. Never refuses. It covers the
# job from its start to its end (the result's ended_utc) and a minute after. Without a result:
# - \Kairo\Run still runs this job (next.json names it): it can't be covered yet;
# - kairo-devrun never started it (no start line in its log, no output files): nothing to check;
# - it ended without one (the owner signed out, the PC went down): up to its time limit
#   (MaxSeconds) or its output files' last write, whichever is later, and a minute after; and when
#   Windows started again before any clean shutdown, with its last event before that start inside
#   this window or up to 10 minutes after it, the event 41 that start logged. The 10 minutes
#   cover a Since that is the Mac's clock less a minute (a check left by ssh's 255), which can end
#   the window seconds before the job's own limit, and the System log's last writes.
# Exit codes: 0 none, 7 found, 5 the job still runs (check again once it has ended), 1 the System
# log can't be read (check it by hand first).

$id = [string]$Pc['Id']
if ($id -cnotmatch '^harness-[0-9]{8}-[0-9]{6}\z') { Fail 'the id must match ^harness-<yyyyMMdd>-<HHmmss>$' 2 }
$sinceText = [string]$Pc['Since']
if ($sinceText -cnotmatch '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{3}Z\z') { Fail 'Since must be a UTC time like 2026-11-11T20:30:00.000Z' 2 }
$since = [DateTime]::MinValue
$styles = [Globalization.DateTimeStyles]::AdjustToUniversal -bor [Globalization.DateTimeStyles]::AssumeUniversal
if (-not [DateTime]::TryParse($sinceText, [Globalization.CultureInfo]::InvariantCulture, $styles, [ref]$since)) { Fail 'Since does not read as a time' 2 }
$maxSeconds = 0
if ($Pc.ContainsKey('MaxSeconds')) { $maxSeconds = [int]$Pc['MaxSeconds'] }
if ($maxSeconds -lt 0 -or $maxSeconds -gt 86400) { Fail 'MaxSeconds must be 0 to 86400' 2 }
$invariant = [Globalization.CultureInfo]::InvariantCulture
$utc = "yyyy-MM-dd'T'HH:mm:ss.fff'Z'"
$from = $since.AddSeconds(-5).ToString($utc, $invariant)

function Read-SystemEvents([string]$XPath, [int]$Max, [switch]$Oldest) {
    # The matching System events (none is no error), or exit 1 if the log can't be read.
    # TODO(verify): NoMatchingEventsFound's id on the owner's Windows build.
    $query = @{ LogName = 'System'; FilterXPath = $XPath; ErrorAction = 'Stop' }
    if ($Max -gt 0) { $query['MaxEvents'] = $Max }
    if ($Oldest) { $query['Oldest'] = $true }
    try {
        return @(Get-WinEvent @query)
    } catch {
        if ([string]$_.FullyQualifiedErrorId -like 'NoMatchingEventsFound*') { return @() }
        Say ('error: the System log could not be read (' + $_.Exception.Message + '); check it by hand before the next job')
        Exit-Pc 1
    }
}

# The job's end, from its result.
$until = [DateTime]::MinValue
$bootRange = ''
$result = Read-JsonFile (Join-Path $Kairo.Logs ($id + '.result.json'))
$ended = [DateTime]::MinValue
if ($null -ne $result -and [DateTime]::TryParse([string](Get-Field $result 'ended_utc'), $invariant, $styles, [ref]$ended)) {
    $until = $ended.AddSeconds(60)
} else {
    $next = Read-JsonFile $Kairo.NextJob
    if ($null -ne $next -and [string](Get-Field $next 'id') -ceq $id -and (@('running', 'queued') -contains (Get-KairoTask 'Run').State)) {
        Say ('the job ' + $id + ' is still running: check again once it has ended')
        Exit-Pc 5
    }
    # kairo-devrun logs "job <id> starts:" before it runs a job (the log moves to devrun.log.1
    # past 5 MiB), and writes the job's .out.log and .err.log while it runs.
    $written = [DateTime]::MinValue
    foreach ($name in @(($id + '.out.log'), ($id + '.err.log'))) {
        $path = Join-Path $Kairo.Logs $name
        if ([IO.File]::Exists($path)) {
            $time = [IO.File]::GetLastWriteTimeUtc($path)
            if ($time -gt $written) { $written = $time }
        }
    }
    $seen = $false
    foreach ($name in @('devrun.log', 'devrun.log.1')) {
        $path = Join-Path $Kairo.Logs $name
        if (-not [IO.File]::Exists($path)) { continue }
        try {
            if ((Read-SharedText $path 33554432).Contains('job ' + $id + ' starts:')) { $seen = $true }
        } catch {
            $seen = $true  # unreadable: assume it ran
        }
    }
    if (-not $seen -and $written -eq [DateTime]::MinValue) {
        Say ('the job ' + $id + ' never ran (\Kairo\Run did not start it): nothing to check')
        Exit-Pc 0
    }
    $until = $since.AddSeconds($maxSeconds)
    if ($written -gt $until) { $until = $written }
    $until = $until.AddSeconds(60)
    # A PC that went down writes event 41 at its next start. TODO(verify): Kernel-General's 12
    # (started) and 13 (shutting down) on the owner's build, and 41's time next to 12's.
    $xpath = "*[System[Provider[@Name='Microsoft-Windows-Kernel-General'] and (EventID=12 or EventID=13) and TimeCreated[@SystemTime>='" + $from + "']]]"
    $first = @(Read-SystemEvents $xpath 1 -Oldest)
    if ($first.Count -eq 1 -and [int]$first[0].Id -eq 12) {
        $boot = $first[0].TimeCreated.ToUniversalTime()
        # The newest event before that start says when the PC went down: during the job, or later.
        $xpath = "*[System[TimeCreated[@SystemTime<'" + $boot.AddSeconds(-1).ToString($utc, $invariant) + "']]]"
        $down = @(Read-SystemEvents $xpath 1)
        if ($down.Count -eq 1 -and $down[0].TimeCreated.ToUniversalTime() -le $until.AddMinutes(10)) {
            $bootRange = " or (EventID=41 and TimeCreated[@SystemTime>='" + $boot.AddSeconds(-60).ToString($utc, $invariant) + "' and @SystemTime<='" + $boot.AddSeconds(600).ToString($utc, $invariant) + "'])"
        }
    }
}
$range = "TimeCreated[@SystemTime>='" + $from + "' and @SystemTime<='" + $until.ToString($utc, $invariant) + "']"

# TODO(verify): the provider names on the owner's Windows build.
$wanted = @{ 4101 = 'Display'; 41 = 'Microsoft-Windows-Kernel-Power' }
$events = @(Read-SystemEvents ("*[System[((EventID=4101 or EventID=41) and " + $range + ")" + $bootRange + "]]") 0)
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
