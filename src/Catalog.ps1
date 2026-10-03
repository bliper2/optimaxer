# Optimaxer tweak catalog. Each tweak is declarative (registry / services / tasks) plus optional Apply/Undo/Test scripts.
# Apply may return a data object that is stored and passed back to Undo. Original values are snapshotted automatically.
# Tags: safe = Recommended preset, gaming, privacy, max = Maximum preset. Tweaks tagged 'optin' are never in presets.

$global:OptiCatalog = New-Object System.Collections.ArrayList

function Rg { param($p, $n, $v, $t = 'DWord') @{ P = $p; N = $n; V = $v; T = $t } }
function Sx { param($name, $start) @{ Name = $name; Start = $start } }

function T {
    param(
        [string]$Id, [string]$Name, [string]$Desc, [string]$Cat, [string]$Risk = 'Safe', [string[]]$Tags = @(),
        [object[]]$Reg = @(), [object[]]$Svc = @(), [string[]]$Task = @(),
        [scriptblock]$Apply, [scriptblock]$Undo, [scriptblock]$Test,
        [switch]$Restart, [switch]$Explorer, [switch]$DesktopOnly, [switch]$SSDOnly, [double]$SvcTol = 1.0
    )
    [void]$global:OptiCatalog.Add([pscustomobject]@{
        Id = $Id; Name = $Name; Desc = $Desc; Cat = $Cat; Risk = $Risk; Tags = $Tags
        Reg = $Reg; Svc = $Svc; Task = $Task; Apply = $Apply; Undo = $Undo; Test = $Test
        Restart = [bool]$Restart; Explorer = [bool]$Explorer; DesktopOnly = [bool]$DesktopOnly; SSDOnly = [bool]$SSDOnly; SvcTol = $SvcTol
    })
}

$CV   = 'HKCU:\Software\Microsoft\Windows\CurrentVersion'
$EXA  = "$CV\Explorer\Advanced"
$CDM  = "$CV\ContentDeliveryManager"
$PERS = "$CV\Themes\Personalize"
$POL  = 'HKLM:\SOFTWARE\Policies\Microsoft\Windows'
$DESK = 'HKCU:\Control Panel\Desktop'
$GCS  = 'HKCU:\System\GameConfigStore'
$MMP  = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Multimedia\SystemProfile'

# ======================================================================= PERFORMANCE
$P = 'Performance'
T -Id 'perf-ultimate' -Cat $P -Name 'Ultimate Performance power plan' -Tags safe, gaming, max -DesktopOnly `
  -Desc 'Activates the hidden Ultimate Performance plan: no CPU throttling, minimal latency. Desktops only; raises power draw.' `
  -Apply {
      $prev = Get-ActiveSchemeGuid
      $guid = Get-SchemeGuidByName 'Ultimate'
      $created = $null
      if (-not $guid) {
          $o = powercfg -duplicatescheme e9a42b02-d5df-448d-aa00-03f14749eb61 | Out-String
          if ($o -match '([0-9a-fA-F]{8}(-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12})') { $guid = $Matches[1]; $created = $guid }
      }
      if ($guid) { powercfg -setactive $guid | Out-Null }
      @{ Prev = $prev; Created = $created }
  } `
  -Undo {
      param($d)
      if ($d -and $d.Prev) { powercfg -setactive $d.Prev | Out-Null }
      if ($d -and $d.Created) { powercfg -delete $d.Created | Out-Null }
  } `
  -Test { (Get-SchemeName (Get-ActiveSchemeGuid)) -match 'Ultimate' }

T -Id 'perf-animations' -Cat $P -Name 'Faster UI: minimize animations and menu delay' -Tags safe, gaming, max -Explorer `
  -Desc 'Removes window minimize/maximize animation, taskbar animation and the menu hover delay. Snappier feel, zero risk.' `
  -Reg @(
      Rg $DESK 'MenuShowDelay' '0' 'String'
      Rg 'HKCU:\Control Panel\Desktop\WindowMetrics' 'MinAnimate' '0' 'String'
      Rg $EXA 'TaskbarAnimations' 0
      Rg $EXA 'ListviewAlphaSelect' 0
      Rg $EXA 'ListviewShadow' 0
      Rg 'HKCU:\Software\Microsoft\Windows\DWM' 'EnableAeroPeek' 0
  )

T -Id 'perf-visualfx' -Cat $P -Name 'Visual effects: best performance preset' -Tags max -Risk Moderate -Restart `
  -Desc 'Windows "Adjust for best performance" visual-effects mask. Flatter look; applies fully after sign-out.' `
  -Reg @(
      Rg "$CV\Explorer\VisualEffects" 'VisualFXSetting' 2
      Rg $DESK 'UserPreferencesMask' ([byte[]](0x90, 0x12, 0x03, 0x80, 0x10, 0, 0, 0)) 'Binary'
  )

