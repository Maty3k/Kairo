# SPDX-License-Identifier: GPL-3.0-or-later
# pc ffmpeg: put the FFmpeg build that tools/pc/bootstrap.ps1 pins into C:\kairo\tools\ffmpeg with the
# bootstrap's own steps (Install-Ffmpeg: curl.exe, the SHA-256 check, the zip's layout, KAIRO-PIN.json),
# so a new pin needs no bootstrap re-run (docs/milestones/M1.md, Carried over from M0). The Mac reads the
# pin from its copy of bootstrap.ps1. Everything happens in C:\kairo\tools, which only Administrators can
# change; the new folder is swapped in whole, so a failure leaves the old one in place. Then pc deploy
# copies its DLLs beside kairo-verify.exe. Refuses while the PC is busy or a \Kairo\Run job is running
# (a build reads the folder's lib\ and include\, a job may run its ffmpeg.exe), unless --force.

Assert-PcIdle 'ffmpeg'
if ((Test-RunTaskBusy) -and -not $Pc['Force']) {
    Fail 'a \Kairo\Run job is running right now, and it may be using FFmpeg; wait for it (tools/mac/pc logs) or re-run with --force' 3
}

$asset = [string]$Pc['FfmpegAsset']
$url = [string]$Pc['FfmpegUrl']
$sha = [string]$Pc['FfmpegSha256']
if (($asset -cnotmatch '^[A-Za-z0-9._-]{1,128}\z') -or ($sha -cnotmatch '^[0-9a-f]{64}\z') -or
    ($url -cnotmatch '^https://github\.com/[A-Za-z0-9._/-]{1,400}\z') -or -not $url.EndsWith('/' + $asset + '.zip', [StringComparison]::Ordinal)) {
    Fail 'internal: the Mac sent no usable FFmpeg pin' 2
}

$dir = $Kairo.Ffmpeg
$old = Read-JsonFile (Join-Path $dir 'KAIRO-PIN.json')
if (([string](Get-Field $old 'sha256') -ceq $sha) -and [IO.File]::Exists((Join-Path $dir 'bin\ffmpeg.exe'))) {
    Say ('FFmpeg is already the pinned build: ' + $asset)
    Exit-Pc 0
}
$was = 'none'
if ($null -ne $old) {
    $was = [string](Get-Field $old 'asset')
} elseif ([IO.Directory]::Exists($dir)) {
    $was = 'a copy without KAIRO-PIN.json'
}

# The bootstrap's own work folder. Anything an earlier, interrupted run left there goes first.
$work = Join-Path $Kairo.Tools '_download'
function Remove-Work {
    if ([IO.Directory]::Exists($work)) { Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue }
}
Remove-Work
[void](New-Item -ItemType Directory -Path $work)

$zip = Join-Path $work ($asset + '.zip')
Say ('downloading ' + $asset + ' (about 80 MB)')
$curl = Join-Path $env:SystemRoot 'System32\curl.exe'
# --disable must stay the first argument: otherwise curl reads a .curlrc or _curlrc from %USERPROFILE% or
# %APPDATA% (stub.ps1 keeps both), which any program running as the owner can write, and this session
# would carry out its options (another URL, another output file) with Administrators' rights.
$r = Invoke-Native -Exe $curl -Arguments @('--disable', '--fail', '--location', '--proto', '=https', '--silent', '--show-error',
    '--retry', '3', '--retry-delay', '5', '--connect-timeout', '30', '--max-time', '900',
    '--user-agent', 'Kairo-pc/1 (+https://github.com/Maty3k/Kairo)', '--output', $zip, $url)
if ($r.Code -ne 0) {
    Remove-Work
    Fail ('the download failed (curl exit ' + $r.Code + '): ' + $r.Text)
}
$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash.ToLowerInvariant()
if ($hash -cne $sha) {
    Remove-Work
    Fail ('the download''s SHA-256 is ' + $hash + ', not the pin''s; it was deleted and nothing changed')
}

# .NET refuses entries that would land outside the folder.
$extract = Join-Path $work 'extract'
try {
    Add-Type -AssemblyName System.IO.Compression.ZipFile
    [IO.Compression.ZipFile]::ExtractToDirectory($zip, $extract)
} catch {
    Remove-Work
    Fail ('the zip could not be unpacked: ' + $_.Exception.Message)
}
$top = @(Get-ChildItem -LiteralPath $extract -Force)
$new = Join-Path $extract $asset
if (($top.Count -ne 1) -or ($top[0].Name -cne $asset) -or -not [IO.File]::Exists((Join-Path $new 'bin\ffmpeg.exe'))) {
    Remove-Work
    Fail ('the zip does not hold one folder ' + $asset + ' with bin\ffmpeg.exe; nothing changed')
}
$stamp = [DateTime]::UtcNow.ToString("yyyy-MM-dd'T'HH:mm:ss'Z'", [Globalization.CultureInfo]::InvariantCulture)
$record = [ordered]@{ asset = $asset; url = $url; sha256 = $sha; installed_utc = $stamp }
Write-TextFileAtomic (Join-Path $new 'KAIRO-PIN.json') ($record | ConvertTo-Json)

# The swap: two renames on one volume. A file still open in the old folder stops the first one.
$aside = Join-Path $work 'previous'
$movedAside = $false
if ([IO.Directory]::Exists($dir)) {
    try {
        [IO.Directory]::Move($dir, $aside)
        $movedAside = $true
    } catch {
        Remove-Work
        Fail ('the old FFmpeg could not be moved aside (is a program still using it?): ' + $_.Exception.Message + '; nothing changed')
    }
}
try {
    [IO.Directory]::Move($new, $dir)
} catch {
    $problem = $_.Exception.Message
    if (-not $movedAside) {
        Remove-Work
        Fail ('the new FFmpeg could not be moved into place: ' + $problem + '; nothing was installed (there was no FFmpeg before)')
    }
    try {
        [IO.Directory]::Move($aside, $dir)
    } catch {
        Fail ('the new FFmpeg could not be moved into place (' + $problem + '), nor the old one back from ' + $aside + ': ' + $_.Exception.Message)
    }
    Remove-Work
    Fail ('the new FFmpeg could not be moved into place: ' + $problem + '; the old one is back')
}
# Administrators own it, as everything elevated sessions run or trust (pc doctor runs its ffmpeg.exe).
Set-AdminOwner $dir
Remove-Work
if ([IO.Directory]::Exists($work)) { Say ('note: ' + $work + ' could not be removed yet (the old copy may still be in use)') }

Say ('installed ' + $asset + ' in ' + $dir + ' (was ' + $was + ')')
Say 'next: tools/mac/pc build, then tools/mac/pc deploy: the new build links against this FFmpeg and gets its DLLs (the current build keeps its own)'
Exit-Pc 0
