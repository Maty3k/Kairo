# SPDX-License-Identifier: GPL-3.0-or-later
# pc status: is the owner at the PC? Reads C:\kairo\state\watch.json and also shows the \Kairo
# tasks and the current build. It never refuses. Exit code: 0 IDLE, 3 BUSY, 4 UNKNOWN.

$watch = Get-KairoWatch
$tasks = @((Get-KairoTask 'Run'), (Get-KairoTask 'Ensure'), (Get-KairoTask 'Engine'))
$build = Get-CurrentBuildId
$commit = ''
if ($build) { $commit = Get-BuildCommit $build }
# TODO(verify): LogonUI.exe runs while the session is locked or at the sign-in screen (a heuristic).
$logonUi = [bool](Get-Process -Name 'LogonUI' -ErrorAction SilentlyContinue)
$consoleUser = $false
try { $consoleUser = [bool](Get-CimInstance -ClassName Win32_ComputerSystem -ErrorAction Stop).UserName } catch { }

if ($Pc['Json']) {
    $report = [ordered]@{
        state                  = $watch.State
        reason                 = $watch.Reason
        watch_age_s            = $watch.Age
        watch                  = $null
        tasks                  = [ordered]@{}
        current_build          = $build
        current_commit         = $commit
        logonui_running        = $logonUi
        console_user_signed_in = $consoleUser
    }
    if ($watch.Raw) {
        try { $report.watch = Read-JsonText $watch.Raw } catch { }
    }
    foreach ($task in $tasks) { $report.tasks[$task.Name] = $task.State }
    Say ($report | ConvertTo-Json -Compress -Depth 6)
} else {
    # The verdict and the file's age only: the raw file (--json shows it) is for tools, not transcripts.
    if ($null -ne $watch.Age) { Say ('watch:   watch.json written ' + $watch.Age + ' s ago') }
    Say ('verdict: ' + $watch.State.ToUpperInvariant() + ' - ' + $watch.Reason)
    $taskText = @($tasks | ForEach-Object { $_.Name + '=' + $_.State }) -join ', '
    Say ('tasks:   ' + $taskText)
    if ($build) {
        $line = 'build:   current = ' + $build
        if ($commit) { $line += ' (commit ' + $commit + ')' }
        Say $line
    } else {
        Say 'build:   none deployed yet (C:\kairo\bin\current is missing)'
    }
    $session = 'nobody is signed in at the console'
    if ($consoleUser) { $session = 'the owner is signed in at the console' }
    if ($logonUi) {
        $session += '; LogonUI.exe is running, so the screen is probably locked (capture tests need it unlocked)'
    } elseif ($consoleUser) {
        $session += ', screen unlocked (no LogonUI.exe)'
    }
    Say ('session: ' + $session)
}

switch ($watch.State) {
    'idle' { Exit-Pc 0 }
    'busy' { Exit-Pc 3 }
    default { Exit-Pc 4 }
}