T -Id 'perf-transparency' -Cat $P -Name 'Disable transparency effects' -Tags max `
  -Desc 'Turns off acrylic/mica transparency. Saves GPU work, especially on integrated graphics.' `
  -Reg @( Rg $PERS 'EnableTransparency' 0 )

T -Id 'perf-shutdown' -Cat $P -Name 'Faster shutdown and hung-app handling' -Tags safe, max `
  -Desc 'Shortens the time Windows waits for stuck services and apps before ending them (5s / 3s instead of 20s / 5s).' `
  -Reg @(
      Rg 'HKLM:\SYSTEM\CurrentControlSet\Control' 'WaitToKillServiceTimeout' '5000' 'String'
      Rg $DESK 'WaitToKillAppTimeout' '5000' 'String'
      Rg $DESK 'HungAppTimeout' '3000' 'String'
  )

T -Id 'perf-startupdelay' -Cat $P -Name 'Remove startup app delay' -Tags safe, max `
  -Desc 'Windows delays startup apps by ~10s after sign-in. This starts them immediately.' `
  -Reg @( Rg "$CV\Explorer\Serialize" 'StartupDelayInMSec' 0 )

T -Id 'perf-priosep' -Cat $P -Name 'Prioritize foreground apps' -Tags gaming, max -Risk Moderate `
  -Desc 'Win32PrioritySeparation = 0x26: short, fixed quanta with strong foreground boost. Better responsiveness for the app in front.' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\PriorityControl' 'Win32PrioritySeparation' 38 )

T -Id 'perf-powerthrottle' -Cat $P -Name 'Disable CPU power throttling' -Tags gaming, max -Risk Moderate -DesktopOnly `
  -Desc 'Stops Windows from throttling background-process CPU speed. Useful on desktops; costs battery on laptops.' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\Power\PowerThrottling' 'PowerThrottlingOff' 1 )

