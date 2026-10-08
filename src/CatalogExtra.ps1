# Additional tweaks (system tuning, shell latency, telemetry, update behavior). Same schema as Catalog.ps1.
# The ideas follow the kinds of changes shipped by tuned Windows builds such as AtlasOS; every tweak here is
# implemented independently with snapshot/undo and honest detection.

$global:SHELLBAGS = 'HKCU:\SOFTWARE\Classes\Local Settings\Software\Microsoft\Windows\Shell'
$global:SVCROOT   = 'HKLM:\SYSTEM\CurrentControlSet\Services'
$WU        = 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate'

# ======================================================================= PERFORMANCE
T -Id 'perf-fth' -Cat $P -Name 'Disable Fault Tolerant Heap' -Tags safe, max -Risk Moderate `
  -Desc 'FTH applies slow-path mitigations to apps that crashed a few times and keeps them there. Resets its list and turns it off.' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Microsoft\FTH' 'Enabled' 0 ) `
  -Apply { rundll32.exe fthsvc.dll,FthSysprepSpecialize; Set-RegEntry 'HKLM:\SOFTWARE\Microsoft\FTH' 'Enabled' 0 'DWord'; $null }

T -Id 'perf-bgapps' -Cat $P -Name 'Disable background apps' -Tags safe, max `
  -Desc 'Store apps no longer run in the background. Less CPU, RAM and network use; notifications from those apps stop.' `
  -Reg @( Rg "$CV\BackgroundAccessApplications" 'GlobalUserDisabled' 1; Rg "$CV\Search" 'BackgroundAppGlobalToggle' 0 )

T -Id 'perf-sleepstudy' -Cat $P -Name 'Disable SleepStudy and power diagnostic logging' -Tags safe, max `
  -Desc 'Turns off Modern Standby / power diagnostic event logs and the Power Efficiency Diagnostics task. Less background disk writes.' `
  -Task @( 'Microsoft\Windows\Power Efficiency Diagnostics\AnalyzeSystem' ) `
  -Apply {
      foreach ($l in 'Microsoft-Windows-SleepStudy/Diagnostic', 'Microsoft-Windows-Kernel-Processor-Power/Diagnostic', 'Microsoft-Windows-UserModePowerService/Diagnostic') { wevtutil.exe set-log $l /e:false 2>&1 | Out-Null }
      $null
  } `
  -Undo {
      foreach ($l in 'Microsoft-Windows-SleepStudy/Diagnostic', 'Microsoft-Windows-Kernel-Processor-Power/Diagnostic', 'Microsoft-Windows-UserModePowerService/Diagnostic') { wevtutil.exe set-log $l /e:true 2>&1 | Out-Null }
  }

T -Id 'perf-paging' -Cat $P -Name 'Keep kernel in RAM (16 GB+ only)' -Tags max -Risk Moderate -Restart `
  -Desc 'DisablePagingExecutive + DisablePageCombining: kernel and drivers are never paged out. Snappier with plenty of RAM; wasteful on small machines.' `
  -Reg @(
      Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Memory Management' 'DisablePagingExecutive' 1
      Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Memory Management' 'DisablePageCombining' 1
  )

T -Id 'perf-svchostsplit' -Cat $P -Name 'Do not split service hosts (fewer svchost processes)' -Tags max -Risk Moderate -Restart `
  -Desc 'Services share svchost processes again instead of one process each: roughly 50-70 fewer processes and a little less RAM. Xbox services are left alone.' `
  -Apply {
      $done = @()
      foreach ($k in Get-ChildItem $global:SVCROOT -ErrorAction SilentlyContinue) {
          if ($k.PSChildName -match 'Xbl|Xbox') { continue }
          $p = Get-ItemProperty -LiteralPath $k.PSPath -ErrorAction SilentlyContinue
          if ($null -ne $p.Start -and $null -eq $p.SvcHostSplitDisable) {
              try { Set-ItemProperty -LiteralPath $k.PSPath -Name SvcHostSplitDisable -Value 1 -Type DWord -ErrorAction Stop; $done += $k.PSChildName } catch {}
          }
      }
      , $done
  } `
  -Undo { param($d) foreach ($n in @($d)) { Remove-ItemProperty -LiteralPath "$global:SVCROOT\$n" -Name SvcHostSplitDisable -ErrorAction SilentlyContinue } } `
  -Test { (Get-ItemProperty "$global:SVCROOT\Dnscache" -ErrorAction SilentlyContinue).SvcHostSplitDisable -eq 1 -and (Get-ItemProperty "$global:SVCROOT\Schedule" -ErrorAction SilentlyContinue).SvcHostSplitDisable -eq 1 }

