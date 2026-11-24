# SPDX-License-Identifier: GPL-3.0-or-later
#
# tools/pc/bench/tests/regdiff.tests.ps1 - regdiff.ps1 and amd-diff.ps1, the AMD settings diff
# (research 17 sections 5.5 and 6).
# Project: https://github.com/Maty3k/Kairo
#
# Run: pwsh -NoProfile -File tools/pc/bench/tests/regdiff.tests.ps1. It exits 1 if a case fails.
# The name and value rules, the walk, the snapshot reader and the comparison run on synthetic keys
# and snapshots built in code, with regdiff.ps1 dot-sourced under testlib's Set-StrictMode -Version
# 3.0 and $ErrorActionPreference = 'Stop' (as the elevated session runs it); the command line runs
# through pwsh -File with its exit codes. tools/mac/remote/amd-diff.ps1 runs as the PC runs it,
# with lib.ps1's output, busy check and exit replaced by stubs that record, and the live registry
# stood in for by functions appended to regdiff.ps1's text, as the Mac sends it.
# The registry itself is read only on Windows CI. One case writes a temporary key under the
# runner's HKCU (the only registry write in Kairo), reads it through the same functions and
# removes it; it and the live -Snapshot case refuse to run unless GITHUB_ACTIONS is set, and
# elsewhere print a skip line. Synthetic names only (kairotester, KAIRO-PC); no PC data.
#
# Rules: ASCII only; Windows PowerShell 5.1 syntax and .NET Framework 4.x APIs; no Add-Type.

. (Join-Path $PSScriptRoot 'testlib.ps1')
. (Join-Path $BenchDir 'regdiff.ps1')

$script:AmdDiffPath = [IO.Path]::GetFullPath((Join-Path $BenchDir (Join-Path '..' (Join-Path '..' (Join-Path 'mac' (Join-Path 'remote' 'amd-diff.ps1'))))))
$script:Hidden = @('kairotester', 'KAIRO-PC')
$script:PathName = 'C:\Users\kairotester\Videos\Radeon ReLive'
$script:SaveFolder = 'C:\Users\kairotester\Videos'

# --- Fixtures ------------------------------------------------------------------------------------

function Get-RegdiffTestHash([string] $Text) {
    # The first 8 hex digits of the SHA-256 of the text's UTF-8 bytes, computed here on its own.
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $digest = $sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($Text)) } finally { $sha.Dispose() }
    return [BitConverter]::ToString($digest, 0, 4).Replace('-', '').ToLowerInvariant()
}

function Get-RegdiffTestToken([string] $Kind, [string] $Text) {
    return ('<' + $Kind + ' len=' + $Text.Length + ' sha256=' + (Get-RegdiffTestHash $Text) + '>')
}

function New-RegdiffTestKey($Values = $null, $Keys = $null) {
    # A synthetic key (regdiff.ps1, "Keys"): values name -> @(kind, data), keys name -> key or 'denied'.
    $key = [ordered]@{}
    $key['values'] = [ordered]@{}
    $key['keys'] = [ordered]@{}
    if ($null -ne $Values) { $key['values'] = $Values }
    if ($null -ne $Keys) { $key['keys'] = $Keys }
    return $key
}

function New-RegdiffTestAmd {
    # A synthetic HKCU\Software\AMD: DVR's values (one named by a path, one holding a folder), a
    # subkey named after the account, and a subkey that may not be read unless -LockedReadable.
    param([int] $Dvr = 1, [bool] $Desktop = $true, [bool] $Replay = $false, [string] $ModeKind = 'DWord',
        [string] $Save = 'C:\Users\kairotester\Videos', [bool] $LockedReadable = $false)
    $dvrValues = [ordered]@{}
    $dvrValues['DvrEnabled'] = @('DWord', $Dvr)
    if ($Desktop) { $dvrValues['DesktopRecordingEnabled'] = @('DWord', 0) }
    if ($Replay) { $dvrValues['InstantReplayEnabled'] = @('DWord', 1) }
    if ($ModeKind -ceq 'DWord') { $dvrValues['Mode'] = @('DWord', 1) } else { $dvrValues['Mode'] = @('String', '1') }
    $dvrValues['FolderPath'] = @('String', $Save)
    $dvrValues[$script:PathName] = @('DWord', 1)
    $dvrValues['Hotkey'] = @('String', 'Ctrl+Shift+S')
    $keys = [ordered]@{}
    $keys['DVR'] = New-RegdiffTestKey $dvrValues
    $accountValues = [ordered]@{}
    $accountValues['Setting'] = @('DWord', 5)
    $keys['kairotester'] = New-RegdiffTestKey $accountValues
    if ($LockedReadable) {
        $lockedValues = [ordered]@{}
        $lockedValues['Secret'] = @('DWord', 1)
        $keys['Locked'] = New-RegdiffTestKey $lockedValues
    } else {
        $keys['Locked'] = 'denied'
    }
    $rootValues = [ordered]@{}
    $rootValues['Install'] = @('DWord', 1)
    return (New-RegdiffTestKey $rootValues $keys)
}

function New-RegdiffTestScopes($Amd) {
    # The three scopes as the live registry gives them: HKCU's read, HKLM's absent, its twin denied.
    return @(
        (New-RegdiffScope 'HKCU\Software\AMD' $Amd 'read'),
        (New-RegdiffScope 'HKLM\SOFTWARE\AMD' $null 'absent'),
        (New-RegdiffScope 'HKLM\SOFTWARE\WOW6432Node\AMD' $null 'denied')
    )
}

function New-RegdiffTestSnapshot($Amd = $null, [switch] $Driver) {
    if ($null -eq $Amd) { $Amd = New-RegdiffTestAmd }
    return (New-RegdiffSnapshot -Scopes (New-RegdiffTestScopes $Amd) -Hidden $script:Hidden -Driver:$Driver)
}

function New-RegdiffTestFlat([bool] $Cut, [object[]] $Entries, [bool] $Driver = $false) {
    # A snapshot built by hand, for the comparison's edge cases: entries are @(key, state, values).
    $list = [Collections.Generic.List[object]]::new()
    $count = 0
    foreach ($item in $Entries) {
        $entry = [ordered]@{}
        $entry['key'] = $item[0]
        $entry['state'] = $item[1]
        if ($item[1] -ceq 'read') {
            $entry['values'] = $item[2]
            $count += $item[2].Count
        }
        $list.Add($entry)
    }
    $snapshot = [ordered]@{}
    $snapshot['kairo_regdiff'] = 1
    $snapshot['driver'] = $Driver
    $snapshot['max_depth'] = 8
    $snapshot['max_values'] = 20000
    $snapshot['value_count'] = $count
    $snapshot['cut'] = $Cut
    $snapshot['scopes'] = [string[]]@('HKCU\Software\AMD')
    $snapshot['keys'] = $list.ToArray()
    return $snapshot
}

function New-RegdiffTestValues([string[]] $Pairs) {
    # name=TYPE shown pairs as a key's values.
    $values = [ordered]@{}
    foreach ($pair in $Pairs) {
        $cut = $pair.IndexOf('=')
        $values[$pair.Substring(0, $cut)] = $pair.Substring($cut + 1)
    }
    return $values
}

