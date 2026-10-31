# SPDX-License-Identifier: GPL-3.0-or-later
# Shared helpers for the scripts tools/mac/pc runs on the Windows dev PC.
#
# How they run: tools/mac/pc pipes "<prelude> + lib.ps1 + <verb>.ps1" over SSH into
#   "C:\Program Files\PowerShell\7\pwsh.exe" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass
#       -OutputFormat Text -EncodedCommand <stub.ps1>
# which the PC's SSH shell, cmd.exe /d /c, starts. stub.ps1 resets the environment to the machine's,
# then reads that text from stdin and runs it as one script block. The prelude defines $Pc,
# a hashtable of arguments the Mac has already validated (plain ASCII, no quotes).
# Output goes to stdout. "@@KV key=value" lines are data for the Mac and are not shown.
# The last line is always "@@KAIRO-EXIT <code>": the Mac turns it back into its exit code (the last
# such line counts, and log and job output is always printed indented, so it cannot fake one).
# Exit codes: 0 ok, 1 failed, 2 bad input, 3 PC busy, 4 busy state unknown, 5 timed out, 6 job aborted.
#
# Contract: docs/PLAN.md section 3.1 and M0; crates/kairo-core/src/{layout,watch,job}.rs.
# TODO(verify): no PowerShell has parsed or run these scripts yet (there is no pwsh on the Mac);
# the first M0 run on the PC checks them, or CI can parse tools/**/*.ps1 on windows-latest.
# Keep this file ASCII-only (see .editorconfig) and free of PowerShell-7-only syntax, so that the
# version check below can still explain itself if an older shell runs it.

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

if ($PSVersionTable.PSVersion.Major -lt 7) {
    [Console]::Out.WriteLine('error: tools/mac/pc needs PowerShell 7 (pwsh) on the PC; this is PowerShell ' + $PSVersionTable.PSVersion)
    [Console]::Out.WriteLine('@@KAIRO-EXIT 1')
    exit 1
}

if (-not (Get-Variable -Name Pc -Scope 0 -ErrorAction SilentlyContinue)) { $Pc = @{} }

# --- The M0 layout (crates/kairo-core/src/layout.rs) ---------------------------------------
# Only logs, state and build are writable by the owner's own programs (tools/pc/bootstrap.ps1,
# build.ps1); records that elevated verbs trust, such as the last build, live in the admin-only
# folders, and nothing elevated runs what the owner's account could change.
$Kairo = @{
    Root      = 'C:\kairo'
    Src       = 'C:\kairo\src'
    Build     = 'C:\kairo\build'
    Target    = 'C:\kairo\build\target'
    Bin       = 'C:\kairo\bin'
    Current   = 'C:\kairo\bin\current'
    LastBuild = 'C:\kairo\bin\last-build.json'
    Jobs      = 'C:\kairo\jobs'
    NextJob   = 'C:\kairo\jobs\next.json'
    Logs      = 'C:\kairo\logs'
    State     = 'C:\kairo\state'
    Watch     = 'C:\kairo\state\watch.json'
    Tools     = 'C:\kairo\tools'
}
$WatchStaleSeconds = 5          # kairo-core WATCH_STALE_AFTER_MS
$BusyInputMs = 600000           # kairo-core BUSY_INPUT_WINDOW_MS
$KeepBuilds = 3
$BuildIdRegex = '^[0-9]{8}-[0-9]{6}-[0-9a-f]{7,40}\z'
$JobIdRegex = '^[A-Za-z0-9_-]{1,64}\z'
$Utf8NoBom = [Text.UTF8Encoding]::new($false)
$OutputEncoding = $Utf8NoBom

# --- Environment ---------------------------------------------------------------------------
# This session is elevated. stub.ps1 has already reset the environment to the machine's values;
# PATH stays the machine's alone (never your user PATH or ~\.cargo\bin, folders any program
# running as you can write), and the programs below are called by absolute path.
$UserHome = [Environment]::GetEnvironmentVariable('USERPROFILE')
if (-not $UserHome) { $UserHome = [Environment]::GetFolderPath('UserProfile') }
$env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine')
$SchtasksExe = Join-Path $env:SystemRoot 'System32\schtasks.exe'
# rustup lives in your profile (tools/pc/bootstrap.ps1), which any program running as you can change, so
# this session never runs it: build.ps1 hands cargo to a \Kairo\Run job, which runs it as you.
$CargoExe = Join-Path $UserHome '.cargo\bin\cargo.exe'

# --- Output --------------------------------------------------------------------------------
function Say([string]$Text = '') {
    [Console]::Out.WriteLine($Text)
}

