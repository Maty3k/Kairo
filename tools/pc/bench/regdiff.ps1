# SPDX-License-Identifier: GPL-3.0-or-later
#
# tools/pc/bench/regdiff.ps1 - AMD's settings in the registry: a snapshot, and the values that changed
# between two snapshots (the AMD settings diff's functions; pc amd-diff runs them).
# Project: https://github.com/Maty3k/Kairo
# Design: docs/research/17-loadgen-bench/README.md section 5.5, with its 2026-11-24 correction.
# Contract: docs/milestones/M1.md, Formats.
#
# Command line:
#   pwsh -NoProfile -File regdiff.ps1 -Before <snapshot.json> -After <snapshot.json>
#   pwsh -NoProfile -File regdiff.ps1 -Snapshot [-Driver] [-Key <32 hex digits>]
# -Before and -After print the lines below for two snapshots taken with the same key, such as the
# ones pc amd-diff keeps in .dogfood/amd-diff/. -Snapshot prints a snapshot of the scopes below as
# one JSON line, with this account's hive as HKCU and its hashes keyed with -Key (16 bytes, written
# as 32 hex digits). Without -Key it makes a random key, which it never prints, and says so on
# stderr; that snapshot then compares with no other. Exit codes: 0 printed; 1 a hive couldn't be
# read (the owner's isn't loaded, the display class key with -Driver, or this isn't Windows); 2 bad
# command line (a -Key that isn't 32 hex digits included), a file that isn't a regdiff snapshot, or
# two snapshots that can't be compared (taken with different keys, or one with -Driver and one
# without).
#
# It never writes the registry. Every key is opened read-only, with
# [Microsoft.Win32.RegistryKey]::OpenBaseKey(...).OpenSubKey(name, $false), and AMD's settings are
# never written (research 13 section 4.2); scans.tests.ps1 checks this file and amd-diff.ps1 for writes.
#
# Scopes, each a whole subtree, in this order:
#   HKCU\Software\AMD             the owner's, read from HKEY_USERS\<SID>: pc amd-diff gives the SID
#                                 of \Kairo\Run's principal (Get-RegdiffOwnerSid, section 5.4)
#   HKLM\SOFTWARE\AMD and HKLM\SOFTWARE\WOW6432Node\AMD
#   with -Driver, every display-class key ...\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\00NN whose
#   ProviderName is AMD's, labelled by its MatchingDeviceId's VEN_/DEV_ part, as
#   HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\0000[VEN_1002&DEV_744C]
#   (keys of removed devices stay behind, so there may be more than two).
# The walk goes depth first, names in order. A subkey it may not read (a SecurityException) is
# <denied>, and the walk goes on. Keys more than 8 levels below their scope are <past depth 8> and
# not opened. At 20,000 values it stops: the key whose values would pass the limit is
# <stopped at 20000 values>, and nothing after it is read.
#
# What a line or a snapshot may show (section 5.5 and its correction): a name or a string as it is
# only when it is plain, and otherwise its length and a keyed hash.
# - A key or value name is plain when it is at most 64 of A-Z a-z 0-9, space and _ . , + = # : ; ( )
#   [ ] - (so no \, / or @); holds no SID-shaped word, (?i)\bS-1-[0-9]+(-[0-9]+)+; holds no
#   program's name, (?i)\.(exe|dll)\b, other than Kairo's own kairo-<name>.exe (research 17's risk
#   15 needs to see those); holds none of the hidden names, in any case (this account, its domain,
#   the PC's name and host name, the owner's account, both profile folders' names, and the owner's
#   and this session's SIDs); and isn't one of the member names PowerShell reserves (PSObject,
#   PSBase, PSAdapted, PSExtended, PSTypeNames, in any case), which ConvertFrom-Json can't read back
#   as a value's name. A GUID in a name shows as it is. A plain name shows as itself; any other as
#   <name len=N hmac=8hex>: N characters, and the first 8 hex digits of the HMAC-SHA256 of its UTF-8
#   bytes under the key. A key's unnamed value is <default>.
# - REG_DWORD and REG_QWORD data shows as an unsigned number. A REG_SZ or REG_EXPAND_SZ string (read
#   with DoNotExpandEnvironmentNames, so %USERPROFILE% never becomes a path with the account's name)
#   shows in quotes when it is plain by the names' test (the reserved names aside) and holds no GUID
#   (8-4-4-4-12 hex digits with their hyphens, or 32 hex digits in a row: an install ID is a PC
#   detail); a REG_MULTI_SZ shows as ["a","b"] when its strings joined by commas do. Any other
#   string is <str len=N hmac=8hex> (a REG_MULTI_SZ hashed as its strings joined by NUL characters).
#   Bytes (REG_BINARY, REG_NONE and the like) are <bin len=N hmac=8hex>, N bytes. Data .NET doesn't
#   return (REG_LINK, resource lists) is <no data>.
# - Types show as REG_SZ, REG_EXPAND_SZ, REG_MULTI_SZ, REG_DWORD, REG_QWORD, REG_BINARY, REG_NONE,
#   and REG_UNKNOWN for the kinds .NET doesn't name.
#
# The key. The hashes are HMAC-SHA256 under a random 16-byte key, so without the key a hash can't
# be reversed by guessing its text, or matched with another session's (section 5.5's correction:
# an unkeyed hash and the exact length gave a short name away). pc amd-diff before makes the key on
# the Mac, keeps it in .dogfood/amd-diff/<stamp>/key and sends it with before and with after. A
# snapshot holds the key's id, never the key: "key_id", the first 16 hex digits of the HMAC-SHA256
# of "kairo-regdiff key id" under the key. Two snapshots compare only when their ids match.
#
# A snapshot is one JSON object (ConvertTo-RegdiffJson), holding only those shown forms:
#   {"kairo_regdiff":1,"key_id":"0123456789abcdef","driver":false,"max_depth":8,"max_values":20000,
#    "value_count":2,"cut":false,
#    "scopes":["HKCU\\Software\\AMD","HKLM\\SOFTWARE\\AMD","HKLM\\SOFTWARE\\WOW6432Node\\AMD"],
#    "keys":[{"key":"HKCU\\Software\\AMD","state":"read","values":{}},
#      {"key":"HKCU\\Software\\AMD\\DVR","state":"read","values":{"DvrEnabled":"REG_DWORD 1",
#        "<name len=21 hmac=0123abcd>":"REG_SZ <str len=33 hmac=89abcdef>"}},
#      {"key":"HKLM\\SOFTWARE\\AMD","state":"absent"},{"key":"HKLM\\SOFTWARE\\WOW6432Node\\AMD","state":"absent"}]}
# A key's state is read (with its values, name -> "TYPE shown"), absent (a scope that doesn't
# exist), denied, deep or cut. Read-RegdiffSnapshot refuses a file that holds anything else: a
# name or string the plain test would hash included.
#
# The lines (docs/milestones/M1.md, Formats), sorted by path, then the count:
#   HKCU\Software\AMD\DVR\DvrEnabled: 1 -> 0
#   HKCU\Software\AMD\DVR\DvrEnabled: (absent) -> 1                 the value appeared
#   HKCU\Software\AMD\DVR\DvrEnabled: 0 -> (absent)                 the value went
#   HKCU\Software\AMD\DVR\DvrEnabled: REG_DWORD 1 -> REG_SZ "1"     its type changed
#   HKLM\SOFTWARE\AMD\Locked: <denied>                              a key either snapshot didn't
#                                    read; also <past depth 8> and <stopped at 20000 values>
#   4 values changed
# Values under a key that either snapshot didn't read are left out, so nothing unread passes for a
# change. The count line always says "values".
#
# As a library (amd-diff.ps1 dot-sources the text the Mac sends; so do the tests):
#   $key = ConvertTo-RegdiffKey '<32 hex digits>'     # or New-RegdiffKey: 16 random bytes
#   $s = Get-RegdiffSnapshot -OwnerSid (Get-RegdiffOwnerSid) -Key $key [-Driver]   # the live registry
#   $json = ConvertTo-RegdiffJson $s; $back = Read-RegdiffSnapshot $json 'before.json'
#   $d = Compare-RegdiffSnapshot $before $after       # $d.Lines (the count line last), $d.Changed
#   Format-RegdiffSummary $s                          # the terminal's view: counts per scope
#   New-RegdiffSnapshot walks given scopes, whose keys may be synthetic dictionaries: the tests use it.
# Functions only return or throw. A thrown exception carries its exit code in .Data['KairoExit'].
# Dot-sourcing defines functions and nothing else; the command line at the bottom runs only when
# the file itself is run, and it never prompts. Load the text with '.', from the file or through
# [scriptblock]::Create: Invoke-Expression reports the caller's name, so the command line would run.
#
# Rules: ASCII only; Windows PowerShell 5.1 syntax and .NET Framework 4.x APIs (it runs on
# PowerShell 7); no Add-Type; clean under Set-StrictMode -Version 3.0. No message names a file's
# folder, the key or anything read from the registry. A dictionary keyed by registry names is
# counted with get_Count() and listed with get_Keys() or GetEnumerator(), never .Count or .Keys,
# which PowerShell answers with a value named Count or Keys.