T -Id 'perf-8dot3' -Cat $P -Name 'NTFS: stop creating 8.3 short names' -Tags max -Risk Moderate `
  -Desc 'No more DOS-style short names for new files: faster directory operations. Very old 16-bit installers may break.' `
  -Apply { $o = Invoke-Native fsutil @('8dot3name', 'query'); $prev = if ($o -match '(?:state|value)[^0-9]*([0-3])') { [int]$Matches[1] } else { 2 }; fsutil 8dot3name set 1 | Out-Null; @{ Prev = $prev } } `
  -Undo  { param($d) $v = if ($d -and $null -ne $d.Prev) { $d.Prev } else { 2 }; fsutil 8dot3name set $v | Out-Null } `
  -Test  { (Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem' -ErrorAction SilentlyContinue).NtfsDisable8dot3NameCreation -eq 1 }

T -Id 'perf-foldertype' -Cat $P -Name 'Explorer: stop auto-detecting folder types' -Tags safe, max -Explorer `
  -Desc 'Every folder opens as a generic folder instead of Windows guessing "Pictures/Music" per folder, which is a classic cause of slow Explorer. Folder view memory is backed up and reset.' `
  -Apply {
      $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
      $bags = Join-Path $OptiBackupDir "shellbags-$stamp.reg"
      $mru  = Join-Path $OptiBackupDir "shellbagmru-$stamp.reg"
      reg.exe export 'HKCU\SOFTWARE\Classes\Local Settings\Software\Microsoft\Windows\Shell\Bags' $bags /y 2>&1 | Out-Null
      reg.exe export 'HKCU\SOFTWARE\Classes\Local Settings\Software\Microsoft\Windows\Shell\BagMRU' $mru /y 2>&1 | Out-Null
      Remove-Item "$global:SHELLBAGS\Bags", "$global:SHELLBAGS\BagMRU" -Recurse -Force -ErrorAction SilentlyContinue
      Set-RegEntry "$global:SHELLBAGS\Bags\AllFolders\Shell" 'FolderType' 'NotSpecified' 'String'
      @{ Bags = $bags; Mru = $mru }
  } `
  -Undo {
      param($d)
      Remove-Item "$global:SHELLBAGS\Bags", "$global:SHELLBAGS\BagMRU" -Recurse -Force -ErrorAction SilentlyContinue
      foreach ($f in $d.Bags, $d.Mru) { if ($f -and (Test-Path -LiteralPath $f)) { reg.exe import $f 2>&1 | Out-Null } }
  } `
  -Test { (Get-ItemProperty "$global:SHELLBAGS\Bags\AllFolders\Shell" -ErrorAction SilentlyContinue).FolderType -eq 'NotSpecified' }

T -Id 'perf-maintenance' -Cat $P -Name 'Automatic Maintenance: never wake the PC' -Tags safe, max `
  -Desc 'Scheduled maintenance still runs when the PC is on, but it can no longer wake it from sleep at night.' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\Task Scheduler\Maintenance' 'WakeUp' 0 )

T -Id 'perf-iconcache' -Cat $P -Name 'Bigger icon cache (4 MB)' -Tags safe, max -Explorer `
  -Desc 'Explorer keeps more icons in memory, so big folders redraw faster.' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer' 'Max Cached Icons' '4096' 'String' )

T -Id 'perf-hover' -Cat $P -Name 'Instant hover tooltips' -Tags max `
  -Desc 'Mouse hover delay for tooltips and file info drops from 400 ms to 20 ms.' `
  -Reg @( Rg 'HKCU:\Control Panel\Mouse' 'MouseHoverTime' '20' 'String' )

T -Id 'perf-wpbt' -Cat $P -Name 'Block WPBT (firmware-injected programs)' -Tags safe, max -Restart `
  -Desc 'Stops firmware from dropping and running OEM software on every boot (the Windows Platform Binary Table). Also a security hardening.' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager' 'DisableWpbtExecution' 1 )

T -Id 'perf-forceend' -Cat $P -Name 'Force-close apps on shutdown and sign-out' -Tags optin -Risk Moderate `
  -Desc 'Apps are closed immediately without the "save changes?" wait. Faster, but unsaved work is lost.' `
  -Reg @( Rg $DESK 'AutoEndTasks' '1' 'String' )

T -Id 'perf-aeroshake' -Cat $P -Name 'Disable Aero Shake' -Tags safe, max `
  -Desc 'Shaking a window no longer minimizes all the others (usually triggered by accident).' `
  -Reg @( Rg $EXA 'DisallowShaking' 1 )

# ======================================================================= NETWORK
T -Id 'net-smbthrottle' -Cat $N -Name 'SMB: remove bandwidth throttling' -Tags max `
  -Desc 'Network share transfers are no longer rate-limited by the client. Helps on fast LANs and NAS copies.' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Services\LanmanWorkstation\Parameters' 'DisableBandwidthThrottling' 1 )

# ======================================================================= PRIVACY
T -Id 'priv-experiment' -Cat $V -Name 'Opt out of Microsoft experiments' -Tags safe, privacy, max `
  -Desc 'Stops Microsoft from using your PC as a test subject for unfinished features.' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Microsoft\PolicyManager\default\System\AllowExperimentation' 'value' 0 )

T -Id 'priv-pca' -Cat $V -Name 'Disable Program Compatibility Assistant' -Tags privacy, max `
  -Desc 'No more "this program might not have installed correctly" popups and app-usage reporting.' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\AppCompat' 'AITEnable' 0; Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\AppCompat' 'DisablePCA' 1 )

T -Id 'priv-perftrack' -Cat $V -Name 'Disable responsiveness tracking' -Tags safe, privacy, max `
  -Desc 'Stops Windows from collecting and reporting app responsiveness events (Windows Diagnostic Infrastructure).' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\WDI\{9c5a40da-b965-4fc3-8781-88dd50a6299d}' 'ScenarioExecutionEnabled' 0 )

T -Id 'priv-mru' -Cat $V -Name 'Disable most-used-apps tracking' -Tags safe, privacy, max `
  -Desc 'Start no longer builds a "most used" list from what you launch.' `
  -Reg @( Rg "$CV\Policies\Explorer" 'NoInstrumentation' 1 )

T -Id 'priv-rsop' -Cat $V -Name 'Disable RSoP policy logging' -Tags safe, privacy, max `
  -Desc 'Stops logging of resultant-set-of-policy data after every Group Policy refresh.' `
  -Reg @( Rg "$POL\System" 'RSoPLogging' 0 )

T -Id 'priv-devicehealth' -Cat $V -Name 'Disable device health attestation reporting' -Tags privacy, max `
  -Desc 'Stops device-health reporting to Microsoft at startup.' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Policies\Microsoft\DeviceHealthAttestationService' 'EnableDeviceHealthAttestationService' 0 )

T -Id 'priv-ceip' -Cat $V -Name 'Disable Customer Experience Improvement Program' -Tags safe, privacy, max `
  -Desc 'Turns off CEIP / SQM data collection for Windows and App-V.' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Policies\Microsoft\SQMClient\Windows' 'CEIPEnable' 0; Rg 'HKLM:\SOFTWARE\Policies\Microsoft\AppV\CEIP' 'CEIPEnable' 0 )

T -Id 'priv-difftrace' -Cat $V -Name 'Disable diagnostic tracing' -Tags privacy, max `
  -Desc 'Turns off Windows performance diagnostic tracing sessions.' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\Diagnostics\Performance' 'DisableDiagnosticTracing' 1 )

T -Id 'priv-dotnet' -Cat $V -Name 'Opt out of .NET CLI telemetry' -Tags privacy, max `
  -Desc 'Sets DOTNET_CLI_TELEMETRY_OPTOUT=1 system-wide for developers using dotnet.' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Environment' 'DOTNET_CLI_TELEMETRY_OPTOUT' '1' 'String' )

T -Id 'priv-oobe' -Cat $V -Name 'Skip post-update privacy re-prompt' -Tags safe, privacy, max `
  -Desc 'Feature updates will not show the privacy-settings screen that can reset your choices.' `
  -Reg @( Rg "$POL\OOBE" 'DisablePrivacyExperience' 1 )

T -Id 'priv-shared' -Cat $V -Name 'Disable Shared Experiences and Cross Device Resume' -Tags privacy `
  -Desc 'No nearby sharing, no continuing apps from other devices.' `
  -Reg @(
      Rg "$CV\CDP" 'CdpSessionUserAuthzPolicy' 0
      Rg "$CV\CDP" 'NearShareChannelUserAuthzPolicy' 0
      Rg "$CV\CDP" 'RomeSdkChannelUserAuthzPolicy' 0
      Rg "$CV\CrossDeviceResume\Configuration" 'IsResumeAllowed' 0
  )

T -Id 'priv-msrt' -Cat $V -Name 'Stop Malicious Software Removal Tool reporting' -Tags safe, privacy, max `
  -Desc 'MSRT still removes malware but stops sending infection reports.' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Policies\Microsoft\MRT' 'DontReportInfectionInformation' 1 )

# ======================================================================= WINDOWS UPDATE BEHAVIOR (updates stay on)
T -Id 'wu-noreboot' -Cat $D -Name 'Windows Update: no forced restarts while signed in' -Tags safe, max `
  -Desc 'Updates still install, but Windows will not reboot under you while you are logged on.' `
  -Reg @( Rg $WU 'AUPowerManagement' 0; Rg "$WU\AU" 'NoAutoRebootWithLoggedOnUsers' 1 )

T -Id 'wu-insider' -Cat $D -Name 'Block Windows Insider enrollment' -Tags safe, max `
  -Desc 'Prevents accidental enrollment in preview builds that can undo tweaks and cause instability.' `
  -Reg @( Rg $WU 'ManagePreviewBuilds' 1; Rg $WU 'ManagePreviewBuildsPolicyValue' 0; Rg "$POL\PreviewBuilds" 'AllowBuildPreview' 0 )

# ======================================================================= INTERFACE / QOL
T -Id 'ui-shortcuttext' -Cat $I -Name 'No "- Shortcut" suffix on new shortcuts' -Tags max `
  -Desc 'New shortcuts are named like the target.' `
  -Reg @( Rg "$CV\Explorer\NamingTemplates" 'ShortcutNameTemplate' '"%s.lnk"' 'String' )

T -Id 'ui-transferdetails' -Cat $I -Name 'Show more details in file copy dialogs' -Tags max `
  -Desc 'Copy/move dialogs open expanded with the speed graph.' `
  -Reg @( Rg "$CV\Explorer\OperationStatusManager" 'EnthusiastMode' 1 )

T -Id 'ui-compact' -Cat $I -Name 'Compact Explorer spacing' -Explorer `
  -Desc 'Tighter rows in Explorer lists (the Windows 10 density).' `
  -Reg @( Rg $EXA 'UseCompactMode' 1 )

T -Id 'ui-lowdisk' -Cat $I -Name 'Disable low disk space warnings' `
  -Desc 'No balloon when a drive is nearly full.' `
  -Reg @( Rg "$CV\Policies\Explorer" 'NoLowDiskSpaceChecks' 1 )

T -Id 'ui-autocorrect' -Cat $I -Name 'Disable autocorrect and text prediction' `
  -Desc 'Turns off touch-keyboard autocorrect, spell check, suggestions and double-tap-space.' `
  -Reg @(
      Rg 'HKCU:\SOFTWARE\Microsoft\TabletTip\1.7' 'EnableAutocorrection' 0
      Rg 'HKCU:\SOFTWARE\Microsoft\TabletTip\1.7' 'EnableSpellchecking' 0
      Rg 'HKCU:\SOFTWARE\Microsoft\TabletTip\1.7' 'EnableTextPrediction' 0
      Rg 'HKCU:\SOFTWARE\Microsoft\TabletTip\1.7' 'EnableDoubleTapSpace' 0
  )

T -Id 'ui-touchfx' -Cat $I -Name 'Disable touch visual feedback' `
  -Desc 'No ripple circles on touch screens.' `
  -Reg @( Rg 'HKCU:\Control Panel\Cursors' 'GestureVisualization' 0; Rg 'HKCU:\Control Panel\Cursors' 'ContactVisualization' 0 )

T -Id 'ui-ducking' -Cat $I -Name 'Do not lower sounds during calls' `
  -Desc 'Windows stops muting other audio when it detects communication activity.' `
  -Reg @( Rg 'HKCU:\SOFTWARE\Microsoft\Multimedia\Audio' 'UserDuckingPreference' 3 )

T -Id 'ui-verbose' -Cat $I -Name 'Verbose startup, shutdown and sign-in messages' `
  -Desc 'Shows what Windows is doing instead of "Please wait".' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System' 'VerboseStatus' 1 )

# ======================================================================= AI & COPILOT (tag deai = "Remove AI / debloat" preset)
$AI = 'AI & Copilot'
$WAI = 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsAI'
$EDGEP = 'HKLM:\SOFTWARE\Policies\Microsoft\Edge'

T -Id 'ai-recall' -Cat $AI -Name 'Turn off Recall and AI data analysis' -Tags safe, privacy, max, deai -Restart `
  -Desc 'Policy blocks Recall snapshots and AI data analysis for you and the machine; the Recall Windows feature is switched off where it exists (Copilot+ PCs).' `
  -Reg @(
      Rg $WAI 'DisableAIDataAnalysis' 1
      Rg $WAI 'AllowRecallEnablement' 0
      Rg 'HKCU:\SOFTWARE\Policies\Microsoft\Windows\WindowsAI' 'DisableAIDataAnalysis' 1
  ) `
  -Apply {
      $f = Get-WindowsOptionalFeature -Online -FeatureName Recall -ErrorAction SilentlyContinue
      if ($f -and $f.State -eq 'Enabled') { Disable-WindowsOptionalFeature -Online -FeatureName Recall -NoRestart -ErrorAction SilentlyContinue | Out-Null; @{ WasEnabled = $true } } else { @{ WasEnabled = $false } }
  } `
  -Undo { param($d) if ($d -and $d.WasEnabled) { Enable-WindowsOptionalFeature -Online -FeatureName Recall -NoRestart -ErrorAction SilentlyContinue | Out-Null } }

T -Id 'ai-clicktodo' -Cat $AI -Name 'Turn off Click to Do and AI search suggestions' -Tags safe, privacy, max, deai `
  -Desc 'Disables the Click to Do overlay and dynamic search-box highlights in the taskbar.' `
  -Reg @(
      Rg $WAI 'DisableClickToDo' 1
      Rg 'HKCU:\SOFTWARE\Policies\Microsoft\Windows\WindowsAI' 'DisableClickToDo' 1
      Rg "$CV\SearchSettings" 'IsDynamicSearchBoxEnabled' 0
  )

T -Id 'ai-copilot-button' -Cat $AI -Name 'Hide Copilot button and block Copilot policy' -Tags safe, privacy, max, deai -Explorer `
  -Desc 'Removes the Copilot taskbar button and sets the TurnOffWindowsCopilot policy for the user and the machine.' `
  -Reg @(
      Rg $EXA 'ShowCopilotButton' 0
      Rg 'HKCU:\Software\Policies\Microsoft\Windows\WindowsCopilot' 'TurnOffWindowsCopilot' 1
      Rg "$POL\WindowsCopilot" 'TurnOffWindowsCopilot' 1
  )

T -Id 'ai-apps' -Cat $AI -Name 'Notepad, Paint and Photos: turn off AI features' -Tags safe, privacy, max, deai `
  -Desc 'Group-policy switches that hide Notepad AI (Rewrite/Summarize), Paint Cocreator, Generative Fill and Image Creator. Which switches apply depends on the app version.' `
  -Reg @(
      Rg 'HKLM:\SOFTWARE\Policies\WindowsNotepad' 'DisableAIFeatures' 1
      Rg "$POL\Paint" 'DisableCocreator' 1
      Rg "$POL\Paint" 'DisableGenerativeFill' 1
      Rg "$POL\Paint" 'DisableImageCreator' 1
  )

T -Id 'ai-edge' -Cat $AI -Name 'Edge: turn off Copilot sidebar and page context' -Tags safe, privacy, max, deai `
  -Desc 'Edge policies: hides the sidebar/Copilot hub, stops Copilot reading page content, hides the Microsoft 365 Copilot icon.' `
  -Reg @(
      Rg $EDGEP 'HubsSidebarEnabled' 0
      Rg $EDGEP 'CopilotCDPPageContext' 0
      Rg $EDGEP 'CopilotPageContext' 0
      Rg $EDGEP 'Microsoft365CopilotChatIconEnabled' 0
      Rg $EDGEP 'EdgeEntraCopilotPageContext' 0
  )

T -Id 'ai-remove-apps' -Cat $AI -Name 'Uninstall the Copilot app packages' -Tags max, deai -Risk Moderate `
  -Desc 'Removes every installed *Copilot* app package for all users and from the provisioning list so new accounts do not get it. Undo cannot reinstall: use the Microsoft Store.' `
  -Apply {
      $names = @()
      foreach ($p in Get-AppxPackage -AllUsers -Name '*Copilot*' -ErrorAction SilentlyContinue) {
          $names += $p.Name
          Remove-AppxPackage -Package $p.PackageFullName -AllUsers -ErrorAction SilentlyContinue
      }
      Get-AppxProvisionedPackage -Online | Where-Object DisplayName -like '*Copilot*' | Remove-AppxProvisionedPackage -Online -ErrorAction SilentlyContinue | Out-Null
      , @($names | Select-Object -Unique)
  } `
  -Undo { Write-Log '  Copilot apps cannot be restored automatically: reinstall from the Microsoft Store if you want them back.' 'WARN' } `
  -Test { -not (Get-AppxPackage -Name '*Copilot*' -ErrorAction SilentlyContinue) }

# ======================================================================= MORE DEBLOAT
T -Id 'debloat-spotlight' -Cat $D -Name 'Turn off Windows Spotlight and lock-screen promos' -Tags safe, privacy, max, deai `
  -Desc 'No Spotlight desktop icon or lock-screen "fun facts", tips and promoted content.' `
  -Reg @(
      Rg "$POL\CloudContent" 'DisableWindowsSpotlightFeatures' 1
      Rg "$POL\CloudContent" 'DisableSpotlightCollectionOnDesktop' 1
      Rg "$POL\CloudContent" 'DisableThirdPartySuggestions' 1
      Rg 'HKCU:\SOFTWARE\Policies\Microsoft\Windows\CloudContent' 'DisableWindowsSpotlightFeatures' 1
  )

T -Id 'debloat-teamsauto' -Cat $D -Name 'Stop Teams (Chat) from auto-installing' -Tags safe, max, deai `
  -Desc 'Prevents Windows from silently installing the consumer Teams app on new sessions and updates.' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Communications' 'ConfigureChatAutoInstall' 0 )

T -Id 'debloat-oobeapps' -Cat $D -Name 'Block forced Outlook and Dev Home installs' -Tags safe, max, deai `
  -Desc 'Deletes the Windows Update orchestrator entries that reinstall the new Outlook and Dev Home after feature updates.' `
  -Apply {
      $base = 'HKLM:\SOFTWARE\Microsoft\WindowsUpdate\Orchestrator\UScheduler_Oobe'
      $gone = @()
      foreach ($n in 'OutlookUpdate', 'DevHomeUpdate') { if (Test-Path "$base\$n") { Remove-Item "$base\$n" -Recurse -Force -ErrorAction SilentlyContinue; $gone += $n } }
      , $gone
  } `
  -Undo { Write-Log '  Orchestrator entries are recreated by Windows Update when needed.' } `
  -Test { -not ((Test-Path 'HKLM:\SOFTWARE\Microsoft\WindowsUpdate\Orchestrator\UScheduler_Oobe\OutlookUpdate') -or (Test-Path 'HKLM:\SOFTWARE\Microsoft\WindowsUpdate\Orchestrator\UScheduler_Oobe\DevHomeUpdate')) }

T -Id 'debloat-store' -Cat $D -Name 'Microsoft Store: no promoted apps or auto-updates of consumer apps' -Tags max, deai `
  -Desc 'Stops the Store from pushing suggested apps and turns off automatic app updates (you can still update manually).' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Policies\Microsoft\WindowsStore' 'AutoDownload' 2; Rg "$CV\ContentDeliveryManager" 'SubscribedContent-338388Enabled' 0 )

T -Id 'debloat-feeds' -Cat $D -Name 'Turn off news, weather and interests feeds' -Tags safe, max, deai `
  -Desc 'Removes the news-and-interests feed from the taskbar and the Widgets news board via policy.' `
  -Reg @( Rg "$POL\Windows Feeds" 'EnableFeeds' 0; Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Dsh' 'AllowNewsAndInterests' 0 )

# ======================================================================= BROWSERS (enterprise policies; browsers show "managed by your organization")
$B = 'Browsers'
$CHR = 'HKLM:\SOFTWARE\Policies\Google\Chrome'
$BRV = 'HKLM:\SOFTWARE\Policies\BraveSoftware\Brave'
$EDG = 'HKLM:\SOFTWARE\Policies\Microsoft\Edge'
$FFX = 'HKLM:\SOFTWARE\Policies\Mozilla\Firefox'
$VIV = 'HKLM:\SOFTWARE\Policies\Vivaldi'
$NOTE = ' Applied as browser policy, so the browser shows "managed by your organization". Undo removes it.'

T -Id 'br-chrome-debloat' -Cat $B -Name 'Chrome: no telemetry, background mode or promos' -Tags safe, privacy, max, deai `
  -Desc ('Turns off usage metrics, background running, startup boost, default-browser nags, shopping and promo tabs, feedback surveys and Privacy Sandbox ad topics.' + $NOTE) `
  -Reg @(
      Rg $CHR 'MetricsReportingEnabled' 0
      Rg $CHR 'UrlKeyedAnonymizedDataCollectionEnabled' 0
      Rg $CHR 'SafeBrowsingExtendedReportingEnabled' 0
      Rg $CHR 'BackgroundModeEnabled' 0
      Rg $CHR 'StartupBoostEnabled' 0
      Rg $CHR 'DefaultBrowserSettingEnabled' 0
      Rg $CHR 'ShoppingListEnabled' 0
      Rg $CHR 'PromotionalTabsEnabled' 0
      Rg $CHR 'UserFeedbackAllowed' 0
      Rg $CHR 'FeedbackSurveysEnabled' 0
      Rg $CHR 'MediaRecommendationsEnabled' 0
      Rg $CHR 'PrivacySandboxPromptEnabled' 0
      Rg $CHR 'PrivacySandboxAdTopicsEnabled' 0
      Rg $CHR 'PrivacySandboxAdMeasurementEnabled' 0
      Rg $CHR 'PrivacySandboxSiteEnabledAdsEnabled' 0
  )

T -Id 'br-chrome-ai' -Cat $B -Name 'Chrome: turn off Gemini and generative AI features' -Tags safe, privacy, max, deai `
  -Desc ('Disables Gemini integration, Help me write, tab organizer, AI themes, tab compare and the AI-mode omnibox entry.' + $NOTE) `
  -Reg @(
      Rg $CHR 'GenAiDefaultSettings' 2
      Rg $CHR 'GeminiSettings' 1
      Rg $CHR 'HelpMeWriteSettings' 2
      Rg $CHR 'TabOrganizerSettings' 2
      Rg $CHR 'CreateThemesSettings' 2
      Rg $CHR 'TabCompareSettings' 2
      Rg $CHR 'AIModeSettings' 1
  )

T -Id 'br-brave-debloat' -Cat $B -Name 'Brave: remove Rewards, Wallet, VPN, News, Talk and AI Chat' -Tags safe, privacy, max, deai `
  -Desc ('Hides the crypto, ads, VPN, news, Talk and Leo features and turns off metrics, stats ping, web discovery and background mode.' + $NOTE) `
  -Reg @(
      Rg $BRV 'BraveRewardsDisabled' 1
      Rg $BRV 'BraveWalletDisabled' 1
      Rg $BRV 'BraveVPNDisabled' 1
      Rg $BRV 'BraveNewsDisabled' 1
      Rg $BRV 'BraveTalkDisabled' 1
      Rg $BRV 'BraveAIChatEnabled' 0
      Rg $BRV 'BraveWebDiscoveryEnabled' 0
      Rg $BRV 'BraveStatsPingEnabled' 0
      Rg $BRV 'MetricsReportingEnabled' 0
      Rg $BRV 'BackgroundModeEnabled' 0
      Rg $BRV 'SafeBrowsingExtendedReportingEnabled' 0
  )

T -Id 'br-edge-debloat' -Cat $B -Name 'Edge: remove new-tab feed, workspaces, Bing ads and promos' -Tags safe, privacy, max, deai `
  -Desc ('Extra Edge policies on top of "Edge: disable nags": new-tab news feed, promotional tabs, workspaces, Bing ads, default-browser nag, metrics and site-info reporting, game mode panel.' + $NOTE) `
  -Reg @(
      Rg $EDG 'PromotionalTabsEnabled' 0
      Rg $EDG 'NewTabPageContentEnabled' 0
      Rg $EDG 'EdgeWorkspacesEnabled' 0
      Rg $EDG 'BingAdsSuppression' 1
      Rg $EDG 'DefaultBrowserSettingEnabled' 0
      Rg $EDG 'MetricsReportingEnabled' 0
      Rg $EDG 'SendSiteInfoToImproveServices' 0
      Rg $EDG 'WebWidgetIsEnabledOnStartup' 0
      Rg $EDG 'ShowAcrobatSubscriptionButton' 0
      Rg $EDG 'GamerModeEnabled' 0
      Rg $EDG 'EdgeShoppingAssistantEnabled' 0
      Rg $EDG 'PersonalizationReportingEnabled' 0
  )

T -Id 'br-firefox-debloat' -Cat $B -Name 'Firefox: no telemetry, studies, Pocket, sponsored content or nags' -Tags safe, privacy, max, deai `
  -Desc ('Disables telemetry, Shield studies, Pocket, sponsored top sites and suggestions, the default-browser agent, feedback commands, whats-new pages and extension/feature recommendations.' + $NOTE) `
  -Reg @(
      Rg $FFX 'DisableTelemetry' 1
      Rg $FFX 'DisableFirefoxStudies' 1
      Rg $FFX 'DisablePocket' 1
      Rg $FFX 'DisableFeedbackCommands' 1
      Rg $FFX 'DisableDefaultBrowserAgent' 1
      Rg $FFX 'DontCheckDefaultBrowser' 1
      Rg "$FFX\FirefoxHome" 'SponsoredTopSites' 0
      Rg "$FFX\FirefoxHome" 'SponsoredPocket' 0
      Rg "$FFX\FirefoxHome" 'Snippets' 0
      Rg "$FFX\FirefoxSuggest" 'WebSuggestions' 0
      Rg "$FFX\FirefoxSuggest" 'SponsoredSuggestions' 0
      Rg "$FFX\FirefoxSuggest" 'ImproveSuggest' 0
      Rg "$FFX\UserMessaging" 'WhatsNew' 0
      Rg "$FFX\UserMessaging" 'ExtensionRecommendations' 0
      Rg "$FFX\UserMessaging" 'FeatureRecommendations' 0
      Rg "$FFX\UserMessaging" 'UrlbarInterventions' 0
      Rg "$FFX\UserMessaging" 'MoreFromMozilla' 0
      Rg "$FFX\UserMessaging" 'SkipOnboarding' 1
  )

T -Id 'br-firefox-ai' -Cat $B -Name 'Firefox: turn off AI chatbot, link previews and AI tab groups' -Tags safe, privacy, max, deai `
  -Desc ('Sets the GenerativeAI policy so the sidebar chatbot, AI link previews and AI tab-group suggestions are off (Firefox 139 and newer).' + $NOTE) `
  -Reg @(
      Rg "$FFX\GenerativeAI" 'Enabled' 0
      Rg "$FFX\GenerativeAI" 'Chatbot' 0
      Rg "$FFX\GenerativeAI" 'LinkPreviews' 0
      Rg "$FFX\GenerativeAI" 'TabGroups' 0
  )

T -Id 'br-vivaldi-debloat' -Cat $B -Name 'Vivaldi: no metrics or background mode' -Tags max `
  -Desc ('Vivaldi honors Chromium policies: turns off metrics and background running.' + $NOTE) `
  -Reg @( Rg $VIV 'MetricsReportingEnabled' 0; Rg $VIV 'BackgroundModeEnabled' 0 )

T -Id 'br-updaters' -Cat $B -Name 'Browser updater services: set to Manual' -Tags max `
  -Desc 'Google Update and Brave Update services stop running constantly and start on demand. Browsers still update through their scheduled tasks.' `
  -Svc @( Sx 'gupdate' 3; Sx 'gupdatem' 3; Sx 'brave' 3; Sx 'bravem' 3 )

T -Id 'br-opera-startup' -Cat $B -Name 'Opera / Opera GX: stop autostart and background auto-update tasks' -Tags max -Risk Moderate `
  -Desc 'Disables the "Opera Browser Assistant" startup entries and the "Opera scheduled Autoupdate" tasks, so Opera no longer launches helpers at sign-in. Opera still updates when you open it. Opera has no policy support, so its AI (Aria), GX Corner, wallet and sidebar toggles can only be changed inside Opera settings.' `
  -Apply {
      $run = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
      $apr = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
      $done = @{ Run = @(); Tasks = @() }
      $key = Get-Item -LiteralPath $run -ErrorAction SilentlyContinue
      if ($key) {
          foreach ($n in $key.GetValueNames() | Where-Object { $_ -like 'Opera*' }) {
              if (Test-StartupEnabled $apr $n) { Set-StartupItem @{ Name = $n; Approved = $apr } $false; $done.Run += $n }
          }
      }
      foreach ($t in Get-ScheduledTask -TaskPath '\' -ErrorAction SilentlyContinue | Where-Object { $_.TaskName -like 'Opera*' -and $_.State -ne 'Disabled' }) {
          Disable-ScheduledTask -TaskPath $t.TaskPath -TaskName $t.TaskName -ErrorAction SilentlyContinue | Out-Null
          $done.Tasks += $t.TaskName
      }
      $done
  } `
  -Undo {
      param($d)
      $apr = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
      foreach ($n in @($d.Run)) { Set-StartupItem @{ Name = $n; Approved = $apr } $true }
      foreach ($n in @($d.Tasks)) { Enable-ScheduledTask -TaskPath '\' -TaskName $n -ErrorAction SilentlyContinue | Out-Null }
  } `
  -Test {
      $installed = (Test-Path "$env:LOCALAPPDATA\Programs\Opera GX") -or (Test-Path "$env:LOCALAPPDATA\Programs\Opera")
      if (-not $installed) { return $false }
      $apr = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
      $key = Get-Item -LiteralPath 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -ErrorAction SilentlyContinue
      if ($key) { foreach ($n in $key.GetValueNames() | Where-Object { $_ -like 'Opera*' }) { if (Test-StartupEnabled $apr $n) { return $false } } }
      -not (Get-ScheduledTask -TaskPath '\' -ErrorAction SilentlyContinue | Where-Object { $_.TaskName -like 'Opera*' -and $_.State -ne 'Disabled' })
  }


# ======================================================================= MORE MODULES
# Ideas drawn from Win11Debloat (MIT), WinUtil (MIT) and AtlasOS-style tuning; implemented here with snapshot/undo.

# ---- context menu clean-up (keys are exported to a .reg backup before deletion; undo re-imports)
T -Id 'ui-ctx-share' -Cat $I -Name 'Remove "Share" from the right-click menu' -Explorer `
  -Desc 'Removes the ModernSharing context-menu handler. The deleted key is exported first; undo re-imports it.' `
  -Apply { Remove-RegKeysWithBackup @('HKEY_CLASSES_ROOT\*\shellex\ContextMenuHandlers\ModernSharing') 'ctx-share' } `
  -Undo  { param($d) Restore-RegFiles $d } `
  -Test  { Test-RegKeysGone @('HKEY_CLASSES_ROOT\*\shellex\ContextMenuHandlers\ModernSharing') }

T -Id 'ui-ctx-giveaccess' -Cat $I -Name 'Remove "Give access to" from the right-click menu' -Explorer `
  -Desc 'Removes the network-sharing "Give access to" handlers from files, folders and drives. Backed up and reversible.' `
  -Apply {
      Remove-RegKeysWithBackup @(
          'HKEY_CLASSES_ROOT\*\shellex\ContextMenuHandlers\Sharing'
          'HKEY_CLASSES_ROOT\Directory\Background\shellex\ContextMenuHandlers\Sharing'
          'HKEY_CLASSES_ROOT\Directory\shellex\ContextMenuHandlers\Sharing'
          'HKEY_CLASSES_ROOT\Drive\shellex\ContextMenuHandlers\Sharing'
          'HKEY_CLASSES_ROOT\LibraryFolder\background\shellex\ContextMenuHandlers\Sharing'
          'HKEY_CLASSES_ROOT\UserLibraryFolder\shellex\ContextMenuHandlers\Sharing') 'ctx-giveaccess'
  } `
  -Undo { param($d) Restore-RegFiles $d } `
  -Test { Test-RegKeysGone @('HKEY_CLASSES_ROOT\*\shellex\ContextMenuHandlers\Sharing', 'HKEY_CLASSES_ROOT\Directory\shellex\ContextMenuHandlers\Sharing', 'HKEY_CLASSES_ROOT\Drive\shellex\ContextMenuHandlers\Sharing') }

T -Id 'ui-ctx-library' -Cat $I -Name 'Remove "Include in library" from the right-click menu' -Explorer `
  -Desc 'Removes the Library Location handler from folders. Backed up and reversible.' `
  -Apply { Remove-RegKeysWithBackup @('HKEY_CLASSES_ROOT\Folder\ShellEx\ContextMenuHandlers\Library Location', 'HKEY_LOCAL_MACHINE\SOFTWARE\Classes\Folder\ShellEx\ContextMenuHandlers\Library Location') 'ctx-library' } `
  -Undo  { param($d) Restore-RegFiles $d } `
  -Test  { Test-RegKeysGone @('HKEY_LOCAL_MACHINE\SOFTWARE\Classes\Folder\ShellEx\ContextMenuHandlers\Library Location') }

T -Id 'ui-thispc-clean' -Cat $I -Name 'This PC: hide 3D Objects, Music and duplicate removable drives' -Explorer `
  -Desc 'Removes those entries from the This PC / navigation pane. Backed up and reversible.' `
  -Apply {
      Remove-RegKeysWithBackup @(
          'HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\MyComputer\NameSpace\{0DB7E03F-FC29-4DC6-9020-FF41B59E513A}'
          'HKEY_LOCAL_MACHINE\SOFTWARE\Wow6432Node\Microsoft\Windows\CurrentVersion\Explorer\MyComputer\NameSpace\{0DB7E03F-FC29-4DC6-9020-FF41B59E513A}'
          'HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\MyComputer\NameSpace\{3dfdf296-dbec-4fb4-81d1-6a3438bcf4de}'
          'HKEY_LOCAL_MACHINE\SOFTWARE\Wow6432Node\Microsoft\Windows\CurrentVersion\Explorer\MyComputer\NameSpace\{3dfdf296-dbec-4fb4-81d1-6a3438bcf4de}'
          'HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Desktop\NameSpace\DelegateFolders\{F5FB2C77-0E2F-4A16-A381-3E560C68BC83}') 'thispc'
  } `
  -Undo { param($d) Restore-RegFiles $d } `
  -Test { Test-RegKeysGone @('HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\MyComputer\NameSpace\{3dfdf296-dbec-4fb4-81d1-6a3438bcf4de}', 'HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Desktop\NameSpace\DelegateFolders\{F5FB2C77-0E2F-4A16-A381-3E560C68BC83}') }

T -Id 'ui-onedrive-nav' -Cat $I -Name 'Hide OneDrive in the Explorer navigation pane' -Explorer `
  -Desc 'Takes the OneDrive entry out of the Explorer sidebar without uninstalling OneDrive.' `
  -Reg @( Rg 'HKCU:\Software\Classes\CLSID\{018D5C66-4533-4307-9B53-224DE2ED1FE6}' 'System.IsPinnedToNameSpaceTree' 0 )

T -Id 'ui-recent' -Cat $I -Name 'Do not track recent and frequent files in Explorer' -Tags privacy `
  -Desc 'Quick access stops showing recent files and frequent folders.' `
  -Reg @( Rg "$CV\Explorer" 'ShowRecent' 0; Rg "$CV\Explorer" 'ShowFrequent' 0 )

T -Id 'ui-alttab' -Cat $I -Name 'Alt+Tab: windows only, no Edge tabs' `
  -Desc 'Alt+Tab lists windows, not individual browser tabs.' `
  -Reg @( Rg $EXA 'MultiTaskingAltTabFilter' 3 )

T -Id 'ui-lastactive' -Cat $I -Name 'Taskbar: clicking a grouped app re-opens its last window' -Explorer `
  -Desc 'Single click on a grouped taskbar icon focuses the last active window instead of showing previews.' `
  -Reg @( Rg $EXA 'LastActiveClick' 1 )

T -Id 'ui-notifs-off' -Cat $I -Name 'Turn off all toast notifications' -Risk Moderate `
  -Desc 'No notification popups from any app (you can still open the notification center). Re-enable in Settings > Notifications.' `
  -Reg @( Rg "$CV\PushNotifications" 'ToastEnabled' 0 )

T -Id 'ui-settingshome' -Cat $I -Name 'Hide the Settings home page' `
  -Desc 'Settings opens on System instead of the Home page with cloud and account promos.' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\Explorer' 'SettingsPageVisibility' 'hide:home' 'String' )

T -Id 'ui-sharetray' -Cat $I -Name 'Disable the Share drag tray' -Tags safe, max `
  -Desc 'Stops the "drag files here to share" tray that appears when dragging files to the top of the screen.' `
  -Reg @( Rg "$CV\CDP" 'DragTrayEnabled' 0 )

T -Id 'ui-phonelink-start' -Cat $I -Name 'Hide Phone Link in the Start menu' -Tags safe, max `
  -Desc 'Removes the Phone Link companion panel from Start.' `
  -Reg @( Rg "$CV\Start\Companions\Microsoft.YourPhone_8wekyb3d8bbwe" 'IsEnabled' 0 )

T -Id 'ui-legacyboot' -Cat $I -Name 'Classic boot menu (F8 advanced options)' -Risk Moderate `
  -Desc 'bcdedit bootmenupolicy legacy: F8 works at boot for Safe Mode. Undo sets it back to standard.' `
  -Apply { bcdedit /set '{default}' bootmenupolicy legacy | Out-Null; $null } `
  -Undo  { bcdedit /set '{default}' bootmenupolicy standard | Out-Null } `
  -Test  { (bcdedit /enum '{current}' | Out-String) -match 'bootmenupolicy\s+Legacy' }

# ---- privacy / debloat
T -Id 'priv-searchhistory' -Cat $V -Name 'Disable device search history' -Tags safe, privacy, max `
  -Desc 'Windows stops remembering what you search for on this device.' `
  -Reg @( Rg "$CV\SearchSettings" 'IsDeviceSearchHistoryEnabled' 0 )

T -Id 'priv-speech' -Cat $V -Name 'Disable online speech recognition' -Tags safe, privacy, max `
  -Desc 'Voice data is no longer sent to Microsoft for cloud speech recognition.' `
  -Reg @( Rg 'HKCU:\Software\Microsoft\Speech_OneCore\Settings\OnlineSpeechPrivacy' 'HasAccepted' 0 )

T -Id 'priv-findmydevice' -Cat $V -Name 'Disable Find My Device' -Tags privacy -Risk Moderate `
  -Desc 'Stops location reporting for Find My Device. You lose the ability to locate this PC from your Microsoft account.' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Policies\Microsoft\FindMyDevice' 'AllowFindMyDevice' 0 )

T -Id 'priv-anon' -Cat $V -Name 'Block anonymous enumeration of accounts and shares' -Tags safe, max `
  -Desc 'Security hardening: anonymous users can no longer list accounts or share names.' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\Lsa' 'RestrictAnonymous' 1; Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\Lsa' 'RestrictAnonymousSAM' 1 )

T -Id 'debloat-devicemeta' -Cat $D -Name 'Do not auto-download device apps from manufacturers' -Tags safe, max, deai `
  -Desc 'Windows stops fetching vendor companion apps and icons when you plug in hardware.' `
  -Reg @( Rg "$POL\Device Metadata" 'PreventDeviceMetadataFromNetwork' 1 )

T -Id 'debloat-settings365' -Cat $D -Name 'Remove Microsoft 365 and account promos from Settings' -Tags safe, max, deai `
  -Desc 'Hides the consumer account-state banners and offers in the Settings app.' `
  -Reg @( Rg "$POL\CloudContent" 'DisableConsumerAccountStateContent' 1 )

T -Id 'debloat-suggestions2' -Cat $D -Name 'Turn off remaining Windows suggestions and nag toasts' -Tags safe, max, deai `
  -Desc 'Account notifications, sync-provider (OneDrive) ads, backup reminders, suggested-toast notifications and mobile-device nags.' `
  -Reg @(
      Rg $CDM 'SubscribedContent-353698Enabled' 0
      Rg "$CV\SystemSettings\AccountNotifications" 'EnableAccountNotifications' 0
      Rg $EXA 'ShowSyncProviderNotifications' 0
      Rg "$CV\Notifications\Settings\Windows.SystemToast.Suggested" 'Enabled' 0
      Rg "$CV\Notifications\Settings\Windows.SystemToast.BackupReminder" 'Enabled' 0
      Rg "$CV\Mobility" 'OptedIn' 0
  )

T -Id 'debloat-aiservice' -Cat $D -Name 'AI Fabric service: start on demand only' -Tags safe, max, deai `
  -Desc 'Sets the Windows AI Fabric service (WSAIFabricSvc) to Manual so it does not start with Windows.' `
  -Svc @( Sx 'WSAIFabricSvc' 3 )

T -Id 'wu-asap' -Cat $D -Name 'Windows Update: do not get new features as soon as available' -Tags safe, max `
  -Desc 'Turns off "Get the latest updates as soon as they are available" so feature rollouts reach you later, after they have settled.' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Microsoft\WindowsUpdate\UX\Settings' 'IsContinuousInnovationOptedIn' 0 )

T -Id 'sto-bitlocker' -Cat $D -Name 'Prevent automatic BitLocker device encryption' -Tags max -Risk Moderate `
  -Desc 'Stops Windows from silently encrypting the drive at first sign-in with a Microsoft account. Already-encrypted drives are not decrypted.' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\BitLocker' 'PreventDeviceEncryption' 1 )

T -Id 'sto-storagesense' -Cat $D -Name 'Turn off Storage Sense' -Tags optin `
  -Desc 'Windows stops auto-deleting temp files and Recycle Bin items on a schedule. Use the Cleaner tab instead.' `
  -Reg @( Rg "$CV\StorageSense\Parameters\StoragePolicy" '01' 0 )

T -Id 'perf-standbynet' -Cat $P -Name 'Disable network in Modern Standby' -Tags max -Risk Moderate `
  -Desc 'No network activity while the PC is in Modern Standby (S0): less battery drain and fewer random wake-ups. Notifications arrive when you wake it.' `
  -Reg @(
      Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Power\PowerSettings\f15576e8-98b7-4186-b944-eafa664402d9' 'ACSettingIndex' 0
      Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Power\PowerSettings\f15576e8-98b7-4186-b944-eafa664402d9' 'DCSettingIndex' 0
  )

# ======================================================================= ATLAS-STYLE QoL / PRIVACY / HARDENING (independent implementations)
T -Id 'ui-wallpaperq' -Cat $I -Name 'Lossless desktop wallpaper quality' -Tags max `
  -Desc 'Windows recompresses wallpapers to ~85% JPEG quality; this keeps 100%. Re-apply your wallpaper afterwards.' `
  -Reg @( Rg $DESK 'JPEGImportQuality' 100 )

T -Id 'ui-dynlighting' -Cat $I -Name 'Stop Windows controlling RGB lighting' -Tags max `
  -Desc 'Turns off Windows Dynamic Lighting so vendor RGB software (Razer, Corsair, ASUS...) is not overridden.' `
  -Reg @( Rg 'HKCU:\Software\Microsoft\Lighting' 'AmbientLightingEnabled' 0 )

T -Id 'ui-usbnotify' -Cat $I -Name 'Disable USB error and weak-charger notifications' `
  -Desc 'No balloon when a USB device malfunctions or charges slowly.' `
  -Reg @( Rg 'HKCU:\SOFTWARE\Microsoft\Shell\USB' 'NotifyOnUsbErrors' 0; Rg 'HKCU:\SOFTWARE\Microsoft\Shell\USB' 'NotifyOnWeakCharger' 0 )

T -Id 'ui-fullctx' -Cat $I -Name 'Full right-click menu on more than 15 selected items' -Tags max `
  -Desc 'Windows stops hiding menu entries when you right-click a large selection.' `
  -Reg @( Rg "$CV\Explorer" 'MultipleInvokePromptMinimum' 100 )

T -Id 'ui-officefiles' -Cat $I -Name 'No cloud/Office files in Quick access' -Tags safe, max `
  -Desc 'Quick access stops listing recent Office/cloud documents.' `
  -Reg @( Rg "$CV\Explorer" 'ShowCloudFilesInQuickAccess' 0 )

T -Id 'ui-autoplay' -Cat $I -Name 'Disable AutoPlay for drives and devices' -Tags safe, max `
  -Desc 'Inserting a USB stick or card no longer pops up AutoPlay. Also a malware-spreading vector closed.' `
  -Reg @( Rg "$CV\Explorer\AutoplayHandlers" 'DisableAutoplay' 1 )

T -Id 'ui-netwizard' -Cat $I -Name 'No "Public or Private network?" prompt' `
  -Desc 'New networks are classified automatically without the location wizard popup.' `
  -Apply { New-Item -Path 'HKLM:\SYSTEM\CurrentControlSet\Control\Network\NewNetworkWindowOff' -Force | Out-Null; $null } `
  -Undo  { Remove-Item -Path 'HKLM:\SYSTEM\CurrentControlSet\Control\Network\NewNetworkWindowOff' -Recurse -Force -ErrorAction SilentlyContinue } `
  -Test  { Test-Path 'HKLM:\SYSTEM\CurrentControlSet\Control\Network\NewNetworkWindowOff' }

T -Id 'ui-eoa' -Cat $I -Name 'Disable Ease of Access auto-read and shortcut launcher' `
  -Desc 'Stops "always read and scan this section" and the Win+U-style assistive tool launch on touch/pen.' `
  -Reg @( Rg 'HKCU:\SOFTWARE\Microsoft\Ease of Access' 'selfscan' 0; Rg 'HKCU:\SOFTWARE\Microsoft\Ease of Access' 'selfvoice' 0; Rg 'HKCU:\Control Panel\Accessibility\SlateLaunch' 'LaunchAT' 0 )

T -Id 'ui-printscreen' -Cat $I -Name 'Print Screen does not open Snipping Tool' -Risk Moderate `
  -Desc 'Frees the Print Screen key for other screenshot tools (it copies the screen as before).' `
  -Reg @( Rg 'HKCU:\Control Panel\Keyboard' 'PrintScreenKeyForSnippingEnabled' 0 )

T -Id 'perf-shortcuts' -Cat $P -Name 'Do not search for missing shortcut targets' -Tags safe, max `
  -Desc 'Windows stops hunting the whole disk when a shortcut is broken, which can freeze Explorer for seconds.' `
  -Reg @( Rg "$CV\Policies\Explorer" 'NoResolveSearch' 1; Rg "$CV\Policies\Explorer" 'NoResolveTrack' 1 )

T -Id 'perf-openwith' -Cat $P -Name 'Do not look up apps online for unknown file types' -Tags safe, max `
  -Desc 'Opening an unknown file no longer queries Microsoft for "Look for an app in the Store".' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\Explorer' 'NoInternetOpenWith' 1 )

T -Id 'perf-nicpower' -Cat $P -Name 'Network adapters: never power down to save energy' -Tags gaming, max -DesktopOnly `
  -Desc 'Disables "Allow the computer to turn off this device to save power" on physical network adapters: fewer dropouts and wake-up lag. Previous settings are saved.' `
  -Apply {
      $prev = @()
      foreach ($a in Get-NetAdapter -Physical -ErrorAction SilentlyContinue) {
          $pm = Get-NetAdapterPowerManagement -Name $a.Name -ErrorAction SilentlyContinue
          if ($pm -and $pm.AllowComputerToTurnOffDevice -ne 'Unsupported') {
              $prev += @{ Name = $a.Name; Value = [string]$pm.AllowComputerToTurnOffDevice }
              try { Set-NetAdapterPowerManagement -Name $a.Name -AllowComputerToTurnOffDevice Disabled -ErrorAction Stop } catch {}
          }
      }
      , $prev
  } `
  -Undo { param($d) foreach ($p in @($d)) { try { Set-NetAdapterPowerManagement -Name $p.Name -AllowComputerToTurnOffDevice $p.Value -ErrorAction Stop } catch {} } } `
  -Test {
      $any = $false
      foreach ($a in Get-NetAdapter -Physical -ErrorAction SilentlyContinue) {
          $pm = Get-NetAdapterPowerManagement -Name $a.Name -ErrorAction SilentlyContinue
          if ($pm -and $pm.AllowComputerToTurnOffDevice -ne 'Unsupported') { $any = $true; if ($pm.AllowComputerToTurnOffDevice -ne 'Disabled') { return $false } }
      }
      $any
  }

T -Id 'sec-nullsess' -Cat $N -Name 'Restrict anonymous (null session) access to shares' -Tags safe, max `
  -Desc 'Security hardening: anonymous connections can no longer reach named pipes and shares.' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Services\LanManServer\Parameters' 'RestrictNullSessAccess' 1 )

T -Id 'priv-settingtips' -Cat $V -Name 'Disable online tips in Settings' -Tags safe, privacy, max `
  -Desc 'Settings stops fetching tips and "did you know" banners from Microsoft.' `
  -Reg @(
      Rg 'HKLM:\SOFTWARE\Microsoft\PolicyManager\default\Settings\AllowOnlineTips' 'value' 0
      Rg 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\Explorer' 'AllowOnlineTips' 0
  )

T -Id 'priv-lockcamera' -Cat $V -Name 'Disable camera on the lock screen' -Tags safe, privacy, max `
  -Desc 'Swiping on the lock screen can no longer open the camera.' `
  -Reg @( Rg "$POL\Personalization" 'NoLockScreenCamera' 1 )

T -Id 'priv-speechmodel' -Cat $V -Name 'Stop automatic speech-model downloads' -Tags safe, privacy, max `
  -Desc 'Windows no longer downloads speech data updates in the background.' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Speech' 'AllowSpeechModelUpdate' 0 )

T -Id 'priv-settingsync' -Cat $V -Name 'Disable settings sync to your Microsoft account' -Tags privacy -Risk Moderate `
  -Desc 'Themes, passwords-metadata and preferences are no longer synced across your devices.' `
  -Reg @( Rg "$POL\SettingSync" 'DisableSettingSync' 2; Rg "$POL\SettingSync" 'DisableSettingSyncUserOverride' 1 )

T -Id 'priv-msgsync' -Cat $V -Name 'Disable message cloud sync' -Tags safe, privacy, max `
  -Desc 'Blocks SMS/message sync to the cloud.' `
  -Reg @( Rg "$POL\Messaging" 'AllowMessageSync' 0 )

T -Id 'priv-nvidia' -Cat $V -Name 'NVIDIA: opt out of telemetry' -Tags safe, privacy, max `
  -Desc 'Turns off NVIDIA Control Panel usage telemetry and the NVIDIA telemetry container service (if installed).' `
  -Reg @( Rg 'HKCU:\Software\NVIDIA Corporation\NVControlPanel2\Client' 'OptInOrOutPreference' 0 ) `
  -Svc @( Sx 'NvTelemetryContainer' 4 )

T -Id 'priv-office' -Cat $V -Name 'Microsoft Office: opt out of telemetry and feedback' -Tags safe, privacy, max `
  -Desc 'User policies that disable Office customer-data upload, telemetry, feedback, screenshots and the Office logging service.' `
  -Reg @(
      Rg 'HKCU:\Software\Policies\Microsoft\office\16.0\common' 'sendcustomerdata' 0
      Rg 'HKCU:\Software\Policies\Microsoft\office\16.0\common' 'qmenable' 0
      Rg 'HKCU:\Software\Policies\Microsoft\office\16.0\common' 'updatereliabilitydata' 0
      Rg 'HKCU:\Software\Policies\Microsoft\office\common\clienttelemetry' 'sendtelemetry' 3
      Rg 'HKCU:\Software\Policies\Microsoft\office\16.0\common\feedback' 'enabled' 0
      Rg 'HKCU:\Software\Policies\Microsoft\office\16.0\common\feedback' 'includescreenshot' 0
      Rg 'HKCU:\Software\Policies\Microsoft\office\16.0\osm' 'enablelogging' 0
      Rg 'HKCU:\Software\Policies\Microsoft\office\16.0\osm' 'enableupload' 0
  )

T -Id 'debloat-apparchive' -Cat $D -Name 'Do not auto-archive Store apps' -Tags max `
  -Desc 'Windows stops silently "archiving" (unloading) apps you have not used for a while.' `
  -Reg @( Rg "$POL\Appx" 'AllowAutomaticAppArchiving' 0 )

T -Id 'wu-nag' -Cat $D -Name 'Windows Update: remove "update and shut down" nag' -Tags safe, max `
  -Desc 'The power menu defaults to plain Shut down and the "Get the latest updates" link is hidden.' `
  -Reg @( Rg "$WU\AU" 'NoAUAsDefaultShutdownOption' 1; Rg 'HKLM:\SOFTWARE\Microsoft\WindowsUpdate\UX\Settings' 'HideMCTLink' 1 )

# ======================================================================= v2: WINDOWS UPDATE CONTROL + HOSTS TELEMETRY BLOCK
$WUX = 'HKLM:\SOFTWARE\Microsoft\WindowsUpdate\UX\Settings'

T -Id 'wu-security' -Cat $D -Name 'Windows Update: security updates first, delay feature updates 1 year' -Risk Moderate `
  -Desc 'Quality (security) updates arrive after 4 days; feature updates (24H2, 25H2...) are held back for 365 days so new builds settle first. Updates are never disabled.' `
  -Reg @(
      Rg $WU 'DeferFeatureUpdates' 1
      Rg $WU 'DeferFeatureUpdatesPeriodInDays' 365
      Rg $WU 'DeferQualityUpdates' 1
      Rg $WU 'DeferQualityUpdatesPeriodInDays' 4
  )

T -Id 'wu-nodrivers' -Cat $D -Name 'Windows Update: do not install drivers automatically' -Tags max -Risk Moderate `
  -Desc 'Windows Update stops swapping your GPU/chipset drivers for its own versions. Install drivers from the vendor instead.' `
  -Reg @(
      Rg $WU 'ExcludeWUDriversInQualityUpdate' 1
      Rg 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\DriverSearching' 'SearchOrderConfig' 0
  )

T -Id 'wu-pause' -Cat $D -Name 'Windows Update: pause all updates for 35 days' -Tags optin -Risk Moderate `
  -Desc 'Uses the same pause Settings offers (maximum 35 days). Updates resume automatically afterwards. Undo resumes immediately.' `
  -Apply {
      $now = [DateTime]::UtcNow
      $start = $now.ToString('yyyy-MM-ddTHH:mm:ssZ'); $end = $now.AddDays(35).ToString('yyyy-MM-ddTHH:mm:ssZ')
      foreach ($n in 'PauseFeatureUpdatesStartTime', 'PauseQualityUpdatesStartTime', 'PauseUpdatesStartTime') { Set-RegEntry 'HKLM:\SOFTWARE\Microsoft\WindowsUpdate\UX\Settings' $n $start 'String' }
      foreach ($n in 'PauseFeatureUpdatesEndTime', 'PauseQualityUpdatesEndTime', 'PauseUpdatesExpiryTime') { Set-RegEntry 'HKLM:\SOFTWARE\Microsoft\WindowsUpdate\UX\Settings' $n $end 'String' }
      $null
  } `
  -Undo {
      foreach ($n in 'PauseFeatureUpdatesStartTime', 'PauseQualityUpdatesStartTime', 'PauseUpdatesStartTime', 'PauseFeatureUpdatesEndTime', 'PauseQualityUpdatesEndTime', 'PauseUpdatesExpiryTime') { Remove-RegEntry 'HKLM:\SOFTWARE\Microsoft\WindowsUpdate\UX\Settings' $n }
  } `
  -Test {
      $e = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\WindowsUpdate\UX\Settings' -ErrorAction SilentlyContinue).PauseUpdatesExpiryTime
      [bool]$e -and ([DateTime]::Parse($e).ToUniversalTime() -gt [DateTime]::UtcNow)
  }

T -Id 'net-hosts-spy' -Cat $N -Name 'Block Windows telemetry and ad servers in the hosts file' -Tags optin -Risk Advanced `
  -Desc 'Downloads the WindowsSpyBlocker "spy" list (MIT, github.com/crazy-max/WindowsSpyBlocker), drops entries that would break developer tooling, and adds the rest to your hosts file between marker lines. The file is backed up first and undo removes only that block. Windows Defender may flag hosts-file edits that block Microsoft domains.' `
  -Apply {
      $hosts = "$env:SystemRoot\System32\drivers\etc\hosts"
      [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
      $raw = (Invoke-WebRequest -Uri 'https://raw.githubusercontent.com/crazy-max/WindowsSpyBlocker/master/data/hosts/spy.txt' -UseBasicParsing -TimeoutSec 25 -ErrorAction Stop).Content
      $entries = @($raw -split "`n" | ForEach-Object { if ($_ -match '^\s*0\.0\.0\.0\s+([a-z0-9][a-z0-9\.\-]+)\s*$') { $Matches[1] } } |
                   Where-Object { $_ -notmatch 'blob\.core\.windows\.net$|visualstudio|nuget|dotnet' } | Sort-Object -Unique)
      if ($entries.Count -lt 50) { throw "blocklist looked wrong ($($entries.Count) entries), hosts file left untouched" }
      $backup = Join-Path $OptiBackupDir ('hosts-{0}.bak' -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
      Copy-Item -LiteralPath $hosts -Destination $backup -Force
      Add-HostsBlock -Path $hosts -Entries $entries
      ipconfig /flushdns | Out-Null
      @{ Backup = $backup; Count = $entries.Count }
  } `
  -Undo {
      $hosts = "$env:SystemRoot\System32\drivers\etc\hosts"
      Remove-HostsBlock -Path $hosts
      ipconfig /flushdns | Out-Null
  } `
  -Test { ([IO.File]::ReadAllText("$env:SystemRoot\System32\drivers\etc\hosts")) -match '# BEGIN Optimaxer telemetry block' }
