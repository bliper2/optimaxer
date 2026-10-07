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
