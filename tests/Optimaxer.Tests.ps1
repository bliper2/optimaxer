# Engine and catalog tests for the PowerShell edition. Plain PowerShell 5.1, no Pester needed:
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\Optimaxer.Tests.ps1
# Every registry write goes to HKCU:\Software\OptimaxerTest, and state/feature files go to %TEMP%. Live system
# commands (bcdedit, wevtutil, restore points) are replaced by stubs; nothing on this PC is changed.

$ErrorActionPreference = 'Stop'
$global:OptiRoot = Split-Path $PSScriptRoot -Parent
. "$global:OptiRoot\src\Core.ps1"
. "$global:OptiRoot\src\Data.ps1"
. "$global:OptiRoot\src\Catalog.ps1"

$tmp = Join-Path ([IO.Path]::GetTempPath()) ('opti-tests-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $tmp -Force | Out-Null
$global:OptiStatePath = Join-Path $tmp 'state.json'
$global:OptiLogPath = Join-Path $tmp 'test.log'
$global:OptiFeaturesPath = Join-Path $tmp 'features.json'
$global:OptiBackupDir = Join-Path $tmp 'backups'
New-Item -ItemType Directory -Path $global:OptiBackupDir -Force | Out-Null
$TestKey = 'HKCU:\Software\OptimaxerTest'

$script:pass = 0; $script:fail = @()
function Assert {
    param([bool]$Cond, [string]$Msg)
    if (-not $Cond) { throw $Msg }
}
function It {
    param([string]$Name, [scriptblock]$Body)
    Remove-Item -LiteralPath $TestKey -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $global:OptiStatePath, $global:OptiFeaturesPath -Force -ErrorAction SilentlyContinue
    $global:OptiUndoFailed = @{}
    try { & $Body; $script:pass++; Write-Host "  ok   $Name" }
    catch { $script:fail += $Name; Write-Host "  FAIL $Name : $($_.Exception.Message)" -ForegroundColor Red }
}
function New-FakeTweak {
    param([hashtable]$P)
    $t = [pscustomobject]@{ Id = 'test-x'; Name = 'test'; Desc = ''; Cat = 'Test'; Risk = 'Safe'; Tags = @(); Reg = @(); Svc = @(); Task = @()
        Apply = $null; Undo = $null; Test = $null; Snapshot = $null; Conflict = ''; Restart = $false; Explorer = $false
        DesktopOnly = $false; SSDOnly = $false; Vendor = ''; SvcTol = 1.0; MinRamGB = 0; Need = @() }
    foreach ($k in $P.Keys) { $t.$k = $P[$k] }
    [void]$global:OptiCatalog.Add($t)
    return $t
}
function Save-Functions { param([string[]]$Names) $h = @{}; foreach ($n in $Names) { $h[$n] = (Get-Item "function:$n").ScriptBlock }; $h }
function Restore-Functions { param($Saved) foreach ($n in $Saved.Keys) { Set-Item -Path "function:global:$n" -Value $Saved[$n] } }
function Remove-FakeTweaks { foreach ($t in @($global:OptiCatalog | Where-Object { $_.Id -like 'test-*' })) { $global:OptiCatalog.Remove($t) } }
function New-FakeDnsSnapshot {
    [pscustomobject]@{
        interfaces = @([pscustomobject]@{
            interface_guid = '11111111-2222-3333-4444-555555555555'; name = 'Ethernet'; status = 'Up'
            ipv4 = [pscustomobject]@{ dhcp = $false; servers = @('192.0.2.53', '192.0.2.54') }
            ipv6 = [pscustomobject]@{ dhcp = $true; servers = @() }
        })
        doh_supported = $false
        doh_servers = @()
    }
}
function Install-FakeDnsCommands {
    function global:Get-OptiDnsSnapshot { param([bool]$RequireDoh = $false) return $global:FakeDnsSnapshot }
    function global:Invoke-OptiDnsNetsh {
        param([string[]]$Arguments)
        $global:FakeDnsMutationCount++
        $global:FakeDnsBackupSeen = Test-Path -LiteralPath (Get-OptiDnsBackupPath)
        if ($global:FakeDnsFailProtocol -eq $Arguments[1]) { throw 'simulated netsh failure' }
        if ($Arguments[0] -eq 'dns') {
            $global:FakeDnsDohVerbs += $Arguments[1]
            $server = ($Arguments | Where-Object { $_.StartsWith('server=', [StringComparison]::OrdinalIgnoreCase) } | Select-Object -First 1).Substring(7)
            $global:FakeDnsSnapshot.doh_servers = @($global:FakeDnsSnapshot.doh_servers | Where-Object { $_.server_address -ine $server })
            if ($Arguments[1] -in @('add', 'set')) {
                $template = ($Arguments | Where-Object { $_.StartsWith('dohtemplate=', [StringComparison]::OrdinalIgnoreCase) } | Select-Object -First 1).Substring(12)
                $global:FakeDnsSnapshot.doh_servers += [pscustomobject]@{ server_address = $server; doh_template = $template; auto_upgrade = ($Arguments -contains 'autoupgrade=yes'); allow_fallback_to_udp = ($Arguments -contains 'udpfallback=yes') }
            }
            return
        }
        if ($Arguments[0] -ne 'interface') { throw "unexpected command: $($Arguments -join ' ')" }
        $adapter = $global:FakeDnsSnapshot.interfaces[0]
        $property = if ($Arguments[1] -eq 'ipv4') { 'ipv4' } else { 'ipv6' }
        $addresses = @($Arguments | Where-Object { $_.StartsWith('address=', [StringComparison]::OrdinalIgnoreCase) } | ForEach-Object { $_.Substring(8) })
        if ($Arguments -contains 'source=dhcp') { $adapter.$property = [pscustomobject]@{ dhcp = $true; servers = @() } }
        elseif ($Arguments[2] -eq 'set') { $adapter.$property = [pscustomobject]@{ dhcp = $false; servers = $addresses } }
        else { $adapter.$property.servers = @($adapter.$property.servers) + $addresses }
    }
    function global:Clear-DnsClientCache { }
}
function Restore-FakeDnsCommands {
    param($Saved)
    Restore-Functions $Saved
    Remove-Item -LiteralPath 'Function:\Clear-DnsClientCache' -Force -ErrorAction SilentlyContinue
}

$demoted = 'perf-prefetch', 'perf-paging', 'perf-timer', 'perf-cpupower', 'perf-priosep', 'game-mpo', 'game-fso', 'net-nagle', 'perf-bgapps', 'perf-memcompress', 'perf-fth'
$refPC = @{ Laptop = $false; SSD = $true; Vendor = ''; RamGB = 63.8; Threads = 28; Gpus = @('NVIDIA GeForce RTX 3090'); Build = 26100
            Tags = @('desktop', 'ssd', 'nvme', 'nvidia', 'dgpu', 'win11', 'ram16', 'ram32', 'cores8') }
$laptop = @{ Laptop = $true; SSD = $true; Vendor = ''; RamGB = 8; Threads = 8; Gpus = @('Intel(R) UHD Graphics'); Build = 26100
             Tags = @('laptop', 'ssd', 'intelgpu', 'win11', 'ram8', 'cores8') }

Write-Host 'catalog'
It 'has no duplicate ids' {
    $d = @($global:OptiCatalog | Group-Object Id | Where-Object Count -gt 1 | ForEach-Object Name)
    Assert ($d.Count -eq 0) "duplicates: $($d -join ', ')"
}
It 'demoted tweaks are opt-in and not in safe, gaming or max' {
    foreach ($id in $demoted) {
        $t = Get-TweakById $id
        Assert ($null -ne $t) "$id missing"
        foreach ($tag in 'safe', 'gaming', 'max') { Assert ($t.Tags -notcontains $tag) "$id still tagged $tag" }
        Assert ($t.Tags -contains 'optin') "$id not opt-in"
        $want = if ($id -eq 'perf-fth') { 'Moderate' } else { 'Advanced' }
        Assert ($t.Risk -eq $want) "$id risk $($t.Risk), want $want"
    }
}
It 'perf-ultimate, perf-bgapps and perf-fth are not tagged safe' {
    foreach ($id in 'perf-ultimate', 'perf-bgapps', 'perf-fth') { Assert ((Get-TweakById $id).Tags -notcontains 'safe') $id }
}
It 'perf-fth no longer resets the FTH list' {
    $t = Get-TweakById 'perf-fth'
    Assert ($null -eq $t.Apply) 'perf-fth still has an apply script'
    Assert (-not ((Get-Content "$global:OptiRoot\src\CatalogExtra.ps1" -Raw) -match 'FthSysprepSpecialize')) 'rundll32 call still in the catalog'
}
It 'every safe-tagged tweak has Risk Safe' {
    $bad = @($global:OptiCatalog | Where-Object { $_.Tags -contains 'safe' -and $_.Risk -ne 'Safe' } | ForEach-Object Id)
    Assert ($bad.Count -eq 0) "safe but not Safe risk: $($bad -join ', ')"
}
It 'Recommended has no power plan, background-app ban, Game DVR machine policy or bcdedit' {
    foreach ($t in @($global:OptiCatalog | Where-Object { $_.Tags -contains 'safe' })) {
        $text = "$($t.Apply) $($t.Snapshot) $(@($t.Reg | ForEach-Object { $_.N }) -join ' ')"
        Assert ($text -notmatch 'powercfg|bcdedit|Invoke-Bcd|LetAppsRunInBackground|GlobalUserDisabled|AllowGameDVR') "$($t.Id) is in Recommended"
    }
}
It 'Gaming preset is Game Mode, per-user Game DVR, sticky keys and mouse acceleration' {
    $g = @($global:OptiCatalog | Where-Object { $_.Tags -contains 'gaming' } | ForEach-Object Id | Sort-Object)
    Assert (($g -join ',') -eq 'game-dvr,game-mode,game-mouse,game-sticky') "gaming = $($g -join ',')"
    Assert (-not (@((Get-TweakById 'game-dvr').Reg) | Where-Object { $_.P -like 'HKLM:*' })) 'game-dvr writes machine-wide values'
    $pol = Get-TweakById 'game-dvr-policy'
    Assert ($pol -and $pol.Tags -contains 'optin' -and (@($pol.Reg | ForEach-Object N) -contains 'AllowGameDVR')) 'AllowGameDVR is not in its own opt-in tweak'
}
It 'HAGS needs a discrete GPU and stays Moderate' {
    $t = Get-TweakById 'perf-hags'
    Assert ($t.Risk -eq 'Moderate' -and $t.Need -contains 'dgpu') 'perf-hags gate or risk changed'
}

Write-Host 'smart optimize'
It 'reference gaming desktop gets the performance set but no Advanced, heat or boot tweaks' {
    $plan = Get-SmartPlan $refPC @{}
    foreach ($id in 'perf-hags', 'perf-powerthrottle', 'perf-usbpcie', 'perf-globaltimer', 'game-mode', 'game-dvr', 'perf-trimon') { Assert ($plan.Ids -contains $id) "$id not selected" }
    foreach ($id in 'perf-timer', 'perf-cpupower', 'net-nagle', 'perf-prefetch', 'perf-ultimate', 'perf-fth', 'game-windowed') { Assert ($plan.Ids -notcontains $id) "$id selected" }
    foreach ($id in $plan.Ids) {
        $t = Get-TweakById $id
        Assert ($t.Risk -ne 'Advanced' -and $t.Tags -notcontains 'optin' -and -not $t.Retired) "$id should never be automatic"
        Assert ($t.Risk -eq 'Safe' -or $t.Tags -contains 'perf') "$id is Moderate without the perf tag"
    }
}
It 'laptop gets no desktop power tweaks' {
    $plan = Get-SmartPlan $laptop @{}
    foreach ($id in 'perf-powerthrottle', 'perf-usbpcie', 'perf-globaltimer', 'perf-hags') { Assert ($plan.Ids -notcontains $id) "$id on a laptop" }
    Assert ($plan.Skipped['perf-powerthrottle'] -match 'laptop') "reason: $($plan.Skipped['perf-powerthrottle'])"
}
It 'retired tweaks are never ticked or offered in presets' {
    $ret = @($global:OptiCatalog | Where-Object Retired)
    Assert ($ret.Count -ge 8) "only $($ret.Count) retired"
    foreach ($p in 'max', 'safe', 'privacy', 'deai', 'gaming') { foreach ($id in Get-PresetTweakIds $p $refPC) { Assert (-not (Get-TweakById $id).Retired) "$id in $p" } }
    Assert (-not @((Get-SmartPlan $refPC @{}).Ids | Where-Object { (Get-TweakById $_).Retired })) 'retired in Smart optimize'
}
It 'skipped Moderate and Advanced tweaks carry a reason' {
    $plan = Get-SmartPlan $refPC @{}
    Assert ($plan.Skipped['perf-mmagent'] -match 'Moderate') "perf-mmagent reason: $($plan.Skipped['perf-mmagent'])"
    Assert ([bool]($plan.Notes -match 'Advanced tweaks and other Moderate ones are left for you to tick')) 'note missing'
    $unexplained = @($global:OptiCatalog | Where-Object { $_.Risk -ne 'Safe' -and $_.Tags -notcontains 'optin' -and -not $_.Retired -and -not (Test-AutoPick $_) -and -not $plan.Skipped.ContainsKey($_.Id) } | ForEach-Object Id)
    Assert ($unexplained.Count -eq 0) "no reason for: $($unexplained -join ', ')"
}
It 'laptop notes name the hardware fact' {
    $plan = Get-SmartPlan $laptop @{}
    Assert ([bool]($plan.Notes -match 'Laptop')) 'no laptop note'
    Assert ([bool]($plan.Notes -match '8 GB RAM')) 'no RAM note'
    Assert ((Get-TweakFitReason (Get-TweakById 'perf-ultimate') $laptop) -eq 'this PC is a laptop') 'fit reason'
}
It 'a tweak tagged gaming only (mouse acceleration) is not auto-selected' {
    Assert ((Get-SmartPlan $refPC @{}).Ids -notcontains 'game-mouse') 'game-mouse auto-selected'
}
It 'presets tick at most one tweak per conflict group' {
    foreach ($p in 'max', 'safe') {
        $ids = @(Get-PresetTweakIds $p $refPC)
        $g = @($ids | ForEach-Object { (Get-TweakById $_).Conflict } | Where-Object { $_ } | Group-Object | Where-Object Count -gt 1)
        Assert ($g.Count -eq 0) "$p preset has two of: $($g.Name -join ', ')"
    }
    Assert (@($global:OptiCatalog | Where-Object { $_.Conflict -eq 'search style' }).Count -ge 2) 'search style group missing'
}

Write-Host 'engine'
It 'retry after partial apply preserves the original registry and script values' {
    New-Item -Path $TestKey -Force | Out-Null
    Set-ItemProperty -LiteralPath $TestKey -Name V -Value 7 -Type DWord
    $saved = Save-Functions @('Set-RegEntry')
    $t = New-FakeTweak @{ Id = 'test-retry'; Reg = @(
        @{ P = $TestKey; N = 'V'; V = 1; T = 'DWord' },
        @{ P = $TestKey; N = 'New'; V = 1; T = 'DWord' })
        Snapshot = { @{ Previous = (Get-ItemProperty -LiteralPath $TestKey).V } }
        Apply = { param($d) }
        Undo = { param($d) Assert ($d.Previous -eq 7) 'script original was replaced' } }
    try {
        function global:Set-RegEntry {
            param($P, $N, $V, $T)
            if ($N -eq 'New') { throw 'simulated write failure' }
            Set-ItemProperty -LiteralPath $P -Name $N -Value $V -Type $T
        }
        Assert (-not (Invoke-TweakApply $t)) 'partial apply reported success'
        Assert ((Get-ItemProperty -LiteralPath $TestKey).V -eq 1) 'first write did not run'
        Restore-Functions $saved
        Assert (Invoke-TweakApply $t) 'retry did not apply'
        $rec = (Get-OptiState)[$t.Id]
        Assert ($rec.Reg[0].Value -eq 7) 'registry original was replaced'
        Assert (-not $rec.Reg[1].Exists) 'absent original was replaced'
        Assert ($rec.Data.Previous -eq 7) 'script original was replaced'
        [void](Invoke-TweakUndo $t)
        Assert (-not $global:OptiUndoFailed.ContainsKey($t.Id)) 'undo failed'
        Assert ((Get-ItemProperty -LiteralPath $TestKey).V -eq 7) 'undo did not restore 7'
        Assert ($null -eq (Get-ItemProperty -LiteralPath $TestKey).New) 'undo did not remove new value'
    } finally { Restore-Functions $saved; Remove-FakeTweaks }
}
It 'record is on disk before the first change' {
    New-Item -Path $TestKey -Force | Out-Null
    Set-ItemProperty -LiteralPath $TestKey -Name V -Value 7 -Type DWord
    $global:SeenInState = $null
    $t = New-FakeTweak @{ Id = 'test-order'; Reg = @(@{ P = $TestKey; N = 'V'; V = 1; T = 'DWord' })
        Apply = { $s = Get-OptiState; $global:SeenInState = $s['test-order']; throw 'apply step failed' } }
    try {
        [void](Invoke-TweakApply $t)
        Assert ($null -ne $global:SeenInState) 'record not saved before the script ran'
        $rec = (Get-OptiState)['test-order']
        Assert ($rec -and $rec.Reg[0].Value -eq 7) 'original value not kept after the failed step'
        [void](Invoke-TweakUndo $t)
        Assert ((Get-ItemProperty -LiteralPath $TestKey).V -eq 7) 'undo did not restore 7'
    } finally { Remove-FakeTweaks }
}
It 'Invoke-TweakUndo keeps the record when the undo script throws' {
    $t = New-FakeTweak @{ Id = 'test-undo'; Apply = { @{ X = 1 } }; Undo = { param($d) throw 'cannot restore' } }
    try {
        [void](Invoke-TweakApply $t)
        Assert ((Get-OptiState).ContainsKey('test-undo')) 'no record after apply'
        [void](Invoke-TweakUndo $t)
        Assert ((Get-OptiState).ContainsKey('test-undo')) 'record removed after a failed undo'
        Assert ($global:OptiUndoFailed.ContainsKey('test-undo')) 'failure not reported'
        $all = Invoke-UndoAll
        Assert ($all.Failed.ContainsKey('test-undo')) 'Undo All does not report the failure'
    } finally { Remove-FakeTweaks }
}
It 'undoing one tweak keeps a value another applied tweak still needs' {
    New-Item -Path $TestKey -Force | Out-Null
    $a = New-FakeTweak @{ Id = 'test-a'; Reg = @(@{ P = $TestKey; N = 'Shared'; V = 0; T = 'DWord' }) }
    $b = New-FakeTweak @{ Id = 'test-b'; Reg = @(@{ P = $TestKey; N = 'Shared'; V = 0; T = 'DWord' }) }
    try {
        [void](Invoke-TweakApply $a); [void](Invoke-TweakApply $b)
        [void](Invoke-TweakUndo $b)
        Assert ((Get-ItemProperty -LiteralPath $TestKey).Shared -eq 0) 'undoing b removed the value a still needs'
        Assert (Test-Tweak $a) 'a is no longer active'
        [void](Invoke-TweakUndo $a)
        Assert ($null -eq (Get-ItemProperty -LiteralPath $TestKey).Shared) 'the last undo did not put the original back'
    } finally { Remove-FakeTweaks }
}
It 'a successful undo removes the record' {
    $t = New-FakeTweak @{ Id = 'test-ok'; Apply = { 1 }; Undo = { param($d) } }
    try {
        [void](Invoke-TweakApply $t); [void](Invoke-TweakUndo $t)
        Assert (-not (Get-OptiState).ContainsKey('test-ok')) 'record kept'
    } finally { Remove-FakeTweaks }
}
It 'net-nagle undo restores a pre-existing TcpAckFrequency instead of deleting it' {
    $old = $global:OptiTcpIfBase
    $global:OptiTcpIfBase = "$TestKey\Interfaces"
    try {
        New-Item -Path "$TestKey\Interfaces\{A}" -Force | Out-Null
        New-Item -Path "$TestKey\Interfaces\{B}" -Force | Out-Null
        Set-ItemProperty -LiteralPath "$TestKey\Interfaces\{A}" -Name DhcpIPAddress -Value '192.168.1.20'
        Set-ItemProperty -LiteralPath "$TestKey\Interfaces\{A}" -Name TcpAckFrequency -Value 2 -Type DWord
        $t = Get-TweakById 'net-nagle'
        Assert (Invoke-TweakApply $t) 'apply not verified'
        $a = Get-ItemProperty -LiteralPath "$TestKey\Interfaces\{A}"
        Assert ($a.TcpAckFrequency -eq 1 -and $a.TCPNoDelay -eq 1) 'apply did not set both values'
        Assert ($null -eq (Get-ItemProperty -LiteralPath "$TestKey\Interfaces\{B}").TCPNoDelay) 'an interface without an address was changed'
        [void](Invoke-TweakUndo $t)
        $a = Get-ItemProperty -LiteralPath "$TestKey\Interfaces\{A}"
        Assert ($a.TcpAckFrequency -eq 2) "TcpAckFrequency is $($a.TcpAckFrequency), want 2"
        Assert ($null -eq $a.TCPNoDelay) 'TCPNoDelay should be gone again'
        Assert (-not (Get-OptiState).ContainsKey('net-nagle')) 'record kept after a clean undo'
    } finally { $global:OptiTcpIfBase = $old }
}
It 'perf-timer undo puts back the exact boot values' {
    $saved = Save-Functions 'Invoke-Bcd', 'Get-BcdTimerValues'
    $global:BcdCalls = @()
    function global:Invoke-Bcd { param([string[]]$Arguments) $global:BcdCalls += ($Arguments -join ' '); '' }
    function global:Get-BcdTimerValues { @{ disabledynamictick = 'Yes'; useplatformclock = $null; useplatformtick = 'No' } }
    try {
        Restore-BcdTimerValues ([pscustomobject]@{ disabledynamictick = $null; useplatformclock = 'Yes'; useplatformtick = 'No' })
        Assert ($global:BcdCalls -contains '/deletevalue disabledynamictick') "calls: $($global:BcdCalls -join '; ')"
        Assert ($global:BcdCalls -contains '/set useplatformclock Yes') "calls: $($global:BcdCalls -join '; ')"
        Assert (-not ($global:BcdCalls -match 'useplatformtick')) 'unchanged value was touched'
    } finally { Restore-Functions $saved }
}
It 'Windows Update pause undo restores prior values and types after a retry' {
    $oldKey = $global:OptiUpdatePauseKey
    $global:OptiUpdatePauseKey = "$TestKey\UpdatePause"
    try {
        Set-RegEntry $global:OptiUpdatePauseKey 'PauseUpdatesStartTime' 'original-start' 'String'
        Set-RegEntry $global:OptiUpdatePauseKey 'PauseUpdatesExpiryTime' 42 'DWord'
        $t = Get-TweakById 'wu-pause'
        Assert (Invoke-TweakApply $t) 'pause apply not verified'
        $expiry = Get-ItemProperty -LiteralPath $global:OptiUpdatePauseKey -Name PauseUpdatesExpiryTime
        Assert ($expiry.PauseUpdatesExpiryTime -is [string] -and [DateTime]::Parse($expiry.PauseUpdatesExpiryTime).ToUniversalTime() -gt [DateTime]::UtcNow) 'pause expiry was not set'
        Set-RegEntry $global:OptiUpdatePauseKey 'PauseUpdatesStartTime' 'drift-before-retry' 'String'
        Assert (Invoke-TweakApply $t) 'retry apply not verified'
        [void](Invoke-TweakUndo $t)
        $state = Get-ItemProperty -LiteralPath $global:OptiUpdatePauseKey
        Assert ($state.PauseUpdatesStartTime -eq 'original-start') 'original start value was not restored'
        Assert ($state.PauseUpdatesExpiryTime -eq 42) 'original expiry value or type was not restored'
        foreach ($name in 'PauseFeatureUpdatesStartTime', 'PauseQualityUpdatesStartTime', 'PauseFeatureUpdatesEndTime', 'PauseQualityUpdatesEndTime') {
            Assert ($null -eq $state.$name) "$name should be absent after undo"
        }
        Assert (-not (Get-OptiState).ContainsKey('wu-pause')) 'record kept after successful restore'
    } finally { $global:OptiUpdatePauseKey = $oldKey }
}
It 'perf-sleepstudy undo restores each log state' {
    $saved = Save-Functions 'Invoke-Wevtutil'
    $global:WevCalls = @()
    function global:Invoke-Wevtutil { param([string[]]$Arguments) $global:WevCalls += ($Arguments -join ' '); '' }
    try {
        Restore-EventLogStates ([pscustomobject]@{ 'Log/A' = $false; 'Log/B' = $true })
        Assert ($global:WevCalls -contains 'set-log Log/A /e:false') "calls: $($global:WevCalls -join '; ')"
        Assert ($global:WevCalls -contains 'set-log Log/B /e:true') "calls: $($global:WevCalls -join '; ')"
    } finally { Restore-Functions $saved }
}

Write-Host 'restore point'
function Use-FailingRestorePoint {
    $global:ApplyCalls = 0
    function global:New-OptiRestorePoint { param($d) $false }
    function global:Invoke-TweakApply { param($T) $global:ApplyCalls++; $true }
}
It 'GUI job: a failed restore point applies nothing and asks' {
    $saved = Save-Functions 'New-OptiRestorePoint', 'Invoke-TweakApply'
    Use-FailingRestorePoint
    try {
        $out = @(Invoke-TweakJobBody @('game-mode', 'ui-ext') 'apply' $true)
        Assert ($out -contains 'restore-failed') "job output: $($out -join ',')"
        Assert ($global:ApplyCalls -eq 0) "$global:ApplyCalls tweak(s) applied"
        $app = Get-Content "$global:OptiRoot\src\App.ps1" -Raw
        Assert ($app -match "-contains 'restore-failed'" -and $app -match 'Confirm-Action "The restore point could not be created') 'window does not ask before going on'
    } finally { Restore-Functions $saved }
}
It '-Silent without -NoRestorePoint exits non-zero and applies nothing' {
    $saved = Save-Functions 'New-OptiRestorePoint', 'Invoke-TweakApply'
    Use-FailingRestorePoint
    try {
        $code = Invoke-UnattendedTweaks -Ids @('game-mode')
        Assert ($code -ne 0) 'exit code 0'
        Assert ($global:ApplyCalls -eq 0) "$global:ApplyCalls tweak(s) applied"
        $code2 = Invoke-UnattendedTweaks -Ids @('game-mode') -NoRestorePoint
        Assert ($code2 -eq 0 -and $global:ApplyCalls -eq 1) '-NoRestorePoint should apply'
        Assert ((Get-Content "$global:OptiRoot\Optimaxer.ps1" -Raw) -match 'Invoke-UnattendedTweaks') 'Optimaxer.ps1 does not use it'
    } finally { Restore-Functions $saved }
}

Write-Host 'auto clean and tools'
It 'hourly auto clean no longer empties working sets or the standby list' {
    $b = Get-AutoCleanBody
    Assert ($b -notmatch 'TrimWorkingSets|PurgeStandbyList') 'memory calls still in the hourly script'
    Assert ($b -match 'Clear-OldTemp') 'temp clean missing'
    Assert ((Get-AutoCleanScript) -notmatch 'OptiMem') 'native memory helpers still bundled'
    Assert ((Get-Content "$global:OptiRoot\src\MainWindow.xaml" -Raw) -notmatch 'Free RAM and clear temp') 'dashboard still says it frees RAM'
}
It 'per-game profile writes only its own values and undo restores them' {
    $global:OptiGpuPrefKey = "$TestKey\Gpu"; $global:OptiLayersKey = "$TestKey\Layers"
    $exe = Join-Path $tmp 'game.exe'; Set-Content -LiteralPath $exe -Value 'x'
    New-Item -Path $global:OptiLayersKey -Force | Out-Null
    Set-ItemProperty -LiteralPath $global:OptiLayersKey -Name $exe -Value '~ HIGHDPIAWARE'
    Add-GameProfile @($exe)
    Assert (Test-GameProfile $exe) 'profile not in effect'
    Assert ((Get-ItemProperty -LiteralPath $global:OptiLayersKey).$exe -match 'HIGHDPIAWARE') 'existing flag lost'
    Assert (Remove-GameProfile) 'undo failed'
    Assert ((Get-ItemProperty -LiteralPath $global:OptiLayersKey).$exe -eq '~ HIGHDPIAWARE') 'Layers not restored'
    Assert ($null -eq (Get-ItemProperty -LiteralPath $global:OptiGpuPrefKey -ErrorAction SilentlyContinue).$exe) 'GPU value not removed'
}
It 'GPU preference merge keeps other DirectX settings' {
    Assert ((Set-KeyValuePair 'SwapEffectUpgradeEnable=1;' 'GpuPreference' '2') -eq 'SwapEffectUpgradeEnable=1;GpuPreference=2;') 'merge'
    Assert ((Set-KeyValuePair 'GpuPreference=1;' 'GpuPreference' '2') -eq 'GpuPreference=2;') 'replace'
}
It 'update and Defender tasks are never offered as wake sources' {
    Assert (Test-ProtectedWakeTask '\Microsoft\Windows\UpdateOrchestrator\') 'UpdateOrchestrator'
    Assert (Test-ProtectedWakeTask '\Microsoft\Windows\Windows Defender\') 'Defender'
    Assert (-not (Test-ProtectedWakeTask '\Vendor\Updater\')) 'third-party task blocked'
}
It 'dev folder search finds build trees and skips AppData' {
    $root = Join-Path $tmp 'profile'
    foreach ($d in 'proj\node_modules\x', 'proj\.git', 'rust\target', 'AppData\Local\proj\node_modules', 'docs') { New-Item -ItemType Directory -Path (Join-Path $root $d) -Force | Out-Null }
    $hits = @(Find-DevTrees -Root $root | ForEach-Object { $_.Substring($root.Length + 1) })
    Assert (($hits -join ',') -eq 'proj\.git,proj\node_modules,rust\target') "hits: $($hits -join ',')"
}
It 'frame statistics match hand-computed values' {
    $s = Get-FrameStats (@(10.0) * 990 + @(40.0) * 10)
    Assert ($s.Frames -eq 1000) 'frames'
    Assert ([math]::Abs($s.AvgFps - 1000 / 10.3) -lt 0.1) "avg $($s.AvgFps)"
    Assert ($s.Low1 -eq 25) "1% low $($s.Low1)"
    Assert ($s.Low01 -eq 25) "0.1% low $($s.Low01)"
    Assert ($s.Stutters -eq 10) "stutters $($s.Stutters)"
    Assert ($null -eq (Get-FrameStats @(16.6, 16.7))) 'too few frames should give no result'
}
It 'measurement comparison reports direction per metric' {
    $old = $global:OptiMeasurePath
    $global:OptiMeasurePath = Join-Path $tmp 'm.json'
    try {
        $b = [ordered]@{ Label = 'before'; Time = '2026-01-01T00:00:00'; Boot = @{ Ms = 30000; At = 'a' }; Latency = @{ DpcPct = 0.5; IsrPct = 0.2; JitterMs = 3 }; Frames = @{ Process = 'g.exe'; AvgFps = 100; Low1 = 60; Low01 = 40; P99Ms = 20 } }
        $a = [ordered]@{ Label = 'after'; Time = '2026-01-01T01:00:00'; Boot = @{ Ms = 25000; At = 'b' }; Latency = @{ DpcPct = 0.6; IsrPct = 0.2; JitterMs = 2 }; Frames = @{ Process = 'g.exe'; AvgFps = 110; Low1 = 60; Low01 = 45; P99Ms = 15 } }
        ConvertTo-Json -InputObject @($b, $a) -Depth 5 | Set-Content -LiteralPath $global:OptiMeasurePath
        $c = @(Compare-OptiMeasurements)
        Assert ([bool]($c -match 'Average FPS \(g.exe\): 100 -> 110 \(\+10.0%, better\)')) ($c -join ' | ')
        Assert ([bool]($c -match '1% low FPS: 60 -> 60 \(0%, same\)')) ($c -join ' | ')
        Assert ([bool]($c -match 'DPC time: 0.5% -> 0.6% \(\+20.0%, worse\)')) ($c -join ' | ')
        Assert ([bool]($c -match 'Boot time: 30 s -> 25 s')) ($c -join ' | ')
    } finally { $global:OptiMeasurePath = $old }
}
It 'Appx removal checks provisioned copies' {
    Assert ((Get-Command Test-AppxGone).ScriptBlock.ToString() -match 'Get-AppxProvisionedPackage') 'Test-AppxGone ignores provisioning'
    Assert ((Get-TweakById 'ai-remove-apps').Test.ToString() -match 'Test-AppxGone') 'ai-remove-apps test'
}

Write-Host 'DNS backup and restore'
It 'DNS changes save originals before mutation and restore both address families' {
    $saved = Save-Functions @('Get-OptiDnsSnapshot', 'Invoke-OptiDnsNetsh')
    $global:FakeDnsSnapshot = New-FakeDnsSnapshot
    $global:FakeDnsMutationCount = 0; $global:FakeDnsBackupSeen = $false; $global:FakeDnsFailProtocol = $null; $global:FakeDnsDohVerbs = @()
    Install-FakeDnsCommands
    try {
        Assert (Set-DnsProvider -Name 'Test DNS' -V4 @('1.1.1.1') -V6 @('2606:4700:4700::1111') -Doh '') 'DNS provider was not applied'
        Assert $global:FakeDnsBackupSeen 'original DNS was not saved before first mutation'
        Assert (Test-Path -LiteralPath (Get-OptiDnsBackupPath)) 'DNS backup missing after apply'
        Assert (Restore-DnsProvider) 'DNS restore failed'
        Assert (Test-OptiDnsFamilyEqual $global:FakeDnsSnapshot.interfaces[0].ipv4 ([pscustomobject]@{ dhcp = $false; servers = @('192.0.2.53', '192.0.2.54') })) 'IPv4 originals were not restored'
        Assert (Test-OptiDnsFamilyEqual $global:FakeDnsSnapshot.interfaces[0].ipv6 ([pscustomobject]@{ dhcp = $true; servers = @() })) 'IPv6 DHCP state was not restored'
        Assert (-not (Test-Path -LiteralPath (Get-OptiDnsBackupPath))) 'verified restore did not remove backup'
    } finally { Restore-FakeDnsCommands $saved; Remove-Item -LiteralPath (Get-OptiDnsBackupPath) -Force -ErrorAction SilentlyContinue }
}
It 'partial DNS apply keeps backup and can be retried through restore' {
    $saved = Save-Functions @('Get-OptiDnsSnapshot', 'Invoke-OptiDnsNetsh')
    $global:FakeDnsSnapshot = New-FakeDnsSnapshot
    $global:FakeDnsMutationCount = 0; $global:FakeDnsBackupSeen = $false; $global:FakeDnsFailProtocol = 'ipv6'
    Install-FakeDnsCommands
    try {
        Assert (-not (Set-DnsProvider -Name 'Test DNS' -V4 @('1.1.1.1') -V6 @('2606:4700:4700::1111') -Doh '')) 'partial failure reported success'
        Assert $global:FakeDnsBackupSeen 'backup was not present at first mutation'
        Assert (Test-Path -LiteralPath (Get-OptiDnsBackupPath)) 'partial apply discarded backup'
        $global:FakeDnsFailProtocol = $null
        Assert (Restore-DnsProvider) 'restore after partial apply failed'
        Assert (-not (Test-Path -LiteralPath (Get-OptiDnsBackupPath))) 'verified retry did not clear backup'
    } finally { Restore-FakeDnsCommands $saved; Remove-Item -LiteralPath (Get-OptiDnsBackupPath) -Force -ErrorAction SilentlyContinue }
}
It 'DNS restore leaves external changes untouched and keeps backup' {
    $saved = Save-Functions @('Get-OptiDnsSnapshot', 'Invoke-OptiDnsNetsh')
    $global:FakeDnsSnapshot = New-FakeDnsSnapshot
    $global:FakeDnsMutationCount = 0; $global:FakeDnsBackupSeen = $false; $global:FakeDnsFailProtocol = $null
    Install-FakeDnsCommands
    try {
        Assert (Set-DnsProvider -Name 'Test DNS' -V4 @('1.1.1.1') -V6 @('2606:4700:4700::1111') -Doh '') 'DNS provider was not applied'
        $beforeRestore = $global:FakeDnsMutationCount
        $global:FakeDnsSnapshot.interfaces[0].ipv4 = [pscustomobject]@{ dhcp = $false; servers = @('203.0.113.53') }
        Assert (-not (Restore-DnsProvider)) 'external DNS change was overwritten'
        Assert ($global:FakeDnsMutationCount -eq $beforeRestore) 'restore mutated DNS before conflict check'
        Assert ($global:FakeDnsSnapshot.interfaces[0].ipv4.servers[0] -eq '203.0.113.53') 'external DNS change did not remain'
        Assert (Test-Path -LiteralPath (Get-OptiDnsBackupPath)) 'conflict discarded recovery backup'
    } finally { Restore-FakeDnsCommands $saved; Remove-Item -LiteralPath (Get-OptiDnsBackupPath) -Force -ErrorAction SilentlyContinue }
}
It 'DNS-over-HTTPS restore preserves the original record and rejects a removed record as external drift' {
    $saved = Save-Functions @('Get-OptiDnsSnapshot', 'Invoke-OptiDnsNetsh')
    $global:FakeDnsSnapshot = New-FakeDnsSnapshot
    $global:FakeDnsSnapshot.doh_supported = $true
    $global:FakeDnsSnapshot.doh_servers = @([pscustomobject]@{ server_address = '1.1.1.1'; doh_template = 'https://old.example/dns-query'; auto_upgrade = $false; allow_fallback_to_udp = $false })
    $global:FakeDnsMutationCount = 0; $global:FakeDnsBackupSeen = $false; $global:FakeDnsFailProtocol = $null
    Install-FakeDnsCommands
    try {
        Assert (Set-DnsProvider -Name 'Test DNS' -V4 @('1.1.1.1') -V6 @() -Doh 'https://new.example/dns-query') 'DNS-over-HTTPS provider was not applied'
        Assert ($global:FakeDnsDohVerbs -contains 'set') 'existing DNS-over-HTTPS entry was added instead of updated'
        $restored = Restore-DnsProvider
        Assert $restored 'DNS-over-HTTPS restore failed'
        Assert ($global:FakeDnsSnapshot.doh_servers[0].doh_template -eq 'https://old.example/dns-query') 'original DNS-over-HTTPS template was not restored'
        Assert (-not $global:FakeDnsSnapshot.doh_servers[0].auto_upgrade) 'original DNS-over-HTTPS auto-upgrade value was not restored'

        Assert (Set-DnsProvider -Name 'Test DNS' -V4 @('1.1.1.1') -V6 @() -Doh 'https://new.example/dns-query') 'DNS-over-HTTPS provider was not reapplied'
        $beforeRestore = $global:FakeDnsMutationCount
        $global:FakeDnsSnapshot.doh_servers = @()
        Assert (-not (Restore-DnsProvider)) 'removed DoH record was treated as unchanged'
        Assert ($global:FakeDnsMutationCount -eq $beforeRestore) 'restore mutated settings after detecting DoH drift'
        Assert (Test-Path -LiteralPath (Get-OptiDnsBackupPath)) 'DoH conflict discarded recovery backup'
    } finally { Restore-FakeDnsCommands $saved; Remove-Item -LiteralPath (Get-OptiDnsBackupPath) -Force -ErrorAction SilentlyContinue }
}
It 'DNS provider rejects wrong address family before saving or changing anything' {
    $saved = Save-Functions @('Get-OptiDnsSnapshot', 'Invoke-OptiDnsNetsh')
    $global:FakeDnsSnapshot = New-FakeDnsSnapshot
    $global:FakeDnsMutationCount = 0; $global:FakeDnsFailProtocol = $null
    Install-FakeDnsCommands
    try {
        $threw = $false
        try { [void](Set-DnsProvider -Name 'Test DNS' -V4 @('::1') -V6 @() -Doh '') } catch { $threw = $true }
        Assert $threw 'IPv6 address accepted as an IPv4 server'
        Assert ($global:FakeDnsMutationCount -eq 0) 'invalid address changed DNS'
        Assert (-not (Test-Path -LiteralPath (Get-OptiDnsBackupPath))) 'invalid address created a misleading backup'
    } finally { Restore-FakeDnsCommands $saved; Remove-Item -LiteralPath (Get-OptiDnsBackupPath) -Force -ErrorAction SilentlyContinue }
}
It 'malformed saved DNS addresses block restore and keep the backup' {
    $invalid = [pscustomobject]@{
        version = 1
        interfaces = @([pscustomobject]@{
            interface_guid = '11111111-2222-3333-4444-555555555555'; name = 'Ethernet'
            original_ipv4 = [pscustomobject]@{ dhcp = $false; servers = @('::1') }
            original_ipv6 = [pscustomobject]@{ dhcp = $true; servers = @() }
            applied = @([pscustomobject]@{
                ipv4 = [pscustomobject]@{ dhcp = $true; servers = @() }
                ipv6 = [pscustomobject]@{ dhcp = $true; servers = @() }
            })
        })
        doh_changes = @()
    }
    ConvertTo-Json -InputObject $invalid -Depth 10 | Set-Content -LiteralPath (Get-OptiDnsBackupPath) -Encoding UTF8
    try {
        $threw = $false
        try { [void](Restore-DnsProvider) } catch { $threw = $true }
        Assert $threw 'wrong-family original accepted from a saved backup'
        Assert (Test-Path -LiteralPath (Get-OptiDnsBackupPath)) 'invalid saved backup was deleted'
    } finally { Remove-Item -LiteralPath (Get-OptiDnsBackupPath) -Force -ErrorAction SilentlyContinue }
}

Write-Host 'safety and failure paths'
It 'failed hardware probes do not invent SSD or desktop traits' {
    function global:Get-CimInstance { param($ClassName, $ErrorAction) throw 'probe unavailable' }
    function global:Get-Partition { param($DriveLetter, $ErrorAction) throw 'probe unavailable' }
    try {
        $traits = Get-SystemTraits
        Assert ($null -eq $traits.SSD -and $null -eq $traits.Laptop) 'unknown hardware converted to a known type'
        Assert (@($traits.Tags | Where-Object { $_ -in 'ssd', 'hdd', 'laptop', 'desktop', 'win10', 'win11' }).Count -eq 0) 'unknown probes invented hardware tags'
        $plan = Get-SmartPlan $traits @{}
        foreach ($id in 'perf-trimon', 'perf-powerthrottle', 'perf-usbpcie', 'perf-globaltimer') { Assert ($plan.Ids -notcontains $id) "$id auto-selected without hardware evidence" }
        Assert ($plan.Skipped['perf-trimon'] -match 'could not be detected') 'unknown SSD reason missing'
        Assert ([bool]($plan.Notes -match 'type unknown')) 'unknown hardware note missing'
        $t = New-FakeTweak @{ Id = 'test-ram'; MinRamGB = 16 }
        Assert (-not (Test-TweakFits $t $traits)) 'unknown RAM passed a minimum-memory gate'
    } finally {
        Remove-Item -LiteralPath 'Function:\Get-CimInstance', 'Function:\Get-Partition'
        Remove-FakeTweaks
    }
}
It 'script failures cannot be reported successful by a positive status test' {
    $t = New-FakeTweak @{ Id = 'test-failure'; Test = { $true }; Apply = { throw 'apply refused' }; Undo = { param($d) $global:LASTEXITCODE = 9 } }
    try {
        Assert (-not (Invoke-TweakApply $t)) 'failed apply reported success'
        [void](Invoke-TweakUndo $t)
        Assert ($global:OptiUndoFailed.ContainsKey($t.Id)) 'native undo failure ignored'
        Assert ((Get-OptiState).ContainsKey($t.Id)) 'undo record discarded after failure'
    } finally { Remove-FakeTweaks }
}
It 'undo without a saved record does not run a default-changing script' {
    $global:UnsavedUndoRan = $false
    $t = New-FakeTweak @{ Id = 'test-unsaved'; Undo = { $global:UnsavedUndoRan = $true } }
    try {
        [void](Invoke-TweakUndo $t)
        Assert (-not $global:UnsavedUndoRan) 'undo guessed defaults without a backup'
        Assert ($global:OptiUndoFailed.ContainsKey($t.Id)) 'missing backup not reported'
    } finally { Remove-FakeTweaks }
}
It 'retry captures newly appearing services and tasks before changing them' {
    $saved = Save-Functions 'Get-SvcSnapshot', 'Set-SvcStart', 'Get-TaskSnapshot', 'Set-TaskEnabled'
    $global:LatePresent = $false; $global:LateStart = 3; $global:LateDelayed = 1; $global:LateEnabled = $true
    function global:Get-SvcSnapshot { param($Name) if ($global:LatePresent) { @{ Name = $Name; Start = $global:LateStart; Delayed = $global:LateDelayed } } }
    function global:Set-SvcStart { param($Name, $Start, $Delayed = 0) $global:LateStart = $Start; $global:LateDelayed = $Delayed; return $true }
    function global:Get-TaskSnapshot {
        param($Full)
        if (-not $global:LatePresent) { return $null }
        if (-not $global:TaskCache.ContainsKey($Full)) { $global:TaskCache[$Full] = [int]$global:LateEnabled }
        return @{ Full = $Full; Enabled = $global:TaskCache[$Full] }
    }
    function global:Set-TaskEnabled { param($Full, $Enabled) $global:LateEnabled = [bool]$Enabled; return $true }
    New-Item -Path $TestKey -Force | Out-Null
    Set-ItemProperty -LiteralPath $TestKey -Name V -Value 7 -Type DWord
    $t = New-FakeTweak @{ Id = 'test-later'; Reg = @(@{ P = $TestKey; N = 'V'; V = 1; T = 'DWord' }); Svc = @(@{ Name = 'LateSvc'; Start = 4 }); Task = @('\Vendor\LateTask') }
    try {
        Assert (Invoke-TweakApply $t) 'initial registry apply failed'
        $global:LatePresent = $true
        Assert (Invoke-TweakApply $t) 'new task status was not refreshed after apply'
        $rec = (Get-OptiState)[$t.Id]
        Assert ($rec.Reg[0].Value -eq 7 -and $rec.Svc[0].Start -eq 3 -and $rec.Task[0].Enabled -eq 1) 'retry lost old or new originals'
        [void](Invoke-TweakUndo $t)
        Assert ($global:LateStart -eq 3 -and $global:LateDelayed -eq 1 -and $global:LateEnabled) 'new resources did not restore'
        $global:LateEnabled = $false
        Assert (Invoke-TweakApply $t) 'apply with originally disabled task failed'
        $global:LateEnabled = $true
        [void](Invoke-TweakUndo $t)
        Assert (-not $global:LateEnabled) 'originally disabled task was not restored after drift'
    } finally { Restore-Functions $saved; Remove-FakeTweaks }
}
It 'TRIM saves before writing, changes NTFS only and restores zero as well as one' {
    $saved = Save-Functions 'Invoke-Native'
    $global:FakeTrim = 1; $global:TrimWrites = @()
    function global:Invoke-Native {
        param($Exe, $Arguments)
        $global:OptiExit = 0; $global:LASTEXITCODE = 0
        if ($Arguments[1] -eq 'query') { return "NTFS DisableDeleteNotify = $global:FakeTrim`nReFS DisableDeleteNotify = 1" }
        Assert ($Arguments[3] -eq 'NTFS') 'TRIM modified filesystems without captured originals'
        $record = (Get-OptiState)['perf-trimon']
        Assert ($record -and $null -ne $record.Data.Prev) 'TRIM changed before its original was saved'
        $global:FakeTrim = [int]$Arguments[4]; $global:TrimWrites += ($Arguments -join ' ')
        return ''
    }
    try {
        $t = Get-TweakById 'perf-trimon'
        Assert (Invoke-TweakApply $t) 'TRIM apply failed'
        [void](Invoke-TweakUndo $t)
        Assert ($global:FakeTrim -eq 1) 'TRIM did not restore disabled original'
        $global:FakeTrim = 0
        Assert (Invoke-TweakApply $t) 'TRIM apply with enabled original failed'
        $global:FakeTrim = 1
        [void](Invoke-TweakUndo $t)
        Assert ($global:FakeTrim -eq 0) 'TRIM did not restore enabled original after drift'
    } finally { Restore-Functions $saved }
}
It 'missing registry export keeps the tweak undo record for retry' {
    $t = New-FakeTweak @{ Id = 'test-regfile'; Apply = { , @((Join-Path $tmp 'missing.reg')) }; Undo = { param($d) Restore-RegFiles $d } }
    try {
        [void](Invoke-TweakApply $t)
        [void](Invoke-TweakUndo $t)
        Assert ($global:OptiUndoFailed.ContainsKey($t.Id)) 'missing export was ignored'
        Assert ((Get-OptiState).ContainsKey($t.Id)) 'missing export lost undo metadata'
    } finally { Remove-FakeTweaks }
}
It 'native launch failure never reuses a prior successful exit code' {
    $global:LASTEXITCODE = 0
    $out = Invoke-Native 'optimaxer-no-such-command-9ab248.exe'
    Assert ($global:OptiExit -ne 0) 'missing command reported success'
    Assert (-not [string]::IsNullOrWhiteSpace($out)) 'missing command omitted error text'
    $threw = $false
    try { [void](Invoke-Native 'optimaxer-no-such-command-9ab248.exe' -CheckExit) } catch { $threw = $true }
    Assert $threw 'checked command failure did not stop its caller'
}
It 'corrupt undo state blocks applying tweaks and preserves the unreadable file' {
    New-Item -Path $TestKey -Force | Out-Null
    Set-ItemProperty -LiteralPath $TestKey -Name V -Value 7 -Type DWord
    $t = New-FakeTweak @{ Id = 'test-state'; Reg = @(@{ P = $TestKey; N = 'V'; V = 1; T = 'DWord' }) }
    try {
        foreach ($bad in '{ invalid', '[]', 'null', '', '{"t":null}', '{"t":{"Reg":false,"Svc":[],"Task":[]}}') {
            Set-Content -LiteralPath $global:OptiStatePath -Value $bad -Encoding UTF8
            $before = [IO.File]::ReadAllText($global:OptiStatePath)
            $threw = $false
            try { [void](Invoke-TweakApply $t) } catch { $threw = $true }
            Assert $threw "unreadable state did not stop apply: [$bad]"
            Assert ((Get-ItemProperty -LiteralPath $TestKey).V -eq 7) 'registry changed without readable originals'
            Assert ([IO.File]::ReadAllText($global:OptiStatePath) -eq $before) 'unreadable state was overwritten'
        }
    } finally { Remove-FakeTweaks }
}
It 'corrupt feature backups block new profiles' {
    foreach ($bad in '{ invalid', '[]', '{"GameProfiles":false}', '{"GameProfiles":{"game.exe":{"Gpu":{},"Layers":{}}}}', '{"Wake":null}') {
        Set-Content -LiteralPath $global:OptiFeaturesPath -Value $bad -Encoding UTF8
        $threw = $false
        try { Add-GameProfile @((Join-Path $tmp 'game.exe')) } catch { $threw = $true }
        Assert $threw "corrupt feature backups were ignored: $bad"
        Assert ((Get-Content -LiteralPath $global:OptiFeaturesPath -Raw).Trim() -eq $bad) 'feature backups were overwritten'
    }
}
It 'registry verification compares all QWord bits and MultiString boundaries' {
    Assert (-not (Compare-RegValue 4294967297L 1L 'QWord')) 'QWord high bits ignored'
    Assert (Compare-RegValue 4294967297L 4294967297L 'QWord') 'equal QWords differ'
    Assert (Compare-RegValue -1 4294967295L 'DWord') 'unsigned DWord compatibility lost'
    Assert (-not (Compare-RegValue @('a b', 'c') @('a', 'b c') 'MultiString')) 'MultiString boundaries ignored'
    Assert (Compare-RegValue @('a', 'b') @('a', 'b') 'MultiString') 'equal MultiStrings differ'
}
It 'cleaner skips junctions and counts only deleted files' {
    $root = Join-Path $tmp 'clean'
    $outside = Join-Path $tmp 'outside'
    New-Item -ItemType Directory -Path "$root\nested", $outside -Force | Out-Null
    [IO.File]::WriteAllText("$root\nested\delete.txt", '12345')
    [IO.File]::WriteAllText("$outside\keep.txt", 'outside')
    $link = Join-Path $root 'nested\link'
    New-Item -ItemType Junction -Path $link -Target $outside -ErrorAction Stop | Out-Null
    try {
        Assert ((Get-PathSize $root) -eq 5) 'scan followed junction or miscounted files'
        Assert ((Clear-PathContents $root) -eq 5) 'cleaner did not count exact bytes removed'
        Assert (Test-Path -LiteralPath "$outside\keep.txt") 'cleaner deleted junction target contents'
        Assert ((Clear-PathContents $link) -eq 0) 'cleaner followed a junction root'
        Assert (Test-Path -LiteralPath "$outside\keep.txt") 'junction root changed target'
    } finally { [IO.Directory]::Delete($link) }
}
It 'service backup failure prevents all service changes' {
    $saved = Save-Functions 'Get-SvcSnapshot', 'Set-SvcStart'
    $old = $global:OptiBackupDir
    $global:ServiceChanges = 0
    function global:Get-SvcSnapshot { param($Name) @{ Name = $Name; Start = 3; Delayed = 0 } }
    function global:Set-SvcStart { param($Name, $Start, $Delayed) $global:ServiceChanges++; return $true }
    try {
        $global:OptiBackupDir = Join-Path $tmp 'missing\backup'
        $threw = $false
        try { Set-OptiServices -Names @('Fax') -Start 4 } catch { $threw = $true }
        Assert $threw 'backup failure was ignored'
        Assert ($global:ServiceChanges -eq 0) 'service changed without a backup'
    } finally { $global:OptiBackupDir = $old; Restore-Functions $saved }
}
It 'service restoration reports refusal and validates all records before changing any' {
    $saved = Save-Functions 'Set-SvcStart'
    $global:ServiceChanges = 0
    function global:Set-SvcStart { param($Name, $Start, $Delayed) $global:ServiceChanges++; return $false }
    $file = Join-Path $tmp 'service-restore.json'
    try {
        '[{"Name":"Fax","Start":3,"Delayed":0}]' | Set-Content -LiteralPath $file
        Assert (-not (Restore-OptiServices $file)) 'failed restoration reported success'
        Assert (Test-Path -LiteralPath $file) 'backup not kept after refusal'
        $global:ServiceChanges = 0
        '[{"Name":"Fax","Start":3,"Delayed":0},{"Name":"Other","Start":99,"Delayed":0}]' | Set-Content -LiteralPath $file
        $threw = $false
        try { [void](Restore-OptiServices $file) } catch { $threw = $true }
        Assert $threw 'invalid backup accepted'
        Assert ($global:ServiceChanges -eq 0) 'part of invalid backup was restored'
    } finally { Restore-Functions $saved }
}
It 'unverified and non-HTTPS updates are rejected before download' {
    Assert (-not (Install-OptiUpdate -Url 'https://example.invalid/update.zip' -Digest '' -Version '99.0' -InstallDir $tmp)) 'missing checksum accepted'
    Assert (-not (Install-OptiUpdate -Url 'http://example.invalid/update.zip' -Digest ('sha256:' + ('0' * 64)) -Version '99.0' -InstallDir $tmp)) 'non-HTTPS update accepted'
}
It 'WinGet failures are returned to unattended callers' {
    function global:Start-Process { param($FilePath, $ArgumentList, [switch]$NoNewWindow, [switch]$Wait, [switch]$PassThru, $RedirectStandardOutput, $RedirectStandardError, $ErrorAction) [pscustomobject]@{ ExitCode = 42 } }
    try {
        $r = Invoke-WingetPackages -Action Install -Programs @('Test.Package')
        Assert (-not $r.Success -and $r.Failed -contains 'Test.Package') 'package failure not returned'
        Assert ((Get-Content "$global:OptiRoot\Optimaxer.ps1" -Raw) -match 'if \(-not \$appResult.Success\) \{ \$code = 1 \}') 'silent runner ignores app failures'
    } finally { Remove-Item -LiteralPath 'Function:\Start-Process' }
}
It 'failed wake-disable retries retain existing restore records' {
    $saved = Save-Functions 'Invoke-Native'
    $f = Get-OptiFeatures
    $f.Wake.Devices = @('Test Device'); $f.Wake.Tasks = @('\Vendor\WakeTask')
    Save-OptiFeatures $f
    function global:Invoke-Native { param($Exe, $Arguments) $global:OptiExit = 1; return 'refused' }
    function global:Get-ScheduledTask { param($TaskPath, $TaskName, $ErrorAction) throw 'task inaccessible' }
    try {
        Disable-WakeSources -Devices @('Test Device') -Tasks @('\Vendor\WakeTask')
        $again = Get-OptiFeatures
        Assert ($again.Wake.Devices -contains 'Test Device') 'device retry discarded existing original'
        Assert ($again.Wake.Tasks -contains '\Vendor\WakeTask') 'task retry discarded existing original'
    } finally { Restore-Functions $saved; Remove-Item -LiteralPath 'Function:\Get-ScheduledTask' }
}
It 'update helper backs up existing files and stops before replacement when backup fails' {
    $ast = (Get-Command Install-OptiUpdate).ScriptBlock.Ast.Find({ param($n) $n -is [System.Management.Automation.Language.StringConstantExpressionAst] -and $n.Value.StartsWith('param($WaitPid, $Stage, $Dst, $Backup, $Relaunch)') }, $true)
    Assert ($null -ne $ast) 'update helper source not found'
    $helper = Join-Path $tmp 'update-helper.ps1'
    [IO.File]::WriteAllText($helper, $ast.Value)
    $stage = Join-Path $tmp 'update-stage'; $dst = Join-Path $tmp 'update-dst'; $backup = Join-Path $tmp 'update-backup'
    New-Item -ItemType Directory -Path $stage, $dst -Force | Out-Null
    [IO.File]::WriteAllText((Join-Path $stage 'app.txt'), 'new')
    [IO.File]::WriteAllText((Join-Path $dst 'app.txt'), 'original')
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $helper -WaitPid 2147483647 -Stage $stage -Dst $dst -Backup $backup -Relaunch 0
    Assert ($LASTEXITCODE -eq 0) 'update helper failed'
    Assert ([IO.File]::ReadAllText((Join-Path $backup 'app.txt')) -eq 'original') 'update lost previous files'
    Assert ([IO.File]::ReadAllText((Join-Path $dst 'app.txt')) -eq 'new') 'update did not replace files'
    $blocked = Join-Path $tmp 'blocked-backup'
    [IO.File]::WriteAllText($blocked, 'not a directory')
    [IO.File]::WriteAllText((Join-Path $stage 'app.txt'), 'should not install')
    try {
        $ErrorActionPreference = 'Continue'
        $failedOutput = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $helper -WaitPid 2147483647 -Stage $stage -Dst $dst -Backup $blocked -Relaunch 0 2>&1
        $failureCode = $LASTEXITCODE
    } finally { $ErrorActionPreference = 'Stop' }
    Assert ($failureCode -ne 0) 'helper ignored backup failure'
    Assert ([IO.File]::ReadAllText((Join-Path $dst 'app.txt')) -eq 'new') 'backup failure changed installed files'
}
It 'updater rejects archive traversal after a valid checksum' {
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $global:TestUpdateArchive = Join-Path $tmp 'unsafe-update.zip'
    $archive = [IO.Compression.ZipFile]::Open($global:TestUpdateArchive, [IO.Compression.ZipArchiveMode]::Create)
    try {
        $entry = $archive.CreateEntry('../escaped.txt')
        $writer = New-Object IO.StreamWriter ($entry.Open())
        try { $writer.Write('must not escape') } finally { $writer.Dispose() }
    } finally { $archive.Dispose() }
    $digest = 'sha256:' + (Get-FileHash -LiteralPath $global:TestUpdateArchive -Algorithm SHA256).Hash
    $global:CapturedUpdateDir = $null
    function global:Invoke-WebRequest {
        param($Uri, $Headers, $OutFile, [switch]$UseBasicParsing, $TimeoutSec, $ErrorAction)
        $global:CapturedUpdateDir = Split-Path $OutFile -Parent
        [IO.File]::Copy($global:TestUpdateArchive, $OutFile)
    }
    try {
        Assert (-not (Install-OptiUpdate -Url 'https://example.invalid/update.zip' -Digest $digest -Version '99.0' -InstallDir $tmp)) 'traversal archive accepted'
        Assert (-not (Test-Path -LiteralPath (Join-Path $global:CapturedUpdateDir 'escaped.txt'))) 'archive escaped extraction root'
    } finally {
        Remove-Item -LiteralPath 'Function:\Invoke-WebRequest'
        if ($global:CapturedUpdateDir) {
            $resolved = [IO.Path]::GetFullPath($global:CapturedUpdateDir)
            Assert ($resolved.StartsWith((Join-Path ([IO.Path]::GetTempPath()) 'optimaxer-update-'), [StringComparison]::OrdinalIgnoreCase)) 'unexpected updater test cleanup path'
            Remove-Item -LiteralPath $resolved -Recurse -Force
        }
    }
}
It 'Windows.old cleanup validates the system-drive target and never follows links' {
    $oldDrive = $env:SystemDrive
    $env:SystemDrive = Join-Path $tmp 'fake-system-drive'
    $root = Join-Path $env:SystemDrive 'Windows.old'
    $outside = Join-Path $tmp 'old-install-outside'
    New-Item -ItemType Directory -Path $root, $outside -Force | Out-Null
    [IO.File]::WriteAllText((Join-Path $root 'delete.txt'), '12345')
    [IO.File]::WriteAllText((Join-Path $outside 'keep.txt'), 'keep')
    $link = Join-Path $root 'link'
    New-Item -ItemType Junction -Path $link -Target $outside -ErrorAction Stop | Out-Null
    try {
        $threw = $false
        try { [void](Invoke-CleanupTarget @{ Special = 'winold'; Paths = @($outside) }) } catch { $threw = $true }
        Assert $threw 'Windows.old accepted an arbitrary directory'
        Assert ((Invoke-CleanupTarget @{ Special = 'winold'; Paths = @($root) }) -eq 5) 'Windows.old cleanup did not count only removed files'
        Assert (Test-Path -LiteralPath (Join-Path $outside 'keep.txt')) 'Windows.old cleanup followed a junction'
    } finally { [IO.Directory]::Delete($link); $env:SystemDrive = $oldDrive }
}
It 'Windows Update reset preserves prior service states and previous backups' {
    $tokens = $null; $errors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseFile((Join-Path $global:OptiRoot 'src\App.ps1'), [ref]$tokens, [ref]$errors)
    $assignment = $ast.Find({ param($n) $n -is [System.Management.Automation.Language.AssignmentStatementAst] -and $n.Left.Extent.Text -eq '$toolJobs' }, $true)
    $jobs = & ([scriptblock]::Create($assignment.Right.Extent.Text))
    $oldWindows = $env:WINDIR
    $env:WINDIR = Join-Path $tmp 'fake-windows'
    $global:StoppedServices = @(); $global:RestartedServices = @()
    function global:Get-Service { param($Name, $ErrorAction) [pscustomobject]@{ Status = $(if ($Name -eq 'bits') { 'Running' } else { 'Stopped' }) } }
    function global:Stop-Service { param($Name, [switch]$Force, $ErrorAction) $global:StoppedServices += $Name }
    function global:Start-Service { param($Name, $ErrorAction) $global:RestartedServices += $Name }
    try {
        New-Item -ItemType Directory -Path "$env:WINDIR\SoftwareDistribution", "$env:WINDIR\System32\catroot2", "$env:WINDIR\SoftwareDistribution.bak" -Force | Out-Null
        [IO.File]::WriteAllText("$env:WINDIR\SoftwareDistribution.bak\keep.txt", 'previous backup')
        & $jobs.wu
        Assert (($global:StoppedServices -join ',') -eq 'bits') 'reset stopped a previously stopped service'
        Assert (($global:RestartedServices -join ',') -eq 'bits') 'reset started a previously stopped service'
        Assert (Test-Path -LiteralPath "$env:WINDIR\SoftwareDistribution.bak\keep.txt") 'reset deleted a previous backup'
        Assert (-not (Test-Path -LiteralPath "$env:WINDIR\SoftwareDistribution")) 'cache directory was not moved'
    } finally {
        $env:WINDIR = $oldWindows
        Remove-Item -LiteralPath 'Function:\Get-Service', 'Function:\Stop-Service', 'Function:\Start-Service'
    }
}

Remove-Item -LiteralPath $TestKey -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
Write-Host ''
Write-Host ("{0} passed, {1} failed" -f $script:pass, $script:fail.Count)
if ($script:fail.Count) { exit 1 }
