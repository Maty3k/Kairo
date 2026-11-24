# SPDX-License-Identifier: GPL-3.0-or-later
# pc amd-diff before|after: the AMD settings diff (research 17 section 5.5; docs/milestones/M1.md,
# Formats), two calls per toggle. "before" takes a snapshot of AMD's registry settings and returns
# it as "@@KV snapshot=<Base64 JSON>". The owner toggles one setting in AMD Software. "after" gets
# that snapshot back in the prelude (BeforeB64), takes a new one with the same scopes, prints the
# values that changed, and returns its snapshot and the lines ("@@KV diff=<Base64 text>").
# The functions are tools/pc/bench/regdiff.ps1's: the Mac sends its text in the prelude
# (RegdiffB64), and it is dot-sourced from memory, so its command line stays off and nothing is
# deployed or written to disk. It only reads: every key is opened read-only, and no key, value or
# file is created, changed or deleted (scans.tests.ps1 checks this file for registry writes).
# The owner's keys come from HKEY_USERS\<SID>, with the SID of \Kairo\Run's principal. No line,
# snapshot or error shows a SID, a path, the account's or the PC's name, or another program's.
# A fullscreen window in front refuses even with --force: a game may be running. The owner is at
# the keyboard by design, so each call passes --force with the owner's OK in that session.
# Exit code: 0 printed; 1 a hive couldn't be read; 2 bad input (no before snapshot or a bad one,
# --driver after a before snapshot without it); 3 PC busy; 4 busy state unknown.

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
    # The after snapshot reads what the before snapshot read.
    if ($driver -and -not $before['driver']) {
        Fail 'the before snapshot was taken without --driver: take a new one with pc amd-diff before --driver' 2
    }
    $driver = [bool]$before['driver']
}

$snapshot = $null
try {
    $snapshot = Get-RegdiffSnapshot -OwnerSid (Get-RegdiffOwnerSid) -Driver:$driver
} catch {
    $problem = Get-AmdDiffProblem $_ 1
}
if ($null -ne $problem) { Fail $problem.Message $problem.Code }
foreach ($line in (Format-RegdiffSummary $snapshot)) { Say $line }

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
Send-KV 'snapshot' ([Convert]::ToBase64String($Utf8NoBom.GetBytes((ConvertTo-RegdiffJson $snapshot))))
Exit-Pc 0
