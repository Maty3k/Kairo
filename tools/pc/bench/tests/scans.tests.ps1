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
# - every script: tools/**/*.ps1, the tests included.
#
# R1 Module lists (CLAUDE.md, Never 3), in the bench scripts: no member named Path, MainModule,
#    Modules, Company, FileVersion, ProductVersion, Description, Product or CommandLine on anything
#    but a type literal ([IO.Path]::Combine is fine), whether written $p.Path, $p.'Path',
#    $p.PSObject.Properties['Path'] or as a cmdlet's property argument (Select-Object Path,
#    ForEach-Object Path); and no member name computed at run time. On a process the first eight go
#    through MainModule, which opens it with memory-read rights and lists its modules, and
#    CommandLine is a WMI query for its command line (research 17 section 5.3).
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
#    (research 17 sections 4.2 and 4.6). Cmdlets that write files, items or properties; a > or >>
#    redirection other than to $null; [IO.File] and [IO.Directory] beyond their reading members;
#    FileStream, StreamWriter or BinaryWriter; members that write, move or delete.
# R5 Dot-sourcing safely (research 17 section 2 item 1), in the bench scripts: function definitions
#    and, last, one command line guarded by $MyInvocation.InvocationName -ne '.'; no script param
#    block (dot-sourcing would bind it), no begin or process block, no exit outside the guard, no
#    Mandatory parameter and no Read-Host (-NonInteractive can't prompt).
# R6 Windows PowerShell 5.1's grammar, in every script: no ?:, &&, ||, ??, ??=, ?., ?[, background
#    &, `u{} escape or number suffix (y, uy, s, us, u, ul, n), which PowerShell 7 added. CI parses
#    every script with 5.1 on Windows; this rule catches them on the Mac and on Linux too.
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

# --- R1 module lists ---------------------------------------------------------------------------

$script:ModuleMembers = @('Path', 'MainModule', 'Modules', 'Company', 'FileVersion', 'ProductVersion', 'Description', 'Product', 'CommandLine')
# Cmdlets (and aliases) that read the properties their arguments name.
$script:PropertyCommands = @('ForEach-Object', '%', 'foreach', 'Select-Object', 'select', 'Sort-Object', 'sort',
    'Group-Object', 'group', 'Where-Object', '?', 'where', 'Measure-Object', 'measure', 'Format-Table', 'ft',
    'Format-List', 'fl', 'Format-Wide', 'fw', 'Format-Custom', 'fc', 'Compare-Object', 'compare', 'diff',
    'Export-Csv', 'epcsv', 'ConvertTo-Csv', 'ConvertTo-Html', 'Out-GridView', 'ogv')