function New-RegdiffError([int] $Code, [string] $Message) {
    $exception = [Exception]::new($Message)
    $exception.Data['KairoExit'] = $Code
    return $exception
}

function Get-RegdiffExitCode($ErrorRecord) {
    # The exit code a caught error stands for: its own, or 1 (failed).
    $exception = $ErrorRecord.Exception
    while ($null -ne $exception) {
        if ($exception.Data.Contains('KairoExit')) { return [int]$exception.Data['KairoExit'] }
        $exception = $exception.InnerException
    }
    return 1
}

function Get-RegdiffLimits {
    # Section 5.5's limits: keys at most 8 levels below their scope, and 20,000 values in all.
    return [pscustomobject]@{ Depth = 8; Values = 20000 }
}

function Get-RegdiffDisplayClass {
    # The display adapters' device setup class (learn.microsoft.com, "System-defined device setup classes").
    return '{4d36e968-e325-11ce-bfc1-08002be10318}'
}

# --- What may be shown ---------------------------------------------------------------------------

function Test-RegdiffPlain([string] $Text, [string[]] $Hidden) {
    # Section 5.5's plain-text test, with its 2026-11-24 correction, for names and data alike: the
    # pattern, which leaves out \, / and @; no SID-shaped word; no program's name (.exe or .dll)
    # other than Kairo's own kairo-<name>.exe; and no hidden name, in any case.
    if ($Text -cnotmatch '^[A-Za-z0-9 _.,+=#:;()\[\]-]{0,64}\z') { return $false }
    if ([regex]::IsMatch($Text, '(?i)\bS-1-[0-9]+(-[0-9]+)+')) { return $false }
    # Kairo's own programs, such as kairo-loadgen.exe, are taken out first; any other .exe or .dll fails.
    $others = [regex]::Replace($Text, '(?i)(?<![A-Za-z0-9_.-])kairo-[A-Za-z0-9_-]+\.exe\b', '')
    if ([regex]::IsMatch($others, '(?i)\.(exe|dll)\b')) { return $false }
    foreach ($name in $Hidden) {
        if (-not [string]::IsNullOrEmpty($name) -and $Text.IndexOf($name, [StringComparison]::OrdinalIgnoreCase) -ge 0) { return $false }
    }
    return $true
}

function Get-RegdiffReservedNames {
    # The member names PowerShell reserves: ConvertFrom-Json refuses an object that has one as a
    # property, so a value of that name could never be read back from a snapshot.
    return @('PSObject', 'PSBase', 'PSAdapted', 'PSExtended', 'PSTypeNames')
}

function Test-RegdiffPlainName([string] $Text, [string[]] $Hidden) {
    # A key or value name: plain, and not a reserved member name in any case. A GUID in a name stays.
    if ((Get-RegdiffReservedNames) -contains $Text) { return $false }
    return (Test-RegdiffPlain $Text $Hidden)
}

function Test-RegdiffPlainData([string] $Text, [string[]] $Hidden) {
    # A string value's data: plain, and holding no GUID, 8-4-4-4-12 hex digits with their hyphens
    # or 32 hex digits in a row (an install ID is a PC detail).
    if ([regex]::IsMatch($Text, '(?i)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}|[0-9a-f]{32}')) { return $false }
    return (Test-RegdiffPlain $Text $Hidden)
}

# --- The key ---------------------------------------------------------------------------------------

function Test-RegdiffKey($Key) {
    return ($Key -is [byte[]] -and $Key.Length -eq 16)
}

function New-RegdiffKey {
    # 16 random bytes, for a snapshot taken without a given key.
    $key = [byte[]]::new(16)
    $random = [Security.Cryptography.RandomNumberGenerator]::Create()
    try {
        $random.GetBytes($key)
    } finally {
        $random.Dispose()
    }
    return ,$key
}

function ConvertTo-RegdiffKey([string] $Text) {
    # A key written as 32 hex digits, as 16 bytes. The message never quotes the text.
    if ($Text -cnotmatch '^[0-9A-Fa-f]{32}\z') { throw (New-RegdiffError 2 'the key isn''t 32 hex digits (16 bytes)') }
    $key = [byte[]]::new(16)
    for ($i = 0; $i -lt 16; $i++) { $key[$i] = [Convert]::ToByte($Text.Substring(2 * $i, 2), 16) }
    return ,$key
}

function Get-RegdiffHmac([byte[]] $Key, [byte[]] $Bytes, [int] $Digits) {
    # The first $Digits hex digits (an even number) of HMAC-SHA256(key, bytes).
    if ($null -eq $Key -or $Key.Length -eq 0) { throw (New-RegdiffError 1 'internal: no key for the hashes') }
    $hmac = [Security.Cryptography.HMACSHA256]::new($Key)
    try {
        $digest = $hmac.ComputeHash($Bytes)
    } finally {
        $hmac.Dispose()
    }
    return [BitConverter]::ToString($digest, 0, [int]($Digits / 2)).Replace('-', '').ToLowerInvariant()
}

function Get-RegdiffHash([byte[]] $Key, [byte[]] $Bytes) {
    # A token's hash: the first 8 hex digits of HMAC-SHA256(key, bytes).
    return (Get-RegdiffHmac $Key $Bytes 8)
}

function Get-RegdiffKeyId([byte[]] $Key) {
    # What a snapshot holds of its key: the first 16 hex digits of HMAC-SHA256(key, "kairo-regdiff key id").
    return (Get-RegdiffHmac $Key ([Text.Encoding]::UTF8.GetBytes('kairo-regdiff key id')) 16)
}

function Format-RegdiffToken([string] $Kind, [long] $Length, [byte[]] $Bytes, [byte[]] $Key) {
    # <name|str|bin len=N hmac=8hex>.
    return ('<' + $Kind + ' len=' + $Length.ToString([Globalization.CultureInfo]::InvariantCulture) + ' hmac=' + (Get-RegdiffHash $Key $Bytes) + '>')
}

