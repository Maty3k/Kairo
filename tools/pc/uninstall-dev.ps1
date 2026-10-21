# SPDX-License-Identifier: GPL-3.0-or-later
#
# tools/pc/uninstall-dev.ps1 - undo tools/pc/bootstrap.ps1 (milestone M0). https://github.com/Maty3k/Kairo
#
# Removes:  the scheduled tasks in the \Kairo\ folder (and the folder), the firewall rule Kairo-SSH-In-TCP, and the
#           Kairo Mac's key ("kairo-mac") from C:\ProgramData\ssh\administrators_authorized_keys.
# Optional: -RemoveData also deletes C:\kairo (with the admin-only Git settings for SSH sessions in
#           C:\kairo\tools\gitconfig), the FFMPEG_DIR system variable that points into it, and the empty
#           admin-only C:\.cargo guard folder.
# Keeps:    every installed tool (Git, Visual Studio Build Tools, LLVM, Rust, Node.js, pnpm, PowerShell 7, the
#           OpenSSH Server feature) and the other settings; the end of the run lists them with how to undo each.
#
# Run in an elevated Windows PowerShell ("Terminal (Admin)"), either from GitHub (only the command Claude sends you in
# your own chat, with a commit that is in main's history; docs/milestones/M0.md):
#   & ([scriptblock]::Create((irm https://raw.githubusercontent.com/Maty3k/Kairo/<COMMIT-SHA>/tools/pc/uninstall-dev.ps1)))
#   & ([scriptblock]::Create((irm https://raw.githubusercontent.com/Maty3k/Kairo/<COMMIT-SHA>/tools/pc/uninstall-dev.ps1))) -RemoveData
# or from the synced copy on the PC:
#   powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\kairo\src\tools\pc\uninstall-dev.ps1 [-RemoveData]
#
# Same editing rules as bootstrap.ps1: Windows PowerShell 5.1 syntax, ASCII only, and never 'exit'.

[CmdletBinding()]
param(
    # Also delete C:\kairo (code snapshots, builds, logs, FFmpeg, PresentMon), FFMPEG_DIR and the C:\.cargo guard.
    [switch] $RemoveData,
    # Skip the "type Y" question (only for runs the owner has agreed to in chat).
    [switch] $Yes
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

# Names shared with bootstrap.ps1 (M0 contract, docs/milestones/M0.md).
$KairoRoot         = 'C:\kairo'
$CurrentBin        = 'C:\kairo\bin\current'
$TaskPath          = '\Kairo\'
$TaskFolderName    = 'Kairo'
$KeyTag            = 'kairo-mac'
$FirewallRuleName  = 'Kairo-SSH-In-TCP'
$SidAdministrators = 'S-1-5-32-544'
$SidSystem         = 'S-1-5-18'
$AdminKeysPath     = Join-Path $env:ProgramData 'ssh\administrators_authorized_keys'
$SshdConfigPath    = Join-Path $env:ProgramData 'ssh\sshd_config'
$CargoGuardDir     = Join-Path $env:SystemDrive '.cargo'

# Shared state (a hashtable, so nested scopes can update it without scope modifiers).
$UninstallState = @{ Problems = $false }

function Write-Line([string] $Text = '') { Write-Host $Text }
function Write-Title([string] $Text) { Write-Host ''; Write-Host ('==> ' + $Text) -ForegroundColor Cyan }
function Write-Note([string] $Text) { Write-Host ('    ' + $Text) }
function Write-Good([string] $Text) { Write-Host ('    OK: ' + $Text) -ForegroundColor Green }
function Write-Problem([string] $Text) { Write-Host ('    PROBLEM: ' + $Text) -ForegroundColor Red }

function Invoke-Native {
    param([Parameter(Mandatory = $true)] [string] $FilePath, [string[]] $ArgumentList = @())
    if (-not (Get-Command -Name $FilePath -ErrorAction SilentlyContinue)) { throw ('Program not found: {0}' -f $FilePath) }
    $saved = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $lines = @(& $FilePath @ArgumentList 2>&1 | ForEach-Object { [string] $_ })
        $code = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $saved
    }
    return New-Object psobject -Property @{ ExitCode = $code; Lines = $lines; Text = (($lines -join "`n").Trim()) }
}