T -Id 'perf-fastboot' -Cat $P -Name 'Disable Fast Startup' -Tags max -Restart `
  -Desc 'Fast Startup is a hybrid hibernate that keeps stale driver state. Disabling gives true clean boots and fixes many odd issues.' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Power' 'HiberbootEnabled' 0 )

T -Id 'perf-hibernate' -Cat $P -Name 'Disable hibernation (frees disk space)' -Tags max -Risk Moderate -DesktopOnly `
  -Desc 'Deletes hiberfil.sys (about 40% of RAM size). Do not use if you rely on hibernate.' `
  -Apply { powercfg /hibernate off | Out-Null; $null } `
  -Undo  { powercfg /hibernate on | Out-Null } `
  -Test  { (Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\Power' -ErrorAction SilentlyContinue).HibernateEnabled -eq 0 }

T -Id 'perf-ntfs' -Cat $P -Name 'NTFS: stop last-access timestamp writes' -Tags safe, max `
  -Desc 'Avoids a disk write for every file read. Reduces I/O and SSD wear.' `
  -Apply { $o = Invoke-Native fsutil @('behavior', 'query', 'disablelastaccess'); $prev = if ($o -match '=\s*(\d)') { [int]$Matches[1] } else { 2 }; fsutil behavior set disablelastaccess 1 | Out-Null; @{ Prev = $prev } } `
  -Undo  { param($d) $v = if ($d -and $null -ne $d.Prev) { $d.Prev } else { 2 }; fsutil behavior set disablelastaccess $v | Out-Null } `
  -Test  { (Invoke-Native fsutil @('behavior', 'query', 'disablelastaccess')) -match '=\s*[13]' }

T -Id 'perf-memcompress' -Cat $P -Name 'Disable memory compression (16 GB+ RAM)' -Tags max -Risk Moderate -Restart `
  -Desc 'Saves CPU cycles spent compressing RAM. Only helps if you have plenty of memory.' `
  -Apply { Disable-MMAgent -MemoryCompression -ErrorAction Stop | Out-Null; $null } `
  -Undo  { Enable-MMAgent -MemoryCompression -ErrorAction Stop | Out-Null } `
  -Test  { -not (Get-MMAgent -ErrorAction Stop).MemoryCompression }

T -Id 'perf-prefetch' -Cat $P -Name 'Disable Prefetch and SysMain (SSD only)' -Tags max -Risk Moderate -SSDOnly `
  -Desc 'Superfetch preloads apps into RAM, pointless on fast SSDs and a source of background disk activity.' `
  -Reg @(
      Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Memory Management\PrefetchParameters' 'EnablePrefetcher' 0
      Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Memory Management\PrefetchParameters' 'EnableSuperfetch' 0
  ) -Svc @( Sx 'SysMain' 4 )

T -Id 'perf-searchindex' -Cat $P -Name 'Disable Windows Search indexing' -Tags optin -Risk Advanced `
  -Desc 'Stops the indexer. Start-menu and Explorer searches become slower but background disk/CPU use drops.' `
  -Svc @( Sx 'WSearch' 4 )

T -Id 'perf-cpupower' -Cat $P -Name 'CPU: no core parking, always full speed (plugged in)' -Tags gaming, max -Risk Moderate -DesktopOnly `
  -Desc 'Min processor state 100% and all cores unparked on AC power. Lowest latency; higher idle power and heat.' `
  -Apply { Set-PowerAC @(
        @{ Sub = '54533251-82be-4824-96c1-47b60b740d00'; Set = '0cc5b647-c1df-4637-891a-dec35c318583'; Val = 100 }
        @{ Sub = '54533251-82be-4824-96c1-47b60b740d00'; Set = 'bc5038f7-23e0-4960-96da-33abaf5935ec'; Val = 100 }) } `
  -Undo  { param($d) if ($d) { Set-PowerAC $d | Out-Null } } `
  -Test  { Test-PowerAC @(
        @{ Sub = '54533251-82be-4824-96c1-47b60b740d00'; Set = '0cc5b647-c1df-4637-891a-dec35c318583'; Val = 100 }
        @{ Sub = '54533251-82be-4824-96c1-47b60b740d00'; Set = 'bc5038f7-23e0-4960-96da-33abaf5935ec'; Val = 100 }) }

T -Id 'perf-usbpcie' -Cat $P -Name 'No USB selective suspend / PCIe link power saving' -Tags gaming, max -DesktopOnly `
  -Desc 'Prevents devices and GPU links from sleeping mid-use: fixes USB stutter/disconnects and PCIe latency spikes.' `
  -Apply { Set-PowerAC @(
        @{ Sub = '2a737441-1930-4402-8d77-b2bebba308a3'; Set = '48e6b7a6-50f5-4782-a5d4-53bb8f07e226'; Val = 0 }
        @{ Sub = '501a4d13-42af-4429-9fd1-a8218c268e20'; Set = 'ee12f906-d277-404b-b6da-e5fa1a576df5'; Val = 0 }) } `
  -Undo  { param($d) if ($d) { Set-PowerAC $d | Out-Null } } `
  -Test  { Test-PowerAC @(
        @{ Sub = '2a737441-1930-4402-8d77-b2bebba308a3'; Set = '48e6b7a6-50f5-4782-a5d4-53bb8f07e226'; Val = 0 }
        @{ Sub = '501a4d13-42af-4429-9fd1-a8218c268e20'; Set = 'ee12f906-d277-404b-b6da-e5fa1a576df5'; Val = 0 }) }

T -Id 'perf-timer' -Cat $P -Name 'Low-latency timer (disable dynamic tick)' -Tags gaming, max -Risk Advanced -DesktopOnly -Restart `
  -Desc 'bcdedit disabledynamictick=yes and removes forced platform clock. Lower input latency; slightly higher idle power.' `
  -Apply { $a = bcdedit /enum '{current}' | Out-String; bcdedit /set disabledynamictick yes | Out-Null; bcdedit /deletevalue useplatformclock 2>&1 | Out-Null; @{ HadPlatformClock = [bool]($a -match 'useplatformclock\s+Yes') } } `
  -Undo  { param($d) bcdedit /deletevalue disabledynamictick 2>&1 | Out-Null; if ($d -and $d.HadPlatformClock) { bcdedit /set useplatformclock yes | Out-Null } } `
  -Test  { (bcdedit /enum '{current}' | Out-String) -match 'disabledynamictick\s+Yes' }

T -Id 'perf-hags' -Cat $P -Name 'Hardware-accelerated GPU scheduling' -Tags gaming, max -Risk Moderate -Restart `
  -Desc 'Lets the GPU manage its own VRAM scheduling. Lower latency on modern NVIDIA/AMD GPUs (needs recent drivers).' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\GraphicsDrivers' 'HwSchMode' 2 )

