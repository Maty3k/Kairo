# SPDX-License-Identifier: GPL-3.0-or-later
# pc doctor: run tools\pc\doctor.ps1 (the PC-side check of the toolchain, FFmpeg, PresentMon,
# tasks and Smart App Control) and pass on its exit code. By default it runs the copy in
# C:\kairo\src as of the last sync; with --local the Mac sends its working-tree copy, which runs
# from a temporary file. -TestPresentMon and -Compress are passed on when the Mac asks for them.
# The doctor only reads (its PresentMon test skips itself while the owner is busy), so it runs
# even while the PC is busy. Its exit code: 0 green, 1 problems, 2 the doctor itself failed.

$temporary = $null
if ($Pc['LocalB64']) {
    $text = $Utf8NoBom.GetString([Convert]::FromBase64String([string]$Pc['LocalB64']))
    # In C:\kairo\tools, which only administrators can change, under a name nobody can guess and
    # created new, so no other program can swap the file before this elevated session runs it.
    [void](New-Item -ItemType Directory -Force -Path $Kairo.Tools)
    $temporary = Join-Path $Kairo.Tools ('doctor-local-' + [guid]::NewGuid().ToString('N') + '.ps1')
    $bytes = $Utf8NoBom.GetBytes($text)
    $stream = [IO.File]::Open($temporary, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try { $stream.Write($bytes, 0, $bytes.Length) } finally { $stream.Dispose() }
    $doctor = $temporary
    $label = 'the Mac working-tree copy of tools/pc/doctor.ps1'
} else {
    $doctor = Join-Path $Kairo.Src 'tools\pc\doctor.ps1'
    if (-not [IO.File]::Exists($doctor)) {
        Fail ($doctor + ' is not on the PC yet: run tools/mac/pc sync first, or use tools/mac/pc doctor --local')
    }
    $label = $doctor
}
Say ('running ' + $label)

# Run the doctor the way it would run on its own: PowerShell's default error handling and no
# strict mode from lib.ps1.
Set-StrictMode -Off
$ErrorActionPreference = 'Continue'
$doctorArgs = @{}
if ($Pc['TestPresentMon']) { $doctorArgs['TestPresentMon'] = $true }
if ($Pc['Compress']) { $doctorArgs['Compress'] = $true }
$global:LASTEXITCODE = 0
$crashed = $false
try {
    & $doctor @doctorArgs
} catch {
    Say ('error: the doctor stopped: ' + $_.Exception.Message)
    $crashed = $true
} finally {
    if ($temporary) { Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue }
}
$code = [int]$LASTEXITCODE
if ($crashed -and $code -eq 0) { $code = 1 }
Exit-Pc $code