function Assert-RegdiffTestEntry($Entry, [string] $Key, [string] $State, [string[]] $Values = $null) {
    Assert-Equal $Key $Entry['key'] 'key'
    Assert-Equal $State $Entry['state'] ($Key + ' state')
    if ($State -cne 'read') {
        Assert-True (-not $Entry.Contains('values')) ($Key + ' has no values')
        return
    }
    $actual = [Collections.Generic.List[string]]::new()
    foreach ($pair in $Entry['values'].GetEnumerator()) { $actual.Add([string]$pair.Key + '=' + [string]$pair.Value) }
    Assert-Equal (@($Values) -join ' | ') ($actual -join ' | ') ($Key + ' values')
}

function Get-RegdiffTestError([scriptblock] $Body) {
    # The exit code and message of the error $Body must throw.
    $record = Assert-Throws $Body 'an error'
    return [pscustomobject]@{ Code = (Get-RegdiffExitCode $record); Message = $record.Exception.Message }
}

function Test-RegdiffTestCi {
    return ((Test-RegdiffWindows) -and -not [string]::IsNullOrEmpty($env:GITHUB_ACTIONS))
}

# --- The plain-text test and the shown forms --------------------------------------------------------

Test-Case 'plain: the pattern, at most 64 characters, no \ / @, never a hidden name in any case' {
    foreach ($text in @('', 'DvrEnabled', 'Main3D_DEF', 'a b.c,d+e=f#g:h;i(j)k[l]m-n', ('a' * 64), 'Ctrl+Shift+S')) {
        Assert-True (Test-RegdiffPlain $text $script:Hidden) ('plain: ' + $text)
    }
    $fails = @(('a' * 65), 'C:\Videos', 'a/b', 'x@y', ('caf' + [char]0xE9), ('a' + [char]9 + 'b'), ('abc' + [char]10),
        '%USERPROFILE%', 'a"b', 'a<b', 'a{b}', 'xKairoTesterx', 'kairo-pc', 'my KAIRO-PC.local')
    foreach ($text in $fails) { Assert-True (-not (Test-RegdiffPlain $text $script:Hidden)) ('not plain: ' + $text) }
    Assert-True (Test-RegdiffPlain 'kairotester' $null) 'without hidden names the account name is plain text'
}

Test-Case 'hashes: SHA-256 of the UTF-8 bytes, its first 8 hex digits (FIPS 180-2 vectors)' {
    Assert-Equal 'ba7816bf' (Get-RegdiffHash ([Text.Encoding]::UTF8.GetBytes('abc'))) 'abc'
    Assert-Equal 'e3b0c442' (Get-RegdiffHash ([byte[]]@())) 'nothing'
    Assert-Equal 'ba7816bf' (Get-RegdiffTestHash 'abc') 'the tests'' own hash'
}

Test-Case 'names: plain ones as they are, <default>, and <name len=N sha256=8hex> for the rest' {
    Assert-Equal 'DvrEnabled' (Format-RegdiffName 'DvrEnabled' $script:Hidden) 'plain'
    Assert-Equal '<default>' (Format-RegdiffName '' $script:Hidden) 'the unnamed value'
    Assert-Equal ('<name len=41 sha256=' + (Get-RegdiffTestHash $script:PathName) + '>') (Format-RegdiffName $script:PathName $script:Hidden) 'a path'
    Assert-Equal (Get-RegdiffTestToken 'name' 'kairotester') (Format-RegdiffName 'kairotester' $script:Hidden) 'the account'
    Assert-Equal (Get-RegdiffTestToken 'name' 'KAIRO-PC') (Format-RegdiffName 'KAIRO-PC' $script:Hidden) 'the PC'
    Assert-Equal '<name len=3 sha256=ba7816bf>' (Format-RegdiffName 'abc' @('ABC')) 'a hidden name, in another case'
    Assert-Equal (Get-RegdiffTestToken 'name' '{4d36e968-e325-11ce-bfc1-08002be10318}') (Format-RegdiffName '{4d36e968-e325-11ce-bfc1-08002be10318}' $null) 'braces are not plain'
}

Test-Case 'values: numbers unsigned, plain strings quoted, everything else hashed, with its type' {
    $h = $script:Hidden
    Assert-Equal 'REG_DWORD 1' (Format-RegdiffValue 'DWord' 1 $h) 'DWORD'
    Assert-Equal 'REG_DWORD 4294967295' (Format-RegdiffValue 'DWord' (-1) $h) 'DWORD all ones'
    Assert-Equal 'REG_QWORD 5' (Format-RegdiffValue 'QWord' ([long]5) $h) 'QWORD'
    Assert-Equal 'REG_QWORD 18446744073709551614' (Format-RegdiffValue 'QWord' ([long]-2) $h) 'QWORD unsigned'
    Assert-Equal 'REG_SZ "Radeon ReLive"' (Format-RegdiffValue 'String' 'Radeon ReLive' $h) 'plain string'
    Assert-Equal 'REG_SZ ""' (Format-RegdiffValue 'String' '' $h) 'empty string'
    Assert-Equal ('REG_SZ ' + (Get-RegdiffTestToken 'str' $script:SaveFolder)) (Format-RegdiffValue 'String' $script:SaveFolder $h) 'a folder'
    Assert-Equal ('REG_SZ ' + (Get-RegdiffTestToken 'str' ('x' * 65))) (Format-RegdiffValue 'String' ('x' * 65) $h) '65 characters'
    Assert-Equal ('REG_SZ ' + (Get-RegdiffTestToken 'str' 'on KAIRO-PC')) (Format-RegdiffValue 'String' 'on KAIRO-PC' $h) 'the PC'
    # REG_EXPAND_SZ arrives unexpanded (DoNotExpandEnvironmentNames), and is hashed as it is.
    Assert-Equal ('REG_EXPAND_SZ <str len=20 sha256=' + (Get-RegdiffTestHash '%USERPROFILE%\Videos') + '>') (Format-RegdiffValue 'ExpandString' '%USERPROFILE%\Videos' $h) 'expand string'
    Assert-Equal 'REG_MULTI_SZ ["a","b c"]' (Format-RegdiffValue 'MultiString' ([string[]]@('a', 'b c')) $h) 'plain list'
    Assert-Equal 'REG_MULTI_SZ []' (Format-RegdiffValue 'MultiString' ([string[]]@()) $h) 'empty list'
    $nul = 'a' + [char]0 + 'C:\x'
    Assert-Equal ('REG_MULTI_SZ <str len=6 sha256=' + (Get-RegdiffTestHash $nul) + '>') (Format-RegdiffValue 'MultiString' ([string[]]@('a', 'C:\x')) $h) 'a list with a path, joined by NUL'
    Assert-Equal ('REG_MULTI_SZ ' + (Get-RegdiffTestToken 'str' ((@('abcdefgh') * 8) -join [string][char]0))) (Format-RegdiffValue 'MultiString' ([string[]](@('abcdefgh') * 8)) $h) 'more than 64 characters with the commas'
    Assert-Equal 'REG_BINARY <bin len=3 sha256=ba7816bf>' (Format-RegdiffValue 'Binary' ([byte[]](0x61, 0x62, 0x63)) $h) 'binary'
    Assert-Equal 'REG_NONE <bin len=0 sha256=e3b0c442>' (Format-RegdiffValue 'None' ([byte[]]@()) $h) 'REG_NONE'
    Assert-Equal 'REG_UNKNOWN <no data>' (Format-RegdiffValue 'Unknown' $null $h) 'REG_LINK and the like'
    Assert-Match '^REG_DWORD <bin len=6 sha256=[0-9a-f]{8}>\z' (Format-RegdiffValue 'DWord' ([byte[]](1, 2, 3, 4, 5, 6)) $h) 'an oversized DWORD, which .NET gives as bytes'
    $e = Get-RegdiffTestError { Format-RegdiffValue 'DWord' 1.5 $h }
    Assert-Equal 1 $e.Code 'an unexpected .NET type'
}