function Exit-Pc([int]$Code) {
    [Console]::Out.WriteLine('@@KAIRO-EXIT ' + $Code)
    [Console]::Out.Flush()
    exit $Code
}

function Fail([string]$Text, [int]$Code = 1) {
    Say ('error: ' + $Text)
    Exit-Pc $Code
}

function Send-KV([string]$Key, [string]$Value) {
    Say ('@@KV ' + $Key + '=' + $Value)
}

# --- Files ---------------------------------------------------------------------------------
# The owner's own programs write C:\kairo\logs, state and build (the M1 contract, Builds), so any
# name there may be a link (a junction, a symbolic link, an object-manager link behind a
# junction) to a file only Administrators may read or write. So this elevated session never
# creates, renames or deletes anything in those folders (kairo-devrun prunes the old logs, as the
# owner), and reads a file there only through a handle it has checked (Open-OwnedFile). Nor does
# git-service.ps1, the other elevated script: its log goes to the admin-only C:\kairo\jobs.
$OwnerSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value

function Test-OwnerPath([string]$Path) {
    # True for a path in logs, state or build.
    $full = [IO.Path]::GetFullPath($Path)
    foreach ($dir in @($Kairo.Logs, $Kairo.State, $Kairo.Build)) {
        if ($full.StartsWith($dir + '\', [StringComparison]::OrdinalIgnoreCase)) { return $true }
    }
    return $false
}

function Open-Shared([string]$Path) {
    # Other processes (kairo-devrun, cargo) may be writing the file right now.
    $share = [IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete
    return [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, $share)
}

function Open-OwnedFile([string]$Path) {
    # A file in an owner-writable folder, opened once and checked through that same handle: a
    # disk file that the owner's account owns, so nothing it couldn't read itself. A link to an
    # admin-only file reaches a file someone else owns and is refused. .NET opens files with
    # anonymous impersonation, so a link to a named pipe gets no token from this session either.
    $stream = Open-Shared $Path
    try {
        if (-not $stream.CanSeek) { throw ('not a file: ' + $Path) }
        $owner = [IO.FileSystemAclExtensions]::GetAccessControl($stream).GetOwner([Security.Principal.SecurityIdentifier]).Value
        if ($owner -cne $OwnerSid) { throw ('not a file the owner''s account made: ' + $Path) }
    } catch {
        $stream.Dispose()
        throw
    }
    return $stream
}

function Open-ReadFile([string]$Path) {
    # Owner-writable folders through Open-OwnedFile; admin-only ones as they are.
    if (Test-OwnerPath $Path) { return (Open-OwnedFile $Path) }
    return (Open-Shared $Path)
}

function Read-SharedText([string]$Path, [long]$MaxBytes = 0) {
    # The whole file as text. With $MaxBytes, a larger file is refused, measured on the open handle:
    # the size of the name can be a link's, and a file can grow while it is read.
    $stream = Open-ReadFile $Path
    try {
        if ($MaxBytes -gt 0) {
            $buffer = [byte[]]::new($MaxBytes + 1)
            $count = 0
            while ($count -lt $buffer.Length) {
                $n = $stream.Read($buffer, $count, $buffer.Length - $count)
                if ($n -le 0) { break }
                $count += $n
            }
            if ($count -gt $MaxBytes) { throw ('larger than ' + $MaxBytes + ' bytes') }
            $stream.Dispose()
            $stream = [IO.MemoryStream]::new($buffer, 0, $count)
        }
        $reader = [IO.StreamReader]::new($stream, $Utf8NoBom, $true)
        return $reader.ReadToEnd()
    } finally {
        $stream.Dispose()
    }
}

function Write-TextFileAtomic([string]$Path, [string]$Text) {
    # Write a sibling temp file, then rename it over the target: readers never see half a file.
    # Only in the admin-only folders: never where the owner's programs write.
    if (Test-OwnerPath $Path) { Fail ('internal: ' + $Path + ' is in a folder the owner''s account can write') }
    $temp = $Path + '.tmp-' + $PID
    [IO.File]::WriteAllText($temp, $Text, $Utf8NoBom)
    [IO.File]::Move($temp, $Path, $true)
}

function Get-TailText([string]$Path, [int]$Lines = 200, [long]$MaxBytes = 1048576) {
    $stream = Open-ReadFile $Path
    try {
        $length = $stream.Length
        $start = [Math]::Max([long]0, $length - $MaxBytes)
        [void]$stream.Seek($start, [IO.SeekOrigin]::Begin)
        $count = [int]($length - $start)
        $buffer = New-Object byte[] $count
        $read = 0
        while ($read -lt $count) {
            $n = $stream.Read($buffer, $read, $count - $read)
            if ($n -le 0) { break }
            $read += $n
        }
    } finally {
        $stream.Dispose()
    }
    $text = $Utf8NoBom.GetString($buffer, 0, $read).TrimEnd([char[]]@([char]13, [char]10))
    $all = @()
    if ($text.Length -gt 0) { $all = @($text -split "`r?`n") }
    if ($start -gt 0 -and $all.Count -gt 1) { $all = @($all | Select-Object -Skip 1) }
    $shown = @($all | Select-Object -Last $Lines)
    return [pscustomobject]@{
        Text  = ($shown -join "`n")
        Shown = $shown.Count
        Cut   = (($start -gt 0) -or ($shown.Count -lt $all.Count))
        Bytes = $length
    }
}

function Say-Indented([string]$Text) {
    # Untrusted text (job output, logs, files the owner's programs write), two spaces in, so no
    # line of it can pass for an "@@KAIRO-EXIT" or "@@KV" line.
    foreach ($line in ($Text -split "`r?`n")) { Say ('  ' + $line) }
}

function Show-Tail([string]$Path, [int]$Lines = 200, [DateTime]$Since = [DateTime]::MinValue) {
    # Prints the end of a log file under a header; prints nothing if the file does not exist, or
    # was last written before $Since (UTC), so an earlier run's output never passes for this one's.
    if (-not [IO.File]::Exists($Path)) { return }
    if ([IO.File]::GetLastWriteTimeUtc($Path) -lt $Since) {
        Say ('--- ' + $Path + ': from an earlier run, not shown ---')
        return
    }
    try {
        $tail = Get-TailText -Path $Path -Lines $Lines
    } catch {
        Say ('--- ' + $Path + ': not read (' + $_.Exception.Message + ') ---')
        return
    }
    $note = ''
    if ($tail.Cut) { $note = ' (last ' + $tail.Shown + ' lines of ' + $tail.Bytes + ' bytes)' }
    Say ('--- ' + $Path + $note + ' ---')
    if ($tail.Text.Length -gt 0) { Say-Indented $tail.Text } else { Say '  (empty)' }
}

# --- JSON (System.Text.Json keeps strings as strings; ConvertFrom-Json would turn ISO dates into DateTime)
function ConvertFrom-JsonElement($Element) {
    switch ($Element.ValueKind.ToString()) {
        'Object' {
            $map = [ordered]@{}
            foreach ($property in $Element.EnumerateObject()) {
                $map[$property.Name] = ConvertFrom-JsonElement $property.Value
            }
            return $map
        }
        'Array' {
            $list = [Collections.Generic.List[object]]::new()
            foreach ($item in $Element.EnumerateArray()) { $list.Add((ConvertFrom-JsonElement $item)) }
            return , $list.ToArray()
        }
        'String' { return $Element.GetString() }
        'Number' {
            $whole = [long]0
            if ($Element.TryGetInt64([ref]$whole)) { return $whole }
            return $Element.GetDouble()
        }
        'True' { return $true }
        'False' { return $false }
        default { return $null }
    }
}

function Read-JsonText([string]$Text) {
    $document = [System.Text.Json.JsonDocument]::Parse($Text)
    try {
        return (ConvertFrom-JsonElement $document.RootElement)
    } finally {
        $document.Dispose()
    }
}

function Read-JsonFile([string]$Path) {
    # The parsed file, or $null if it is missing or not valid JSON (e.g. caught mid-write).
    if (-not [IO.File]::Exists($Path)) { return $null }
    for ($attempt = 0; $attempt -lt 5; $attempt++) {
        try {
            return (Read-JsonText (Read-SharedText $Path))
        } catch {
            Start-Sleep -Milliseconds 200
        }
    }
    return $null
}

function Get-Field($Map, [string]$Name) {
    if ($null -ne $Map -and $Map -is [Collections.IDictionary] -and $Map.Contains($Name)) { return $Map[$Name] }
    return $null
}

# --- The owner's activity: C:\kairo\state\watch.json (crates/kairo-core/src/watch.rs) -----------
function Get-KairoWatch {
    $watch = [ordered]@{
        State         = 'unknown'
        Reason        = ''
        Age           = $null
        Raw           = ''
        Busy          = $null
        LastInputMs   = $null
        Fullscreen    = $null
        ForegroundExe = ''
    }
    $path = $Kairo.Watch
    if (-not [IO.File]::Exists($path)) {
        $watch.Reason = $path + ' does not exist: the watch agent (kairo-devrun watch) is not running. Is a build with kairo-devrun deployed and \Kairo\Ensure registered?'
        return [pscustomobject]$watch
    }
    $data = $null
    $problem = ''
    for ($attempt = 0; ($attempt -lt 5) -and ($null -eq $data); $attempt++) {
        try {
            $text = Read-SharedText $path
            $watch.Raw = $text.Trim()
            $data = Read-JsonText $text
        } catch {
            $problem = $_.Exception.Message
            Start-Sleep -Milliseconds 200
        }
    }
    $watch.Age = [Math]::Round(([DateTime]::UtcNow - [IO.File]::GetLastWriteTimeUtc($path)).TotalSeconds, 1)
    if ($null -eq $data -or -not ($data -is [Collections.IDictionary])) {
        $watch.Reason = 'watch.json cannot be read as a JSON object: ' + $problem
        return [pscustomobject]$watch
    }
    $busy = Get-Field $data 'busy'
    if ($busy -is [bool]) { $watch.Busy = $busy }
    $lastInput = Get-Field $data 'last_input_ms_ago'
    if ($lastInput -is [long] -or $lastInput -is [double]) { $watch.LastInputMs = [double]$lastInput }
    $fullscreen = Get-Field $data 'foreground_fullscreen'
    if ($fullscreen -is [bool]) { $watch.Fullscreen = $fullscreen }
    $exe = Get-Field $data 'foreground_exe'
    if ($exe -is [string]) { $watch.ForegroundExe = $exe }

    # Busy = input in the last 10 minutes (or an unknown input time), or a fullscreen window in
    # front. Derived here too, so a wrong "busy" field can only make the PC look busier.
    $reasons = @()
    if ($watch.Fullscreen -eq $true) {
        $name = $watch.ForegroundExe
        if (-not $name) { $name = 'unknown program' }
        $reasons += ('a fullscreen window is in front (' + $name + ')')
    }
    if ($null -eq $watch.LastInputMs) {
        $reasons += 'the time of the last input is unknown'
    } elseif ($watch.LastInputMs -lt $BusyInputMs) {
        $reasons += ('owner input ' + [Math]::Round($watch.LastInputMs / 1000) + ' s ago')
    }
    if ($watch.Busy -eq $true -and $reasons.Count -eq 0) { $reasons += 'the watch agent reports busy' }

    if ($watch.Age -gt $WatchStaleSeconds) {
        $watch.Reason = 'watch.json is stale (last written ' + $watch.Age + ' s ago): the watch agent seems to have stopped'
    } elseif ($null -eq $watch.Busy) {
        $watch.Reason = 'watch.json has no true/false "busy" field'
    } elseif ($reasons.Count -gt 0) {
        $watch.State = 'busy'
        $watch.Reason = ($reasons -join '; ')
    } else {
        $watch.State = 'idle'
        $watch.Reason = 'no owner input for ' + [Math]::Floor($watch.LastInputMs / 60000) + ' min and nothing fullscreen'
    }
    return [pscustomobject]$watch
}

function Assert-PcIdle([string]$Verb) {
    # Stops the script unless the owner is away, or --force was given (the owner said it is OK).
    $watch = Get-KairoWatch
    if ($watch.State -eq 'idle') { return }
    if ($Pc['Force']) {
        Say ('warning: the PC is ' + $watch.State.ToUpperInvariant() + ' (' + $watch.Reason + '); going ahead with "' + $Verb + '" because of --force')
        return
    }
    if ($watch.State -eq 'busy') {
        Say ('refused: the PC is BUSY (' + $watch.Reason + '), so "' + $Verb + '" did not run.')
        Say 'The owner may be playing: work on the Mac. Use --force only if the owner said in this session that remote work is OK now.'
        Exit-Pc 3
    }
    Say ('refused: the busy state of the PC is UNKNOWN (' + $watch.Reason + '), so "' + $Verb + '" did not run.')
    Say 'Ask the owner whether the PC is free; with their OK, re-run with --force.'
    Exit-Pc 4
}

# --- Programs ------------------------------------------------------------------------------
function Find-Exe([string]$Name, [string[]]$Fallbacks = @()) {
    $command = Get-Command -Name $Name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($command) { return $command.Source }
    foreach ($candidate in $Fallbacks) {
        if ($candidate -and [IO.File]::Exists($candidate)) { return $candidate }
    }
    return $null
}

function Invoke-Native([string]$Exe, [string[]]$Arguments = @()) {
    # Runs a program, captures stdout and stderr as text lines, and returns its exit code.
    $path = Find-Exe $Exe
    if (-not $path) {
        return [pscustomobject]@{ Code = 9009; Lines = @('not found: ' + $Exe); Text = 'not found: ' + $Exe }
    }
    $ErrorActionPreference = 'Continue'
    $global:LASTEXITCODE = 0
    $lines = @(& $path @Arguments 2>&1 | ForEach-Object { [string]$_ })
    $code = $LASTEXITCODE
    return [pscustomobject]@{ Code = [int]$code; Lines = $lines; Text = ($lines -join "`n") }
}

# Git for Windows' machine install (Inno Setup DefaultDirName={commonpf}\Git; winget Git.Git, scope
# machine), as tools/pc/bootstrap.ps1 installs it, and nothing else: the machine PATH can hold
# folders your account writes (nvm-windows puts one under your AppData).
$GitExe = Join-Path $env:ProgramFiles 'Git\cmd\git.exe'
if (-not [IO.File]::Exists($GitExe)) { $GitExe = $null }

function Invoke-Git([string[]]$GitArgs) {
    if (-not $GitExe) {
        $missing = 'Git for Windows is missing from Program Files on the PC'
        return [pscustomobject]@{ Code = 9009; Lines = @($missing); Text = $missing }
    }
    return (Invoke-Native -Exe $GitExe -Arguments $GitArgs)
}

$IcaclsExe = Join-Path $env:SystemRoot 'System32\icacls.exe'

function Set-AdminOwner([string]$Path) {
    # Administrators own everything in C:\kairo that elevated sessions run or trust (docs/milestones/M0.md):
    # depending on a Windows policy, new files of an elevated session belong to the owner's account instead,
    # and an owner can always rewrite a file's permissions. So: the owner first, then a reset to the inherited
    # permissions, which removes anything the old owner may have added. Only for folders without links.
    $r = Invoke-Native -Exe $IcaclsExe -Arguments @($Path, '/setowner', '*S-1-5-32-544', '/T', '/C', '/Q')
    if ($r.Code -ne 0) { Fail ('could not make Administrators the owner of ' + $Path + ': ' + $r.Text) }
    $r = Invoke-Native -Exe $IcaclsExe -Arguments @($Path, '/reset', '/T', '/C', '/Q')
    if ($r.Code -ne 0) { Fail ('could not reset the permissions of ' + $Path + ': ' + $r.Text) }
}

function Get-FirstLine($Result) {
    if ($Result.Code -ne 0 -or $Result.Lines.Count -eq 0) { return '' }
    return ([string]$Result.Lines[0]).Trim()
}

# --- Scheduled tasks in \Kairo\ (COM, so state names are not localised) ----------------------
function Get-KairoTask([string]$Name) {
    $info = [ordered]@{ Name = $Name; Exists = $false; State = 'missing'; LastRun = ''; LastResult = '' }
    try {
        $service = New-Object -ComObject 'Schedule.Service'
        $service.Connect()
        $task = $service.GetFolder('\Kairo').GetTask($Name)
        $info.Exists = $true
        $names = @('unknown', 'disabled', 'queued', 'ready', 'running')   # TASK_STATE 0..4
        $state = [int]$task.State
        if ($state -ge 0 -and $state -lt $names.Count) { $info.State = $names[$state] } else { $info.State = 'state ' + $state }
        $info.LastRun = ([datetime]$task.LastRunTime).ToString('yyyy-MM-dd HH:mm:ss', [Globalization.CultureInfo]::InvariantCulture)
        $info.LastResult = '0x{0:X8}' -f ([int]$task.LastTaskResult)
    } catch {
        # Missing folder or task: keep the defaults.
    }
    return [pscustomobject]$info
}

# --- Builds in C:\kairo\bin -------------------------------------------------------------------
function Get-CurrentBuildId {
    $current = $Kairo.Current
    if (-not [IO.Directory]::Exists($current)) { return '' }
    $target = ([IO.DirectoryInfo]::new($current)).LinkTarget
    if (-not $target) { return '' }
    return [IO.Path]::GetFileName($target.TrimEnd([char]'\'))
}

function Get-BuildDirs {
    # Build folders, oldest first (their names start with a UTC timestamp). Call as @(Get-BuildDirs).
    if (-not [IO.Directory]::Exists($Kairo.Bin)) { return }
    Get-ChildItem -LiteralPath $Kairo.Bin -Directory -Force | Where-Object {
        ($_.Name -cmatch $BuildIdRegex) -and -not ($_.Attributes -band [IO.FileAttributes]::ReparsePoint)
    } | Sort-Object -Property Name
}

function Get-BuildCommit([string]$Id) {
    $info = Read-JsonFile (Join-Path (Join-Path $Kairo.Bin $Id) 'build.json')
    $commit = Get-Field $info 'commit'
    if ($commit -is [string]) { return $commit }
    return ''
}

function Remove-Junction([string]$Path) {
    $attributes = [IO.File]::GetAttributes($Path)
    if (-not ($attributes -band [IO.FileAttributes]::ReparsePoint)) {
        Fail ($Path + ' is a real folder, not a junction; refusing to delete it. Move it away by hand.')
    }
    # RemoveDirectory on a junction deletes the link itself, never the build it points to.
    [IO.Directory]::Delete($Path, $false)
}

function Set-CurrentBuild([string]$Id) {
    $target = Join-Path $Kairo.Bin $Id
    if (-not [IO.Directory]::Exists($target)) { Fail ('build ' + $Id + ' does not exist in ' + $Kairo.Bin) }
    $current = $Kairo.Current
    $staged = $current + '.new'
    if ([IO.Directory]::Exists($staged)) { Remove-Junction $staged }
    New-Item -ItemType Junction -Path $staged -Target $target | Out-Null
    # Swap: the moment without C:\kairo\bin\current is as short as two renames.
    if ([IO.Directory]::Exists($current)) { Remove-Junction $current }
    [IO.Directory]::Move($staged, $current)
}

function Remove-OldBuilds {
    # Keeps the newest $KeepBuilds builds plus the current one; folders still in use are skipped.
    $current = Get-CurrentBuildId
    $dirs = @(Get-BuildDirs)
    $keep = @($dirs | Select-Object -Last $KeepBuilds | ForEach-Object { $_.Name })
    foreach ($dir in $dirs) {
        if (($keep -contains $dir.Name) -or ($dir.Name -ceq $current)) { continue }
        try {
            Remove-Item -LiteralPath $dir.FullName -Recurse -Force -ErrorAction Stop
            Say ('  removed old build ' + $dir.Name)
        } catch {
            Say ('  kept old build ' + $dir.Name + ' for now (in use?): ' + $_.Exception.Message)
        }
    }
}

# --- Jobs (crates/kairo-core/src/job.rs) ----------------------------------------------------
function Test-KairoJob([string]$Text) {
    # The same rules as crates/kairo-core/src/job.rs, checked again here so that a bad file
    # never reaches C:\kairo\jobs\next.json.
    try { $job = Read-JsonText $Text } catch { Fail ('the job is not valid JSON: ' + $_.Exception.Message) 2 }
    if (-not ($job -is [Collections.IDictionary])) { Fail 'the job must be a JSON object' 2 }
    $known = @('id', 'cmd', 'cwd', 'max_seconds', 'abort_on_input', 'window', 'priority')
    foreach ($key in @($job.Keys)) {
        if ($known -cnotcontains $key) { Fail ('the job has an unknown field "' + $key + '" (kairo-devrun rejects unknown fields)') 2 }
    }
    foreach ($key in @('id', 'cmd', 'max_seconds')) {
        if (-not $job.Contains($key)) { Fail ('the job has no "' + $key + '"') 2 }
    }
    $id = $job['id']
    if (-not ($id -is [string]) -or ($id -cnotmatch $JobIdRegex)) { Fail 'the job "id" must match ^[A-Za-z0-9_-]{1,64}$' 2 }
    $cmd = $job['cmd']
    if (-not ($cmd -is [object[]]) -or $cmd.Count -eq 0) { Fail 'the job "cmd" must be a non-empty list of strings' 2 }
    foreach ($part in $cmd) {
        if (-not ($part -is [string]) -or ($part -match '[\x00-\x1f]')) { Fail 'every "cmd" entry must be a string without control characters' 2 }
    }
    if ([string]$cmd[0] -eq '') { Fail 'the job "cmd" starts with an empty program' 2 }
    $cwd = $null
    if ($job.Contains('cwd') -and ($null -ne $job['cwd'])) {
        $cwd = $job['cwd']
        if (-not ($cwd -is [string]) -or ($cwd -eq '') -or ($cwd -match '[\x00-\x1f]')) { Fail 'the job "cwd" must be null or a folder' 2 }
    }
    $max = $job['max_seconds']
    if (-not ($max -is [long]) -or ($max -lt 1) -or ($max -gt 86400)) { Fail 'the job "max_seconds" must be a whole number from 1 to 86400' 2 }
    $abort = $true
    if ($job.Contains('abort_on_input')) {
        $abort = $job['abort_on_input']
        if (-not ($abort -is [bool])) { Fail 'the job "abort_on_input" must be true or false' 2 }
    }
    $window = 'hidden'
    if ($job.Contains('window')) {
        $window = $job['window']
        if (-not ($window -is [string]) -or (@('visible', 'hidden') -cnotcontains $window)) { Fail 'the job "window" must be "visible" or "hidden"' 2 }
    }
    $priority = 'normal'
    if ($job.Contains('priority')) {
        $priority = $job['priority']
        if (-not ($priority -is [string]) -or (@('normal', 'below_normal') -cnotcontains $priority)) { Fail 'the job "priority" must be "normal" or "below_normal"' 2 }
    }
    return [pscustomobject]@{
        Id           = $id
        Program      = [string]$cmd[0]
        MaxSeconds   = [int]$max
        AbortOnInput = $abort
        Window       = $window
        Priority     = $priority
        Cwd          = $cwd
    }
}

function Start-KairoJob([string]$JobText) {
    # Checks the job, writes C:\kairo\jobs\next.json and starts \Kairo\Run, which runs it in the owner's
    # session with the owner's limited token. Returns the job and the files its result and output go to.
    $job = Test-KairoJob $JobText
    $devrun = Join-Path $Kairo.Current 'kairo-devrun.exe'
    if (-not [IO.File]::Exists($devrun)) {
        Fail ('there is no ' + $devrun + ' yet: build and deploy first (tools/mac/pc build, then tools/mac/pc deploy)')
    }
    $task = Get-KairoTask 'Run'
    if (-not $task.Exists) { Fail 'the scheduled task \Kairo\Run is not registered (the bootstrap creates it)' }
    if ($task.State -eq 'disabled') { Fail 'the scheduled task \Kairo\Run is disabled' }
    if ($task.State -eq 'running') {
        Fail 'another \Kairo\Run job is still running, and \Kairo\Run ignores new starts until it ends; wait for it (tools/mac/pc logs) and try again'
    }
    if ([IO.Path]::IsPathRooted($job.Program) -and -not [IO.File]::Exists($job.Program)) {
        Say ('warning: ' + $job.Program + ' does not exist on the PC')
    }
    if ($job.AbortOnInput -and $Pc['Force'] -and ((Get-KairoWatch).State -eq 'busy')) {
        Say 'note: abort_on_input is true and the owner is active, so kairo-devrun will probably stop the job at the next input'
    }
    $run = [pscustomobject]@{
        Job        = $job
        Started    = [DateTime]::UtcNow
        ResultFile = Join-Path $Kairo.Logs ($job.Id + '.result.json')
        OutFile    = Join-Path $Kairo.Logs ($job.Id + '.out.log')
        ErrFile    = Join-Path $Kairo.Logs ($job.Id + '.err.log')
        OwnLog     = Join-Path $Kairo.Logs ($job.Id + '.log')
    }
    # An earlier run's files for this id stay: kairo-devrun replaces them as the owner, and
    # Wait-KairoJob takes only a result that started after this one.
    [void](New-Item -ItemType Directory -Force -Path $Kairo.Jobs)
    Write-TextFileAtomic $Kairo.NextJob $JobText
    $run.Started = [DateTime]::UtcNow
    # TODO(verify): schtasks /run takes the folder form \Kairo\Run (the M0 contract; documented for /create).
    $trigger = Invoke-Native -Exe $SchtasksExe -Arguments @('/run', '/tn', '\Kairo\Run')
    if ($trigger.Code -ne 0) { Fail ('schtasks /run /tn \Kairo\Run failed: ' + $trigger.Text) }
    return $run
}

function Stop-KairoRun {
    # Ends the running \Kairo\Run instance: kairo-devrun dies and its kill-on-close job object takes the
    # job's whole process tree with it.
    $null = Invoke-Native -Exe $SchtasksExe -Arguments @('/end', '/tn', '\Kairo\Run')
}

function Test-ResultOf($Run, $Result) {
    # Whether a result file is this run's: it started no earlier than the job was handed over,
    # not an earlier run's under the same id. Both times come from this PC's clock, and
    # kairo-devrun stamps after the hand-over; 2 ms allow for started_utc's whole milliseconds.
    $text = [string](Get-Field $Result 'started_utc')
    $styles = [Globalization.DateTimeStyles]::AdjustToUniversal -bor [Globalization.DateTimeStyles]::AssumeUniversal
    $started = [DateTime]::MinValue
    if (-not [DateTime]::TryParse($text, [Globalization.CultureInfo]::InvariantCulture, $styles, [ref]$started)) { return $false }
    return ($started -ge $Run.Started.AddMilliseconds(-2))
}

function Wait-KairoJob($Run, [int]$WaitSeconds, [scriptblock]$StopWhen = $null) {
    # Waits for the job's result file and returns it; $null if the job is still running at the deadline,
    # or \Kairo\Run ended without writing one. $StopWhen runs about every 2 s; a non-empty string it returns
    # stops the job (Stop-KairoRun), and the reason comes back as StoppedFor.
    $deadline = $Run.Started.AddSeconds($WaitSeconds)
    $sawRunning = $false
    $idleSince = $null
    $nextNote = 60
    $tick = 0
    while ([DateTime]::UtcNow -lt $deadline) {
        Start-Sleep -Milliseconds 1000
        if ([IO.File]::Exists($Run.ResultFile)) {
            $result = Read-JsonFile $Run.ResultFile
            if (($null -ne $result) -and (Test-ResultOf $Run $result)) { return [pscustomobject]@{ Result = $result; StoppedFor = '' } }
        }
        $tick += 1
        if ($StopWhen -and ($tick % 2 -eq 0)) {
            $why = [string](& $StopWhen)
            if ($why) {
                Stop-KairoRun
                return [pscustomobject]@{ Result = $null; StoppedFor = $why }
            }
        }
        if ((Get-KairoTask 'Run').State -eq 'running') {
            $sawRunning = $true
            $idleSince = $null
        } else {
            if ($null -eq $idleSince) { $idleSince = [DateTime]::UtcNow }
            $grace = 5
            if (-not $sawRunning) { $grace = 20 }   # the task may take a moment to start
            if (([DateTime]::UtcNow - $idleSince).TotalSeconds -gt $grace) { break }
        }
        $elapsed = ([DateTime]::UtcNow - $Run.Started).TotalSeconds
        if ($elapsed -ge $nextNote) {
            Say ('... job ' + $Run.Job.Id + ' is still running (' + [int]$elapsed + ' s)')
            $nextNote += 60
        }
    }
    return [pscustomobject]@{ Result = $null; StoppedFor = '' }
}

function Test-RunTaskBusy {
    # True while a \Kairo\Run job is running (deploy, rollback and run must not pull the rug).
    $run = Get-KairoTask 'Run'
    return ($run.State -eq 'running')
}

function Restart-WatchAgent {
    # The watch agent keeps running the old build after the junction moves. Stop the
    # "kairo-devrun watch" (and any "ensure") processes, never a "run" job or the "engine", and
    # start \Kairo\Ensure, which starts "kairo-devrun watch" from the new build.
    $stopped = 0
    $processes = @(Get-CimInstance -ClassName Win32_Process -Filter "Name = 'kairo-devrun.exe'" -ErrorAction SilentlyContinue)
    foreach ($process in $processes) {
        $commandLine = [string]$process.CommandLine
        if ($commandLine -notmatch '(^|\s)(watch|ensure)(\s|$)') { continue }
        try {
            Stop-Process -Id ([int]$process.ProcessId) -Force -ErrorAction Stop
            $stopped++
        } catch {
            Say ('  could not stop kairo-devrun (pid ' + $process.ProcessId + '): ' + $_.Exception.Message)
        }
    }
    if ($stopped -gt 0) { Say ('  stopped ' + $stopped + ' kairo-devrun process(es) of the previous build') }
    if (-not [IO.File]::Exists((Join-Path $Kairo.Current 'kairo-devrun.exe'))) {
        Say '  warning: the current build has no kairo-devrun.exe, so the watch agent cannot start'
        return
    }
    $ensure = Get-KairoTask 'Ensure'
    if (-not $ensure.Exists) {
        Say '  warning: \Kairo\Ensure is not registered, so nothing restarts the watch agent'
        return
    }
    $since = [DateTime]::UtcNow
    # TODO(verify): schtasks /run takes the folder form \Kairo\Ensure (documented for /create).
    $result = Invoke-Native -Exe $SchtasksExe -Arguments @('/run', '/tn', '\Kairo\Ensure')
    if ($result.Code -ne 0) {
        Say ('  warning: schtasks /run /tn \Kairo\Ensure failed: ' + $result.Text)
        return
    }
    $deadline = $since.AddSeconds(15)
    while ([DateTime]::UtcNow -lt $deadline) {
        Start-Sleep -Milliseconds 500
        if ([IO.File]::Exists($Kairo.Watch) -and ([IO.File]::GetLastWriteTimeUtc($Kairo.Watch) -gt $since)) {
            Say '  the watch agent of the new build is writing watch.json'
            return
        }
    }
    Say '  warning: watch.json was not rewritten within 15 s of starting \Kairo\Ensure (is the owner signed in?)'
}
