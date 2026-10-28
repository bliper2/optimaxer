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

Remove-Item -LiteralPath $TestKey -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
Write-Host ''
Write-Host ("{0} passed, {1} failed" -f $script:pass, $script:fail.Count)
if ($script:fail.Count) { exit 1 }
