# SPDX-License-Identifier: GPL-3.0-or-later
#
# tools/pc/bootstrap.ps1 - Kairo dev-PC setup for milestone M0 (docs/PLAN.md section 4, docs/milestones/M0.md).
# Project and contact: https://github.com/Maty3k/Kairo
#
# Run it on the gaming PC in an elevated Windows PowerShell 5.1 window ("Terminal (Admin)"), with the one-liner
# from SETUP.md. Claude fills in the commit and the Mac's public key in chat; neither is ever committed:
#
#   & ([scriptblock]::Create((irm https://raw.githubusercontent.com/Maty3k/Kairo/<COMMIT-SHA>/tools/pc/bootstrap.ps1))) -MacKey '<public key>'
#
# Claude only hands out a <COMMIT-SHA> that is in main's history with a green CI run (docs/milestones/M0.md):
# raw.githubusercontent.com also serves commits that exist only in a fork of the repository.
#
# It prints what it found, lists everything it will install or change, and waits until you type Y.
# Running it again is safe: every step first checks what is already there. Undo: tools/pc/uninstall-dev.ps1.
#
# Re-register only the three \Kairo\ tasks for the current user (works without admin rights too, e.g. as a
# \Kairo\Run job to prove that your limited token can register \Kairo\Ensure):
#   powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\kairo\src\tools\pc\bootstrap.ps1 -RegisterTasksOnly [-Only Ensure]
# Print the PC facts report as JSON and change nothing (also usable by CI on a Windows runner):
#   powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools\pc\bootstrap.ps1 -ReportOnly
#
# Editing rules: Windows PowerShell 5.1 syntax only (no PowerShell 7 features), ASCII only, and never 'exit'
# (the script runs as a scriptblock inside the owner's own window, which 'exit' would close).

