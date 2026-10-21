# SPDX-License-Identifier: GPL-3.0-or-later
# pc build: cargo build --release --locked of the workspace in C:\kairo\src, at below-normal
# priority. Unless --force, cargo is stopped as soon as the owner becomes active (input or a
# fullscreen window). The build is logged like a job (C:\kairo\logs\build-<utc>.{out,err}.log and
# .result.json, so "pc logs" lists it) and recorded in C:\kairo\bin\last-build.json for
# "pc deploy". Exit code: 0 built, 1 failed, 3 stopped because the owner became active.
#
# This runs in the elevated SSH session, so every build script and proc-macro of every dependency
# runs as an administrator. Hence: only with a committed Cargo.lock (--locked, nothing resolved
# here), never while a Cargo config file sits in a parent folder of C:\kairo\src, and always into
# C:\kairo\src\target. docs/milestones/M0.md records the plan to move builds to the limited token.

Assert-PcIdle 'build'

function Format-Utc([DateTime]$Time) {
    return $Time.ToUniversalTime().ToString("yyyy-MM-dd'T'HH:mm:ss.fff'Z'", [Globalization.CultureInfo]::InvariantCulture)
}

$repo = $Kairo.Src
if (-not [IO.File]::Exists((Join-Path $repo 'Cargo.toml'))) {
    Fail ('there is no Cargo.toml in ' + $repo + ': run tools/mac/pc sync first')
}
$cargo = $CargoExe
if (-not [IO.File]::Exists($cargo)) { Fail ('cargo is not installed on the PC (' + $cargo + '): see SETUP.md, or run tools/mac/pc doctor') }
# cargo reads .cargo\config.toml (and the older .cargo\config) in every parent folder of the build
# folder (doc.rust-lang.org/cargo/reference/config.html). C:\.cargo is admin-only (the bootstrap),
# but refuse anyway if anything is there, and in C:\kairo\.cargo.
foreach ($dir in @((Join-Path $env:SystemDrive '.cargo'), (Join-Path $Kairo.Root '.cargo'))) {
    if (-not [IO.Directory]::Exists($dir)) { continue }
    $configs = @(Get-ChildItem -LiteralPath $dir -Force -ErrorAction SilentlyContinue | Where-Object { $_.Name -like 'config*' })
    if ($configs.Count -gt 0) {
        Fail ($dir + ' holds a Cargo config (' + (($configs | ForEach-Object { $_.Name }) -join ', ') + '), which this elevated build would read; refusing. Ask the owner what it is.')
    }
}

$others = @(Get-Process -Name 'cargo' -ErrorAction SilentlyContinue)
if ($others.Count -gt 0 -and -not $Pc['Force']) {
    Fail ('cargo is already running on the PC (pid ' + (($others | ForEach-Object { $_.Id }) -join ', ') + '); wait for it, or re-run with --force')
}

$commit = Get-FirstLine (Invoke-Git @('-C', $repo, 'rev-parse', '--verify', '-q', 'HEAD'))
$porcelain = Invoke-Git @('-C', $repo, 'status', '--porcelain', '--untracked-files=normal')
$dirty = (@($porcelain.Lines | Where-Object { $_ }).Count -gt 0)
$lockTracked = ((Invoke-Git @('-C', $repo, 'ls-files', '--error-unmatch', '--', 'Cargo.lock')).Code -eq 0)
if (-not $lockTracked) {
    Fail 'Cargo.lock is not in the snapshot; refusing to resolve dependencies in an elevated build (commit Cargo.lock, then sync again)'
}

$targetDir = $Kairo.Target
$cargoArgs = @('build', '--release', '--locked', '--target-dir', $targetDir)
$packages = @($Pc['Packages'] | Where-Object { $_ })
if ($packages.Count -gt 0) {
    foreach ($package in $packages) { $cargoArgs += @('-p', $package) }
} else {
    $cargoArgs += '--workspace'
}

$invariant = [Globalization.CultureInfo]::InvariantCulture
$started = [DateTime]::UtcNow
$id = 'build-' + $started.ToString('yyyyMMdd-HHmmss', $invariant)
[void](New-Item -ItemType Directory -Force -Path $Kairo.Logs)
[void](New-Item -ItemType Directory -Force -Path $Kairo.Bin)
$outLog = Join-Path $Kairo.Logs ($id + '.out.log')
$errLog = Join-Path $Kairo.Logs ($id + '.err.log')
$env:CARGO_TERM_COLOR = 'never'
# rustup's cargo.exe is a proxy that finds rustc and the toolchain through ~\.cargo\bin, so only the
# build itself gets that folder on its PATH (the same folder cargo.exe comes from).
$env:Path = (Split-Path -Parent $cargo) + ';' + $env:Path