# --- The walk ---------------------------------------------------------------------------------------

Test-Case 'walk: a path-named value, an account-named subkey, a <denied> subkey, absent and denied scopes' {
    $s = New-RegdiffTestSnapshot
    Assert-Equal 1 $s['kairo_regdiff'] 'version'
    Assert-Equal $false $s['driver'] 'driver'
    Assert-Equal 8 $s['max_depth'] 'max_depth'
    Assert-Equal 20000 $s['max_values'] 'max_values'
    Assert-Equal 8 $s['value_count'] 'value_count'
    Assert-Equal $false $s['cut'] 'cut'
    Assert-Equal 'HKCU\Software\AMD,HKLM\SOFTWARE\AMD,HKLM\SOFTWARE\WOW6432Node\AMD' ($s['scopes'] -join ',') 'scopes'
    $keys = @($s['keys'])
    Assert-Equal 6 $keys.Count 'entries'
    $account = Get-RegdiffTestToken 'name' 'kairotester'
    Assert-RegdiffTestEntry $keys[0] 'HKCU\Software\AMD' 'read' @('Install=REG_DWORD 1')
    # Subkeys in the order of their shown names: '<' sorts before letters.
    Assert-RegdiffTestEntry $keys[1] ('HKCU\Software\AMD\' + $account) 'read' @('Setting=REG_DWORD 5')
    Assert-RegdiffTestEntry $keys[2] 'HKCU\Software\AMD\DVR' 'read' @(
        ((Get-RegdiffTestToken 'name' $script:PathName) + '=REG_DWORD 1'),
        'DesktopRecordingEnabled=REG_DWORD 0',
        'DvrEnabled=REG_DWORD 1',
        ('FolderPath=REG_SZ ' + (Get-RegdiffTestToken 'str' $script:SaveFolder)),
        'Hotkey=REG_SZ "Ctrl+Shift+S"',
        'Mode=REG_DWORD 1')
    Assert-RegdiffTestEntry $keys[3] 'HKCU\Software\AMD\Locked' 'denied'
    Assert-RegdiffTestEntry $keys[4] 'HKLM\SOFTWARE\AMD' 'absent'
    Assert-RegdiffTestEntry $keys[5] 'HKLM\SOFTWARE\WOW6432Node\AMD' 'denied'
}

Test-Case 'walk: keys more than 8 levels down are <past depth 8>, and nothing below them is opened' {
    $leaf = New-RegdiffTestKey ([ordered]@{ Deepest = @('DWord', 1) })
    $chain = $leaf
    for ($level = 10; $level -ge 1; $level--) {
        $keys = [ordered]@{}
        $keys['k' + $level] = $chain
        $chain = New-RegdiffTestKey ([ordered]@{ Level = @('DWord', ($level - 1)) }) $keys
    }
    $s = New-RegdiffSnapshot -Scopes @((New-RegdiffScope 'HKLM\SOFTWARE\AMD' $chain 'read')) -Hidden $script:Hidden
    $keys = @($s['keys'])
    Assert-Equal 10 $keys.Count 'the root and 8 levels read, then one deep key'
    $label = 'HKLM\SOFTWARE\AMD'
    for ($level = 0; $level -le 8; $level++) {
        if ($level -gt 0) { $label += '\k' + $level }
        Assert-RegdiffTestEntry $keys[$level] $label 'read' @('Level=REG_DWORD ' + $level)
    }
    Assert-RegdiffTestEntry $keys[9] ($label + '\k9') 'deep'
    Assert-Equal 9 $s['value_count'] 'values read'
    Assert-Equal $false $s['cut'] 'the depth limit is not a cut'
}

Test-Case 'walk: past 20,000 values it stops, at the key that would pass the limit, and reads nothing more' {
    $many = [ordered]@{}
    for ($i = 0; $i -lt 20001; $i++) { $many['V' + $i] = @('DWord', 1) }
    $big = New-RegdiffTestKey $many
    $s = New-RegdiffSnapshot -Scopes @((New-RegdiffScope 'HKCU\Software\AMD' $big 'read'), (New-RegdiffScope 'HKLM\SOFTWARE\AMD' (New-RegdiffTestAmd) 'read')) -Hidden $script:Hidden
    Assert-Equal $true $s['cut'] 'cut'
    Assert-Equal 0 $s['value_count'] 'no value of the key that passes the limit'
    Assert-Equal 1 @($s['keys']).Count 'one entry'
    Assert-RegdiffTestEntry $s['keys'][0] 'HKCU\Software\AMD' 'cut'
    Assert-Equal 'HKCU\Software\AMD,HKLM\SOFTWARE\AMD' ($s['scopes'] -join ',') 'both scopes listed, the second unread'
    Assert-Equal 'snapshot: 0 values in 0 keys|  HKCU\Software\AMD: 0 values in 0 keys, stopped at 20000 values|  HKLM\SOFTWARE\AMD: not read (the walk had stopped)' ((Format-RegdiffSummary $s) -join '|') 'summary'
}

Test-Case 'walk: exactly 20,000 values are read; the next key is where it stops' {
    $many = [ordered]@{}
    for ($i = 0; $i -lt 20000; $i++) { $many['V' + $i] = @('DWord', 1) }
    $keys = [ordered]@{}
    $keys['Next'] = New-RegdiffTestKey ([ordered]@{ One = @('DWord', 1) })
    $s = New-RegdiffSnapshot -Scopes @((New-RegdiffScope 'HKCU\Software\AMD' (New-RegdiffTestKey $many $keys) 'read')) -Hidden $null
    Assert-Equal 20000 $s['value_count'] 'values read'
    Assert-Equal $true $s['cut'] 'cut'
    Assert-Equal 2 @($s['keys']).Count 'entries'
    Assert-Equal 20000 $s['keys'][0]['values'].Count 'the first key whole'
    Assert-RegdiffTestEntry $s['keys'][1] 'HKCU\Software\AMD\Next' 'cut'
    $lines = (Compare-RegdiffSnapshot $s $s).Lines
    Assert-Equal 'HKCU\Software\AMD\Next: <stopped at 20000 values>|0 values changed' ($lines -join '|') 'the stop shows in the lines'
}

Test-Case 'snapshot JSON: one ASCII line, read back field for field, with no hidden name or path in it' {
    $s = New-RegdiffTestSnapshot
    $json = ConvertTo-RegdiffJson $s
    Assert-True ($json.IndexOf([char]10) -lt 0) 'one line'
    foreach ($c in $json.ToCharArray()) { if ([int]$c -gt 126 -or [int]$c -lt 32) { throw ('not printable ASCII: ' + [int]$c) } }
    foreach ($secret in @('kairotester', 'KAIRO-PC', 'Users', 'C:', 'Videos')) {
        Assert-True ($json.IndexOf($secret, [StringComparison]::OrdinalIgnoreCase) -lt 0) ('the JSON holds ' + $secret)
    }
    Assert-Match '^\{"kairo_regdiff":1,"driver":false,"max_depth":8,"max_values":20000,"value_count":8,"cut":false,"scopes":\["HKCU\\\\Software\\\\AMD",' $json 'its head'
    $back = Read-RegdiffSnapshot $json 'before.json'
    Assert-Equal $json (ConvertTo-RegdiffJson $back) 'read back and written again'
    $d = Compare-RegdiffSnapshot $s $back
    Assert-Equal 0 $d.Changed 'no change against itself'
}

Test-Case 'snapshot JSON: names that look like dates stay names (ConvertFrom-Json would turn values into dates)' {
    $values = [ordered]@{}
    $values['2026-11-24T20:15:03'] = @('String', '2026-11-24T20:15:03')
    $s = New-RegdiffSnapshot -Scopes @((New-RegdiffScope 'HKCU\Software\AMD' (New-RegdiffTestKey $values) 'read')) -Hidden $null
    $back = Read-RegdiffSnapshot (ConvertTo-RegdiffJson $s)
    Assert-RegdiffTestEntry $back['keys'][0] 'HKCU\Software\AMD' 'read' @('2026-11-24T20:15:03=REG_SZ "2026-11-24T20:15:03"')
}

Test-Case 'reader: refuses anything but the shown forms, exit 2, naming the file and never the text' {
    $cases = @(
        @('not JSON', { param($s) }, 'nope'),
        @('a list', { param($s) }, '[1,2]'),
        @('another version', { param($s) $s['kairo_regdiff'] = 2 }, $null),
        @('a SID in a key', { param($s) $s['keys'][0]['key'] = 'HKEY_USERS\S-1-5-21-1-2-3-1001\Software\AMD' }, $null),
        @('a SID in a scope', { param($s) $s['scopes'][0] = 'HKU\S-1-5-21-1-2-3-1001' }, $null),
        @('a path as a name', { param($s) $s['keys'][2]['values']['C:\Users\x'] = 'REG_DWORD 1'; $s['value_count'] = 9 }, $null),
        @('a path as data', { param($s) $s['keys'][2]['values']['Hotkey'] = 'REG_SZ "C:\Users\x"' }, $null),
        @('free text as data', { param($s) $s['keys'][2]['values']['Hotkey'] = 'set by S-1-5-21-1-2-3-1001' }, $null),
        @('a number as data', { param($s) $s['keys'][2]['values']['Hotkey'] = 5 }, $null),
        @('a wrong count', { param($s) $s['value_count'] = 7 }, $null),
        @('an unknown state', { param($s) $s['keys'][3]['state'] = 'hidden' }, $null),
        @('a read key without values', { param($s) $s['keys'][0].Remove('values') }, $null),
        @('a denied key with values', { param($s) $s['keys'][3]['values'] = [ordered]@{} }, $null),
        @('a flag that is text', { param($s) $s['driver'] = 'yes' }, $null),
        @('a depth of 0', { param($s) $s['max_depth'] = 0 }, $null)
    )
    foreach ($case in $cases) {
        $text = $case[2]
        if ($null -eq $text) {
            $s = New-RegdiffTestSnapshot
            & $case[1] $s
            $text = ConvertTo-RegdiffJson $s
        }
        $e = Get-RegdiffTestError { Read-RegdiffSnapshot $text 'before.json' }
        Assert-Equal 2 $e.Code ($case[0] + ': exit code')
        Assert-Match '^before\.json isn''t a regdiff snapshot: ' $e.Message ($case[0] + ': message')
        foreach ($secret in @('S-1-5', 'C:\', 'Users')) { Assert-True ($e.Message.IndexOf($secret) -lt 0) ($case[0] + ': the message quotes ' + $secret) }
    }
    $driver = Read-RegdiffSnapshot (ConvertTo-RegdiffJson (New-RegdiffTestSnapshot -Driver))
    Assert-Equal $true $driver['driver'] 'a driver snapshot reads'
    $labels = New-RegdiffTestFlat $false @(, @('HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\0001[VEN_1002&DEV_744C]', 'read', (New-RegdiffTestValues @('x=REG_DWORD 1'))))
    Assert-Equal 1 (Read-RegdiffSnapshot (ConvertTo-RegdiffJson $labels))['value_count'] 'a display-class key''s label reads'
}

# --- The comparison ---------------------------------------------------------------------------------

Test-Case 'compare: section 5.5''s lines, sorted by path, then the count' {
    $before = New-RegdiffTestSnapshot
    $after = New-RegdiffTestSnapshot (New-RegdiffTestAmd -Dvr 0 -Desktop $false -Replay $true -ModeKind 'String' -Save 'D:\Captures')
    $d = Compare-RegdiffSnapshot $before $after
    $expected = @(
        'HKCU\Software\AMD\DVR\DesktopRecordingEnabled: 0 -> (absent)',
        'HKCU\Software\AMD\DVR\DvrEnabled: 1 -> 0',
        ('HKCU\Software\AMD\DVR\FolderPath: ' + (Get-RegdiffTestToken 'str' $script:SaveFolder) + ' -> ' + (Get-RegdiffTestToken 'str' 'D:\Captures')),
        'HKCU\Software\AMD\DVR\InstantReplayEnabled: (absent) -> 1',
        'HKCU\Software\AMD\DVR\Mode: REG_DWORD 1 -> REG_SZ "1"',
        'HKCU\Software\AMD\Locked: <denied>',
        'HKLM\SOFTWARE\WOW6432Node\AMD: <denied>',
        '5 values changed'
    )
    Assert-Equal ($expected -join [char]10) ($d.Lines -join [char]10) 'lines'
    Assert-Equal 5 $d.Changed 'Changed'
}

Test-Case 'compare: M1''s examples word for word' {
    $old = New-RegdiffTestFlat $false @(, @('HKCU\Software\AMD\DVR', 'read', (New-RegdiffTestValues @('DvrEnabled=REG_DWORD 1', 'Gone=REG_DWORD 0', 'Typed=REG_DWORD 1'))))
    $new = New-RegdiffTestFlat $false @(, @('HKCU\Software\AMD\DVR', 'read', (New-RegdiffTestValues @('DvrEnabled=REG_DWORD 0', 'New=REG_DWORD 1', 'Typed=REG_SZ "1"'))))
    $lines = (Compare-RegdiffSnapshot $old $new).Lines
    Assert-Equal 'HKCU\Software\AMD\DVR\DvrEnabled: 1 -> 0|HKCU\Software\AMD\DVR\Gone: 0 -> (absent)|HKCU\Software\AMD\DVR\New: (absent) -> 1|HKCU\Software\AMD\DVR\Typed: REG_DWORD 1 -> REG_SZ "1"|4 values changed' ($lines -join '|') 'lines'
    Assert-Equal '0 values changed' ((Compare-RegdiffSnapshot $old $old).Lines -join '|') 'nothing changed'
}

Test-Case 'compare: a key either snapshot didn''t read hides its values, and shows once' {
    $before = New-RegdiffTestSnapshot
    $after = New-RegdiffTestSnapshot (New-RegdiffTestAmd -LockedReadable $true)
    $lines = (Compare-RegdiffSnapshot $before $after).Lines
    Assert-Equal 'HKCU\Software\AMD\Locked: <denied>|HKLM\SOFTWARE\WOW6432Node\AMD: <denied>|0 values changed' ($lines -join '|') 'denied before, read after'
    $lines = (Compare-RegdiffSnapshot $after $before).Lines
    Assert-Equal 'HKCU\Software\AMD\Locked: <denied>|HKLM\SOFTWARE\WOW6432Node\AMD: <denied>|0 values changed' ($lines -join '|') 'read before, denied after'
    $old = New-RegdiffTestFlat $false @(@('HKCU\Software\AMD', 'read', (New-RegdiffTestValues @('A=REG_DWORD 1'))), @('HKCU\Software\AMD\a\b', 'deep', $null))
    $new = New-RegdiffTestFlat $false @(@('HKCU\Software\AMD', 'read', (New-RegdiffTestValues @('A=REG_DWORD 1'))), @('HKCU\Software\AMD\a\b', 'read', (New-RegdiffTestValues @('Below=REG_DWORD 1'))))
    Assert-Equal 'HKCU\Software\AMD\a\b: <past depth 8>|0 values changed' ((Compare-RegdiffSnapshot $old $new).Lines -join '|') 'past the depth limit'
}

Test-Case 'compare: after a stop, keys the stopped snapshot never reached are left out' {
    $old = New-RegdiffTestFlat $true @(@('HKCU\Software\AMD', 'read', (New-RegdiffTestValues @('A=REG_DWORD 1'))), @('HKCU\Software\AMD\B', 'cut', $null))
    $new = New-RegdiffTestFlat $false @(
        @('HKCU\Software\AMD', 'read', (New-RegdiffTestValues @('A=REG_DWORD 2'))),
        @('HKCU\Software\AMD\B', 'read', (New-RegdiffTestValues @('X=REG_DWORD 1'))),
        @('HKCU\Software\AMD\C', 'read', (New-RegdiffTestValues @('Y=REG_DWORD 1'))),
        @('HKLM\SOFTWARE\AMD', 'read', (New-RegdiffTestValues @('Z=REG_DWORD 1'))))
    Assert-Equal 'HKCU\Software\AMD\A: 1 -> 2|HKCU\Software\AMD\B: <stopped at 20000 values>|1 values changed' ((Compare-RegdiffSnapshot $old $new).Lines -join '|') 'lines'
}

Test-Case 'compare: names ignore case as the registry does, and print as the after snapshot spells them' {
    $old = New-RegdiffTestFlat $false @(, @('HKCU\Software\AMD\DVR', 'read', (New-RegdiffTestValues @('dvrenabled=REG_DWORD 1'))))
    $new = New-RegdiffTestFlat $false @(, @('HKCU\Software\AMD\dvr', 'read', (New-RegdiffTestValues @('DvrEnabled=REG_DWORD 0'))))
    Assert-Equal 'HKCU\Software\AMD\dvr\DvrEnabled: 1 -> 0|1 values changed' ((Compare-RegdiffSnapshot $old $new).Lines -join '|') 'lines'
}

Test-Case 'compare: refuses snapshots with different scopes or limits, exit 2' {
    $e = Get-RegdiffTestError { Compare-RegdiffSnapshot (New-RegdiffTestSnapshot) (New-RegdiffTestSnapshot -Driver) }
    Assert-Equal 2 $e.Code 'driver'
    Assert-Match 'one was taken with --driver and one without' $e.Message 'message'
    $other = New-RegdiffTestSnapshot
    $other['max_depth'] = 4
    Assert-Equal 2 (Get-RegdiffTestError { Compare-RegdiffSnapshot (New-RegdiffTestSnapshot) $other }).Code 'limits'
}

Test-Case 'summary: counts per scope, and no name or value' {
    $expected = @(
        'snapshot: 8 values in 3 keys',
        '  HKCU\Software\AMD: 8 values in 3 keys, 1 denied',
        '  HKLM\SOFTWARE\AMD: (absent)',
        '  HKLM\SOFTWARE\WOW6432Node\AMD: <denied>'
    )
    Assert-Equal ($expected -join '|') ((Format-RegdiffSummary (New-RegdiffTestSnapshot)) -join '|') 'lines'
    Assert-Match '^snapshot: 8 values in 3 keys, display-class keys included\z' (Format-RegdiffSummary (New-RegdiffTestSnapshot -Driver))[0] 'with --driver'
    $one = New-RegdiffTestFlat $false @(, @('HKCU\Software\AMD', 'read', (New-RegdiffTestValues @('A=REG_DWORD 1'))))
    Assert-Equal 'snapshot: 1 value in 1 key|  HKCU\Software\AMD: 1 value in 1 key' ((Format-RegdiffSummary $one) -join '|') 'one of each'
}

# --- The command line, through pwsh -File ------------------------------------------------------------

$script:CliDir = New-BenchTempDir
$script:CliBefore = Join-Path $script:CliDir 'before.json'
$script:CliAfter = Join-Path $script:CliDir 'after-on.json'
$script:CliDriver = Join-Path $script:CliDir 'after-driver.json'
$script:CliJunk = Join-Path $script:CliDir 'notes.txt'
[IO.File]::WriteAllText($script:CliBefore, (ConvertTo-RegdiffJson (New-RegdiffTestSnapshot)), [Text.UTF8Encoding]::new($true))
[IO.File]::WriteAllText($script:CliAfter, (ConvertTo-RegdiffJson (New-RegdiffTestSnapshot (New-RegdiffTestAmd -Dvr 0))))
[IO.File]::WriteAllText($script:CliDriver, (ConvertTo-RegdiffJson (New-RegdiffTestSnapshot -Driver)))
[IO.File]::WriteAllText($script:CliJunk, 'C:\Users\kairotester said hello')

Test-Case 'command line: -Before and -After print the lines, exit 0 (a byte-order mark allowed)' {
    $run = Invoke-BenchTool 'regdiff.ps1' @('-Before', $script:CliBefore, '-After', $script:CliAfter)
    Assert-Equal 0 $run.ExitCode ('exit code; stderr: ' + $run.StdErr)
    $expected = @('HKCU\Software\AMD\DVR\DvrEnabled: 1 -> 0', 'HKCU\Software\AMD\Locked: <denied>', 'HKLM\SOFTWARE\WOW6432Node\AMD: <denied>', '1 values changed')
    Assert-Equal ($expected -join '|') (($run.StdOut.TrimEnd() -split "`r?`n") -join '|') 'stdout'
    Assert-Equal '' $run.StdErr 'stderr'
    $lower = Invoke-BenchTool 'regdiff.ps1' @('-before', $script:CliBefore, '-AFTER', $script:CliAfter)
    Assert-Equal 0 $lower.ExitCode 'names ignore case'
}

Test-Case 'command line: bad arguments and files exit 2, naming the file and never its folder' {
    $cases = @(
        @(@('-Before', $script:CliBefore), 'missing -After'),
        @(@('-Before', $script:CliBefore, '-After', $script:CliAfter, '-Before', $script:CliBefore), '-Before given twice'),
        @(@('-Before', $script:CliBefore, '-After', $script:CliAfter, '-Driver'), '-Driver goes with -Snapshot'),
        @(@('-Snapshot', '-Before', $script:CliBefore), '-Snapshot takes no -Before'),
        @(@('-Compare'), 'unknown parameter -Compare'),
        @(@($script:CliBefore), 'unexpected argument at position 1'),
        @(@('-Before', (Join-Path $script:CliDir 'missing.json'), '-After', $script:CliAfter), 'no such file: missing.json'),
        @(@('-Before', $script:CliJunk, '-After', $script:CliAfter), 'notes.txt isn''t a regdiff snapshot: not a JSON object'),
        @(@('-Before', $script:CliBefore, '-After', $script:CliDriver), 'one was taken with --driver and one without')
    )
    foreach ($case in $cases) {
        $run = Invoke-BenchTool 'regdiff.ps1' $case[0]
        Assert-Equal 2 $run.ExitCode ($case[1] + ': exit code')
        Assert-True ($run.StdErr.Contains($case[1])) ($case[1] + ': stderr was ' + $run.StdErr)
        Assert-Equal '' $run.StdOut ($case[1] + ': stdout')
        foreach ($secret in @($script:CliDir, 'kairotester')) { Assert-True ($run.StdErr.IndexOf($secret) -lt 0) ($case[1] + ': stderr names ' + $secret) }
    }
}

if (Test-RegdiffWindows) {
    [Console]::Out.WriteLine('skip command line: -Snapshot off Windows (this is Windows; the Windows CI case below reads the registry)')
} else {
    Test-Case 'command line: -Snapshot off Windows exits 1' {
        $run = Invoke-BenchTool 'regdiff.ps1' @('-Snapshot', '-Driver')
        Assert-Equal 1 $run.ExitCode 'exit code'
        Assert-Equal 'regdiff: the registry exists only on Windows' $run.StdErr.Trim() 'stderr'
    }
}

Test-Case 'dot-sourcing defines functions only: no output, no variables, settings untouched' {
    $path = Join-Path $BenchDir 'regdiff.ps1'
    $result = & {
        Set-StrictMode -Off
        $ErrorActionPreference = 'Continue'
        $emitted = @(. $path)
        $leaked = @(Get-Variable -Name 'regdiff*' -Scope 0 -ErrorAction SilentlyContinue)
        # Strict mode is still off: an unset variable reads as $null.
        $unset = $variableThatIsNeverSet
        [pscustomobject]@{
            Emitted  = $emitted.Count
            Leaked   = $leaked.Count
            Eap      = $ErrorActionPreference
            Unset    = ($null -eq $unset)
            Function = ($null -ne (Get-Command -Name 'Compare-RegdiffSnapshot' -CommandType Function -ErrorAction SilentlyContinue))
        }
    }
    Assert-Equal 0 $result.Emitted 'objects emitted'
    Assert-Equal 0 $result.Leaked 'command-line variables'
    Assert-Equal 'Continue' $result.Eap 'ErrorActionPreference'
    Assert-Equal $true $result.Unset 'strict mode left off'
    Assert-Equal $true $result.Function 'Compare-RegdiffSnapshot defined'
}

Test-Case 'dot-sourcing the text through [scriptblock]::Create, as amd-diff.ps1 does' {
    $text = [IO.File]::ReadAllText((Join-Path $BenchDir 'regdiff.ps1'))
    $emitted = @(& { . ([scriptblock]::Create($text)); (Compare-RegdiffSnapshot (New-RegdiffTestSnapshot) (New-RegdiffTestSnapshot)).Lines })
    Assert-Equal 'HKCU\Software\AMD\Locked: <denied>|HKLM\SOFTWARE\WOW6432Node\AMD: <denied>|0 values changed' ($emitted -join '|') 'the functions work'
}

# --- amd-diff.ps1, the verb body -------------------------------------------------------------------

# Stand-ins for the live registry, appended to regdiff.ps1's text as the Mac sends it: the before
# and after snapshots of one toggle (Instant Replay switched on), and a failing hive.
$script:VerbLive = @'
function Get-RegdiffOwnerSid { return 'S-1-5-21-1-2-3-1001' }
function Get-RegdiffSnapshot {
    param([string] $OwnerSid, [switch] $Driver)
    if ($OwnerSid -cne 'S-1-5-21-1-2-3-1001') { throw 'the SID did not arrive' }
    return (New-RegdiffTestSnapshot (New-RegdiffTestAmd -Replay $REPLAY) -Driver:$Driver)
}
'@
$script:VerbBefore = $script:VerbLive.Replace('$REPLAY', '$false')
$script:VerbAfter = $script:VerbLive.Replace('$REPLAY', '$true')
$script:VerbHiveGone = @'
function Get-RegdiffOwnerSid { return 'S-1-5-21-1-2-3-1001' }
function Get-RegdiffSnapshot { param([string] $OwnerSid, [switch] $Driver) throw (New-RegdiffError 1 'the owner''s registry hive can''t be read: is the owner signed in?') }
'@
$script:VerbBug = @'
function Get-RegdiffOwnerSid { return 'S-1-5-21-1-2-3-1001' }
function Get-RegdiffSnapshot { param([string] $OwnerSid, [switch] $Driver) throw ('cannot open HKEY_USERS\' + $OwnerSid + '\Software\AMD') }
'@

function Invoke-RegdiffTestVerb([hashtable] $Prelude, [string] $Live = '') {
    # amd-diff.ps1 run as the PC runs it after lib.ps1, whose Say, Send-KV, Fail, Exit-Pc and
    # Assert-PcIdle are stubs here that record; $Live is appended to regdiff.ps1's text.
    $verbText = [IO.File]::ReadAllText($script:AmdDiffPath)
    $regdiffText = [IO.File]::ReadAllText((Join-Path $BenchDir 'regdiff.ps1')) + "`n" + $Live
    $Pc = @{}
    $Pc['RegdiffB64'] = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($regdiffText))
    foreach ($key in @($Prelude.Keys)) { $Pc[$key] = $Prelude[$key] }
    $Utf8NoBom = [Text.UTF8Encoding]::new($false)
    $said = [Collections.Generic.List[string]]::new()
    $kv = @{}
    $idle = [Collections.Generic.List[string]]::new()
    function Say([string] $Text = '') { $said.Add($Text) }
    function Send-KV([string] $Key, [string] $Value) { $kv[$Key] = $Value }
    function Exit-Pc([int] $Code) {
        $exit = [Exception]::new('Exit-Pc')
        $exit.Data['AmdDiffExit'] = $Code
        throw $exit
    }
    function Fail([string] $Text, [int] $Code = 1) {
        Say ('error: ' + $Text)
        Exit-Pc $Code
    }
    function Assert-PcIdle([string] $Verb, [bool] $FullscreenRefuses = $false) { $idle.Add($Verb + ' ' + $FullscreenRefuses + ' after ' + $said.Count + ' lines') }
    $code = -1
    try {
        & ([scriptblock]::Create($verbText))
    } catch {
        if (-not $_.Exception.Data.Contains('AmdDiffExit')) { throw }
        $code = [int]$_.Exception.Data['AmdDiffExit']
    }
    return [pscustomobject]@{ Lines = $said.ToArray(); KV = $kv; Idle = $idle.ToArray(); Code = $code }
}

function ConvertFrom-RegdiffTestBase64([string] $Text) {
    return [Text.UTF8Encoding]::new($false).GetString([Convert]::FromBase64String($Text))
}

Test-Case 'amd-diff.ps1: before, then after, as two calls: the snapshot travels back, the lines come out' {
    $before = Invoke-RegdiffTestVerb @{ Mode = 'before'; Force = $true; Driver = $false } $script:VerbBefore
    Assert-Equal 0 $before.Code ('before: exit code; said: ' + ($before.Lines -join ' / '))
    Assert-Equal 'amd-diff True after 0 lines' ($before.Idle -join ',') 'the busy check comes first, and a fullscreen window refuses even with --force'
    Assert-Equal ((Format-RegdiffSummary (New-RegdiffTestSnapshot)) -join '|') ($before.Lines -join '|') 'before prints the summary only'
    Assert-Equal 'snapshot' (@($before.KV.Keys) -join ',') 'before returns the snapshot only'
    $json = ConvertFrom-RegdiffTestBase64 $before.KV['snapshot']
    Assert-Equal (ConvertTo-RegdiffJson (New-RegdiffTestSnapshot)) $json 'the snapshot, as JSON'

    $after = Invoke-RegdiffTestVerb @{ Mode = 'after'; Force = $true; Driver = $false; BeforeB64 = $before.KV['snapshot'] } $script:VerbAfter
    Assert-Equal 0 $after.Code ('after: exit code; said: ' + ($after.Lines -join ' / '))
    $diff = @('HKCU\Software\AMD\DVR\InstantReplayEnabled: (absent) -> 1', 'HKCU\Software\AMD\Locked: <denied>', 'HKLM\SOFTWARE\WOW6432Node\AMD: <denied>', '1 values changed')
    $summary = Format-RegdiffSummary (New-RegdiffTestSnapshot (New-RegdiffTestAmd -Replay $true))
    Assert-Equal ((@($summary) + $diff) -join '|') ($after.Lines -join '|') 'after prints its summary, then the lines'
    Assert-Equal (($diff -join "`n") + "`n") (ConvertFrom-RegdiffTestBase64 $after.KV['diff']) 'the lines, for diff-<label>.txt'
    $afterSnapshot = Read-RegdiffSnapshot (ConvertFrom-RegdiffTestBase64 $after.KV['snapshot'])
    Assert-Equal 9 $afterSnapshot['value_count'] 'the after snapshot'
}

Test-Case 'amd-diff.ps1: after takes the before snapshot''s scopes, and --driver needs a before with it' {
    $before = Invoke-RegdiffTestVerb @{ Mode = 'before'; Driver = $true } $script:VerbBefore
    Assert-Equal 0 $before.Code 'before --driver'
    Assert-Match 'display-class keys included' $before.Lines[0] 'the summary says so'
    $after = Invoke-RegdiffTestVerb @{ Mode = 'after'; Driver = $false; BeforeB64 = $before.KV['snapshot'] } $script:VerbAfter
    Assert-Equal 0 $after.Code 'after without --driver'
    Assert-Equal $true (Read-RegdiffSnapshot (ConvertFrom-RegdiffTestBase64 $after.KV['snapshot']))['driver'] 'after read the display-class keys too'
    $plain = Invoke-RegdiffTestVerb @{ Mode = 'before' } $script:VerbBefore
    $refused = Invoke-RegdiffTestVerb @{ Mode = 'after'; Driver = $true; BeforeB64 = $plain.KV['snapshot'] } $script:VerbAfter
    Assert-Equal 2 $refused.Code 'after --driver over a before without it'
    Assert-Equal 'error: the before snapshot was taken without --driver: take a new one with pc amd-diff before --driver' ($refused.Lines -join '|') 'message'
    Assert-Equal 0 $refused.KV.Count 'no snapshot returned'
}

Test-Case 'amd-diff.ps1: refusals and failures: their exit codes, and messages without a SID or path' {
    $cases = @(
        @(@{ Mode = 'after' }, $script:VerbAfter, 2, 'error: no before snapshot: run pc amd-diff before first'),
        @(@{ Mode = 'after'; BeforeB64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes('C:\Users\kairotester')) }, $script:VerbAfter, 2, 'error: the before snapshot isn''t a regdiff snapshot: not a JSON object'),
        @(@{ Mode = 'after'; BeforeB64 = '%%%' }, $script:VerbAfter, 2, 'error: internal error: FormatException at line '),
        @(@{ Mode = 'sideways' }, $script:VerbBefore, 2, 'error: internal: the mode must be before or after'),
        @(@{ Mode = 'before'; RegdiffB64 = '' }, $script:VerbBefore, 1, 'error: internal: the Mac sent no regdiff.ps1'),
        @(@{ Mode = 'before' }, $script:VerbHiveGone, 1, 'error: the owner''s registry hive can''t be read: is the owner signed in?'),
        @(@{ Mode = 'before' }, $script:VerbBug, 1, 'error: internal error: RuntimeException at line '),
        @(@{ Mode = 'before' }, '', 1, 'error: the owner''s account is unknown: \Kairo\Run can''t be read')
    )
    foreach ($case in $cases) {
        $run = Invoke-RegdiffTestVerb $case[0] $case[1]
        Assert-Equal $case[2] $run.Code ($case[3] + ': exit code')
        Assert-Equal 1 $run.Lines.Count ($case[3] + ': one line, said: ' + ($run.Lines -join ' / '))
        Assert-True $run.Lines[0].StartsWith($case[3]) ($case[3] + ': said ' + $run.Lines[0])
        Assert-Equal 0 $run.KV.Count ($case[3] + ': no data returned')
        Assert-Equal 1 $run.Idle.Count ($case[3] + ': the busy check ran first')
        foreach ($secret in @('S-1-5', 'kairotester', 'HKEY_USERS')) { Assert-True ($run.Lines[0].IndexOf($secret) -lt 0) ($case[3] + ': names ' + $secret) }
    }
}

# --- The registry itself: Windows CI only -------------------------------------------------------------

if (Test-RegdiffTestCi) {
    Test-Case 'HKCU (Windows CI): a temporary key read through HKEY_USERS\<SID>, compared, then removed' {
        $view = [Microsoft.Win32.RegistryView]::Registry64
        $name = 'Kairo-regdiff-test-' + [guid]::NewGuid().ToString('N')
        $label = 'HKCU\Software\' + $name
        $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
        $kinds = [Microsoft.Win32.RegistryValueKind]
        $software = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software', $true)
        $deny = $null
        try {
            $test = $software.CreateSubKey($name)
            try {
                $test.SetValue('Dword', 1, $kinds::DWord)
                $test.SetValue('AllOnes', -1, $kinds::DWord)
                $test.SetValue('Qword', [long]5, $kinds::QWord)
                $test.SetValue('Plain', 'Radeon ReLive', $kinds::String)
                $test.SetValue('Folder', 'C:\Users\x\Videos', $kinds::String)
                $test.SetValue('Expand', '%USERPROFILE%\Videos', $kinds::ExpandString)
                $test.SetValue('Multi', [string[]]@('a', 'b'), $kinds::MultiString)
                $test.SetValue('Bytes', [byte[]](0x61, 0x62, 0x63), $kinds::Binary)
                $test.SetValue('', 'default text', $kinds::String)
                $test.SetValue([Environment]::MachineName, 1, $kinds::DWord)
                $account = $test.CreateSubKey([Environment]::UserName)
                $account.SetValue('Inner', 2, $kinds::DWord)
                $account.Dispose()
                $chain = $test.CreateSubKey('k1\k2\k3\k4\k5\k6\k7\k8\k9\k10')
                $chain.SetValue('Deepest', 1, $kinds::DWord)
                $chain.Dispose()
                $locked = $test.CreateSubKey('Locked')
                $locked.SetValue('Hidden', 1, $kinds::DWord)
                # Denying the right to list subkeys makes OpenSubKey(name, $false) fail, while the
                # owner keeps the rights to change the permissions back.
                $deny = [Security.AccessControl.RegistryAccessRule]::new([Security.Principal.SecurityIdentifier]::new($sid),
                    [Security.AccessControl.RegistryRights]::EnumerateSubKeys, [Security.AccessControl.AccessControlType]::Deny)
                $acl = $locked.GetAccessControl()
                $acl.AddAccessRule($deny)
                $locked.SetAccessControl($acl)
                $locked.Dispose()
            } finally {
                $test.Dispose()
            }

            $take = {
                # Opened as pc amd-diff opens the owner's hive: read-only, through HKEY_USERS\<SID>.
                $users = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::Users, $view)
                $machine = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::LocalMachine, $view)
                $hive = $users.OpenSubKey($sid, $false)
                $scope = Open-RegdiffScope $hive ('Software\' + $name) $label
                try {
                    return (New-RegdiffSnapshot -Scopes @($scope) -Hidden (Get-RegdiffHiddenNames $machine $sid))
                } finally {
                    Close-RegdiffKey $scope.Key
                    $hive.Dispose()
                    $machine.Dispose()
                    $users.Dispose()
                }
            }
            $first = & $take
            $keys = @($first['keys'])
            $machineName = Get-RegdiffTestToken 'name' ([Environment]::MachineName)
            $rootValues = @(
                '<default>=REG_SZ "default text"',
                ($machineName + '=REG_DWORD 1'),
                'AllOnes=REG_DWORD 4294967295',
                'Bytes=REG_BINARY <bin len=3 sha256=ba7816bf>',
                'Dword=REG_DWORD 1',
                ('Expand=REG_EXPAND_SZ <str len=20 sha256=' + (Get-RegdiffTestHash '%USERPROFILE%\Videos') + '>'),
                ('Folder=REG_SZ ' + (Get-RegdiffTestToken 'str' 'C:\Users\x\Videos')),
                'Multi=REG_MULTI_SZ ["a","b"]',
                'Plain=REG_SZ "Radeon ReLive"',
                'Qword=REG_QWORD 5')
            Assert-RegdiffTestEntry $keys[0] $label 'read' $rootValues
            Assert-RegdiffTestEntry $keys[1] ($label + '\' + (Get-RegdiffTestToken 'name' ([Environment]::UserName))) 'read' @('Inner=REG_DWORD 2')
            $deep = $label
            for ($level = 1; $level -le 8; $level++) {
                $deep += '\k' + $level
                Assert-RegdiffTestEntry $keys[1 + $level] $deep 'read' @()
            }
            Assert-RegdiffTestEntry $keys[10] ($deep + '\k9') 'deep'
            Assert-RegdiffTestEntry $keys[11] ($label + '\Locked') 'denied'
            Assert-Equal 12 $keys.Count 'entries'
            $json = ConvertTo-RegdiffJson $first
            foreach ($secret in @($sid, [Environment]::UserName, [Environment]::MachineName, '%USERPROFILE%', 'C:\\')) {
                Assert-True ($json.IndexOf($secret, [StringComparison]::OrdinalIgnoreCase) -lt 0) ('the snapshot holds ' + $secret)
            }

            $test = $software.OpenSubKey($name, $true)
            try { $test.SetValue('Dword', 0, $kinds::DWord) } finally { $test.Dispose() }
            $lines = (Compare-RegdiffSnapshot (Read-RegdiffSnapshot $json) (& $take)).Lines
            $expected = @(($label + '\Dword: 1 -> 0'), ($deep + '\k9: <past depth 8>'), ($label + '\Locked: <denied>'), '1 values changed')
            Assert-Equal ($expected -join '|') ($lines -join '|') 'lines'
        } finally {
            if ($null -ne $deny) {
                $rights = [Security.AccessControl.RegistryRights]::ReadPermissions -bor [Security.AccessControl.RegistryRights]::ChangePermissions
                $locked = $software.OpenSubKey($name + '\Locked', [Microsoft.Win32.RegistryKeyPermissionCheck]::ReadWriteSubTree, $rights)
                if ($null -ne $locked) {
                    $acl = $locked.GetAccessControl()
                    [void]$acl.RemoveAccessRule($deny)
                    $locked.SetAccessControl($acl)
                    $locked.Dispose()
                }
            }
            $software.DeleteSubKeyTree($name, $false)
            $software.Dispose()
        }
        $gone = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\' + $name, $false)
        Assert-True ($null -eq $gone) 'the temporary key is gone'
    }

    Test-Case 'command line -Snapshot -Driver (Windows CI): the runner''s registry, one JSON line, exit 0' {
        $run = Invoke-BenchTool 'regdiff.ps1' @('-Snapshot', '-Driver')
        Assert-Equal 0 $run.ExitCode ('exit code; stderr: ' + $run.StdErr)
        $s = Read-RegdiffSnapshot $run.StdOut 'the runner''s snapshot'
        Assert-Equal $true $s['driver'] 'driver'
        Assert-Equal 'HKCU\Software\AMD,HKLM\SOFTWARE\AMD,HKLM\SOFTWARE\WOW6432Node\AMD' ((@($s['scopes']) | Select-Object -First 3) -join ',') 'the first three scopes'
        Assert-Equal 0 (Compare-RegdiffSnapshot $s $s).Changed 'compares with itself'
    }
} else {
    [Console]::Out.WriteLine('skip HKCU (Windows CI): a temporary key under HKCU, and -Snapshot -Driver; they run only on Windows with GITHUB_ACTIONS set')
}

Test-Case 'regdiff.ps1, amd-diff.ps1 and this file are ASCII' {
    foreach ($file in @((Join-Path $BenchDir 'regdiff.ps1'), $script:AmdDiffPath, $PSCommandPath)) {
        $bytes = [IO.File]::ReadAllBytes($file)
        $bad = 0
        foreach ($b in $bytes) { if ($b -gt 0x7E -or ($b -lt 0x20 -and $b -ne 9 -and $b -ne 10 -and $b -ne 13)) { $bad++ } }
        Assert-Equal 0 $bad ((Split-Path -Leaf $file) + ': bytes outside printable ASCII')
    }
}

Complete-BenchTests
