# SPDX-License-Identifier: GPL-3.0-or-later
# Loader for tools/mac/pc. The Mac sends this file as -EncodedCommand (Base64 of UTF-16LE), so
# no quoting survives or breaks between bash, ssh, the PC's SSH shell (cmd.exe /d /c) and pwsh. It
# reads the real script from stdin as UTF-8 and runs it. Every script ends by printing
# "@@KAIRO-EXIT <code>". Comment lines are dropped before encoding: cmd.exe allows 8191 characters.
#
# First it makes the environment the machine's: the SSH session is elevated, but Win32-OpenSSH
# gives it your user variables (HKCU\Environment), which any program running as you can change.
# So every user variable except the profile folders goes back to its machine value (or away), PATH
# is the machine's alone, modules load only from admin-only folders, and Git reads the admin-only
# C:\kairo\tools\gitconfig instead of your .gitconfig (tools/pc/bootstrap.ps1 writes it). This can't
# reach what pwsh does while it starts, before any script runs (docs/milestones/M1.md, open items).
# git-service.ps1 repeats this, because git's transport runs without this stub.
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
$utf8 = [Text.UTF8Encoding]::new($false)
try { [Console]::OutputEncoding = $utf8 } catch { }
$reader = [IO.StreamReader]::new([Console]::OpenStandardInput(), $utf8)
$source = $reader.ReadToEnd()
$reader.Dispose()
try {
    & ([scriptblock]::Create($source))
    [Console]::Out.WriteLine('@@KAIRO-EXIT 0')
} catch {
    [Console]::Out.WriteLine('error: ' + $_.Exception.Message)
    if ($_.InvocationInfo -and $_.InvocationInfo.Line) {
        [Console]::Out.WriteLine('  at: ' + $_.InvocationInfo.Line.Trim())
    }
    [Console]::Out.WriteLine('@@KAIRO-EXIT 1')
}
