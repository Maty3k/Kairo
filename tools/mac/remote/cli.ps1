# SPDX-License-Identifier: GPL-3.0-or-later
# pc cli ping|keys [--pipe NAME]: run C:\kairo\bin\current\kairo-cli.exe from the SSH session.
# It talks to kairo-hello (and later the engine) over its named pipe in the owner's session, which
# is what M0 checks: the pipe's DACL lets the same user in from SSH. Refuses while busy unless --force.

Assert-PcIdle 'cli'

$cli = Join-Path $Kairo.Current 'kairo-cli.exe'
if (-not [IO.File]::Exists($cli)) {
    Fail ('there is no ' + $cli + ' yet: build and deploy first (tools/mac/pc build, then tools/mac/pc deploy)')
}
$arguments = @($Pc['CliArgs'] | Where-Object { $null -ne $_ })
$result = Invoke-Native -Exe $cli -Arguments $arguments
if ($result.Text) { Say-Indented $result.Text }
if ($result.Code -ne 0) { Say ('kairo-cli exited with code ' + $result.Code) }
$code = 1
if ($result.Code -eq 0) { $code = 0 }
Exit-Pc $code
