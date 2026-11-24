# SPDX-License-Identifier: GPL-3.0-or-later
#
# tools/pc/bench/tests/scans.tests.ps1 - research 17 section 6's scans of the PowerShell scripts.
# Project: https://github.com/Maty3k/Kairo
# Design: docs/research/17-loadgen-bench/README.md sections 2 (item 1), 5 and 6.
#
#   pwsh -NoProfile -File tools/pc/bench/tests/scans.tests.ps1
#
# CI's house-rules scan reads only the Rust files under crates/ (M1, How M1 jobs run on the PC:
# Privacy), so these scans keep the Never list in tools/**/*.ps1. They walk the syntax tree that
# System.Management.Automation.Language.Parser builds, never the text: a regular expression would
# trip on [IO.Path]:: and miss a member written in quotes. Each rule runs over the real files, and
# over synthetic snippets, some that it must flag and some that it must pass, so a broken scan fails
# a case. It prints one line per case and exits 0 when every case passed, 1 when one failed.
#
# The scripts each rule covers:
# - the bench scripts: tools/pc/bench/*.ps1, the libraries the elevated session dot-sources;
# - the elevated scripts: the bench scripts and tools/mac/remote/*.ps1, the verb bodies the elevated
#   SSH session runs;
# - the elevated bench scripts: the bench scripts and the bench verbs, tools/mac/remote/bench*.ps1;
# - the registry readers: tools/pc/bench/regdiff.ps1 and tools/mac/remote/amd-diff.ps1, the AMD
#   settings diff;
# - every script: tools/**/*.ps1, the tests included.
#
# A command's module qualifier is dropped before its name is matched, in every rule:
# Microsoft.PowerShell.Management\Get-Process is Get-Process.
#
# R1 Module lists (CLAUDE.md, Never 3), in the bench scripts: no member named Path, MainModule,
#    Modules, Company, FileVersion, ProductVersion, Description, Product or CommandLine on anything
#    but a type literal ([IO.Path]::Combine is fine), whether written $p.Path, $p.'Path',
#    $p.get_Path(), $p.PSObject.Properties['Path'] or as a cmdlet's property argument (Select-Object
#    Path, ForEach-Object Path), a wildcard that matches one included (Select-Object *, P*); and no
#    member name computed at run time. On a process the first eight go through MainModule, which
#    opens it with memory-read rights and lists its modules, and CommandLine is a WMI query for its
#    command line (research 17 section 5.3). Nor anything that gives a Process object, whose every
#    property a whole-object read (Format-List *, ConvertTo-Json) would read: Get-Process (gps, ps),
#    GetProcessById or GetProcessesByName. Get-ProcessIdAndName, R2's one helper, gives names.
# R2 Process lists, in every script: GetProcesses only in the one helper, a function that loops
#    over it reading only Id and ProcessName (and Dispose) of each process and returns
#    [pscustomobject]@{ Id; Name }; Get-Process only by name or id (status.ps1's LogonUI lookup
#    stays allowed), never matching every name, and never with -Module or -FileVersionInfo, which
#    list modules; Get-CimInstance or Get-WmiObject over Win32_Process only with a WHERE clause,
#    since without one it lists every program's path and command line.
# R3 Add-Type and Import-Csv, in the elevated scripts: Add-Type compiles in a folder the owner's
#    programs can write (M1, Carried over from M0), and Import-Csv opens a path by its name, which a
#    link in logs\ could redirect (research 17 section 2 item 16). The bootstrap, which the owner
#    runs, keeps its Add-Type. Synthetic input (CLAUDE.md, Never 4: SendInput, SendKeys, SendWait,
#    keybd_event, mouse_event) in every script, as a command or member name, and in strings outside
#    the tests, where an Add-Type definition would carry it.
# R4 File writes, in the elevated bench scripts: none, except Write-TextFileAtomic's decision file
#    (research 17 sections 4.2 and 4.6), and only when its path (-Path, else the first positional
#    argument) names that file: a string ending in .decision.json, or a variable named
#    $decisionFile or $decisionPath. Cmdlets that write files, items or properties; Start-Process
#    (start, saps) with -RedirectStandardOutput or -RedirectStandardError (RSO, RSE) or splatted
#    arguments; a > or >> redirection other than to $null; [IO.File] and [IO.Directory] beyond their
#    reading members; FileStream, StreamWriter or BinaryWriter in any form, a conversion
#    ([IO.StreamWriter]$f creates the file), -as or a typed variable included; members that write,
#    move or delete, Create among them; and Open, unless it is Open(Open, Read) written out
#    (FileInfo.Open(mode) alone opens for reading and writing). A type may be written with or
#    without its namespace (using namespace System.IO).
# R5 Dot-sourcing safely (research 17 section 2 item 1), in the bench scripts: function definitions
#    and, last, one command line guarded by $MyInvocation.InvocationName -ne '.'; no script param
#    block (dot-sourcing would bind it), no begin or process block, no exit outside the guard, no
#    Mandatory parameter and no Read-Host (-NonInteractive can't prompt).
# R6 Windows PowerShell 5.1's grammar, in every script: no ?:, &&, ||, ??, ??=, ?., ?[, background
#    &, `u{} escape or number suffix (y, uy, s, us, u, ul, n, also before a multiplier: 1ukb), which
#    PowerShell 7 added. CI parses every script with 5.1 on Windows; this rule catches them on the
#    Mac and on Linux too.
#
# R7 Registry writes, in the registry readers (research 17 section 5.5: AMD's settings are never
#    written): no cmdlet that writes items or properties (Set-ItemProperty, New-Item,
#    New-ItemProperty, Remove-Item, their kin and their aliases), no reg.exe, regedit or regini, by
#    name or path, run or launched (Start-Process, cmd /c); no SetValue, CreateSubKey, DeleteValue, DeleteSubKey, DeleteSubKeyTree or
#    SetAccessControl, on a key or on [Microsoft.Win32.Registry]; OpenSubKey only with $false as its
#    second and last argument, since $true, a variable or a permission check can open a key for
#    writing; and no member name computed at run time.
#
# Rules: ASCII only; Windows PowerShell 5.1 syntax (CI parses tools/** there); no Add-Type. The
# snippets below are strings, which these rules never read as code, except R3's strings check,
# which skips the tests.

. (Join-Path $PSScriptRoot 'testlib.ps1')

$script:ToolsDir = [IO.Path]::GetFullPath((Join-Path (Join-Path $BenchDir '..') '..'))
$script:RepoDir = [IO.Path]::GetFullPath((Join-Path $script:ToolsDir '..'))

# --- Sources: a file or a snippet, parsed ------------------------------------------------------

function New-ScanSource([string] $Name, $Ast, $Tokens, [bool] $IsTest) {
    return [pscustomobject]@{ Name = $Name; Ast = $Ast; Tokens = $Tokens; IsTest = $IsTest }
}