$what = $commit
if (-not $what) { $what = '(no commit)' }
if ($dirty) { $what += ' with uncommitted changes on the PC' }
Say ('building ' + $what + ': cargo ' + ($cargoArgs -join ' '))
Say ('log: ' + $errLog)

$process = Start-Process -FilePath $cargo -ArgumentList $cargoArgs -WorkingDirectory $repo -NoNewWindow -PassThru `
    -RedirectStandardOutput $outLog -RedirectStandardError $errLog
$null = $process.Handle   # keep the handle so that ExitCode is still readable after the exit
try {
    $process.PriorityClass = [Diagnostics.ProcessPriorityClass]::BelowNormal   # rustc children inherit it
} catch {
    Say ('warning: could not lower the priority of cargo: ' + $_.Exception.Message)
}

$abortReason = ''
$nextNote = 60
while (-not $process.WaitForExit(2000)) {
    if (-not $Pc['Force']) {
        $now = Get-KairoWatch
        if ($now.State -eq 'busy') {
            $abortReason = $now.Reason
            break
        }
    }
    $elapsed = ([DateTime]::UtcNow - $started).TotalSeconds
    if ($elapsed -ge $nextNote) {
        Say ('... still building (' + [int]$elapsed + ' s)')
        $nextNote += 60
    }
}
if ($abortReason) {
    try { $process.Kill($true) } catch { }
    [void]$process.WaitForExit(15000)
} else {
    $process.WaitForExit()
}
$ended = [DateTime]::UtcNow
$exitCode = $null
try { $exitCode = [int]$process.ExitCode } catch { }
$aborted = [bool]$abortReason
$reason = 'exited'
if ($aborted) {
    $reason = 'input'
    $exitCode = 1
}

# Where did the executables go? target\release, or target\<triple>\release if the workspace
# sets build.target; pick the folder with the newest .exe.
$artifacts = ''
$newest = [DateTime]::MinValue
foreach ($dir in @((Join-Path $targetDir 'release'), (Join-Path $targetDir 'x86_64-pc-windows-msvc\release'))) {
    if (-not [IO.Directory]::Exists($dir)) { continue }
    $exe = Get-ChildItem -LiteralPath $dir -Filter '*.exe' -File -ErrorAction SilentlyContinue |
        Sort-Object -Property LastWriteTimeUtc -Descending | Select-Object -First 1
    if ($exe -and $exe.LastWriteTimeUtc -gt $newest) {
        $newest = $exe.LastWriteTimeUtc
        $artifacts = $dir
    }
}

$result = [ordered]@{
    id          = $id
    exit_code   = $exitCode
    aborted     = $aborted
    reason      = $reason
    started_utc = (Format-Utc $started)
    ended_utc   = (Format-Utc $ended)
}
Write-TextFileAtomic (Join-Path $Kairo.Logs ($id + '.result.json')) ($result | ConvertTo-Json -Compress)
$record = [ordered]@{
    id            = $id
    commit        = $commit
    dirty         = $dirty
    exit_code     = $exitCode
    aborted       = $aborted
    locked        = $lockTracked
    packages      = $packages
    artifacts_dir = $artifacts
    started_utc   = (Format-Utc $started)
    ended_utc     = (Format-Utc $ended)
}
Write-TextFileAtomic $Kairo.LastBuild ($record | ConvertTo-Json -Compress -Depth 4)

# Keep the logs of the last 10 builds.
$oldBuilds = @(Get-ChildItem -LiteralPath $Kairo.Logs -Filter 'build-*.result.json' -File -ErrorAction SilentlyContinue |
    Sort-Object -Property Name -Descending | Select-Object -Skip 10)
foreach ($old in $oldBuilds) {
    $base = $old.Name.Substring(0, $old.Name.Length - '.result.json'.Length)
    foreach ($suffix in @('.result.json', '.out.log', '.err.log')) {
        Remove-Item -LiteralPath (Join-Path $Kairo.Logs ($base + $suffix)) -Force -ErrorAction SilentlyContinue
    }
}

$seconds = [int]($ended - $started).TotalSeconds
$tail = 60
if ($Pc['Tail']) { $tail = [int]$Pc['Tail'] }
if ($aborted) {
    Say ('STOPPED after ' + $seconds + ' s: the owner became active (' + $abortReason + '). Nothing was deployed.')
    Show-Tail $errLog 10
    Exit-Pc 3
}
if ($exitCode -ne 0) {
    Say ('BUILD FAILED (cargo exit ' + $exitCode + ') after ' + $seconds + ' s')
    Show-Tail $outLog $tail
    Show-Tail $errLog $tail
    Exit-Pc 1
}
Show-Tail $errLog ([Math]::Min($tail, 12))
Say ('built ' + $commit + ' in ' + $seconds + ' s; executables in ' + $artifacts)
Say 'next: tools/mac/pc deploy'
Exit-Pc 0
