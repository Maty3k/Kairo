# SPDX-License-Identifier: GPL-3.0-or-later
# pc amd-diff before|after: the AMD settings diff (research 17 section 5.5; docs/milestones/M1.md,
# Formats), two calls per toggle. "before" takes a snapshot of AMD's registry settings and returns
# it as "@@KV snapshot=<Base64 JSON>". The owner toggles one setting in AMD Software. "after" gets
# that snapshot back in the prelude (BeforeB64), takes a new one with the same scopes, prints the
# values that changed, and returns its snapshot and the lines ("@@KV diff=<Base64 text>").
# The functions are tools/pc/bench/regdiff.ps1's: the Mac sends its text in the prelude
# (RegdiffB64), and it is dot-sourced from memory, so its command line stays off and nothing is
# deployed or written to disk. It only reads: every key is opened read-only, and no key, value or
# file is created, changed or deleted (scans.tests.ps1's R7 checks this file for registry writes).
# The owner's keys come from HKEY_USERS\<SID>, with the SID of \Kairo\Run's principal; hives show
# as HKCU and HKLM. A name or string shows only when regdiff.ps1's plain test passes it (that
# file's header gives it: at most 64 characters of a plain set, so no path; no SID-shaped word; no
# program's .exe or .dll other than Kairo's own kairo-<name>.exe; no GUID in a string; and none of
# the hidden names, the owner's and this session's account names and SIDs among them), and
# otherwise as its length and an 8-hex HMAC-SHA256 under the key the Mac sends (RegdiffKey, 32 hex
# digits: pc amd-diff before makes it and keeps it in .dogfood/amd-diff/<stamp>/key, and after
# sends the same one). No message quotes the key. Every regdiff call runs inside a try: its own
# errors keep their fixed messages, and any other error (a bug, whose message could quote a path,
# a SID or registry data) shows only its exception's type and line.
# A fullscreen window in front refuses even with --force: a game may be running. The owner is at
# the keyboard by design, so each call passes --force with the owner's OK in that session.
# Exit code: 0 printed; 1 a hive couldn't be read, or the Mac sent no key; 2 bad input (no before
# snapshot or a bad one, one taken with another key, --driver after a before snapshot without it);
# 3 PC busy; 4 busy state unknown.

Assert-PcIdle 'amd-diff' $true

function Get-AmdDiffProblem($ErrorRecord, [int] $Code) {
    # A caught error as a message and exit code: regdiff's own message, or for anything else (a
    # bug, whose message could quote a path or a name) only its type and line.
    $exception = $ErrorRecord.Exception
    if ($exception.Data.Contains('KairoExit')) {
        return [pscustomobject]@{ Message = $exception.Message; Code = [int]$exception.Data['KairoExit'] }
    }
    $line = 0
    if ($null -ne $ErrorRecord.InvocationInfo) { $line = $ErrorRecord.InvocationInfo.ScriptLineNumber }
    # A .NET method's exception arrives wrapped: name the method's own.
    while ($exception -is [Management.Automation.MethodInvocationException] -and $null -ne $exception.InnerException) { $exception = $exception.InnerException }
    return [pscustomobject]@{ Message = ('internal error: ' + $exception.GetType().Name + ' at line ' + $line); Code = $Code }
}

$mode = [string]$Pc['Mode']
if ($mode -cne 'before' -and $mode -cne 'after') { Fail 'internal: the mode must be before or after' 2 }
if (-not $Pc['RegdiffB64']) { Fail 'internal: the Mac sent no regdiff.ps1' }

# regdiff.ps1's functions, from the text the Mac sent: dot-sourced, so its command line doesn't run.
$problem = $null
try {
    . ([scriptblock]::Create($Utf8NoBom.GetString([Convert]::FromBase64String([string]$Pc['RegdiffB64']))))
} catch {
    $problem = Get-AmdDiffProblem $_ 1
}
if ($null -ne $problem) { Fail ('regdiff.ps1 did not load: ' + $problem.Message) $problem.Code }

# The session's key, 16 bytes as 32 hex digits, and its id. No message quotes the key.
$key = $null
$keyId = ''
try {
    $key = ConvertTo-RegdiffKey ([string]$Pc['RegdiffKey'])
    $keyId = Get-RegdiffKeyId $key
} catch {
    $key = $null
}
if ($null -eq $key) { Fail 'internal: the Mac sent no key of 32 hex digits' 1 }

$driver = [bool]$Pc['Driver']
$before = $null
if ($mode -ceq 'after') {
    if (-not $Pc['BeforeB64']) { Fail 'no before snapshot: run pc amd-diff before first' 2 }
    try {
        $before = Read-RegdiffSnapshot ($Utf8NoBom.GetString([Convert]::FromBase64String([string]$Pc['BeforeB64']))) 'the before snapshot'
    } catch {
        $problem = Get-AmdDiffProblem $_ 2
    }
    if ($null -ne $problem) { Fail $problem.Message 2 }
    # Its hashes compare with the new snapshot's only under the same key.
    if ([string]$before['key_id'] -cne $keyId) {
        Fail 'the before snapshot was taken with another key: run pc amd-diff before again' 2
    }
    # The after snapshot reads what the before snapshot read.
    if ($driver -and -not $before['driver']) {
        Fail 'the before snapshot was taken without --driver: take a new one with pc amd-diff before --driver' 2
    }
    $driver = [bool]$before['driver']
}

$snapshot = $null
$summary = $null
$json = $null
try {
    $snapshot = Get-RegdiffSnapshot -OwnerSid (Get-RegdiffOwnerSid) -Driver:$driver -Key $key
    $summary = Format-RegdiffSummary $snapshot
    $json = ConvertTo-RegdiffJson $snapshot
} catch {
    $problem = Get-AmdDiffProblem $_ 1
}
if ($null -ne $problem) { Fail $problem.Message $problem.Code }
foreach ($line in $summary) { Say $line }

if ($null -ne $before) {
    $diff = $null
    try {
        $diff = Compare-RegdiffSnapshot $before $snapshot
    } catch {
        $problem = Get-AmdDiffProblem $_ 2
    }
    if ($null -ne $problem) { Fail $problem.Message $problem.Code }
    foreach ($line in $diff.Lines) { Say $line }
    Send-KV 'diff' ([Convert]::ToBase64String($Utf8NoBom.GetBytes(($diff.Lines -join "`n") + "`n")))
}
Send-KV 'snapshot' ([Convert]::ToBase64String($Utf8NoBom.GetBytes($json)))
Exit-Pc 0