function Format-RegdiffName([string] $Name, [string[]] $Hidden, [byte[]] $Key) {
    # A key or value name as it shows: itself when plain, <default> for a key's unnamed value, else
    # <name len=N hmac=8hex>.
    if ($Name.Length -eq 0) { return '<default>' }
    if (Test-RegdiffPlainName $Name $Hidden) { return $Name }
    return (Format-RegdiffToken 'name' $Name.Length ([Text.Encoding]::UTF8.GetBytes($Name)) $Key)
}

function Format-RegdiffData($Data, [string[]] $Hidden, [byte[]] $Key) {
    # A value's data as it shows, by its .NET type: Int32 (DWORD) and Int64 (QWORD) as unsigned
    # numbers, a string or a string array by the data's plain test, bytes always hashed.
    $inv = [Globalization.CultureInfo]::InvariantCulture
    if ($null -eq $Data) { return '<no data>' }
    if ($Data -is [int]) { return [BitConverter]::ToUInt32([BitConverter]::GetBytes([int]$Data), 0).ToString($inv) }
    if ($Data -is [long]) { return [BitConverter]::ToUInt64([BitConverter]::GetBytes([long]$Data), 0).ToString($inv) }
    if ($Data -is [byte[]]) { return (Format-RegdiffToken 'bin' $Data.Length $Data $Key) }
    if ($Data -is [string]) {
        if (Test-RegdiffPlainData $Data $Hidden) { return ('"' + $Data + '"') }
        return (Format-RegdiffToken 'str' $Data.Length ([Text.Encoding]::UTF8.GetBytes($Data)) $Key)
    }
    if ($Data -is [string[]]) {
        if (Test-RegdiffPlainData ([string]::Join(',', $Data)) $Hidden) {
            $quoted = [Collections.Generic.List[string]]::new()
            foreach ($item in $Data) { $quoted.Add('"' + $item + '"') }
            return ('[' + [string]::Join(',', $quoted) + ']')
        }
        $whole = [string]::Join([string][char]0, $Data)
        return (Format-RegdiffToken 'str' $whole.Length ([Text.Encoding]::UTF8.GetBytes($whole)) $Key)
    }
    throw (New-RegdiffError 1 ('a value of an unexpected .NET type, ' + $Data.GetType().Name))
}

function Get-RegdiffTypeName([string] $Kind) {
    # A RegistryValueKind's name as Windows writes it.
    switch -CaseSensitive ($Kind) {
        'String' { return 'REG_SZ' }
        'ExpandString' { return 'REG_EXPAND_SZ' }
        'MultiString' { return 'REG_MULTI_SZ' }
        'DWord' { return 'REG_DWORD' }
        'QWord' { return 'REG_QWORD' }
        'Binary' { return 'REG_BINARY' }
        'None' { return 'REG_NONE' }
    }
    return 'REG_UNKNOWN'
}

function Format-RegdiffValue([string] $Kind, $Data, [string[]] $Hidden, [byte[]] $Key) {
    # "TYPE shown", as a snapshot holds a value: REG_DWORD 1, REG_SZ "abc", REG_BINARY <bin len=4 hmac=...>.
    return ((Get-RegdiffTypeName $Kind) + ' ' + (Format-RegdiffData $Data $Hidden $Key))
}

# --- Keys: the registry's, or the tests' synthetic ones ----------------------------------------------
# A synthetic key is a dictionary: 'values' maps a value's name to @(kind, data), and 'keys' a
# subkey's name to a synthetic key, or to 'denied' for one that may not be read. Their names are
# listed with get_Keys(): .Keys would give the entry of a value or subkey named Keys.

function Get-RegdiffValueNames($Key) {
    if ($Key -is [Microsoft.Win32.RegistryKey]) { return ,[string[]]$Key.GetValueNames() }
    $names = [Collections.Generic.List[string]]::new()
    if ($Key.Contains('values')) { foreach ($name in $Key['values'].get_Keys()) { $names.Add([string]$name) } }
    return ,$names.ToArray()
}

function Get-RegdiffSubKeyNames($Key) {
    if ($Key -is [Microsoft.Win32.RegistryKey]) { return ,[string[]]$Key.GetSubKeyNames() }
    $names = [Collections.Generic.List[string]]::new()
    if ($Key.Contains('keys')) { foreach ($name in $Key['keys'].get_Keys()) { $names.Add([string]$name) } }
    return ,$names.ToArray()
}

function Open-RegdiffSubKey($Key, [string] $Name) {
    # A subkey, read-only, or $null when it isn't there. One it may not read throws a SecurityException.
    if ($Key -is [Microsoft.Win32.RegistryKey]) { return $Key.OpenSubKey($Name, $false) }
    if (-not $Key.Contains('keys')) { return $null }
    $child = $Key['keys'][$Name]
    if ($child -is [string] -and $child -ceq 'denied') { throw [Security.SecurityException]::new('denied') }
    return $child
}