T -Id 'perf-vbs' -Cat $P -Name 'Disable Memory Integrity (VBS/HVCI)' -Tags optin -Risk Advanced -Restart `
  -Desc 'WARNING: lowers kernel security. Can give 5-10% FPS in some games. Only for dedicated gaming rigs.' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\DeviceGuard\Scenarios\HypervisorEnforcedCodeIntegrity' 'Enabled' 0 )

# ======================================================================= GAMING
$G = 'Gaming'
T -Id 'game-mode' -Cat $G -Name 'Enable Game Mode' -Tags safe, gaming, max `
  -Desc 'Prioritizes the running game and suppresses driver/Windows-Update interruptions.' `
  -Reg @( Rg 'HKCU:\Software\Microsoft\GameBar' 'AutoGameModeEnabled' 1; Rg 'HKCU:\Software\Microsoft\GameBar' 'AllowAutoGameMode' 1 )

T -Id 'game-dvr' -Cat $G -Name 'Disable Game DVR and background capture' -Tags safe, gaming, max `
  -Desc 'Stops Xbox Game Bar background recording that costs FPS. Game Bar overlay shortcuts stop working.' `
  -Reg @(
      Rg $GCS 'GameDVR_Enabled' 0
      Rg "$CV\GameDVR" 'AppCaptureEnabled' 0
      Rg "$POL\GameDVR" 'AllowGameDVR' 0
      Rg 'HKCU:\Software\Microsoft\GameBar' 'UseNexusForGameBarEnabled' 0
      Rg 'HKCU:\Software\Microsoft\GameBar' 'ShowStartupPanel' 0
  )

T -Id 'game-fso' -Cat $G -Name 'Disable fullscreen optimizations globally' -Tags gaming -Risk Moderate `
  -Desc 'Forces true exclusive fullscreen behaviour. Helps some older titles; a few modern games prefer it on.' `
  -Reg @(
      Rg $GCS 'GameDVR_FSEBehaviorMode' 2
      Rg $GCS 'GameDVR_HonorUserFSEBehaviorMode' 1
      Rg $GCS 'GameDVR_FSEBehavior' 2
      Rg $GCS 'GameDVR_DXGIHonorFSEWindowsCompatible' 1
  )

T -Id 'game-windowed' -Cat $G -Name 'Optimizations for windowed games' -Tags safe, gaming, max `
  -Desc 'Enables flip-model presentation for borderless/windowed DirectX 10/11 games: lower latency, VRR support.' `
  -Reg @( Rg 'HKCU:\Software\Microsoft\DirectX\UserGpuPreferences' 'DirectXUserGlobalSettings' 'SwapEffectUpgradeEnable=1;' 'String' )

T -Id 'game-mmcss' -Cat $G -Name 'Multimedia scheduler: favor games and remove network throttle' -Tags safe, gaming, max `
  -Desc 'MMCSS Games task gets high GPU/CPU priority and network throttling is lifted. Standard low-latency setup.' `
  -Reg @(
      Rg $MMP 'NetworkThrottlingIndex' 4294967295
      Rg $MMP 'SystemResponsiveness' 10
      Rg "$MMP\Tasks\Games" 'GPU Priority' 8
      Rg "$MMP\Tasks\Games" 'Priority' 6
      Rg "$MMP\Tasks\Games" 'Scheduling Category' 'High' 'String'
      Rg "$MMP\Tasks\Games" 'SFIO Priority' 'High' 'String'
  )

T -Id 'game-mouse' -Cat $G -Name 'Disable mouse acceleration (enhance pointer precision)' -Tags gaming `
  -Desc '1:1 mouse movement for consistent aim. Takes effect after sign-out.' `
  -Reg @( Rg 'HKCU:\Control Panel\Mouse' 'MouseSpeed' '0' 'String'; Rg 'HKCU:\Control Panel\Mouse' 'MouseThreshold1' '0' 'String'; Rg 'HKCU:\Control Panel\Mouse' 'MouseThreshold2' '0' 'String' )

T -Id 'game-sticky' -Cat $G -Name 'Disable Sticky/Filter/Toggle Keys popups' -Tags safe, gaming, max `
  -Desc 'No more accessibility popup when you tap Shift five times in a game.' `
  -Reg @(
      Rg 'HKCU:\Control Panel\Accessibility\StickyKeys' 'Flags' '58' 'String'
      Rg 'HKCU:\Control Panel\Accessibility\Keyboard Response' 'Flags' '122' 'String'
      Rg 'HKCU:\Control Panel\Accessibility\ToggleKeys' 'Flags' '58' 'String'
  )

# ======================================================================= NETWORK
$N = 'Network'
T -Id 'net-nagle' -Cat $N -Name 'Disable Nagle algorithm (lower online-game latency)' -Tags gaming, max -Risk Moderate `
  -Desc 'Sets TcpAckFrequency=1 and TCPNoDelay=1 on active interfaces so small packets are sent immediately.' `
  -Apply {
      $done = @()
      $base = 'HKLM:\SYSTEM\CurrentControlSet\Services\Tcpip\Parameters\Interfaces'
      foreach ($k in Get-ChildItem $base -ErrorAction SilentlyContinue) {
          $p = Get-ItemProperty -LiteralPath $k.PSPath
          if ($p.DhcpIPAddress -or $p.IPAddress) {
              Set-ItemProperty -LiteralPath $k.PSPath -Name TcpAckFrequency -Value 1 -Type DWord
              Set-ItemProperty -LiteralPath $k.PSPath -Name TCPNoDelay -Value 1 -Type DWord
              $done += $k.PSChildName
          }
      }
      , $done
  } `
  -Undo {
      param($d)
      foreach ($i in @($d)) {
          $k = "HKLM:\SYSTEM\CurrentControlSet\Services\Tcpip\Parameters\Interfaces\$i"
          Remove-ItemProperty -LiteralPath $k -Name TcpAckFrequency, TCPNoDelay -ErrorAction SilentlyContinue
      }
  }