[CmdletBinding(DefaultParameterSetName = 'Full')]
param(
    # The Mac's SSH public key: exactly one 'ssh-ed25519 AAAA... [comment]' line.
    [Parameter(ParameterSetName = 'Full', Mandatory = $true, Position = 0)]
    [string] $MacKey,

    # Skip the "type Y" question. Only for re-runs that the owner has agreed to in chat.
    [Parameter(ParameterSetName = 'Full')]
    [switch] $Yes,

    # Only (re-)register the \Kairo\ scheduled tasks for the user running this script.
    [Parameter(ParameterSetName = 'TasksOnly')]
    [switch] $RegisterTasksOnly,

    [Parameter(ParameterSetName = 'TasksOnly')]
    [ValidateSet('Run', 'Ensure', 'Engine')]
    [string[]] $Only = @('Run', 'Ensure', 'Engine'),

    # Change nothing: print the PC facts report (the bootstrap-report.json content) as JSON. Lets CI on a Windows
    # runner exercise this script, and lets Claude refresh the facts later.
    [Parameter(ParameterSetName = 'ReportOnly')]
    [switch] $ReportOnly
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

# =============================================================================================================
# Names from the M0 contract (docs/milestones/M0.md). Change them there first, then everywhere.
# =============================================================================================================
$BootstrapVersion = 'm0-2026-10-21'
$KairoRoot  = 'C:\kairo'
$SrcDir     = 'C:\kairo\src'
$BinDir     = 'C:\kairo\bin'
$JobsDir    = 'C:\kairo\jobs'
$LogsDir    = 'C:\kairo\logs'
$ToolsDir   = 'C:\kairo\tools'
$StateDir   = 'C:\kairo\state'
$CurrentBin = 'C:\kairo\bin\current'
$DevrunExe  = 'C:\kairo\bin\current\kairo-devrun.exe'
$JobFile    = 'C:\kairo\jobs\next.json'
$TaskPath   = '\Kairo\'
$RepoUrl    = 'https://github.com/Maty3k/Kairo'
$UserAgent  = 'Kairo-bootstrap/1 (+https://github.com/Maty3k/Kairo)'

# Names that only the PC scripts (bootstrap, doctor, uninstall-dev) share, plus the Mac's tools/mac/remote.
$KeyTag            = 'kairo-mac'
$FirewallRuleName  = 'Kairo-SSH-In-TCP'
$SshdBlockBegin    = '# BEGIN Kairo (tools/pc/bootstrap.ps1)'
$SshdBlockEnd      = '# END Kairo'
$SidAdministrators = 'S-1-5-32-544'
$SidSystem         = 'S-1-5-18'
$SidUsers          = 'S-1-5-32-545'
$SidPerfLogUsers   = 'S-1-5-32-559'
# Git settings for Claude's elevated SSH sessions (GIT_CONFIG_GLOBAL in tools/mac/remote), in a folder only
# administrators can change: those sessions never read your own .gitconfig.
$GitConfigFile     = 'C:\kairo\tools\gitconfig'
# cargo reads .cargo\config.toml in every parent folder of C:\kairo\src (doc.rust-lang.org/cargo/reference/config.html),
# and C:\ lets any signed-in user create folders, so an admin-only C:\.cargo keeps such a file away.
$CargoGuardDir     = Join-Path $env:SystemDrive '.cargo'

# File access masks, as icacls writes them: (F), (M), (RX).
$RightsFull   = 2032127   # 0x1F01FF FILE_ALL_ACCESS
$RightsModify = 1245631   # 0x1301BF
$RightsRead   = 1179817   # 0x1200A9 read and execute

# OpenSSH (learn.microsoft.com: "Get started with OpenSSH Server for Windows", "OpenSSH Server configuration";
# github.com/PowerShell/openssh-portable, branch latestw_all: contrib/win32/win32compat/wmain_sshd.c, pwd.c,
# w32-doexec.c).
$SshCapabilityName = 'OpenSSH.Server~~~~0.0.1.0'
$SshDefaultRule    = 'OpenSSH-Server-In-TCP'
$SshdExe           = Join-Path $env:SystemRoot 'System32\OpenSSH\sshd.exe'
$SshdConfigDefault = Join-Path $env:SystemRoot 'System32\OpenSSH\sshd_config_default'
$SshDir            = Join-Path $env:ProgramData 'ssh'
$SshdConfigPath    = Join-Path $SshDir 'sshd_config'
$AdminKeysPath     = Join-Path $SshDir 'administrators_authorized_keys'
$HostKeyPubPath    = Join-Path $SshDir 'ssh_host_ed25519_key.pub'
# The SDDL sshd's first start gives %ProgramData%\ssh (wmain_sshd.c, create_prgdata_ssh_folder).
$SshDirSddl        = 'O:BAD:PAI(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;AU)'
# The SSH shell: sshd runs '"<shell>" <option> "<command>"' as plain text for cmd (w32-doexec.c), and /d skips the
# Command Processor AutoRun commands that any program running as you could set. tools/mac/pc starts PowerShell 7
# itself, with -NoProfile. (A PowerShell DefaultShell would load your profile into every elevated SSH session.)
$SshShellExe       = Join-Path $env:SystemRoot 'System32\cmd.exe'
$SshShellOption    = '/d /c'
$SshShellArguments = '/d'          # interactive logins (DefaultShellArguments, pwd.c)

# =============================================================================================================
# Pins. Each was checked against its primary source on 2026-10-21; re-check before handing out a new one-liner.
# =============================================================================================================

# winget IDs (microsoft/winget-pkgs manifests):
#   Git.Git 2.55.0.5 (has a machine-scope installer); Microsoft.PowerShell 7.6.7.0 (winget installs the MSIX by
#   default since 7.6.0, whose pwsh.exe has no fixed path, so '--installer-type wix' picks the MSI that puts
#   pwsh.exe in C:\Program Files\PowerShell\7, where tools/mac/pc starts it over SSH; 7.7 will ship no MSI,
#   hence the pin); LLVM.LLVM 23.1.4 (MSI);
#   Rustlang.Rustup 1.29.1; OpenJS.NodeJS.LTS (MSI and zip; '--installer-type wix' picks the MSI with corepack);
#   Microsoft.VisualStudio.BuildTools 18.x is "Visual Studio BuildTools 2026" (2022 is the separate ID
#   Microsoft.VisualStudio.2022.BuildTools).
$PwshVersion = '7.6.7.0'
$PwshExe     = Join-Path $env:ProgramFiles 'PowerShell\7\pwsh.exe'

# Visual Studio 2026 Build Tools IDs (learn.microsoft.com, "Visual Studio Build Tools workload and component IDs",
# VS 2026 moniker): the C++ workload plus two of its Recommended components, which --add alone does not install.
$VsWorkload = 'Microsoft.VisualStudio.Workload.VCTools'
$VsCompMsvc = 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64'        # "MSVC Build Tools for x64/x86 (Latest)"
$VsCompSdk  = 'Microsoft.VisualStudio.Component.Windows11SDK.26100'      # "Windows 11 SDK (10.0.26100.6901)"
$VsProductBuildTools = 'Microsoft.VisualStudio.Product.BuildTools'
# --override replaces winget's own switches, so it repeats --wait (bootstrapper waits for the install).
$VsOverride = '--passive --wait --norestart --nocache --add {0} --add {1} --add {2}' -f $VsWorkload, $VsCompMsvc, $VsCompSdk

# Rust (static.rust-lang.org/dist/channel-rust-1.99.1.toml, dated 2026-10-15; PLAN section 2.11 pin).
# With -y rustup-init only warns when it cannot see Visual Studio; it never installs VS Community by itself.
$RustToolchain  = '1.99.1'
$RustupOverride = '-y --default-host x86_64-pc-windows-msvc --default-toolchain {0} --profile minimal --component clippy,rustfmt' -f $RustToolchain

# Node.js 24 LTS. nodejs.org lists v24.22.1 (2026-10-20, LTS "Krypton"); winget-pkgs had OpenJS.NodeJS.LTS only
# up to 24.20.0 on 2026-10-21, and that ID moves to Node 26 when 26 becomes LTS. So the major is pinned and the
# script takes 24.22.1 if winget has it, else the newest 24.x that winget offers.
$NodeMajor     = 24
$NodePreferred = '24.22.1'

# pnpm via corepack (corepack ships with Node.js up to, not including, 25.0.0). 12.14.0 was npm's "latest" on
# 2026-10-21. TODO(verify): keep in step with apps/ui/package.json "packageManager" once that file exists.
$PnpmVersion = '12.14.0'

# FFmpeg: BtbN LGPL shared build of n9.0.2 (PLAN section 2.11: n9.0.2-29-g6dfaba896f). The SHA-256 matches both
# the GitHub release asset digest and BtbN's own checksums.sha256 for release autobuild-2026-10-20-13-08.
# NOTE: BtbN keeps daily builds for 14 days (month-end builds for 2 years), so this URL stops working around
# 2026-11-03. After that, re-pin (for example to the 2026-10-31 month-end build) and update PLAN section 2.11.
$FfmpegAsset  = 'ffmpeg-n9.0.2-29-g6dfaba896f-win64-lgpl-shared-9.0'
$FfmpegUrl    = 'https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-10-20-13-08/ffmpeg-n9.0.2-29-g6dfaba896f-win64-lgpl-shared-9.0.zip'
$FfmpegSha256 = 'c39750c8dfe849e4389843e5f4af51e8062a6d6a3e8b74c1fd7e469594ef68e2'
$FfmpegDir    = 'C:\kairo\tools\ffmpeg'

# PresentMon 2.6.0 console build, official GameTechDev release v2.6.0 (980,320 bytes). The SHA-256 matches the
# GitHub release asset digest and a local hash of the downloaded file.
$PresentMonFile   = 'PresentMon-2.6.0-x64.exe'
$PresentMonUrl    = 'https://github.com/GameTechDev/PresentMon/releases/download/v2.6.0/PresentMon-2.6.0-x64.exe'
$PresentMonSha256 = 'b2a706bc6ad475749e3b7e3409263aa1e6906d45bdcf993f6dbc0f660188f1af'
$PresentMonDir    = 'C:\kairo\tools\presentmon'

# Smart App Control policy IDs (learn.microsoft.com, "Inbox App Control policies"): read with 'CiTool -lp -json'.
$SacPolicyOn   = '0283ac0f-fff1-49ae-ada1-8a933130cad6'   # VerifiedAndReputableDesktop: Smart App Control on
$SacPolicyEval = '1283ac0f-fff1-49ae-ada1-8a933130cad6'   # VerifiedAndReputableDesktopEvaluation: evaluation mode

# winget return codes (winget-cli doc/windows/package-manager/winget/returnCodes.md).
$WingetOkCodes = @(
    0,
    -1978335189,   # 0x8A15002B UPDATE_NOT_APPLICABLE: already installed, nothing newer
    -1978335135,   # 0x8A150061 PACKAGE_ALREADY_INSTALLED
    -1978334963,   # 0x8A15010D INSTALL_ALREADY_INSTALLED
    -1978334962    # 0x8A15010E INSTALL_DOWNGRADE: a newer version is already installed
)
$WingetRebootCodes = @(
    -1978334967,   # 0x8A150109 INSTALL_REBOOT_REQUIRED_TO_FINISH
    -1978334965    # 0x8A15010B INSTALL_REBOOT_INITIATED
)

# Shared state for this run (a hashtable, so nested scopes can update it without scope modifiers).
$Ctx = @{
    Stamp              = (Get-Date -Format 'yyyyMMdd-HHmmss')
    Steps              = New-Object System.Collections.ArrayList
    Notes              = New-Object System.Collections.ArrayList
    Winget             = $null
    RebootNeeded       = $false
    SshdRestartPending = $false
    SshLeftOff         = $false   # a failed SSH step stopped sshd and set it to Manual (fail closed)
    TreeOk             = $false   # C:\kairo has the permissions of the M0 contract; later steps write into it only then
    DisabledRules      = @()
    FirewallInterfaces = @()
    SshdBackup         = $null
    Transcript         = $null
}

# =============================================================================================================
# Small helpers
# =============================================================================================================
function Write-Line([string] $Text = '') { Write-Host $Text }
function Write-Title([string] $Text) { Write-Host ''; Write-Host ('==> ' + $Text) -ForegroundColor Cyan }
function Write-Note([string] $Text) { Write-Host ('    ' + $Text) }
function Write-Good([string] $Text) { Write-Host ('    OK: ' + $Text) -ForegroundColor Green }
function Write-Problem([string] $Text) { Write-Host ('    PROBLEM: ' + $Text) -ForegroundColor Red }
function Write-Attention([string] $Text) {
    Write-Host ('    ATTENTION: ' + $Text) -ForegroundColor Yellow
    [void] $Ctx.Notes.Add($Text)
}

function Write-TextFile([string] $Path, [string] $Text) {
    # UTF-8 without a byte order mark: serde_json and sshd both choke on a BOM.
    $encoding = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($Path, $Text, $encoding)
}

function Invoke-Native {
    # Runs a program, captures stdout and stderr as text, and returns its exit code. Windows PowerShell 5.1 turns
    # redirected stderr lines into errors, so the error preference is relaxed for the duration of the call.
    param(
        [Parameter(Mandatory = $true)] [string] $FilePath,
        [string[]] $ArgumentList = @()
    )
    if (-not (Get-Command -Name $FilePath -ErrorAction SilentlyContinue)) {
        throw ('Program not found: {0}' -f $FilePath)
    }
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

function Update-SessionPath {
    # Picks up PATH changes made by installers, so later steps (and this window) can use the new tools.
    $machine = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $user = [Environment]::GetEnvironmentVariable('Path', 'User')
    $env:Path = (@($machine, $user) | Where-Object { $_ }) -join ';'
}

function Get-Sha256([string] $Path) {
    return (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
}

function Invoke-Download([string] $Url, [string] $OutFile) {
    $curl = Join-Path $env:SystemRoot 'System32\curl.exe'
    if (Test-Path -LiteralPath $curl) {
        $r = Invoke-Native -FilePath $curl -ArgumentList @('--fail', '--location', '--silent', '--show-error',
            '--retry', '3', '--retry-delay', '5', '--connect-timeout', '30',
            '--user-agent', $UserAgent, '--output', $OutFile, $Url)
        if ($r.ExitCode -ne 0) { throw ('Download failed (curl exit code {0}): {1} {2}' -f $r.ExitCode, $Url, $r.Text) }
    } else {
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
        Invoke-WebRequest -Uri $Url -OutFile $OutFile -UseBasicParsing -UserAgent $UserAgent
    }
}

function Get-UtcStamp { return (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ') }

function Test-InSsh { return [bool] ($env:SSH_CONNECTION -or $env:SSH_CLIENT) }

function Test-IsElevated {
    $id = [System.Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object System.Security.Principal.WindowsPrincipal($id)
    return $principal.IsInRole([System.Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Get-OwnerIdentity {
    $id = [System.Security.Principal.WindowsIdentity]::GetCurrent()
    return New-Object psobject -Property @{ Name = $id.Name; Sid = $id.User.Value }
}

function Get-ConsoleUserSid {
    # The user signed in at the PC's screen. If an administrator other than the owner elevated this window, the
    # tasks and the SSH key would go to the wrong account, so the caller compares SIDs.
    try {
        $name = (Get-CimInstance -ClassName Win32_ComputerSystem).UserName
        if (-not $name) { return $null }
        $account = New-Object System.Security.Principal.NTAccount($name)
        return $account.Translate([System.Security.Principal.SecurityIdentifier]).Value
    } catch {
        return $null
    }
}

function ConvertTo-KairoKey([string] $Key) {
    # Accepts exactly one ed25519 public key line; the comment is replaced by the tag "kairo-mac".
    if ([string]::IsNullOrWhiteSpace($Key)) { throw 'The -MacKey value is empty.' }
    $k = $Key.Trim()
    if ($k -match "[`r`n]") { throw 'The -MacKey value must be exactly one line.' }
    $parts = @($k -split '\s+')
    if ($parts[0] -cne 'ssh-ed25519') { throw 'The -MacKey value must start with ssh-ed25519 (one ed25519 public key, no options).' }
    if ($parts.Count -lt 2) { throw 'The -MacKey value has no key data after ssh-ed25519.' }
    try { $blob = [Convert]::FromBase64String($parts[1]) } catch { throw 'The key data in -MacKey is not valid base64.' }
    # An ed25519 public key blob is: uint32 11, "ssh-ed25519", uint32 32, then the 32 key bytes (51 bytes).
    $ok = ($blob.Length -eq 51)
    if ($ok) { $ok = ($blob[0] -eq 0) -and ($blob[1] -eq 0) -and ($blob[2] -eq 0) -and ($blob[3] -eq 11) }
    if ($ok) { $ok = ([System.Text.Encoding]::ASCII.GetString($blob, 4, 11) -ceq 'ssh-ed25519') }
    if ($ok) { $ok = ($blob[15] -eq 0) -and ($blob[16] -eq 0) -and ($blob[17] -eq 0) -and ($blob[18] -eq 32) }
    if (-not $ok) { throw 'The -MacKey value is not a valid ssh-ed25519 public key.' }
    $sha = New-Object System.Security.Cryptography.SHA256Managed
    $fingerprint = 'SHA256:' + [Convert]::ToBase64String($sha.ComputeHash($blob)).TrimEnd('=')
    return New-Object psobject -Property @{
        Line        = ('ssh-ed25519 {0} {1}' -f $parts[1], $KeyTag)
        Blob        = $parts[1]
        Fingerprint = $fingerprint
    }
}

function Invoke-Icacls([string[]] $Arguments) {
    $r = Invoke-Native -FilePath (Join-Path $env:SystemRoot 'System32\icacls.exe') -ArgumentList $Arguments
    if ($r.ExitCode -ne 0) { throw ('icacls {0} failed: {1}' -f ($Arguments -join ' '), $r.Text) }
}

function Resolve-SidName([string] $Sid) {
    try {
        return (New-Object System.Security.Principal.SecurityIdentifier($Sid)).Translate([System.Security.Principal.NTAccount]).Value
    } catch {
        return $Sid
    }
}

function Get-FileRights($Rule) {
    # The access mask of one ACL entry, with generic rights mapped to file rights, so (OI)(CI) entries compare the
    # same whether Windows stores them as one entry or as two.
    $m = [int64] ([int] $Rule.FileSystemRights)
    if ($m -lt 0) { $m += 4294967296 }
    if (($m -band 268435456) -ne 0) { $m = $m -bor 2032127 }     # GENERIC_ALL -> FILE_ALL_ACCESS
    if (($m -band 2147483648) -ne 0) { $m = $m -bor 1179785 }    # GENERIC_READ -> FILE_GENERIC_READ
    if (($m -band 1073741824) -ne 0) { $m = $m -bor 1179926 }    # GENERIC_WRITE -> FILE_GENERIC_WRITE
    if (($m -band 536870912) -ne 0) { $m = $m -bor 1179808 }     # GENERIC_EXECUTE -> FILE_GENERIC_EXECUTE
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

function Find-ReparsePoint([string] $Root) {
    # Links (junctions, symbolic links) anywhere below $Root, without following any of them.
    $found = New-Object System.Collections.ArrayList
    $pending = New-Object System.Collections.Stack
    $pending.Push((New-Object System.IO.DirectoryInfo($Root)))
    while ($pending.Count -gt 0) {
        $dir = $pending.Pop()
        foreach ($entry in @($dir.GetFileSystemInfos())) {
            if (([int] $entry.Attributes -band [int] [System.IO.FileAttributes]::ReparsePoint) -ne 0) { [void] $found.Add($entry.FullName); continue }
            if ($entry -is [System.IO.DirectoryInfo]) { $pending.Push($entry) }
        }
    }
    return @($found)
}

function Test-PrivateIPv4([string] $Address) {
    # 10.0.0.0/8, 172.16.0.0/12 or 192.168.0.0/16 (RFC 1918): a home network, not a public or VPN range.
    $ip = $null
    if (-not [System.Net.IPAddress]::TryParse($Address, [ref] $ip)) { return $false }
    $b = $ip.GetAddressBytes()
    if ($b.Length -ne 4) { return $false }
    return ($b[0] -eq 10) -or (($b[0] -eq 172) -and ($b[1] -ge 16) -and ($b[1] -le 31)) -or (($b[0] -eq 192) -and ($b[1] -eq 168))
}

function Get-SshInterface {
    # The adapters SSH may listen on: physical ones (wired or Wi-Fi, not VPN or virtual adapters) on a Private
    # network whose IPv4 addresses are all private. Each entry says whether it qualifies, and why not.
    $physical = @{}
    foreach ($a in @(Get-NetAdapter -Physical -ErrorAction SilentlyContinue)) { $physical[[int] $a.ifIndex] = $a }
    $list = New-Object System.Collections.ArrayList
    foreach ($p in @(Get-NetConnectionProfile -ErrorAction SilentlyContinue)) {
        $adapter = $physical[[int] $p.InterfaceIndex]
        if (-not $adapter) { continue }
        $ips = @(Get-NetIPAddress -InterfaceIndex $p.InterfaceIndex -AddressFamily IPv4 -ErrorAction SilentlyContinue |
            Where-Object { $_.IPAddress -notlike '169.254.*' } | ForEach-Object { [string] $_.IPAddress })
        $public = @($ips | Where-Object { -not (Test-PrivateIPv4 $_) })
        $why = ''
        if ([string] $p.NetworkCategory -ne 'Private') { $why = 'its network is not Private' }
        elseif ($ips.Count -eq 0) { $why = 'it has no IPv4 address' }
        elseif ($public.Count -gt 0) { $why = ('{0} is not a home-network address' -f ($public -join ', ')) }
        [void] $list.Add([ordered]@{
            alias     = [string] $adapter.Name
            category  = [string] $p.NetworkCategory
            ipv4      = $ips
            qualifies = ($why -eq '')
            reason    = $why
        })
    }
    return @($list)
}

function Get-HostKeyFingerprint {
    # The PC's SSH host key as ssh-keygen -lf prints it (SHA256 of the key blob, Base64 without padding), so the
    # Mac can pin it on the first connection instead of trusting whatever answers at the PC's address.
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

function Get-KairoLogDir {
    # C:\kairo\logs once the C:\kairo step has secured it; until then (or if it failed) your TEMP folder.
    if ($Ctx.TreeOk) { return $LogsDir }
    return $env:TEMP
}

function Assert-KairoTree {
    if (-not $Ctx.TreeOk) { throw 'Skipped: C:\kairo is not set up safely (see the first step), so nothing is put into it.' }
}

function Invoke-Step {
    # Runs one setup step. A failing step is reported and recorded, and the next step still runs, so a single
    # problem never leaves the PC without SSH (Claude can then fix the rest remotely).
    param([Parameter(Mandatory = $true)] [string] $Title, [Parameter(Mandatory = $true)] [scriptblock] $StepBody)
    Write-Title $Title
    $record = [ordered]@{ step = $Title; ok = $false; detail = '' }
    try {
        $result = & $StepBody
        $record.ok = $true
        $record.detail = ((@($result) | Where-Object { $null -ne $_ } | ForEach-Object { [string] $_ }) -join '; ')
        if ($record.detail) { Write-Good $record.detail } else { Write-Good 'done' }
    } catch {
        $record.detail = $_.Exception.Message
        Write-Problem $_.Exception.Message
    }
    [void] $Ctx.Steps.Add($record)
}

# =============================================================================================================
# Detection (read-only)
# =============================================================================================================
function Get-WingetExe {
    $cmd = Get-Command -Name 'winget.exe' -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $alias = Join-Path $env:LOCALAPPDATA 'Microsoft\WindowsApps\winget.exe'
    if (Test-Path -LiteralPath $alias) { return $alias }
    return $null
}

function Get-GitExe {
    $machine = Join-Path $env:ProgramFiles 'Git\cmd\git.exe'
    if (Test-Path -LiteralPath $machine) { return $machine }
    $cmd = Get-Command -Name 'git.exe' -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    return $null
}

function Get-PwshExe {
    if (Test-Path -LiteralPath $PwshExe) { return $PwshExe }
    return $null
}

function Get-VswhereExe {
    $p = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (Test-Path -LiteralPath $p) { return $p }
    return $null
}

function Get-VsInstancePath([string[]] $VswhereArgs) {
    $vswhere = Get-VswhereExe
    if (-not $vswhere) { return $null }
    $r = Invoke-Native -FilePath $vswhere -ArgumentList ($VswhereArgs + @('-latest', '-property', 'installationPath', '-nologo'))
    if ($r.ExitCode -ne 0) { return $null }
    foreach ($line in $r.Lines) {
        $p = $line.Trim()
        if ($p -and (Test-Path -LiteralPath $p)) { return $p }
    }
    return $null
}

function Get-VsCppPath {
    # Any Visual Studio product (Build Tools included) that has the MSVC x64 tools and the Windows 11 SDK.
    return Get-VsInstancePath @('-products', '*', '-requires', $VsCompMsvc, $VsCompSdk)
}

function Get-LlvmBin {
    $bin = Join-Path $env:ProgramFiles 'LLVM\bin'
    if (Test-Path -LiteralPath (Join-Path $bin 'libclang.dll')) { return $bin }
    return $null
}

function Get-RustupExe {
    $p = Join-Path $env:USERPROFILE '.cargo\bin\rustup.exe'
    if (Test-Path -LiteralPath $p) { return $p }
    return $null
}

function Get-NodeExe {
    $p = Join-Path $env:ProgramFiles 'nodejs\node.exe'
    if (Test-Path -LiteralPath $p) { return $p }
    return $null
}

function Get-NodeVersion {
    $node = Get-NodeExe
    if (-not $node) { return $null }
    $r = Invoke-Native -FilePath $node -ArgumentList @('--version')
    if ($r.Text -match '^v(\d+\.\d+\.\d+)') { return $Matches[1] }
    return $null
}

function Get-NodeMajor {
    $v = Get-NodeVersion
    if ($v) { return [int] ($v.Split('.')[0]) }
    return 0
}

function Get-PnpmVersion {
    $pnpm = Join-Path $env:ProgramFiles 'nodejs\pnpm.cmd'
    if (-not (Test-Path -LiteralPath $pnpm)) { return $null }
    $env:COREPACK_ENABLE_DOWNLOAD_PROMPT = '0'
    $r = Invoke-Native -FilePath $pnpm -ArgumentList @('--version')
    if ($r.ExitCode -eq 0 -and $r.Text -match '(\d+\.\d+\.\d+)\s*$') { return $Matches[1] }
    return $null
}

function Test-FfmpegPinned {
    $marker = Join-Path $FfmpegDir 'KAIRO-PIN.json'
    if (-not (Test-Path -LiteralPath $marker)) { return $false }
    if (-not (Test-Path -LiteralPath (Join-Path $FfmpegDir 'bin\ffmpeg.exe'))) { return $false }
    try {
        $pin = Get-Content -LiteralPath $marker -Raw | ConvertFrom-Json
        return ([string] $pin.sha256 -eq $FfmpegSha256)
    } catch {
        return $false
    }
}

function Test-PresentMonPinned {
    $exe = Join-Path $PresentMonDir $PresentMonFile
    if (-not (Test-Path -LiteralPath $exe)) { return $false }
    return ((Get-Sha256 $exe) -eq $PresentMonSha256)
}

function Get-SshCapabilityState {
    try {
        return [string] (Get-WindowsCapability -Online -Name $SshCapabilityName).State
    } catch {
        return 'unknown'
    }
}

function Get-GroupName([string] $Sid) {
    $s = New-Object System.Security.Principal.SecurityIdentifier($Sid)
    $full = $s.Translate([System.Security.Principal.NTAccount]).Value
    return $full.Substring($full.IndexOf('\') + 1)
}

function Test-GroupMember([string] $GroupSid, [string] $MemberSid) {
    # Group membership in the local account database (not this window's token, which predates any change).
    try {
        $members = @(Get-LocalGroupMember -SID $GroupSid -ErrorAction Stop)
        foreach ($m in $members) { if ($m.SID -and $m.SID.Value -eq $MemberSid) { return $true } }
        return $false
    } catch {
        $group = [ADSI] ('WinNT://./{0},group' -f (Get-GroupName $GroupSid))
        foreach ($m in @($group.psbase.Invoke('Members'))) {
            $bytes = $m.GetType().InvokeMember('objectSid', 'GetProperty', $null, $m, $null)
            $sid = (New-Object System.Security.Principal.SecurityIdentifier($bytes, 0)).Value
            if ($sid -eq $MemberSid) { return $true }
        }
        return $false
    }
}

function Get-WindowsFacts {
    $cv = Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
    $os = Get-CimInstance -ClassName Win32_OperatingSystem
    $buildNumber = 0
    [void] [int]::TryParse([string] $cv.CurrentBuild, [ref] $buildNumber)
    $ubr = ''
    if ($null -ne $cv.UBR) { $ubr = '.' + [string] $cv.UBR }
    return [ordered]@{
        caption         = [string] $os.Caption
        edition_id      = [string] $cv.EditionID
        display_version = [string] $cv.DisplayVersion
        build           = ([string] $cv.CurrentBuild) + $ubr
        build_number    = $buildNumber
        architecture    = [string] $os.OSArchitecture
    }
}

function Get-NetworkFacts {
    $profiles = New-Object System.Collections.ArrayList
    $privateIps = New-Object System.Collections.ArrayList
    foreach ($p in @(Get-NetConnectionProfile -ErrorAction SilentlyContinue)) {
        $ips = @(Get-NetIPAddress -InterfaceIndex $p.InterfaceIndex -AddressFamily IPv4 -ErrorAction SilentlyContinue |
            Where-Object { $_.IPAddress -notlike '169.254.*' } | ForEach-Object { [string] $_.IPAddress })
        [void] $profiles.Add([ordered]@{
            interface = [string] $p.InterfaceAlias
            category  = [string] $p.NetworkCategory
            ipv4      = $ips
        })
        if ([string] $p.NetworkCategory -eq 'Private') { foreach ($ip in $ips) { [void] $privateIps.Add($ip) } }
    }
    return [ordered]@{ profiles = @($profiles); private_ipv4 = @($privateIps) }
}

function Get-DiskFacts {
    $d = New-Object System.IO.DriveInfo('C')
    return [ordered]@{ drive = 'C:'; free_bytes = [uint64] $d.AvailableFreeSpace; total_bytes = [uint64] $d.TotalSize }
}

function Get-SmartAppControl {
    # Read-only. Never changes Smart App Control.
    $res = [ordered]@{
        state            = 'unknown'
        source           = 'none'
        citool           = $null
        registry_value   = $null
        registry_meaning = $null
        defender_value   = $null
    }
    # 1) CiTool (Windows 11 22H2+), documented: which App Control policies are enforced right now.
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
    # 2) The registry value named in PLAN section 3.1. Sources disagree on what 1 and 2 mean, so this is only a
    #    fallback. TODO(verify): confirm the mapping on the PC by comparing with the CiTool result above.
    try {
        $v = (Get-ItemProperty -LiteralPath 'HKLM:\SYSTEM\CurrentControlSet\Control\CI\Policy' -Name 'VerifiedAndReputablePolicyState' -ErrorAction Stop).VerifiedAndReputablePolicyState
        $res.registry_value = [int] $v
        if ([int] $v -eq 0) { $res.registry_meaning = 'off' }
        elseif ([int] $v -eq 1) { $res.registry_meaning = 'on' }
        elseif ([int] $v -eq 2) { $res.registry_meaning = 'evaluation' }
    } catch {
        $res.registry_value = $null
    }
    # 3) TODO(verify): Defender's status object may carry a SmartAppControlState property; undocumented, so it is
    #    read only if present and never relied on.
    try {
        $mp = Get-MpComputerStatus -ErrorAction Stop
        if ($mp.PSObject.Properties['SmartAppControlState']) { $res.defender_value = [string] $mp.SmartAppControlState }
    } catch {
        $res.defender_value = $null
    }
    if ($res.citool) { $res.state = $res.citool; $res.source = 'CiTool policy list' }
    elseif ($res.registry_meaning) { $res.state = $res.registry_meaning; $res.source = 'registry VerifiedAndReputablePolicyState (mapping unverified)' }
    return $res
}

function Get-AmdInstantReplay {
    # Read-only. The value names come from third-party code (research 13, unverified), so the state stays
    # 'unknown' unless InstantReplayEnabled exists, and the owner's own report (SETUP step 7) wins.
    # (The dictionary key is "registry", not "values": $dict.Values would return the dictionary's own collection.)
    $found = [ordered]@{}
    foreach ($key in @('HKLM:\SOFTWARE\AMD\DVR', 'HKCU:\Software\AMD\DVR')) {
        if (-not (Test-Path -LiteralPath $key)) { continue }
        $item = Get-ItemProperty -LiteralPath $key -ErrorAction SilentlyContinue
        if (-not $item) { continue }
        foreach ($prop in $item.PSObject.Properties) {
            if ($prop.Name -like 'PS*') { continue }
            if ($prop.Name -match 'Replay|Dvr|Record|Hotkey') { $found[($key + '\' + $prop.Name)] = [string] $prop.Value }
        }
    }
    $res = [ordered]@{ state = 'unknown'; basis = 'No AMD DVR registry values found'; registry = $found }
    # TODO(verify): HKLM\SOFTWARE\AMD\DVR\InstantReplayEnabled (DWORD, 1 = on) per research 13 section 4.2.
    $flag = $found['HKLM:\SOFTWARE\AMD\DVR\InstantReplayEnabled']
    if ($null -ne $flag) {
        if ($flag -eq '1') { $res.state = 'on' } else { $res.state = 'off' }
        $res.basis = 'HKLM\SOFTWARE\AMD\DVR\InstantReplayEnabled (value name unverified)'
    } elseif ($found.Count -gt 0) {
        $res.basis = 'AMD DVR values found, but none that is known to mean Instant Replay'
    }
    return $res
}

function Get-CpuFacts {
    $cpu = @(Get-CimInstance -ClassName Win32_Processor)[0]
    return [ordered]@{
        name               = ([string] $cpu.Name).Trim()
        cores              = [int] $cpu.NumberOfCores
        logical_processors = [int] $cpu.NumberOfLogicalProcessors
        max_clock_mhz      = [int] $cpu.MaxClockSpeed
    }
}

function Get-RamFacts {
    $cs = Get-CimInstance -ClassName Win32_ComputerSystem
    $modules = New-Object System.Collections.ArrayList
    [uint64] $installed = 0
    foreach ($m in @(Get-CimInstance -ClassName Win32_PhysicalMemory -ErrorAction SilentlyContinue)) {
        $installed += [uint64] $m.Capacity
        [void] $modules.Add([ordered]@{
            capacity_bytes = [uint64] $m.Capacity
            speed_mts      = $m.Speed
            configured_mts = $m.ConfiguredClockSpeed
        })
    }
    return [ordered]@{ visible_bytes = [uint64] $cs.TotalPhysicalMemory; installed_bytes = $installed; modules = @($modules) }
}

function Get-GpuFacts {
    # Win32_VideoController for name and driver version; the display-adapter class key (GUID from
    # learn.microsoft.com "System-defined device setup classes") for AMD's software version and the real VRAM size.
    $classKey = 'HKLM:\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}'
    $byName = @{}
    foreach ($sub in @(Get-ChildItem -LiteralPath $classKey -ErrorAction SilentlyContinue)) {
        if ($sub.PSChildName -notmatch '^\d{4}$') { continue }
        $p = Get-ItemProperty -LiteralPath $sub.PSPath -ErrorAction SilentlyContinue
        if ($p -and $p.DriverDesc) { $byName[[string] $p.DriverDesc] = $p }
    }
    $list = New-Object System.Collections.ArrayList
    foreach ($vc in @(Get-CimInstance -ClassName Win32_VideoController)) {
        $entry = [ordered]@{
            name           = [string] $vc.Name
            driver_version = [string] $vc.DriverVersion
            driver_date    = $null
            pnp_device_id  = [string] $vc.PNPDeviceID
            status         = [string] $vc.Status
            vram_bytes     = $null
            amd_software   = $null
        }
        if ($vc.DriverDate) { $entry.driver_date = ([datetime] $vc.DriverDate).ToString('yyyy-MM-dd') }
        $reg = $byName[[string] $vc.Name]
        if ($reg) {
            # TODO(verify): value names used by AMD drivers; reported only when present.
            $mem = $reg.'HardwareInformation.qwMemorySize'
            if ($mem -is [long] -or $mem -is [int] -or $mem -is [uint64]) { $entry.vram_bytes = [uint64] $mem }
            if ($reg.RadeonSoftwareVersion) { $entry.amd_software = [string] $reg.RadeonSoftwareVersion }
        }
        [void] $list.Add($entry)
    }
    return @($list)
}

# QueryDisplayConfig / DisplayConfigGetDeviceInfo (user32.dll). Constants and struct layouts checked against the
# windows-rs 0.62 bindings generated from Microsoft's metadata: QDC_ONLY_ACTIVE_PATHS = 2; device info types
# GET_SOURCE_NAME = 1, GET_TARGET_NAME = 2, GET_ADVANCED_COLOR_INFO = 9, GET_SDR_WHITE_LEVEL = 11; mode info type
# SOURCE = 1; DISPLAYCONFIG_PATH_INFO = 72 bytes, DISPLAYCONFIG_MODE_INFO = 64 bytes, header = 20 bytes.
$DisplayConfigCode = @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;

namespace KairoBootstrap
{
    public class MonitorRecord
    {
        public string FriendlyName;
        public string GdiName;
        public int Width;
        public int Height;
        public double RefreshHz;
        public bool Primary;
        public int OutputTechnology;
        public long AdvancedColorValue = -1;
        public int ColorEncoding = -1;
        public int BitsPerColorChannel = -1;
        public double SdrWhiteNits = -1;
    }

    public static class DisplayConfigReader1
    {
        [DllImport("user32.dll")]
        private static extern int GetDisplayConfigBufferSizes(uint flags, out uint numPathArrayElements, out uint numModeInfoArrayElements);

        [DllImport("user32.dll")]
        private static extern int QueryDisplayConfig(uint flags, ref uint numPathArrayElements, IntPtr pathArray, ref uint numModeInfoArrayElements, IntPtr modeInfoArray, IntPtr currentTopologyId);

        [DllImport("user32.dll")]
        private static extern int DisplayConfigGetDeviceInfo(IntPtr requestPacket);

        private const uint QDC_ONLY_ACTIVE_PATHS = 2;
        private const int PATH_SIZE = 72;
        private const int MODE_SIZE = 64;

        private static IntPtr At(IntPtr basePtr, int offset)
        {
            return new IntPtr(basePtr.ToInt64() + offset);
        }

        private static IntPtr AllocZeroed(int size)
        {
            if (size < 1) { size = 1; }
            IntPtr p = Marshal.AllocHGlobal(size);
            Marshal.Copy(new byte[size], 0, p, size);
            return p;
        }

        private static string ReadWide(IntPtr p, int offset, int maxChars)
        {
            char[] buffer = new char[maxChars];
            Marshal.Copy(At(p, offset), buffer, 0, maxChars);
            int n = Array.IndexOf(buffer, '\0');
            if (n < 0) { n = maxChars; }
            return new string(buffer, 0, n);
        }

        private static IntPtr GetInfo(int type, int size, int adapterLow, int adapterHigh, int id)
        {
            IntPtr p = AllocZeroed(size);
            Marshal.WriteInt32(p, 0, type);
            Marshal.WriteInt32(p, 4, size);
            Marshal.WriteInt32(p, 8, adapterLow);
            Marshal.WriteInt32(p, 12, adapterHigh);
            Marshal.WriteInt32(p, 16, id);
            if (DisplayConfigGetDeviceInfo(p) != 0)
            {
                Marshal.FreeHGlobal(p);
                return IntPtr.Zero;
            }
            return p;
        }

        public static List<MonitorRecord> Query()
        {
            List<MonitorRecord> result = new List<MonitorRecord>();
            IntPtr paths = IntPtr.Zero;
            IntPtr modes = IntPtr.Zero;
            uint numPaths = 0;
            uint numModes = 0;
            try
            {
                int rc = 0;
                for (int attempt = 0; attempt < 5; attempt++)
                {
                    rc = GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, out numPaths, out numModes);
                    if (rc != 0) { throw new System.ComponentModel.Win32Exception(rc); }
                    paths = AllocZeroed((int)numPaths * PATH_SIZE);
                    modes = AllocZeroed((int)numModes * MODE_SIZE);
                    rc = QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, ref numPaths, paths, ref numModes, modes, IntPtr.Zero);
                    if (rc == 0) { break; }
                    Marshal.FreeHGlobal(paths);
                    paths = IntPtr.Zero;
                    Marshal.FreeHGlobal(modes);
                    modes = IntPtr.Zero;
                    if (rc != 122) { throw new System.ComponentModel.Win32Exception(rc); }
                }
                if (rc != 0) { throw new System.ComponentModel.Win32Exception(rc); }

                for (int i = 0; i < (int)numPaths; i++)
                {
                    IntPtr path = At(paths, i * PATH_SIZE);
                    int srcAdapterLow = Marshal.ReadInt32(path, 0);
                    int srcAdapterHigh = Marshal.ReadInt32(path, 4);
                    int srcId = Marshal.ReadInt32(path, 8);
                    uint srcModeIdx = (uint)Marshal.ReadInt32(path, 12);
                    int tgtAdapterLow = Marshal.ReadInt32(path, 20);
                    int tgtAdapterHigh = Marshal.ReadInt32(path, 24);
                    int tgtId = Marshal.ReadInt32(path, 28);

                    MonitorRecord m = new MonitorRecord();
                    m.OutputTechnology = Marshal.ReadInt32(path, 36);
                    uint num = (uint)Marshal.ReadInt32(path, 48);
                    uint den = (uint)Marshal.ReadInt32(path, 52);
                    if (den != 0) { m.RefreshHz = Math.Round((double)num / den, 3); }

                    if (srcModeIdx != 0xFFFFFFFF && srcModeIdx < numModes)
                    {
                        IntPtr mode = At(modes, (int)srcModeIdx * MODE_SIZE);
                        if (Marshal.ReadInt32(mode, 0) == 1)
                        {
                            m.Width = Marshal.ReadInt32(mode, 16);
                            m.Height = Marshal.ReadInt32(mode, 20);
                            int x = Marshal.ReadInt32(mode, 28);
                            int y = Marshal.ReadInt32(mode, 32);
                            m.Primary = (x == 0 && y == 0);
                        }
                    }

                    IntPtr p = GetInfo(2, 420, tgtAdapterLow, tgtAdapterHigh, tgtId);
                    if (p != IntPtr.Zero)
                    {
                        m.FriendlyName = ReadWide(p, 36, 64);
                        Marshal.FreeHGlobal(p);
                    }

                    p = GetInfo(1, 84, srcAdapterLow, srcAdapterHigh, srcId);
                    if (p != IntPtr.Zero)
                    {
                        m.GdiName = ReadWide(p, 20, 32);
                        Marshal.FreeHGlobal(p);
                    }

                    p = GetInfo(9, 32, tgtAdapterLow, tgtAdapterHigh, tgtId);
                    if (p != IntPtr.Zero)
                    {
                        m.AdvancedColorValue = (uint)Marshal.ReadInt32(p, 20);
                        m.ColorEncoding = Marshal.ReadInt32(p, 24);
                        m.BitsPerColorChannel = Marshal.ReadInt32(p, 28);
                        Marshal.FreeHGlobal(p);
                    }

                    p = GetInfo(11, 24, tgtAdapterLow, tgtAdapterHigh, tgtId);
                    if (p != IntPtr.Zero)
                    {
                        uint white = (uint)Marshal.ReadInt32(p, 20);
                        m.SdrWhiteNits = Math.Round(white / 1000.0 * 80.0, 1);
                        Marshal.FreeHGlobal(p);
                    }

                    result.Add(m);
                }
            }
            finally
            {
                if (paths != IntPtr.Zero) { Marshal.FreeHGlobal(paths); }
                if (modes != IntPtr.Zero) { Marshal.FreeHGlobal(modes); }
            }
            return result;
        }
    }
}
'@

function Get-MonitorFacts {
    try {
        $type = 'KairoBootstrap.DisplayConfigReader1' -as [type]
        if (-not $type) {
            # -IgnoreWarnings: Add-Type otherwise treats any compiler warning as an error.
            Add-Type -TypeDefinition $DisplayConfigCode -Language CSharp -IgnoreWarnings
            $type = 'KairoBootstrap.DisplayConfigReader1' -as [type]
        }
        $list = New-Object System.Collections.ArrayList
        foreach ($m in $type::Query()) {
            $entry = [ordered]@{
                name                     = $m.FriendlyName
                gdi_name                 = $m.GdiName
                width                    = $m.Width
                height                   = $m.Height
                refresh_hz               = $m.RefreshHz
                primary                  = $m.Primary
                output_technology        = $m.OutputTechnology
                advanced_color_raw       = $null
                advanced_color_supported = $null
                advanced_color_enabled   = $null
                bits_per_color_channel   = $null
                color_encoding           = $null
                sdr_white_nits           = $null
            }
            if ($m.AdvancedColorValue -ge 0) {
                # TODO(verify): bit 0 = advancedColorSupported, bit 1 = advancedColorEnabled (wingdi.h layout of
                # DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO); "enabled" also covers auto colour management on SDR.
                $entry.advanced_color_raw = $m.AdvancedColorValue
                $entry.advanced_color_supported = (($m.AdvancedColorValue -band 1) -ne 0)
                $entry.advanced_color_enabled = (($m.AdvancedColorValue -band 2) -ne 0)
                $entry.bits_per_color_channel = $m.BitsPerColorChannel
                $entry.color_encoding = $m.ColorEncoding
            }
            if ($m.SdrWhiteNits -ge 0) { $entry.sdr_white_nits = $m.SdrWhiteNits }
            [void] $list.Add($entry)
        }
        return [ordered]@{ source = 'QueryDisplayConfig'; monitors = @($list) }
    } catch {
        # Fallback: names from WMI and the current mode of each adapter's main output.
        $queryError = $_.Exception.Message
        $list = New-Object System.Collections.ArrayList
        foreach ($id in @(Get-CimInstance -Namespace 'root\wmi' -ClassName 'WmiMonitorID' -ErrorAction SilentlyContinue)) {
            $name = (@($id.UserFriendlyName) | Where-Object { $_ -ne 0 } | ForEach-Object { [char] $_ }) -join ''
            [void] $list.Add([ordered]@{ name = $name })
        }
        $modes = @(Get-CimInstance -ClassName Win32_VideoController -ErrorAction SilentlyContinue | ForEach-Object {
            [ordered]@{ adapter = [string] $_.Name; width = $_.CurrentHorizontalResolution; height = $_.CurrentVerticalResolution; refresh_hz = $_.CurrentRefreshRate }
        })
        return [ordered]@{ source = 'WMI fallback'; error = $queryError; monitors = @($list); adapter_modes = $modes }
    }
}

function Get-ToolchainFacts {
    $t = [ordered]@{}
    try { $git = Get-GitExe; if ($git) { $t.git = (Invoke-Native -FilePath $git -ArgumentList @('--version')).Text } } catch { $t.git = 'error' }
    try { $pw = Get-PwshExe; if ($pw) { $t.pwsh = (Get-Item -LiteralPath $pw).VersionInfo.ProductVersion } } catch { $t.pwsh = 'error' }
    try { $t.msvc = Get-VsCppPath } catch { $t.msvc = 'error' }
    try {
        $rustup = Get-RustupExe
        if ($rustup) { $t.rustc = (Invoke-Native -FilePath (Join-Path (Split-Path -Parent $rustup) 'rustc.exe') -ArgumentList @('--version')).Text }
    } catch { $t.rustc = 'error' }
    try { $t.node = Get-NodeVersion } catch { $t.node = 'error' }
    try { $t.pnpm = Get-PnpmVersion } catch { $t.pnpm = 'error' }
    try {
        $llvm = Get-LlvmBin
        if ($llvm) { $t.clang = @((Invoke-Native -FilePath (Join-Path $llvm 'clang.exe') -ArgumentList @('--version')).Lines)[0] }
    } catch { $t.clang = 'error' }
    try {
        $ff = Join-Path $FfmpegDir 'bin\ffmpeg.exe'
        if (Test-Path -LiteralPath $ff) { $t.ffmpeg = @((Invoke-Native -FilePath $ff -ArgumentList @('-hide_banner', '-version')).Lines)[0] }
    } catch { $t.ffmpeg = 'error (blocked by Smart App Control?)' }
    $pm = Join-Path $PresentMonDir $PresentMonFile
    if (Test-Path -LiteralPath $pm) { $t.presentmon = $pm }
    return $t
}

# =============================================================================================================
# Install and configure (each function is idempotent)
# =============================================================================================================
function Install-WingetPackage {
    param(
        [Parameter(Mandatory = $true)] [string] $Id,
        [string] $Version,
        [string[]] $ExtraArgs = @(),
        [string] $Override
    )
    $safeId = $Id -replace '[^A-Za-z0-9.]', '_'
    $log = Join-Path (Get-KairoLogDir) ('winget-{0}-{1}.log' -f $safeId, $Ctx.Stamp)
    $wingetArgs = @('install', '--id', $Id, '--exact', '--source', 'winget',
        '--accept-package-agreements', '--accept-source-agreements', '--silent', '--disable-interactivity', '--log', $log)
    $label = $Id
    if ($Version) { $wingetArgs += @('--version', $Version); $label = $Id + ' ' + $Version }
    if ($ExtraArgs.Count -gt 0) { $wingetArgs += $ExtraArgs }
    if ($Override) { $wingetArgs += @('--override', $Override) }
    Write-Note ('winget install {0}' -f $label)
    $wingetExe = $Ctx.Winget
    # Out-Host shows winget's output (and puts it in the transcript) instead of making it this function's result.
    & $wingetExe @wingetArgs | Out-Host
    $code = $LASTEXITCODE
    Update-SessionPath
    $ok = ($WingetOkCodes -contains $code) -or ($WingetRebootCodes -contains $code)
    if ($WingetRebootCodes -contains $code) { $Ctx.RebootNeeded = $true }
    if (-not $ok) { Write-Note ('winget exit code {0} (0x{1}); its log: {2}' -f $code, ('{0:X8}' -f $code), $log) }
    return New-Object psobject -Property @{ Code = $code; Ok = $ok }
}

function Assert-NoOutsideLink([string] $LeftAlone) {
    # Recursive icacls changes follow links, so they run only while C:\kairo holds none but bin\current, a junction
    # to a build inside C:\kairo\bin.
    $links = @(Find-ReparsePoint $KairoRoot | Where-Object { $_ -ne $CurrentBin })
    if ($links.Count -gt 0) {
        throw ('C:\kairo contains links that may point outside it ({0}), so {1}. Tell Claude.' -f ((@($links) | Select-Object -First 3) -join ', '), $LeftAlone)
    }
    if (Test-Path -LiteralPath $CurrentBin) {
        $item = Get-Item -LiteralPath $CurrentBin -Force
        if (([int] $item.Attributes -band [int] [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
            $target = ([string] (@($item.Target) | Select-Object -First 1)) -replace '^\\\?\?\\', ''
            if (-not $target.StartsWith($BinDir + '\', [System.StringComparison]::OrdinalIgnoreCase)) {
                throw ('C:\kairo\bin\current points outside C:\kairo\bin ({0}), so {1}. Tell Claude.' -f $target, $LeftAlone)
            }
        }
    }
}

function Initialize-KairoTree([string] $OwnerSid) {
    # C:\kairo (docs/milestones/M0.md): Administrators and SYSTEM have full control and own everything; you can
    # read and run it all, but your own programs (kairo-devrun, kairo-hello, the watch agent) write only in logs
    # and state. So nothing running as you can change what Claude's elevated SSH sessions run or trust (builds,
    # scripts, Git settings, FFmpeg). Set-KairoOwnership keeps Administrators the owner of what later steps add.
    $existed = Test-Path -LiteralPath $KairoRoot
    if ($existed) {
        if (Test-KairoAcl $OwnerSid) {
            $Ctx.TreeOk = $true
            return 'C:\kairo is there with the expected permissions'
        }
        # Any signed-in user may create folders in C:\, so a C:\kairo this setup did not make is not trusted.
        $owner = (Get-Acl -LiteralPath $KairoRoot).GetOwner([System.Security.Principal.SecurityIdentifier]).Value
        if (@($SidAdministrators, $SidSystem) -notcontains $owner) {
            throw ('C:\kairo already exists, but this setup did not create it (its owner is {0}). If nothing in it matters, delete it, otherwise move it away; then run the command again.' -f (Resolve-SidName $owner))
        }
        # Resetting permissions below must never follow a link out of C:\kairo.
        Assert-NoOutsideLink 'its permissions were left alone'
    } else {
        # Created with its final permissions in one call: inheriting from C:\ would give every signed-in user
        # Modify on it until icacls ran.
        $sddl = 'D:PAI(A;OICI;FA;;;BA)(A;OICI;FA;;;SY)(A;OICI;0x1200a9;;;{0})' -f $OwnerSid
        $security = New-Object System.Security.AccessControl.DirectorySecurity
        $security.SetSecurityDescriptorSddlForm($sddl)
        $null = [System.IO.Directory]::CreateDirectory($KairoRoot, $security)
        # CreateDirectory quietly returns a folder someone else made a moment earlier: make sure it is ours.
        $newOwner = (Get-Acl -LiteralPath $KairoRoot).GetOwner([System.Security.Principal.SecurityIdentifier]).Value
        if ((@($SidAdministrators, $SidSystem, $OwnerSid) -notcontains $newOwner) -or (@(Get-ChildItem -LiteralPath $KairoRoot -Force).Count -gt 0)) {
            throw 'C:\kairo appeared while this setup was creating it, so it is not trusted. Delete it, then run the command again.'
        }
    }
    Invoke-Icacls @($KairoRoot, '/inheritance:r',
        '/grant:r', ('*{0}:(OI)(CI)F' -f $SidAdministrators), '/grant:r', ('*{0}:(OI)(CI)F' -f $SidSystem),
        '/grant:r', ('*{0}:(OI)(CI)RX' -f $OwnerSid), '/C', '/Q')
    $allowed = @($SidAdministrators, $SidSystem, $OwnerSid)
    $acl = Get-Acl -LiteralPath $KairoRoot
    foreach ($rule in @($acl.GetAccessRules($true, $false, [System.Security.Principal.SecurityIdentifier]))) {
        $sid = $rule.IdentityReference.Value
        if ($allowed -notcontains $sid) { Invoke-Icacls @($KairoRoot, '/remove', ('*{0}' -f $sid), '/C', '/Q') }
    }
    foreach ($d in @($SrcDir, $BinDir, $JobsDir, $LogsDir, $ToolsDir, $StateDir)) {
        if (-not (Test-Path -LiteralPath $d)) { $null = New-Item -ItemType Directory -Path $d }
    }
    # Everything below the root: inherited permissions only (a repair; a new tree has nothing else anyway).
    if ($existed) { Invoke-Icacls @(($KairoRoot + '\*'), '/reset', '/T', '/C', '/Q') }
    Invoke-Icacls @($KairoRoot, '/setowner', ('*{0}' -f $SidAdministrators), '/T', '/C', '/Q')
    foreach ($d in @($LogsDir, $StateDir)) { Invoke-Icacls @($d, '/grant', ('*{0}:(OI)(CI)M' -f $OwnerSid), '/C', '/Q') }
    $problem = Get-KairoAclProblem $OwnerSid
    if ($problem) { throw ('C:\kairo permissions are not as expected after icacls: ' + $problem) }
    $Ctx.TreeOk = $true
    if ($existed) { return 'C:\kairo repaired: Administrators and SYSTEM full control; you read everything and write logs and state' }
    return 'created C:\kairo: Administrators and SYSTEM full control; you read everything and write logs and state'
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
    foreach ($d in @($LogsDir, $StateDir)) {
        $problem = Get-FolderAclProblem -Path $d -Explicit $writable
        if ($problem) { return $problem }
    }
    return ''
}

function Test-KairoAcl([string] $OwnerSid) { return ((Get-KairoAclProblem $OwnerSid) -eq '') }

function Set-KairoOwnership([string] $OwnerSid) {
    # Administrators own everything in C:\kairo but logs and state, and nothing in src, bin, jobs or tools has
    # permissions of its own (docs/milestones/M0.md). Depending on a Windows policy, files an elevated window creates
    # belong to Administrators or to your account, and an owner can always rewrite a file's permissions; so this runs
    # after the steps that add files (Git settings, the code folder, FFmpeg, PresentMon): first the owner, then a reset
    # that removes anything an owner may have added in the meantime.
    Assert-NoOutsideLink 'the owner was left as it is'
    Invoke-Icacls @($KairoRoot, '/setowner', ('*{0}' -f $SidAdministrators), '/C', '/Q')
    foreach ($d in @($SrcDir, $BinDir, $JobsDir, $ToolsDir)) {
        Invoke-Icacls @($d, '/setowner', ('*{0}' -f $SidAdministrators), '/T', '/C', '/Q')
        Invoke-Icacls @($d, '/reset', '/T', '/C', '/Q')
    }
    $problem = Get-KairoAclProblem $OwnerSid
    if ($problem) { throw ('C:\kairo permissions are not as expected: ' + $problem) }
    # icacls fails on any file it could not change; these spot checks cover what the doctor checks too.
    foreach ($p in @($KairoRoot, $SrcDir, $BinDir, $JobsDir, $ToolsDir, $GitConfigFile)) {
        if (-not (Test-Path -LiteralPath $p)) { continue }
        $owner = (Get-Acl -LiteralPath $p).GetOwner([System.Security.Principal.SecurityIdentifier]).Value
        if ($owner -ne $SidAdministrators) { throw ('{0} is still owned by {1}, not Administrators.' -f $p, (Resolve-SidName $owner)) }
    }
    return 'Administrators own C:\kairo and everything in src, bin, jobs and tools, which have no permissions of their own'
}

function Install-Git {
    if (Get-GitExe) { return ('already installed: ' + (Invoke-Native -FilePath (Get-GitExe) -ArgumentList @('--version')).Text) }
    $res = Install-WingetPackage -Id 'Git.Git' -ExtraArgs @('--scope', 'machine')
    $git = Get-GitExe
    if (-not $git) { throw ('Git is still missing after winget (exit code {0}).' -f $res.Code) }
    return (Invoke-Native -FilePath $git -ArgumentList @('--version')).Text
}

function Set-PwshTelemetryOptOut {
    # PowerShell 7 sends startup telemetry unless POWERSHELL_TELEMETRY_OPTOUT is set before pwsh starts
    # (learn.microsoft.com, about_Telemetry). tools/mac/pc starts pwsh on every verb, so set it machine-wide now.
    [Environment]::SetEnvironmentVariable('POWERSHELL_TELEMETRY_OPTOUT', '1', 'Machine')
    $env:POWERSHELL_TELEMETRY_OPTOUT = '1'
}

function Install-Pwsh {
    Set-PwshTelemetryOptOut
    if (Get-PwshExe) { return ('already installed: PowerShell ' + (Get-Item -LiteralPath $PwshExe).VersionInfo.ProductVersion + '; telemetry off') }
    $res = Install-WingetPackage -Id 'Microsoft.PowerShell' -Version $PwshVersion -ExtraArgs @('--installer-type', 'wix', '--scope', 'machine')
    if (-not (Get-PwshExe)) {
        Write-Note ('PowerShell {0} was not available from winget; trying the newest version that has an MSI.' -f $PwshVersion)
        $res = Install-WingetPackage -Id 'Microsoft.PowerShell' -ExtraArgs @('--installer-type', 'wix', '--scope', 'machine')
    }
    if (-not (Get-PwshExe)) { throw ('PowerShell 7 is still missing at {0} (winget exit code {1}).' -f $PwshExe, $res.Code) }
    return ('PowerShell ' + (Get-Item -LiteralPath $PwshExe).VersionInfo.ProductVersion + '; telemetry off')
}

function Install-BuildTools {
    $have = Get-VsCppPath
    if ($have) { return ('already installed: ' + $have) }
    Write-Note 'This is the longest step (often 10 to 25 minutes). A Visual Studio Installer window shows the progress.'
    $res = Install-WingetPackage -Id 'Microsoft.VisualStudio.BuildTools' -Override $VsOverride
    $have = Get-VsCppPath
    if ($have) { return ('installed: ' + $have) }
    # Build Tools may already exist without the C++ parts; winget then reports "already installed".
    # Add the parts with the Visual Studio Installer, started from another folder as Microsoft's docs require.
    $buildTools = Get-VsInstancePath @('-products', $VsProductBuildTools)
    $setup = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\setup.exe'
    if ($buildTools -and (Test-Path -LiteralPath $setup)) {
        Write-Note 'Build Tools are installed without C++; adding the C++ tools and the Windows 11 SDK.'
        $modifyArgs = @('modify', '--installPath', ('"{0}"' -f $buildTools), '--add', $VsWorkload, '--add', $VsCompMsvc,
            '--add', $VsCompSdk, '--passive', '--norestart')
        $p = Start-Process -FilePath $setup -ArgumentList $modifyArgs -WorkingDirectory $env:TEMP -Wait -PassThru
        if ($p.ExitCode -eq 3010) { $Ctx.RebootNeeded = $true }
        $have = Get-VsCppPath
        if ($have) { return ('added C++ to ' + $have) }
        throw ('The Visual Studio Installer could not add the C++ tools (exit code {0}).' -f $p.ExitCode)
    }
    throw ('The C++ build tools are still missing (winget exit code {0}).' -f $res.Code)
}

function Install-Llvm {
    if (Get-LlvmBin) { return 'already installed' }
    $res = Install-WingetPackage -Id 'LLVM.LLVM'
    if (Get-LlvmBin) { return ('installed in ' + (Get-LlvmBin)) }
    throw ('libclang.dll is missing from C:\Program Files\LLVM\bin after the install (winget exit code {0}).' -f $res.Code)
}

function Install-Rust {
    $rustup = Get-RustupExe
    if (-not $rustup) {
        $res = Install-WingetPackage -Id 'Rustlang.Rustup' -Override $RustupOverride
        $rustup = Get-RustupExe
        if (-not $rustup) { throw ('rustup.exe is missing after winget (exit code {0}).' -f $res.Code) }
    }
    Write-Note ('Making sure Rust {0} is installed (minimal profile, plus clippy and rustfmt)...' -f $RustToolchain)
    $r = Invoke-Native -FilePath $rustup -ArgumentList @('toolchain', 'install', $RustToolchain, '--profile', 'minimal',
        '--component', 'clippy,rustfmt', '--no-self-update')
    if ($r.ExitCode -ne 0) { throw ('rustup toolchain install failed: {0}' -f $r.Text) }
    $r = Invoke-Native -FilePath $rustup -ArgumentList @('default', $RustToolchain)
    if ($r.ExitCode -ne 0) { throw ('rustup default failed: {0}' -f $r.Text) }
    Update-SessionPath
    $rustc = Join-Path (Split-Path -Parent $rustup) 'rustc.exe'
    return (Invoke-Native -FilePath $rustc -ArgumentList @('--version')).Text
}

function Select-NodeVersion {
    # Newest Node.js 24.x that winget offers, preferring the plan's pin.
    $found = @{}
    try {
        $r = Invoke-Native -FilePath $Ctx.Winget -ArgumentList @('show', '--id', 'OpenJS.NodeJS.LTS', '--exact', '--source', 'winget',
            '--versions', '--accept-source-agreements', '--disable-interactivity')
        $pattern = '(?<![\d.]){0}\.\d+\.\d+(?![\d.])' -f $NodeMajor
        foreach ($m in [regex]::Matches($r.Text, $pattern)) { $found[$m.Value] = $true }
    } catch {
        return $NodePreferred
    }
    if ($found.ContainsKey($NodePreferred)) { return $NodePreferred }
    $best = @($found.Keys | Sort-Object { [version] $_ } -Descending)
    if ($best.Count -gt 0) {
        Write-Note ('winget does not offer Node.js {0} yet; using {1}, the newest {2}.x it has.' -f $NodePreferred, $best[0], $NodeMajor)
        return $best[0]
    }
    return $NodePreferred
}

function Install-Node {
    $current = Get-NodeVersion
    if ($current -and (Get-NodeMajor) -eq $NodeMajor) { return ('already installed: Node.js ' + $current) }
    $version = Select-NodeVersion
    $res = Install-WingetPackage -Id 'OpenJS.NodeJS.LTS' -Version $version -ExtraArgs @('--installer-type', 'wix', '--scope', 'machine')
    if ((Get-NodeMajor) -ne $NodeMajor) { throw ('Node.js {0} is not installed (winget exit code {1}).' -f $NodeMajor, $res.Code) }
    return ('Node.js ' + (Get-NodeVersion))
}

function Install-Pnpm {
    $nodeDir = Join-Path $env:ProgramFiles 'nodejs'
    $corepack = Join-Path $nodeDir 'corepack.cmd'
    if (-not (Test-Path -LiteralPath $corepack)) { throw 'corepack is missing (it comes with Node.js 24; see the Node.js step).' }
    $env:COREPACK_ENABLE_DOWNLOAD_PROMPT = '0'
    $r = Invoke-Native -FilePath $corepack -ArgumentList @('enable', 'pnpm')
    if ($r.ExitCode -ne 0) { throw ('corepack enable pnpm failed: {0}' -f $r.Text) }
    $r = Invoke-Native -FilePath $corepack -ArgumentList @('install', '-g', ('pnpm@{0}' -f $PnpmVersion))
    if ($r.ExitCode -ne 0) { throw ('corepack install -g pnpm@{0} failed: {1}' -f $PnpmVersion, $r.Text) }
    $v = Get-PnpmVersion
    if (-not $v) { throw 'pnpm does not answer after corepack enable.' }
    return ('pnpm ' + $v)
}

function Install-Ffmpeg {
    if (Test-FfmpegPinned) { return ('already installed: ' + $FfmpegAsset) }
    $download = Join-Path $ToolsDir '_download'
    if (-not (Test-Path -LiteralPath $download)) { $null = New-Item -ItemType Directory -Path $download -Force }
    $zip = Join-Path $download ($FfmpegAsset + '.zip')
    Write-Note 'Downloading FFmpeg (77 MB)...'
    Invoke-Download $FfmpegUrl $zip
    $hash = Get-Sha256 $zip
    if ($hash -ne $FfmpegSha256) {
        Remove-Item -LiteralPath $zip -Force
        throw ('The FFmpeg download has the wrong SHA-256 ({0}), so it was deleted and nothing was installed.' -f $hash)
    }
    $extract = Join-Path $download 'ffmpeg-extract'
    if (Test-Path -LiteralPath $extract) { Remove-Item -LiteralPath $extract -Recurse -Force }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [System.IO.Compression.ZipFile]::ExtractToDirectory($zip, $extract)
    $inner = Join-Path $extract $FfmpegAsset
    if (-not (Test-Path -LiteralPath (Join-Path $inner 'bin\ffmpeg.exe'))) { throw 'The FFmpeg zip does not have the expected folder layout.' }
    if (Test-Path -LiteralPath $FfmpegDir) { Remove-Item -LiteralPath $FfmpegDir -Recurse -Force }
    Move-Item -LiteralPath $inner -Destination $FfmpegDir
    $pin = [ordered]@{ asset = $FfmpegAsset; url = $FfmpegUrl; sha256 = $FfmpegSha256; installed_utc = (Get-UtcStamp) }
    Write-TextFile (Join-Path $FfmpegDir 'KAIRO-PIN.json') (ConvertTo-Json -InputObject $pin)
    Remove-Item -LiteralPath $download -Recurse -Force
    return ('installed ' + $FfmpegAsset + ' in ' + $FfmpegDir)
}

function Install-PresentMon {
    if (Test-PresentMonPinned) { return 'already installed' }
    if (-not (Test-Path -LiteralPath $PresentMonDir)) { $null = New-Item -ItemType Directory -Path $PresentMonDir -Force }
    $exe = Join-Path $PresentMonDir $PresentMonFile
    $part = $exe + '.download'
    Invoke-Download $PresentMonUrl $part
    $hash = Get-Sha256 $part
    if ($hash -ne $PresentMonSha256) {
        Remove-Item -LiteralPath $part -Force
        throw ('The PresentMon download has the wrong SHA-256 ({0}), so it was deleted.' -f $hash)
    }
    Move-Item -LiteralPath $part -Destination $exe -Force
    return ('installed ' + $exe)
}

function Initialize-CargoGuard {
    # cargo also reads C:\.cargo\config.toml when it builds in C:\kairo\src, and C:\ lets any signed-in user create
    # folders. An empty C:\.cargo that only administrators can change keeps anyone from planting a build setting
    # (a rustc wrapper, a linker) that Claude's elevated builds would run. tools/mac/remote/build.ps1 also refuses
    # to build while C:\.cargo or C:\kairo\.cargo holds a config file.
    $sddl = 'O:BAD:PAI(A;OICI;FA;;;BA)(A;OICI;FA;;;SY)(A;OICI;0x1200a9;;;BU)'
    if (-not (Test-Path -LiteralPath $CargoGuardDir)) {
        $security = New-Object System.Security.AccessControl.DirectorySecurity
        $security.SetSecurityDescriptorSddlForm($sddl)
        $null = [System.IO.Directory]::CreateDirectory($CargoGuardDir, $security)
    } else {
        $owner = (Get-Acl -LiteralPath $CargoGuardDir).GetOwner([System.Security.Principal.SecurityIdentifier]).Value
        if (@($SidAdministrators, $SidSystem) -notcontains $owner) {
            throw ('C:\.cargo already exists and belongs to {0}, so it was left alone. Claude''s builds refuse to run while it holds a config file; tell Claude what it is for.' -f (Resolve-SidName $owner))
        }
        Invoke-Icacls @($CargoGuardDir, '/inheritance:r', '/grant:r', ('*{0}:(OI)(CI)F' -f $SidAdministrators),
            '/grant:r', ('*{0}:(OI)(CI)F' -f $SidSystem), '/grant:r', ('*{0}:(OI)(CI)RX' -f $SidUsers), '/C', '/Q')
        foreach ($rule in @((Get-Acl -LiteralPath $CargoGuardDir).GetAccessRules($true, $false, [System.Security.Principal.SecurityIdentifier]))) {
            $sid = $rule.IdentityReference.Value
            if (@($SidAdministrators, $SidSystem, $SidUsers) -notcontains $sid) { Invoke-Icacls @($CargoGuardDir, '/remove', ('*{0}' -f $sid), '/C', '/Q') }
        }
        Invoke-Icacls @($CargoGuardDir, '/setowner', ('*{0}' -f $SidAdministrators), '/C', '/Q')
    }
    $explicit = @{}
    $explicit[$SidAdministrators] = $RightsFull
    $explicit[$SidSystem] = $RightsFull
    $explicit[$SidUsers] = $RightsRead
    if (-not (Test-FolderAcl -Path $CargoGuardDir -Explicit $explicit -Protected)) {
        throw 'C:\.cargo does not have the expected permissions (Administrators and SYSTEM full control, Users read only).'
    }
    $configs = @(Get-ChildItem -LiteralPath $CargoGuardDir -Force -ErrorAction SilentlyContinue | Where-Object { $_.Name -like 'config*' })
    if ($configs.Count -gt 0) {
        Write-Attention ('C:\.cargo holds {0}; Claude''s builds refuse to run until it is gone. Tell Claude.' -f (($configs | ForEach-Object { $_.Name }) -join ', '))
    }
    return 'C:\.cargo: only administrators can change it'
}

function Set-BuildEnvironment {
    $done = @()
    $llvm = Get-LlvmBin
    if ($llvm) {
        [Environment]::SetEnvironmentVariable('LIBCLANG_PATH', $llvm, 'Machine')
        $env:LIBCLANG_PATH = $llvm
        $done += ('LIBCLANG_PATH=' + $llvm)
    } else {
        Write-Attention 'LLVM is missing, so LIBCLANG_PATH was not set.'
    }
    if (Test-Path -LiteralPath (Join-Path $FfmpegDir 'include')) {
        [Environment]::SetEnvironmentVariable('FFMPEG_DIR', $FfmpegDir, 'Machine')
        $env:FFMPEG_DIR = $FfmpegDir
        $done += ('FFMPEG_DIR=' + $FfmpegDir)
    } else {
        Write-Attention 'FFmpeg is missing, so FFMPEG_DIR was not set.'
    }
    return ($done -join ', ')
}

function Set-SshdHardening {
    # Puts a marked block at the top of sshd_config (sshd uses the first value it reads for each keyword) and keeps
    # Windows' "Match Group administrators" block. Returns $true when the file changed.
    $original = [System.IO.File]::ReadAllText($SshdConfigPath)
    $nl = "`n"
    if ($original.Contains("`r`n")) { $nl = "`r`n" }
    $pristine = $SshdConfigPath + '.before-kairo'
    if (-not (Test-Path -LiteralPath $pristine)) { Copy-Item -LiteralPath $SshdConfigPath -Destination $pristine }

    $kept = New-Object System.Collections.Generic.List[string]
    $inBlock = $false
    foreach ($line in ($original -split "`r?`n")) {
        if ($line -eq $SshdBlockBegin) { $inBlock = $true; continue }
        if ($inBlock) { if ($line -eq $SshdBlockEnd) { $inBlock = $false }; continue }
        $kept.Add($line)
    }
    $hasAdminMatch = $false
    $inMatch = $false
    foreach ($line in $kept) {
        if ($line -match '^\s*Match\s+Group\s+administrators\s*$') { $hasAdminMatch = $true }
        if ($line -match '^\s*Match\s') { $inMatch = $true; continue }
        if ($inMatch -and $line -match '^\s*(PasswordAuthentication|KbdInteractiveAuthentication|ChallengeResponseAuthentication)\s+yes') {
            Write-Attention ('sshd_config has "{0}" inside a Match block, which can still allow passwords there.' -f $line.Trim())
        }
    }
    if (-not $hasAdminMatch) {
        Write-Attention 'The "Match Group administrators" block was missing from sshd_config; it was added back.'
        $kept.Add('Match Group administrators')
        $kept.Add('       AuthorizedKeysFile __PROGRAMDATA__/ssh/administrators_authorized_keys')
        $kept.Add('')
    }
    $block = @(
        $SshdBlockBegin,
        '# Key-only logins for the Kairo Mac. sshd uses the first value it reads, so this block sits at the top.',
        '# Undo: copy sshd_config.before-kairo over this file, then run: Restart-Service sshd',
        'PasswordAuthentication no',
        'KbdInteractiveAuthentication no',
        'PubkeyAuthentication yes',
        $SshdBlockEnd
    )
    $newText = ($block + $kept.ToArray()) -join $nl
    if ($newText -ceq $original) { return $false }

    $backup = '{0}.kairo-{1}' -f $SshdConfigPath, $Ctx.Stamp
    Copy-Item -LiteralPath $SshdConfigPath -Destination $backup -Force
    $Ctx.SshdBackup = $backup
    Write-TextFile $SshdConfigPath $newText
    $test = Test-SshdConfig
    if ($test.ExitCode -ne 0 -and $test.Text -match '(?i)kbdinteractiveauthentication') {
        # Older OpenSSH spells the same switch ChallengeResponseAuthentication (now its alias, servconf.h).
        Write-Attention 'This OpenSSH version rejects KbdInteractiveAuthentication; ChallengeResponseAuthentication no is used instead.'
        $block = @($block | ForEach-Object { if ($_ -eq 'KbdInteractiveAuthentication no') { 'ChallengeResponseAuthentication no' } else { $_ } })
        Write-TextFile $SshdConfigPath (($block + $kept.ToArray()) -join $nl)
        $test = Test-SshdConfig
    }
    # Before sshd's first start there are no host keys yet, and "sshd -t" stops at that point, after it has
    # parsed the whole file without complaint. Install-SshServer tests again once the keys exist.
    if ($test.ExitCode -ne 0 -and -not (Test-NoHostKeys $test)) {
        Copy-Item -LiteralPath $backup -Destination $SshdConfigPath -Force
        throw ('sshd rejected the new configuration, so the previous one was put back: {0}' -f $test.Text)
    }
    return $true
}

function Test-SshdConfig {
    return (Invoke-Native -FilePath $SshdExe -ArgumentList @('-t', '-f', $SshdConfigPath))
}

function Test-NoHostKeys($Result) {
    # sshd.c: "sshd: no hostkeys available -- exiting." comes after the configuration was read.
    return ([string] $Result.Text -match '(?i)no hostkeys available')
}

function Initialize-SshdConfig {
    # What sshd's own first start does (wmain_sshd.c, create_prgdata_ssh_folder: the folder with its SDDL, then a
    # copy of sshd_config_default), done before that start, so the first configuration sshd ever runs with is
    # already key-only. That first start still writes the host keys and keeps this sshd_config.
    if (-not (Test-Path -LiteralPath $SshDir)) {
        $security = New-Object System.Security.AccessControl.DirectorySecurity
        $security.SetSecurityDescriptorSddlForm($SshDirSddl)
        $null = [System.IO.Directory]::CreateDirectory($SshDir, $security)
        $sshOwner = (Get-Acl -LiteralPath $SshDir).GetOwner([System.Security.Principal.SecurityIdentifier]).Value
        if (@($SidAdministrators, $SidSystem) -notcontains $sshOwner) { throw ('{0} appeared while this setup was creating it; SSH was not set up. Tell Claude.' -f $SshDir) }
    }
    if (-not (Test-Path -LiteralPath $SshdConfigDefault)) { throw ('{0} is missing, so sshd_config cannot be prepared.' -f $SshdConfigDefault) }
    Copy-Item -LiteralPath $SshdConfigDefault -Destination $SshdConfigPath
}

function Set-AdminKeyAcl([string] $Path) {
    # As in PLAN section 3.1: no inheritance; Administrators and SYSTEM only.
    $r = Invoke-Native -FilePath 'icacls.exe' -ArgumentList @($Path, '/inheritance:r', '/grant', ('*{0}:F' -f $SidAdministrators), '/grant', ('*{0}:F' -f $SidSystem))
    if ($r.ExitCode -ne 0) { throw ('icacls failed on {0}: {1}' -f $Path, $r.Text) }
    $null = Invoke-Native -FilePath 'icacls.exe' -ArgumentList @($Path, '/setowner', ('*{0}' -f $SidAdministrators))
    $acl = Get-Acl -LiteralPath $Path
    foreach ($rule in @($acl.GetAccessRules($true, $true, [System.Security.Principal.SecurityIdentifier]))) {
        $sid = $rule.IdentityReference.Value
        if ($sid -ne $SidAdministrators -and $sid -ne $SidSystem) {
            $null = Invoke-Native -FilePath 'icacls.exe' -ArgumentList @($Path, '/remove:g', ('*{0}' -f $sid))
        }
    }
    $acl = Get-Acl -LiteralPath $Path
    $others = @($acl.GetAccessRules($true, $true, [System.Security.Principal.SecurityIdentifier]) |
        Where-Object { $_.IdentityReference.Value -ne $SidAdministrators -and $_.IdentityReference.Value -ne $SidSystem })
    if (-not $acl.AreAccessRulesProtected -or $others.Count -gt 0) { throw ('{0} still allows access beyond Administrators and SYSTEM.' -f $Path) }
}

function Set-AdminKey($KeyInfo) {
    # The owner is an administrator, so Windows' OpenSSH reads administrators_authorized_keys, not ~\.ssh.
    $kept = New-Object System.Collections.Generic.List[string]
    $others = 0
    if (Test-Path -LiteralPath $AdminKeysPath) {
        foreach ($line in @(Get-Content -LiteralPath $AdminKeysPath)) {
            $t = ([string] $line).Trim()
            if ($t -eq '') { continue }
            if ($t -match ('\s' + [regex]::Escape($KeyTag) + '$')) { continue }   # an earlier Kairo key
            if ($t.Contains($KeyInfo.Blob)) { continue }                         # the same key, other comment
            $kept.Add($t)
            if (-not $t.StartsWith('#')) { $others++ }
        }
    }
    $kept.Add($KeyInfo.Line)
    Write-TextFile $AdminKeysPath (($kept.ToArray() -join "`r`n") + "`r`n")
    Set-AdminKeyAcl $AdminKeysPath
    if ($others -gt 0) {
        Write-Attention ('{0} other key(s) in administrators_authorized_keys were kept; they can also log in as an administrator.' -f $others)
    }
    return ('key {0} ({1})' -f $KeyInfo.Fingerprint, $KeyTag)
}

function Get-SshOpenRule {
    # Read-only: every enabled inbound rule other than Kairo's that opens TCP 22 or sshd.exe (OpenSSH's own rule
    # allows every network profile). Set-SshFirewall switches these off; the consent screen lists them first.
    $portFilters = @{}
    foreach ($pf in @(Get-NetFirewallPortFilter -All -ErrorAction SilentlyContinue)) { $portFilters[[string] $pf.InstanceID] = $pf }
    $appFilters = @{}
    foreach ($af in @(Get-NetFirewallApplicationFilter -All -ErrorAction SilentlyContinue)) { $appFilters[[string] $af.InstanceID] = $af }
    $found = New-Object System.Collections.ArrayList
    foreach ($rule in @(Get-NetFirewallRule -Direction Inbound -Enabled True -Action Allow -ErrorAction SilentlyContinue)) {
        if ($rule.Name -eq $FirewallRuleName) { continue }
        $hit = ($rule.Name -eq $SshDefaultRule)
        $pf = $portFilters[[string] $rule.InstanceID]
        if ($pf -and ([string] $pf.Protocol -eq 'TCP') -and (@($pf.LocalPort) -contains '22')) { $hit = $true }
        $af = $appFilters[[string] $rule.InstanceID]
        if ($af -and ([string] $af.Program -match '(?i)\\sshd\.exe$')) { $hit = $true }
        if ($hit) { [void] $found.Add([ordered]@{ name = [string] $rule.Name; display_name = [string] $rule.DisplayName }) }
    }
    return @($found)
}

function Set-SshFirewall {
    # One rule: only sshd.exe, TCP 22, the Private profile, only the physical adapters whose addresses are all
    # private (Get-SshInterface), and only from their local subnet. A gaming VPN adapter set to Private (Hamachi
    # 25.0.0.0/8, Radmin 26.0.0.0/8) or a public address from a bridged modem never qualifies. With no qualifying
    # adapter the rule is switched off, so SSH stays closed. Then every other rule that opens SSH is switched off.
    $description = 'Lets the Kairo Mac reach this PC over SSH: sshd.exe only, TCP 22, Private networks on wired or Wi-Fi adapters with home-network addresses only, local subnet only. Added by tools/pc/bootstrap.ps1 (run it again after changing adapters); removed by tools/pc/uninstall-dev.ps1. ' + $RepoUrl
    $interfaces = @(Get-SshInterface)
    $aliases = @($interfaces | Where-Object { $_.qualifies } | ForEach-Object { $_.alias })
    foreach ($i in @($interfaces | Where-Object { -not $_.qualifies -and $_.category -eq 'Private' })) {
        Write-Attention ('SSH is not opened on "{0}": {1}.' -f $i.alias, $i.reason)
    }
    $enabled = 'True'
    if ($aliases.Count -eq 0) {
        $enabled = 'False'
        Write-Attention 'No wired or Wi-Fi adapter is on a Private network with a home-network address, so the SSH rule stays switched off. Set your network to Private (SETUP.md step 2), then run the command again.'
    }
    $ruleArgs = @{
        Direction     = 'Inbound'
        Protocol      = 'TCP'
        LocalPort     = 22
        Program       = $SshdExe
        Profile       = 'Private'
        RemoteAddress = 'LocalSubnet'
        Action        = 'Allow'
        Enabled       = $enabled
    }
    if ($aliases.Count -gt 0) { $ruleArgs.InterfaceAlias = $aliases }
    if (Get-NetFirewallRule -Name $FirewallRuleName -ErrorAction SilentlyContinue) {
        Set-NetFirewallRule -Name $FirewallRuleName @ruleArgs
    } else {
        $null = New-NetFirewallRule -Name $FirewallRuleName -DisplayName 'Kairo SSH (sshd, TCP 22, Private network, local subnet)' -Description $description -Group 'Kairo' @ruleArgs
    }
    $disabled = New-Object System.Collections.ArrayList
    foreach ($rule in @(Get-SshOpenRule)) {
        Disable-NetFirewallRule -Name $rule.name -Confirm:$false
        [void] $disabled.Add($rule)
        Write-Note ('Switched off the broader firewall rule "{0}".' -f $rule.display_name)
    }
    $Ctx.DisabledRules = @($disabled)
    $Ctx.FirewallInterfaces = $aliases
    if ($aliases.Count -eq 0) { return ('firewall rule {0} created but switched off (no qualifying adapter)' -f $FirewallRuleName) }
    return ('firewall rule {0}: sshd.exe, TCP 22, Private, LocalSubnet, on {1}' -f $FirewallRuleName, ($aliases -join ', '))
}

function Set-SshDefaultShell {
    # cmd.exe with AutoRun off (see $SshShellExe). The Mac's tools start PowerShell 7 with -NoProfile themselves.
    $key = 'HKLM:\SOFTWARE\OpenSSH'
    if (-not (Test-Path -LiteralPath $key)) { $null = New-Item -Path $key -Force }
    $null = New-ItemProperty -Path $key -Name 'DefaultShell' -Value $SshShellExe -PropertyType String -Force
    $null = New-ItemProperty -Path $key -Name 'DefaultShellCommandOption' -Value $SshShellOption -PropertyType String -Force
    # TODO(verify) on the PC: the inbox sshd reads DefaultShellArguments (current Win32-OpenSSH pwd.c does; older
    # builds ignore it, and then an interactive "pc ssh" login runs cmd.exe without /d).
    $null = New-ItemProperty -Path $key -Name 'DefaultShellArguments' -Value $SshShellArguments -PropertyType String -Force
    return ('SSH shell: {0} {1} (no AutoRun)' -f $SshShellExe, $SshShellOption)
}

function Get-SshdEffective {
    $r = Invoke-Native -FilePath $SshdExe -ArgumentList @('-T')
    if ($r.ExitCode -ne 0) { throw ('sshd -T failed: {0}' -f $r.Text) }
    $map = @{}
    foreach ($line in $r.Lines) { if ($line -match '^(\S+)\s+(.*)$') { $map[$Matches[1].ToLowerInvariant()] = $Matches[2].Trim() } }
    return $map
}

function Install-SshServer($KeyInfo) {
    # Fails closed: only the Kairo firewall rule admits port 22 before sshd ever runs, the first configuration sshd
    # runs with is already key-only, sshd starts with Windows only after "sshd -T" proves that, and any failure
    # stops sshd again (except inside an SSH session, which stopping would cut off).
    $notes = @()
    $state = Get-SshCapabilityState
    if ($state -ne 'Installed') {
        Write-Note 'Adding the OpenSSH Server feature (Windows downloads it; this can take a few minutes)...'
        $res = Add-WindowsCapability -Online -Name $SshCapabilityName
        if ($res.RestartNeeded) { $Ctx.RebootNeeded = $true }
        $notes += 'OpenSSH Server added'
    }
    $null = Get-Service -Name 'sshd'   # throws if the feature is still missing
    $wasRunning = ((Get-Service -Name 'sshd').Status -eq 'Running')
    $inSsh = Test-InSsh
    try {
        if (-not $wasRunning) { Set-Service -Name 'sshd' -StartupType Manual }
        # Any user may create folders in C:\ProgramData, so the key and the configuration go only into an
        # %ProgramData%\ssh that Administrators or SYSTEM own.
        if (Test-Path -LiteralPath $SshDir) {
            $sshOwner = (Get-Acl -LiteralPath $SshDir).GetOwner([System.Security.Principal.SecurityIdentifier]).Value
            if (@($SidAdministrators, $SidSystem) -notcontains $sshOwner) {
                throw ('{0} belongs to {1}, not to Administrators or SYSTEM, so SSH was not set up. Tell Claude.' -f $SshDir, (Resolve-SidName $sshOwner))
            }
        }
        $notes += (Set-SshFirewall)
        if (-not (Test-Path -LiteralPath $SshdConfigPath)) {
            Initialize-SshdConfig
            $notes += 'sshd_config prepared before the first start'
        }
        $changed = Set-SshdHardening
        $notes += (Set-AdminKey $KeyInfo)
        $notes += (Set-SshDefaultShell)
        if (-not $wasRunning) {
            # The first start also writes the host keys (wmain_sshd.c) before the service reports Running.
            Start-Service -Name 'sshd'
            $deadline = (Get-Date).AddSeconds(30)
            while (-not (Test-Path -LiteralPath $HostKeyPubPath) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
        } elseif ($changed) {
            if ($inSsh) {
                # Restarting sshd from inside an SSH session would cut this run off.
                $Ctx.SshdRestartPending = $true
                Write-Attention 'sshd_config changed; run "Restart-Service sshd" after this SSH session to apply it.'
            } else {
                Restart-Service -Name 'sshd' -Force
            }
        }
        $test = Test-SshdConfig
        if ($test.ExitCode -ne 0) { throw ('sshd -t rejects the configuration: {0}' -f $test.Text) }
        $effective = Get-SshdEffective
        $kbd = $effective['kbdinteractiveauthentication']
        if ($null -eq $kbd) { $kbd = $effective['challengeresponseauthentication'] }
        $bad = @()
        if ($effective['passwordauthentication'] -ne 'no') { $bad += ('passwordauthentication=' + $effective['passwordauthentication']) }
        if ($kbd -ne 'no') { $bad += ('kbdinteractiveauthentication=' + $kbd) }
        if ($effective['pubkeyauthentication'] -ne 'yes') { $bad += ('pubkeyauthentication=' + $effective['pubkeyauthentication']) }
        if ($bad.Count -gt 0) { throw ('sshd -T reports unexpected values: ' + ($bad -join ', ')) }
        Set-Service -Name 'sshd' -StartupType Automatic
    } catch {
        $failure = $_
        if (-not $inSsh) {
            try { Stop-Service -Name 'sshd' -Force -ErrorAction Stop } catch { Write-Note ('Could not stop sshd: ' + $_.Exception.Message) }
            try { Set-Service -Name 'sshd' -StartupType Manual -ErrorAction Stop } catch { Write-Note ('Could not set sshd to Manual: ' + $_.Exception.Message) }
            $Ctx.SshLeftOff = $true
        }
        throw $failure
    }
    $fingerprint = Get-HostKeyFingerprint
    if (-not $fingerprint) { Write-Attention ('The host key {0} is missing, so its fingerprint cannot be shown.' -f $HostKeyPubPath) }
    $notes += 'passwords and keyboard-interactive off, keys on, starts with Windows'
    return ($notes -join '; ')
}

function Add-PerfLogUser([string] $OwnerSid) {
    if (Test-GroupMember -GroupSid $SidPerfLogUsers -MemberSid $OwnerSid) { return 'already a member' }
    try {
        Add-LocalGroupMember -SID $SidPerfLogUsers -Member $OwnerSid -ErrorAction Stop
    } catch {
        if ([string] $_.FullyQualifiedErrorId -like 'MemberExists*') { return 'already a member' }
        # Fallback without the LocalAccounts module: net.exe with the localised names.
        $group = Get-GroupName $SidPerfLogUsers
        $user = (New-Object System.Security.Principal.SecurityIdentifier($OwnerSid)).Translate([System.Security.Principal.NTAccount]).Value
        $r = Invoke-Native -FilePath 'net.exe' -ArgumentList @('localgroup', $group, $user, '/add')
        if ($r.ExitCode -ne 0) { throw ('Could not add you to Performance Log Users: {0}' -f $r.Text) }
    }
    if (-not (Test-GroupMember -GroupSid $SidPerfLogUsers -MemberSid $OwnerSid)) { throw 'Performance Log Users still does not list you.' }
    return 'added (takes effect after you sign out and back in)'
}

function Write-GitConfigFile {
    # The global Git settings of Claude's elevated SSH sessions (tools/mac/remote sets GIT_CONFIG_GLOBAL to this
    # file). It lives in C:\kairo\tools, which only administrators can change, so those sessions never read your
    # own .gitconfig, where any program running as you could add a command for elevated Git to run.
    $text = @(
        '# Written by tools/pc/bootstrap.ps1 (https://github.com/Maty3k/Kairo). Git settings for Claude''s elevated',
        '# SSH sessions (GIT_CONFIG_GLOBAL); your own .gitconfig is never read there.',
        '[safe]',
        "`tdirectory = C:/kairo/src",
        ''
    ) -join "`n"
    Write-TextFile $GitConfigFile $text
}

function Initialize-SourceRepo {
    $git = Get-GitExe
    if (-not $git) { throw 'Git is not installed (see the Git step).' }
    Write-GitConfigFile
    # This elevated window uses the same admin-only settings as the SSH sessions, not your .gitconfig.
    $savedGlobal = $env:GIT_CONFIG_GLOBAL
    $env:GIT_CONFIG_GLOBAL = $GitConfigFile
    try {
        return (Initialize-SourceRepoCore $git)
    } finally {
        $env:GIT_CONFIG_GLOBAL = $savedGlobal
    }
}

function Initialize-SourceRepoCore([string] $git) {
    $created = $false
    if (-not (Test-Path -LiteralPath (Join-Path $SrcDir '.git'))) {
        $r = Invoke-Native -FilePath $git -ArgumentList @('init', '--initial-branch=snapshot', $SrcDir)
        if ($r.ExitCode -ne 0) { throw ('git init failed: {0}' -f $r.Text) }
        $created = $true
    }
    $settings = [ordered]@{
        'receive.denyCurrentBranch' = 'updateInstead'   # pushes update the checked-out tree (M0 contract)
        'core.autocrlf'             = 'false'           # builds must not dirty the tree (research 11 section 8.3)
        'core.longpaths'            = 'true'            # deep node_modules and target paths
    }
    foreach ($name in @($settings.Keys)) {
        $r = Invoke-Native -FilePath $git -ArgumentList @('-C', $SrcDir, 'config', $name, $settings[$name])
        if ($r.ExitCode -ne 0) { throw ('git config {0} failed: {1}' -f $name, $r.Text) }
    }
    $head = Invoke-Native -FilePath $git -ArgumentList @('-C', $SrcDir, 'symbolic-ref', '-q', 'HEAD')
    if ($head.Text -ne 'refs/heads/snapshot') {
        $hasCommits = Invoke-Native -FilePath $git -ArgumentList @('-C', $SrcDir, 'rev-parse', '-q', '--verify', 'HEAD')
        if ($hasCommits.ExitCode -ne 0) {
            $null = Invoke-Native -FilePath $git -ArgumentList @('-C', $SrcDir, 'symbolic-ref', 'HEAD', 'refs/heads/snapshot')
        } else {
            Write-Attention ('C:\kairo\src is on "{0}", not "snapshot"; it was left as it is.' -f $head.Text)
        }
    }
    # Owned by Administrators (the C:\kairo step), which Git for Windows accepts for administrators; the
    # safe.directory entry in the admin-only settings file covers the rest.
    $null = Invoke-Native -FilePath (Join-Path $env:SystemRoot 'System32\icacls.exe') -ArgumentList @($SrcDir, '/setowner', ('*{0}' -f $SidAdministrators), '/T', '/C', '/Q')
    if ($created) { return ('created an empty clone on branch snapshot; Git settings for SSH sessions in {0}' -f $GitConfigFile) }
    return ('clone already there; settings checked; Git settings for SSH sessions in {0}' -f $GitConfigFile)
}

function Register-KairoTasks {
    # The three M0 tasks. They point at C:\kairo\bin\current, which the first deploy creates; until then they fail
    # to start, which is expected.
    param([Parameter(Mandatory = $true)] [string] $UserId, [string[]] $Which = @('Run', 'Ensure', 'Engine'))
    $principal = New-ScheduledTaskPrincipal -UserId $UserId -LogonType Interactive -RunLevel Limited
    # Priority 4: tasks made by schtasks get 7 (below normal) and would skew timing (cookbook P13).
    $settings = New-ScheduledTaskSettingsSet -Priority 4 -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
    foreach ($name in $Which) {
        $arguments = ''
        $description = ''
        $triggers = @()
        if ($name -eq 'Run') {
            $arguments = 'run --job ' + $JobFile
            $description = 'Kairo dev: runs the job in C:\kairo\jobs\next.json in your session, on demand. ' + $RepoUrl
        } elseif ($name -eq 'Ensure') {
            $arguments = 'ensure'
            $description = 'Kairo dev: at logon and every minute, starts the watch agent if it is not running, then exits. ' + $RepoUrl
            # No -RepetitionDuration: per the Task Scheduler docs an empty duration repeats indefinitely.
            $triggers = @(
                (New-ScheduledTaskTrigger -AtLogOn -User $UserId),
                (New-ScheduledTaskTrigger -Once -At ((Get-Date).Date) -RepetitionInterval (New-TimeSpan -Minutes 1))
            )
        } elseif ($name -eq 'Engine') {
            $arguments = 'engine'
            $description = 'Kairo dev: starts the current test build in your session, on demand. ' + $RepoUrl
        } else {
            throw ('Unknown task name: {0}' -f $name)
        }
        $action = New-ScheduledTaskAction -Execute $DevrunExe -Argument $arguments -WorkingDirectory $CurrentBin
        $taskArgs = @{
            TaskPath    = $TaskPath
            TaskName    = $name
            Action      = $action
            Principal   = $principal
            Settings    = $settings
            Description = $description
            Force       = $true
        }
        if ($triggers.Count -gt 0) { $taskArgs.Trigger = $triggers }
        $null = Register-ScheduledTask @taskArgs
        if ($name -eq 'Ensure') { Repair-EnsureRepetition }
    }
}

function Repair-EnsureRepetition {
    # The Task Scheduler docs (RepetitionPattern.Duration) say an empty duration repeats indefinitely. If this
    # Windows filled one in anyway, remove it from the task's XML and register the task again. TODO(verify) on the PC
    # whether this ever triggers (docs/milestones/M0.md lists it as an open item).
    $t = Get-ScheduledTask -TaskPath $TaskPath -TaskName 'Ensure'
    $limited = @($t.Triggers | Where-Object { $_.Repetition -and $_.Repetition.Interval -and -not [string]::IsNullOrEmpty([string] $_.Repetition.Duration) })
    if ($limited.Count -eq 0) { return }
    Write-Note ('The 1-minute repetition got a duration ({0}); removing it so it repeats indefinitely.' -f $limited[0].Repetition.Duration)
    $xml = Export-ScheduledTask -TaskPath $TaskPath -TaskName 'Ensure'
    $pattern = '(<Repetition>(?:(?!</Repetition>).)*?)\s*<Duration>[^<]*</Duration>'
    $fixed = [regex]::Replace($xml, $pattern, '$1', [System.Text.RegularExpressions.RegexOptions]::Singleline)
    $null = Register-ScheduledTask -TaskPath $TaskPath -TaskName 'Ensure' -Xml $fixed -Force
}

function Test-KairoTasks([string[]] $Which = @('Run', 'Ensure', 'Engine')) {
    # Reads the tasks back and lists every difference from the M0 contract.
    $expected = @{ Run = ('run --job ' + $JobFile); Ensure = 'ensure'; Engine = 'engine' }
    $problems = @()
    foreach ($name in $Which) {
        $t = Get-ScheduledTask -TaskPath $TaskPath -TaskName $name -ErrorAction SilentlyContinue
        if (-not $t) { $problems += ('{0}: missing' -f $name); continue }
        if ([string] $t.Principal.LogonType -ne 'Interactive') { $problems += ('{0}: logon type {1}' -f $name, $t.Principal.LogonType) }
        if ([string] $t.Principal.RunLevel -ne 'Limited') { $problems += ('{0}: run level {1}' -f $name, $t.Principal.RunLevel) }
        if ([int] $t.Settings.Priority -ne 4) { $problems += ('{0}: priority {1}' -f $name, $t.Settings.Priority) }
        if ([string] $t.Settings.ExecutionTimeLimit -ne 'PT0S') { $problems += ('{0}: time limit {1}' -f $name, $t.Settings.ExecutionTimeLimit) }
        if ([string] $t.Settings.MultipleInstances -ne 'IgnoreNew') { $problems += ('{0}: multiple instances {1}' -f $name, $t.Settings.MultipleInstances) }
        $action = @($t.Actions)[0]
        if ([string] $action.Execute -ne $DevrunExe) { $problems += ('{0}: runs {1}' -f $name, $action.Execute) }
        if ([string] $action.Arguments -ne $expected[$name]) { $problems += ('{0}: arguments "{1}"' -f $name, $action.Arguments) }
        if ($name -eq 'Ensure') {
            $logon = @($t.Triggers | Where-Object { $_.CimClass.CimClassName -eq 'MSFT_TaskLogonTrigger' })
            $repeat = @($t.Triggers | Where-Object { $_.Repetition -and [string] $_.Repetition.Interval -eq 'PT1M' })
            if ($logon.Count -lt 1) { $problems += 'Ensure: no logon trigger' }
            if ($repeat.Count -lt 1) { $problems += 'Ensure: no 1-minute repetition' }
            foreach ($r in $repeat) {
                if (-not [string]::IsNullOrEmpty([string] $r.Repetition.Duration)) { $problems += ('Ensure: repeats only for {0}' -f $r.Repetition.Duration) }
            }
        }
    }
    return $problems
}

function Invoke-TasksOnly {
    $me = Get-OwnerIdentity
    Register-KairoTasks -UserId $me.Name -Which $Only
    $problems = @(Test-KairoTasks -Which $Only)
    if ($problems.Count -gt 0) { throw ('Tasks registered, but they differ from the M0 contract: ' + ($problems -join '; ')) }
    Write-Host ('Registered {0} in {1} for {2} (elevated: {3}).' -f ($Only -join ', '), $TaskPath, $me.Name, (Test-IsElevated))
}

# =============================================================================================================
# Report
# =============================================================================================================
function New-BootstrapReport($OwnerInfo, $KeyInfo) {
    $report = [ordered]@{
        schema            = 1
        bootstrap_version = $BootstrapVersion
        generated_utc     = (Get-UtcStamp)
        pc_name           = $env:COMPUTERNAME
        # name is DOMAIN\user; login is what SSH wants (tools/mac/ssh-setup.sh <ip> <login>).
        user              = [ordered]@{ name = $OwnerInfo.Name; login = $env:USERNAME; sid = $OwnerInfo.Sid }
    }
    # Each section returns one dictionary. The GPU list is collected with @() right here: an array passed through
    # the pipeline as one object would come out of Windows PowerShell's ConvertTo-Json as {"value":..,"Count":..}.
    $sections = [ordered]@{
        windows            = { Get-WindowsFacts }
        cpu                = { Get-CpuFacts }
        ram                = { Get-RamFacts }
        gpus               = $null
        monitors           = { Get-MonitorFacts }
        smart_app_control  = { Get-SmartAppControl }
        amd_instant_replay = { Get-AmdInstantReplay }
        network            = { Get-NetworkFacts }
        disk               = { Get-DiskFacts }
        toolchain          = { Get-ToolchainFacts }
    }
    foreach ($name in @($sections.Keys)) {
        try {
            if ($name -eq 'gpus') { $report[$name] = @(Get-GpuFacts) } else { $report[$name] = & $sections[$name] }
        } catch {
            $report[$name] = [ordered]@{ error = $_.Exception.Message }
        }
    }
    $sshd = $null
    try { $sshd = Get-Service -Name 'sshd' -ErrorAction Stop } catch { $sshd = $null }
    $report.ssh = [ordered]@{
        key_fingerprint          = $KeyInfo.Fingerprint
        key_tag                  = $KeyTag
        host_key_fingerprint     = (Get-HostKeyFingerprint)
        sshd_status              = $(if ($sshd) { [string] $sshd.Status } else { $null })
        sshd_start_type          = $(if ($sshd) { [string] $sshd.StartType } else { $null })
        sshd_left_off            = $Ctx.SshLeftOff
        default_shell            = ('{0} {1}' -f $SshShellExe, $SshShellOption)
        firewall_rule            = $FirewallRuleName
        firewall_interfaces      = @($Ctx.FirewallInterfaces)
        firewall_rules_disabled  = @($Ctx.DisabledRules)
        sshd_config_backup       = $Ctx.SshdBackup
        sshd_restart_pending     = $Ctx.SshdRestartPending
    }
    $report.steps = @($Ctx.Steps)
    $report.notes = @($Ctx.Notes)
    $report.reboot_needed = $Ctx.RebootNeeded
    return $report
}

# =============================================================================================================
# Main
# =============================================================================================================
function Show-Plan($Facts, $State, $OwnerInfo, $KeyInfo) {
    Write-Line ''
    Write-Line 'What this setup found:'
    Write-Note ('Windows: {0} {1} (build {2})' -f $Facts.windows.caption, $Facts.windows.display_version, $Facts.windows.build)
    foreach ($p in $Facts.network.profiles) {
        Write-Note ('Network: {0} = {1} {2}' -f $p.interface, $p.category, (@($p.ipv4) -join ', '))
    }
    Write-Note ('Free space on C: {0:N0} GB' -f ($Facts.disk.free_bytes / 1GB))
    Write-Note ('Smart App Control: {0} (read only; this setup never changes it)' -f $Facts.sac.state)
    Write-Note ('winget: {0}' -f $Facts.winget_version)
    Write-Note ('You: {0}' -f $OwnerInfo.Name)

    Write-Line ''
    Write-Line 'It will install these with winget, Microsoft''s package manager (already installed ones are skipped):'
    $items = @(
        @($State.Git, 'Git (Git.Git)'),
        @($State.Pwsh, ('PowerShell {0} (Microsoft.PowerShell, MSI) - Claude''s tools run in it over SSH' -f $PwshVersion)),
        @($State.BuildTools, 'Visual Studio Build Tools 2026 with the C++ tools and the Windows 11 SDK (about 7 GB; 10-25 minutes)'),
        @($State.Llvm, 'LLVM / clang (LLVM.LLVM) - Rust needs its libclang to read FFmpeg''s headers'),
        @($State.Rustup, ('Rust: rustup (Rustlang.Rustup), then Rust {0}' -f $RustToolchain)),
        @($State.Node, ('Node.js {0} LTS (OpenJS.NodeJS.LTS), then pnpm {1} through corepack' -f $NodeMajor, $PnpmVersion))
    )
    foreach ($item in $items) {
        if ($item[0]) { Write-Note ('[already there] ' + $item[1]) } else { Write-Note ('[install] ' + $item[1]) }
    }
    if ($State.Rustup) { Write-Note ('[check] Rust {0} is installed and set as the default' -f $RustToolchain) }

    Write-Line ''
    Write-Line 'It will download into C:\kairo\tools and check each file against a pinned SHA-256 fingerprint:'
    if ($State.Ffmpeg) { Write-Note '[already there] FFmpeg 9.0.2 (LGPL build by BtbN)' } else { Write-Note '[download] FFmpeg 9.0.2 (LGPL build by BtbN, 77 MB)' }
    if ($State.PresentMon) { Write-Note '[already there] PresentMon 2.6.0' } else { Write-Note '[download] PresentMon 2.6.0 (Intel''s frame-timing tool, 1 MB)' }

    Write-Line ''
    Write-Line 'It will change:'
    if ($State.Ssh -ne 'Installed') { Write-Note '- Add Windows'' OpenSSH Server feature.' }
    Write-Note '- OpenSSH Server: key logins only (passwords and keyboard-interactive off) before it first starts;'
    Write-Note '  it starts with Windows only once that is proven, and is switched off again if a step fails.'
    Write-Note ('  Allow the Mac''s key {0}.' -f $KeyInfo.Fingerprint)
    Write-Note '- SSH sessions start in cmd.exe with AutoRun off; Claude''s tools start PowerShell 7 without profiles.'
    $aliases = @($Facts.ssh_interfaces | Where-Object { $_.qualifies } | ForEach-Object { $_.alias })
    if ($aliases.Count -gt 0) {
        Write-Note ('- Firewall: a rule lets only sshd.exe receive TCP port 22, only on {0}' -f ($aliases -join ', '))
        Write-Note '  (Private network, home-network address) and only from that local network.'
    } else {
        Write-Note '- Firewall: an SSH rule is created but stays OFF: no wired or Wi-Fi adapter is on a Private network'
        Write-Note '  with a home-network address yet (SETUP.md step 2). Run the command again after fixing that.'
    }
    foreach ($i in @($Facts.ssh_interfaces | Where-Object { -not $_.qualifies -and $_.category -eq 'Private' })) {
        Write-Note ('  Not opened on "{0}": {1}.' -f $i.alias, $i.reason)
    }
    Write-Note '  These rules that open SSH more widely will be switched off:'
    foreach ($r in @($Facts.ssh_open_rules)) { Write-Note ('    "{0}" ({1})' -f $r.display_name, $r.name) }
    if (@($Facts.ssh_open_rules | Where-Object { $_.name -eq $SshDefaultRule }).Count -eq 0) {
        Write-Note ('    {0}, Windows'' own OpenSSH rule (it opens every network), once it exists' -f $SshDefaultRule)
    }
    if (-not $State.PerfGroup) { Write-Note ('- Add {0} to the "Performance Log Users" group (lets PresentMon run without admin).' -f $OwnerInfo.Name) }
    Write-Note '- Create C:\kairo with an empty code folder C:\kairo\src. Administrators and SYSTEM have full control;'
    Write-Note '  you can read everything, and your own programs can write only C:\kairo\logs and C:\kairo\state.'
    Write-Note '- Create an empty C:\.cargo that only administrators can change, so no other program can plant a'
    Write-Note '  Cargo setting there that Claude''s builds would read.'
    Write-Note '- Set three system variables: LIBCLANG_PATH and FFMPEG_DIR for builds, and'
    Write-Note '  POWERSHELL_TELEMETRY_OPTOUT=1, which turns PowerShell 7''s telemetry off.'
    Write-Note '- Create three scheduled tasks in the folder "Kairo" (Run, Ensure, Engine). They do nothing until'
    Write-Note '  Claude installs Kairo''s test programs; Ensure checks once a minute and exits at once.'
    Write-Line ''
    Write-Line 'It does not touch your games, AMD Software, Smart App Control or anything else.'
    Write-Line 'A log is saved in C:\kairo\logs. SETUP.md explains how to undo all of this.'
    Write-Line ''
}

function Read-Consent {
    if ($Yes) { Write-Line 'Continuing without asking (-Yes).'; return $true }
    try {
        $answer = Read-Host 'Type Y and press Enter to start (anything else stops without changing anything)'
    } catch {
        Write-Problem 'This window cannot ask questions. Run the command in Terminal (Admin), or add -Yes.'
        return $false
    }
    return ([string] $answer).Trim().ToUpperInvariant() -eq 'Y'
}

function Show-Done($Report) {
    $ips = @($Report.network.private_ipv4)
    $ip = '(none on a Private network yet)'
    if ($ips.Count -gt 0) { $ip = $ips -join ', ' }
    $failed = @($Ctx.Steps | Where-Object { -not $_.ok })
    Write-Line ''
    Write-Host '==============================================================================' -ForegroundColor Green
    if ($failed.Count -eq 0) {
        Write-Host ' DONE' -ForegroundColor Green
    } else {
        Write-Host (' DONE, but {0} step(s) had a problem:' -f $failed.Count) -ForegroundColor Yellow
        foreach ($f in $failed) { Write-Host ('   - {0}: {1}' -f $f.step, $f.detail) -ForegroundColor Yellow }
        Write-Host ' Tell Claude; most problems can be fixed over SSH. Running the same command again is safe.' -ForegroundColor Yellow
    }
    $hostKey = [string] $Report.ssh.host_key_fingerprint
    if (-not $hostKey) { $hostKey = '(missing: SSH is not set up yet)' }
    Write-Line ''
    Write-Line ('   PC name : {0}' -f $env:COMPUTERNAME)
    Write-Line ('   IP      : {0}' -f $ip)
    Write-Line ('   User    : {0}' -f $env:USERNAME)
    Write-Line ('   Host key: {0}' -f $hostKey)
    Write-Line ''
    Write-Line ' Send Claude these four lines. The host key lets the Mac check that it really talks to this PC.'
    if ($Ctx.SshLeftOff) {
        Write-Host ' SSH was switched OFF again because its step had a problem, so nothing can log in. Tell Claude.' -ForegroundColor Yellow
    }
    $sacState = [string] $Report.smart_app_control.state
    if (-not $sacState) { $sacState = 'unknown' }
    Write-Line (' Smart App Control is: {0}. Tell Claude what Start > "Smart App Control" shows too (SETUP.md step 6).' -f $sacState)
    if ($ips.Count -eq 0) {
        Write-Line ' No Private network was found: set your connection to Private (SETUP.md step 2), then tell'
        Write-Line ' Claude the IPv4 address shown in Settings > Network & internet > your connection.'
    }
    if ($Ctx.Transcript) { Write-Line (' The full log is in {0}' -f $Ctx.Transcript) }
    Write-Line ''
    if (Test-InSsh) {
        Write-Line ' (Run over SSH: no sign-out needed for this re-run.)'
    } elseif ($Ctx.RebootNeeded) {
        Write-Host ' Next: RESTART the PC (an install asked for it). That also signs you out and back in.' -ForegroundColor Cyan
    } else {
        Write-Host ' Next: SIGN OUT of Windows and sign back in (Start > your name > Sign out).' -ForegroundColor Cyan
        Write-Host ' That switches on the new group membership and the logon task.' -ForegroundColor Cyan
    }
    Write-Host '==============================================================================' -ForegroundColor Green
}

function Invoke-Bootstrap {
    Write-Line ''
    Write-Host ('Kairo PC setup ({0}) - {1}' -f $BootstrapVersion, $RepoUrl) -ForegroundColor Cyan

    # ---- Checks that change nothing ------------------------------------------------------------------------
    if ($env:OS -ne 'Windows_NT') { Write-Problem 'This script only runs on Windows.'; return }
    if ($PSVersionTable.PSEdition -eq 'Core') { Write-Problem 'Please run this in Windows PowerShell 5.1 (Terminal (Admin) opens it by default), not PowerShell 7.'; return }
    if (-not [Environment]::Is64BitProcess) { Write-Problem 'Please use the normal 64-bit Windows PowerShell, not the (x86) one.'; return }
    if ($ExecutionContext.SessionState.LanguageMode -ne 'FullLanguage') { Write-Problem ('PowerShell runs in {0} mode here (a security policy), so the setup cannot run. Tell Claude.' -f $ExecutionContext.SessionState.LanguageMode); return }
    if (-not (Test-IsElevated)) { Write-Problem 'This window is not running as administrator. Right-click Start, choose "Terminal (Admin)", click Yes, and paste the command again.'; return }
    try {
        $keyInfo = ConvertTo-KairoKey $MacKey
    } catch {
        Write-Problem $_.Exception.Message
        Write-Note 'Nothing was changed. Ask Claude for a fresh copy of the command.'
        return
    }
    $ownerInfo = Get-OwnerIdentity
    $consoleSid = Get-ConsoleUserSid
    if ($consoleSid -and $consoleSid -ne $ownerInfo.Sid) {
        Write-Problem ('This window runs as {0}, but someone else is signed in at the screen.' -f $ownerInfo.Name)
        Write-Note 'Run the command from your own Windows account (your account must be an administrator). Nothing was changed.'
        return
    }
    $Ctx.Winget = Get-WingetExe
    if (-not $Ctx.Winget) {
        Write-Problem 'winget (Microsoft''s package manager) is missing. Open the Microsoft Store, update "App Installer", then run the command again.'
        return
    }

    $tempLog = Join-Path $env:TEMP ('kairo-bootstrap-{0}.log' -f $Ctx.Stamp)
    $finalLog = Join-Path $LogsDir ('bootstrap-{0}.log' -f $Ctx.Stamp)
    $consented = $false
    $transcribing = $false
    try {
        try { $null = Start-Transcript -Path $tempLog -Force; $transcribing = $true } catch { $transcribing = $false }

        # ---- 1. Preflight -------------------------------------------------------------------------------------
        $facts = @{
            windows        = (Get-WindowsFacts)
            network        = (Get-NetworkFacts)
            disk           = (Get-DiskFacts)
            sac            = (Get-SmartAppControl)
            winget_version = (Invoke-Native -FilePath $Ctx.Winget -ArgumentList @('--version')).Text
            ssh_interfaces = @(Get-SshInterface)
            ssh_open_rules = @(Get-SshOpenRule)
        }
        $state = @{
            Git        = [bool] (Get-GitExe)
            Pwsh       = [bool] (Get-PwshExe)
            BuildTools = [bool] (Get-VsCppPath)
            Llvm       = [bool] (Get-LlvmBin)
            Rustup     = [bool] (Get-RustupExe)
            Node       = ((Get-NodeMajor) -eq $NodeMajor)
            Ffmpeg     = (Test-FfmpegPinned)
            PresentMon = (Test-PresentMonPinned)
            Ssh        = (Get-SshCapabilityState)
            PerfGroup  = (Test-GroupMember -GroupSid $SidPerfLogUsers -MemberSid $ownerInfo.Sid)
        }

        # ---- 2. What will change, then consent ----------------------------------------------------------------
        Show-Plan $facts $state $ownerInfo $keyInfo
        # Builds per learn.microsoft.com "Windows 11 release information": 21H2 = 22000 (first Windows 11),
        # 24H2 = 26100, 25H2 = 26200, 26H2 = 26300.
        $build = [int] $facts.windows.build_number
        if ($build -lt 22000) { Write-Problem 'This is not Windows 11, which Kairo''s dev setup needs. Nothing was changed.'; return }
        if ($build -lt 26200) { Write-Attention 'Windows is older than 25H2. SETUP.md step 1 says to take the 25H2 update first; you can still continue.' }
        if (@($facts.network.private_ipv4).Count -eq 0) { Write-Attention 'No network is set to Private, so the Mac cannot reach SSH yet (SETUP.md step 2). You can still continue.' }
        if ($facts.disk.free_bytes -lt 15GB) { Write-Problem 'Less than 15 GB is free on C:. Free some space and run the command again. Nothing was changed.'; return }
        if ($facts.disk.free_bytes -lt 40GB) { Write-Attention 'Less than 40 GB is free on C:; the tools take about 12 GB and builds need more later.' }
        if ($facts.sac.state -eq 'on') { Write-Attention 'Smart App Control is On: it may block FFmpeg and Kairo''s test builds until you choose a route (SETUP.md step 6).' }
        Write-Line ''
        $consented = Read-Consent
        if (-not $consented) { Write-Line 'Stopped. Nothing was changed.'; return }

        # ---- 3. Steps. SSH comes early so Claude can repair anything else remotely. ----------------------------
        # C:\kairo comes first, and nothing (this log included) goes into it unless it was set up safely.
        Invoke-Step 'Create C:\kairo (Administrators and SYSTEM full control; you read, and write logs and state)' { Initialize-KairoTree $ownerInfo.Sid }
        if ($transcribing -and $Ctx.TreeOk) {
            $null = Stop-Transcript
            $transcribing = $false
            $logTarget = $finalLog
            try { Move-Item -LiteralPath $tempLog -Destination $finalLog -Force } catch { $logTarget = $tempLog }
            try {
                $null = Start-Transcript -Path $logTarget -Append
                $transcribing = $true
                $Ctx.Transcript = $logTarget
            } catch {
                $transcribing = $false
            }
        } elseif ($transcribing) {
            $Ctx.Transcript = $tempLog
        }
        Invoke-Step 'Install Git' { Install-Git }
        Invoke-Step 'Install PowerShell 7 (Claude''s tools use it; telemetry off)' { Install-Pwsh }
        Invoke-Step 'Set up the OpenSSH server (keys only, Private network only)' { Install-SshServer $keyInfo }
        Invoke-Step 'Add you to Performance Log Users (S-1-5-32-559)' { Add-PerfLogUser $ownerInfo.Sid }
        Invoke-Step 'Prepare the code folder C:\kairo\src (branch snapshot)' { Assert-KairoTree; Initialize-SourceRepo }
        Invoke-Step 'Register the scheduled tasks \Kairo\Run, \Kairo\Ensure, \Kairo\Engine' {
            Assert-KairoTree
            Register-KairoTasks -UserId $ownerInfo.Name
            $taskProblems = @(Test-KairoTasks)
            if ($taskProblems.Count -gt 0) { throw ('registered, but not as the M0 contract says: ' + ($taskProblems -join '; ')) }
            'all three registered for ' + $ownerInfo.Name
        }
        Invoke-Step 'Create C:\.cargo for administrators only (keeps Cargo settings away from builds)' { Initialize-CargoGuard }
        Invoke-Step 'Install Visual Studio Build Tools 2026 (C++ tools, Windows 11 SDK)' { Install-BuildTools }
        Invoke-Step 'Install LLVM (libclang)' { Install-Llvm }
        Invoke-Step ('Install Rust (rustup, then {0})' -f $RustToolchain) { Install-Rust }
        Invoke-Step ('Install Node.js {0} LTS' -f $NodeMajor) { Install-Node }
        Invoke-Step ('Install pnpm {0} with corepack' -f $PnpmVersion) { Install-Pnpm }
        Invoke-Step 'Download FFmpeg 9.0.2 LGPL (BtbN, SHA-256 checked)' { Assert-KairoTree; Install-Ffmpeg }
        Invoke-Step 'Download PresentMon 2.6.0 (SHA-256 checked)' { Assert-KairoTree; Install-PresentMon }
        Invoke-Step 'Set LIBCLANG_PATH and FFMPEG_DIR' { Set-BuildEnvironment }
        Invoke-Step 'Make Administrators the owner of C:\kairo (all but logs and state)' { Assert-KairoTree; Set-KairoOwnership $ownerInfo.Sid }

        # ---- 4. Report -----------------------------------------------------------------------------------------
        Write-Title 'Write C:\kairo\state\bootstrap-report.json'
        $report = New-BootstrapReport $ownerInfo $keyInfo
        if ($Ctx.TreeOk) {
            try {
                Write-TextFile (Join-Path $StateDir 'bootstrap-report.json') (ConvertTo-Json -InputObject $report -Depth 8)
                Write-Good 'report written'
            } catch {
                Write-Problem ('Could not write the report: {0}' -f $_.Exception.Message)
            }
        } else {
            Write-Problem 'Not written: C:\kairo is not set up safely (see the first step).'
        }

        # ---- 5. Done -------------------------------------------------------------------------------------------
        Show-Done $report
    } finally {
        if ($transcribing) { try { $null = Stop-Transcript } catch { } }
        if (-not $consented -and (Test-Path -LiteralPath $tempLog)) { Remove-Item -LiteralPath $tempLog -Force -ErrorAction SilentlyContinue }
    }
}

if ($PSCmdlet.ParameterSetName -eq 'TasksOnly') {
    Invoke-TasksOnly
    return
}
if ($PSCmdlet.ParameterSetName -eq 'ReportOnly') {
    # Read-only: no key, no consent, no changes. Sections that need admin rights report their error instead.
    ConvertTo-Json -InputObject (New-BootstrapReport (Get-OwnerIdentity) $null) -Depth 8
    return
}
try {
    Invoke-Bootstrap
} catch {
    Write-Problem ('The setup stopped unexpectedly: {0}' -f $_.Exception.Message)
    Write-Note 'Running the same command again is safe. If it keeps failing, send Claude a photo of this window.'
}