function Test-IsElevated {
    $id = [System.Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object System.Security.Principal.WindowsPrincipal($id)
    return $principal.IsInRole([System.Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Get-KairoKeyLines {
    if (-not (Test-Path -LiteralPath $AdminKeysPath)) { return @() }
    $pattern = '\s' + [regex]::Escape($KeyTag) + '$'
    return @(Get-Content -LiteralPath $AdminKeysPath | ForEach-Object { ([string] $_).Trim() } | Where-Object { $_ -match $pattern })
}

function Invoke-Step([string] $Title, [scriptblock] $StepBody) {
    Write-Title $Title
    try {
        $result = & $StepBody
        $text = ((@($result) | Where-Object { $null -ne $_ } | ForEach-Object { [string] $_ }) -join '; ')
        if ($text) { Write-Good $text } else { Write-Good 'done' }
    } catch {
        Write-Problem $_.Exception.Message
        $UninstallState.Problems = $true
    }
}

function Remove-KairoTasks {
    $tasks = @(Get-ScheduledTask -TaskPath $TaskPath -ErrorAction SilentlyContinue)
    foreach ($t in $tasks) {
        try { Stop-ScheduledTask -TaskPath $TaskPath -TaskName $t.TaskName -ErrorAction SilentlyContinue } catch { }
        Unregister-ScheduledTask -TaskPath $TaskPath -TaskName $t.TaskName -Confirm:$false
    }
    # Remove the now empty \Kairo\ folder (Task Scheduler scripting API: Schedule.Service, TaskFolder.DeleteFolder).
    $folderNote = 'task folder \Kairo\ removed'
    try {
        $service = New-Object -ComObject 'Schedule.Service'
        $service.Connect()
        $service.GetFolder('\').DeleteFolder($TaskFolderName, 0)
    } catch {
        $folderNote = 'task folder \Kairo\ left in place (' + $_.Exception.Message + ')'
    }
    return ('{0} task(s) removed; {1}' -f $tasks.Count, $folderNote)
}

function Stop-KairoProcesses {
    # Programs started from C:\kairo (the watch agent, test windows) keep running after their task is gone.
    $prefix = $KairoRoot + '\'
    $stopped = 0
    foreach ($proc in @(Get-Process -ErrorAction SilentlyContinue)) {
        $path = $null
        try { $path = $proc.Path } catch { $path = $null }
        if ($path -and $path.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
            try { Stop-Process -Id $proc.Id -Force -ErrorAction Stop; $stopped++ } catch { Write-Note ('Could not stop {0}: {1}' -f $path, $_.Exception.Message) }
        }
    }
    return ('{0} program(s) from C:\kairo stopped' -f $stopped)
}

function Remove-KairoFirewallRule {
    if (Get-NetFirewallRule -Name $FirewallRuleName -ErrorAction SilentlyContinue) {
        Remove-NetFirewallRule -Name $FirewallRuleName -Confirm:$false
        return ('firewall rule {0} removed' -f $FirewallRuleName)
    }
    return 'no Kairo firewall rule found'
}

function Remove-KairoKey {
    if (-not (Test-Path -LiteralPath $AdminKeysPath)) { return 'no administrators_authorized_keys file' }
    $pattern = '\s' + [regex]::Escape($KeyTag) + '$'
    $kept = @(Get-Content -LiteralPath $AdminKeysPath | ForEach-Object { ([string] $_).Trim() } | Where-Object { $_ -and $_ -notmatch $pattern })
    $realKeys = @($kept | Where-Object { -not $_.StartsWith('#') })
    if ($realKeys.Count -eq 0) {
        Remove-Item -LiteralPath $AdminKeysPath -Force
        return 'the Kairo key was the only key; the file was removed'
    }
    $encoding = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($AdminKeysPath, (($kept -join "`r`n") + "`r`n"), $encoding)
    # Same permissions as bootstrap.ps1 (PLAN section 3.1): Administrators and SYSTEM only.
    $r = Invoke-Native -FilePath 'icacls.exe' -ArgumentList @($AdminKeysPath, '/inheritance:r', '/grant', ('*{0}:F' -f $SidAdministrators), '/grant', ('*{0}:F' -f $SidSystem))
    if ($r.ExitCode -ne 0) { throw ('icacls failed: {0}' -f $r.Text) }
    return ('the Kairo key was removed; {0} other key(s) kept' -f $realKeys.Count)
}

function Remove-KairoData {
    Set-Location -LiteralPath ($env:SystemDrive + '\')
    $notes = @()
    # Settings that point into C:\kairo.
    $ffmpegDir = [Environment]::GetEnvironmentVariable('FFMPEG_DIR', 'Machine')
    if ($ffmpegDir -and $ffmpegDir.StartsWith($KairoRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
        [Environment]::SetEnvironmentVariable('FFMPEG_DIR', $null, 'Machine')
        $notes += 'FFMPEG_DIR removed'
    }
    # The C:\.cargo guard (bootstrap.ps1, Initialize-CargoGuard), only while it is still empty.
    if (Test-Path -LiteralPath $CargoGuardDir) {
        if (@(Get-ChildItem -LiteralPath $CargoGuardDir -Force -ErrorAction SilentlyContinue).Count -eq 0) {
            Remove-Item -LiteralPath $CargoGuardDir -Force
            $notes += 'the empty C:\.cargo removed'
        } else {
            $notes += 'C:\.cargo kept: it is not empty'
        }
    }
    if (-not (Test-Path -LiteralPath $KairoRoot)) { $notes += 'C:\kairo was already gone'; return ($notes -join '; ') }

    # The bin\current junction first, so deleting never follows it anywhere: RemoveDirectory removes only the link.
    if (Test-Path -LiteralPath $CurrentBin) {
        $item = Get-Item -LiteralPath $CurrentBin -Force
        if (([int] $item.Attributes -band [int] [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
            [System.IO.Directory]::Delete($CurrentBin, $false)
        }
    }
    # rd handles read-only files (Git objects) and, with the \\?\ prefix, paths longer than 260 characters.
    $null = Invoke-Native -FilePath (Join-Path $env:SystemRoot 'System32\cmd.exe') -ArgumentList @('/d', '/c', 'rd', '/s', '/q', ('\\?\' + $KairoRoot))
    if (Test-Path -LiteralPath $KairoRoot) {
        Remove-Item -LiteralPath $KairoRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
    if (Test-Path -LiteralPath $KairoRoot) { throw ('C:\kairo could not be fully deleted (a file may be in use). ' + ($notes -join '; ')) }
    $notes += 'C:\kairo deleted'
    return ($notes -join '; ')
}

function Show-Kept {
    Write-Line ''
    Write-Line 'Kept on purpose (remove them yourself only if you want to):'
    Write-Note '- The installed tools: Git, Visual Studio Build Tools 2026, LLVM, Node.js, PowerShell 7.'
    Write-Note '  Remove them in Settings > Apps > Installed apps. Rust: run "rustup self uninstall".'
    Write-Note '- OpenSSH Server. Without the Kairo key and firewall rule nobody can log in from the network.'
    Write-Note '  To remove it: Settings > System > Optional features > OpenSSH Server > Remove.'
    Write-Note ('- Key-only logins in sshd_config. Your original is {0}.before-kairo' -f $SshdConfigPath)
    Write-Note '  (copy it over sshd_config, then run: Restart-Service sshd).'
    Write-Note '- cmd.exe as the SSH shell: DefaultShell, DefaultShellCommandOption and DefaultShellArguments'
    Write-Note '  under HKLM\SOFTWARE\OpenSSH.'
    Write-Note '- The POWERSHELL_TELEMETRY_OPTOUT system variable (it keeps PowerShell 7''s telemetry off).'
    Write-Note '- Your membership of "Performance Log Users". To leave it:'
    Write-Note '  Remove-LocalGroupMember -SID S-1-5-32-559 -Member $env:USERNAME'
    Write-Note '- The LIBCLANG_PATH system variable (it points to LLVM).'
    Write-Note '- Windows'' own SSH firewall rule stays switched off: turning it back on would open SSH on every network.'
    if (-not $RemoveData) { Write-Note '- C:\kairo itself and the empty C:\.cargo. Run this again with -RemoveData to delete them.' }
}

function Invoke-Uninstall {
    Write-Line ''
    Write-Host 'Undo Kairo''s PC setup (tools/pc/uninstall-dev.ps1) - https://github.com/Maty3k/Kairo' -ForegroundColor Cyan
    if ($env:OS -ne 'Windows_NT') { Write-Problem 'This script only runs on Windows.'; return }
    if ($ExecutionContext.SessionState.LanguageMode -ne 'FullLanguage') { Write-Problem 'PowerShell runs in a restricted mode here, so this cannot run.'; return }
    if (-not (Test-IsElevated)) { Write-Problem 'This window is not running as administrator. Right-click Start, choose "Terminal (Admin)", and try again.'; return }

    $tasks = @(Get-ScheduledTask -TaskPath $TaskPath -ErrorAction SilentlyContinue)
    $rule = Get-NetFirewallRule -Name $FirewallRuleName -ErrorAction SilentlyContinue
    $keys = @(Get-KairoKeyLines)
    Write-Line ''
    Write-Line 'This will:'
    Write-Note ('- stop and remove {0} scheduled task(s) in the "Kairo" folder, and stop programs running from C:\kairo' -f $tasks.Count)
    if ($rule) { Write-Note ('- remove the firewall rule {0} (SSH from your home network)' -f $FirewallRuleName) } else { Write-Note '- (no Kairo firewall rule found)' }
    Write-Note ('- remove {0} Kairo key(s) from administrators_authorized_keys, so the Mac can no longer log in' -f $keys.Count)
    if ($RemoveData) {
        Write-Note '- DELETE C:\kairo with everything in it (code snapshots, builds, logs, FFmpeg, PresentMon),'
        Write-Note '  the FFMPEG_DIR system variable that points into it, and the C:\.cargo guard folder if it is empty'
    }
    Write-Line 'Installed tools stay. Nothing else changes.'
    Write-Line ''
    if (-not $Yes) {
        try {
            $answer = Read-Host 'Type Y and press Enter to continue (anything else stops without changes)'
        } catch {
            Write-Problem 'This window cannot ask questions. Run it in Terminal (Admin), or add -Yes.'
            return
        }
        if (([string] $answer).Trim().ToUpperInvariant() -ne 'Y') { Write-Line 'Stopped. Nothing was changed.'; return }
    }

    Invoke-Step 'Remove the \Kairo\ scheduled tasks' { Remove-KairoTasks }
    Invoke-Step 'Stop programs running from C:\kairo' { Stop-KairoProcesses }
    Invoke-Step 'Remove the Kairo SSH firewall rule' { Remove-KairoFirewallRule }
    Invoke-Step 'Remove the Kairo Mac''s SSH key' { Remove-KairoKey }
    if ($RemoveData) { Invoke-Step 'Delete C:\kairo' { Remove-KairoData } }
    Show-Kept
    Write-Line ''
    if ($UninstallState.Problems) {
        Write-Host 'Finished, with problems (see above). Running this again is safe.' -ForegroundColor Yellow
    } else {
        Write-Host 'Finished.' -ForegroundColor Green
    }
}

try {
    Invoke-Uninstall
} catch {
    Write-Problem ('Stopped unexpectedly: {0}' -f $_.Exception.Message)
}