T -Id 'net-teredo' -Cat $N -Name 'Disable Teredo tunneling' -Tags max `
  -Desc 'Removes an IPv6-over-IPv4 tunnel that adds latency and is rarely needed.' `
  -Apply { netsh interface teredo set state disabled | Out-Null; $null } `
  -Undo  { netsh interface teredo set state default | Out-Null }

T -Id 'net-ipv4pref' -Cat $N -Name 'Prefer IPv4 over IPv6' -Tags max `
  -Desc 'Keeps IPv6 enabled but tries IPv4 first. Avoids slow connects on networks with broken IPv6.' -Restart `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Services\Tcpip6\Parameters' 'DisabledComponents' 32 )

T -Id 'net-llmnr' -Cat $N -Name 'Disable LLMNR name resolution' -Tags privacy, max `
  -Desc 'Closes a spoofing-prone legacy name-resolution protocol. Regular DNS is unaffected.' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Windows NT\DNSClient' 'EnableMulticast' 0 )

T -Id 'net-do' -Cat $N -Name 'Delivery Optimization: no peer-to-peer uploads' -Tags safe, privacy, max `
  -Desc 'Windows Update downloads only from Microsoft; your PC stops uploading updates to strangers.' `
  -Reg @(
      Rg "$POL\DeliveryOptimization" 'DODownloadMode' 0
      Rg 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\DeliveryOptimization\Config' 'DODownloadMode' 0
  )

T -Id 'net-smb1' -Cat $N -Name 'Disable SMBv1 server' -Tags safe `
  -Desc 'Security hardening: SMBv1 is the protocol exploited by WannaCry.' `
  -Apply { Set-SmbServerConfiguration -EnableSMB1Protocol $false -Force -Confirm:$false | Out-Null; $null } `
  -Undo  { Set-SmbServerConfiguration -EnableSMB1Protocol $true -Force -Confirm:$false | Out-Null } `
  -Test  { (Get-SmbServerConfiguration).EnableSMB1Protocol -eq $false }