function Get-ScanName([string] $Full) {
    # A file's path from the repository's root, with forward slashes.
    $relative = $Full.Substring($script:RepoDir.TrimEnd([char]'\', [char]'/').Length).TrimStart([char]'\', [char]'/')
    return $relative.Replace('\', '/')
}

function Read-ScanFile([string] $Full) {
    $tokens = $null
    $errors = $null
    $ast = [Management.Automation.Language.Parser]::ParseFile($Full, [ref] $tokens, [ref] $errors)
    $name = Get-ScanName $Full
    if (@($errors).Count -gt 0) { throw ($name + ' does not parse: ' + $errors[0].Message) }
    return (New-ScanSource $name $ast $tokens ($name -match '/tests/'))
}

function ConvertTo-ScanSource([string] $Text, [string] $Name = 'snippet.ps1', [switch] $IsTest) {
    $tokens = $null
    $errors = $null
    $ast = [Management.Automation.Language.Parser]::ParseInput($Text, [ref] $tokens, [ref] $errors)
    if (@($errors).Count -gt 0) { throw ('a snippet does not parse (' + $errors[0].Message + '): ' + $Text) }
    return (New-ScanSource $Name $ast $tokens $IsTest.IsPresent)
}

function New-ScanFinding($Source, $Node, [string] $What) {
    return ($Source.Name + ':' + $Node.Extent.StartLineNumber + ': ' + $What)
}

function Find-ScanNodes($Ast, [type] $Type) {
    # Every node of that type in the tree, nested script blocks included.
    return @($Ast.FindAll({ param($n) $n -is $Type }.GetNewClosure(), $true))
}

function Get-ScanStrings($Node) {
    # The string constants an argument holds: itself, or the items of an array literal.
    $out = [Collections.Generic.List[string]]::new()
    if ($Node -is [Management.Automation.Language.CommandParameterAst]) {
        if ($null -eq $Node.Argument) { return ,$out.ToArray() }
        $Node = $Node.Argument
    }
    if ($Node -is [Management.Automation.Language.StringConstantExpressionAst]) {
        $out.Add($Node.Value)
    } elseif ($Node -is [Management.Automation.Language.ArrayLiteralAst]) {
        foreach ($item in $Node.Elements) {
            if ($item -is [Management.Automation.Language.StringConstantExpressionAst]) { $out.Add($item.Value) }
        }
    }
    return ,$out.ToArray()
}

function Find-ScanQuotedStrings($Ast) {
    # The strings written in quotes, here-strings included: not the bare words that name a
    # command, a member or an unquoted argument.
    return @($Ast.FindAll({ param($n) ($n -is [Management.Automation.Language.StringConstantExpressionAst] -and
                [string]$n.StringConstantType -ne 'BareWord') -or $n -is [Management.Automation.Language.ExpandableStringExpressionAst] }, $true))
}

function Get-ScanMemberName($Node) {
    # A member expression's name, or $null when it is computed at run time.
    if ($Node.Member -is [Management.Automation.Language.StringConstantExpressionAst]) { return $Node.Member.Value }
    return $null
}

function Test-ScanInside($Inner, $Outer) {
    return ($Outer.Extent.StartOffset -le $Inner.Extent.StartOffset -and $Inner.Extent.EndOffset -le $Outer.Extent.EndOffset)
}

function Get-ScanFunction($Node) {
    # The function definition that holds the node, or $null at a script's top level.
    $parent = $Node.Parent
    while ($null -ne $parent -and $parent -isnot [Management.Automation.Language.FunctionDefinitionAst]) { $parent = $parent.Parent }
    return $parent
}

function Get-ScanCommandName($Command) {
    # A command's name without its module qualifier, the text after the last '\'
    # (Microsoft.PowerShell.Management\Get-Process runs Get-Process), or $null when the name is
    # computed at run time.
    $name = $Command.GetCommandName()
    if ($null -eq $name) { return $null }
    return $name.Substring($name.LastIndexOf([char]'\') + 1)
}

function Test-ScanIoType([string] $TypeName, [string[]] $Names) {
    # Whether a type name is one of these System.IO types, or an array of one, written with its
    # namespace, with IO. alone, or bare after 'using namespace System.IO'.
    return ($TypeName -match ('^((System\.)?IO\.)?(' + ($Names -join '|') + ')(\[\])?\z'))
}

# --- R1 module lists ---------------------------------------------------------------------------

$script:ModuleMembers = @('Path', 'MainModule', 'Modules', 'Company', 'FileVersion', 'ProductVersion', 'Description', 'Product', 'CommandLine')
# Cmdlets (and aliases) that read the properties their arguments name.
$script:PropertyCommands = @('ForEach-Object', '%', 'foreach', 'Select-Object', 'select', 'Sort-Object', 'sort',
    'Group-Object', 'group', 'Where-Object', '?', 'where', 'Measure-Object', 'measure', 'Format-Table', 'ft',
    'Format-List', 'fl', 'Format-Wide', 'fw', 'Format-Custom', 'fc', 'Compare-Object', 'compare', 'diff',
    'Export-Csv', 'epcsv', 'ConvertTo-Csv', 'ConvertTo-Html', 'Out-GridView', 'ogv')
# What gives a Process object, other than R2's one helper, which reads only Id and ProcessName.
$script:ProcessCommands = @('Get-Process', 'gps', 'ps')
$script:ProcessGetters = @('GetProcessById', 'GetProcessesByName')

function Get-ScanPropertyProblem([string] $Name) {
    # $null unless a property argument reads a banned member: by its name, or as a wildcard that
    # matches one (*, P*, Pa?h, [P]ath). A wildcard that doesn't parse counts as matching.
    if ($script:ModuleMembers -contains $Name) { return $Name }
    if ($Name.IndexOfAny([char[]]@('*', '?', '[')) -lt 0) { return $null }
    $pattern = [Management.Automation.WildcardPattern]::new($Name, [Management.Automation.WildcardOptions]::IgnoreCase)
    $hits = [Collections.Generic.List[string]]::new()
    foreach ($member in $script:ModuleMembers) {
        $isMatch = $true
        try { $isMatch = $pattern.IsMatch($member) } catch { $isMatch = $true }
        if ($isMatch) { $hits.Add($member) }
    }
    if ($hits.Count -eq 0) { return $null }
    return ($Name + ', which matches ' + ($hits -join ', '))
}

function Find-ScanModuleMembers($Source) {
    $found = [Collections.Generic.List[string]]::new()
    foreach ($node in (Find-ScanNodes $Source.Ast ([Management.Automation.Language.MemberExpressionAst]))) {
        $name = Get-ScanMemberName $node
        if ($null -ne $name -and $script:ProcessGetters -contains $name) {
            # [Diagnostics.Process]::GetProcessById: on a type literal too.
            $found.Add((New-ScanFinding $Source $node ('.' + $name + ', which gives Process objects: Get-ProcessIdAndName gives names')))
            continue
        }
        if ($node.Expression -is [Management.Automation.Language.TypeExpressionAst]) { continue }
        if ($null -eq $name) {
            $found.Add((New-ScanFinding $Source $node ('a member name computed at run time: ' + $node.Extent.Text)))
        } elseif ($script:ModuleMembers -contains $name) {
            $found.Add((New-ScanFinding $Source $node ('.' + $name)))
        } elseif ($name -match '^get_(.+)\z' -and $script:ModuleMembers -contains $Matches[1]) {
            # A property's getter, $p.get_MainModule(), reads it as the property does.
            $found.Add((New-ScanFinding $Source $node ('.' + $name + '()')))
        }
    }
    foreach ($node in (Find-ScanNodes $Source.Ast ([Management.Automation.Language.IndexExpressionAst]))) {
        foreach ($name in (Get-ScanStrings $node.Index)) {
            if ($script:ModuleMembers -contains $name) { $found.Add((New-ScanFinding $Source $node ('[''' + $name + ''']'))) }
        }
    }
    foreach ($node in (Find-ScanNodes $Source.Ast ([Management.Automation.Language.CommandAst]))) {
        $command = Get-ScanCommandName $node
        if ($null -eq $command) { continue }
        if ($script:ProcessCommands -contains $command) {
            $found.Add((New-ScanFinding $Source $node ($command + ', which gives Process objects: Get-ProcessIdAndName gives names')))
            continue
        }
        if ($script:PropertyCommands -notcontains $command) { continue }
        for ($i = 1; $i -lt $node.CommandElements.Count; $i++) {
            foreach ($name in (Get-ScanStrings $node.CommandElements[$i])) {
                $problem = Get-ScanPropertyProblem $name
                if ($null -ne $problem) { $found.Add((New-ScanFinding $Source $node ($command + ' ' + $problem))) }
            }
        }
    }
    return ,$found.ToArray()
}

# --- R2 process lists --------------------------------------------------------------------------

function Get-ScanHelperProblem($Call) {
    # $null when this GetProcesses call is in the helper research 17 section 6 allows, else what
    # is wrong: it must be the list of a foreach whose variable is only ever read as .Id,
    # .ProcessName or .Dispose(), in a function whose every [pscustomobject] is @{ Id; Name }.
    $function = Get-ScanFunction $Call
    if ($null -eq $function) { return 'outside a function' }
    $loop = $Call.Parent
    while ($null -ne $loop -and $loop -isnot [Management.Automation.Language.ForEachStatementAst]) { $loop = $loop.Parent }
    if ($null -eq $loop -or -not (Test-ScanInside $Call $loop.Condition) -or -not (Test-ScanInside $loop $function)) {
        return 'outside a foreach over its processes'
    }
    $variable = $loop.Variable.VariablePath.UserPath
    foreach ($use in (Find-ScanNodes $loop.Body ([Management.Automation.Language.VariableExpressionAst]))) {
        if ($use.VariablePath.UserPath -ne $variable) { continue }
        $member = $use.Parent
        if ($member -isnot [Management.Automation.Language.MemberExpressionAst] -or $member.Expression -ne $use -or
            @('Id', 'ProcessName', 'Dispose') -notcontains (Get-ScanMemberName $member)) {
            return ('reading more of a process than Id and ProcessName: ' + $use.Parent.Extent.Text)
        }
    }
    $objects = 0
    foreach ($convert in (Find-ScanNodes $function ([Management.Automation.Language.ConvertExpressionAst]))) {
        if (@('pscustomobject', 'PSCustomObject', 'System.Management.Automation.PSCustomObject') -notcontains $convert.Type.TypeName.FullName) { continue }
        $keys = @()
        if ($convert.Child -is [Management.Automation.Language.HashtableAst]) {
            $keys = @($convert.Child.KeyValuePairs | ForEach-Object { $_.Item1.Extent.Text.Trim([char]39, [char]34) } | Sort-Object)
        }
        if (($keys -join ',') -ne 'Id,Name') { return ('returning more than [pscustomobject]@{ Id; Name }: ' + $convert.Extent.Text) }
        $objects++
    }
    if ($objects -eq 0) { return 'in a function that returns no [pscustomobject]@{ Id; Name }' }
    return $null
}

function Test-ScanWildcardOnly($Name) {
    return ($Name -match '^[*?]+\z')
}

function Get-ScanGetProcessProblem($Command) {
    # $null for a lookup by name or id, else what is wrong.
    $valued = @('Name', 'ProcessName', 'Id', 'PID', 'InputObject')
    $common = @('ErrorAction', 'EA', 'ErrorVariable', 'EV', 'WarningAction', 'WA', 'WarningVariable', 'WV',
        'InformationAction', 'InfA', 'InformationVariable', 'IV', 'OutVariable', 'OV', 'OutBuffer', 'OB',
        'PipelineVariable', 'PV', 'ProgressAction', 'proga')
    $filtered = $false
    $elements = $Command.CommandElements
    for ($i = 1; $i -lt $elements.Count; $i++) {
        $element = $elements[$i]
        if ($element -is [Management.Automation.Language.CommandParameterAst]) {
            $parameter = $element.ParameterName
            if (@('Module', 'FileVersionInfo', 'FV', 'FVI') -contains $parameter) { return ('-' + $parameter + ', which lists modules') }
            if ($valued -contains $parameter -or $common -contains $parameter) {
                $value = $element.Argument
                if ($null -eq $value -and $i + 1 -lt $elements.Count) {
                    $i++
                    $value = $elements[$i]
                }
                if ($valued -contains $parameter) {
                    foreach ($name in (Get-ScanStrings $value)) {
                        if (Test-ScanWildcardOnly $name) { return ('-' + $parameter + ' ' + $name + ', which matches every process') }
                    }
                    $filtered = $true
                }
                continue
            }
            if (@('Verbose', 'VB', 'Debug', 'DB') -contains $parameter) { continue }
            return ('-' + $parameter + ': write -Name or -Id')
        }
        if ($element -is [Management.Automation.Language.VariableExpressionAst] -and $element.Splatted) { return 'splatted arguments: write -Name or -Id' }
        # A positional argument binds to -Name.
        foreach ($name in (Get-ScanStrings $element)) {
            if (Test-ScanWildcardOnly $name) { return ($name + ', which matches every process') }
        }
        $filtered = $true
    }
    if (-not $filtered) { return 'no -Name or -Id: it lists every process' }
    return $null
}

function Get-ScanProcessQueryProblem($Command) {
    # $null unless the command queries Win32_Process with neither -Filter nor a WHERE clause.
    $strings = @($Command.FindAll({ param($n) $n -is [Management.Automation.Language.StringConstantExpressionAst] -or $n -is [Management.Automation.Language.ExpandableStringExpressionAst] }, $true))
    $mentions = @($strings | Where-Object { $_.Value -match '(?i)\bWin32_Process\b' })
    if ($mentions.Count -eq 0) { return $null }
    foreach ($element in $Command.CommandElements) {
        if ($element -is [Management.Automation.Language.CommandParameterAst] -and $element.ParameterName -eq 'Filter') { return $null }
    }
    foreach ($mention in $mentions) { if ($mention.Value -match '(?i)\bWHERE\b') { return $null } }
    return 'Win32_Process without a WHERE clause lists every program''s path and command line'
}

function Find-ScanProcessLists([object[]] $Sources) {
    # Over several sources together, since only one helper may exist in all of them.
    $found = [Collections.Generic.List[string]]::new()
    $helpers = [Collections.Generic.List[string]]::new()
    foreach ($source in $Sources) {
        foreach ($node in (Find-ScanNodes $source.Ast ([Management.Automation.Language.MemberExpressionAst]))) {
            if ((Get-ScanMemberName $node) -ne 'GetProcesses') { continue }
            $problem = Get-ScanHelperProblem $node
            if ($null -eq $problem) {
                $helpers.Add($source.Name + ' ' + (Get-ScanFunction $node).Name)
            } else {
                $found.Add((New-ScanFinding $source $node ('GetProcesses ' + $problem)))
            }
        }
        foreach ($node in (Find-ScanNodes $source.Ast ([Management.Automation.Language.CommandAst]))) {
            $command = Get-ScanCommandName $node
            $problem = $null
            if (@('Get-Process', 'gps', 'ps') -contains $command) {
                $problem = Get-ScanGetProcessProblem $node
                if ($null -ne $problem) { $problem = $command + ' ' + $problem }
            } elseif (@('Get-CimInstance', 'gcim', 'Get-WmiObject', 'gwmi') -contains $command) {
                $problem = Get-ScanProcessQueryProblem $node
            }
            if ($null -ne $problem) { $found.Add((New-ScanFinding $source $node $problem)) }
        }
        # A query kept in a variable first; the tests' strings are data.
        if (-not $source.IsTest) {
            foreach ($node in (Find-ScanQuotedStrings $source.Ast)) {
                if ($node.Value -match '(?i)\bFROM\s+Win32_Process\b' -and $node.Value -notmatch '(?i)\bWHERE\b') {
                    $found.Add((New-ScanFinding $source $node 'a query of Win32_Process without a WHERE clause lists every program''s path and command line'))
                }
            }
        }
    }
    if ($helpers.Count -gt 1) { $found.Add('more than one GetProcesses helper: ' + ($helpers -join '; ')) }
    return [pscustomobject]@{ Findings = $found.ToArray(); Helpers = $helpers.ToArray() }
}

# --- R3 Add-Type, Import-Csv and synthetic input -----------------------------------------------

$script:InputNames = @('SendInput', 'SendKeys', 'SendWait', 'keybd_event', 'mouse_event')

function Find-ScanBannedCalls($Source, [switch] $Elevated) {
    $found = [Collections.Generic.List[string]]::new()
    foreach ($node in (Find-ScanNodes $Source.Ast ([Management.Automation.Language.CommandAst]))) {
        $command = Get-ScanCommandName $node
        if ($null -eq $command) { continue }
        if ($Elevated -and @('Add-Type', 'Import-Csv', 'ipcsv') -contains $command) { $found.Add((New-ScanFinding $Source $node $command)) }
        if ($script:InputNames -contains $command) { $found.Add((New-ScanFinding $Source $node ('synthetic input: ' + $command))) }
    }
    foreach ($node in (Find-ScanNodes $Source.Ast ([Management.Automation.Language.MemberExpressionAst]))) {
        $name = Get-ScanMemberName $node
        if ($null -ne $name -and $script:InputNames -contains $name) { $found.Add((New-ScanFinding $Source $node ('synthetic input: .' + $name))) }
    }
    if (-not $Source.IsTest) {
        foreach ($node in (Find-ScanQuotedStrings $Source.Ast)) {
            if ($node.Value -match '(?i)\b(SendInput|SendKeys|keybd_event|mouse_event)\b') { $found.Add((New-ScanFinding $Source $node ('synthetic input in a string: ' + $Matches[1]))) }
        }
    }
    return ,$found.ToArray()
}

# --- R4 file writes ----------------------------------------------------------------------------

$script:WriteCommands = @('Set-Content', 'sc', 'Add-Content', 'ac', 'Clear-Content', 'clc', 'Out-File', 'Export-Csv', 'epcsv',
    'Export-Clixml', 'Tee-Object', 'tee', 'New-Item', 'ni', 'mkdir', 'md', 'Remove-Item', 'ri', 'rm', 'rmdir', 'del', 'erase',
    'rd', 'Copy-Item', 'cpi', 'cp', 'copy', 'Move-Item', 'mi', 'mv', 'move', 'Rename-Item', 'rni', 'ren', 'Set-Item', 'si',
    'Clear-Item', 'cli', 'New-ItemProperty', 'Set-ItemProperty', 'sp', 'Remove-ItemProperty', 'rp', 'Rename-ItemProperty',
    'rnp', 'Copy-ItemProperty', 'cpp', 'Move-ItemProperty', 'mp', 'Clear-ItemProperty', 'clp', 'Set-Acl', 'Start-Transcript',
    'Compress-Archive', 'Expand-Archive', 'New-TemporaryFile', 'Export-Alias', 'epal', 'Export-FormatData', 'Export-PSSession', 'epsn')
# Start-Process writes a file when it sends the program's output or errors to one.
$script:StartCommands = @('Start-Process', 'start', 'saps')
# [IO.File] and [IO.Directory] may only read.
$script:FileTypes = @('File', 'Directory')
$script:FileReads = @('ReadAllText', 'ReadAllBytes', 'ReadAllLines', 'ReadLines', 'Exists', 'OpenRead', 'GetAttributes',
    'GetLastWriteTime', 'GetLastWriteTimeUtc', 'GetCreationTime', 'GetCreationTimeUtc', 'GetLastAccessTime',
    'GetLastAccessTimeUtc', 'EnumerateFiles', 'EnumerateDirectories', 'EnumerateFileSystemEntries', 'GetFiles',
    'GetDirectories', 'GetFileSystemEntries', 'GetCurrentDirectory')
# Types that open a file for writing, used in any form: [IO.StreamWriter]$f creates the file.
$script:WriterTypes = @('FileStream', 'StreamWriter', 'BinaryWriter')
# Members that write, move or delete, whatever they are called on: files (FileInfo and the like),
# and the registry keys and values the scripts never change.
$script:WriteMembers = @('WriteAllText', 'WriteAllBytes', 'WriteAllLines', 'AppendAllText', 'AppendAllLines', 'AppendText',
    'Create', 'CreateText', 'OpenWrite', 'CreateDirectory', 'CreateSubdirectory', 'MoveTo', 'CopyTo', 'Delete', 'Encrypt',
    'Decrypt', 'SetAccessControl', 'SetAttributes', 'SetLastWriteTime', 'SetLastWriteTimeUtc', 'SetCreationTime',
    'SetCreationTimeUtc', 'CreateSymbolicLink', 'SetValue', 'DeleteValue', 'DeleteSubKey', 'DeleteSubKeyTree', 'CreateSubKey')

function Get-ScanStartProblem($Command) {
    # $null unless Start-Process sends the program's output or errors to a file: by
    # -RedirectStandardOutput or -RedirectStandardError (an unambiguous prefix, or RSO and RSE), or
    # through splatted arguments, which could. -RedirectStandardInput only reads.
    foreach ($element in $Command.CommandElements) {
        if ($element -is [Management.Automation.Language.VariableExpressionAst] -and $element.Splatted) { return 'with splatted arguments, which could redirect to a file' }
        if ($element -isnot [Management.Automation.Language.CommandParameterAst]) { continue }
        $parameter = $element.ParameterName
        $redirects = (@('RSO', 'RSE') -contains $parameter)
        if ($parameter.Length -gt 'RedirectStandard'.Length) {
            foreach ($full in @('RedirectStandardOutput', 'RedirectStandardError')) {
                if ($full.StartsWith($parameter, [StringComparison]::OrdinalIgnoreCase)) { $redirects = $true }
            }
        }
        if ($redirects) { return ('-' + $parameter + ', a redirection to a file') }
    }
    return $null
}

function Get-ScanAtomicPath($Command) {
    # The argument that Write-TextFileAtomic([string]$Path, [string]$Text) (lib.ps1) binds to
    # $Path: -Path's, under any prefix of the name, else the first positional one. $null when there
    # is none, or when splatted arguments hide it.
    $elements = $Command.CommandElements
    $positional = $null
    for ($i = 1; $i -lt $elements.Count; $i++) {
        $element = $elements[$i]
        if ($element -is [Management.Automation.Language.CommandParameterAst]) {
            $value = $element.Argument
            if ($null -eq $value -and $i + 1 -lt $elements.Count) {
                $i++
                $value = $elements[$i]
            }
            if ('Path'.StartsWith($element.ParameterName, [StringComparison]::OrdinalIgnoreCase)) { return $value }
            continue
        }
        if ($element -is [Management.Automation.Language.VariableExpressionAst] -and $element.Splatted) { return $null }
        if ($null -eq $positional) { $positional = $element }
    }
    return $positional
}

function Test-ScanDecisionPath($Node) {
    # Whether Write-TextFileAtomic's path names the decision file, C:\kairo\jobs\<id>.decision.json:
    # a variable named for it ($decisionFile, $script:DecisionPath), or an expression holding a
    # string that ends in .decision.json.
    if ($null -eq $Node) { return $false }
    if ($Node -is [Management.Automation.Language.VariableExpressionAst]) {
        return ($Node.VariablePath.UserPath -match '(?i)(^|:)decision(file|path)\z')
    }
    $strings = @($Node.FindAll({ param($n) $n -is [Management.Automation.Language.StringConstantExpressionAst] -or $n -is [Management.Automation.Language.ExpandableStringExpressionAst] }, $true))
    foreach ($string in $strings) {
        if ($string.Value -match '(?i)\.decision\.json\z') { return $true }
    }
    return $false
}

function Get-ScanEnumName($Node, [string] $Type) {
    # An argument's System.IO enum value when it is written out ([IO.FileMode]::Open, 'Open' or
    # [IO.FileMode]'Open'); $null otherwise.
    if ($Node -is [Management.Automation.Language.MemberExpressionAst] -and $Node -isnot [Management.Automation.Language.InvokeMemberExpressionAst] -and
        $Node.Expression -is [Management.Automation.Language.TypeExpressionAst]) {
        if (Test-ScanIoType $Node.Expression.TypeName.FullName @($Type)) { return (Get-ScanMemberName $Node) }
        return $null
    }
    if ($Node -is [Management.Automation.Language.ConvertExpressionAst] -and (Test-ScanIoType $Node.Type.TypeName.FullName @($Type))) { $Node = $Node.Child }
    if ($Node -is [Management.Automation.Language.StringConstantExpressionAst]) { return $Node.Value }
    return $null
}

function Get-ScanOpenProblem($Node) {
    # $null for an Open call that only reads, Open(<FileMode> Open, <FileAccess> Read[, <FileShare>])
    # written out, or for a member named Open that isn't called. FileInfo.Open(mode) alone opens
    # for reading and writing, and the other modes can create or empty the file.
    if ($Node -isnot [Management.Automation.Language.InvokeMemberExpressionAst]) { return $null }
    $arguments = @()
    if ($null -ne $Node.Arguments) { $arguments = @($Node.Arguments) }
    if ($arguments.Count -ge 2 -and (Get-ScanEnumName $arguments[0] 'FileMode') -eq 'Open' -and (Get-ScanEnumName $arguments[1] 'FileAccess') -eq 'Read') { return $null }
    return 'that can write: only Open(Open, Read), written out, only reads'
}

function Find-ScanFileWrites($Source) {
    $found = [Collections.Generic.List[string]]::new()
    foreach ($node in (Find-ScanNodes $Source.Ast ([Management.Automation.Language.CommandAst]))) {
        $command = Get-ScanCommandName $node
        if ($null -eq $command) { continue }
        if ($script:WriteCommands -contains $command) {
            $found.Add((New-ScanFinding $Source $node $command))
        } elseif ($script:StartCommands -contains $command) {
            $problem = Get-ScanStartProblem $node
            if ($null -ne $problem) { $found.Add((New-ScanFinding $Source $node ($command + ' ' + $problem))) }
        } elseif ($command -eq 'Write-TextFileAtomic') {
            # The decision file's writer, and only for that file: its path must name it.
            if (-not (Test-ScanDecisionPath (Get-ScanAtomicPath $node))) {
                $found.Add((New-ScanFinding $Source $node 'Write-TextFileAtomic on a file other than the decision file'))
            }
        } elseif ($command -eq 'New-Object') {
            for ($i = 1; $i -lt $node.CommandElements.Count; $i++) {
                foreach ($name in (Get-ScanStrings $node.CommandElements[$i])) {
                    if (Test-ScanIoType $name $script:WriterTypes) { $found.Add((New-ScanFinding $Source $node ('New-Object ' + $name))) }
                }
            }
        }
    }
    foreach ($node in (Find-ScanNodes $Source.Ast ([Management.Automation.Language.FileRedirectionAst]))) {
        $toNull = ($node.Location -is [Management.Automation.Language.VariableExpressionAst] -and $node.Location.VariablePath.UserPath -eq 'null')
        if (-not $toNull) { $found.Add((New-ScanFinding $Source $node ('a redirection to a file: ' + $node.Extent.Text))) }
    }
    # A writer type in any form: [IO.StreamWriter]::new, a conversion, -as, a typed variable or
    # parameter. Type literals are TypeExpressionAst nodes, and conversions and type constraints
    # TypeConstraintAst ones.
    foreach ($node in $Source.Ast.FindAll({ param($n) $n -is [Management.Automation.Language.TypeExpressionAst] -or $n -is [Management.Automation.Language.TypeConstraintAst] }, $true)) {
        $type = $node.TypeName.FullName
        if (Test-ScanIoType $type $script:WriterTypes) { $found.Add((New-ScanFinding $Source $node ('[' + $type + '], a type that opens a file for writing'))) }
    }
    foreach ($node in (Find-ScanNodes $Source.Ast ([Management.Automation.Language.MemberExpressionAst]))) {
        $name = Get-ScanMemberName $node
        if ($null -eq $name) { continue }
        if ($node.Expression -is [Management.Automation.Language.TypeExpressionAst]) {
            $type = $node.Expression.TypeName.FullName
            if ((Test-ScanIoType $type $script:FileTypes) -and $script:FileReads -notcontains $name) {
                $found.Add((New-ScanFinding $Source $node ('[' + $type + ']::' + $name)))
            }
        } elseif ($script:WriteMembers -contains $name) {
            $found.Add((New-ScanFinding $Source $node ('.' + $name)))
        } elseif ($name -eq 'Open') {
            $problem = Get-ScanOpenProblem $node
            if ($null -ne $problem) { $found.Add((New-ScanFinding $Source $node ('.Open ' + $problem))) }
        }
    }
    return ,$found.ToArray()
}

# --- R5 dot-sourcing safely --------------------------------------------------------------------

function Find-ScanLibraryShape($Source) {
    $found = [Collections.Generic.List[string]]::new()
    $ast = $Source.Ast
    if ($null -ne $ast.ParamBlock) { $found.Add((New-ScanFinding $Source $ast.ParamBlock 'a script param block, which dot-sourcing would bind')) }
    foreach ($block in @($ast.BeginBlock, $ast.ProcessBlock, $ast.DynamicParamBlock)) {
        if ($null -ne $block) { $found.Add((New-ScanFinding $Source $block 'a begin, process or dynamicparam block')) }
    }
    $guard = $null
    $statements = @()
    if ($null -ne $ast.EndBlock) { $statements = @($ast.EndBlock.Statements) }
    for ($i = 0; $i -lt $statements.Count; $i++) {
        $statement = $statements[$i]
        if ($statement -is [Management.Automation.Language.FunctionDefinitionAst]) { continue }
        $isGuard = ($statement -is [Management.Automation.Language.IfStatementAst] -and $statement.Clauses.Count -eq 1 -and
            $null -eq $statement.ElseClause -and $statement.Clauses[0].Item1.Extent.Text -ceq '$MyInvocation.InvocationName -ne ''.''')
        if ($isGuard -and $i -eq $statements.Count - 1) {
            $guard = $statement
            continue
        }
        $found.Add((New-ScanFinding $Source $statement 'a top-level statement outside the guarded command line, which dot-sourcing would run'))
    }
    if ($null -eq $guard) { $found.Add($Source.Name + ': no command line guarded by $MyInvocation.InvocationName -ne ''.'' after the functions') }
    foreach ($node in (Find-ScanNodes $ast ([Management.Automation.Language.ExitStatementAst]))) {
        if ($null -eq $guard -or -not (Test-ScanInside $node $guard) -or $null -ne (Get-ScanFunction $node)) {
            $found.Add((New-ScanFinding $Source $node 'exit outside the guarded command line: a function would end the elevated session'))
        }
    }
    foreach ($node in (Find-ScanNodes $ast ([Management.Automation.Language.NamedAttributeArgumentAst]))) {
        if ($node.ArgumentName -eq 'Mandatory') { $found.Add((New-ScanFinding $Source $node 'a Mandatory parameter, which would prompt')) }
    }
    foreach ($node in (Find-ScanNodes $ast ([Management.Automation.Language.CommandAst]))) {
        if ((Get-ScanCommandName $node) -eq 'Read-Host') { $found.Add((New-ScanFinding $Source $node 'Read-Host, which would prompt')) }
    }
    return ,$found.ToArray()
}

# --- R6 Windows PowerShell 5.1's grammar ---------------------------------------------------------

function Test-ScanFlag($Node, [string] $Name) {
    # A bool property that PowerShell 7's syntax tree has and 5.1's lacks.
    $property = $Node.PSObject.Properties[$Name]
    return ($null -ne $property -and $property.Value -eq $true)
}

function Find-ScanPowerShell7Syntax($Source) {
    $found = [Collections.Generic.List[string]]::new()
    foreach ($node in $Source.Ast.FindAll({ param($n) $true }, $true)) {
        $kind = $node.GetType().Name
        $what = $null
        if ($kind -eq 'TernaryExpressionAst') { $what = '?: (the ternary operator)' }
        elseif ($kind -eq 'PipelineChainAst') { $what = '&& or ||' }
        elseif ($node -is [Management.Automation.Language.BinaryExpressionAst] -and [string]$node.Operator -eq 'QuestionQuestion') { $what = '??' }
        elseif ($node -is [Management.Automation.Language.AssignmentStatementAst] -and [string]$node.Operator -eq 'QuestionQuestionEquals') { $what = '??=' }
        elseif ($node -is [Management.Automation.Language.MemberExpressionAst] -and (Test-ScanFlag $node 'NullConditional')) { $what = '?.' }
        elseif ($node -is [Management.Automation.Language.IndexExpressionAst] -and (Test-ScanFlag $node 'NullConditional')) { $what = '?[' }
        elseif ($node -is [Management.Automation.Language.PipelineAst] -and (Test-ScanFlag $node 'Background')) { $what = '& (a background job)' }
        if ($null -ne $what) { $found.Add((New-ScanFinding $Source $node ($what + ', which PowerShell 5.1 can''t parse'))) }
    }
    foreach ($token in $Source.Tokens) {
        $kind = [string]$token.Kind
        if (($kind -eq 'StringExpandable' -or $kind -eq 'HereStringExpandable') -and $token.Text.Contains('`u{')) {
            $found.Add((New-ScanFinding $Source $token 'a `u{} escape, which PowerShell 5.1 reads as u{}'))
        }
        # A type suffix, before a multiplier or not (10ul, 1ukb); 5.1's own L, D and kb..pb pass.
        if ($kind -eq 'Number' -and $token.Text -match '(?i)(u[lsy]?|[nsy])(kb|mb|gb|tb|pb)?\z') {
            $found.Add((New-ScanFinding $Source $token ('the number ' + $token.Text + ', whose suffix PowerShell 5.1 lacks')))
        }
    }
    return ,$found.ToArray()
}

# --- R7 registry writes --------------------------------------------------------------------------

$script:RegistryWriteCommands = @('Set-ItemProperty', 'sp', 'New-Item', 'ni', 'mkdir', 'md', 'New-ItemProperty',
    'Remove-Item', 'ri', 'rm', 'rmdir', 'del', 'erase', 'rd', 'Remove-ItemProperty', 'rp', 'Rename-ItemProperty', 'rnp',
    'Copy-ItemProperty', 'cpp', 'Move-ItemProperty', 'mp', 'Clear-ItemProperty', 'clp', 'Set-Item', 'si', 'Clear-Item',
    'cli', 'Rename-Item', 'rni', 'ren', 'Copy-Item', 'cpi', 'cp', 'copy', 'Move-Item', 'mi', 'mv', 'move', 'Set-Acl',
    'reg', 'reg.exe', 'regedit', 'regedit.exe', 'regini', 'regini.exe')
$script:RegistryTools = @('reg', 'reg.exe', 'regedit', 'regedit.exe', 'regini', 'regini.exe')
$script:Launchers = @('Start-Process', 'saps', 'start', 'cmd', 'cmd.exe')
$script:RegistryWriteMembers = @('SetValue', 'CreateSubKey', 'DeleteValue', 'DeleteSubKey', 'DeleteSubKeyTree', 'SetAccessControl')

function Get-ScanLeaf([string] $Name) {
    # A program named by its path counts by its file name (C:\Windows\System32\reg.exe is reg.exe),
    # and a cmdlet by its name without a module (Microsoft.PowerShell.Management\New-Item).
    return $Name.Substring($Name.LastIndexOfAny([char[]]@('\', '/')) + 1)
}

function Find-ScanRegistryWrites($Source) {
    $found = [Collections.Generic.List[string]]::new()
    foreach ($node in (Find-ScanNodes $Source.Ast ([Management.Automation.Language.CommandAst]))) {
        $command = $node.GetCommandName()
        if ($null -eq $command) { continue }
        $leaf = Get-ScanLeaf $command
        if ($script:RegistryWriteCommands -contains $leaf) { $found.Add((New-ScanFinding $Source $node $command)) }
        if ($script:Launchers -notcontains $leaf) { continue }
        # A launcher's words: Start-Process reg.exe, cmd /c "reg add ...".
        for ($i = 1; $i -lt $node.CommandElements.Count; $i++) {
            foreach ($text in (Get-ScanStrings $node.CommandElements[$i])) {
                foreach ($word in $text.Split([char[]]@(' ', [char]9), [StringSplitOptions]::RemoveEmptyEntries)) {
                    if ($script:RegistryTools -contains (Get-ScanLeaf $word.Trim([char]34, [char]39))) { $found.Add((New-ScanFinding $Source $node ($command + ' ' + $word))) }
                }
            }
        }
    }
    foreach ($node in (Find-ScanNodes $Source.Ast ([Management.Automation.Language.MemberExpressionAst]))) {
        $name = Get-ScanMemberName $node
        if ($null -eq $name) {
            $found.Add((New-ScanFinding $Source $node ('a member name computed at run time: ' + $node.Extent.Text)))
        } elseif ($script:RegistryWriteMembers -contains $name) {
            $found.Add((New-ScanFinding $Source $node ('.' + $name)))
        } elseif ($name -eq 'OpenSubKey') {
            $arguments = @()
            if ($node -is [Management.Automation.Language.InvokeMemberExpressionAst] -and $null -ne $node.Arguments) { $arguments = @($node.Arguments) }
            $readOnly = ($arguments.Count -eq 2 -and $arguments[1] -is [Management.Automation.Language.VariableExpressionAst] -and
                $arguments[1].VariablePath.UserPath -eq 'false')
            if (-not $readOnly) { $found.Add((New-ScanFinding $Source $node ('OpenSubKey without $false as its second and last argument: ' + $node.Extent.Text))) }
        }
    }
    return ,$found.ToArray()
}

# --- The files -----------------------------------------------------------------------------------

$script:AllFiles = @(Get-ChildItem -LiteralPath $script:ToolsDir -Recurse -File -Filter '*.ps1' | Sort-Object FullName | ForEach-Object { Read-ScanFile $_.FullName })
$script:BenchFiles = @($script:AllFiles | Where-Object { $_.Name -match '^tools/pc/bench/[^/]+\.ps1\z' })
$script:ElevatedFiles = @($script:AllFiles | Where-Object { $_.Name -match '^tools/pc/bench/[^/]+\.ps1\z' -or $_.Name -match '^tools/mac/remote/[^/]+\.ps1\z' })
$script:ElevatedBenchFiles = @($script:AllFiles | Where-Object { $_.Name -match '^tools/pc/bench/[^/]+\.ps1\z' -or $_.Name -match '^tools/mac/remote/bench[^/]*\.ps1\z' })
$script:RegistryFiles = @($script:AllFiles | Where-Object { $_.Name -eq 'tools/pc/bench/regdiff.ps1' -or $_.Name -eq 'tools/mac/remote/amd-diff.ps1' })

function Assert-ScanClean([string[]] $Findings, [string] $What) {
    if (@($Findings).Count -gt 0) { throw ($What + ': ' + (@($Findings) -join ' | ')) }
}

function Get-ScanFindings([scriptblock] $Scan, [string] $Snippet) {
    # A rule's findings for one snippet, as a flat array of strings, however the rule returns them.
    $out = [Collections.Generic.List[string]]::new()
    foreach ($item in @(& $Scan $Snippet)) {
        foreach ($inner in @($item)) { if ($null -ne $inner) { $out.Add([string]$inner) } }
    }
    return ,$out.ToArray()
}

function Assert-ScanFlags([scriptblock] $Scan, [string[]] $Snippets, [string] $Expect) {
    # Each snippet must give at least one finding, and the first must match $Expect.
    foreach ($snippet in $Snippets) {
        $findings = Get-ScanFindings $Scan $snippet
        if ($findings.Count -eq 0) { throw ('not flagged: ' + $snippet) }
        Assert-Match $Expect $findings[0] ('the finding for ' + $snippet)
    }
}

function Assert-ScanPasses([scriptblock] $Scan, [string[]] $Snippets) {
    foreach ($snippet in $Snippets) {
        Assert-ScanClean (Get-ScanFindings $Scan $snippet) ('flagged: ' + $snippet)
    }
}

Test-Case 'the file sets hold the scripts the rules are for' {
    $bench = @($script:BenchFiles | ForEach-Object { $_.Name }) -join ','
    Assert-Equal 'tools/pc/bench/ab-stats.ps1,tools/pc/bench/bench-rules.ps1,tools/pc/bench/encoder-usage.ps1,tools/pc/bench/pm-stats.ps1,tools/pc/bench/regdiff.ps1' $bench 'the bench scripts'
    Assert-Equal 'tools/mac/remote/amd-diff.ps1,tools/pc/bench/regdiff.ps1' (@($script:RegistryFiles | ForEach-Object { $_.Name }) -join ',') 'the registry readers'
    Assert-True (@($script:ElevatedFiles | Where-Object { $_.Name -eq 'tools/mac/remote/lib.ps1' }).Count -eq 1) 'lib.ps1 is an elevated script'
    Assert-True (@($script:AllFiles | Where-Object { $_.Name -eq 'tools/mac/remote/status.ps1' }).Count -eq 1) 'status.ps1 is scanned'
    Assert-True (@($script:AllFiles | Where-Object { $_.Name -eq 'tools/pc/bench/tests/scans.tests.ps1' -and $_.IsTest }).Count -eq 1) 'this file is scanned, as a test'
    Assert-True ($script:AllFiles.Count -ge 20) ('every script: ' + $script:AllFiles.Count)
}

# --- R1 --------------------------------------------------------------------------------------------

# R2's one helper, which R1 must pass too.
$script:Helper = @'
function Get-ProcessIdAndName {
    $list = [Collections.Generic.List[object]]::new()
    foreach ($process in [Diagnostics.Process]::GetProcesses()) {
        try { $list.Add([pscustomobject]@{ Id = [long]$process.Id; Name = [string]$process.ProcessName }) } finally { $process.Dispose() }
    }
    return ,$list.ToArray()
}
'@
$script:R1 = { param($Text) Find-ScanModuleMembers (ConvertTo-ScanSource $Text) }

Test-Case 'R1 the bench scripts read no member that lists modules or reads a command line' {
    foreach ($source in $script:BenchFiles) { Assert-ScanClean (Find-ScanModuleMembers $source) $source.Name }
}

Test-Case 'R1 flags whatever would give a bench script a Process object' {
    Assert-ScanFlags $script:R1 @(
        'Get-Process -Name game',
        'gps -Id 4',
        'ps game',
        'Get-Process -Name game | Select-Object Id',
        'Microsoft.PowerShell.Management\Get-Process -Name game',
        '[Diagnostics.Process]::GetProcessById(4)',
        '[System.Diagnostics.Process]::GetProcessesByName(''game'')',
        '$p = $type::GetProcessById(4)'
    ) '^snippet\.ps1:1: \.?(Get-Process|gps|ps|GetProcessById|GetProcessesByName), which gives Process objects: Get-ProcessIdAndName gives names$'
}

Test-Case 'R1 flags a getter, and a wildcard property argument, that read a banned member' {
    Assert-ScanFlags $script:R1 @('$p.get_MainModule().FileName', '$p.get_Path()', '$p.GET_COMMANDLINE()') '(?i)^snippet\.ps1:1: \.get_(MainModule|Path|CommandLine)\(\)$'
    Assert-ScanFlags $script:R1 @(
        '$all | Select-Object *',
        '$all | Select-Object P*',
        '$all | Select-Object p*',
        '$all | Format-List *',
        '$all | Select-Object -Property Id, ''Pa?h''',
        '$all | Sort-Object -Property [M]ainModule',
        '$all | Format-Table Id, Comm*',
        '$all | Select-Object ''[P'''
    ) '^snippet\.ps1:1: (Select-Object|Format-List|Sort-Object|Format-Table) \S+, which matches [A-Za-z, ]+$'
}

Test-Case 'R1 flags a process''s Path, MainModule or CommandLine in every form' {
    Assert-ScanFlags $script:R1 @(
        '$p = Get-Thing; $p.Path',
        '$p.MainModule.FileName',
        '$p.''Modules''',
        '$p.Company',
        '$p.CommandLine',
        '$p.PSObject.Properties[''Path''].Value',
        '$all | Select-Object -ExpandProperty Path',
        '$all | Select-Object Id, ProductVersion',
        '$all | ForEach-Object Description',
        '$all | Sort-Object -Property FileVersion'
    ) '^snippet\.ps1:1: '
    Assert-ScanFlags $script:R1 @('$name = ''Path''; $p.$name', '$p."Pa$x"') 'computed at run time'
}

Test-Case 'R1 passes type literals, parameters named Path and other members' {
    # [Environment]::CommandLine is a static member on a type literal: this process's own command line.
    Assert-ScanPasses $script:R1 @(
        '[IO.Path]::GetFullPath(''x'')',
        '[System.IO.Path]::Combine(''a'', ''b'')',
        '[Environment]::CommandLine',
        '$p.Id; $p.ProcessName; $p.Dispose()',
        'Join-Path -Path ''a'' -ChildPath ''b''',
        'Get-ChildItem -Path x -Filter *.ps1',
        '$h[''Name'']',
        '$all | Select-Object -Property Id, Name',
        '$all | Select-Object -Property Id, Na*',
        '$p.get_Id(); $p.get_ProcessName()',
        '[Environment]::get_CommandLine()',
        '# $p.Path in a comment',
        '''$p.Path in a string''',
        $script:Helper
    )
}

# --- R2 --------------------------------------------------------------------------------------------

$script:R2 ={ param($Text) (Find-ScanProcessLists @(ConvertTo-ScanSource $Text)).Findings }

Test-Case 'R2 every script lists processes only through the one helper, or by name or id' {
    $result = Find-ScanProcessLists $script:AllFiles
    Assert-ScanClean $result.Findings 'tools/**/*.ps1'
    Assert-Equal 'tools/pc/bench/encoder-usage.ps1 Get-ProcessIdAndName' ($result.Helpers -join '; ') 'the one helper'
    # The by-name lookups that stay allowed are seen and passed, so the rule isn't vacuous.
    $lookups = 0
    foreach ($source in $script:AllFiles) {
        foreach ($node in (Find-ScanNodes $source.Ast ([Management.Automation.Language.CommandAst]))) {
            if ($node.GetCommandName() -eq 'Get-Process') { $lookups++ }
        }
    }
    Assert-True ($lookups -ge 4) ('Get-Process lookups seen: ' + $lookups)
}

Test-Case 'R2 flags GetProcesses outside the helper, an unfiltered Get-Process and a Win32_Process list' {
    Assert-ScanFlags $script:R2 @('$all = [Diagnostics.Process]::GetProcesses()') 'GetProcesses outside a function'
    Assert-ScanFlags $script:R2 @('function F { [Diagnostics.Process]::GetProcesses() | ForEach-Object { $_.Id } }') 'GetProcesses outside a foreach'
    Assert-ScanFlags $script:R2 @($script:Helper.Replace('[string]$process.ProcessName', '[string]$process.Path')) 'reading more of a process'
    Assert-ScanFlags $script:R2 @($script:Helper.Replace('$list.Add([pscustomobject]', '$list.Add($process); $list.Add([pscustomobject]')) 'reading more of a process'
    Assert-ScanFlags $script:R2 @($script:Helper.Replace('Name = [string]$process.ProcessName', 'Name = [string]$process.ProcessName; Started = 1')) 'returning more than'
    Assert-ScanFlags $script:R2 @(($script:Helper + "`n" + $script:Helper.Replace('Get-ProcessIdAndName', 'Get-Second'))) 'more than one GetProcesses helper'
    Assert-ScanFlags $script:R2 @(
        'Get-Process',
        'Get-Process -ErrorAction SilentlyContinue | Select-Object Id',
        'gps',
        'Get-Process *',
        'Get-Process -Name ''*''',
        'Get-Process @splat',
        'Microsoft.PowerShell.Management\Get-Process',
        'Microsoft.PowerShell.Management\Get-Process -Name *'
    ) '^snippet\.ps1:1: (Get-Process|gps) '
    Assert-ScanFlags $script:R2 @('Get-Process -Name ''game'' -Module', 'Get-Process -Id 4 -FileVersionInfo') 'which lists modules'
    Assert-ScanFlags $script:R2 @(
        'Get-CimInstance -ClassName Win32_Process',
        'Get-WmiObject Win32_Process | Select-Object ProcessId',
        '$q = "SELECT ExecutablePath FROM Win32_Process"; Get-CimInstance -Query $q'
    ) 'Win32_Process without a WHERE clause'
}

Test-Case 'R2 passes the helper and lookups by name or id' {
    Assert-ScanPasses $script:R2 @(
        $script:Helper,
        '$logonUi = [bool](Get-Process -Name ''LogonUI'' -ErrorAction SilentlyContinue)',
        'Get-Process LogonUI',
        'Get-Process -Name ''kairo*'', ''testbed'' -ErrorAction SilentlyContinue',
        'Get-Process -Id $PID',
        '$q = "SELECT ExecutablePath FROM Win32_Process WHERE Handle = ''{0}''" -f 4; Get-CimInstance -Query $q',
        'Get-CimInstance -Query ("SELECT ProcessId FROM Win32_Process WHERE Handle = ''{0}''" -f 4)',
        'Get-CimInstance -ClassName Win32_Process -Filter "ProcessId = 4"',
        'Get-CimInstance -ClassName Win32_Processor',
        '[Diagnostics.Process]::GetProcessesByName(''kairo-devrun'')',
        '''[Diagnostics.Process]::GetProcesses() in a string'''
    )
}

# --- R3 --------------------------------------------------------------------------------------------

$script:R3 = { param($Text) Find-ScanBannedCalls (ConvertTo-ScanSource $Text) -Elevated }

Test-Case 'R3 no Add-Type or Import-Csv in the elevated scripts, no synthetic input anywhere' {
    $elevated = @($script:ElevatedFiles | ForEach-Object { $_.Name })
    foreach ($source in $script:AllFiles) {
        Assert-ScanClean (Find-ScanBannedCalls $source -Elevated:($elevated -contains $source.Name)) $source.Name
    }
}

Test-Case 'R3 flags Add-Type, Import-Csv and synthetic input' {
    Assert-ScanFlags $script:R3 @(
        'Add-Type -TypeDefinition $code',
        'Import-Csv -LiteralPath $file',
        'ipcsv x.csv',
        'Microsoft.PowerShell.Utility\Add-Type -TypeDefinition $code',
        'Microsoft.PowerShell.Utility\Import-Csv -LiteralPath $file'
    ) '^snippet\.ps1:1: (Add-Type|Import-Csv|ipcsv)$'
    Assert-ScanFlags $script:R3 @('Kairo.Input\SendInput 1 $inputs 40') '^snippet\.ps1:1: synthetic input: SendInput$'
    Assert-ScanFlags $script:R3 @(
        '[User32]::SendInput(1, $inputs, 40)',
        '[System.Windows.Forms.SendKeys]::SendWait(''x'')',
        '(New-Object -ComObject WScript.Shell).SendKeys(''x'')',
        '$code = ''[DllImport("user32.dll")] public static extern uint SendInput(uint n, IntPtr p, int s);'''
    ) 'synthetic input'
    # Outside the elevated scripts, Add-Type passes; synthetic input never does.
    Assert-ScanClean (Find-ScanBannedCalls (ConvertTo-ScanSource 'Add-Type -AssemblyName System.IO.Compression.FileSystem')) 'Add-Type in the bootstrap'
    Assert-Equal 1 (Find-ScanBannedCalls (ConvertTo-ScanSource '$shell.SendKeys(''x'')')).Count 'SendKeys outside the elevated scripts'
}

Test-Case 'R3 passes other commands, comments, and the word in a test''s strings' {
    Assert-ScanPasses $script:R3 @(
        'ConvertFrom-Json -InputObject $text',
        '$text | ConvertFrom-Csv',
        'Add-TypeName x',
        '# never SendInput or Add-Type',
        '$s.Send($x)'
    )
    Assert-ScanClean (Find-ScanBannedCalls (ConvertTo-ScanSource '$banned = @(''SendInput'', ''keybd_event'')' -IsTest) -Elevated) 'a test''s strings'
}

# --- R4 --------------------------------------------------------------------------------------------

$script:R4 = { param($Text) Find-ScanFileWrites (ConvertTo-ScanSource $Text) }

Test-Case 'R4 the elevated bench scripts write no file' {
    foreach ($source in $script:ElevatedBenchFiles) { Assert-ScanClean (Find-ScanFileWrites $source) $source.Name }
}

Test-Case 'R4 flags writes by cmdlet, redirection and .NET' {
    Assert-ScanFlags $script:R4 @(
        'Set-Content -LiteralPath $f -Value $t',
        '$t | Out-File $f',
        'Add-Content $f $t',
        'New-Item -ItemType File $f',
        'Remove-Item $f',
        'Move-Item $a $b',
        'Set-ItemProperty -Path $key -Name x -Value 1',
        'Export-Csv -InputObject $x -Path $f',
        '$t > out.txt',
        '$t >> $f',
        'Get-Thing 2> errors.txt',
        '[IO.File]::WriteAllText($f, $t)',
        '[System.IO.File]::Delete($f)',
        '[IO.File]::Move($a, $b)',
        '[IO.Directory]::CreateDirectory($d)',
        '[IO.StreamWriter]::new($f)',
        'New-Object -TypeName System.IO.FileStream -ArgumentList $f, 2',
        '$info.MoveTo($b)',
        '$info.Delete()',
        '$key.SetValue(''x'', 1)'
    ) '^snippet\.ps1:1: '
    Assert-ScanFlags $script:R4 @(
        'Microsoft.PowerShell.Management\Set-Content -LiteralPath $f -Value $t',
        'Microsoft.PowerShell.Utility\Out-File -FilePath $f -InputObject $t'
    ) '^snippet\.ps1:1: (Set-Content|Out-File)$'
    Assert-ScanFlags $script:R4 @('using namespace System.IO' + "`n" + '[File]::WriteAllText($f, $t)') '^snippet\.ps1:2: \[File\]::WriteAllText$'
}

Test-Case 'R4 flags Start-Process sending a program''s output or errors to a file' {
    Assert-ScanFlags $script:R4 @(
        'Start-Process -FilePath $exe -RedirectStandardOutput (Join-Path $logs ''pm.out'') -Wait',
        'Start-Process $exe -RedirectStandardError $err -NoNewWindow',
        'Start-Process $exe -RedirectStandardOutput:$f',
        'start $exe -RedirectStandardO $f',
        'saps $exe -RSE $f',
        'Start-Process $exe -rso $f',
        'Microsoft.PowerShell.Management\Start-Process $exe -RedirectStandardOutput $f'
    ) '^snippet\.ps1:1: (Start-Process|start|saps) -(RedirectStandard[A-Za-z]+|RSO|RSE|rso), a redirection to a file$'
    Assert-ScanFlags $script:R4 @('Start-Process @splat') '^snippet\.ps1:1: Start-Process with splatted arguments'
}

Test-Case 'R4 flags Create, OpenWrite, an Open that can write, and the writer types in any form' {
    Assert-ScanFlags $script:R4 @(
        '[IO.FileInfo]::new($f).Create().Dispose()',
        '$info.Create()',
        '$info.OpenWrite()'
    ) '^snippet\.ps1:1: \.(Create|OpenWrite)$'
    Assert-ScanFlags $script:R4 @(
        '$info.Open([IO.FileMode]::Create)',
        '$info.Open([IO.FileMode]::Open)',
        '$info.Open()',
        '$info.Open(''Append'', ''Write'')',
        '$info.Open([System.IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::Read)',
        '$info.Open([IO.FileMode]::OpenOrCreate, [IO.FileAccess]::Read)',
        '$info.Open([IO.FileMode]::Truncate, ''Write'')',
        '$info.Open($mode, $access)',
        '(Get-Item $f).open(''Open'', ''Write'')'
    ) '^snippet\.ps1:1: \.Open that can write'
    Assert-ScanFlags $script:R4 @(
        '$w = [IO.StreamWriter]$f',
        '$w = [System.IO.StreamWriter]($f)',
        '$w = $f -as [IO.StreamWriter]',
        '[IO.FileStream] $s = $f',
        'function F([IO.BinaryWriter] $w) { $w }',
        '$w = [IO.StreamWriter[]]@($f)',
        '$w = [System.IO.BinaryWriter]::new($s)'
    ) '^snippet\.ps1:1: \[[A-Za-z.]+(\[\])?\], a type that opens a file for writing$'
    Assert-ScanFlags $script:R4 @('using namespace System.IO' + "`n" + '$w = [StreamWriter]$f') '^snippet\.ps1:2: \[StreamWriter\], a type that opens a file for writing$'
}

Test-Case 'R4 lets Write-TextFileAtomic write only the decision file, named by its path' {
    Assert-ScanFlags $script:R4 @(
        'Write-TextFileAtomic $f $t',
        'Write-TextFileAtomic -Path (Join-Path $Kairo.Jobs ''next.json'') -Text $t',
        'Write-TextFileAtomic (Join-Path $Kairo.Logs ''bench.json'') $decisionJson',
        'Write-TextFileAtomic -Path (Join-Path $env:ProgramData ''x.ps1'') -Text ''# decision''',
        'Write-TextFileAtomic -Text $decisionFile $f',
        'Write-TextFileAtomic -Text $t -Path $decisionJson',
        'Write-TextFileAtomic -Path (Join-Path $Kairo.Jobs ''decision.txt'') -Text $t',
        'Write-TextFileAtomic -Path ($decisionFile + ''.ps1'') -Text $t',
        'Write-TextFileAtomic @splat',
        'Write-TextFileAtomic $decisionFile @rest',
        'Write-TextFileAtomic -Text $t'
    ) '^snippet\.ps1:1: Write-TextFileAtomic on a file other than the decision file$'
}

Test-Case 'R4 passes reads, console output and the decision file' {
    Assert-ScanPasses $script:R4 @(
        '[IO.File]::ReadAllBytes($f)',
        '[IO.File]::Exists($f)',
        '[IO.FileInfo]::new($f).Length',
        '[IO.Path]::GetFullPath($f)',
        '[IO.StreamReader]::new($f).ReadToEnd()',
        '$info.OpenRead()',
        '$info.Open([IO.FileMode]::Open, [IO.FileAccess]::Read)',
        '$info.Open(''Open'', ''Read'', ''ReadWrite'')',
        '$info.Open([IO.FileMode]''Open'', [System.IO.FileAccess]::Read)',
        '$state.Open',
        'Start-Process -FilePath $exe -ArgumentList $a -Wait -NoNewWindow -RedirectStandardInput $in',
        '[Console]::Out.WriteLine($t)',
        '[Console]::Error.WriteLine($t)',
        '$null = Get-Thing 2> $null',
        'Get-Thing 2>&1',
        '$t.Replace(''a'', ''b'')',
        '[scriptblock]::Create($text)',
        'Write-TextFileAtomic (Join-Path $Kairo.Jobs ($id + ''.decision.json'')) $json',
        'Write-TextFileAtomic -Path $decisionFile -Text $json',
        'Write-TextFileAtomic -Text $json -Path (Join-Path $Kairo.Jobs "$id.decision.json")',
        'Write-TextFileAtomic -T $json $script:DecisionPath',
        'Write-TextFileAtomic -P:$decisionFile -Text $json'
    )
}

# --- R5 --------------------------------------------------------------------------------------------

$script:Library = @'
function Get-Thing([string] $Name) {
    if ($Name.Length -eq 0) { throw 'no name' }
    return $Name
}
if ($MyInvocation.InvocationName -ne '.') {
    Set-StrictMode -Version 3.0
    $code = 0
    try { [Console]::Out.WriteLine((Get-Thing $args[0])) } catch { $code = 2 }
    exit $code
}
'@
$script:R5 = { param($Text) Find-ScanLibraryShape (ConvertTo-ScanSource $Text) }

Test-Case 'R5 the bench scripts are libraries with a guarded command line' {
    foreach ($source in $script:BenchFiles) { Assert-ScanClean (Find-ScanLibraryShape $source) $source.Name }
}

Test-Case 'R5 flags what dot-sourcing would run, exits and prompts' {
    Assert-ScanFlags $script:R5 @(('$x = 1' + "`n" + $script:Library), ($script:Library + "`n" + 'Set-StrictMode -Version 3.0')) 'outside the guarded command line'
    Assert-ScanFlags $script:R5 @($script:Library.Replace('throw ''no name''', 'exit 2')) 'exit outside the guarded command line'
    Assert-ScanFlags $script:R5 @(('param([string] $Csv)' + "`n" + $script:Library)) 'a script param block'
    Assert-ScanFlags $script:R5 @($script:Library.Replace('function Get-Thing([string] $Name) {', 'function Get-Thing { param([Parameter(Mandatory)] [string] $Name)')) 'a Mandatory parameter'
    Assert-ScanFlags $script:R5 @(
        $script:Library.Replace('return $Name', 'return (Read-Host ''name'')'),
        $script:Library.Replace('return $Name', 'return (Microsoft.PowerShell.Utility\Read-Host ''name'')')
    ) 'Read-Host, which would prompt$'
    Assert-ScanFlags $script:R5 @($script:Library.Replace('-ne ''.''', '-ne ''&''')) 'outside the guarded command line'
    Assert-ScanFlags $script:R5 @(($script:Library -split "`n")[0..3] -join "`n") 'no command line guarded'
}

Test-Case 'R5 passes functions and the guarded command line' {
    Assert-ScanPasses $script:R5 @($script:Library, ('# a comment first' + "`n" + $script:Library))
}

# --- R6 --------------------------------------------------------------------------------------------

$script:R6 = { param($Text) Find-ScanPowerShell7Syntax (ConvertTo-ScanSource $Text) }

Test-Case 'R6 every script keeps to Windows PowerShell 5.1''s grammar' {
    foreach ($source in $script:AllFiles) { Assert-ScanClean (Find-ScanPowerShell7Syntax $source) $source.Name }
}

Test-Case 'R6 flags PowerShell 7''s operators, escapes and number suffixes' {
    Assert-ScanFlags $script:R6 @(
        '$x = $a ? 1 : 2',
        'Get-Thing && Get-Other',
        'Get-Thing || Get-Other',
        '$x = $a ?? 1',
        '$a ??= 1',
        '${a}?.Name',
        '${a}?[0]',
        'Get-Thing &',
        '"`u{2013}"',
        '$x = 10u',
        '$x = 5s',
        '$x = 1n',
        '$x = 0x1Fuy',
        '$x = 10ul',
        '$x = 0xFFul',
        '$x = 10UL',
        '$x = 1ukb',
        '$x = 1sKB'
    ) 'PowerShell 5.1'
}

Test-Case 'R6 passes the 5.1 forms' {
    Assert-ScanPasses $script:R6 @(
        'if ($a) { $x = 1 } else { $x = 2 }',
        'Get-Thing; if ($?) { Get-Other }',
        '$x = $a -and $b',
        '$all | ? { $_ } | where { $_ }',
        '$a.Name; $a[0]',
        '& { 1 }; & $command',
        '"`n`t"',
        '$x = 10; $y = 10L; $z = 1kb; $w = 0x1F; $v = 2.5d; $u = 1e5; $t = 1PB; $s = 0x1b',
        '''$a ?? 1 in a string'''
    )
}

# --- R7 --------------------------------------------------------------------------------------------

$script:R7 = { param($Text) Find-ScanRegistryWrites (ConvertTo-ScanSource $Text) }

Test-Case 'R7 the registry readers write nothing to the registry' {
    foreach ($source in $script:RegistryFiles) { Assert-ScanClean (Find-ScanRegistryWrites $source) $source.Name }
    # Their read-only opens are seen and passed, so the rule isn't vacuous.
    $opens = 0
    foreach ($source in $script:RegistryFiles) {
        foreach ($node in (Find-ScanNodes $source.Ast ([Management.Automation.Language.MemberExpressionAst]))) {
            if ((Get-ScanMemberName $node) -eq 'OpenSubKey') { $opens++ }
        }
    }
    Assert-True ($opens -ge 5) ('OpenSubKey calls seen: ' + $opens)
}

Test-Case 'R7 flags registry writes by cmdlet, reg.exe and .NET, and keys opened for writing' {
    Assert-ScanFlags $script:R7 @(
        'Set-ItemProperty -LiteralPath ''HKCU:\Software\AMD\DVR'' -Name DvrEnabled -Value 0',
        'New-Item -Path ''HKCU:\Software\Kairo''',
        'New-ItemProperty -Path $key -Name x -Value 1',
        'Remove-Item -LiteralPath ''HKCU:\Software\AMD'' -Recurse',
        'Remove-ItemProperty -Path $key -Name x',
        'sp HKCU:\Software\AMD x 1',
        'Microsoft.PowerShell.Management\Set-ItemProperty -Path $key -Name x -Value 1',
        'ri $key',
        'reg add HKCU\Software\AMD /v x /d 1',
        '& ''C:\Windows\System32\reg.exe'' delete HKCU\Software\AMD /f',
        'regedit /s settings.reg',
        'Start-Process -FilePath reg.exe -ArgumentList ''add HKCU\Software\AMD /v x /d 1''',
        'cmd /c "reg add HKCU\Software\AMD /v x /d 1"',
        '$key.SetValue(''DvrEnabled'', 0)',
        '[Microsoft.Win32.Registry]::SetValue(''HKEY_CURRENT_USER\Software\AMD'', ''x'', 1)',
        '$key.CreateSubKey(''x'')',
        '$key.DeleteValue(''x'')',
        '$key.DeleteSubKeyTree(''x'', $false)',
        '$key.SetAccessControl($acl)',
        '$key.OpenSubKey(''x'', $true)',
        '$key.OpenSubKey(''x'', $writable)',
        '$key.OpenSubKey(''x'', [Microsoft.Win32.RegistryKeyPermissionCheck]::ReadWriteSubTree)',
        '$key.OpenSubKey(''x'', $false, [Security.AccessControl.RegistryRights]::FullControl)',
        '$key.OpenSubKey(''x'')',
        '$open = $key.OpenSubKey',
        '$key.$method(''x'', 1)'
    ) '^snippet\.ps1:1: '
}

Test-Case 'R7 passes read-only opens, reads, other members, and the words in strings and comments' {
    Assert-ScanPasses $script:R7 @(
        '$key.OpenSubKey(''Software\AMD'', $false)',
        '[Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::LocalMachine, [Microsoft.Win32.RegistryView]::Registry64).OpenSubKey(''SOFTWARE\AMD'', $FALSE)',
        '$key.GetValue(''x'', $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)',
        '$key.GetValueNames(); $key.GetSubKeyNames(); $key.GetValueKind(''x''); $key.Dispose()',
        'Get-ItemProperty -LiteralPath ''HKLM:\SOFTWARE\AMD\DVR'' -Name InstantReplayEnabled',
        'Get-ChildItem -LiteralPath ''HKCU:\Software\AMD''',
        '$map.Remove(''x''); $list.Add(1); $set.Contains(''x'')',
        'Start-Process -FilePath $exe -ArgumentList ''--register'', ''regular''',
        '''Set-ItemProperty and $key.SetValue in a string''',
        '# $key.SetValue(''x'', 1) in a comment'
    )
}

Complete-BenchTests