function Find-ScanModuleMembers($Source) {
    $found = [Collections.Generic.List[string]]::new()
    foreach ($node in (Find-ScanNodes $Source.Ast ([Management.Automation.Language.MemberExpressionAst]))) {
        if ($node.Expression -is [Management.Automation.Language.TypeExpressionAst]) { continue }
        $name = Get-ScanMemberName $node
        if ($null -eq $name) {
            $found.Add((New-ScanFinding $Source $node ('a member name computed at run time: ' + $node.Extent.Text)))
        } elseif ($script:ModuleMembers -contains $name) {
            $found.Add((New-ScanFinding $Source $node ('.' + $name)))
        }
    }
    foreach ($node in (Find-ScanNodes $Source.Ast ([Management.Automation.Language.IndexExpressionAst]))) {
        foreach ($name in (Get-ScanStrings $node.Index)) {
            if ($script:ModuleMembers -contains $name) { $found.Add((New-ScanFinding $Source $node ('[''' + $name + ''']'))) }
        }
    }
    foreach ($node in (Find-ScanNodes $Source.Ast ([Management.Automation.Language.CommandAst]))) {
        $command = $node.GetCommandName()
        if ($null -eq $command -or $script:PropertyCommands -notcontains $command) { continue }
        for ($i = 1; $i -lt $node.CommandElements.Count; $i++) {
            foreach ($name in (Get-ScanStrings $node.CommandElements[$i])) {
                if ($script:ModuleMembers -contains $name) { $found.Add((New-ScanFinding $Source $node ($command + ' ' + $name))) }
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
            $command = $node.GetCommandName()
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
        $command = $node.GetCommandName()
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
# [IO.File] and [IO.Directory] may only read.
$script:FileTypes = @('IO.File', 'System.IO.File', 'IO.Directory', 'System.IO.Directory')
$script:FileReads = @('ReadAllText', 'ReadAllBytes', 'ReadAllLines', 'ReadLines', 'Exists', 'OpenRead', 'GetAttributes',
    'GetLastWriteTime', 'GetLastWriteTimeUtc', 'GetCreationTime', 'GetCreationTimeUtc', 'GetLastAccessTime',
    'GetLastAccessTimeUtc', 'EnumerateFiles', 'EnumerateDirectories', 'EnumerateFileSystemEntries', 'GetFiles',
    'GetDirectories', 'GetFileSystemEntries', 'GetCurrentDirectory')
$script:WriterTypes = '^(System\.)?IO\.(FileStream|StreamWriter|BinaryWriter)\z'
# Members that write, move or delete, whatever they are called on: files (FileInfo and the like),
# and the registry keys and values the scripts never change.
$script:WriteMembers = @('WriteAllText', 'WriteAllBytes', 'WriteAllLines', 'AppendAllText', 'AppendAllLines', 'AppendText',
    'CreateText', 'OpenWrite', 'CreateDirectory', 'CreateSubdirectory', 'MoveTo', 'CopyTo', 'Delete', 'Encrypt', 'Decrypt',
    'SetAccessControl', 'SetAttributes', 'SetLastWriteTime', 'SetLastWriteTimeUtc', 'SetCreationTime', 'SetCreationTimeUtc',
    'CreateSymbolicLink', 'SetValue', 'DeleteValue', 'DeleteSubKey', 'DeleteSubKeyTree', 'CreateSubKey')

function Find-ScanFileWrites($Source) {
    $found = [Collections.Generic.List[string]]::new()
    foreach ($node in (Find-ScanNodes $Source.Ast ([Management.Automation.Language.CommandAst]))) {
        $command = $node.GetCommandName()
        if ($null -eq $command) { continue }
        if ($script:WriteCommands -contains $command) {
            $found.Add((New-ScanFinding $Source $node $command))
        } elseif ($command -eq 'Write-TextFileAtomic') {
            # The decision file's writer, and only for that file: an argument must name it.
            $named = $false
            for ($i = 1; $i -lt $node.CommandElements.Count; $i++) {
                if ($node.CommandElements[$i].Extent.Text -match '(?i)decision') { $named = $true }
            }
            if (-not $named) { $found.Add((New-ScanFinding $Source $node 'Write-TextFileAtomic on a file other than the decision file')) }
        } elseif ($command -eq 'New-Object') {
            for ($i = 1; $i -lt $node.CommandElements.Count; $i++) {
                foreach ($name in (Get-ScanStrings $node.CommandElements[$i])) {
                    if ($name -match $script:WriterTypes) { $found.Add((New-ScanFinding $Source $node ('New-Object ' + $name))) }
                }
            }
        }
    }
    foreach ($node in (Find-ScanNodes $Source.Ast ([Management.Automation.Language.FileRedirectionAst]))) {
        $toNull = ($node.Location -is [Management.Automation.Language.VariableExpressionAst] -and $node.Location.VariablePath.UserPath -eq 'null')
        if (-not $toNull) { $found.Add((New-ScanFinding $Source $node ('a redirection to a file: ' + $node.Extent.Text))) }
    }
    foreach ($node in (Find-ScanNodes $Source.Ast ([Management.Automation.Language.MemberExpressionAst]))) {
        $name = Get-ScanMemberName $node
        if ($null -eq $name) { continue }
        if ($node.Expression -is [Management.Automation.Language.TypeExpressionAst]) {
            $type = $node.Expression.TypeName.FullName
            if ($script:FileTypes -contains $type -and $script:FileReads -notcontains $name) {
                $found.Add((New-ScanFinding $Source $node ('[' + $type + ']::' + $name)))
            } elseif ($type -match $script:WriterTypes) {
                $found.Add((New-ScanFinding $Source $node ('[' + $type + ']::' + $name)))
            }
        } elseif ($script:WriteMembers -contains $name) {
            $found.Add((New-ScanFinding $Source $node ('.' + $name)))
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
        if ($node.GetCommandName() -eq 'Read-Host') { $found.Add((New-ScanFinding $Source $node 'Read-Host, which would prompt')) }
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
        if ($kind -eq 'Number' -and $token.Text -match '(?i)[ysun]\z') {
            $found.Add((New-ScanFinding $Source $token ('the number ' + $token.Text + ', whose suffix PowerShell 5.1 lacks')))
        }
    }
    return ,$found.ToArray()
}

# --- The files -----------------------------------------------------------------------------------

$script:AllFiles = @(Get-ChildItem -LiteralPath $script:ToolsDir -Recurse -File -Filter '*.ps1' | Sort-Object FullName | ForEach-Object { Read-ScanFile $_.FullName })
$script:BenchFiles = @($script:AllFiles | Where-Object { $_.Name -match '^tools/pc/bench/[^/]+\.ps1\z' })
$script:ElevatedFiles = @($script:AllFiles | Where-Object { $_.Name -match '^tools/pc/bench/[^/]+\.ps1\z' -or $_.Name -match '^tools/mac/remote/[^/]+\.ps1\z' })
$script:ElevatedBenchFiles = @($script:AllFiles | Where-Object { $_.Name -match '^tools/pc/bench/[^/]+\.ps1\z' -or $_.Name -match '^tools/mac/remote/bench[^/]*\.ps1\z' })

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
    Assert-Equal 'tools/pc/bench/ab-stats.ps1,tools/pc/bench/bench-rules.ps1,tools/pc/bench/encoder-usage.ps1,tools/pc/bench/pm-stats.ps1' $bench 'the bench scripts'
    Assert-True (@($script:ElevatedFiles | Where-Object { $_.Name -eq 'tools/mac/remote/lib.ps1' }).Count -eq 1) 'lib.ps1 is an elevated script'
    Assert-True (@($script:AllFiles | Where-Object { $_.Name -eq 'tools/mac/remote/status.ps1' }).Count -eq 1) 'status.ps1 is scanned'
    Assert-True (@($script:AllFiles | Where-Object { $_.Name -eq 'tools/pc/bench/tests/scans.tests.ps1' -and $_.IsTest }).Count -eq 1) 'this file is scanned, as a test'
    Assert-True ($script:AllFiles.Count -ge 20) ('every script: ' + $script:AllFiles.Count)
}

# --- R1 --------------------------------------------------------------------------------------------

$script:R1 = { param($Text) Find-ScanModuleMembers (ConvertTo-ScanSource $Text) }

Test-Case 'R1 the bench scripts read no member that lists modules or reads a command line' {
    foreach ($source in $script:BenchFiles) { Assert-ScanClean (Find-ScanModuleMembers $source) $source.Name }
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
        '# $p.Path in a comment',
        '''$p.Path in a string'''
    )
}

# --- R2 --------------------------------------------------------------------------------------------

$script:Helper = @'
function Get-ProcessIdAndName {
    $list = [Collections.Generic.List[object]]::new()
    foreach ($process in [Diagnostics.Process]::GetProcesses()) {
        try { $list.Add([pscustomobject]@{ Id = [long]$process.Id; Name = [string]$process.ProcessName }) } finally { $process.Dispose() }
    }
    return ,$list.ToArray()
}
'@
$script:R2 = { param($Text) (Find-ScanProcessLists @(ConvertTo-ScanSource $Text)).Findings }

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
        'Get-Process @splat'
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
    Assert-ScanFlags $script:R3 @('Add-Type -TypeDefinition $code', 'Import-Csv -LiteralPath $file', 'ipcsv x.csv') '^snippet\.ps1:1: (Add-Type|Import-Csv|ipcsv)'
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
        '$key.SetValue(''x'', 1)',
        'Write-TextFileAtomic $f $t',
        'Write-TextFileAtomic -Path (Join-Path $Kairo.Jobs ''next.json'') -Text $t'
    ) '^snippet\.ps1:1: '
}

Test-Case 'R4 passes reads, console output and the decision file' {
    Assert-ScanPasses $script:R4 @(
        '[IO.File]::ReadAllBytes($f)',
        '[IO.File]::Exists($f)',
        '[IO.FileInfo]::new($f).Length',
        '[IO.Path]::GetFullPath($f)',
        '[Console]::Out.WriteLine($t)',
        '[Console]::Error.WriteLine($t)',
        '$null = Get-Thing 2> $null',
        'Get-Thing 2>&1',
        '$t.Replace(''a'', ''b'')',
        '[scriptblock]::Create($text)',
        'Write-TextFileAtomic (Join-Path $Kairo.Jobs ($id + ''.decision.json'')) $json',
        'Write-TextFileAtomic -Path $decisionFile -Text $json'
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
    Assert-ScanFlags $script:R5 @($script:Library.Replace('return $Name', 'return (Read-Host ''name'')')) 'Read-Host'
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
        '$x = 0x1Fuy'
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
        '$x = 10; $y = 10L; $z = 1kb; $w = 0x1F; $v = 2.5d; $u = 1e5',
        '''$a ?? 1 in a string'''
    )
}

Complete-BenchTests