# ======================================================================= PRIVACY
$V = 'Privacy & Telemetry'
T -Id 'priv-telemetry' -Cat $V -Name 'Disable telemetry and diagnostic data' -Tags safe, privacy, max `
  -Desc 'Sets telemetry to the minimum, stops DiagTrack and disables Microsoft data-collection tasks.' `
  -Reg @(
      Rg "$POL\DataCollection" 'AllowTelemetry' 0
      Rg 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\DataCollection' 'AllowTelemetry' 0
      Rg "$POL\DataCollection" 'DoNotShowFeedbackNotifications' 1
      Rg "$POL\DataCollection" 'DisableOneSettingsDownloads' 1
  ) -Svc @( Sx 'DiagTrack' 4; Sx 'dmwappushservice' 4 ) `
  -Task @(
      'Microsoft\Windows\Application Experience\Microsoft Compatibility Appraiser'
      'Microsoft\Windows\Application Experience\ProgramDataUpdater'
      'Microsoft\Windows\Application Experience\StartupAppTask'
      'Microsoft\Windows\Customer Experience Improvement Program\Consolidator'
      'Microsoft\Windows\Customer Experience Improvement Program\KernelCeipTask'
      'Microsoft\Windows\Customer Experience Improvement Program\UsbCeip'
      'Microsoft\Windows\DiskDiagnostic\Microsoft-Windows-DiskDiagnosticDataCollector'
      'Microsoft\Windows\Feedback\Siuf\DmClient'
      'Microsoft\Windows\Feedback\Siuf\DmClientOnScenarioDownload'
  )

T -Id 'priv-activity' -Cat $V -Name 'Disable activity history and timeline' -Tags safe, privacy, max `
  -Desc 'Stops Windows from recording and uploading your app/file activity. Clipboard history is unaffected.' `
  -Reg @(
      Rg "$POL\System" 'EnableActivityFeed' 0
      Rg "$POL\System" 'PublishUserActivities' 0
      Rg "$POL\System" 'UploadUserActivities' 0
  )

T -Id 'priv-adid' -Cat $V -Name 'Disable advertising ID and tailored ads' -Tags safe, privacy, max `
  -Desc 'Removes the per-user advertising identifier and diagnostic-data-based personalization.' `
  -Reg @(
      Rg "$CV\AdvertisingInfo" 'Enabled' 0
      Rg "$POL\AdvertisingInfo" 'DisabledByGroupPolicy' 1
      Rg "$CV\Privacy" 'TailoredExperiencesWithDiagnosticDataEnabled' 0
      Rg 'HKCU:\Control Panel\International\User Profile' 'HttpAcceptLanguageOptOut' 1
  )

T -Id 'priv-consumer' -Cat $V -Name 'Disable Start/lock-screen ads and suggested apps' -Tags safe, privacy, max `
  -Desc 'Stops consumer features, silent app installs, Start suggestions, tips and "finish setting up" nags.' `
  -Reg @(
      Rg "$POL\CloudContent" 'DisableWindowsConsumerFeatures' 1
      Rg "$POL\CloudContent" 'DisableSoftLanding' 1
      Rg "$POL\CloudContent" 'DisableCloudOptimizedContent' 1
      Rg $CDM 'ContentDeliveryAllowed' 0
      Rg $CDM 'OemPreInstalledAppsEnabled' 0
      Rg $CDM 'PreInstalledAppsEnabled' 0
      Rg $CDM 'PreInstalledAppsEverEnabled' 0
      Rg $CDM 'SilentInstalledAppsEnabled' 0
      Rg $CDM 'SystemPaneSuggestionsEnabled' 0
      Rg $CDM 'SoftLandingEnabled' 0
      Rg $CDM 'FeatureManagementEnabled' 0
      Rg $CDM 'RotatingLockScreenEnabled' 0
      Rg $CDM 'RotatingLockScreenOverlayEnabled' 0
      Rg $CDM 'SubscribedContent-310093Enabled' 0
      Rg $CDM 'SubscribedContent-338387Enabled' 0
      Rg $CDM 'SubscribedContent-338388Enabled' 0
      Rg $CDM 'SubscribedContent-338389Enabled' 0
      Rg $CDM 'SubscribedContent-338393Enabled' 0
      Rg $CDM 'SubscribedContent-353694Enabled' 0
      Rg $CDM 'SubscribedContent-353696Enabled' 0
      Rg $CDM 'SubscribedContent-88000326Enabled' 0
      Rg "$CV\UserProfileEngagement" 'ScoobeSystemSettingEnabled' 0
  )

T -Id 'priv-websearch' -Cat $V -Name 'Disable Bing web results in Start search' -Tags safe, privacy, max `
  -Desc 'Start search only searches your PC. Faster and nothing you type goes to Bing.' `
  -Reg @(
      Rg "$CV\Search" 'BingSearchEnabled' 0
      Rg "$CV\Search" 'CortanaConsent' 0
      Rg 'HKCU:\SOFTWARE\Policies\Microsoft\Windows\Explorer' 'DisableSearchBoxSuggestions' 1
  )

T -Id 'priv-ai' -Cat $V -Name 'Disable Copilot, Recall and Click to Do' -Tags safe, privacy, max `
  -Desc 'Turns off the Windows AI features and the Copilot taskbar button via policy.' `
  -Reg @(
      Rg 'HKCU:\Software\Policies\Microsoft\Windows\WindowsCopilot' 'TurnOffWindowsCopilot' 1
      Rg "$POL\WindowsCopilot" 'TurnOffWindowsCopilot' 1
      Rg "$POL\WindowsAI" 'DisableAIDataAnalysis' 1
      Rg "$POL\WindowsAI" 'DisableClickToDo' 1
      Rg $EXA 'ShowCopilotButton' 0
  )

T -Id 'priv-location' -Cat $V -Name 'Disable location tracking' -Tags privacy -Risk Moderate `
  -Desc 'Denies system-wide location access. Maps and Find My Device will not work.' `
  -Reg @(
      Rg 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\location' 'Value' 'Deny' 'String'
      Rg "$POL\LocationAndSensors" 'DisableLocation' 1
      Rg 'HKLM:\SYSTEM\Maps' 'AutoUpdateEnabled' 0
  )

T -Id 'priv-input' -Cat $V -Name 'Disable typing, inking and speech data collection' -Tags safe, privacy, max `
  -Desc 'Stops Windows from harvesting what you type, ink and say to personalize services.' `
  -Reg @(
      Rg 'HKCU:\Software\Microsoft\InputPersonalization' 'RestrictImplicitTextCollection' 1
      Rg 'HKCU:\Software\Microsoft\InputPersonalization' 'RestrictImplicitInkCollection' 1
      Rg 'HKCU:\Software\Microsoft\InputPersonalization\TrainedDataStore' 'HarvestContacts' 0
      Rg 'HKCU:\Software\Microsoft\Personalization\Settings' 'AcceptedPrivacyPolicy' 0
      Rg 'HKCU:\Software\Microsoft\Input\TIPC' 'Enabled' 0
  )

T -Id 'priv-feedback' -Cat $V -Name 'Disable feedback prompts and app-launch tracking' -Tags safe, privacy, max `
  -Desc 'No "How likely are you to recommend Windows" popups; Start stops tracking launched apps.' `
  -Reg @( Rg 'HKCU:\Software\Microsoft\Siuf\Rules' 'NumberOfSIUFInPeriod' 0; Rg $EXA 'Start_TrackProgs' 0 )

T -Id 'priv-wer' -Cat $V -Name 'Disable Windows Error Reporting' -Tags privacy, max -Risk Moderate `
  -Desc 'Crash reports are no longer sent to Microsoft. Local crash dialogs remain.' `
  -Reg @(
      Rg 'HKLM:\SOFTWARE\Microsoft\Windows\Windows Error Reporting' 'Disabled' 1
      Rg "$POL\Windows Error Reporting" 'Disabled' 1
  ) -Svc @( Sx 'WerSvc' 4 ) -Task @( 'Microsoft\Windows\Windows Error Reporting\QueueReporting' )

T -Id 'priv-clipboard' -Cat $V -Name 'Disable clipboard cloud sync' -Tags privacy `
  -Desc 'Clipboard history stays local; nothing is synced to your Microsoft account.' `
  -Reg @( Rg "$POL\System" 'AllowCrossDeviceClipboard' 0 )

T -Id 'priv-remoteassist' -Cat $V -Name 'Disable Remote Assistance' -Tags safe, privacy, max `
  -Desc 'Blocks inbound Remote Assistance invitations. Reduces attack surface.' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\Remote Assistance' 'fAllowToGetHelp' 0; Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\Remote Assistance' 'fAllowFullControl' 0 )