function Read-RegdiffValue($Key, [string] $Name) {
    # @{ Kind; Data } of one value, REG_EXPAND_SZ unexpanded; $null when it has gone.
    if ($Key -is [Microsoft.Win32.RegistryKey]) {
        try {
            $kind = $Key.GetValueKind($Name)
        } catch [IO.IOException] {
            return $null
        }
        $data = $Key.GetValue($Name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        return @{ Kind = [string]$kind; Data = $data }
    }
    if (-not $Key['values'].Contains($Name)) { return $null }
    $pair = $Key['values'][$Name]
    return @{ Kind = [string]$pair[0]; Data = $pair[1] }
}

function Close-RegdiffKey($Key) {
    if ($Key -is [Microsoft.Win32.RegistryKey]) { $Key.Dispose() }
}

function Get-RegdiffText($Key, [string] $Name) {
    # One string value of a registry key, read by name and unexpanded; '' when it isn't a string.
    $data = $null
    try {
        $data = $Key.GetValue($Name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
    } catch [IO.IOException] {
        $data = $null
    }
    if ($data -is [string]) { return $data }
    return ''
}

# --- The walk --------------------------------------------------------------------------------------

function Get-RegdiffShownNames([string[]] $Names, [string[]] $Hidden, [byte[]] $Key) {
    # Each name with its shown form, in the order of the shown forms (ignoring case). A name shown
    # like one before it (two hashes alike in 32 bits) is left out.
    $shown = [string[]]::new($Names.Length)
    $raw = [string[]]::new($Names.Length)
    for ($i = 0; $i -lt $Names.Length; $i++) {
        $raw[$i] = $Names[$i]
        $shown[$i] = Format-RegdiffName $Names[$i] $Hidden $Key
    }
    [Array]::Sort($shown, $raw, [StringComparer]::OrdinalIgnoreCase)
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $out = [Collections.Generic.List[object]]::new()
    for ($i = 0; $i -lt $shown.Length; $i++) {
        if ($seen.Add($shown[$i])) { $out.Add([pscustomobject]@{ Shown = $shown[$i]; Raw = $raw[$i] }) }
    }
    return ,$out.ToArray()
}

function New-RegdiffWalk([string[]] $Hidden, [byte[]] $Key) {
    if (-not (Test-RegdiffKey $Key)) { throw (New-RegdiffError 1 'internal: the key isn''t 16 bytes') }
    $limits = Get-RegdiffLimits
    return [pscustomobject]@{
        Hidden     = $Hidden
        Key        = $Key
        MaxDepth   = $limits.Depth
        MaxValues  = $limits.Values
        ValueCount = 0
        Cut        = $false
        Entries    = [Collections.Generic.List[object]]::new()
    }
}

function Add-RegdiffEntry($Walk, [string] $Label, [string] $State, $Values) {
    $entry = [ordered]@{}
    $entry['key'] = $Label
    $entry['state'] = $State
    if ($State -ceq 'read') { $entry['values'] = $Values }
    $Walk.Entries.Add($entry)
}

function Read-RegdiffTree($Walk, $Key, [string] $Label, [int] $Depth) {
    # One key's values, then each subkey's tree, depth first.
    $valueNames = [string[]]@()
    try { $valueNames = Get-RegdiffValueNames $Key } catch [IO.IOException] { $valueNames = [string[]]@() }
    if ($Walk.ValueCount + $valueNames.Length -gt $Walk.MaxValues) {
        $Walk.Cut = $true
        Add-RegdiffEntry $Walk $Label 'cut' $null
        return
    }
    $values = [ordered]@{}
    foreach ($item in (Get-RegdiffShownNames $valueNames $Walk.Hidden $Walk.Key)) {
        $value = Read-RegdiffValue $Key $item.Raw
        if ($null -eq $value) { continue }
        $values[$item.Shown] = Format-RegdiffValue $value['Kind'] $value['Data'] $Walk.Hidden $Walk.Key
    }
    # get_Count(): .Count would give the data of a value named Count.
    $Walk.ValueCount += $values.get_Count()
    Add-RegdiffEntry $Walk $Label 'read' $values

    $subNames = [string[]]@()
    try { $subNames = Get-RegdiffSubKeyNames $Key } catch [IO.IOException] { $subNames = [string[]]@() }
    foreach ($item in (Get-RegdiffShownNames $subNames $Walk.Hidden $Walk.Key)) {
        $childLabel = $Label + '\' + $item.Shown
        if ($Depth -ge $Walk.MaxDepth) {
            Add-RegdiffEntry $Walk $childLabel 'deep' $null
            continue
        }
        $child = $null
        try {
            $child = Open-RegdiffSubKey $Key $item.Raw
        } catch [Security.SecurityException] {
            Add-RegdiffEntry $Walk $childLabel 'denied' $null
            continue
        } catch [UnauthorizedAccessException] {
            Add-RegdiffEntry $Walk $childLabel 'denied' $null
            continue
        }
        if ($null -eq $child) { continue }
        try {
            Read-RegdiffTree $Walk $child $childLabel ($Depth + 1)
        } finally {
            Close-RegdiffKey $child
        }
        if ($Walk.Cut) { return }
    }
}

function New-RegdiffScope([string] $Label, $Key, [string] $State) {
    # One scope: its label as lines show it, its opened root (or $null) and read, absent or denied.
    return [pscustomobject]@{ Label = $Label; Key = $Key; State = $State }
}

function Open-RegdiffScope($Base, [string] $SubKey, [string] $Label) {
    # A scope whose root is $SubKey under an opened key, opened read-only.
    $key = $null
    try {
        $key = Open-RegdiffSubKey $Base $SubKey
    } catch [Security.SecurityException] {
        return (New-RegdiffScope $Label $null 'denied')
    } catch [UnauthorizedAccessException] {
        return (New-RegdiffScope $Label $null 'denied')
    }
    if ($null -eq $key) { return (New-RegdiffScope $Label $null 'absent') }
    return (New-RegdiffScope $Label $key 'read')
}

function New-RegdiffSnapshot {
    # The scopes walked in order into a snapshot: an ordered dictionary in the JSON's shape. Its
    # hashes are keyed with $Key, 16 bytes.
    param([object[]] $Scopes, [string[]] $Hidden, [byte[]] $Key, [switch] $Driver)
    $walk = New-RegdiffWalk $Hidden $Key
    $labels = [Collections.Generic.List[string]]::new()
    foreach ($scope in $Scopes) {
        $labels.Add($scope.Label)
        if ($walk.Cut) { continue }
        if ($scope.State -ceq 'read') {
            Read-RegdiffTree $walk $scope.Key $scope.Label 0
        } else {
            Add-RegdiffEntry $walk $scope.Label $scope.State $null
        }
    }
    $snapshot = [ordered]@{}
    $snapshot['kairo_regdiff'] = 1
    $snapshot['key_id'] = Get-RegdiffKeyId $Key
    $snapshot['driver'] = [bool]$Driver
    $snapshot['max_depth'] = $walk.MaxDepth
    $snapshot['max_values'] = $walk.MaxValues
    $snapshot['value_count'] = $walk.ValueCount
    $snapshot['cut'] = $walk.Cut
    $snapshot['scopes'] = $labels.ToArray()
    $snapshot['keys'] = $walk.Entries.ToArray()
    return $snapshot
}

# --- The live registry -----------------------------------------------------------------------------

function Test-RegdiffWindows {
    return ([Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT)
}

function Get-RegdiffOwnerSid {
    # The SID of \Kairo\Run's principal (research 17 section 5.4): the owner's account, which an
    # administrator registered the task for. Task Scheduler gives an account name or a SID; the
    # name is turned into a SID in memory and never shown.
    $userId = ''
    try {
        $service = New-Object -ComObject 'Schedule.Service'
        $service.Connect()
        $userId = [string]$service.GetFolder('\Kairo').GetTask('Run').Definition.Principal.UserId
    } catch {
        throw (New-RegdiffError 1 'the owner''s account is unknown: \Kairo\Run can''t be read')
    }
    if ($userId -cmatch '^S-1-[0-9]+(-[0-9]+)+\z') { return $userId }
    try {
        return ([Security.Principal.NTAccount]::new($userId)).Translate([Security.Principal.SecurityIdentifier]).Value
    } catch {
        throw (New-RegdiffError 1 'the owner''s account is unknown: \Kairo\Run''s principal has no SID')
    }
}

function Get-RegdiffLeaf([string] $Text) {
    # The last part of a Windows path or account name, on any platform.
    $trimmed = $Text.TrimEnd([char]'\', [char]'/')
    $cut = $trimmed.LastIndexOfAny([char[]]@('\', '/'))
    if ($cut -ge 0) { return $trimmed.Substring($cut + 1) }
    return $trimmed
}

function Add-RegdiffHiddenName($List, [string] $Name) {
    if ($null -eq $Name) { return }
    $name = $Name.Trim()
    if ($name.Length -gt 0 -and -not $List.Contains($name)) { $List.Add($name) }
}

function Read-RegdiffNamedText($Machine, [string] $SubKey, [string] $Name) {
    # One string value under HKLM, by name; '' when it can't be read.
    $key = $null
    try {
        $key = $Machine.OpenSubKey($SubKey, $false)
    } catch [Security.SecurityException] {
        return ''
    }
    if ($null -eq $key) { return '' }
    try {
        return (Get-RegdiffText $key $Name)
    } finally {
        $key.Dispose()
    }
}

function Get-RegdiffHiddenNames($Machine, [string] $OwnerSid, [string] $SessionSid) {
    # The names no line or snapshot may show: this session's account, its domain and the PC's name;
    # the PC's DNS host name; the owner's account (from its SID); both accounts' profile folder
    # names; and both accounts' SIDs. They are compared, never shown.
    $names = [Collections.Generic.List[string]]::new()
    Add-RegdiffHiddenName $names $OwnerSid
    Add-RegdiffHiddenName $names $SessionSid
    Add-RegdiffHiddenName $names ([Environment]::UserName)
    Add-RegdiffHiddenName $names ([Environment]::MachineName)
    Add-RegdiffHiddenName $names (Get-RegdiffLeaf ([Environment]::GetFolderPath('UserProfile')))
    try {
        Add-RegdiffHiddenName $names ([Environment]::UserDomainName)
    } catch {
        # It asks the domain, which can fail; the PC's own name is in the list either way.
    }
    try {
        $account = ([Security.Principal.SecurityIdentifier]::new($OwnerSid)).Translate([Security.Principal.NTAccount]).Value
        foreach ($part in $account.Split([char]'\')) { Add-RegdiffHiddenName $names $part }
    } catch {
        # A SID without a name (a deleted account) hides nothing more.
    }
    $profiles = 'SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\' + $OwnerSid
    Add-RegdiffHiddenName $names (Get-RegdiffLeaf (Read-RegdiffNamedText $Machine $profiles 'ProfileImagePath'))
    $tcpip = 'SYSTEM\CurrentControlSet\Services\Tcpip\Parameters'
    Add-RegdiffHiddenName $names (Read-RegdiffNamedText $Machine $tcpip 'Hostname')
    Add-RegdiffHiddenName $names (Read-RegdiffNamedText $Machine $tcpip 'NV Hostname')
    return ,$names.ToArray()
}

function Get-RegdiffDriverScopes($Machine, $Opened) {
    # With -Driver: each display-class key whose ProviderName is AMD's, labelled by the VEN_ and
    # DEV_ part of its MatchingDeviceId. Every key it opens goes into $Opened, to be closed.
    $classPath = 'SYSTEM\CurrentControlSet\Control\Class\' + (Get-RegdiffDisplayClass)
    $class = $null
    try {
        $class = $Machine.OpenSubKey($classPath, $false)
    } catch [Security.SecurityException] {
        $class = $null
    }
    if ($null -eq $class) { throw (New-RegdiffError 1 'the display adapters'' class key can''t be read') }
    $Opened.Add($class)
    $scopes = [Collections.Generic.List[object]]::new()
    $names = [string[]]$class.GetSubKeyNames()
    [Array]::Sort($names, [StringComparer]::Ordinal)
    foreach ($name in $names) {
        if ($name -cnotmatch '^[0-9]{4}\z') { continue }
        $label = 'HKLM\' + $classPath + '\' + $name
        $key = $null
        try {
            $key = $class.OpenSubKey($name, $false)
        } catch [Security.SecurityException] {
            $scopes.Add((New-RegdiffScope $label $null 'denied'))
            continue
        }
        if ($null -eq $key) { continue }
        $Opened.Add($key)
        if ((Get-RegdiffText $key 'ProviderName') -notmatch '^(Advanced Micro Devices|AMD)\b') { continue }
        $device = '?'
        $match = [regex]::Match((Get-RegdiffText $key 'MatchingDeviceId'), '(?i)VEN_([0-9A-F]{4})&DEV_([0-9A-F]{4})')
        if ($match.Success) { $device = 'VEN_' + $match.Groups[1].Value.ToUpperInvariant() + '&DEV_' + $match.Groups[2].Value.ToUpperInvariant() }
        $scopes.Add((New-RegdiffScope ($label + '[' + $device + ']') $key 'read'))
    }
    return ,$scopes.ToArray()
}

function Get-RegdiffSnapshot {
    # A snapshot of section 5.5's scopes in the live registry, with the owner's hive found by its
    # SID and the hashes keyed with $Key, 16 bytes.
    param([string] $OwnerSid, [switch] $Driver, [byte[]] $Key)
    if (-not (Test-RegdiffWindows)) { throw (New-RegdiffError 1 'the registry exists only on Windows') }
    if ($OwnerSid -cnotmatch '^S-1-[0-9]+(-[0-9]+)+\z') { throw (New-RegdiffError 1 'the owner''s SID is not a SID') }
    if (-not (Test-RegdiffKey $Key)) { throw (New-RegdiffError 1 'internal: the key isn''t 16 bytes') }
    $view = [Microsoft.Win32.RegistryView]::Registry64
    $opened = [Collections.Generic.List[object]]::new()
    try {
        $users = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::Users, $view)
        $opened.Add($users)
        $machine = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::LocalMachine, $view)
        $opened.Add($machine)
        $hive = $null
        try {
            $hive = $users.OpenSubKey($OwnerSid, $false)
        } catch [Security.SecurityException] {
            $hive = $null
        }
        if ($null -eq $hive) { throw (New-RegdiffError 1 'the owner''s registry hive can''t be read: is the owner signed in?') }
        $opened.Add($hive)
        $hidden = Get-RegdiffHiddenNames $machine $OwnerSid ([Security.Principal.WindowsIdentity]::GetCurrent().User.Value)
        $scopes = [Collections.Generic.List[object]]::new()
        $scopes.Add((Open-RegdiffScope $hive 'Software\AMD' 'HKCU\Software\AMD'))
        $scopes.Add((Open-RegdiffScope $machine 'SOFTWARE\AMD' 'HKLM\SOFTWARE\AMD'))
        $scopes.Add((Open-RegdiffScope $machine 'SOFTWARE\WOW6432Node\AMD' 'HKLM\SOFTWARE\WOW6432Node\AMD'))
        foreach ($scope in $scopes) { if ($null -ne $scope.Key) { $opened.Add($scope.Key) } }
        if ($Driver) {
            foreach ($scope in (Get-RegdiffDriverScopes $machine $opened)) { $scopes.Add($scope) }
        }
        return (New-RegdiffSnapshot -Scopes $scopes.ToArray() -Hidden $hidden -Key $Key -Driver:$Driver)
    } finally {
        # Not $key: PowerShell's names ignore case, so that would assign each RegistryKey to the
        # [byte[]] $Key parameter, and the conversion would throw.
        foreach ($openKey in $opened) { Close-RegdiffKey $openKey }
    }
}

# --- Snapshots as JSON -----------------------------------------------------------------------------

function ConvertTo-RegdiffJson($Snapshot) {
    # One line. Every string in it is a shown form, label or state, so it holds ASCII only.
    return (ConvertTo-Json -InputObject $Snapshot -Compress -Depth 8)
}

function Get-RegdiffField($Object, [string] $Name) {
    # A parsed JSON object's field, kept whole; $null when it is missing.
    if ($Object -isnot [Management.Automation.PSCustomObject]) { return $null }
    foreach ($property in $Object.PSObject.Properties) {
        if ([string]::Equals($property.Name, $Name, [StringComparison]::Ordinal)) { return ,$property.Value }
    }
    return $null
}

function ConvertTo-RegdiffInteger($Value) {
    # A whole JSON number as a long (PowerShell 7 parses longs, 5.1 ints); $null for anything else.
    if ($Value -is [int] -or $Value -is [long]) { return [long]$Value }
    return $null
}

function Test-RegdiffShownName([string] $Text) {
    # A value's name in a snapshot: plain by the names' test (a snapshot holds no hidden name to
    # compare), <default>, or a name token.
    if ($Text.Length -gt 0 -and (Test-RegdiffPlainName $Text $null)) { return $true }
    return ($Text -ceq '<default>' -or $Text -cmatch '^<name len=[0-9]{1,10} hmac=[0-9a-f]{8}>\z')
}

function Test-RegdiffShownKey([string] $Text) {
    # A key's label: HKCU or HKLM, then names as they show, the display class's GUID and its
    # 00NN[VEN_xxxx&DEV_xxxx] keys.
    $parts = $Text.Split([char]'\')
    if ($parts.Length -lt 2 -or ($parts[0] -cne 'HKCU' -and $parts[0] -cne 'HKLM')) { return $false }
    for ($i = 1; $i -lt $parts.Length; $i++) {
        $part = $parts[$i]
        if ($part -ceq '<default>') { return $false }
        if (Test-RegdiffShownName $part) { continue }
        if ($part -ceq (Get-RegdiffDisplayClass)) { continue }
        if ($part -cmatch '^[0-9]{4}\[(VEN_[0-9A-F]{4}&DEV_[0-9A-F]{4}|\?)\]\z') { continue }
        return $false
    }
    return $true
}

function Test-RegdiffShownValue([string] $Text) {
    # "TYPE shown", as a snapshot holds a value: a number, a token, <no data>, or a string or list
    # whose text passes the data's plain test.
    if ($Text.Length -gt 300) { return $false }
    $match = [regex]::Match($Text, '^REG_(SZ|EXPAND_SZ|MULTI_SZ|DWORD|QWORD|BINARY|NONE|UNKNOWN) (.*)\z')
    if (-not $match.Success) { return $false }
    $shown = $match.Groups[2].Value
    if ($shown -cmatch '^([0-9]{1,20}|<(str|bin) len=[0-9]{1,10} hmac=[0-9a-f]{8}>|<no data>)\z') { return $true }
    $quoted = [regex]::Match($shown, '^"([^"]*)"\z')
    if ($quoted.Success) { return (Test-RegdiffPlainData $quoted.Groups[1].Value $null) }
    # A list, ["a","b"]: its strings hold no quote, so they split at ","; joined by commas, they
    # must pass as the walk's did.
    $list = [regex]::Match($shown, '^\[("[^"]*"(,"[^"]*")*)?\]\z')
    if (-not $list.Success) { return $false }
    $inner = $list.Groups[1].Value
    if ($inner.Length -eq 0) { return $true }
    $items = $inner.Substring(1, $inner.Length - 2).Split([string[]]@('","'), [StringSplitOptions]::None)
    return (Test-RegdiffPlainData ([string]::Join(',', $items)) $null)
}

function Read-RegdiffSnapshot([string] $Text, [string] $What = 'the snapshot') {
    # A snapshot's JSON, checked field by field and turned back into New-RegdiffSnapshot's shape.
    # Every key, name and value must be in a shown form, so no line made from it can carry anything
    # else. Anything that isn't a snapshot throws exit 2, naming $What and never the text.
    if ($null -eq $Text) { $Text = '' }
    $Text = $Text.Trim([char[]]@([char]0xFEFF, ' ', [char]9, [char]10, [char]13))
    $bad = $What + ' isn''t a regdiff snapshot'
    if ($Text.Length -eq 0 -or $Text[0] -ne [char]'{') { throw (New-RegdiffError 2 ($bad + ': not a JSON object')) }
    $parsed = $null
    try {
        $parsed = ConvertFrom-Json -InputObject $Text -ErrorAction Stop
    } catch {
        throw (New-RegdiffError 2 ($bad + ': not valid JSON'))
    }
    if ((ConvertTo-RegdiffInteger (Get-RegdiffField $parsed 'kairo_regdiff')) -ne 1) { throw (New-RegdiffError 2 ($bad + ': no "kairo_regdiff":1')) }
    # The fields in New-RegdiffSnapshot's order, so a snapshot read back writes the same JSON.
    $snapshot = [ordered]@{}
    $snapshot['kairo_regdiff'] = 1
    $keyId = Get-RegdiffField $parsed 'key_id'
    if ($keyId -isnot [string] -or $keyId -cnotmatch '^[0-9a-f]{16}\z') { throw (New-RegdiffError 2 ($bad + ': no "key_id" of 16 hex digits')) }
    $snapshot['key_id'] = $keyId
    foreach ($name in @('driver', 'max_depth', 'max_values', 'value_count', 'cut')) {
        $field = Get-RegdiffField $parsed $name
        if ($name -ceq 'driver' -or $name -ceq 'cut') {
            if ($field -isnot [bool]) { throw (New-RegdiffError 2 ($bad + ': "' + $name + '" isn''t true or false')) }
            $snapshot[$name] = $field
            continue
        }
        $number = ConvertTo-RegdiffInteger $field
        if ($null -eq $number -or $number -lt 0 -or ($number -eq 0 -and $name -cne 'value_count')) {
            throw (New-RegdiffError 2 ($bad + ': "' + $name + '" isn''t a whole number'))
        }
        $snapshot[$name] = $number
    }
    $scopes = Get-RegdiffField $parsed 'scopes'
    if ($scopes -isnot [Array]) { throw (New-RegdiffError 2 ($bad + ': "scopes" isn''t a list')) }
    $labels = [Collections.Generic.List[string]]::new()
    foreach ($scope in $scopes) {
        if ($scope -isnot [string] -or -not (Test-RegdiffShownKey $scope)) { throw (New-RegdiffError 2 ($bad + ': a scope isn''t a key as lines show it')) }
        $labels.Add($scope)
    }
    $keys = Get-RegdiffField $parsed 'keys'
    if ($keys -isnot [Array]) { throw (New-RegdiffError 2 ($bad + ': "keys" isn''t a list')) }
    $entries = [Collections.Generic.List[object]]::new()
    $count = 0
    foreach ($item in $keys) {
        $label = Get-RegdiffField $item 'key'
        if ($label -isnot [string] -or -not (Test-RegdiffShownKey $label)) { throw (New-RegdiffError 2 ($bad + ': a key isn''t a key as lines show it')) }
        $state = Get-RegdiffField $item 'state'
        if (@('read', 'absent', 'denied', 'deep', 'cut') -cnotcontains $state) { throw (New-RegdiffError 2 ($bad + ': a key''s state isn''t read, absent, denied, deep or cut')) }
        $entry = [ordered]@{}
        $entry['key'] = $label
        $entry['state'] = $state
        $values = Get-RegdiffField $item 'values'
        if ($state -ceq 'read') {
            if ($values -isnot [Management.Automation.PSCustomObject]) { throw (New-RegdiffError 2 ($bad + ': a read key has no "values" object')) }
            $map = [ordered]@{}
            foreach ($property in $values.PSObject.Properties) {
                if (-not (Test-RegdiffShownName $property.Name)) { throw (New-RegdiffError 2 ($bad + ': a value''s name isn''t a name as lines show it')) }
                if ($property.Value -isnot [string] -or -not (Test-RegdiffShownValue $property.Value)) { throw (New-RegdiffError 2 ($bad + ': a value isn''t a type and data as lines show them')) }
                $map[$property.Name] = $property.Value
            }
            $count += $map.get_Count()
            $entry['values'] = $map
        } elseif ($null -ne $values) {
            throw (New-RegdiffError 2 ($bad + ': a key that wasn''t read has values'))
        }
        $entries.Add($entry)
    }
    if ($count -ne $snapshot['value_count']) { throw (New-RegdiffError 2 ($bad + ': "value_count" doesn''t match its values')) }
    $snapshot['scopes'] = $labels.ToArray()
    $snapshot['keys'] = $entries.ToArray()
    return $snapshot
}

# --- The comparison --------------------------------------------------------------------------------

function Get-RegdiffIndex($Snapshot) {
    # A snapshot's values by path (key\name, ignoring case, as the registry does) and its keys' states.
    $values = [Collections.Generic.Dictionary[string, object]]::new([StringComparer]::OrdinalIgnoreCase)
    $states = [Collections.Generic.Dictionary[string, string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($entry in $Snapshot['keys']) {
        $label = [string]$entry['key']
        if (-not $states.ContainsKey($label)) { $states[$label] = [string]$entry['state'] }
        if ($entry['state'] -cne 'read') { continue }
        foreach ($pair in $entry['values'].GetEnumerator()) {
            $path = $label + '\' + [string]$pair.Key
            if (-not $values.ContainsKey($path)) { $values[$path] = [pscustomobject]@{ Key = $label; Shown = $path; Text = [string]$pair.Value } }
        }
    }
    return [pscustomobject]@{ Values = $values; States = $states; Cut = [bool]$Snapshot['cut'] }
}

function Test-RegdiffUnread($Index, [string] $KeyLabel) {
    # Whether a snapshot didn't read the key: it or a key above it was denied, past the depth limit
    # or where the walk stopped; or the walk stopped before reaching it.
    $label = $KeyLabel
    while ($true) {
        $state = ''
        if ($Index.States.TryGetValue($label, [ref] $state)) {
            if ($state -ceq 'denied' -or $state -ceq 'deep' -or $state -ceq 'cut') { return $true }
        }
        $cut = $label.LastIndexOf([char]'\')
        if ($cut -le 0) { break }
        $label = $label.Substring(0, $cut)
    }
    if ($Index.Cut) {
        $own = ''
        if (-not $Index.States.TryGetValue($KeyLabel, [ref] $own) -or $own -cne 'read') { return $true }
    }
    return $false
}

function Format-RegdiffChange([string] $Old, [string] $New) {
    # "before -> after" for one value from its "TYPE shown" texts ('' when absent): the data, or
    # the types with the data when the type changed.
    if ($Old.Length -eq 0) { return ('(absent) -> ' + $New.Substring($New.IndexOf(' ') + 1)) }
    if ($New.Length -eq 0) { return ($Old.Substring($Old.IndexOf(' ') + 1) + ' -> (absent)') }
    $oldType = $Old.Substring(0, $Old.IndexOf(' '))
    $newType = $New.Substring(0, $New.IndexOf(' '))
    if ($oldType -ceq $newType) { return ($Old.Substring($Old.IndexOf(' ') + 1) + ' -> ' + $New.Substring($New.IndexOf(' ') + 1)) }
    return ($Old + ' -> ' + $New)
}

function Add-RegdiffLine($Sorted, [string] $Path, [string] $Line) {
    # Lines sort by path ignoring case, a key's own line before those below it, then exactly.
    $Sorted.Add([pscustomobject]@{ Sort = ($Path.ToUpperInvariant() + [char]0 + $Line); Line = $Line })
}

function Compare-RegdiffSnapshot($Before, $After) {
    # The lines of section 5.5 for two snapshots: one per changed value, one per key either didn't
    # read, sorted by path, then "N values changed". Returns Lines and Changed.
    if ([string]$Before['key_id'] -cne [string]$After['key_id']) {
        throw (New-RegdiffError 2 'the snapshots were taken with different keys, so their hashes don''t compare: take both with one key')
    }
    if ([bool]$Before['driver'] -ne [bool]$After['driver']) {
        throw (New-RegdiffError 2 'the snapshots read different scopes: one was taken with --driver and one without')
    }
    if ([long]$Before['max_depth'] -ne [long]$After['max_depth'] -or [long]$Before['max_values'] -ne [long]$After['max_values']) {
        throw (New-RegdiffError 2 'the snapshots were taken with different limits')
    }
    $inv = [Globalization.CultureInfo]::InvariantCulture
    $old = Get-RegdiffIndex $Before
    $new = Get-RegdiffIndex $After
    $sorted = [Collections.Generic.List[object]]::new()
    $paths = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($path in $old.Values.get_Keys()) { [void]$paths.Add($path) }
    foreach ($path in $new.Values.get_Keys()) { [void]$paths.Add($path) }
    $changed = 0
    foreach ($path in $paths) {
        $was = $null
        $is = $null
        [void]$old.Values.TryGetValue($path, [ref] $was)
        [void]$new.Values.TryGetValue($path, [ref] $is)
        $wasText = ''
        $isText = ''
        $key = ''
        $shown = ''
        if ($null -ne $was) { $wasText = $was.Text; $key = $was.Key; $shown = $was.Shown }
        if ($null -ne $is) { $isText = $is.Text; $key = $is.Key; $shown = $is.Shown }
        if ([string]::Equals($wasText, $isText, [StringComparison]::Ordinal)) { continue }
        if ((Test-RegdiffUnread $old $key) -or (Test-RegdiffUnread $new $key)) { continue }
        Add-RegdiffLine $sorted $shown ($shown + ': ' + (Format-RegdiffChange $wasText $isText))
        $changed++
    }
    $notes = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($index in @($old, $new)) {
        foreach ($pair in $index.States.GetEnumerator()) {
            $note = ''
            if ($pair.Value -ceq 'denied') { $note = '<denied>' }
            elseif ($pair.Value -ceq 'deep') { $note = '<past depth ' + ([long]$Before['max_depth']).ToString($inv) + '>' }
            elseif ($pair.Value -ceq 'cut') { $note = '<stopped at ' + ([long]$Before['max_values']).ToString($inv) + ' values>' }
            if ($note.Length -gt 0 -and $notes.Add($pair.Key + ': ' + $note)) { Add-RegdiffLine $sorted $pair.Key ($pair.Key + ': ' + $note) }
        }
    }
    $keys = [string[]]::new($sorted.Count)
    $lines = [string[]]::new($sorted.Count)
    for ($i = 0; $i -lt $sorted.Count; $i++) {
        $keys[$i] = $sorted[$i].Sort
        $lines[$i] = $sorted[$i].Line
    }
    [Array]::Sort($keys, $lines, [StringComparer]::Ordinal)
    $out = [Collections.Generic.List[string]]::new()
    foreach ($line in $lines) { $out.Add($line) }
    $out.Add($changed.ToString($inv) + ' values changed')
    return [pscustomobject]@{ Lines = $out.ToArray(); Changed = $changed }
}

function Format-RegdiffCount([long] $Count, [string] $One, [string] $Many) {
    # "1 value", "2 values": the summary's counts. (The lines' last one always says "values".)
    $noun = $Many
    if ($Count -eq 1) { $noun = $One }
    return ($Count.ToString([Globalization.CultureInfo]::InvariantCulture) + ' ' + $noun)
}

function Format-RegdiffSummary($Snapshot) {
    # The terminal's view of a snapshot: how much each scope held. No value or name is shown.
    $inv = [Globalization.CultureInfo]::InvariantCulture
    $entries = @($Snapshot['keys'])
    $read = 0
    foreach ($entry in $entries) { if ($entry['state'] -ceq 'read') { $read++ } }
    $head = 'snapshot: ' + (Format-RegdiffCount $Snapshot['value_count'] 'value' 'values') + ' in ' + (Format-RegdiffCount $read 'key' 'keys')
    if ($Snapshot['driver']) { $head += ', display-class keys included' }
    $lines = [Collections.Generic.List[string]]::new()
    $lines.Add($head)
    foreach ($scope in $Snapshot['scopes']) {
        $state = ''
        $values = 0
        $keys = 0
        $denied = 0
        $deep = 0
        $cut = $false
        foreach ($entry in $entries) {
            $label = [string]$entry['key']
            $own = [string]::Equals($label, $scope, [StringComparison]::OrdinalIgnoreCase)
            if (-not $own -and -not $label.StartsWith($scope + '\', [StringComparison]::OrdinalIgnoreCase)) { continue }
            if ($own) { $state = [string]$entry['state'] }
            if ($entry['state'] -ceq 'read') { $keys++; $values += $entry['values'].get_Count() }
            elseif ($entry['state'] -ceq 'denied') { $denied++ }
            elseif ($entry['state'] -ceq 'deep') { $deep++ }
            elseif ($entry['state'] -ceq 'cut') { $cut = $true }
        }
        $text = '  ' + $scope + ': '
        if ($state.Length -eq 0) { $text += 'not read (the walk had stopped)' }
        elseif ($state -ceq 'absent') { $text += '(absent)' }
        elseif ($state -ceq 'denied') { $text += '<denied>' }
        else {
            $text += (Format-RegdiffCount $values 'value' 'values') + ' in ' + (Format-RegdiffCount $keys 'key' 'keys')
            if ($denied -gt 0) { $text += ', ' + $denied.ToString($inv) + ' denied' }
            if ($deep -gt 0) { $text += ', ' + $deep.ToString($inv) + ' past depth ' + ([long]$Snapshot['max_depth']).ToString($inv) }
            if ($cut) { $text += ', stopped at ' + ([long]$Snapshot['max_values']).ToString($inv) + ' values' }
        }
        $lines.Add($text)
    }
    return ,$lines.ToArray()
}

# --- The command line ------------------------------------------------------------------------------

function Read-RegdiffFile([string] $File) {
    # A snapshot file's text (UTF-8), at most 64 MiB, with errors that name the file only. A relative
    # name is taken from the current folder.
    $leaf = Get-RegdiffLeaf $File
    $bytes = $null
    try {
        $full = [IO.Path]::GetFullPath([IO.Path]::Combine((Get-Location -PSProvider FileSystem).ProviderPath, $File))
        $info = [IO.FileInfo]::new($full)
        if (-not $info.Exists) { throw (New-RegdiffError 2 ('no such file: ' + $leaf)) }
        if ($info.Length -gt 67108864) { throw (New-RegdiffError 2 ($leaf + ' is larger than 64 MiB')) }
        $bytes = [IO.File]::ReadAllBytes($full)
    } catch {
        if ($_.Exception.Data.Contains('KairoExit')) { throw }
        throw (New-RegdiffError 2 ('can''t read ' + $leaf + ' (' + $_.Exception.GetType().Name + ')'))
    }
    return [Text.UTF8Encoding]::new($false).GetString($bytes)
}

function ConvertFrom-RegdiffArguments([object[]] $Arguments) {
    # pwsh -File passes every argument as text: -Name:value arrives as -Name and value, and
    # -Name:$false as -Name and a bool. & passes -Name: and the value, a switch's as a bool.
    # Names are matched whole, ignoring case; anything else is refused.
    $values = @('Before', 'After', 'Key')
    $switches = @('Snapshot', 'Driver')
    $parsed = @{}
    $i = 0
    while ($i -lt $Arguments.Count) {
        $token = $Arguments[$i]
        $i++
        $match = $null
        if ($token -is [string]) { $match = [regex]::Match($token, '^-([A-Za-z]+):?\z') }
        if ($null -eq $match -or -not $match.Success) {
            throw (New-RegdiffError 2 ('unexpected argument at position ' + $i))
        }
        $name = $null
        foreach ($known in ($values + $switches)) {
            if ([string]::Equals($known, $match.Groups[1].Value, [StringComparison]::OrdinalIgnoreCase)) { $name = $known }
        }
        if ($null -eq $name) { throw (New-RegdiffError 2 ('unknown parameter -' + $match.Groups[1].Value)) }
        if ($parsed.ContainsKey($name)) { throw (New-RegdiffError 2 ('-' + $name + ' given twice')) }
        if ($switches -contains $name) {
            $parsed[$name] = $true
            if ($i -lt $Arguments.Count -and $Arguments[$i] -is [bool]) {
                $parsed[$name] = $Arguments[$i]
                $i++
            }
            continue
        }
        if ($i -ge $Arguments.Count) { throw (New-RegdiffError 2 ('-' + $name + ' needs a value')) }
        $parsed[$name] = $Arguments[$i]
        $i++
    }
    return $parsed
}

# --- Command line: only when the file is run, never when it is dot-sourced. -----------------
if ($MyInvocation.InvocationName -ne '.') {
    Set-StrictMode -Version 3.0
    $ErrorActionPreference = 'Stop'
    $regdiffCode = 0
    try {
        $regdiffArgs = ConvertFrom-RegdiffArguments $args
        if ($regdiffArgs.ContainsKey('Snapshot') -and [bool]$regdiffArgs['Snapshot']) {
            if ($regdiffArgs.ContainsKey('Before') -or $regdiffArgs.ContainsKey('After')) { throw (New-RegdiffError 2 '-Snapshot takes no -Before or -After') }
            $regdiffKey = $null
            if ($regdiffArgs.ContainsKey('Key')) {
                try {
                    $regdiffKey = ConvertTo-RegdiffKey ([string]$regdiffArgs['Key'])
                } catch {
                    throw (New-RegdiffError 2 '-Key must be 32 hex digits (16 bytes)')
                }
            }
            if (-not (Test-RegdiffWindows)) { throw (New-RegdiffError 1 'the registry exists only on Windows') }
            if ($null -eq $regdiffKey) {
                $regdiffKey = New-RegdiffKey
                [Console]::Error.WriteLine('regdiff: no -Key, so this snapshot''s hashes use a random key that isn''t shown or kept: it compares with no other snapshot')
            }
            $regdiffDriver = $regdiffArgs.ContainsKey('Driver') -and [bool]$regdiffArgs['Driver']
            $regdiffSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
            [Console]::Out.WriteLine((ConvertTo-RegdiffJson (Get-RegdiffSnapshot -OwnerSid $regdiffSid -Driver:$regdiffDriver -Key $regdiffKey)))
        } else {
            if ($regdiffArgs.ContainsKey('Driver')) { throw (New-RegdiffError 2 '-Driver goes with -Snapshot') }
            if ($regdiffArgs.ContainsKey('Key')) { throw (New-RegdiffError 2 '-Key goes with -Snapshot: two snapshots compare by the key id they hold') }
            foreach ($required in @('Before', 'After')) {
                if (-not $regdiffArgs.ContainsKey($required) -or [string]::IsNullOrEmpty([string]$regdiffArgs[$required])) {
                    throw (New-RegdiffError 2 ('missing -' + $required + ' (or -Snapshot)'))
                }
            }
            $regdiffBeforeFile = [string]$regdiffArgs['Before']
            $regdiffAfterFile = [string]$regdiffArgs['After']
            $regdiffBefore = Read-RegdiffSnapshot (Read-RegdiffFile $regdiffBeforeFile) (Get-RegdiffLeaf $regdiffBeforeFile)
            $regdiffAfter = Read-RegdiffSnapshot (Read-RegdiffFile $regdiffAfterFile) (Get-RegdiffLeaf $regdiffAfterFile)
            foreach ($line in (Compare-RegdiffSnapshot $regdiffBefore $regdiffAfter).Lines) { [Console]::Out.WriteLine($line) }
        }
    } catch {
        $regdiffCode = Get-RegdiffExitCode $_
        $message = $_.Exception.Message
        if (-not $_.Exception.Data.Contains('KairoExit')) {
            # A bug. Its message could quote a path, a SID or a name from the registry, so only its
            # type and line are printed.
            $line = 0
            if ($null -ne $_.InvocationInfo) { $line = $_.InvocationInfo.ScriptLineNumber }
            $message = 'internal error: ' + $_.Exception.GetType().Name + ' at line ' + $line
        }
        [Console]::Error.WriteLine('regdiff: ' + $message)
    }
    exit $regdiffCode
}
