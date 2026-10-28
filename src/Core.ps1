# Optimaxer engine: logging, registry/service/task/power helpers, tweak apply/undo/test, state store.
# Dot-sourced by the UI process and by every background job runspace. Shared state lives in $global:Opti*.

$global:OptiData      = Join-Path $env:ProgramData 'Optimaxer'
$global:OptiStatePath = Join-Path $global:OptiData 'state.json'
$global:OptiLogPath   = Join-Path $global:OptiData 'optimaxer.log'
$global:OptiBackupDir = Join-Path $global:OptiData 'backups'
foreach ($d in $global:OptiData, $global:OptiBackupDir) {
    if (-not (Test-Path -LiteralPath $d)) { New-Item -ItemType Directory -Path $d -Force | Out-Null }
}
if (-not (Get-PSDrive -Name HKU -ErrorAction SilentlyContinue)) {
    New-PSDrive -Name HKU -PSProvider Registry -Root HKEY_USERS -Scope Global | Out-Null
}

# ---------------------------------------------------------------- logging
function Write-Log {
    param([string]$Message, [string]$Level = 'INFO')
    $line = '[{0}] {1,-5} {2}' -f (Get-Date -Format 'HH:mm:ss'), $Level, $Message
    if ($global:OptiLogQueue) { $global:OptiLogQueue.Enqueue($line) }
    if ($global:OptiConsole) { Write-Host $line }
    try { Add-Content -LiteralPath $global:OptiLogPath -Value $line -Encoding UTF8 } catch {}
}

function Format-Size {
    param([double]$Bytes)
    if ($Bytes -ge 1GB) { return '{0:N2} GB' -f ($Bytes / 1GB) }
    if ($Bytes -ge 1MB) { return '{0:N1} MB' -f ($Bytes / 1MB) }
    if ($Bytes -ge 1KB) { return '{0:N0} KB' -f ($Bytes / 1KB) }
    return '{0:N0} B' -f $Bytes
}

function Invoke-Native {
    # Runs an external program, returns combined output as one string. Exit code in $global:OptiExit.
    param([string]$Exe, [string[]]$Arguments = @())
    $out = & $Exe @Arguments 2>&1 | Out-String
    $global:OptiExit = $LASTEXITCODE
    return $out
}