# ======================================================================= DEBLOAT
$D = 'Debloat'
T -Id 'debloat-edge' -Cat $D -Name 'Edge: disable nags, telemetry and background mode' -Tags safe, privacy, max `
  -Desc 'Edge policies: no startup boost, no background running, no shopping/rewards/recommendation clutter, no diagnostic data.' `
  -Reg @(
      Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Edge' 'StartupBoostEnabled' 0
      Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Edge' 'BackgroundModeEnabled' 0
      Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Edge' 'HideFirstRunExperience' 1
      Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Edge' 'PersonalizationReportingEnabled' 0
      Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Edge' 'ShowRecommendationsEnabled' 0
      Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Edge' 'UserFeedbackAllowed' 0
      Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Edge' 'DiagnosticData' 0
      Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Edge' 'EdgeShoppingAssistantEnabled' 0
      Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Edge' 'EdgeCollectionsEnabled' 0
      Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Edge' 'EdgeFollowEnabled' 0
      Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Edge' 'ShowMicrosoftRewards' 0
      Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Edge' 'WebWidgetAllowed' 0
      Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Edge' 'MicrosoftEdgeInsiderPromotionEnabled' 0
      Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Edge' 'SpotlightExperiencesAndRecommendationsEnabled' 0
  )

T -Id 'debloat-widgets' -Cat $D -Name 'Remove Widgets and Chat taskbar buttons' -Tags safe, max -Explorer `
  -Desc 'Hides the Widgets board and Teams Chat, and stops the news feed.' `
  -Reg @( Rg $EXA 'TaskbarDa' 0; Rg $EXA 'TaskbarMn' 0; Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Dsh' 'AllowNewsAndInterests' 0 )

T -Id 'debloat-onedrive' -Cat $D -Name 'Uninstall OneDrive' -Tags optin -Risk Advanced `
  -Desc 'Removes the OneDrive client. Make sure your files are not only in the cloud first. Undo reinstalls it.' `
  -Reg @( Rg "$POL\OneDrive" 'DisableFileSyncNGSC' 1 ) `
  -Apply {
      Stop-Process -Name OneDrive -Force -ErrorAction SilentlyContinue
      $exe = "$env:SystemRoot\SysWOW64\OneDriveSetup.exe"; if (-not (Test-Path $exe)) { $exe = "$env:SystemRoot\System32\OneDriveSetup.exe" }
      if (Test-Path $exe) { Start-Process $exe '/uninstall' -Wait }
      $null
  } `
  -Undo {
      $exe = "$env:SystemRoot\SysWOW64\OneDriveSetup.exe"; if (-not (Test-Path $exe)) { $exe = "$env:SystemRoot\System32\OneDriveSetup.exe" }
      if (Test-Path $exe) { Start-Process $exe -Wait }
  } `
  -Test {
      -not ((Test-Path "$env:LOCALAPPDATA\Microsoft\OneDrive\OneDrive.exe") -or (Test-Path "$env:ProgramFiles\Microsoft OneDrive\OneDrive.exe") -or (Test-Path "${env:ProgramFiles(x86)}\Microsoft OneDrive\OneDrive.exe"))
  }

T -Id 'svc-manual' -Cat $D -Name 'Services: set unneeded services to Manual' -Tags safe, max -SvcTol 0.9 `
  -Desc 'Moves ~80 rarely-needed services (fax, remote registry helpers, peer networking, etc.) to Manual. They still start if something asks.' `
  -Svc @( $global:OptiServiceRecs | Where-Object { $_.Start -eq 3 } | ForEach-Object { Sx $_.Name 3 } )

T -Id 'svc-disable' -Cat $D -Name 'Services: disable truly useless services' -Tags max -Risk Moderate -SvcTol 0.9 `
  -Desc 'Disables Fax, Remote Registry, Retail Demo, SNMP trap, Maps broker, Media Player sharing and similar.' `
  -Svc @( $global:OptiServiceRecs | Where-Object { $_.Start -eq 4 } | ForEach-Object { Sx $_.Name 4 } )

T -Id 'sto-reserved' -Cat $D -Name 'Disable Reserved Storage' -Tags optin -Risk Moderate `
  -Desc 'Gives back about 7 GB that Windows reserves for updates. Updates may fail on a nearly full disk.' `
  -Apply { Dism.exe /Online /Set-ReservedStorageState /State:Disabled | Out-Null; $null } `
  -Undo  { Dism.exe /Online /Set-ReservedStorageState /State:Enabled | Out-Null }

# ======================================================================= INTERFACE
$I = 'Interface & Explorer'
T -Id 'ui-ext' -Cat $I -Name 'Show file extensions' -Tags safe -Explorer -Desc 'Always show .exe, .pdf, .zip... A basic security habit.' `
  -Reg @( Rg $EXA 'HideFileExt' 0 )
T -Id 'ui-hidden' -Cat $I -Name 'Show hidden files' -Explorer -Desc 'Show hidden files and folders in Explorer.' `
  -Reg @( Rg $EXA 'Hidden' 1 )
T -Id 'ui-thispc' -Cat $I -Name 'Open Explorer to This PC' -Explorer -Desc 'Instead of Home/Quick access.' `
  -Reg @( Rg $EXA 'LaunchTo' 1 )
T -Id 'ui-nohome' -Cat $I -Name 'Hide Home and Gallery in Explorer' -Explorer -Desc 'Removes the Home and Gallery entries from the navigation pane.' `
  -Reg @(
      Rg 'HKLM:\SOFTWARE\Classes\CLSID\{e88865ea-0e1c-4e20-9aa6-edcd0212c87c}' 'System.IsPinnedToNameSpaceTree' 0
      Rg 'HKLM:\SOFTWARE\Classes\CLSID\{f874310e-b6b7-47dc-bc84-b9e6b38f5903}' 'System.IsPinnedToNameSpaceTree' 0
      Rg $EXA 'LaunchTo' 1
  )
T -Id 'ui-classicmenu' -Cat $I -Name 'Classic right-click menu (Windows 11)' -Explorer -Risk Safe `
  -Desc 'Brings back the full context menu without "Show more options".' `
  -Apply { New-Item -Path 'HKCU:\Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}\InprocServer32' -Force -Value '' | Out-Null; $null } `
  -Undo  { Remove-Item -Path 'HKCU:\Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}' -Recurse -Force -ErrorAction SilentlyContinue } `
  -Test  { Test-Path 'HKCU:\Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}\InprocServer32' }
T -Id 'ui-dark' -Cat $I -Name 'Dark mode for apps and system' -Desc 'Dark theme for Windows and apps.' `
  -Reg @( Rg $PERS 'AppsUseLightTheme' 0; Rg $PERS 'SystemUsesLightTheme' 0 )
T -Id 'ui-taskbarleft' -Cat $I -Name 'Left-aligned taskbar' -Explorer -Desc 'Classic left taskbar alignment (Windows 11).' `
  -Reg @( Rg $EXA 'TaskbarAl' 0 )
T -Id 'ui-searchicon' -Cat $I -Name 'Hide taskbar search box' -Explorer -Desc 'Removes the search box from the taskbar (Win+S still works).' `
  -Reg @( Rg "$CV\Search" 'SearchboxTaskbarMode' 0 )
T -Id 'ui-taskview' -Cat $I -Name 'Hide Task View button' -Explorer -Desc 'Removes the Task View button from the taskbar.' `
  -Reg @( Rg $EXA 'ShowTaskViewButton' 0 )
T -Id 'ui-endtask' -Cat $I -Name 'End Task from taskbar right-click' -Tags safe -Explorer -Desc 'Adds "End task" to the taskbar app context menu.' `
  -Reg @( Rg "$EXA\TaskbarDeveloperSettings" 'TaskbarEndTask' 1 )
T -Id 'ui-startrecs' -Cat $I -Name 'Start menu: hide recommendations' -Explorer -Desc 'Hides the Recommended tips/shortcuts and account notifications in Start.' `
  -Reg @( Rg $EXA 'Start_IrisRecommendations' 0; Rg $EXA 'Start_AccountNotifications' 0 )
T -Id 'ui-numlock' -Cat $I -Name 'Num Lock on at startup' -Desc 'Turn Num Lock on at sign-in.' `
  -Reg @( Rg 'HKCU:\Control Panel\Keyboard' 'InitialKeyboardIndicators' '2' 'String'; Rg 'HKU:\.DEFAULT\Control Panel\Keyboard' 'InitialKeyboardIndicators' '2' 'String' )
T -Id 'ui-longpaths' -Cat $I -Name 'Enable long file paths' -Tags safe -Desc 'Lifts the 260-character path limit for apps that support it.' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem' 'LongPathsEnabled' 1 )
T -Id 'ui-nolock' -Cat $I -Name 'Disable lock screen' -Desc 'Skips the lock screen picture and goes straight to sign-in.' `
  -Reg @( Rg "$POL\Personalization" 'NoLockScreen' 1 )
T -Id 'ui-scrollbars' -Cat $I -Name 'Scrollbars always visible' -Desc 'Stops scrollbars hiding automatically.' `
  -Reg @( Rg 'HKCU:\Control Panel\Accessibility' 'DynamicScrollbars' 0 )
T -Id 'ui-verbosebsod' -Cat $I -Name 'Verbose blue-screen details' -Desc 'Shows technical parameters on BSOD for troubleshooting.' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\CrashControl' 'DisplayParameters' 1 )
