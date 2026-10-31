# SPDX-License-Identifier: GPL-3.0-or-later
# Git transport for "pc sync": tools/mac/pc acts as git's ssh program (GIT_SSH_VARIANT=simple)
# and sends this file as -EncodedCommand after replacing GIT_SERVICE and GIT_REPO with checked
# values (receive-pack or upload-pack; C:/kairo/src). It replaces git's own remote command
# "git-receive-pack 'C:/kairo/src'", which the PC's shell would have to parse: Git for Windows
# keeps git-receive-pack off PATH, and quoting through Win32-OpenSSH is fragile. git inherits
# this process's stdin and stdout, so the pack data passes untouched. Its stderr goes to a log
# file in the admin-only C:\kairo\jobs instead, so that nothing can mix text into the pack stream
# and nothing elevated writes where your account could plant a link.
#
# This runs elevated and without stub.ps1, so it resets the environment the same way first: your
# user variables back to the machine's, the machine PATH only, and the admin-only Git settings in
# C:\kairo\tools\gitconfig instead of your .gitconfig (which could make elevated git run a command).
# Comment lines are dropped before encoding: cmd.exe allows 8191 characters.
$ErrorActionPreference = 'Stop'
# No command runs before PSModulePath names admin-only folders: PowerShell 7 lists your
# Documents\PowerShell\Modules first and loads a module from there on a command's first use.
# Until then only .NET runs, and folders come from .NET, not from variables you can set.
$keep = @('TEMP', 'TMP', 'USERPROFILE', 'APPDATA', 'LOCALAPPDATA', 'HOMEDRIVE', 'HOMEPATH', 'USERNAME', 'USERDOMAIN')
foreach ($name in @([Environment]::GetEnvironmentVariables('User').Keys)) {
    if ($keep -notcontains $name) { [Environment]::SetEnvironmentVariable($name, [Environment]::GetEnvironmentVariable($name, 'Machine'), 'Process') }
}
$env:SystemRoot = [Environment]::GetFolderPath('Windows')
$env:SystemDrive = [IO.Path]::GetPathRoot($env:SystemRoot).TrimEnd('\')
$env:ProgramFiles = [Environment]::GetFolderPath('ProgramFiles')
$env:PSModulePath = [string]::Join(';', @([IO.Path]::Combine($env:ProgramFiles, 'PowerShell', 'Modules'), [IO.Path]::Combine($PSHOME, 'Modules'), [IO.Path]::Combine([Environment]::SystemDirectory, 'WindowsPowerShell', 'v1.0', 'Modules')))
$env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine')
$env:GIT_CONFIG_GLOBAL = 'C:\kairo\tools\gitconfig'
# Git's other per-user files (~/.config/git/attributes and ignore) come from an admin-only folder.
$env:HOME = 'C:\kairo\tools\home'
$env:XDG_CONFIG_HOME = 'C:\kairo\tools\home'
# Git for Windows' machine install (as in lib.ps1), and nothing else: the machine PATH can hold
# folders your account writes.
$git = [IO.Path]::Combine($env:ProgramFiles, 'Git', 'cmd', 'git.exe')
if (-not [IO.File]::Exists($git)) {
    [Console]::Error.WriteLine('pc: Git for Windows is missing from Program Files on the PC')
    exit 1
}
$ErrorActionPreference = 'Continue'
& $git 'GIT_SERVICE' 'GIT_REPO' 2>> 'C:\kairo\jobs\git-transport.log'
exit $LASTEXITCODE