# ---------------------------------------------------------------- state store (original values for undo)
function Get-OptiState {
    $h = @{}
    if (Test-Path -LiteralPath $global:OptiStatePath) {
        try {
            $o = Get-Content -LiteralPath $global:OptiStatePath -Raw -ErrorAction Stop | ConvertFrom-Json
            foreach ($p in $o.PSObject.Properties) { $h[$p.Name] = $p.Value }
        } catch {
            # Keep the unreadable file: the next Save-OptiState would otherwise overwrite every saved original value.
            $why = $_.Exception.Message
            $keep = Join-Path $global:OptiBackupDir ('state-unreadable-{0}.json' -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
            try { Copy-Item -LiteralPath $global:OptiStatePath -Destination $keep -Force -ErrorAction Stop; Write-Log "State file unreadable ($why); copy kept at $keep" 'WARN' }
            catch { Write-Log "State file unreadable: $why" 'WARN' }
        }
    }
    return $h
}

function Save-OptiState {
    # Write to a temp file and swap it in, so a crash or full disk cannot leave a half-written state file.
    param($State)
    $tmp = "$global:OptiStatePath.tmp"
    $State | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $tmp -Encoding UTF8 -ErrorAction Stop
    Move-Item -LiteralPath $tmp -Destination $global:OptiStatePath -Force -ErrorAction Stop
}

# ---------------------------------------------------------------- registry
function Open-RegKey {
    # Read-only .NET handle for a PowerShell-style path (HKLM:\..., HKCU:\..., HKU:\..., HKCR:\...); $null when missing.
    # Roughly 10x faster than Get-Item on the provider, which matters for the ~400 reads of a status pass.
    param([string]$Path)
    if ($Path -notmatch '^(HKLM|HKCU|HKU|HKCR):\\?(.*)$') { return $null }
    $root = switch ($Matches[1]) {
        'HKLM' { [Microsoft.Win32.Registry]::LocalMachine }
        'HKCU' { [Microsoft.Win32.Registry]::CurrentUser }
        'HKU'  { [Microsoft.Win32.Registry]::Users }
        'HKCR' { [Microsoft.Win32.Registry]::ClassesRoot }
    }
    $sub = $Matches[2].TrimEnd('\')
    try { if ($sub) { return $root.OpenSubKey($sub, $false) } else { return $root } } catch { return $null }
}

function Get-RegSnapshot {
    param($Entry)
    $exists = $false; $val = $null; $type = $null
    try {
        $key = Open-RegKey $Entry.P
        if ($key) {
            try {
                $val = $key.GetValue($Entry.N, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
                if ($null -ne $val) { $exists = $true; $type = $key.GetValueKind($Entry.N).ToString() }
            } finally { $key.Close() }
        } elseif ($Entry.P -notmatch '^(HKLM|HKCU|HKU|HKCR):') {
            $key = Get-Item -LiteralPath $Entry.P -ErrorAction Stop
            if ($key.GetValueNames() -contains $Entry.N) {
                $exists = $true
                $val  = $key.GetValue($Entry.N, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
                $type = $key.GetValueKind($Entry.N).ToString()
            }
        }
    } catch {}
    return [pscustomobject]@{ Exists = $exists; Value = $val; Type = $type }
}

function ConvertTo-RegValue {
    param($Value, [string]$Type)
    switch ($Type) {
        'DWord'       { return [BitConverter]::ToInt32([BitConverter]::GetBytes([uint32]([int64]$Value -band 4294967295L)), 0) }
        'QWord'       { return [int64]$Value }
        'Binary'      { return [byte[]]@($Value) }
        'MultiString' { return [string[]]@($Value) }
        default       { return [string]$Value }
    }
}

function Set-RegEntry {
    param([string]$Path, [string]$Name, $Value, [string]$Type = 'DWord')
    # -ErrorAction Stop so callers can catch and log protected keys instead of silently failing
    if (-not (Test-Path -LiteralPath $Path)) { New-Item -Path $Path -Force -ErrorAction Stop | Out-Null }
    $v = ConvertTo-RegValue $Value $Type
    New-ItemProperty -LiteralPath $Path -Name $Name -Value $v -PropertyType $Type -Force -ErrorAction Stop | Out-Null
}

function Remove-RegEntry {
    param([string]$Path, [string]$Name)
    Remove-ItemProperty -LiteralPath $Path -Name $Name -ErrorAction SilentlyContinue
}

function Compare-RegValue {
    param($Actual, $Wanted, [string]$Type)
    switch ($Type) {
        'Binary' { return (($Actual -join ',') -eq ($Wanted -join ',')) }
        { $_ -in 'DWord', 'QWord' } { return (([int64]$Actual -band 4294967295L) -eq ([int64]$Wanted -band 4294967295L)) }
        default  { return ("$Actual" -eq "$Wanted") }
    }
}

function Restore-RegSnapshot {
    # Puts one saved value back (or removes it when it did not exist). Returns $false when the write failed.
    param($Rec)
    try {
        if ($Rec.Exists) { Set-RegEntry $Rec.P $Rec.N $Rec.Value $Rec.Type }
        else {
            if ((Test-Path -LiteralPath $Rec.P) -and $null -ne (Get-ItemProperty -LiteralPath $Rec.P -Name $Rec.N -ErrorAction SilentlyContinue)) {
                Remove-ItemProperty -LiteralPath $Rec.P -Name $Rec.N -ErrorAction Stop
            }
        }
        return $true
    } catch { Write-Log "  restore failed $($Rec.P)\$($Rec.N): $($_.Exception.Message)" 'WARN'; return $false }
}

# ---------------------------------------------------------------- services
function Get-SvcSnapshot {
    param([string]$Name)
    $key = $null
    try {
        $key = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey("SYSTEM\CurrentControlSet\Services\$Name", $false)
        if (-not $key) { return $null }
        $start = $key.GetValue('Start', $null)
        if ($null -eq $start) { return $null }
        return @{ Name = $Name; Start = [int]$start; Delayed = [int]($key.GetValue('DelayedAutostart', 0) -eq 1) }
    } catch { return $null }
    finally { if ($key) { $key.Close() } }
}

function Set-SvcStart {
    param([string]$Name, [int]$Start, [int]$Delayed = 0)
    $mode = switch ($Start) {
        2 { if ($Delayed) { 'delayed-auto' } else { 'auto' } }
        3 { 'demand' }
        4 { 'disabled' }
        default { return $false }
    }
    & sc.exe config $Name start= $mode 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) {
        try {
            $k = "HKLM:\SYSTEM\CurrentControlSet\Services\$Name"
            Set-ItemProperty -LiteralPath $k -Name Start -Value $Start -Type DWord -ErrorAction Stop
            if ($Start -eq 2 -and $Delayed) { Set-ItemProperty -LiteralPath $k -Name DelayedAutostart -Value 1 -Type DWord }
            else { Remove-ItemProperty -LiteralPath $k -Name DelayedAutostart -ErrorAction SilentlyContinue }
        } catch { return $false }
    }
    if ($Start -eq 4) { Stop-Service -Name $Name -Force -ErrorAction SilentlyContinue }
    return $true
}

# ---------------------------------------------------------------- scheduled tasks
function Split-TaskPath {
    param([string]$Full)
    $i = $Full.LastIndexOf('\')
    return @{ Path = '\' + $Full.Substring(0, $i + 1).TrimStart('\'); Name = $Full.Substring($i + 1) }
}

$global:TaskCache = @{}
function Get-TaskSnapshot {
    # one Get-ScheduledTask call per folder (each call is slow), cached until Clear-TaskCache
    param([string]$Full)
    $s = Split-TaskPath $Full
    if (-not $global:TaskCache.ContainsKey($s.Path)) {
        $global:TaskCache[$s.Path] = @{}
        foreach ($t in Get-ScheduledTask -TaskPath $s.Path -ErrorAction SilentlyContinue) { $global:TaskCache[$s.Path][$t.TaskName] = [int]($t.State -ne 'Disabled') }
    }
    $state = $global:TaskCache[$s.Path][$s.Name]
    if ($null -eq $state) { return $null }
    return @{ Full = $Full; Enabled = $state }
}

function Clear-TaskCache { $global:TaskCache = @{} }

function Set-TaskEnabled {
    param([string]$Full, [bool]$Enabled)
    $s = Split-TaskPath $Full
    try {
        if ($Enabled) { Enable-ScheduledTask  -TaskPath $s.Path -TaskName $s.Name -ErrorAction Stop | Out-Null }
        else          { Disable-ScheduledTask -TaskPath $s.Path -TaskName $s.Name -ErrorAction Stop | Out-Null }
        return $true
    } catch { Write-Log "  task $Full : $($_.Exception.Message)" 'WARN'; return $false }
}

# ---------------------------------------------------------------- power (powercfg output is parsed language-independently)
# During a status pass these caches stop every power tweak from spawning its own powercfg.exe (set by Get-TweakStatus).
$global:OptiPowerCache = $null

function Get-ActiveSchemeGuid {
    if ($global:OptiPowerCache -and $global:OptiPowerCache.ContainsKey('active')) { return $global:OptiPowerCache['active'] }
    $o = powercfg /getactivescheme | Out-String
    if ($global:OptiPowerCache -and $o -match '([0-9a-fA-F]{8}(-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12})') { $global:OptiPowerCache['active'] = $Matches[1] }
    if ($o -match '([0-9a-fA-F]{8}(-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12})') { return $Matches[1] }
}

function Get-SchemeName {
    param([string]$Guid)
    if ($global:OptiPowerCache) {
        if (-not $global:OptiPowerCache.ContainsKey('list')) { $global:OptiPowerCache['list'] = powercfg /list | Out-String }
        $o = $global:OptiPowerCache['list']
    } else { $o = powercfg /list | Out-String }
    if ($o -match [regex]::Escape($Guid) + '\s+\((.+?)\)') { return $Matches[1] }
    return ''
}

function Get-SchemeGuidByName {
    param([string]$Pattern)
    foreach ($line in (powercfg /list)) {
        if ($line -match '([0-9a-fA-F]{8}(-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12})\s+\((.+?)\)' -and $Matches[3] -match $Pattern) { return $Matches[1] }
    }
}

function Get-PowerAC {
    param([string]$Sub, [string]$Setting)
    $ck = "q:$Sub/$Setting"
    if ($global:OptiPowerCache -and $global:OptiPowerCache.ContainsKey($ck)) { $o = $global:OptiPowerCache[$ck] }
    else {
        $o = powercfg /q SCHEME_CURRENT $Sub $Setting | Out-String
        if ($global:OptiPowerCache) { $global:OptiPowerCache[$ck] = $o }
    }
    $hex = [regex]::Matches($o, '0x[0-9a-fA-F]{8}')
    if ($hex.Count -ge 2) { return [Convert]::ToInt32($hex[$hex.Count - 2].Value.Substring(2), 16) }
    return $null
}

function Set-PowerAC {
    # Items: @{Sub=guid; Set=guid; Val=int}. Returns the previous values in the same shape (for undo).
    param($Items)
    $old = @()
    foreach ($i in $Items) {
        $cur = Get-PowerAC $i.Sub $i.Set
        if ($null -ne $cur) { $old += @{ Sub = $i.Sub; Set = $i.Set; Val = $cur } }
        powercfg /setacvalueindex SCHEME_CURRENT $i.Sub $i.Set ([int]$i.Val) | Out-Null
    }
    powercfg /setactive SCHEME_CURRENT | Out-Null
    return , $old
}

function Test-PowerAC {
    param($Items)
    foreach ($i in $Items) { if ((Get-PowerAC $i.Sub $i.Set) -ne $i.Val) { return $false } }
    return $true
}

# ---------------------------------------------------------------- restore point
function New-OptiRestorePoint {
    param([string]$Description = 'Optimaxer')
    $k = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\SystemRestore'
    try {
        Enable-ComputerRestore -Drive "$env:SystemDrive\" -ErrorAction SilentlyContinue
        $old = (Get-ItemProperty -LiteralPath $k -Name SystemRestorePointCreationFrequency -ErrorAction SilentlyContinue).SystemRestorePointCreationFrequency
        Set-ItemProperty -LiteralPath $k -Name SystemRestorePointCreationFrequency -Value 0 -Type DWord
        Checkpoint-Computer -Description $Description -RestorePointType MODIFY_SETTINGS -ErrorAction Stop
        if ($null -eq $old) { Remove-ItemProperty -LiteralPath $k -Name SystemRestorePointCreationFrequency -ErrorAction SilentlyContinue }
        else { Set-ItemProperty -LiteralPath $k -Name SystemRestorePointCreationFrequency -Value $old -Type DWord }
        Write-Log "Restore point created: $Description" 'OK'
        return $true
    } catch {
        Write-Log "Restore point failed: $($_.Exception.Message)" 'WARN'
        return $false
    }
}

# ---------------------------------------------------------------- snapshot helpers for script tweaks
# Each Get-* reads what a tweak will change and the matching Restore-* puts exactly that back. A Restore-* throws when
# a step fails, so Invoke-TweakUndo keeps the record.

# TCP interface keys (net-nagle). A variable so tests can point it at a scratch key.
$global:OptiTcpIfBase = 'HKLM:\SYSTEM\CurrentControlSet\Services\Tcpip\Parameters\Interfaces'

function Get-NagleSnapshot {
    # One entry per interface with an IP address: whether TcpAckFrequency / TCPNoDelay existed, and their values.
    foreach ($k in Get-ChildItem -LiteralPath $global:OptiTcpIfBase -ErrorAction SilentlyContinue) {
        $p = Get-ItemProperty -LiteralPath $k.PSPath -ErrorAction SilentlyContinue
        if (-not ($p.DhcpIPAddress -or $p.IPAddress)) { continue }
        $path = Join-Path $global:OptiTcpIfBase $k.PSChildName
        $rec = @{ If = $k.PSChildName }
        foreach ($n in 'TcpAckFrequency', 'TCPNoDelay') {
            $s = Get-RegSnapshot @{ P = $path; N = $n }
            $rec[$n] = @{ Exists = [bool]$s.Exists; Value = $s.Value; Type = $s.Type }
        }
        $rec
    }
}

function Restore-NagleSnapshot {
    param($Data)
    $bad = @()
    foreach ($i in @($Data)) {
        if (-not $i) { continue }
        if ($i -is [string]) {
            # record from v2.13.5 or older: only the interface name was kept, and those versions deleted both values
            Remove-ItemProperty -LiteralPath (Join-Path $global:OptiTcpIfBase $i) -Name TcpAckFrequency, TCPNoDelay -ErrorAction SilentlyContinue
            continue
        }
        $path = Join-Path $global:OptiTcpIfBase $i.If
        foreach ($n in 'TcpAckFrequency', 'TCPNoDelay') {
            $o = $i.$n
            if (-not $o) { continue }
            if (-not (Restore-RegSnapshot @{ P = $path; N = $n; Exists = [bool]$o.Exists; Value = $o.Value; Type = $o.Type })) { $bad += "$path\$n" }
        }
    }
    if ($bad.Count) { throw "could not restore $($bad -join ', ')" }
}

function Test-NagleApplied {
    $any = $false
    foreach ($i in @(Get-NagleSnapshot)) {
        $any = $true
        if (-not ($i.TcpAckFrequency.Exists -and $i.TcpAckFrequency.Value -eq 1 -and $i.TCPNoDelay.Exists -and $i.TCPNoDelay.Value -eq 1)) { return $false }
    }
    return $any
}

function Invoke-Bcd {
    param([string[]]$Arguments)
    $o = Invoke-Native bcdedit.exe $Arguments
    if ($global:OptiExit -ne 0) { throw "bcdedit $($Arguments -join ' ') failed: $($o.Trim())" }
    return $o
}

function Get-BcdTimerValues {
    # The boot entries perf-timer touches, exactly as bcdedit prints them for {current}; $null = not set.
    $o = Invoke-Bcd @('/enum', '{current}')
    $v = @{}
    foreach ($n in 'disabledynamictick', 'useplatformclock', 'useplatformtick') {
        $v[$n] = if ($o -match "(?im)^$n\s+(\S+)\s*$") { $Matches[1] } else { $null }
    }
    return $v
}

function Restore-BcdTimerValues {
    param($Data)
    if (-not $Data) { throw 'no saved boot values' }
    if ($null -ne $Data.HadPlatformClock) {
        # record from v2.13.5 or older, which kept only this flag
        [void](Invoke-Bcd @('/deletevalue', 'disabledynamictick'))
        if ($Data.HadPlatformClock) { [void](Invoke-Bcd @('/set', 'useplatformclock', 'yes')) }
        return
    }
    $now = Get-BcdTimerValues
    foreach ($n in 'disabledynamictick', 'useplatformclock', 'useplatformtick') {
        $old = $Data.$n
        if ($old) { if ($now[$n] -ne $old) { [void](Invoke-Bcd @('/set', $n, $old)) } }
        elseif ($now[$n]) { [void](Invoke-Bcd @('/deletevalue', $n)) }
    }
}

$global:OptiSleepLogs = @('Microsoft-Windows-SleepStudy/Diagnostic', 'Microsoft-Windows-Kernel-Processor-Power/Diagnostic', 'Microsoft-Windows-UserModePowerService/Diagnostic')

function Invoke-Wevtutil {
    param([string[]]$Arguments)
    $o = Invoke-Native wevtutil.exe $Arguments
    if ($global:OptiExit -ne 0) { throw "wevtutil $($Arguments -join ' ') failed: $($o.Trim())" }
    return $o
}

function Get-EventLogState {
    # $true / $false = the log's enabled flag; $null = this Windows has no such log.
    param([string]$Log)
    $o = Invoke-Native wevtutil.exe @('gl', $Log)
    if ($global:OptiExit -ne 0) { return $null }
    if ($o -match '(?im)^\s*enabled:\s*(true|false)') { return ($Matches[1] -eq 'true') }
    return $null
}

function Get-EventLogStates {
    param([string[]]$Logs)
    $h = @{}
    foreach ($l in $Logs) { $s = Get-EventLogState $l; if ($null -ne $s) { $h[$l] = $s } }
    return $h
}

function Restore-EventLogStates {
    param($Data)
    # records from v2.13.5 or older saved nothing; those versions always turned the logs back on
    if ($null -eq $Data) { $Data = @{}; foreach ($l in $global:OptiSleepLogs) { if ($null -ne (Get-EventLogState $l)) { $Data[$l] = $true } } }
    $bad = @()
    # a hashtable right after apply, a PSCustomObject once it went through state.json
    $pairs = if ($Data -is [hashtable]) { foreach ($k in $Data.Keys) { @{ Name = $k; Value = $Data[$k] } } }
             else { foreach ($p in $Data.PSObject.Properties) { @{ Name = $p.Name; Value = $p.Value } } }
    foreach ($p in @($pairs)) {
        $want = if ([bool]$p.Value) { 'true' } else { 'false' }
        try { [void](Invoke-Wevtutil @('set-log', $p.Name, "/e:$want")) } catch { $bad += $p.Name }
    }
    if ($bad.Count) { throw "could not restore log state: $($bad -join ', ')" }
}

# ---------------------------------------------------------------- tweak engine
function Get-TweakById {
    param([string]$Id)
    return $global:OptiCatalog | Where-Object { $_.Id -eq $Id } | Select-Object -First 1
}

function Test-Tweak {
    # True when the tweak is currently in effect.
    param($T)
    if ($T.Test) { try { return [bool](& $T.Test) } catch { return $false } }
    $checked = $false
    foreach ($r in $T.Reg) {
        $checked = $true
        $s = Get-RegSnapshot $r
        if (-not $s.Exists -or -not (Compare-RegValue $s.Value $r.V $r.T)) { return $false }
    }
    if ($T.Svc.Count) {
        $total = 0; $ok = 0
        foreach ($s in $T.Svc) {
            $snap = Get-SvcSnapshot $s.Name
            if (-not $snap) { continue }
            $total++; if ($snap.Start -eq $s.Start) { $ok++ }
        }
        if ($total) { $checked = $true; if (($ok / $total) -lt $T.SvcTol) { return $false } }
    }
    foreach ($tk in $T.Task) {
        $snap = Get-TaskSnapshot $tk
        if (-not $snap) { continue }
        $checked = $true
        if ($snap.Enabled) { return $false }
    }
    if (-not $checked) {
        $st = if ($global:OptiStateCache) { $global:OptiStateCache } else { Get-OptiState }
        return $null -ne $st[$T.Id]
    }
    return $true
}

function Get-TweakStatus {
    # Returns one @{Id; On} per catalog entry. The saved state and powercfg answers are read once for the whole pass.
    param([string[]]$Ids)
    Clear-TaskCache
    $global:OptiStateCache = Get-OptiState
    $global:OptiPowerCache = @{}
    try {
        foreach ($t in $global:OptiCatalog) {
            if ($Ids -and $Ids -notcontains $t.Id) { continue }
            @{ Id = $t.Id; On = [bool](Test-Tweak $t) }
        }
    } finally { $global:OptiStateCache = $null; $global:OptiPowerCache = $null }
}

function Find-SharedOriginal {
    # If another tweak already recorded this registry value / service / task, reuse its saved original
    # (the current value may just be what that tweak wrote). Keeps undo order-independent.
    param($State, [string]$ExceptId, [string]$Kind, [string]$Key1, [string]$Key2)
    foreach ($id in $State.Keys) {
        if ($id -eq $ExceptId) { continue }
        $rec = $State[$id]
        switch ($Kind) {
            'Reg'  { foreach ($r in @($rec.Reg))  { if ($r.P -eq $Key1 -and $r.N -eq $Key2) { return $r } } }
            'Svc'  { foreach ($s in @($rec.Svc))  { if ($s.Name -eq $Key1) { return $s } } }
            'Task' { foreach ($t in @($rec.Task)) { if ($t.Full -eq $Key1) { return $t } } }
        }
    }
    return $null
}

function Find-SharedOwner {
    # Id of another saved tweak that also changed this registry value / service / task, or $null.
    param($State, [string]$ExceptId, [string]$Kind, [string]$Key1, [string]$Key2)
    foreach ($id in $State.Keys) {
        if ($id -eq $ExceptId) { continue }
        $rec = $State[$id]
        switch ($Kind) {
            'Reg'  { foreach ($r in @($rec.Reg))  { if ($r.P -eq $Key1 -and $r.N -eq $Key2) { return $id } } }
            'Svc'  { foreach ($s in @($rec.Svc))  { if ($s.Name -eq $Key1) { return $id } } }
            'Task' { foreach ($t in @($rec.Task)) { if ($t.Full -eq $Key1) { return $id } } }
        }
    }
    return $null
}

function Invoke-TweakApply {
    # Order matters for undo: every original value (registry, services, tasks and the tweak's own -Snapshot script)
    # is read first and the record is written to state.json BEFORE the first change. A step that fails later
    # cannot leave a change on disk without its original value saved.
    param($T)
    Write-Log "Apply: $($T.Name)"
    Clear-TaskCache
    $state = Get-OptiState
    # A failed apply or later setting drift must not replace the first saved originals on retry.
    $fresh = -not $state.ContainsKey($T.Id)
    $rec = if ($fresh) { @{ Reg = @(); Svc = @(); Task = @(); Data = $null; Time = (Get-Date).ToString('s') } } else { $state[$T.Id] }

    # 1. read the originals (no writes)
    $svcSnaps = @{}; $taskSnaps = @{}
    foreach ($r in $T.Reg) {
        if (-not $fresh) { break }
        $shared = Find-SharedOriginal $state $T.Id 'Reg' $r.P $r.N
        if ($shared) { $rec.Reg += @{ P = $r.P; N = $r.N; Exists = [bool]$shared.Exists; Value = $shared.Value; Type = $shared.Type } }
        else {
            $s = Get-RegSnapshot $r
            $rec.Reg += @{ P = $r.P; N = $r.N; Exists = $s.Exists; Value = $s.Value; Type = $s.Type }
        }
    }
    foreach ($s in $T.Svc) {
        $snap = Get-SvcSnapshot $s.Name
        if (-not $snap) { continue }
        $svcSnaps[$s.Name] = $snap
        if ($fresh) { $sh = Find-SharedOriginal $state $T.Id 'Svc' $s.Name; $rec.Svc += $(if ($sh) { @{ Name = $s.Name; Start = [int]$sh.Start; Delayed = [int]$sh.Delayed } } else { $snap }) }
    }
    foreach ($tk in $T.Task) {
        $snap = Get-TaskSnapshot $tk
        if (-not $snap) { continue }
        $taskSnaps[$tk] = $snap
        if ($fresh) { $sh = Find-SharedOriginal $state $T.Id 'Task' $tk; $rec.Task += $(if ($sh) { @{ Full = $tk; Enabled = [int]$sh.Enabled } } else { $snap }) }
    }
    if ($fresh -and $T.Snapshot) {
        try { $rec.Data = & $T.Snapshot }
        catch {
            Write-Log "  could not read the current values, nothing changed: $($_.Exception.Message)" 'WARN'
            return (Test-Tweak $T)
        }
    }

    # 2. save the record before anything changes
    $state[$T.Id] = $rec
    try { Save-OptiState $state }
    catch {
        Write-Log "  could not save the undo record, nothing changed: $($_.Exception.Message)" 'WARN'
        return (Test-Tweak $T)
    }

    # 3. change things
    foreach ($r in $T.Reg) {
        try { Set-RegEntry $r.P $r.N $r.V $r.T }
        catch {
            $msg = $_.Exception.Message
            if ($msg -match 'not allowed|denied') { $msg += ' (this key is write-protected on this PC by Windows, a policy or another tool)' }
            Write-Log "  registry $($r.P)\$($r.N): $msg" 'WARN'
        }
    }
    foreach ($s in $T.Svc) {
        $snap = $svcSnaps[$s.Name]
        if (-not $snap) { continue }
        if ($snap.Start -ne $s.Start -and -not (Set-SvcStart $s.Name $s.Start)) { Write-Log "  service $($s.Name): could not change (protected?)" 'WARN' }
    }
    foreach ($tk in $T.Task) { if ($taskSnaps[$tk]) { [void](Set-TaskEnabled $tk $false) } }
    if ($T.Apply) {
        try {
            $d = if ($T.Snapshot) { & $T.Apply $rec.Data } else { & $T.Apply }
            # tweaks without a -Snapshot script return their originals from Apply; save them as soon as they exist
            if ($fresh -and -not $T.Snapshot) { $rec.Data = $d; $state[$T.Id] = $rec; Save-OptiState $state }
        } catch { Write-Log "  script: $($_.Exception.Message)" 'WARN' }
    }
    return (Test-Tweak $T)
}

# Ids whose last undo left something unrestored, with the failed steps. Filled by Invoke-TweakUndo.
$global:OptiUndoFailed = @{}

function Invoke-TweakUndo {
    # Restores every saved original. The record is removed only when every step worked; otherwise it stays so the
    # undo can be retried, the failed steps are logged, and the id is listed in $global:OptiUndoFailed.
    param($T)
    Write-Log "Undo: $($T.Name)"
    Clear-TaskCache
    $state = Get-OptiState
    $rec = $state[$T.Id]
    $failed = @()
    if ($rec) {
        # A value another applied tweak also changed stays as it is until that tweak is undone too; restoring it now
        # would silently switch the other tweak off. The last one undone puts the original back.
        foreach ($r in @($rec.Reg)) {
            $owner = Find-SharedOwner $state $T.Id 'Reg' $r.P $r.N
            if ($owner) { Write-Log "  kept $($r.P)\$($r.N): still used by $owner"; continue }
            if (-not (Restore-RegSnapshot $r)) { $failed += "registry $($r.P)\$($r.N)" }
        }
        foreach ($s in @($rec.Svc)) {
            $owner = Find-SharedOwner $state $T.Id 'Svc' $s.Name
            if ($owner) { Write-Log "  kept service $($s.Name): still used by $owner"; continue }
            if (-not (Set-SvcStart $s.Name $s.Start $s.Delayed)) { $failed += "service $($s.Name)" }
        }
        foreach ($tk in @($rec.Task)) {
            $owner = Find-SharedOwner $state $T.Id 'Task' $tk.Full
            if ($owner) { Write-Log "  kept task $($tk.Full): still used by $owner"; continue }
            if ($tk.Enabled -and -not (Set-TaskEnabled $tk.Full $true)) { $failed += "task $($tk.Full)" }
        }
    } elseif ($T.Reg.Count -or $T.Svc.Count -or $T.Task.Count) {
        Write-Log '  no saved original values for this tweak (was it applied by Optimaxer?)' 'WARN'
    }
    if ($T.Undo) {
        try { & $T.Undo $(if ($rec) { $rec.Data }) | Out-Null } catch { $failed += "undo script: $($_.Exception.Message)" }
    }
    if ($failed.Count) {
        $global:OptiUndoFailed[$T.Id] = $failed
        foreach ($f in $failed) { Write-Log "  not restored: $f" 'WARN' }
        Write-Log "  $($T.Name) was not fully undone; its saved values are kept so you can try again." 'WARN'
    } else {
        $global:OptiUndoFailed.Remove($T.Id)
        if ($state.ContainsKey($T.Id)) { $state.Remove($T.Id); Save-OptiState $state }
    }
    return (Test-Tweak $T)
}

function Invoke-UndoAll {
    # Undoes every tweak that has a saved record. Returns @{ Undone = ids; Failed = @{ id = steps } }.
    $state = Get-OptiState
    $done = @(); $bad = @{}
    foreach ($id in @($state.Keys)) {
        $t = Get-TweakById $id
        if (-not $t) { continue }
        [void](Invoke-TweakUndo $t)
        if ($global:OptiUndoFailed.ContainsKey($id)) { $bad[$id] = $global:OptiUndoFailed[$id] } else { $done += $id }
    }
    if ($bad.Count) { Write-Log ("Undo all: {0} tweak(s) restored, {1} NOT undone: {2}" -f $done.Count, $bad.Count, (@($bad.Keys) -join ', ')) 'WARN' }
    else { Write-Log "Undo all: $($done.Count) tweak(s) restored." 'OK' }
    return @{ Undone = $done; Failed = $bad }
}

function Invoke-TweakBatch {
    # Shared by the window and unattended mode. When a restore point was requested and could not be made, nothing is
    # applied and RestoreFailed is set; the caller decides whether to go on without one.
    param([string[]]$Ids, [ValidateSet('apply', 'undo')][string]$Mode, [bool]$RestorePoint)
    if ($Mode -eq 'apply' -and $RestorePoint) {
        if (-not (New-OptiRestorePoint 'Optimaxer - before tweaks')) {
            Write-Log 'No restore point, so nothing was changed.' 'WARN'
            return @{ RestoreFailed = $true; Results = @(); Restart = $false; Explorer = $false }
        }
    }
    $results = @(); $restart = $false; $explorer = $false
    foreach ($id in $Ids) {
        $t = Get-TweakById $id
        if (-not $t) { Write-Log "Unknown tweak id: $id" 'WARN'; continue }
        $on = if ($Mode -eq 'apply') { Invoke-TweakApply $t } else { Invoke-TweakUndo $t }
        if ($t.Restart) { $restart = $true }
        if ($t.Explorer) { $explorer = $true }
        $results += @{ Id = $id; On = [bool]$on; Ok = (($Mode -eq 'apply') -eq [bool]$on) -and -not $global:OptiUndoFailed.ContainsKey($id) }
    }
    return @{ RestoreFailed = $false; Results = $results; Restart = $restart; Explorer = $explorer }
}

function Invoke-TweakJobBody {
    # What the window's Apply/Undo job runs. Emits 'restore-failed' (and changes nothing) when the requested restore
    # point could not be made; otherwise one @{ Id; On } per tweak.
    param([string[]]$Ids, [string]$Mode, [bool]$RestorePoint)
    $r = Invoke-TweakBatch -Ids $Ids -Mode $Mode -RestorePoint $RestorePoint
    if ($r.RestoreFailed) { return 'restore-failed' }
    if ($r.Explorer) { Write-Log 'Restarting Explorer to apply interface changes'; Stop-Process -Name explorer -Force -ErrorAction SilentlyContinue }
    $ok = @($r.Results | Where-Object { $_.Ok }).Count
    $notUndone = @($r.Results | Where-Object { $Mode -eq 'undo' -and -not $_.Ok } | ForEach-Object Id)
    Write-Log "Finished: $ok of $(@($Ids).Count) tweak(s) verified." $(if ($ok -eq @($Ids).Count) { 'OK' } else { 'WARN' })
    if ($notUndone.Count) { Write-Log "Not undone: $($notUndone -join ', '). Their saved values are kept; see the log lines above." 'WARN' }
    if ($r.Restart) { Write-Log 'Restart Windows for all changes to take full effect.' 'OK' }
    foreach ($x in $r.Results) { @{ Id = $x.Id; On = $x.On } }
}

function Invoke-UnattendedTweaks {
    # -Silent: returns the process exit code. 3 = the restore point failed and nothing was applied.
    param([string[]]$Ids, [switch]$NoRestorePoint)
    $r = Invoke-TweakBatch -Ids $Ids -Mode apply -RestorePoint (-not $NoRestorePoint)
    if ($r.RestoreFailed) {
        Write-Log 'Restore point failed: nothing was applied. Fix System Restore, or run again with -NoRestorePoint.' 'ERROR'
        return 3
    }
    $ok = @($r.Results | Where-Object { $_.Ok }).Count
    $bad = @($r.Results).Count - $ok + @($Ids | Where-Object { -not (Get-TweakById $_) }).Count
    foreach ($x in @($r.Results | Where-Object { -not $_.Ok })) { Write-Log "  not verified: $($x.Id)" 'WARN' }
    Write-Log "Done: $ok tweak(s) verified, $bad not verified." $(if ($bad) { 'WARN' } else { 'OK' })
    return $(if ($bad) { 1 } else { 0 })
}

# ---------------------------------------------------------------- startup items
function Get-StartupItems {
    param([switch]$NoTasks)
    $apr = 'Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved'
    $defs = @(
        @{ Scope = 'Registry (user)';    Path = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run';              Approved = "HKCU:\$apr\Run" },
        @{ Scope = 'Registry (machine)'; Path = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Run';              Approved = "HKLM:\$apr\Run" },
        @{ Scope = 'Registry (32-bit)';  Path = 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Run'; Approved = "HKLM:\$apr\Run32" }
    )
    foreach ($d in $defs) {
        if (-not (Test-Path -LiteralPath $d.Path)) { continue }
        $props = Get-ItemProperty -LiteralPath $d.Path
        foreach ($p in $props.PSObject.Properties) {
            if ($p.Name -like 'PS*') { continue }
            @{ Name = $p.Name; Cmd = [string]$p.Value; Scope = $d.Scope; Approved = $d.Approved; Enabled = (Test-StartupEnabled $d.Approved $p.Name); Kind = 'Reg'; TaskPath = '' }
        }
    }
    $folders = @(
        @{ Scope = 'Startup folder (user)';    Path = [Environment]::GetFolderPath('Startup');       Approved = "HKCU:\$apr\StartupFolder" },
        @{ Scope = 'Startup folder (machine)'; Path = [Environment]::GetFolderPath('CommonStartup'); Approved = "HKLM:\$apr\StartupFolder" }
    )
    foreach ($f in $folders) {
        if (-not (Test-Path -LiteralPath $f.Path)) { continue }
        foreach ($i in Get-ChildItem -LiteralPath $f.Path -File -Force -ErrorAction SilentlyContinue) {
            if ($i.Name -eq 'desktop.ini') { continue }
            @{ Name = $i.Name; Cmd = $i.FullName; Scope = $f.Scope; Approved = $f.Approved; Enabled = (Test-StartupEnabled $f.Approved $i.Name); Kind = 'Reg'; TaskPath = '' }
        }
    }
    if (-not $NoTasks) {
        # third-party scheduled tasks that run at logon or boot (Microsoft's own tasks are left out)
        foreach ($t in Get-ScheduledTask -ErrorAction SilentlyContinue) {
            if ($t.TaskPath -like '\Microsoft\*') { continue }
            $hit = @($t.Triggers | Where-Object { $_.CimClass.CimClassName -in 'MSFT_TaskLogonTrigger', 'MSFT_TaskBootTrigger' })
            if (-not $hit.Count) { continue }
            $cmd = (@($t.Actions | ForEach-Object { ("$($_.Execute) $($_.Arguments)").Trim() }) -join '; ')
            @{ Name = $t.TaskName; Cmd = $cmd; Scope = 'Scheduled task (logon/boot)'; Approved = ''; Enabled = ($t.State -ne 'Disabled'); Kind = 'Task'; TaskPath = $t.TaskPath }
        }
    }
}

function Test-StartupEnabled {
    param([string]$ApprovedKey, [string]$Name)
    try {
        $v = (Get-ItemProperty -LiteralPath $ApprovedKey -Name $Name -ErrorAction Stop).$Name
        return -not ($v -and ($v[0] -band 1))
    } catch { return $true }
}

function Set-StartupItem {
    param($Item, [bool]$Enable)
    if ($Item.Kind -eq 'Task') {
        if ($Enable) { Enable-ScheduledTask -TaskPath $Item.TaskPath -TaskName $Item.Name -ErrorAction Stop | Out-Null }
        else { Disable-ScheduledTask -TaskPath $Item.TaskPath -TaskName $Item.Name -ErrorAction Stop | Out-Null }
        return
    }
    $bytes = New-Object byte[] 12
    if ($Enable) { $bytes[0] = 2 }
    else {
        $bytes[0] = 3
        [BitConverter]::GetBytes([DateTime]::UtcNow.ToFileTimeUtc()).CopyTo($bytes, 4)
    }
    if (-not (Test-Path -LiteralPath $Item.Approved)) { New-Item -Path $Item.Approved -Force | Out-Null }
    New-ItemProperty -LiteralPath $Item.Approved -Name $Item.Name -Value $bytes -PropertyType Binary -Force | Out-Null
}

# ---------------------------------------------------------------- DNS
function Set-DnsProvider {
    param([string]$Name, [string[]]$V4, [string[]]$V6, [string]$Doh)
    $adapters = @(Get-NetAdapter -ErrorAction SilentlyContinue | Where-Object { $_.Status -eq 'Up' -and $_.HardwareInterface })
    if (-not $adapters) { Write-Log 'No active network adapter found' 'WARN'; return }
    foreach ($a in $adapters) {
        try {
            if (-not $V4) { Set-DnsClientServerAddress -InterfaceIndex $a.ifIndex -ResetServerAddresses -ErrorAction Stop }
            else { Set-DnsClientServerAddress -InterfaceIndex $a.ifIndex -ServerAddresses ($V4 + $V6) -ErrorAction Stop }
            Write-Log "  DNS on $($a.Name): $(if($V4){$name}else{'automatic'})"
        } catch { Write-Log "  DNS on $($a.Name): $($_.Exception.Message)" 'WARN' }
    }
    if ($Doh -and (Get-Command Add-DnsClientDohServerAddress -ErrorAction SilentlyContinue)) {
        foreach ($ip in ($V4 + $V6)) {
            try { Add-DnsClientDohServerAddress -ServerAddress $ip -DohTemplate $Doh -AllowFallbackToUdp $true -AutoUpgrade $true -ErrorAction Stop | Out-Null } catch {}
        }
    }
    Clear-DnsClientCache
}

# ---------------------------------------------------------------- cleanup
function Get-DirSize {
    # Iterative .NET walk: much faster than Get-ChildItem -Recurse, skips junctions/symlinks, ignores unreadable folders.
    param([string]$Dir)
    $total = 0.0
    $stack = New-Object 'System.Collections.Generic.Stack[string]'
    $stack.Push($Dir)
    while ($stack.Count) {
        $d = New-Object IO.DirectoryInfo ($stack.Pop())
        try {
            foreach ($f in $d.EnumerateFiles()) { $total += $f.Length }
            foreach ($s in $d.EnumerateDirectories()) { if (-not ($s.Attributes -band [IO.FileAttributes]::ReparsePoint)) { $stack.Push($s.FullName) } }
        } catch {}
    }
    return $total
}

function Get-PathSize {
    param([string]$Path)
    $t = 0.0
    foreach ($i in Get-ChildItem -Path $Path -Force -ErrorAction SilentlyContinue) {
        if ($i.PSIsContainer) { if (-not ($i.Attributes -band [IO.FileAttributes]::ReparsePoint)) { $t += Get-DirSize $i.FullName } }
        else { $t += [double]$i.Length }
    }
    return $t
}

function Clear-PathContents {
    param([string]$Path)
    $freed = 0.0
    foreach ($i in Get-ChildItem -Path $Path -Force -ErrorAction SilentlyContinue) {
        $size = if ($i.PSIsContainer) { Get-PathSize $i.FullName } else { [double]$i.Length }
        try { Remove-Item -LiteralPath $i.FullName -Recurse -Force -ErrorAction Stop; $freed += $size }
        catch {
            if ($i.PSIsContainer) {
                # locked files inside: remove what can be removed
                foreach ($f in Get-ChildItem -LiteralPath $i.FullName -Recurse -Force -File -ErrorAction SilentlyContinue) {
                    try { $len = $f.Length; Remove-Item -LiteralPath $f.FullName -Force -ErrorAction Stop; $freed += $len } catch {}
                }
            }
        }
    }
    return $freed
}

function Get-CleanupSize {
    param($Target)
    if ($Target.Special -eq 'recycle') {
        $size = 0.0
        try { $sh = New-Object -ComObject Shell.Application; foreach ($i in $sh.Namespace(0xA).Items()) { $size += $i.Size } } catch {}
        return $size
    }
    if ($Target.Special -and $Target.Special -notin 'eventlogs', 'winold') { return -1.0 }
    $size = 0.0
    foreach ($p in $Target.Paths) { $size += Get-PathSize $p }
    return $size
}

function Invoke-CleanupTarget {
    param($Target)
    switch ($Target.Special) {
        'recycle'   { $before = Get-CleanupSize $Target; Clear-RecycleBin -Force -ErrorAction SilentlyContinue; return $before }
        'component' { Dism.exe /Online /Cleanup-Image /StartComponentCleanup | Out-Null; return 0.0 }
        'eventlogs' {
            $before = Get-CleanupSize $Target
            foreach ($log in (wevtutil.exe el)) { wevtutil.exe cl $log 2>&1 | Out-Null }
            return $before
        }
        'winold' {
            $dir = $Target.Paths[0]
            if (-not (Test-Path -LiteralPath $dir)) { return 0.0 }
            $before = Get-CleanupSize $Target
            takeown.exe /F $dir /R /A /D Y 2>&1 | Out-Null
            icacls.exe $dir /grant 'Administrators:F' /T /C /Q 2>&1 | Out-Null
            cmd.exe /c rd /s /q "$dir" 2>&1 | Out-Null
            return $(if (Test-Path -LiteralPath $dir) { 0.0 } else { $before })
        }
    }
    $wasRunning = @()
    if ($Target.Services) { foreach ($s in $Target.Services) { if ((Get-Service -Name $s -ErrorAction SilentlyContinue).Status -eq 'Running') { $wasRunning += $s }; Stop-Service -Name $s -Force -ErrorAction SilentlyContinue } }
    $freed = 0.0
    foreach ($p in $Target.Paths) { $freed += Clear-PathContents $p }
    foreach ($s in $wasRunning) { Start-Service -Name $s -ErrorAction SilentlyContinue }
    return $freed
}

# ---------------------------------------------------------------- registry key deletion with .reg backup
function Remove-RegKeysWithBackup {
    # Keys in native form (HKEY_CLASSES_ROOT\...). A key is only deleted after its export succeeded.
    param([string[]]$Keys, [string]$Tag)
    $files = @(); $i = 0
    foreach ($k in $Keys) {
        $p = "Registry::$k"
        if (-not (Test-Path -LiteralPath $p)) { continue }
        $i++
        $f = Join-Path $global:OptiBackupDir ('{0}-{1}-{2}.reg' -f $Tag, (Get-Date -Format 'yyyyMMdd-HHmmss'), $i)
        reg.exe export $k $f /y 2>&1 | Out-Null
        if (Test-Path -LiteralPath $f) { $files += $f; Remove-Item -LiteralPath $p -Recurse -Force -ErrorAction SilentlyContinue }
        else { Write-Log "  could not back up $k, left untouched" 'WARN' }
    }
    , $files
}

function Restore-RegFiles {
    param($Files)
    foreach ($f in @($Files)) { if ($f -and (Test-Path -LiteralPath $f)) { reg.exe import $f 2>&1 | Out-Null } }
}

function Test-RegKeysGone {
    param([string[]]$Keys)
    foreach ($k in $Keys) { if (Test-Path -LiteralPath "Registry::$k") { return $false } }
    return $true
}

# ---------------------------------------------------------------- hourly auto clean (temp files)
# A scheduled task runs a small script as SYSTEM every hour. The script is copied to Program Files, which only
# administrators can write, so a standard user cannot edit something that runs with SYSTEM rights.
$global:OptiAutoName = 'Auto Clean'
$global:OptiAutoPath = '\Optimaxer\'
$global:OptiAutoStatus = Join-Path $global:OptiData 'autoclean.json'

function Get-AutoCleanBody {
    @'
$ErrorActionPreference = 'SilentlyContinue'
$data = Join-Path $env:ProgramData 'Optimaxer'
New-Item -ItemType Directory -Path $data -Force | Out-Null
$minAge = (Get-Date).AddMinutes(-60)

function Clear-OldTemp {
    # Deletes files that have not been touched for an hour. Never follows junctions or symlinks.
    param([string]$Root)
    $freed = [int64]0
    if (-not (Test-Path -LiteralPath $Root)) { return $freed }
    $stack = New-Object System.Collections.Stack
    $dirs = New-Object System.Collections.ArrayList
    $stack.Push((New-Object IO.DirectoryInfo $Root))
    while ($stack.Count) {
        $d = $stack.Pop()
        try { $items = $d.GetFileSystemInfos() } catch { continue }
        foreach ($i in $items) {
            if ($i.Attributes -band [IO.FileAttributes]::ReparsePoint) { continue }
            if ($i -is [IO.DirectoryInfo]) { [void]$dirs.Add($i); $stack.Push($i) }
            elseif ($i.LastWriteTime -lt $minAge -and $i.CreationTime -lt $minAge) {
                $len = [int64]$i.Length
                if ($ReportOnly) { $freed += $len }
                else { try { $i.Delete(); $freed += $len } catch {} }
            }
        }
    }
    for ($k = $dirs.Count - 1; $k -ge 0; $k--) {
        $d = $dirs[$k]
        if ($d.LastWriteTime -lt $minAge -and -not $ReportOnly) { try { if (-not $d.GetFileSystemInfos().Count) { $d.Delete() } } catch {} }
    }
    return $freed
}

# Working sets and the standby list are left alone: standby memory is the file cache, and emptying it every hour
# only makes Windows read the same files from disk again.
$tempFreed = [int64]0
$roots = @("$env:SystemRoot\Temp")
foreach ($u in Get-ChildItem -LiteralPath (Join-Path $env:SystemDrive 'Users') -Directory -Force) {
    if ($u.Name -in 'Public', 'Default', 'Default User', 'All Users') { continue }
    $roots += (Join-Path $u.FullName 'AppData\Local\Temp')
}
foreach ($r in $roots) { $tempFreed += [int64](Clear-OldTemp $r) }

$rec = [pscustomobject]@{ Time = (Get-Date).ToString('s'); TempFreedBytes = $tempFreed; ReportOnly = [bool]$ReportOnly }
$rec | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $data 'autoclean.json') -Encoding UTF8
$line = '[{0}] INFO  Auto clean: temp files {1:N1} MB removed' -f (Get-Date -Format 'HH:mm:ss'), ($tempFreed / 1MB)
Add-Content -LiteralPath (Join-Path $data 'optimaxer.log') -Value $line -Encoding UTF8
'@
}

function Get-AutoCleanScript {
    # param() has to be the first statement. The hourly script does not touch memory, so no native helpers.
    return ("param([switch]`$ReportOnly)`r`n" + (Get-AutoCleanBody))
}

function Update-AutoCleanScript {
    # A task installed by v2.13.x still runs a script that empties the standby list. Rewrite it in place (same path,
    # same task) so the hourly run only deletes old temp files. Needs administrator rights; quietly does nothing without.
    $script = Join-Path $env:ProgramFiles 'Optimaxer\AutoClean.ps1'
    if (-not (Test-Path -LiteralPath $script)) { return }
    try {
        if ((Get-Content -LiteralPath $script -Raw -ErrorAction Stop) -match 'PurgeStandbyList|TrimWorkingSets') {
            Set-Content -LiteralPath $script -Value (Get-AutoCleanScript) -Encoding UTF8 -ErrorAction Stop
            Write-Log 'Auto clean updated: the hourly task now only removes old temp files.' 'OK'
        }
    } catch {}
}

function Test-AutoClean {
    $t = Get-ScheduledTask -TaskName $global:OptiAutoName -TaskPath $global:OptiAutoPath -ErrorAction SilentlyContinue
    return [bool]($t -and $t.State -ne 'Disabled')
}

function Get-AutoCleanStatus {
    # @{ Enabled; Next; Last = parsed autoclean.json or $null }
    $r = @{ Enabled = (Test-AutoClean); Next = $null; Last = $null }
    if ($r.Enabled) { $r.Next = (Get-ScheduledTaskInfo -TaskName $global:OptiAutoName -TaskPath $global:OptiAutoPath -ErrorAction SilentlyContinue).NextRunTime }
    if (Test-Path -LiteralPath $global:OptiAutoStatus) { try { $r.Last = Get-Content -LiteralPath $global:OptiAutoStatus -Raw | ConvertFrom-Json } catch {} }
    return $r
}

function Enable-AutoClean {
    $dir = Join-Path $env:ProgramFiles 'Optimaxer'
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    $script = Join-Path $dir 'AutoClean.ps1'
    Set-Content -LiteralPath $script -Value (Get-AutoCleanScript) -Encoding UTF8 -ErrorAction Stop

    $action = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument ('-NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File "{0}"' -f $script)
    $trigger = New-ScheduledTaskTrigger -Once -At (Get-Date).AddMinutes(1) -RepetitionInterval (New-TimeSpan -Hours 1) -RepetitionDuration (New-TimeSpan -Days 9999)
    $principal = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
    $settings = New-ScheduledTaskSettingsSet -StartWhenAvailable -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -MultipleInstances IgnoreNew -ExecutionTimeLimit (New-TimeSpan -Minutes 30)
    Register-ScheduledTask -TaskName $global:OptiAutoName -TaskPath $global:OptiAutoPath -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Description 'Optimaxer: removes temp files older than an hour, every hour.' -Force -ErrorAction Stop | Out-Null
    Start-ScheduledTask -TaskName $global:OptiAutoName -TaskPath $global:OptiAutoPath -ErrorAction SilentlyContinue
    Write-Log 'Auto clean on: temp files older than an hour are removed every hour (first run now).' 'OK'
}

function Disable-AutoClean {
    Unregister-ScheduledTask -TaskName $global:OptiAutoName -TaskPath $global:OptiAutoPath -Confirm:$false -ErrorAction SilentlyContinue
    $dir = Join-Path $env:ProgramFiles 'Optimaxer'
    Remove-Item -LiteralPath (Join-Path $dir 'AutoClean.ps1') -Force -ErrorAction SilentlyContinue
    if ((Test-Path -LiteralPath $dir) -and -not (Get-ChildItem -LiteralPath $dir -Force -ErrorAction SilentlyContinue)) { Remove-Item -LiteralPath $dir -Force -ErrorAction SilentlyContinue }
    Write-Log 'Auto clean off.' 'OK'
}

# ---------------------------------------------------------------- Appx removal (installed + provisioned)
function Remove-AppxByName {
    # Removes packages whose Name matches any pattern, for all users, then drops matching provisioned copies so new
    # accounts do not get them. One failing package no longer stops the rest. Returns @{ Removed; Failed }.
    param([string[]]$Pattern)
    $removed = @(); $failed = @()
    foreach ($pat in $Pattern) {
        foreach ($p in @(Get-AppxPackage -AllUsers -Name $pat -ErrorAction SilentlyContinue)) {
            if ($p.NonRemovable -or $p.IsFramework) { continue }
            try { Remove-AppxPackage -Package $p.PackageFullName -AllUsers -ErrorAction Stop; $removed += $p.Name }
            catch {
                try { Remove-AppxPackage -Package $p.PackageFullName -ErrorAction Stop; $removed += $p.Name }
                catch { $failed += ('{0}: {1}' -f $p.Name, $_.Exception.Message.Split("`n")[0].Trim()) }
            }
        }
        try {
            foreach ($pp in @(Get-AppxProvisionedPackage -Online -ErrorAction Stop | Where-Object { $_.DisplayName -like $pat })) {
                try { Remove-AppxProvisionedPackage -Online -PackageName $pp.PackageName -ErrorAction Stop | Out-Null; $removed += $pp.DisplayName }
                catch { $failed += ('{0} (provisioned): {1}' -f $pp.DisplayName, $_.Exception.Message.Split("`n")[0].Trim()) }
            }
        } catch { Write-Log "  could not read provisioned apps: $($_.Exception.Message)" 'WARN' }
    }
    return @{ Removed = @($removed | Select-Object -Unique); Failed = @($failed | Select-Object -Unique) }
}

# ---------------------------------------------------------------- OneDrive
function Get-OneDriveExePaths {
    @("$env:LOCALAPPDATA\Microsoft\OneDrive\OneDrive.exe", "$env:ProgramFiles\Microsoft OneDrive\OneDrive.exe", "${env:ProgramFiles(x86)}\Microsoft OneDrive\OneDrive.exe")
}

function Test-OneDriveInstalled {
    foreach ($e in Get-OneDriveExePaths) { if ($e -and (Test-Path -LiteralPath $e)) { return $true } }
    return $false
}

function Get-OneDriveRedirectedFolders {
    # Known folders (Desktop, Documents, Pictures...) that Windows currently keeps inside a OneDrive folder.
    # Uninstalling while these point there leaves online-only files unreachable, so callers refuse.
    $hits = @()
    $k = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders'
    foreach ($n in 'Desktop', 'Personal', 'My Pictures', 'My Music', 'My Video', '{374DE290-123F-4565-9164-39C4925E467B}') {
        $v = (Get-ItemProperty -LiteralPath $k -Name $n -ErrorAction SilentlyContinue).$n
        if ($v) { $x = [Environment]::ExpandEnvironmentVariables($v); if ($x -match '\\OneDrive[^\\]*(\\|$)') { $hits += $x } }
    }
    , $hits
}

function Stop-OneDriveProcesses {
    foreach ($n in 'OneDrive', 'OneDriveStandaloneUpdater', 'FileCoAuth', 'FileSyncHelper') { Stop-Process -Name $n -Force -ErrorAction SilentlyContinue }
}

function Uninstall-OneDrive {
    # Runs every uninstaller that exists (per-user setup, per-machine setup), then removes the leftovers that make it
    # come back: scheduled update tasks, the Run entries for new users, and the Explorer sidebar entry.
    # Returns @{ Tasks = full names of the OneDrive tasks that were enabled and are now disabled }.
    Stop-OneDriveProcesses
    # WinUtil's safeguard: while the uninstaller runs, deny Administrators the right to delete the OneDrive folders,
    # so a leftover-cleanup step can never take the user's files. Skipped for a folder that already has a deny entry.
    $guarded = @()
    foreach ($v in 'OneDrive', 'OneDriveConsumer', 'OneDriveCommercial') {
        $f = [Environment]::GetEnvironmentVariable($v)
        if ($f -and (Test-Path -LiteralPath $f) -and ($guarded -notcontains $f)) {
            $acl = (& icacls.exe $f 2>&1 | Out-String)
            if ($acl -notmatch '\(DENY\)') { & icacls.exe $f /deny '*S-1-5-32-544:(D,DC)' 2>&1 | Out-Null; $guarded += $f }
        }
    }
    try {
    $setups = @()
    $sys = "$env:SystemRoot\SysWOW64\OneDriveSetup.exe"; if (-not (Test-Path -LiteralPath $sys)) { $sys = "$env:SystemRoot\System32\OneDriveSetup.exe" }
    if (Test-Path -LiteralPath $sys) { $setups += @{ Exe = $sys; Args = '/uninstall' } }
    foreach ($root in @($env:ProgramFiles, ${env:ProgramFiles(x86)})) {
        if (-not $root) { continue }
        foreach ($f in @(Get-ChildItem -Path (Join-Path $root 'Microsoft OneDrive\*\Installer\setup.exe') -ErrorAction SilentlyContinue)) { $setups += @{ Exe = $f.FullName; Args = '/uninstall /allusers' } }
    }
    if (-not $setups.Count) { Write-Log '  no OneDrive uninstaller found (already removed?)' 'WARN' }
    foreach ($s in $setups) {
        Write-Log "  running $($s.Exe) $($s.Args)"
        try {
            $proc = Start-Process -FilePath $s.Exe -ArgumentList $s.Args -PassThru -WindowStyle Hidden -ErrorAction Stop
            if (-not $proc.WaitForExit(180000)) { Write-Log '  uninstaller still running after 3 minutes, continuing' 'WARN' }
        } catch { Write-Log "  uninstaller failed: $($_.Exception.Message)" 'WARN' }
    }
    Stop-OneDriveProcesses

    $tasks = @()
    foreach ($t in @(Get-ScheduledTask -TaskName 'OneDrive*' -ErrorAction SilentlyContinue)) {
        if ($t.State -ne 'Disabled') { $tasks += ($t.TaskPath + $t.TaskName); Disable-ScheduledTask -TaskPath $t.TaskPath -TaskName $t.TaskName -ErrorAction SilentlyContinue | Out-Null }
    }

    Remove-ItemProperty -LiteralPath 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'OneDrive' -ErrorAction SilentlyContinue
    # New accounts get OneDrive from the Default user's Run key; remove it from that hive.
    $def = Join-Path $env:SystemDrive 'Users\Default\NTUSER.DAT'
    if (Test-Path -LiteralPath $def) {
        $null = reg.exe load 'HKU\OptiDefault' $def 2>&1
        if ($LASTEXITCODE -eq 0) {
            $null = reg.exe delete 'HKU\OptiDefault\Software\Microsoft\Windows\CurrentVersion\Run' /v OneDriveSetup /f 2>&1
            [GC]::Collect(); [GC]::WaitForPendingFinalizers()
            $null = reg.exe unload 'HKU\OptiDefault' 2>&1
        }
    }
    foreach ($d in "$env:LOCALAPPDATA\Microsoft\OneDrive", "$env:ProgramData\Microsoft OneDrive") {
        if (Test-Path -LiteralPath $d) { Remove-Item -LiteralPath $d -Recurse -Force -ErrorAction SilentlyContinue }
    }
    # The user's own OneDrive folders hold their files: only an empty one is removed.
    foreach ($d in @(Get-ChildItem -LiteralPath $env:USERPROFILE -Directory -Filter 'OneDrive*' -ErrorAction SilentlyContinue)) {
        if (-not (Get-ChildItem -LiteralPath $d.FullName -Force -ErrorAction SilentlyContinue)) { Remove-Item -LiteralPath $d.FullName -Force -ErrorAction SilentlyContinue }
    }
    @{ Tasks = @($tasks) }
    } finally {
        foreach ($f in $guarded) { & icacls.exe $f /remove:d '*S-1-5-32-544' 2>&1 | Out-Null }
    }
}

function Install-OneDrive {
    $sys = "$env:SystemRoot\SysWOW64\OneDriveSetup.exe"; if (-not (Test-Path -LiteralPath $sys)) { $sys = "$env:SystemRoot\System32\OneDriveSetup.exe" }
    if (Test-Path -LiteralPath $sys) {
        $proc = Start-Process -FilePath $sys -PassThru -ErrorAction SilentlyContinue
        if ($proc) { [void]$proc.WaitForExit(180000) }
    }
    if (-not (Test-OneDriveInstalled)) {
        Write-Log '  OneDrive setup not found, installing through WinGet'
        Invoke-WingetPackages -Action Install -Programs @('Microsoft.OneDrive')
    }
}

# ---------------------------------------------------------------- WinGet runner (same flow and exit-code handling as WinUtil's installer)
function Invoke-WingetPackages {
    # One winget process per package, real exit code, per-package outcome, and a verification step.
    param([ValidateSet('Install', 'Uninstall', 'Upgrade')][string]$Action, [string[]]$Programs)

    $adminContextProhibited = -1978335107   # per-user package touched from an elevated process
    $nothingToDo = @{ -1978335135 = 'already installed'; -1978335189 = 'no applicable update' }
    if ($Action -eq 'Uninstall') { $nothingToDo[-1978335212] = 'not installed' }
    $rebootCodes = @{ 3010 = 'installed, a restart is needed to finish'; 1641 = 'installed, the installer started a restart'
                      -1978334967 = 'installed, a restart is needed to finish'; -1978334965 = 'installed, the installer started a restart' }

    $total = @($Programs).Count; $n = 0
    $ok = 0; $skipped = 0; $failed = @()
    foreach ($program in $Programs) {
        if ([string]::IsNullOrWhiteSpace($program)) { continue }
        $n++
        $upgradeAll = $Action -eq 'Upgrade' -and $program -eq 'all'
        Write-Log "[$n/$total] $Action $program"

        $arguments = switch ($Action) {
            'Uninstall' { @('uninstall', '--id', $program, '--source', 'winget', '--silent') }
            'Upgrade'   { if ($upgradeAll) { @('upgrade', '--all', '--accept-package-agreements', '--accept-source-agreements', '--include-unknown', '--silent') }
                          else { @('upgrade', '--id', $program, '--accept-package-agreements', '--accept-source-agreements', '--source', 'winget', '--include-unknown', '--silent') } }
            default     { @('install', '--id', $program, '--accept-package-agreements', '--accept-source-agreements', '--source', 'winget', '--silent') }
        }

        $outFile = [IO.Path]::GetTempFileName(); $errFile = [IO.Path]::GetTempFileName()
        $sw = [Diagnostics.Stopwatch]::StartNew()
        try {
            $proc = Start-Process -FilePath winget -ArgumentList $arguments -NoNewWindow -Wait -PassThru -RedirectStandardOutput $outFile -RedirectStandardError $errFile -ErrorAction Stop
            $exit = $proc.ExitCode
        } catch {
            $exit = -1
            Write-Log "    could not start winget: $($_.Exception.Message)" 'ERROR'
        }
        $tail = (Get-Content -LiteralPath $outFile, $errFile -ErrorAction SilentlyContinue | Where-Object { $_ -and $_.Trim() -and $_ -notmatch '^[\s\-\\|/]+$' } | Select-Object -Last 2) -join ' | '
        Remove-Item -LiteralPath $outFile, $errFile -Force -ErrorAction SilentlyContinue

        if ($exit -eq 0 -or $rebootCodes.ContainsKey($exit)) {
            $detail = if ($exit -eq 0) { 'exit code 0' } else { $rebootCodes[$exit] }
            # do not trust the exit code alone: confirm the package state changed
            if (-not $upgradeAll) {
                $null = winget list --id $program --exact --accept-source-agreements 2>&1
                $listed = ($LASTEXITCODE -eq 0)
                if ($Action -eq 'Install' -and -not $listed) { Write-Log ("    winget reported success ({0}) but {1} is not listed as installed" -f $detail, $program) 'WARN'; $failed += $program; continue }
                if ($Action -eq 'Uninstall' -and $listed) { Write-Log "    winget reported success but $program is still listed" 'WARN'; $failed += $program; continue }
            }
            $ok++
            Write-Log ("    {0} ok in {1:N0}s ({2}), verified" -f $Action.ToLower(), $sw.Elapsed.TotalSeconds, $detail) 'OK'
        }
        elseif ($nothingToDo.ContainsKey($exit)) { $skipped++; Write-Log "    skipped: $($nothingToDo[$exit])" }
        elseif ($exit -eq $adminContextProhibited) { $skipped++; Write-Log '    skipped: installed for the current user only; an elevated session cannot change it' 'WARN' }
        else {
            $failed += $program
            Write-Log ("    FAILED (WinGet 0x{0:X8}, exit {1}) {2}" -f $exit, $exit, $tail) 'WARN'
        }
    }
    Write-Log ("{0} finished: {1} ok, {2} skipped, {3} failed{4}" -f $Action, $ok, $skipped, $failed.Count, $(if ($failed.Count) { ' (' + ($failed -join ', ') + ')' } else { '' })) $(if ($failed.Count) { 'WARN' } else { 'OK' })
}

function Repair-WingetClient {
    # Same repair path WinUtil uses when winget is missing or broken.
    Install-PackageProvider -Name NuGet -Force | Out-Null
    Install-Module -Name Microsoft.WinGet.Client -Force -Repository PSGallery
    Import-Module Microsoft.WinGet.Client
    Repair-WinGetPackageManager -AllUsers
}

# ---------------------------------------------------------------- self-update (GitHub releases)
$global:OptiRepo = 'bliper2/optimaxer'

function Get-OptiVersion {
    try { return [version]((Get-Content -LiteralPath (Join-Path $global:OptiRoot 'VERSION') -Raw -ErrorAction Stop).Trim()) } catch { return [version]'0.0.0' }
}

function Get-OptiGitHubHeaders {
    $h = @{ 'User-Agent' = 'Optimaxer-Updater'; 'Accept' = 'application/vnd.github+json' }
    if ($env:OPTIMAXER_TOKEN) { $h['Authorization'] = "Bearer $env:OPTIMAXER_TOKEN" }   # optional, only needed while the repo is private
    return $h
}

function Get-LatestRelease {
    # Returns @{Version; Tag; Notes; Url; Digest; Name; Newer} or @{Error}
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    try {
        $r = Invoke-RestMethod -Uri "https://api.github.com/repos/$global:OptiRepo/releases/latest" -Headers (Get-OptiGitHubHeaders) -TimeoutSec 12 -ErrorAction Stop
    } catch {
        return @{ Error = "could not reach GitHub releases ($($_.Exception.Message))" }
    }
    $asset = @($r.assets | Where-Object { $_.name -like 'Optimaxer-v*.zip' })[0]
    if (-not $asset) { return @{ Error = 'latest release has no Optimaxer zip' } }
    try { $v = [version]($r.tag_name.TrimStart('v', 'V')) } catch { return @{ Error = "unreadable version tag $($r.tag_name)" } }
    $url = if ($env:OPTIMAXER_TOKEN) { $asset.url } else { $asset.browser_download_url }
    return @{ Version = $v.ToString(); Tag = $r.tag_name; Notes = [string]$r.body; Url = $url; Digest = [string]$asset.digest; Name = $asset.name; Newer = ($v -gt (Get-OptiVersion)) }
}

function Install-OptiUpdate {
    # Downloads and verifies the release zip, stages it, and starts a helper that swaps the files once this process exits.
    # Returns $true when the helper is running and the caller should exit.
    param([string]$Url, [string]$Digest, [string]$Version, [string]$InstallDir = $global:OptiRoot, [int]$WaitPid = $PID, [switch]$NoRelaunch)
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    $work = Join-Path ([IO.Path]::GetTempPath()) ('optimaxer-update-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
    New-Item -ItemType Directory -Path $work -Force | Out-Null
    $zip = Join-Path $work 'update.zip'
    $headers = Get-OptiGitHubHeaders
    if ($Url -like 'https://api.github.com/*') { $headers['Accept'] = 'application/octet-stream' }
    Write-Log "Downloading Optimaxer v$Version ..."
    try { Invoke-WebRequest -Uri $Url -Headers $headers -OutFile $zip -UseBasicParsing -TimeoutSec 120 -ErrorAction Stop } catch { Write-Log "Download failed: $($_.Exception.Message)" 'ERROR'; return $false }
    if ($Digest -match '^sha256:(?<h>[0-9a-fA-F]{64})$') {
        $got = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash
        if ($got -ne $Matches['h']) { Write-Log "Update rejected: checksum mismatch (expected $($Matches['h']), got $got)" 'ERROR'; return $false }
        Write-Log 'Checksum verified (SHA-256).' 'OK'
    } else { Write-Log 'Release has no published checksum; relying on HTTPS only.' 'WARN' }
    $stage = Join-Path $work 'stage'
    try { Expand-Archive -LiteralPath $zip -DestinationPath $stage -Force -ErrorAction Stop } catch { Write-Log "Could not unpack update: $($_.Exception.Message)" 'ERROR'; return $false }
    foreach ($need in 'Optimaxer.ps1', 'src\Core.ps1', 'src\App.ps1', 'VERSION') {
        if (-not (Test-Path -LiteralPath (Join-Path $stage $need))) { Write-Log "Update rejected: package is missing $need" 'ERROR'; return $false }
    }
    $backup = Join-Path $global:OptiBackupDir ('app-' + (Get-OptiVersion).ToString() + '-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
    $helper = Join-Path $work 'apply.ps1'
    @'
param($WaitPid, $Stage, $Dst, $Backup, $Relaunch)
try { Wait-Process -Id $WaitPid -Timeout 90 -ErrorAction SilentlyContinue } catch {}
Start-Sleep -Milliseconds 400
New-Item -ItemType Directory -Path $Backup -Force | Out-Null
Copy-Item -Path (Join-Path $Dst '*') -Destination $Backup -Recurse -Force -ErrorAction SilentlyContinue
Copy-Item -Path (Join-Path $Stage '*') -Destination $Dst -Recurse -Force
if ($Relaunch -eq '1') {
    Start-Process powershell.exe -ArgumentList ('-NoProfile -ExecutionPolicy Bypass -STA -WindowStyle Hidden -File "{0}"' -f (Join-Path $Dst 'Optimaxer.ps1'))
}
'@ | Set-Content -LiteralPath $helper -Encoding UTF8
    $helperArgs = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-WindowStyle', 'Hidden', '-File', $helper, '-WaitPid', $WaitPid, '-Stage', $stage, '-Dst', $InstallDir, '-Backup', $backup, '-Relaunch', $(if ($NoRelaunch) { '0' } else { '1' }))
    $p = Start-Process -FilePath powershell.exe -ArgumentList ($helperArgs | ForEach-Object { if ($_ -match '\s') { '"' + $_ + '"' } else { $_ } }) -WindowStyle Hidden -PassThru
    Write-Log "Update staged; Optimaxer will restart as v$Version. Previous files are backed up in $backup" 'OK'
    return $true
}

# ---------------------------------------------------------------- hosts file block (marker-delimited, reversible)
$global:OptiHostsBegin = '# BEGIN Optimaxer telemetry block'
$global:OptiHostsEnd   = '# END Optimaxer telemetry block'

function Remove-HostsBlock {
    param([string]$Path)
    $text = [IO.File]::ReadAllText($Path)
    $text = [regex]::Replace($text, '(?s)\r?\n?# BEGIN Optimaxer telemetry block.*?# END Optimaxer telemetry block\r?\n?', "`r`n")
    [IO.File]::WriteAllText($Path, $text.TrimEnd() + "`r`n", [Text.Encoding]::ASCII)
}

function Add-HostsBlock {
    param([string]$Path, [string[]]$Entries)
    Remove-HostsBlock -Path $Path   # never stack blocks
    $block = "$global:OptiHostsBegin (WindowsSpyBlocker, MIT)`r`n" + (($Entries | ForEach-Object { "0.0.0.0 $_" }) -join "`r`n") + "`r`n$global:OptiHostsEnd`r`n"
    [IO.File]::WriteAllText($Path, ([IO.File]::ReadAllText($Path)).TrimEnd() + "`r`n" + $block, [Text.Encoding]::ASCII)
}

# ---------------------------------------------------------------- presets (shared by the GUI buttons and unattended mode)
function Get-SystemTraits {
    # Hardware profile used to pick tweaks. Tags: laptop|desktop, ssd|hdd, nvme, nvidia|amdgpu|intelgpu, dgpu, win11|win10,
    # ram8 (8 GB or less), ram16 (16 GB or more), ram32, cores8 (8+ logical processors), lowend (weak CPU or little RAM).
    $laptop = [bool](Get-CimInstance Win32_Battery -ErrorAction SilentlyContinue)
    $ssd = $true; $nvme = $false
    try {
        $dn = (Get-Partition -DriveLetter $env:SystemDrive[0] -ErrorAction Stop).DiskNumber
        $pd = Get-PhysicalDisk | Where-Object { $_.DeviceId -eq "$dn" } | Select-Object -First 1
        if ($pd.MediaType -eq 'HDD') { $ssd = $false }
        if ($pd.BusType -eq 'NVMe') { $nvme = $true }
    } catch {}
    $ram = 0.0; $threads = 0
    try { $ram = [math]::Round((Get-CimInstance Win32_ComputerSystem -ErrorAction Stop).TotalPhysicalMemory / 1GB, 1) } catch {}
    try { $threads = [int](Get-CimInstance Win32_Processor -ErrorAction Stop | Measure-Object NumberOfLogicalProcessors -Sum).Sum } catch {}
    $gpus = @()
    try { $gpus = @(Get-CimInstance Win32_VideoController -ErrorAction Stop | ForEach-Object { [string]$_.Name } | Where-Object { $_ }) } catch {}
    $nv = [bool]($gpus -match 'NVIDIA')
    $amd = [bool]($gpus -match 'AMD|Radeon')
    # an AMD "Radeon Graphics" / Vega iGPU is not a discrete card
    $amdDiscrete = [bool]($gpus -match 'Radeon\s+(RX|Pro|R[0-9])|Radeon\s+[0-9]{3,4}\b|Arc\s+[AB][0-9]')
    $dgpu = $nv -or $amdDiscrete
    $build = 0; try { $build = [int](Get-CimInstance Win32_OperatingSystem -ErrorAction Stop).BuildNumber } catch {}
    $tags = New-Object System.Collections.ArrayList
    [void]$tags.Add($(if ($laptop) { 'laptop' } else { 'desktop' }))
    [void]$tags.Add($(if ($ssd) { 'ssd' } else { 'hdd' }))
    if ($nvme) { [void]$tags.Add('nvme') }
    if ($nv) { [void]$tags.Add('nvidia') }
    if ($amd) { [void]$tags.Add('amdgpu') }
    if ($gpus -match 'Intel') { [void]$tags.Add('intelgpu') }
    if ($dgpu) { [void]$tags.Add('dgpu') }
    [void]$tags.Add($(if ($build -ge 22000) { 'win11' } else { 'win10' }))
    if ($ram -gt 0 -and $ram -le 8.5) { [void]$tags.Add('ram8') }
    if ($ram -ge 15.5) { [void]$tags.Add('ram16') }
    if ($ram -ge 31.5) { [void]$tags.Add('ram32') }
    if ($threads -ge 8) { [void]$tags.Add('cores8') }
    if (($ram -gt 0 -and $ram -le 6) -or ($threads -gt 0 -and $threads -le 2) -or -not $ssd) { [void]$tags.Add('lowend') }
    return @{ Laptop = $laptop; SSD = $ssd; Vendor = Get-SystemVendor; RamGB = $ram; Threads = $threads; Gpus = $gpus; Build = $build; Tags = @($tags) }
}

function Test-TweakFits {
    # True when a tweak makes sense on this hardware. -Need lists traits that must all be present; a leading '!' means absent.
    param($T, $Traits)
    if ($T.DesktopOnly -and $Traits.Laptop) { return $false }
    if ($T.SSDOnly -and -not $Traits.SSD) { return $false }
    if ($T.Vendor -and $T.Vendor -ne $Traits.Vendor) { return $false }
    if ($T.MinRamGB -and $Traits.RamGB -and $Traits.RamGB -lt $T.MinRamGB) { return $false }
    foreach ($n in @($T.Need)) {
        if (-not $n) { continue }
        if ($n.StartsWith('!')) { if (@($Traits.Tags) -contains $n.Substring(1)) { return $false } }
        elseif (@($Traits.Tags) -notcontains $n) { return $false }
    }
    return $true
}

# Normalizes the board/system manufacturer string Windows reports to one of a few known brands, or ''.
# Checked for informational display and to gate vendor-tagged catalog entries; never used to pick or fetch
# a driver automatically (no network calls, no downloads) - that stays a manual, user-confirmed step.
function Get-SystemVendor {
    $names = @()
    try { $names += (Get-CimInstance Win32_ComputerSystem -ErrorAction Stop).Manufacturer } catch {}
    try { $names += (Get-CimInstance Win32_BaseBoard -ErrorAction Stop).Manufacturer } catch {}
    $names = @($names | Where-Object { $_ }) -join ' '
    if ($names -match 'MSI|Micro-Star') { return 'MSI' }
    if ($names -match 'Acer') { return 'Acer' }
    if ($names -match 'Gigabyte|Giga-Byte') { return 'Gigabyte' }
    return ''
}

# URL of each detected vendor's own driver/support center. Opened in the default browser on request only;
# Optimaxer never downloads or installs a driver itself.
$global:OptiVendorDriverUrl = @{
    MSI      = 'https://www.msi.com/support/download'
    Acer     = 'https://www.acer.com/us-en/support/drivers-and-manuals'
    Gigabyte = 'https://www.gigabyte.com/Support'
}

function Resolve-ConflictIds {
    # Keeps the first id of each conflict group (search style, drive letters, ...) so a preset never ticks two
    # tweaks that write the same setting.
    param([string[]]$Ids)
    $seen = @{}
    foreach ($id in $Ids) {
        $t = Get-TweakById $id
        $g = if ($t) { $t.Conflict } else { '' }
        if ($g) { if ($seen.ContainsKey($g)) { continue }; $seen[$g] = $true }
        $id
    }
}

function Get-PresetTweakIds {
    param([string]$Preset, $Traits)
    if ($Preset -eq 'auto') { return @((Get-SmartPlan $Traits).Ids) }
    $ids = foreach ($t in $global:OptiCatalog) {
        if ($t.Tags -contains 'optin' -or $t.Retired) { continue }
        if (-not (Test-TweakFits $t $Traits)) { continue }
        if ($t.Tags -contains $Preset) { $t.Id }
    }
    @(Resolve-ConflictIds @($ids))
}

function Test-AutoPick {
    # The one rule every automatic plan uses (Smart optimize, -Preset auto, the desktop app). Never: opt-in, retired or
    # Advanced. Yes: the performance set (tag 'perf', Safe or Moderate; its hardware gate is checked by the caller),
    # and Safe tweaks from the recommended set or hardware-gated 'smart' tweaks.
    param($T)
    if ($T.Tags -contains 'optin' -or $T.Retired -or $T.Risk -eq 'Advanced') { return $false }
    if ($T.Tags -contains 'perf') { return $true }
    return ($T.Risk -eq 'Safe') -and (($T.Tags -contains 'safe') -or ($T.Tags -contains 'smart'))
}

function Get-TweakFitReason {
    # Plain-language hardware fact that rules a tweak out on this PC, or '' when it fits.
    param($T, $Traits)
    if ($T.DesktopOnly -and $Traits.Laptop) { return 'this PC is a laptop' }
    if ($T.SSDOnly -and -not $Traits.SSD) { return 'the system drive is a hard disk' }
    if ($T.Vendor -and $T.Vendor -ne $Traits.Vendor) { return "not a $($T.Vendor) system" }
    if ($T.MinRamGB -and $Traits.RamGB -and $Traits.RamGB -lt $T.MinRamGB) { return ('{0:N0} GB RAM, needs {1:N0} GB' -f $Traits.RamGB, $T.MinRamGB) }
    $names = @{ dgpu = 'a discrete GPU'; hdd = 'a hard disk'; ssd = 'an SSD'; laptop = 'a laptop'; desktop = 'a desktop'; lowend = 'low-end hardware'; nvidia = 'an NVIDIA GPU'; win11 = 'Windows 11' }
    foreach ($n in @($T.Need)) {
        if (-not $n) { continue }
        if ($n.StartsWith('!')) { $k = $n.Substring(1); if (@($Traits.Tags) -contains $k) { return "not for $(if ($names[$k]) { $names[$k] } else { $k })" } }
        elseif (@($Traits.Tags) -notcontains $n) { return "needs $(if ($names[$n]) { $names[$n] } else { $n })" }
    }
    return ''
}

function Get-SmartPlan {
    # Picks tweaks for THIS machine with Test-AutoPick: the performance set where its hardware gate fits, Safe
    # recommended tweaks and Safe 'smart' tweaks. Advanced, opt-in, retired and unfit tweaks are never ticked; other
    # Moderate tweaks are left for the user.
    # Returns @{ Ids; Notes; Skipped = @{ id = reason } } - Notes and Skipped explain the choices in plain language.
    param($Traits, $Applied)
    $ids = @(); $notes = New-Object System.Collections.ArrayList; $skipped = @{}
    foreach ($t in $global:OptiCatalog) {
        $why = Get-TweakFitReason $t $Traits
        $tagged = ($t.Tags -contains 'safe') -or ($t.Tags -contains 'smart') -or ($t.Tags -contains 'gaming') -or ($t.Tags -contains 'perf')
        if ($t.Tags -contains 'optin' -or $t.Retired) { continue }
        if ($t.Risk -ne 'Safe' -and -not (Test-AutoPick $t)) {
            # every Moderate or Advanced tweak gets a reason, so the Tweaks page can say why it stayed unticked
            $skipped[$t.Id] = $(if ($why) { "$($t.Risk) risk, and $why" } else { "$($t.Risk) risk: left for you to tick" })
            continue
        }
        if ($why) { if ($tagged) { $skipped[$t.Id] = "does not fit: $why" }; continue }
        if (-not (Test-AutoPick $t)) { continue }
        if ($Applied -and $Applied.ContainsKey($t.Id) -and $Applied[$t.Id]) { continue }
        $ids += $t.Id
    }
    $ids = @(Resolve-ConflictIds $ids)
    $tg = @($Traits.Tags)
    [void]$notes.Add('Ticked: the recommended set and the performance tweaks that fit this hardware. Advanced tweaks and other Moderate ones are left for you to tick.')
    [void]$notes.Add($(if ($tg -contains 'laptop') { 'Laptop: desktop-only tweaks (power throttling, USB/PCIe power saving, timer) are skipped to protect battery life.' } else { 'Desktop: CPU power throttling off, no USB/PCIe link sleep, per-app timer requests honored. The Ultimate plan and 100% minimum CPU stay off (heat, no measured gain).' }))
    [void]$notes.Add($(if ($tg -contains 'ssd') { 'SSD: TRIM check included.' } else { 'Hard disk: SSD-only tweaks are skipped; run Defragment in Tools.' }))
    if ($Traits.RamGB) { [void]$notes.Add(('{0:N0} GB RAM: tweaks that need more memory than this are skipped.' -f $Traits.RamGB)) }
    [void]$notes.Add($(if ($tg -contains 'dgpu') { 'Discrete GPU: hardware-accelerated GPU scheduling, Game Mode and Game DVR capture off are ticked.' } else { 'No discrete GPU: GPU scheduling is skipped.' }))
    return @{ Ids = @($ids); Notes = @($notes); Skipped = $skipped }
}

# ---------------------------------------------------------------- desktop shortcut (optionally "Run as administrator")
function New-OptiShortcut {
    param([string]$Path, [string]$Target, [string]$Arguments, [string]$WorkDir, [string]$Icon, [switch]$RunAsAdmin)
    $sh = New-Object -ComObject WScript.Shell
    $lnk = $sh.CreateShortcut($Path)
    $lnk.TargetPath = $Target; $lnk.Arguments = $Arguments; $lnk.WorkingDirectory = $WorkDir
    $lnk.WindowStyle = 7
    if ($Icon -and (Test-Path -LiteralPath $Icon)) { $lnk.IconLocation = $Icon }
    $lnk.Description = 'Optimaxer'
    $lnk.Save()
    if ($RunAsAdmin) {
        $bytes = [IO.File]::ReadAllBytes($Path)
        $bytes[0x15] = $bytes[0x15] -bor 0x20      # SLDF_RUNAS_USER flag
        [IO.File]::WriteAllBytes($Path, $bytes)
    }
}


# ---------------------------------------------------------------- tools with their own undo records
. (Join-Path $PSScriptRoot 'Features.ps1')
