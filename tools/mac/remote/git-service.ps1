# SPDX-License-Identifier: GPL-3.0-or-later
# Git transport for "pc sync": tools/mac/pc acts as git's ssh program (GIT_SSH_VARIANT=simple)
# and sends this file as -EncodedCommand after replacing GIT_SERVICE and GIT_REPO with checked
# values (receive-pack or upload-pack; C:/kairo/src). It replaces git's own remote command
# "git-receive-pack 'C:/kairo/src'", which the PC's shell would have to parse: Git for Windows
# keeps git-receive-pack off PATH, and quoting through Win32-OpenSSH is fragile. git inherits
# this process's stdin and stdout, so the pack data passes untouched. Its stderr goes to a log
# file instead, so that nothing can mix text into the pack stream.
#
# This runs elevated and without stub.ps1, so it resets the environment the same way first: your
# user variables back to the machine's, the machine PATH only, and the admin-only Git settings in
# C:\kairo\tools\gitconfig instead of your .gitconfig (which could make elevated git run a command).
# Comment lines are dropped before encoding: cmd.exe allows 8191 characters.
$ErrorActionPreference = 'Stop'
$keep = @('TEMP', 'TMP', 'USERPROFILE', 'APPDATA', 'LOCALAPPDATA', 'HOMEDRIVE', 'HOMEPATH', 'USERNAME', 'USERDOMAIN')
foreach ($name in @([Environment]::GetEnvironmentVariables('User').Keys)) {
    if ($keep -notcontains $name) { [Environment]::SetEnvironmentVariable($name, [Environment]::GetEnvironmentVariable($name, 'Machine'), 'Process') }
}
$env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine')
$env:PSModulePath = @((Join-Path $env:ProgramFiles 'PowerShell\Modules'), (Join-Path $PSHOME 'Modules'), (Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\Modules')) -join ';'
$env:GIT_CONFIG_GLOBAL = 'C:\kairo\tools\gitconfig'
# Git for Windows' machine install (as in lib.ps1); the machine PATH only as a fallback.
$git = Join-Path $env:ProgramFiles 'Git\cmd\git.exe'
if (-not [IO.File]::Exists($git)) {
    $git = Get-Command -Name 'git.exe' -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty Source
}
if (-not $git -or -not [IO.File]::Exists($git)) {
    [Console]::Error.WriteLine('pc: git is not installed on the PC')
    exit 1
}
[void][IO.Directory]::CreateDirectory('C:\kairo\logs')
$ErrorActionPreference = 'Continue'
& $git 'GIT_SERVICE' 'GIT_REPO' 2>> 'C:\kairo\logs\git-transport.log'
exit $LASTEXITCODE
