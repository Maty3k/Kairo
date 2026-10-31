# SPDX-License-Identifier: GPL-3.0-or-later
#
# tools/pc/doctor.ps1 - health check of the Kairo dev PC (milestone M0). Prints ONE JSON object on stdout.
# Project: https://github.com/Maty3k/Kairo
#
# Run it over SSH with "tools/mac/pc doctor" (PowerShell 7; Windows PowerShell 5.1 works too), after the first sync:
#   & C:\kairo\src\tools\pc\doctor.ps1                   # read-only checks
#   & C:\kairo\src\tools\pc\doctor.ps1 -TestPresentMon   # also proves PresentMon starts with your limited token
#   & C:\kairo\src\tools\pc\doctor.ps1 -Compress         # the same JSON on one line
# With Windows PowerShell 5.1 add -ExecutionPolicy Bypass: powershell.exe -NoProfile -ExecutionPolicy Bypass -File ...
#
# Exit code (when run as a file): 0 = green, 1 = problems (listed in "problems"), 2 = the doctor itself failed.
# Everything is read-only except -TestPresentMon: it registers a temporary task \Kairo\DoctorPresentMon (S4U logon,
# limited token, no window on your desktop), runs PresentMon for 2 seconds (the task itself deletes the CSV, as you),
# deletes the task again, and skips itself while C:\kairo\state\watch.json says you are busy.
#
# Names and checks mirror tools/pc/bootstrap.ps1; keep the two in step. Windows PowerShell 5.1 syntax, ASCII only.

