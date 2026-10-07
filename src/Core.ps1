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
        } catch { Write-Log "State file unreadable: $($_.Exception.Message)" 'WARN' }
    }
    return $h
}

function Save-OptiState {
    param($State)
    $State | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $global:OptiStatePath -Encoding UTF8
}

# ---------------------------------------------------------------- registry
function Get-RegSnapshot {
    param($Entry)
    $exists = $false; $val = $null; $type = $null
    try {
        $key = Get-Item -LiteralPath $Entry.P -ErrorAction Stop
        if ($key.GetValueNames() -contains $Entry.N) {
            $exists = $true
            $val  = $key.GetValue($Entry.N, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
            $type = $key.GetValueKind($Entry.N).ToString()
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
    param($Rec)
    try {
        if ($Rec.Exists) { Set-RegEntry $Rec.P $Rec.N $Rec.Value $Rec.Type }
        else             { Remove-RegEntry $Rec.P $Rec.N }
    } catch { Write-Log "  restore failed $($Rec.P)\$($Rec.N): $($_.Exception.Message)" 'WARN' }
}

# ---------------------------------------------------------------- services
function Get-SvcSnapshot {
    param([string]$Name)
    $k = "HKLM:\SYSTEM\CurrentControlSet\Services\$Name"
    if (-not (Test-Path -LiteralPath $k)) { return $null }
    $p = Get-ItemProperty -LiteralPath $k
    if ($null -eq $p.Start) { return $null }
    return @{ Name = $Name; Start = [int]$p.Start; Delayed = [int]($p.DelayedAutostart -eq 1) }
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
    } catch { Write-Log "  task $Full : $($_.Exception.Message)" 'WARN' }
}

# ---------------------------------------------------------------- power (powercfg output is parsed language-independently)
function Get-ActiveSchemeGuid {
    $o = powercfg /getactivescheme | Out-String
    if ($o -match '([0-9a-fA-F]{8}(-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12})') { return $Matches[1] }
}

function Get-SchemeName {
    param([string]$Guid)
    $o = powercfg /list | Out-String
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
    $o = powercfg /q SCHEME_CURRENT $Sub $Setting | Out-String
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
    if (-not $checked) { return $null -ne (Get-OptiState)[$T.Id] }
    return $true
}

function Get-TweakStatus {
    # Returns one @{Id; On} per catalog entry. Slow-ish (spawns powercfg etc.), run in a job.
    param([string[]]$Ids)
    Clear-TaskCache
    foreach ($t in $global:OptiCatalog) {
        if ($Ids -and $Ids -notcontains $t.Id) { continue }
        @{ Id = $t.Id; On = [bool](Test-Tweak $t) }
    }
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

function Invoke-TweakApply {
    param($T)
    Write-Log "Apply: $($T.Name)"
    Clear-TaskCache
    $state = Get-OptiState
    $fresh = -not ($state.ContainsKey($T.Id) -and (Test-Tweak $T))
    $rec = if ($fresh) { @{ Reg = @(); Svc = @(); Task = @(); Data = $null; Time = (Get-Date).ToString('s') } } else { $state[$T.Id] }

    foreach ($r in $T.Reg) {
        try {
            if ($fresh) {
                $shared = Find-SharedOriginal $state $T.Id 'Reg' $r.P $r.N
                if ($shared) { $rec.Reg += @{ P = $r.P; N = $r.N; Exists = [bool]$shared.Exists; Value = $shared.Value; Type = $shared.Type } }
                else {
                    $s = Get-RegSnapshot $r
                    $rec.Reg += @{ P = $r.P; N = $r.N; Exists = $s.Exists; Value = $s.Value; Type = $s.Type }
                }
            }
            Set-RegEntry $r.P $r.N $r.V $r.T
        } catch {
            $msg = $_.Exception.Message
            if ($msg -match 'not allowed|denied') { $msg += ' (this key is write-protected on this PC by Windows, a policy or another tool)' }
            Write-Log "  registry $($r.P)\$($r.N): $msg" 'WARN'
        }
    }
    foreach ($s in $T.Svc) {
        $snap = Get-SvcSnapshot $s.Name
        if (-not $snap) { continue }
        if ($fresh) { $sh = Find-SharedOriginal $state $T.Id 'Svc' $s.Name; $rec.Svc += $(if ($sh) { @{ Name = $s.Name; Start = [int]$sh.Start; Delayed = [int]$sh.Delayed } } else { $snap }) }
        if ($snap.Start -ne $s.Start -and -not (Set-SvcStart $s.Name $s.Start)) { Write-Log "  service $($s.Name): could not change (protected?)" 'WARN' }
    }
    foreach ($tk in $T.Task) {
        $snap = Get-TaskSnapshot $tk
        if (-not $snap) { continue }
        if ($fresh) { $sh = Find-SharedOriginal $state $T.Id 'Task' $tk; $rec.Task += $(if ($sh) { @{ Full = $tk; Enabled = [int]$sh.Enabled } } else { $snap }) }
        Set-TaskEnabled $tk $false
    }
    if ($T.Apply) {
        try {
            $d = & $T.Apply
            if ($fresh) { $rec.Data = $d }
        } catch { Write-Log "  script: $($_.Exception.Message)" 'WARN' }
    }
    $state[$T.Id] = $rec
    Save-OptiState $state
    return (Test-Tweak $T)
}

function Invoke-TweakUndo {
    param($T)
    Write-Log "Undo: $($T.Name)"
    Clear-TaskCache
    $state = Get-OptiState
    $rec = $state[$T.Id]
    if ($rec) {
        foreach ($r in @($rec.Reg)) { Restore-RegSnapshot $r }
        foreach ($s in @($rec.Svc)) { [void](Set-SvcStart $s.Name $s.Start $s.Delayed) }
        foreach ($tk in @($rec.Task)) { if ($tk.Enabled) { Set-TaskEnabled $tk.Full $true } }
    } elseif ($T.Reg.Count -or $T.Svc.Count -or $T.Task.Count) {
        Write-Log '  no saved original values for this tweak (was it applied by Optimaxer?)' 'WARN'
    }
    if ($T.Undo) {
        try { & $T.Undo $(if ($rec) { $rec.Data }) | Out-Null } catch { Write-Log "  undo script: $($_.Exception.Message)" 'WARN' }
    }
    if ($state.ContainsKey($T.Id)) { $state.Remove($T.Id); Save-OptiState $state }
    return (Test-Tweak $T)
}

# ---------------------------------------------------------------- startup items
function Get-StartupItems {
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
            @{ Name = $p.Name; Cmd = [string]$p.Value; Scope = $d.Scope; Approved = $d.Approved; Enabled = (Test-StartupEnabled $d.Approved $p.Name) }
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
            @{ Name = $i.Name; Cmd = $i.FullName; Scope = $f.Scope; Approved = $f.Approved; Enabled = (Test-StartupEnabled $f.Approved $i.Name) }
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
function Get-PathSize {
    param([string]$Path)
    $t = 0.0
    foreach ($i in Get-ChildItem -Path $Path -Force -ErrorAction SilentlyContinue) {
        if ($i.PSIsContainer) { $t += [double]((Get-ChildItem -LiteralPath $i.FullName -Recurse -Force -File -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum) }
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
    if ($Target.Special) { return -1.0 }
    $size = 0.0
    foreach ($p in $Target.Paths) { $size += Get-PathSize $p }
    return $size
}

function Invoke-CleanupTarget {
    param($Target)
    switch ($Target.Special) {
        'recycle'   { $before = Get-CleanupSize $Target; Clear-RecycleBin -Force -ErrorAction SilentlyContinue; return $before }
        'component' { Dism.exe /Online /Cleanup-Image /StartComponentCleanup | Out-Null; return 0.0 }
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

# ---------------------------------------------------------------- WinGet runner (same flow and exit-code handling as WinUtil's installer)
function Invoke-WingetPackages {
    # One winget process per package, real exit code, per-package outcome, and a verification step.
    param([ValidateSet('Install', 'Uninstall', 'Upgrade')][string]$Action, [string[]]$Programs)

    $adminContextProhibited = -1978335107   # per-user package touched from an elevated process
    $nothingToDo = @{ -1978335135 = 'already installed'; -1978335189 = 'no applicable update' }
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