[CmdletBinding()]
param(
    [switch] $TestPresentMon,
    [switch] $Compress
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

# ---- M0 contract names (docs/milestones/M0.md) ------------------------------------------------------------------
$DoctorVersion = 'm1-2026-10-24'
$KairoRoot     = 'C:\kairo'
$SrcDir        = 'C:\kairo\src'
$BinDir        = 'C:\kairo\bin'
$JobsDir       = 'C:\kairo\jobs'
$LogsDir       = 'C:\kairo\logs'
$ToolsDir      = 'C:\kairo\tools'
$StateDir      = 'C:\kairo\state'
$BuildDir      = 'C:\kairo\build'
$CurrentBin    = 'C:\kairo\bin\current'
$DevrunExe     = 'C:\kairo\bin\current\kairo-devrun.exe'
$JobFile       = 'C:\kairo\jobs\next.json'
$TaskPath      = '\Kairo\'

# ---- Shared with bootstrap.ps1 and uninstall-dev.ps1 --------------------------------------------------------------
$KeyTag            = 'kairo-mac'
$FirewallRuleName  = 'Kairo-SSH-In-TCP'
$SshdBlockBegin    = '# BEGIN Kairo (tools/pc/bootstrap.ps1)'
$SidAdministrators = 'S-1-5-32-544'
$SidSystem         = 'S-1-5-18'
$SidUsers          = 'S-1-5-32-545'
$SidPerfLogUsers   = 'S-1-5-32-559'
$GitConfigFile     = 'C:\kairo\tools\gitconfig'
$CargoGuardDir     = Join-Path $env:SystemDrive '.cargo'
$RightsFull        = 2032127   # 0x1F01FF
$RightsModify      = 1245631   # 0x1301BF
$RightsRead        = 1179817   # 0x1200A9
$SshDefaultRule    = 'OpenSSH-Server-In-TCP'
$SshdExe           = Join-Path $env:SystemRoot 'System32\OpenSSH\sshd.exe'
$SshDir            = Join-Path $env:ProgramData 'ssh'
$SshdConfigPath    = Join-Path $SshDir 'sshd_config'
$AdminKeysPath     = Join-Path $SshDir 'administrators_authorized_keys'
$HostKeyPubPath    = Join-Path $SshDir 'ssh_host_ed25519_key.pub'
$SshShellExe       = Join-Path $env:SystemRoot 'System32\cmd.exe'
$SshShellOption    = '/d /c'
$SshShellArguments = '/d'
$PwshExe           = Join-Path $env:ProgramFiles 'PowerShell\7\pwsh.exe'
$VsSqmPolicyKey    = 'HKLM:\SOFTWARE\Policies\Microsoft\VisualStudio\SQM'
$WhoamiExe         = Join-Path $env:SystemRoot 'System32\whoami.exe'
$RustToolchain     = '1.99.1'
$NodeMajor         = 24
$VsCompMsvc        = 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64'
$VsCompSdk         = 'Microsoft.VisualStudio.Component.Windows11SDK.26100'
$SdkVersion        = '10.0.26100.0'
$FfmpegDir         = 'C:\kairo\tools\ffmpeg'
$FfmpegSha256      = '7157177b8a6cb2174c1650ba8c71b363f2c78cba5330f88c4c02cf5b2b880646'
$PresentMonDir     = 'C:\kairo\tools\presentmon'
$PresentMonFile    = 'PresentMon-2.6.0-x64.exe'
$PresentMonSha256  = 'b2a706bc6ad475749e3b7e3409263aa1e6906d45bdcf993f6dbc0f660188f1af'
$SacPolicyOn       = '0283ac0f-fff1-49ae-ada1-8a933130cad6'   # learn.microsoft.com "Inbox App Control policies"
$SacPolicyEval     = '1283ac0f-fff1-49ae-ada1-8a933130cad6'

# What PLAN M0 exit 4 asks FFmpeg to list (names as printed by 'ffmpeg -encoders' and 'ffmpeg -filters').
$WantEncoders = @('h264_amf', 'hevc_amf', 'av1_amf', 'h264_mf', 'aac_mf')
$WantFilters  = @('gfxcapture', 'ddagrab')

# Task Scheduler result codes that mean "still running" and "has not run yet".
$TaskRunning  = 267009   # 0x41301 SCHED_S_TASK_RUNNING
$TaskNeverRan = 267011   # 0x41303 SCHED_S_TASK_HAS_NOT_RUN

$Doc = [ordered]@{
    schema         = 1
    doctor_version = $DoctorVersion
    generated_utc  = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    ok             = $false
    problems       = New-Object System.Collections.ArrayList
    warnings       = New-Object System.Collections.ArrayList
}

# =============================================================================================================
# Helpers
# =============================================================================================================
function Add-Problem([string] $Text) { [void] $Doc.problems.Add($Text) }
function Add-Concern([string] $Text) { [void] $Doc.warnings.Add($Text) }

function Invoke-Native {
    # Runs a program and returns its exit code and output (stdout and stderr) as text.
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

function Get-FirstLine($Result, [string] $Pattern = '.') {
    foreach ($line in @($Result.Lines)) { if ($line -match $Pattern) { return $line.Trim() } }
    return $null
}

function Invoke-Section([string] $SectionName, [scriptblock] $SectionBody) {
    try {
        $Doc[$SectionName] = & $SectionBody
    } catch {
        $message = $_.Exception.Message
        $Doc[$SectionName] = [ordered]@{ error = $message }
        Add-Problem ('{0}: the check itself failed: {1}' -f $SectionName, $message)
    }
}

function Test-IsElevated {
    $id = [System.Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object System.Security.Principal.WindowsPrincipal($id)
    return $principal.IsInRole([System.Security.Principal.WindowsBuiltInRole]::Administrator)
}

function ConvertTo-Sid([string] $Account) {
    if ($Account -match '^S-1-\d+(-\d+)+$') { return $Account }   # Task Scheduler sometimes stores the SID itself
    try {
        return (New-Object System.Security.Principal.NTAccount($Account)).Translate([System.Security.Principal.SecurityIdentifier]).Value
    } catch {
        return $null
    }
}

function Resolve-SidName([string] $Sid) {
    try {
        return (New-Object System.Security.Principal.SecurityIdentifier($Sid)).Translate([System.Security.Principal.NTAccount]).Value
    } catch {
        return $Sid
    }
}

function Get-KeyFingerprint([string] $Base64) {
    try {
        $sha = New-Object System.Security.Cryptography.SHA256Managed
        return 'SHA256:' + [Convert]::ToBase64String($sha.ComputeHash([Convert]::FromBase64String($Base64))).TrimEnd('=')
    } catch {
        return $null
    }
}

function Format-Utc($When) {
    if ($null -eq $When) { return $null }
    $d = [datetime] $When
    if ($d.Year -lt 2000) { return $null }   # Task Scheduler reports 1999-11-30 for "never"
    return $d.ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
}

function Get-GitExe {
    # Git for Windows' machine install, and nothing else: the PATH can hold folders your account writes.
    $machine = Join-Path $env:ProgramFiles 'Git\cmd\git.exe'
    if (Test-Path -LiteralPath $machine) { return $machine }
    return $null
}

function Get-UserHome([string] $Name, [string] $Default) {
    # A tool's folder setting from your user variables, used only when it lies in your profile, where those
    # tools keep their files; otherwise $Default.
    $value = [Environment]::GetEnvironmentVariable($Name, 'User')
    if (-not $value) { $value = [Environment]::GetEnvironmentVariable($Name, 'Machine') }
    if (-not $value) { return $Default }
    try { $full = [System.IO.Path]::GetFullPath($value).TrimEnd('\') + '\' } catch { return $Default }
    $profileDir = [System.IO.Path]::GetFullPath($env:USERPROFILE).TrimEnd('\') + '\'
    if ($full.StartsWith($profileDir, [System.StringComparison]::OrdinalIgnoreCase)) { return $value }
    return $Default
}

function Get-VsInstancePath([string[]] $VswhereArgs, [string] $Property = 'installationPath') {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere)) { return $null }
    $r = Invoke-Native -FilePath $vswhere -ArgumentList ($VswhereArgs + @('-latest', '-property', $Property, '-nologo'))
    if ($r.ExitCode -ne 0) { return $null }
    foreach ($line in $r.Lines) { if ($line.Trim()) { return $line.Trim() } }
    return $null
}

function Test-GroupMemberSam([string] $GroupSid, [string] $MemberSid) {
    # Membership in the local account database (a fresh SSH logon also shows it in its token, see token_has_group).
    try {
        $groupName = (New-Object System.Security.Principal.SecurityIdentifier($GroupSid)).Translate([System.Security.Principal.NTAccount]).Value
        $groupName = $groupName.Substring($groupName.IndexOf('\') + 1)
        $group = [ADSI] ('WinNT://./{0},group' -f $groupName)
        foreach ($m in @($group.psbase.Invoke('Members'))) {
            $bytes = $m.GetType().InvokeMember('objectSid', 'GetProperty', $null, $m, $null)
            if ((New-Object System.Security.Principal.SecurityIdentifier($bytes, 0)).Value -eq $MemberSid) { return $true }
        }
        return $false
    } catch {
        $members = @(Get-LocalGroupMember -SID $GroupSid -ErrorAction Stop)
        foreach ($m in $members) { if ($m.SID -and $m.SID.Value -eq $MemberSid) { return $true } }
        return $false
    }
}

function Get-FileRights($Rule) {
    # Same as bootstrap.ps1: the access mask with generic rights mapped to file rights.
    $m = [int64] ([int] $Rule.FileSystemRights)
    if ($m -lt 0) { $m += 4294967296 }
    if (($m -band 268435456) -ne 0) { $m = $m -bor 2032127 }
    if (($m -band 2147483648) -ne 0) { $m = $m -bor 1179785 }
    if (($m -band 1073741824) -ne 0) { $m = $m -bor 1179926 }
    if (($m -band 536870912) -ne 0) { $m = $m -bor 1179808 }
    return ($m -band 2032127)
}

function Get-FolderAclProblem {
    # '' if $Path is a real folder (not a link) owned by Administrators whose explicit entries are exactly
    # $Explicit (SID -> access mask, each for the folder and everything below it); otherwise the first difference,
    # in words. With -Protected nothing is inherited from the parent; without it the folder inherits and may add
    # only the entries in $Explicit.
    param([string] $Path, [hashtable] $Explicit, [switch] $Protected)
    if (-not (Test-Path -LiteralPath $Path -PathType Container)) { return ('{0} is missing' -f $Path) }
    $item = Get-Item -LiteralPath $Path -Force
    if (([int] $item.Attributes -band [int] [System.IO.FileAttributes]::ReparsePoint) -ne 0) { return ('{0} is a link' -f $Path) }
    $acl = Get-Acl -LiteralPath $Path
    $owner = $acl.GetOwner([System.Security.Principal.SecurityIdentifier]).Value
    if ($owner -ne $SidAdministrators) { return ('{0} is owned by {1}, not Administrators' -f $Path, (Resolve-SidName $owner)) }
    if ([bool] $acl.AreAccessRulesProtected -ne [bool] $Protected) {
        if ($Protected) { return ('{0} inherits permissions from its parent folder' -f $Path) }
        return ('{0} does not inherit the permissions of its parent folder' -f $Path)
    }
    $coverage = @{}
    foreach ($rule in @($acl.GetAccessRules($true, $false, [System.Security.Principal.SecurityIdentifier]))) {
        $sid = $rule.IdentityReference.Value
        $rights = Get-FileRights $rule
        $entry = '{0}: {1} {2} 0x{3:X} ({4}; {5})' -f $Path, $rule.AccessControlType, (Resolve-SidName $sid), $rights, $rule.InheritanceFlags, $rule.PropagationFlags
        if ((-not $Explicit.ContainsKey($sid)) -or ([string] $rule.AccessControlType -ne 'Allow')) { return ('unexpected entry ' + $entry) }
        if ($rights -ne $Explicit[$sid]) { return ('entry {0}, expected 0x{1:X}' -f $entry, $Explicit[$sid]) }
        if (-not $coverage.ContainsKey($sid)) { $coverage[$sid] = @{ Here = $false; Down = $false } }
        $propagation = [int] $rule.PropagationFlags
        if (($propagation -band 2) -eq 0) { $coverage[$sid].Here = $true }                  # not InheritOnly
        $inherit = [int] $rule.InheritanceFlags
        if ((($inherit -band 3) -eq 3) -and (($propagation -band 1) -eq 0)) { $coverage[$sid].Down = $true }
    }
    foreach ($sid in @($Explicit.Keys)) {
        if (-not $coverage.ContainsKey($sid)) { return ('{0}: no entry for {1}' -f $Path, (Resolve-SidName $sid)) }
        if (-not ($coverage[$sid].Here -and $coverage[$sid].Down)) { return ('{0}: the entry for {1} does not cover the folder and everything in it' -f $Path, (Resolve-SidName $sid)) }
    }
    return ''
}

function Test-FolderAcl {
    param([string] $Path, [hashtable] $Explicit, [switch] $Protected)
    return ((Get-FolderAclProblem -Path $Path -Explicit $Explicit -Protected:$Protected) -eq '')
}

function Get-KairoAclProblem([string] $OwnerSid) {
    # '' if C:\kairo has the layout Initialize-KairoTree makes (bootstrap.ps1 and doctor.ps1 check the same);
    # otherwise the first difference.
    $root = @{}
    $root[$SidAdministrators] = $RightsFull
    $root[$SidSystem] = $RightsFull
    $root[$OwnerSid] = $RightsRead
    $problem = Get-FolderAclProblem -Path $KairoRoot -Explicit $root -Protected
    if ($problem) { return $problem }
    foreach ($d in @($SrcDir, $BinDir, $JobsDir, $ToolsDir)) {
        $problem = Get-FolderAclProblem -Path $d -Explicit @{}
        if ($problem) { return $problem }
    }
    $writable = @{}
    $writable[$OwnerSid] = $RightsModify
    # A PC set up before builds moved to your account (M1) gets build with its first pc build
    # (tools/mac/remote/build.ps1), with the rights of logs and state.
    foreach ($d in @($LogsDir, $StateDir, $BuildDir)) {
        if (($d -eq $BuildDir) -and -not (Test-Path -LiteralPath $d)) { continue }
        $problem = Get-FolderAclProblem -Path $d -Explicit $writable
        if ($problem) { return $problem }
    }
    return ''
}

function Test-KairoAcl([string] $OwnerSid) { return ((Get-KairoAclProblem $OwnerSid) -eq '') }

function Test-AdminOnlyFile([string] $Path) {
    # True if only Administrators and SYSTEM (and TrustedInstaller) can change the file: one of them owns it (an
    # owner can always rewrite a file's permissions), and no other entry grants a write right.
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $false }
    $acl = Get-Acl -LiteralPath $Path
    $admins = @($SidAdministrators, $SidSystem, 'S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464')
    $writeBits = 852310   # write data, append, write EA, delete child, write attributes, DELETE, WRITE_DAC, WRITE_OWNER
    foreach ($rule in @($acl.GetAccessRules($true, $true, [System.Security.Principal.SecurityIdentifier]))) {
        if ([string] $rule.AccessControlType -ne 'Allow') { continue }
        $sid = $rule.IdentityReference.Value
        if (($admins -notcontains $sid) -and (((Get-FileRights $rule) -band $writeBits) -ne 0)) { return $false }
    }
    return ($admins -contains $acl.GetOwner([System.Security.Principal.SecurityIdentifier]).Value)
}

function Test-PrivateIPv4([string] $Address) {
    # Same as bootstrap.ps1: 10.0.0.0/8, 172.16.0.0/12 or 192.168.0.0/16.
    $ip = $null
    if (-not [System.Net.IPAddress]::TryParse($Address, [ref] $ip)) { return $false }
    $b = $ip.GetAddressBytes()
    if ($b.Length -ne 4) { return $false }
    return ($b[0] -eq 10) -or (($b[0] -eq 172) -and ($b[1] -ge 16) -and ($b[1] -le 31)) -or (($b[0] -eq 192) -and ($b[1] -eq 168))
}

function Get-SshInterface {
    # Same as bootstrap.ps1: physical adapters on a Private network whose IPv4 addresses are all private.
    $physical = @{}
    foreach ($a in @(Get-NetAdapter -Physical -ErrorAction SilentlyContinue)) { $physical[[int] $a.ifIndex] = $a }
    $list = New-Object System.Collections.ArrayList
    foreach ($p in @(Get-NetConnectionProfile -ErrorAction SilentlyContinue)) {
        $adapter = $physical[[int] $p.InterfaceIndex]
        if (-not $adapter) { continue }
        $ips = @(Get-NetIPAddress -InterfaceIndex $p.InterfaceIndex -AddressFamily IPv4 -ErrorAction SilentlyContinue |
            Where-Object { $_.IPAddress -notlike '169.254.*' } | ForEach-Object { [string] $_.IPAddress })
        $public = @($ips | Where-Object { -not (Test-PrivateIPv4 $_) })
        $qualifies = ([string] $p.NetworkCategory -eq 'Private') -and ($ips.Count -gt 0) -and ($public.Count -eq 0)
        [void] $list.Add([ordered]@{ alias = [string] $adapter.Name; category = [string] $p.NetworkCategory; qualifies = $qualifies })
    }
    return @($list)
}

function Get-HostKeyFingerprint {
    # Same as bootstrap.ps1: what ssh-keygen -lf prints for the PC's ed25519 host key.
    if (-not (Test-Path -LiteralPath $HostKeyPubPath)) { return $null }
    try {
        $parts = @(([string] (Get-Content -LiteralPath $HostKeyPubPath -Raw)).Trim() -split '\s+')
        if ($parts.Count -lt 2 -or $parts[0] -cne 'ssh-ed25519') { return $null }
        $sha = [System.Security.Cryptography.SHA256]::Create()
        return 'SHA256:' + [Convert]::ToBase64String($sha.ComputeHash([Convert]::FromBase64String($parts[1]))).TrimEnd('=')
    } catch {
        return $null
    }
}

function Read-OwnedText([string] $Path) {
    # A file your own programs write (in logs or state), opened once and checked through that same handle: a
    # disk file your account owns, so an elevated doctor reads nothing you couldn't read yourself, whatever
    # link may sit at that name. $null when it isn't one.
    try {
        $stream = [System.IO.File]::Open($Path, 'Open', 'Read', 'ReadWrite, Delete')
    } catch {
        return $null
    }
    try {
        if (-not $stream.CanSeek) { return $null }
        # .NET Framework (Windows PowerShell 5.1) has the method; .NET (PowerShell 7) the extension.
        if ($stream.PSObject.Methods['GetAccessControl']) { $security = $stream.GetAccessControl() } else { $security = [System.IO.FileSystemAclExtensions]::GetAccessControl($stream) }
        if ($security.GetOwner([System.Security.Principal.SecurityIdentifier]).Value -ne $MeSid) { return $null }
        return (New-Object System.IO.StreamReader($stream)).ReadToEnd()
    } catch {
        return $null
    } finally {
        $stream.Dispose()
    }
}

function Read-WatchState {
    # C:\kairo\state\watch.json, written every second by the watch agent (M0 contract).
    $path = Join-Path $StateDir 'watch.json'
    $res = [ordered]@{ present = $false; fresh = $false; age_s = $null; busy = $null; last_input_ms_ago = $null; foreground_fullscreen = $null }
    if (-not (Test-Path -LiteralPath $path)) { return $res }
    $res.present = $true
    $text = Read-OwnedText $path
    if ($null -eq $text) { return $res }
    $j = $text | ConvertFrom-Json
    $ts = $j.ts_utc
    if ($ts -isnot [datetime]) {
        # PowerShell 7 turns ISO dates into [datetime] by itself; 5.1 leaves a string.
        $styles = [System.Globalization.DateTimeStyles]::AdjustToUniversal -bor [System.Globalization.DateTimeStyles]::AssumeUniversal
        $ts = [datetime]::Parse([string] $ts, [System.Globalization.CultureInfo]::InvariantCulture, $styles)
    }
    $res.age_s = [math]::Round(((Get-Date).ToUniversalTime() - $ts.ToUniversalTime()).TotalSeconds, 1)
    $res.fresh = ($res.age_s -lt 10)
    $res.busy = [bool] $j.busy
    $res.last_input_ms_ago = $j.last_input_ms_ago
    $res.foreground_fullscreen = $j.foreground_fullscreen
    return $res
}

function Get-SmartAppControl {
    # Read-only; mirrors bootstrap.ps1.
    $res = [ordered]@{ state = 'unknown'; source = 'none'; citool = $null; registry_value = $null; registry_meaning = $null; defender_value = $null }
    $citool = Join-Path $env:SystemRoot 'System32\CiTool.exe'
    if (Test-Path -LiteralPath $citool) {
        try {
            $r = Invoke-Native -FilePath $citool -ArgumentList @('-lp', '-json')
            if ($r.ExitCode -eq 0) {
                $json = ($r.Lines -join "`n") | ConvertFrom-Json
                $enforced = @($json.Policies | Where-Object { [string] $_.IsEnforced -eq 'True' } |
                    ForEach-Object { ([string] $_.PolicyID).Trim('{', '}', ' ').ToLowerInvariant() })
                if ($enforced -contains $SacPolicyOn) { $res.citool = 'on' }
                elseif ($enforced -contains $SacPolicyEval) { $res.citool = 'evaluation' }
                else { $res.citool = 'off' }
            }
        } catch {
            $res.citool = $null
        }
    }
    try {
        # TODO(verify): meaning of 1 and 2 (sources disagree); compare with the CiTool result on the PC.
        $v = (Get-ItemProperty -LiteralPath 'HKLM:\SYSTEM\CurrentControlSet\Control\CI\Policy' -Name 'VerifiedAndReputablePolicyState' -ErrorAction Stop).VerifiedAndReputablePolicyState
        $res.registry_value = [int] $v
        if ([int] $v -eq 0) { $res.registry_meaning = 'off' }
        elseif ([int] $v -eq 1) { $res.registry_meaning = 'on' }
        elseif ([int] $v -eq 2) { $res.registry_meaning = 'evaluation' }
    } catch {
        $res.registry_value = $null
    }
    try {
        # TODO(verify): undocumented property; read only if present.
        $mp = Get-MpComputerStatus -ErrorAction Stop
        if ($mp.PSObject.Properties['SmartAppControlState']) { $res.defender_value = [string] $mp.SmartAppControlState }
    } catch {
        $res.defender_value = $null
    }
    if ($res.citool) { $res.state = $res.citool; $res.source = 'CiTool policy list' }
    elseif ($res.registry_meaning) { $res.state = $res.registry_meaning; $res.source = 'registry VerifiedAndReputablePolicyState (mapping unverified)' }
    return $res
}

function Test-PresentMonUnelevated {
    # Proves that PresentMon can start its ETW session with your limited token, as PLAN M0 exit 4 asks. An S4U task
    # gets a fresh token from the account database (so it already reflects Performance Log Users) and runs without a
    # window on your desktop. TODO(verify) on the PC: S4U with a Microsoft-account-linked user.
    $res = [ordered]@{ status = 'not-run'; detail = ''; exit_code = $null; output = $null }
    if (-not $IsElevated) {
        $res.status = 'skipped'
        $res.detail = 'Needs an elevated doctor run (a normal SSH session) to create the temporary test task.'
        return $res
    }
    $exe = Join-Path $PresentMonDir $PresentMonFile
    if (-not (Test-Path -LiteralPath $exe)) { $res.status = 'skipped'; $res.detail = 'PresentMon is missing.'; return $res }
    $watch = Read-WatchState
    if ($watch.fresh -and $watch.busy) { $res.status = 'skipped'; $res.detail = 'watch.json says the owner is busy.'; return $res }

    $name = 'DoctorPresentMon'
    $csv = Join-Path $LogsDir 'doctor-presentmon.csv'
    $log = Join-Path $LogsDir 'doctor-presentmon.log'
    # The elevated doctor never writes or deletes in logs, which your programs write: the task, running as you,
    # deletes any CSV an earlier test left, overwrites the log, notes PresentMon's exit code ("pm-exit=N", with
    # delayed expansion) and "csv-ok" if this run wrote a non-empty CSV, and deletes the CSV (it lists every
    # program that drew frames in those 2 seconds; nothing needs it). Paths have no spaces, so no quotes.
    $cmdArgs = ('/d /v:on /c del /q {1} 2>nul & {0} --session_name KairoDoctor --stop_existing_session --timed 2 --terminate_after_timed --no_console_stats --output_file {1} > {2} 2>&1' +
        ' & echo pm-exit=!errorlevel!>> {2} & (if exist {1} for %A in ({1}) do @if %~zA GTR 0 echo csv-ok>> {2}) & del /q {1} 2>nul') -f $exe, $csv, $log
    $action = New-ScheduledTaskAction -Execute (Join-Path $env:SystemRoot 'System32\cmd.exe') -Argument $cmdArgs -WorkingDirectory $LogsDir
    $principal = New-ScheduledTaskPrincipal -UserId $Me.Name -LogonType S4U -RunLevel Limited
    $settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 2) -MultipleInstances IgnoreNew -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
    try {
        $null = Register-ScheduledTask -TaskPath $TaskPath -TaskName $name -Action $action -Principal $principal -Settings $settings -Description 'Kairo doctor: temporary PresentMon test (deleted right after).' -Force
        $started = Get-Date
        Start-ScheduledTask -TaskPath $TaskPath -TaskName $name
        $deadline = $started.AddSeconds(45)
        $info = $null
        do {
            Start-Sleep -Milliseconds 500
            $info = Get-ScheduledTaskInfo -TaskPath $TaskPath -TaskName $name
            $task = Get-ScheduledTask -TaskPath $TaskPath -TaskName $name
            $ran = ($info.LastRunTime -ge $started.AddSeconds(-2))
            $finished = $ran -and ([string] $task.State -ne 'Running') -and ($info.LastTaskResult -ne $TaskRunning) -and ($info.LastTaskResult -ne $TaskNeverRan)
        } while (-not $finished -and (Get-Date) -lt $deadline)
        $text = ''
        if ((Test-Path -LiteralPath $log) -and ((Get-Item -LiteralPath $log).LastWriteTime -ge $started.AddSeconds(-2))) { $text = [string] (Read-OwnedText $log) }
        $lines = @($text -split "`r?`n" | Where-Object { $_.Trim() })
        $csvOk = $lines -contains 'csv-ok'
        # PresentMon's own exit code: the task's result is the last command's.
        $exitLine = @($lines | Where-Object { $_ -match '^pm-exit=(-?\d+)\s*$' }) | Select-Object -Last 1
        if ($exitLine -and ($exitLine -match '^pm-exit=(-?\d+)')) { $res.exit_code = [int] $Matches[1] }
        $res.output = (@($lines | Where-Object { ($_ -ne 'csv-ok') -and ($_ -notmatch '^pm-exit=') }) | Select-Object -First 6) -join ' | '
        if (-not $finished) {
            $res.status = 'failed'
            $res.detail = 'The test task did not finish within 45 seconds.'
        } elseif ($res.exit_code -eq 0 -and $csvOk) {
            $res.status = 'passed'
            $res.detail = 'PresentMon recorded 2 seconds with a limited token.'
        } elseif ($text -match '(?i)access denied|Performance Log Users') {
            $res.status = 'failed'
            $res.detail = 'Access denied: the account is not (yet) in Performance Log Users.'
        } else {
            $res.status = 'failed'
            $res.detail = ('PresentMon exit code {0}.' -f $res.exit_code)
        }
    } catch {
        $res.status = 'error'
        $res.detail = ('Could not run the test task: {0}' -f $_.Exception.Message)
    } finally {
        try { Unregister-ScheduledTask -TaskPath $TaskPath -TaskName $name -Confirm:$false -ErrorAction Stop } catch { }
    }
    return $res
}

# =============================================================================================================
# Checks
# =============================================================================================================
$IsElevated = Test-IsElevated
$Me = [System.Security.Principal.WindowsIdentity]::GetCurrent()
$MeSid = $Me.User.Value

function Invoke-Doctor {
    Invoke-Section 'pc' {
        $cv = Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
        if ([string] $ExecutionContext.SessionState.LanguageMode -ne 'FullLanguage') { Add-Problem ('PowerShell runs in {0} mode' -f $ExecutionContext.SessionState.LanguageMode) }
        if (-not $IsElevated) { Add-Concern 'The doctor is not elevated, so the sshd, key file and Smart App Control checks are partial.' }
        [ordered]@{
            name            = $env:COMPUTERNAME
            user            = $Me.Name
            user_sid        = $MeSid
            elevated        = $IsElevated
            in_ssh          = [bool] ($env:SSH_CONNECTION -or $env:SSH_CLIENT)
            powershell      = ('{0} {1}' -f $PSVersionTable.PSEdition, $PSVersionTable.PSVersion)
            language_mode   = [string] $ExecutionContext.SessionState.LanguageMode
            windows_version = [string] $cv.DisplayVersion
            windows_build   = ('{0}.{1}' -f $cv.CurrentBuild, $cv.UBR)
        }
    }

    Invoke-Section 'toolchain' {
        $t = [ordered]@{}

        $git = Get-GitExe
        $t.git = $null
        if ($git) { $t.git = Get-FirstLine (Invoke-Native -FilePath $git -ArgumentList @('--version')) '^git version' } else { Add-Problem 'Git is missing' }

        # rustup, cargo and rustc live in your profile, which any program running as you can change, so an
        # elevated session never runs them (builds run as you: tools/mac/remote/build.ps1). Their files say
        # which toolchain is there.
        $cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
        $rustupHome = Get-UserHome 'RUSTUP_HOME' (Join-Path $env:USERPROFILE '.rustup')
        $t.rustup = $null; $t.rust_active = $null; $t.rustc = $null; $t.cargo = $null
        if (Test-Path -LiteralPath (Join-Path $cargoBin 'rustup.exe')) {
            $t.rustup = 'installed'
            $settings = Join-Path $rustupHome 'settings.toml'
            # Through a checked handle (it is your file), and only a toolchain's name comes out.
            $text = Read-OwnedText $settings
            if ($text -and ($text -match '(?m)^\s*default_toolchain\s*=\s*"([A-Za-z0-9._-]{1,64})"')) {
                $t.rust_active = $Matches[1]
            }
            $toolchain = Join-Path $rustupHome ('toolchains\{0}-x86_64-pc-windows-msvc\bin' -f $RustToolchain)
            if (Test-Path -LiteralPath (Join-Path $toolchain 'rustc.exe')) { $t.rustc = 'rustc ' + $RustToolchain }
            if (Test-Path -LiteralPath (Join-Path $toolchain 'cargo.exe')) { $t.cargo = 'cargo ' + $RustToolchain }
            if (-not $t.rustc) { Add-Problem ('the Rust {0} toolchain (x86_64-pc-windows-msvc) is not installed' -f $RustToolchain) }
        } else {
            Add-Problem 'rustup is missing'
        }

        $vs = Get-VsInstancePath @('-products', '*', '-requires', $VsCompMsvc, $VsCompSdk)
        $t.msvc = [ordered]@{ install_path = $vs; installation_version = $null; vc_tools_version = $null; cl_exe = $false }
        if ($vs) {
            $t.msvc.installation_version = Get-VsInstancePath @('-products', '*', '-requires', $VsCompMsvc, $VsCompSdk) 'installationVersion'
            $verFile = Join-Path $vs 'VC\Auxiliary\Build\Microsoft.VCToolsVersion.default.txt'
            if (Test-Path -LiteralPath $verFile) {
                $vcVer = ([string] (Get-Content -LiteralPath $verFile -Raw)).Trim()
                $t.msvc.vc_tools_version = $vcVer
                $t.msvc.cl_exe = Test-Path -LiteralPath (Join-Path $vs ('VC\Tools\MSVC\{0}\bin\Hostx64\x64\cl.exe' -f $vcVer))
            }
            if (-not $t.msvc.cl_exe) { Add-Problem 'cl.exe (MSVC x64) was not found in the Build Tools' }
        } else {
            Add-Problem 'Visual Studio Build Tools with the MSVC x64 tools and the Windows 11 SDK are missing'
        }

        $kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\Include'
        $sdks = @()
        if (Test-Path -LiteralPath $kits) { $sdks = @(Get-ChildItem -LiteralPath $kits -Directory | Where-Object { $_.Name -like '10.0.*' } | ForEach-Object { $_.Name }) }
        $t.windows_sdks = $sdks
        if ($sdks -notcontains $SdkVersion) { Add-Problem ('Windows SDK {0} is missing' -f $SdkVersion) }

        $llvmBin = Join-Path $env:ProgramFiles 'LLVM\bin'
        $libclang = Test-Path -LiteralPath (Join-Path $llvmBin 'libclang.dll')
        $t.llvm = [ordered]@{ libclang = $libclang; clang = $null; libclang_path_env = [Environment]::GetEnvironmentVariable('LIBCLANG_PATH', 'Machine') }
        if ($libclang) {
            $t.llvm.clang = Get-FirstLine (Invoke-Native -FilePath (Join-Path $llvmBin 'clang.exe') -ArgumentList @('--version'))
            if ($t.llvm.libclang_path_env -ne $llvmBin) { Add-Problem ('LIBCLANG_PATH is "{0}", expected {1}' -f $t.llvm.libclang_path_env, $llvmBin) }
        } else {
            Add-Problem 'libclang.dll (LLVM) is missing'
        }

        $nodeDir = Join-Path $env:ProgramFiles 'nodejs'
        $node = Join-Path $nodeDir 'node.exe'
        $t.node = $null; $t.corepack = $null; $t.pnpm = $null; $t.node_is_link = $false
        if (Test-Path -LiteralPath $node) {
            # Never run: with nvm, Program Files\nodejs is a link into your profile, which any program
            # running as you can change. node.exe's version resource has the version.
            $nodeItem = Get-Item -LiteralPath $nodeDir -Force
            if (([int] $nodeItem.Attributes -band [int] [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                $t.node_is_link = $true
                Add-Concern ('{0} is a link (nvm): its programs live where your account can change them, so elevated sessions never run them' -f $nodeDir)
            }
            $productVersion = (Get-Item -LiteralPath $node).VersionInfo.ProductVersion
            if ($productVersion) { $t.node = 'v' + $productVersion.TrimStart('v') }
            if (-not ([string] $t.node).StartsWith(('v{0}.' -f $NodeMajor))) { Add-Problem ('Node.js is {0}, expected {1}.x' -f $t.node, $NodeMajor) }
            # Not corepack.cmd: Windows runs a .cmd through cmd.exe without /d, which would run the AutoRun
            # command in your registry elevated. Corepack's package.json has the version.
            # Behind an nvm link it is your file: read only through a checked handle.
            $corepackJson = Join-Path $nodeDir 'node_modules\corepack\package.json'
            if (Test-Path -LiteralPath $corepackJson) {
                try {
                    if ($t.node_is_link) { $json = Read-OwnedText $corepackJson } else { $json = Get-Content -LiteralPath $corepackJson -Raw }
                    $version = [string] ($json | ConvertFrom-Json).version
                    if ($version -match '^\d+\.\d+\.\d+$') { $t.corepack = $version }
                } catch { $t.corepack = $null }
            }
            # pnpm.cmd would run the pnpm corepack keeps in your profile: read the versions in its cache
            # instead (COREPACK_HOME, by default %LOCALAPPDATA%\node\corepack).
            # This session's environment is the machine's (tools/mac/remote/stub.ps1): read your own setting.
            $corepackHome = Get-UserHome 'COREPACK_HOME' (Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'node\corepack')
            $versions = @()
            foreach ($cache in @((Join-Path $corepackHome 'v1\pnpm'), (Join-Path $corepackHome 'pnpm'))) {
                if (Test-Path -LiteralPath $cache) {
                    $versions += @(Get-ChildItem -LiteralPath $cache -Directory | Where-Object { $_.Name -match '^\d+\.\d+\.\d+$' } | ForEach-Object { [version] $_.Name })
                }
            }
            if ($versions.Count -gt 0) { $t.pnpm = [string] ($versions | Sort-Object -Descending | Select-Object -First 1) }
            if (-not $t.pnpm) { Add-Problem 'pnpm is not in corepack''s cache (run pnpm once in your own session)' }
        } else {
            Add-Problem ('Node.js {0} is missing' -f $NodeMajor)
        }

        $t.pwsh = $null
        if (Test-Path -LiteralPath $PwshExe) { $t.pwsh = (Get-Item -LiteralPath $PwshExe).VersionInfo.ProductVersion } else { Add-Problem 'PowerShell 7 (MSI) is missing' }
        $t.pwsh_telemetry_optout = [Environment]::GetEnvironmentVariable('POWERSHELL_TELEMETRY_OPTOUT', 'Machine')
        if ($t.pwsh_telemetry_optout -ne '1') { Add-Concern 'POWERSHELL_TELEMETRY_OPTOUT is not 1 machine-wide, so PowerShell 7 sends startup telemetry' }
        $t.vs_usage_data_optin = $null
        $vsPolicy = Get-ItemProperty -LiteralPath $VsSqmPolicyKey -Name 'OptIn' -ErrorAction SilentlyContinue
        if ($vsPolicy) { $t.vs_usage_data_optin = $vsPolicy.OptIn }
        if ($t.vs_usage_data_optin -ne 0) { Add-Concern 'OptIn is not 0 under HKLM\SOFTWARE\Policies\Microsoft\VisualStudio\SQM, so Visual Studio sends its optional usage data' }

        $t.ffmpeg_dir_env = [Environment]::GetEnvironmentVariable('FFMPEG_DIR', 'Machine')
        if ($t.ffmpeg_dir_env -ne $FfmpegDir) { Add-Problem ('FFMPEG_DIR is "{0}", expected {1}' -f $t.ffmpeg_dir_env, $FfmpegDir) }
        $t
    }

    Invoke-Section 'ffmpeg' {
        $exe = Join-Path $FfmpegDir 'bin\ffmpeg.exe'
        $res = [ordered]@{ path = $exe; present = (Test-Path -LiteralPath $exe); pin_ok = $false; dev_files = $false; version = $null; encoders = [ordered]@{}; filters = [ordered]@{} }
        $marker = Join-Path $FfmpegDir 'KAIRO-PIN.json'
        if (Test-Path -LiteralPath $marker) {
            $pin = Get-Content -LiteralPath $marker -Raw | ConvertFrom-Json
            $res.pin_ok = ([string] $pin.sha256 -eq $FfmpegSha256)
        }
        $res.dev_files = (Test-Path -LiteralPath (Join-Path $FfmpegDir 'include\libavcodec\avcodec.h')) -and (Test-Path -LiteralPath (Join-Path $FfmpegDir 'lib\avcodec.lib'))
        if (-not $res.present) { Add-Problem ('FFmpeg is missing at {0}' -f $exe); return $res }
        if (-not $res.pin_ok) { Add-Problem 'FFmpeg is not the pinned build (KAIRO-PIN.json missing or different)' }
        if (-not $res.dev_files) { Add-Problem 'FFmpeg include\ or lib\ files are missing' }
        $res.version = Get-FirstLine (Invoke-Native -FilePath $exe -ArgumentList @('-hide_banner', '-version')) '^ffmpeg version'
        $enc = Invoke-Native -FilePath $exe -ArgumentList @('-hide_banner', '-encoders')
        foreach ($n in $WantEncoders) {
            $res.encoders[$n] = ($enc.Text -match ('(?m)^\s*[A-Za-z.|]{1,6}\s+' + [regex]::Escape($n) + '\s'))
            if (-not $res.encoders[$n]) { Add-Problem ('FFmpeg lacks the encoder {0}' -f $n) }
        }
        $fil = Invoke-Native -FilePath $exe -ArgumentList @('-hide_banner', '-filters')
        foreach ($n in $WantFilters) {
            $res.filters[$n] = ($fil.Text -match ('(?m)^\s*[A-Za-z.|]{1,6}\s+' + [regex]::Escape($n) + '\s'))
            if (-not $res.filters[$n]) { Add-Problem ('FFmpeg lacks the filter {0}' -f $n) }
        }
        $res
    }

    Invoke-Section 'presentmon' {
        $exe = Join-Path $PresentMonDir $PresentMonFile
        $res = [ordered]@{ path = $exe; present = (Test-Path -LiteralPath $exe); sha256_ok = $false; group_member = $null; token_has_group = $null; unelevated_test = $null }
        if ($res.present) {
            $res.sha256_ok = ((Get-FileHash -Algorithm SHA256 -LiteralPath $exe).Hash.ToLowerInvariant() -eq $PresentMonSha256)
            if (-not $res.sha256_ok) { Add-Problem 'PresentMon does not match its pinned SHA-256' }
        } else {
            Add-Problem 'PresentMon is missing'
        }
        $res.group_member = Test-GroupMemberSam -GroupSid $SidPerfLogUsers -MemberSid $MeSid
        if (-not $res.group_member) { Add-Problem ('{0} is not in Performance Log Users' -f $Me.Name) }
        $groups = Invoke-Native -FilePath $WhoamiExe -ArgumentList @('/groups', '/fo', 'csv', '/nh')
        $res.token_has_group = ($groups.Text -match [regex]::Escape($SidPerfLogUsers))
        if ($res.group_member -and -not $res.token_has_group) { Add-Concern 'This session does not carry Performance Log Users yet; sign out and back in.' }
        if ($TestPresentMon) {
            $res.unelevated_test = Test-PresentMonUnelevated
            if ($res.unelevated_test.status -eq 'failed' -or $res.unelevated_test.status -eq 'error') { Add-Problem ('Unelevated PresentMon test: ' + $res.unelevated_test.detail) }
            elseif ($res.unelevated_test.status -eq 'skipped') { Add-Concern ('Unelevated PresentMon test skipped: ' + $res.unelevated_test.detail) }
        } else {
            $res.unelevated_test = [ordered]@{ status = 'not-run'; detail = 'Run the doctor with -TestPresentMon to prove it (M0 exit 4).' }
        }
        $res
    }

    Invoke-Section 'tasks' {
        $expected = @{ Run = ('run --job ' + $JobFile); Ensure = 'ensure'; Engine = 'engine' }
        $out = [ordered]@{}
        foreach ($name in @('Run', 'Ensure', 'Engine')) {
            $t = Get-ScheduledTask -TaskPath $TaskPath -TaskName $name -ErrorAction SilentlyContinue
            if (-not $t) {
                $out[$name] = [ordered]@{ exists = $false }
                Add-Problem ('Task \Kairo\{0} is missing' -f $name)
                continue
            }
            $info = Get-ScheduledTaskInfo -TaskPath $TaskPath -TaskName $name -ErrorAction SilentlyContinue
            $action = @($t.Actions)[0]
            $mismatches = @()
            $userSid = ConvertTo-Sid ([string] $t.Principal.UserId)
            if ($userSid -ne $MeSid) { $mismatches += ('runs as {0}, not as {1}' -f $t.Principal.UserId, $Me.Name) }
            if ([string] $t.Principal.LogonType -ne 'Interactive') { $mismatches += ('logon type {0}' -f $t.Principal.LogonType) }
            if ([string] $t.Principal.RunLevel -ne 'Limited') { $mismatches += ('run level {0}' -f $t.Principal.RunLevel) }
            if ([int] $t.Settings.Priority -ne 4) { $mismatches += ('priority {0}' -f $t.Settings.Priority) }
            if ([string] $t.Settings.ExecutionTimeLimit -ne 'PT0S') { $mismatches += ('time limit {0}' -f $t.Settings.ExecutionTimeLimit) }
            if ([string] $t.Settings.MultipleInstances -ne 'IgnoreNew') { $mismatches += ('multiple instances {0}' -f $t.Settings.MultipleInstances) }
            if ([string] $action.Execute -ne $DevrunExe) { $mismatches += ('runs {0}' -f $action.Execute) }
            if ([string] $action.Arguments -ne $expected[$name]) { $mismatches += ('arguments "{0}"' -f $action.Arguments) }
            $triggers = @()
            foreach ($tr in @($t.Triggers)) {
                $triggers += [ordered]@{
                    type                = [string] $tr.CimClass.CimClassName
                    enabled             = [bool] $tr.Enabled
                    user                = [string] $tr.UserId
                    start               = [string] $tr.StartBoundary
                    repetition_interval = [string] $tr.Repetition.Interval
                    repetition_duration = [string] $tr.Repetition.Duration
                }
            }
            if ($name -eq 'Ensure') {
                if (@($t.Triggers | Where-Object { $_.CimClass.CimClassName -eq 'MSFT_TaskLogonTrigger' }).Count -lt 1) { $mismatches += 'no logon trigger' }
                $repeat = @($t.Triggers | Where-Object { $_.Repetition -and [string] $_.Repetition.Interval -eq 'PT1M' })
                if ($repeat.Count -lt 1) { $mismatches += 'no 1-minute repetition' }
                foreach ($r in $repeat) { if ([string] $r.Repetition.Duration) { $mismatches += ('repeats only for {0}' -f $r.Repetition.Duration) } }
            }
            $entry = [ordered]@{
                exists               = $true
                ok                   = ($mismatches.Count -eq 0)
                mismatches           = $mismatches
                state                = [string] $t.State
                user                 = [string] $t.Principal.UserId
                logon_type           = [string] $t.Principal.LogonType
                run_level            = [string] $t.Principal.RunLevel
                priority             = $t.Settings.Priority
                execution_time_limit = [string] $t.Settings.ExecutionTimeLimit
                multiple_instances   = [string] $t.Settings.MultipleInstances
                execute              = [string] $action.Execute
                arguments            = [string] $action.Arguments
                working_directory    = [string] $action.WorkingDirectory
                triggers             = $triggers
                last_run_utc         = $null
                last_result          = $null
                next_run_utc         = $null
            }
            if ($info) {
                $entry.last_run_utc = Format-Utc $info.LastRunTime
                $entry.last_result = $info.LastTaskResult
                $entry.next_run_utc = Format-Utc $info.NextRunTime
            }
            if ($mismatches.Count -gt 0) { Add-Problem ('Task \Kairo\{0}: {1}' -f $name, ($mismatches -join '; ')) }
            $out[$name] = $entry
        }
        if (-not (Test-Path -LiteralPath $DevrunExe)) { Add-Concern ('{0} does not exist yet (no deploy), so the tasks cannot start.' -f $DevrunExe) }
        $out
    }

    Invoke-Section 'sshd' {
        $res = [ordered]@{}
        $svc = Get-Service -Name 'sshd' -ErrorAction SilentlyContinue
        $res.service = [ordered]@{ exists = [bool] $svc; status = $null; start_type = $null }
        if ($svc) {
            $res.service.status = [string] $svc.Status
            $res.service.start_type = [string] $svc.StartType
            if ($res.service.status -ne 'Running') { Add-Problem ('sshd is {0}' -f $res.service.status) }
            if ($res.service.start_type -ne 'Automatic') { Add-Problem ('sshd starts {0}, not Automatic' -f $res.service.start_type) }
        } else {
            Add-Problem 'The sshd service is missing'
        }

        # cmd.exe /d /c (bootstrap.ps1, docs/milestones/M0.md): no profile or AutoRun runs in elevated sessions.
        $openSsh = $null
        try { $openSsh = Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\OpenSSH' -ErrorAction Stop } catch { $openSsh = $null }
        $shell = $null; $option = $null; $shellArgs = $null
        if ($openSsh) {
            if ($openSsh.PSObject.Properties['DefaultShell']) { $shell = [string] $openSsh.DefaultShell }
            if ($openSsh.PSObject.Properties['DefaultShellCommandOption']) { $option = [string] $openSsh.DefaultShellCommandOption }
            if ($openSsh.PSObject.Properties['DefaultShellArguments']) { $shellArgs = [string] $openSsh.DefaultShellArguments }
        }
        $res.default_shell = [ordered]@{ value = $shell; command_option = $option; arguments = $shellArgs; exists = ($shell -and (Test-Path -LiteralPath $shell)) }
        if ($shell -ne $SshShellExe) { Add-Problem ('The SSH DefaultShell is "{0}", expected {1}' -f $shell, $SshShellExe) }
        if ($option -cne $SshShellOption) { Add-Problem ('The SSH DefaultShellCommandOption is "{0}", expected "{1}" (no AutoRun)' -f $option, $SshShellOption) }
        if ($shellArgs -cne $SshShellArguments) { Add-Concern ('The SSH DefaultShellArguments is "{0}", expected "{1}" for interactive logins' -f $shellArgs, $SshShellArguments) }
        $res.host_key_fingerprint = Get-HostKeyFingerprint
        if (-not $res.host_key_fingerprint) { Add-Problem ('The SSH host key {0} is missing' -f $HostKeyPubPath) }

        if (-not $IsElevated) {
            $res.note = 'Run the doctor elevated (a normal SSH session) for the config, key file and firewall checks.'
            return $res
        }

        $effective = @{}
        $sshdTest = Invoke-Native -FilePath $SshdExe -ArgumentList @('-T')
        foreach ($line in $sshdTest.Lines) { if ($line -match '^(\S+)\s+(.*)$') { $effective[$Matches[1].ToLowerInvariant()] = $Matches[2].Trim() } }
        $kbd = $effective['kbdinteractiveauthentication']
        if ($null -eq $kbd) { $kbd = $effective['challengeresponseauthentication'] }   # older OpenSSH (bootstrap.ps1)
        $res.effective = [ordered]@{
            exit_code                    = $sshdTest.ExitCode
            passwordauthentication       = $effective['passwordauthentication']
            kbdinteractiveauthentication = $kbd
            pubkeyauthentication         = $effective['pubkeyauthentication']
        }
        if ($sshdTest.ExitCode -ne 0) { Add-Problem ('sshd -T failed: {0}' -f $sshdTest.Text) }
        if ($effective['passwordauthentication'] -ne 'no') { Add-Problem 'sshd allows passwords' }
        if ($effective['pubkeyauthentication'] -ne 'yes') { Add-Problem 'sshd does not allow keys' }
        if ($kbd -ne 'no') { Add-Problem 'sshd allows keyboard-interactive logins' }

        $config = ''
        if (Test-Path -LiteralPath $SshdConfigPath) { $config = [System.IO.File]::ReadAllText($SshdConfigPath) }
        $res.config_has_kairo_block = $config.Contains($SshdBlockBegin)
        $res.config_has_admin_match = ($config -match '(?m)^\s*Match\s+Group\s+administrators\s*$')
        if (-not $res.config_has_kairo_block) { Add-Problem 'sshd_config lacks the Kairo block' }
        if (-not $res.config_has_admin_match) { Add-Problem 'sshd_config lacks "Match Group administrators"' }

        $keys = [ordered]@{ exists = (Test-Path -LiteralPath $AdminKeysPath); kairo_keys = 0; other_keys = 0; fingerprints = @(); acl_protected = $null; acl_sids = @(); owner_sid = $null }
        if ($keys.exists) {
            foreach ($line in @(Get-Content -LiteralPath $AdminKeysPath)) {
                $keyLine = ([string] $line).Trim()
                if (-not $keyLine -or $keyLine.StartsWith('#')) { continue }
                if ($keyLine -match ('\s' + [regex]::Escape($KeyTag) + '$')) {
                    $keys.kairo_keys = $keys.kairo_keys + 1
                    $parts = @($keyLine -split '\s+')
                    if ($parts.Count -ge 2) { $keys.fingerprints += (Get-KeyFingerprint $parts[1]) }
                } else {
                    $keys.other_keys = $keys.other_keys + 1
                }
            }
            $acl = Get-Acl -LiteralPath $AdminKeysPath
            $keys.acl_protected = $acl.AreAccessRulesProtected
            $keys.acl_sids = @($acl.GetAccessRules($true, $true, [System.Security.Principal.SecurityIdentifier]) | ForEach-Object { $_.IdentityReference.Value } | Sort-Object -Unique)
            $keys.owner_sid = $acl.GetOwner([System.Security.Principal.SecurityIdentifier]).Value
            $extra = @($keys.acl_sids | Where-Object { $_ -ne $SidAdministrators -and $_ -ne $SidSystem })
            if (-not $keys.acl_protected -or $extra.Count -gt 0) { Add-Problem 'administrators_authorized_keys allows access beyond Administrators and SYSTEM' }
            if ($keys.kairo_keys -ne 1) { Add-Problem ('administrators_authorized_keys has {0} Kairo keys, expected 1' -f $keys.kairo_keys) }
            if ($keys.other_keys -gt 0) { Add-Concern ('administrators_authorized_keys has {0} other key(s)' -f $keys.other_keys) }
        } else {
            Add-Problem 'administrators_authorized_keys is missing'
        }
        $res.keys_file = $keys

        $fw = [ordered]@{ kairo_rule = $null; qualifying_interfaces = @(); default_rule_enabled = $null; other_open_rules = @() }
        $fw.qualifying_interfaces = @(Get-SshInterface | Where-Object { $_.qualifies } | ForEach-Object { $_.alias })
        $rule = Get-NetFirewallRule -Name $FirewallRuleName -ErrorAction SilentlyContinue
        if ($rule) {
            $addr = $rule | Get-NetFirewallAddressFilter
            $port = $rule | Get-NetFirewallPortFilter
            $app = $rule | Get-NetFirewallApplicationFilter
            $nic = $rule | Get-NetFirewallInterfaceFilter
            $fw.kairo_rule = [ordered]@{
                enabled         = [string] $rule.Enabled
                profile         = [string] $rule.Profile
                direction       = [string] $rule.Direction
                action          = [string] $rule.Action
                protocol        = [string] $port.Protocol
                local_port      = (@($port.LocalPort) -join ',')
                remote_address  = (@($addr.RemoteAddress) -join ',')
                program         = [string] $app.Program
                interface_alias = @($nic.InterfaceAlias | ForEach-Object { [string] $_ })
            }
            $ruleOk = ($fw.kairo_rule.profile -eq 'Private') -and ($fw.kairo_rule.action -eq 'Allow') -and ($fw.kairo_rule.direction -eq 'Inbound')
            if ($ruleOk) { $ruleOk = ($fw.kairo_rule.protocol -eq 'TCP') -and ($fw.kairo_rule.local_port -eq '22') -and ($fw.kairo_rule.remote_address -eq 'LocalSubnet') }
            if ($ruleOk) { $ruleOk = ([Environment]::ExpandEnvironmentVariables($fw.kairo_rule.program) -eq $SshdExe) }
            if (-not $ruleOk) { Add-Problem ('Firewall rule {0} differs from sshd.exe / TCP 22 / Private / LocalSubnet' -f $FirewallRuleName) }
            if ($fw.kairo_rule.enabled -ne 'True') {
                Add-Problem ('Firewall rule {0} is switched off (no wired or Wi-Fi adapter on a Private network with a home-network address when the bootstrap ran); fix the network, then run the bootstrap again' -f $FirewallRuleName)
            } else {
                $aliases = @($fw.kairo_rule.interface_alias)
                if ($aliases.Count -eq 0 -or $aliases -contains 'Any') { Add-Problem ('Firewall rule {0} is not limited to the home-network adapters' -f $FirewallRuleName) }
                $outside = @($aliases | Where-Object { $fw.qualifying_interfaces -notcontains $_ })
                if ($outside.Count -gt 0) { Add-Problem ('Firewall rule {0} opens SSH on adapters that no longer qualify ({1}); run the bootstrap again' -f $FirewallRuleName, ($outside -join ', ')) }
                $missing = @($fw.qualifying_interfaces | Where-Object { $aliases -notcontains $_ })
                if ($missing.Count -gt 0) { Add-Concern ('SSH is not open on {0}; run the bootstrap again if the Mac uses that adapter' -f ($missing -join ', ')) }
            }
        } else {
            Add-Problem ('Firewall rule {0} is missing' -f $FirewallRuleName)
        }
        $default = Get-NetFirewallRule -Name $SshDefaultRule -ErrorAction SilentlyContinue
        if ($default) { $fw.default_rule_enabled = ([string] $default.Enabled -eq 'True') }
        $portFilters = @{}
        foreach ($pf in @(Get-NetFirewallPortFilter -All -ErrorAction SilentlyContinue)) { $portFilters[[string] $pf.InstanceID] = $pf }
        $appFilters = @{}
        foreach ($af in @(Get-NetFirewallApplicationFilter -All -ErrorAction SilentlyContinue)) { $appFilters[[string] $af.InstanceID] = $af }
        foreach ($r in @(Get-NetFirewallRule -Direction Inbound -Enabled True -Action Allow -ErrorAction SilentlyContinue)) {
            if ($r.Name -eq $FirewallRuleName) { continue }
            $pf = $portFilters[[string] $r.InstanceID]
            $af = $appFilters[[string] $r.InstanceID]
            $hit = ($pf -and ([string] $pf.Protocol -eq 'TCP') -and (@($pf.LocalPort) -contains '22')) -or ($af -and ([string] $af.Program -match '(?i)\\sshd\.exe$'))
            if ($hit) { $fw.other_open_rules += [ordered]@{ name = [string] $r.Name; display_name = [string] $r.DisplayName; profile = [string] $r.Profile } }
        }
        if ($fw.other_open_rules.Count -gt 0) { Add-Problem ('Other firewall rules still open SSH: ' + ((@($fw.other_open_rules) | ForEach-Object { $_.name }) -join ', ')) }
        $res.firewall = $fw
        $res
    }

    Invoke-Section 'smart_app_control' {
        $sac = Get-SmartAppControl
        if ($sac.state -eq 'on') { Add-Problem 'Smart App Control is On: it blocks unsigned builds and FFmpeg (SETUP.md step 6, Q3)' }
        elseif ($sac.state -eq 'evaluation') { Add-Concern 'Smart App Control is in Evaluation and may switch itself On (SETUP.md step 6)' }
        elseif ($sac.state -eq 'unknown') { Add-Concern 'Smart App Control state is unknown' }
        $sac
    }

    Invoke-Section 'disk' {
        $d = New-Object System.IO.DriveInfo('C')
        $free = [uint64] $d.AvailableFreeSpace
        if ($free -lt 20GB) { Add-Problem ('Only {0:N1} GB free on C:' -f ($free / 1GB)) }
        elseif ($free -lt 40GB) { Add-Concern ('{0:N1} GB free on C:' -f ($free / 1GB)) }
        [ordered]@{ drive = 'C:'; free_bytes = $free; total_bytes = [uint64] $d.TotalSize; free_gb = [math]::Round($free / 1GB, 1) }
    }

    Invoke-Section 'kairo' {
        $res = [ordered]@{ folders = [ordered]@{}; acl_ok = $null; acl_problem = $null; current = $null; builds = @(); src = $null }
        foreach ($d in @($KairoRoot, $SrcDir, $BinDir, $JobsDir, $LogsDir, $ToolsDir, $StateDir)) {
            $res.folders[$d] = (Test-Path -LiteralPath $d)
            if (-not $res.folders[$d]) { Add-Problem ('{0} is missing' -f $d) }
        }
        $res.acl_problem = Get-KairoAclProblem $MeSid
        $res.acl_ok = ($res.acl_problem -eq '')
        if (-not $res.acl_ok) { Add-Problem ('C:\kairo permissions differ from the M0 layout (Administrators and SYSTEM full control and owner; you read, and write only logs, state and build): {0}. Run the bootstrap again' -f $res.acl_problem) }

        # The admin-only Git settings of the SSH sessions, and files elevated sessions run or trust.
        $res.git_config = [ordered]@{ path = $GitConfigFile; present = (Test-Path -LiteralPath $GitConfigFile); safe_directory = $false; admin_only = $false }
        if ($res.git_config.present) {
            $res.git_config.safe_directory = ([System.IO.File]::ReadAllText($GitConfigFile) -match '(?m)^\s*directory\s*=\s*C:/kairo/src\s*$')
            $res.git_config.admin_only = Test-AdminOnlyFile $GitConfigFile
            if (-not $res.git_config.safe_directory) { Add-Problem ('{0} does not mark C:/kairo/src as safe' -f $GitConfigFile) }
            if (-not $res.git_config.admin_only) { Add-Problem ('{0} can be changed by more than Administrators and SYSTEM' -f $GitConfigFile) }
        } else {
            Add-Problem ('{0} is missing (the bootstrap writes it)' -f $GitConfigFile)
        }
        $ffmpegExe = Join-Path $FfmpegDir 'bin\ffmpeg.exe'
        if ((Test-Path -LiteralPath $ffmpegExe) -and -not (Test-AdminOnlyFile $ffmpegExe)) { Add-Problem ('{0} can be changed by more than Administrators and SYSTEM' -f $ffmpegExe) }

        # cargo reads .cargo\config* from every parent folder of C:\kairo\src (bootstrap.ps1, Initialize-CargoGuard).
        $guard = [ordered]@{ path = $CargoGuardDir; ok = $false; configs = @() }
        $guardRules = @{}
        $guardRules[$SidAdministrators] = $RightsFull
        $guardRules[$SidSystem] = $RightsFull
        $guardRules[$SidUsers] = $RightsRead
        $guard.ok = Test-FolderAcl -Path $CargoGuardDir -Explicit $guardRules -Protected
        foreach ($dir in @($CargoGuardDir, (Join-Path $KairoRoot '.cargo'))) {
            if (Test-Path -LiteralPath $dir) {
                $guard.configs += @(Get-ChildItem -LiteralPath $dir -Force -ErrorAction SilentlyContinue | Where-Object { $_.Name -like 'config*' } | ForEach-Object { $_.FullName })
            }
        }
        if (-not $guard.ok) { Add-Problem 'C:\.cargo is missing or others can change it (the bootstrap makes it admin-only)' }
        if ($guard.configs.Count -gt 0) { Add-Problem ('Cargo config files that builds would read: ' + ($guard.configs -join ', ')) }
        $res.cargo_guard = $guard

        $current = [ordered]@{ exists = (Test-Path -LiteralPath $CurrentBin); junction = $false; target = $null }
        if ($current.exists) {
            $item = Get-Item -LiteralPath $CurrentBin -Force
            $current.junction = (([int] $item.Attributes -band [int] [System.IO.FileAttributes]::ReparsePoint) -ne 0)
            if ($item.PSObject.Properties['Target']) { $current.target = (@($item.Target) -join ';') }
            if (-not $current.junction) { Add-Problem ('{0} is not a junction' -f $CurrentBin) }
        }
        $res.current = $current
        if (Test-Path -LiteralPath $BinDir) {
            $res.builds = @(Get-ChildItem -LiteralPath $BinDir -Directory -Force | Where-Object { $_.Name -ne 'current' } | Sort-Object LastWriteTime -Descending | ForEach-Object { $_.Name })
            if ($res.builds.Count -gt 3) { Add-Concern ('{0} builds in C:\kairo\bin; the contract keeps 3' -f $res.builds.Count) }
        }

        $git = Get-GitExe
        if ($git -and (Test-Path -LiteralPath (Join-Path $SrcDir '.git'))) {
            $branch = (Invoke-Native -FilePath $git -ArgumentList @('-C', $SrcDir, 'symbolic-ref', '-q', 'HEAD')).Text
            $deny = (Invoke-Native -FilePath $git -ArgumentList @('-C', $SrcDir, 'config', '--get', 'receive.denyCurrentBranch')).Text
            $head = Invoke-Native -FilePath $git -ArgumentList @('-C', $SrcDir, 'rev-parse', '--short', 'HEAD')
            $dirty = Invoke-Native -FilePath $git -ArgumentList @('-C', $SrcDir, 'status', '--porcelain', '--untracked-files=no')
            $res.src = [ordered]@{
                head_ref                  = $branch
                head_commit               = $(if ($head.ExitCode -eq 0) { $head.Text } else { $null })
                receive_deny_current      = $deny
                tracked_files_clean       = ($dirty.ExitCode -eq 0 -and -not $dirty.Text)
            }
            if ($branch -ne 'refs/heads/snapshot') { Add-Problem ('C:\kairo\src is on "{0}", not snapshot' -f $branch) }
            if ($deny -ne 'updateInstead') { Add-Problem ('receive.denyCurrentBranch is "{0}", not updateInstead' -f $deny) }
            if ($dirty.ExitCode -eq 0 -and $dirty.Text) { Add-Concern 'C:\kairo\src has modified tracked files, so the next push will be refused' }
        } else {
            Add-Problem 'C:\kairo\src is not a Git repository'
        }
        $res
    }

    Invoke-Section 'watch' {
        $w = Read-WatchState
        if (-not $w.present) { Add-Concern 'watch.json does not exist yet (the watch agent comes with kairo-devrun)' }
        elseif (-not $w.fresh) { Add-Concern ('watch.json is {0} s old; the watch agent may not be running' -f $w.age_s) }
        $w
    }

    Invoke-Section 'network' {
        $profiles = @()
        foreach ($p in @(Get-NetConnectionProfile -ErrorAction SilentlyContinue)) {
            $ips = @(Get-NetIPAddress -InterfaceIndex $p.InterfaceIndex -AddressFamily IPv4 -ErrorAction SilentlyContinue |
                Where-Object { $_.IPAddress -notlike '169.254.*' } | ForEach-Object { [string] $_.IPAddress })
            $profiles += [ordered]@{ interface = [string] $p.InterfaceAlias; category = [string] $p.NetworkCategory; ipv4 = $ips }
        }
        if (@($profiles | Where-Object { $_.category -eq 'Private' }).Count -eq 0) { Add-Problem 'No network is set to Private, so the SSH rule does not apply' }
        [ordered]@{ profiles = $profiles }
    }
}

# Git here reads the admin-only settings of the SSH sessions, never your own .gitconfig (which could make an elevated
# "git status" run a command). A missing file just means no global settings.
$savedGitConfig = $env:GIT_CONFIG_GLOBAL
$env:GIT_CONFIG_GLOBAL = $GitConfigFile
$exitCode = 0
try {
    Invoke-Doctor
    $Doc.ok = ($Doc.problems.Count -eq 0)
    if (-not $Doc.ok) { $exitCode = 1 }
} catch {
    Add-Problem ('The doctor failed: {0}' -f $_.Exception.Message)
    $Doc.ok = $false
    $exitCode = 2
} finally {
    $env:GIT_CONFIG_GLOBAL = $savedGitConfig
}
$Doc.problems = @($Doc.problems)
$Doc.warnings = @($Doc.warnings)
if ($Compress) { ConvertTo-Json -InputObject $Doc -Depth 8 -Compress } else { ConvertTo-Json -InputObject $Doc -Depth 8 }
if ($PSCommandPath) { exit $exitCode }
