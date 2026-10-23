# Additional tweaks (system tuning, shell latency, telemetry, update behavior). Same schema as Catalog.ps1.
# The ideas follow the kinds of changes shipped by tuned Windows builds such as AtlasOS; every tweak here is
# implemented independently with snapshot/undo and honest detection.

$global:SHELLBAGS = 'HKCU:\SOFTWARE\Classes\Local Settings\Software\Microsoft\Windows\Shell'
$global:SVCROOT   = 'HKLM:\SYSTEM\CurrentControlSet\Services'
$WU        = 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate'

# ======================================================================= PERFORMANCE
T -Id 'perf-fth' -Cat $P -Name 'Disable Fault Tolerant Heap' -Tags optin -Risk Moderate `
  -Desc 'FTH applies slower, safer heap handling to apps that crashed several times. This sets Enabled=0; undo restores the old value. The list of apps FTH already tracks is left as it is (resetting it cannot be undone). Not recommended on Windows 11 unless an app is stuck in FTH mode.' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Microsoft\FTH' 'Enabled' 0 )

T -Id 'perf-bgapps' -Cat $P -Name 'Disable background apps' -Tags optin -Risk Advanced `
  -Desc 'Store apps no longer run in the background, including the LetAppsRunInBackground=2 policy that forces it off for every account. Not recommended on Windows 11: Mail, Calendar, Teams, Phone Link and other Store apps stop delivering notifications, and modern Windows already suspends idle background apps.' `
  -Reg @(
      Rg "$CV\BackgroundAccessApplications" 'GlobalUserDisabled' 1
      Rg "$CV\Search" 'BackgroundAppGlobalToggle' 0
      Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\AppPrivacy' 'LetAppsRunInBackground' 2
  )

T -Id 'perf-sleepstudy' -Cat $P -Name 'Disable SleepStudy and power diagnostic logging' -Tags max `
  -Desc 'Turns off Modern Standby / power diagnostic event logs and the Power Efficiency Diagnostics task. Less background disk writes. Undo puts each log back to the state it had before (on or off).' `
  -Task @( 'Microsoft\Windows\Power Efficiency Diagnostics\AnalyzeSystem' ) `
  -Snapshot { Get-EventLogStates $global:OptiSleepLogs } `
  -Apply { param($d) foreach ($l in $global:OptiSleepLogs) { if ($null -ne (Get-EventLogState $l)) { Invoke-Wevtutil @('set-log', $l, '/e:false') } } } `
  -Undo  { param($d) Restore-EventLogStates $d }

T -Id 'perf-paging' -Cat $P -Name 'Keep kernel in RAM (16 GB+ only)' -Tags optin -Risk Advanced -Restart -MinRamGB 15.5 `
  -Desc 'DisablePagingExecutive + DisablePageCombining: kernel and drivers are never paged out. Not recommended on Windows 11: it raises committed memory for little measurable gain, because Windows rarely pages kernel code on a PC with enough RAM.' `
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

T -Id 'perf-foldertype' -Cat $P -Name 'Explorer: stop auto-detecting folder types' -Tags max -Explorer `
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

T -Id 'perf-maintenance' -Cat $P -Name 'Automatic Maintenance: never wake the PC' -Tags max `
  -Desc 'Scheduled maintenance still runs when the PC is on, but it can no longer wake it from sleep at night.' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\Task Scheduler\Maintenance' 'WakeUp' 0 )

T -Id 'perf-iconcache' -Cat $P -Name 'Bigger icon cache (4 MB)' -Tags max -Explorer `
  -Desc 'Explorer keeps more icons in memory, so big folders redraw faster.' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer' 'Max Cached Icons' '4096' 'String' )

T -Id 'perf-hover' -Cat $P -Name 'Instant hover tooltips' -Tags max `
  -Desc 'Mouse hover delay for tooltips and file info drops from 400 ms to 20 ms.' `
  -Reg @( Rg 'HKCU:\Control Panel\Mouse' 'MouseHoverTime' '20' 'String' )

T -Id 'perf-wpbt' -Cat $P -Name 'Block WPBT (firmware-injected programs)' -Tags max -Restart `
  -Desc 'Stops firmware from dropping and running OEM software on every boot (the Windows Platform Binary Table). Also a security hardening.' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager' 'DisableWpbtExecution' 1 )

T -Id 'perf-forceend' -Cat $P -Name 'Force-close apps on shutdown and sign-out' -Tags optin -Risk Moderate `
  -Desc 'Apps are closed immediately without the "save changes?" wait. Faster, but unsaved work is lost.' `
  -Reg @( Rg $DESK 'AutoEndTasks' '1' 'String' )

T -Id 'perf-aeroshake' -Cat $P -Name 'Disable Aero Shake' -Tags max `
  -Desc 'Shaking a window no longer minimizes all the others (usually triggered by accident).' `
  -Reg @( Rg $EXA 'DisallowShaking' 1 )

# ======================================================================= NETWORK
T -Id 'net-smbthrottle' -Cat $N -Name 'SMB: remove bandwidth throttling' -Tags max `
  -Desc 'Network share transfers are no longer rate-limited by the client. Helps on fast LANs and NAS copies.' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Services\LanmanWorkstation\Parameters' 'DisableBandwidthThrottling' 1 )

# ======================================================================= PRIVACY
T -Id 'priv-experiment' -Cat $V -Name 'Opt out of Microsoft experiments' -Tags privacy, max `
  -Desc 'Stops Microsoft from using your PC as a test subject for unfinished features.' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Microsoft\PolicyManager\default\System\AllowExperimentation' 'value' 0 )

T -Id 'priv-pca' -Cat $V -Name 'Disable Program Compatibility Assistant' -Tags privacy, max `
  -Desc 'No more "this program might not have installed correctly" popups and app-usage reporting.' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\AppCompat' 'AITEnable' 0; Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\AppCompat' 'DisablePCA' 1 )

T -Id 'priv-perftrack' -Cat $V -Name 'Disable responsiveness tracking' -Tags privacy, max `
  -Desc 'Stops Windows from collecting and reporting app responsiveness events (Windows Diagnostic Infrastructure).' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\WDI\{9c5a40da-b965-4fc3-8781-88dd50a6299d}' 'ScenarioExecutionEnabled' 0 )

T -Id 'priv-mru' -Cat $V -Name 'Disable most-used-apps tracking' -Tags privacy, max `
  -Desc 'Start no longer builds a "most used" list from what you launch.' `
  -Reg @( Rg "$CV\Policies\Explorer" 'NoInstrumentation' 1 )

T -Id 'priv-rsop' -Cat $V -Name 'Disable RSoP policy logging' -Tags privacy, max `
  -Desc 'Stops logging of resultant-set-of-policy data after every Group Policy refresh.' `
  -Reg @( Rg "$POL\System" 'RSoPLogging' 0 )

T -Id 'priv-devicehealth' -Cat $V -Name 'Disable device health attestation reporting' -Tags privacy, max `
  -Desc 'Stops device-health reporting to Microsoft at startup.' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Policies\Microsoft\DeviceHealthAttestationService' 'EnableDeviceHealthAttestationService' 0 )

T -Id 'priv-ceip' -Cat $V -Name 'Disable Customer Experience Improvement Program' -Tags privacy, max `
  -Desc 'Turns off CEIP / SQM data collection for Windows and App-V.' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Policies\Microsoft\SQMClient\Windows' 'CEIPEnable' 0; Rg 'HKLM:\SOFTWARE\Policies\Microsoft\AppV\CEIP' 'CEIPEnable' 0 )

T -Id 'priv-difftrace' -Cat $V -Name 'Disable diagnostic tracing' -Tags privacy, max `
  -Desc 'Turns off Windows performance diagnostic tracing sessions.' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\Diagnostics\Performance' 'DisableDiagnosticTracing' 1 )

T -Id 'priv-dotnet' -Cat $V -Name 'Opt out of .NET CLI telemetry' -Tags privacy, max `
  -Desc 'Sets DOTNET_CLI_TELEMETRY_OPTOUT=1 system-wide for developers using dotnet.' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Environment' 'DOTNET_CLI_TELEMETRY_OPTOUT' '1' 'String' )

T -Id 'priv-oobe' -Cat $V -Name 'Skip post-update privacy re-prompt' -Tags privacy, max `
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

T -Id 'priv-msrt' -Cat $V -Name 'Stop Malicious Software Removal Tool reporting' -Tags privacy, max `
  -Desc 'MSRT still removes malware but stops sending infection reports.' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Policies\Microsoft\MRT' 'DontReportInfectionInformation' 1 )

# ======================================================================= WINDOWS UPDATE BEHAVIOR (updates stay on)
T -Id 'wu-noreboot' -Cat $D -Name 'Windows Update: no forced restarts while signed in' -Tags max `
  -Desc 'Sets the NoAutoRebootWithLoggedOnUsers policy so scheduled update restarts wait until you sign out. Updates still install. It is not a guarantee: Windows can still show restart reminders and restart after its deadlines.' `
  -Reg @( Rg "$WU\AU" 'NoAutoRebootWithLoggedOnUsers' 1 )

T -Id 'wu-insider' -Cat $D -Name 'Block Windows Insider enrollment' -Tags max `
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

T -Id 'ai-recall' -Cat $AI -Name 'Turn off Recall and AI data analysis' -Tags privacy, max, deai -Restart `
  -Desc 'Policy blocks Recall snapshots and AI data analysis for you and the machine; the Recall Windows feature is switched off where it exists (Copilot+ PCs).' `
  -Reg @(
      Rg $WAI 'DisableAIDataAnalysis' 1
      Rg $WAI 'AllowRecallEnablement' 0
      Rg $WAI 'TurnOffSavingSnapshots' 1
      Rg 'HKCU:\SOFTWARE\Policies\Microsoft\Windows\WindowsAI' 'DisableAIDataAnalysis' 1
  ) `
  -Apply {
      $f = Get-WindowsOptionalFeature -Online -FeatureName Recall -ErrorAction SilentlyContinue
      if ($f -and $f.State -eq 'Enabled') { Disable-WindowsOptionalFeature -Online -FeatureName Recall -NoRestart -ErrorAction SilentlyContinue | Out-Null; @{ WasEnabled = $true } } else { @{ WasEnabled = $false } }
  } `
  -Undo { param($d) if ($d -and $d.WasEnabled) { Enable-WindowsOptionalFeature -Online -FeatureName Recall -NoRestart -ErrorAction SilentlyContinue | Out-Null } }

T -Id 'ai-clicktodo' -Cat $AI -Name 'Turn off Click to Do and AI search suggestions' -Tags privacy, max, deai `
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

T -Id 'ai-apps' -Cat $AI -Name 'Notepad, Paint and Photos: turn off AI features' -Tags privacy, max, deai `
  -Desc 'Group-policy switches that hide Notepad AI (Rewrite/Summarize), Paint Cocreator, Generative Fill and Image Creator. Which switches apply depends on the app version.' `
  -Reg @(
      Rg 'HKLM:\SOFTWARE\Policies\WindowsNotepad' 'DisableAIFeatures' 1
      Rg 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\Paint' 'DisableCocreator' 1
      Rg 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\Paint' 'DisableGenerativeFill' 1
      Rg 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\Paint' 'DisableImageCreator' 1
      Rg 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\Paint' 'DisableGenerativeErase' 1
      Rg 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\Paint' 'DisableRemoveBackground' 1
  )

T -Id 'ai-edge' -Cat $AI -Name 'Edge: turn off Copilot sidebar and page context' -Tags privacy, max, deai `
  -Desc 'Edge policies: hides the sidebar/Copilot hub, stops Copilot reading page content, hides the Microsoft 365 Copilot icon.' `
  -Reg @(
      Rg $EDGEP 'HubsSidebarEnabled' 0
      Rg $EDGEP 'CopilotCDPPageContext' 0
      Rg $EDGEP 'CopilotPageContext' 0
      Rg $EDGEP 'Microsoft365CopilotChatIconEnabled' 0
      Rg $EDGEP 'EdgeEntraCopilotPageContext' 0
      Rg $EDGEP 'EdgeHistoryAISearchEnabled' 0
      Rg $EDGEP 'ComposeInlineEnabled' 0
      Rg $EDGEP 'GenAILocalFoundationalModelSettings' 1
      Rg $EDGEP 'NewTabPageBingChatEnabled' 0
  )

T -Id 'ai-cloudsearch' -Cat $AI -Name 'Search: no cloud account content or search highlights' -Tags privacy, max, deai `
  -Desc 'Stops Windows Search from querying your Microsoft/work account cloud content and removes the daily search highlights (doodles, trending topics) from the search box.' `
  -Reg @(
      Rg "$CV\SearchSettings" 'IsMSACloudSearchEnabled' 0
      Rg "$CV\SearchSettings" 'IsAADCloudSearchEnabled' 0
      Rg "$CV\SearchSettings" 'IsDynamicSearchBoxEnabled' 0
      Rg "$POL\Windows Search" 'EnableDynamicContentInWSB' 0
  )

T -Id 'ai-tailored' -Cat $AI -Name 'No tailored experiences from your diagnostic data' -Tags privacy, max, deai `
  -Desc 'Windows stops using your diagnostic data to personalize tips, ads and recommendations.' `
  -Reg @(
      Rg "$CV\Privacy" 'TailoredExperiencesWithDiagnosticDataEnabled' 0
      Rg 'HKCU:\SOFTWARE\Policies\Microsoft\Windows\CloudContent' 'DisableTailoredExperiencesWithDiagnosticData' 1
  )

$global:OptiAiApps = @('*Copilot*', 'Microsoft.Windows.AIHub', 'MicrosoftWindows.Client.AIX', 'MicrosoftWindows.Client.CoreAI')

T -Id 'ai-remove-apps' -Cat $AI -Name 'Uninstall the Copilot and AI app packages' -Tags max, deai -Risk Moderate `
  -Desc 'Removes every installed Copilot / AI Hub package, the Microsoft 365 (Office hub) Copilot launcher and the Store Copilot for all users and from the provisioning list so new accounts do not get them. Also removes the Windows AI core package using the same end-of-life method WinUtil uses. Undo cannot reinstall: use the Microsoft Store.' `
  -Apply {
      # Same approach as WinUtil's "Windows AI - Disable And Remove": marking CoreAI end-of-life lets Windows remove it.
      try {
          $core = Get-AppxPackage -AllUsers -Name 'MicrosoftWindows.Client.CoreAI' -ErrorAction SilentlyContinue | Select-Object -First 1
          if ($core) {
              $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
              New-Item -Path "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Appx\AppxAllUserStore\EndOfLife\$sid\$($core.PackageFullName)" -Force -ErrorAction Stop | Out-Null
          }
      } catch { Write-Log "  could not mark the Windows AI core package for removal: $($_.Exception.Message)" 'WARN' }
      $r = Remove-AppxByName ($global:OptiAiApps + 'Microsoft.MicrosoftOfficeHub')
      foreach ($f in $r.Failed) { Write-Log "  could not remove $f" 'WARN' }
      # Store version of Copilot (Win11Debloat removes it by Store id)
      if (Get-Command winget -ErrorAction SilentlyContinue) {
          $null = winget list --id XP9CXNGPPJ97XX --source msstore --accept-source-agreements 2>&1
          if ($LASTEXITCODE -eq 0) { $null = winget uninstall --id XP9CXNGPPJ97XX --source msstore --silent --accept-source-agreements 2>&1 }
      }
      , @($r.Removed)
  } `
  -Undo { Write-Log '  Copilot apps cannot be restored automatically: reinstall from the Microsoft Store if you want them back.' 'WARN' } `
  -Test { Test-AppxGone ($global:OptiAiApps + 'Microsoft.MicrosoftOfficeHub') }

T -Id 'ai-settingspage' -Cat $AI -Name 'Hide the Settings "AI components" page' -Tags privacy, max, deai `
  -Desc 'Hides Settings > System > AI components (Windows 11 24H2+), as WinUtil does. Other pages you hid with this policy are kept.' `
  -Apply {
      $k = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\Explorer'
      $snap = Get-RegSnapshot @{ P = $k; N = 'SettingsPageVisibility' }
      $cur = if ($snap.Exists) { [string]$snap.Value } else { '' }
      if ($cur -like 'showonly:*') { Write-Log '  Settings page visibility is in "show only" mode; not changed' 'WARN'; return @{ Existed = [bool]$snap.Exists; Value = $cur; Changed = $false } }
      if ($cur -notmatch '(^|[:;])aicomponents(;|$)') {
          $new = if ($cur -like 'hide:*') { $cur.TrimEnd(';') + ';aicomponents' } else { 'hide:aicomponents' }
          Set-RegEntry $k 'SettingsPageVisibility' $new 'String'
      }
      @{ Existed = [bool]$snap.Exists; Value = $cur; Changed = $true }
  } `
  -Undo {
      param($d)
      if (-not $d -or -not $d.Changed) { return }
      $k = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\Explorer'
      if ($d.Existed) { Set-RegEntry $k 'SettingsPageVisibility' $d.Value 'String' } else { Remove-RegEntry $k 'SettingsPageVisibility' }
  } `
  -Test {
      $v = (Get-RegSnapshot @{ P = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\Explorer'; N = 'SettingsPageVisibility' }).Value
      [bool]($v -and ([string]$v -match '(^|[:;])aicomponents(;|$)'))
  }

# ======================================================================= MORE DEBLOAT
T -Id 'debloat-spotlight' -Cat $D -Name 'Turn off Windows Spotlight and lock-screen promos' -Tags privacy, max, deai `
  -Desc 'No Spotlight desktop icon or lock-screen "fun facts", tips and promoted content.' `
  -Reg @(
      Rg "$POL\CloudContent" 'DisableWindowsSpotlightFeatures' 1
      Rg "$POL\CloudContent" 'DisableSpotlightCollectionOnDesktop' 1
      Rg "$POL\CloudContent" 'DisableThirdPartySuggestions' 1
      Rg 'HKCU:\SOFTWARE\Policies\Microsoft\Windows\CloudContent' 'DisableWindowsSpotlightFeatures' 1
  )

T -Id 'debloat-teamsauto' -Cat $D -Name 'Stop Teams (Chat) from auto-installing' -Tags max, deai `
  -Desc 'Prevents Windows from silently installing the consumer Teams app on new sessions and updates.' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Communications' 'ConfigureChatAutoInstall' 0 )

T -Id 'debloat-oobeapps' -Cat $D -Name 'Block forced Outlook and Dev Home installs' -Tags max, deai `
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

# ======================================================================= PORTED FROM WINUTIL / WIN11DEBLOAT (both MIT; see README)
T -Id 'debloat-startrec' -Cat $D -Name 'Start menu: hide the Recommended section' -Tags max `
  -Desc 'Removes "Recommended" (recent files and suggested apps) from the Windows 11 Start menu using the user and machine policies plus the Pro-edition override WinUtil uses. Side effect (WinUtil warns the same): Windows Spotlight on the lock screen stops.' `
  -Reg @(
      Rg 'HKCU:\SOFTWARE\Policies\Microsoft\Windows\Explorer' 'HideRecommendedSection' 1
      Rg "$POL\Explorer" 'HideRecommendedSection' 1
      Rg 'HKLM:\SOFTWARE\Microsoft\PolicyManager\current\device\Start' 'HideRecommendedSection' 1
      Rg 'HKLM:\SOFTWARE\Microsoft\PolicyManager\current\device\Education' 'IsEducationEnvironment' 1
  ) -Explorer

T -Id 'debloat-edge-remove' -Cat $D -Name 'Uninstall Microsoft Edge' -Tags optin -Risk Advanced `
  -Desc 'Uses the method WinUtil uses: a placeholder legacy Edge file unlocks Edge''s own system-level uninstaller, with its profile data deleted. WebView2 (needed by many apps) stays. Windows Update can bring Edge back. Undo reinstalls it through WinGet.' `
  -Apply {
      $setup = @(Get-ChildItem -Path "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\*\Installer\setup.exe" -ErrorAction SilentlyContinue | Select-Object -Last 1)
      if (-not $setup.Count) { Write-Log '  Edge is not installed' ; return $null }
      Stop-Process -Name 'msedge' -Force -ErrorAction SilentlyContinue
      New-Item -Path "$env:SystemRoot\SystemApps\Microsoft.MicrosoftEdge_8wekyb3d8bbwe\MicrosoftEdge.exe" -Force -ErrorAction SilentlyContinue | Out-Null
      $proc = Start-Process -FilePath $setup[0].FullName -ArgumentList '--uninstall --system-level --force-uninstall --delete-profile' -PassThru -WindowStyle Hidden -ErrorAction SilentlyContinue
      if ($proc -and -not $proc.WaitForExit(240000)) { Write-Log '  Edge uninstaller still running after 4 minutes' 'WARN' }
      $null
  } `
  -Undo { Invoke-WingetPackages -Action Install -Programs @('Microsoft.Edge') } `
  -Test { -not ((Test-Path "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe") -or (Test-Path "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe")) }

T -Id 'sys-utc' -Cat $P -Name 'Hardware clock in UTC (dual boot with Linux)' -Tags optin `
  -Desc 'Windows keeps the hardware clock in UTC like Linux does, so the time stays right when you dual boot (WinUtil).' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\TimeZoneInformation' 'RealTimeIsUniversal' 1 'QWord' )

T -Id 'ui-bsod' -Cat $I -Name 'Detailed blue-screen information' -Tags max `
  -Desc 'Shows the stop code and parameters on a blue screen instead of the smiley face (WinUtil).' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\CrashControl' 'DisplayParameters' 1; Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\CrashControl' 'DisableEmoticon' 1 )

T -Id 'ui-battery' -Cat $I -Name 'Show battery percentage in the system tray' -Explorer `
  -Desc 'Adds the numeric percentage next to the battery icon (WinUtil).' `
  -Reg @( Rg $EXA 'IsBatteryPercentageEnabled' 1 )

T -Id 'ui-loginblur' -Cat $I -Name 'Sharp sign-in screen background (no acrylic blur)' `
  -Desc 'Turns off the blur on the sign-in screen background (WinUtil).' `
  -Reg @( Rg "$POL\System" 'DisableAcrylicBackgroundOnLogon' 1 )

T -Id 'ui-nosnap' -Cat $I -Name 'Turn off window snapping' -Tags optin `
  -Desc 'Windows stop snapping and resizing when dragged to screen edges (Win11Debloat / WinUtil).' `
  -Reg @( Rg $DESK 'WindowArrangementActive' '0' 'String' )

# Device-software blockers: Windows Update keeps pushing these helper installers. An empty folder with a deny-write ACL stops them.
function Test-FolderWriteDenied {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) { return $false }
    try {
        foreach ($a in (Get-Acl -LiteralPath $Path).Access) {
            if ($a.AccessControlType -eq 'Deny' -and $a.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value -eq 'S-1-1-0') { return $true }
        }
    } catch {}
    return $false
}

T -Id 'debloat-razer' -Cat $D -Name 'Block Razer software auto-install' -Tags optin `
  -Desc 'Stops Windows from silently installing Razer Synapse when a Razer device is plugged in, by locking the Razer installer folder (WinUtil). The hardware keeps working. Undo unlocks the folder.' `
  -Apply {
      $d = "$env:SystemRoot\Installer\Razer"
      if (Test-Path -LiteralPath $d) { Get-ChildItem -LiteralPath $d -Force -ErrorAction SilentlyContinue | Remove-Item -Recurse -Force -ErrorAction SilentlyContinue }
      else { New-Item -Path $d -ItemType Directory -Force | Out-Null }
      & icacls.exe $d /deny '*S-1-1-0:(W)' 2>&1 | Out-Null
      $null
  } `
  -Undo { & icacls.exe "$env:SystemRoot\Installer\Razer" /remove:d '*S-1-1-0' 2>&1 | Out-Null } `
  -Test { Test-FolderWriteDenied "$env:SystemRoot\Installer\Razer" }

T -Id 'debloat-logitech' -Cat $D -Name 'Block Logitech Download Assistant auto-install' -Tags optin `
  -Desc 'Stops Windows Update from reinstalling the Logi Download Assistant with Logitech drivers, by locking its folder (WinUtil). Logitech hardware keeps working. Undo unlocks the folder.' `
  -Apply {
      Stop-Process -Name 'logi_download_assistant' -Force -ErrorAction SilentlyContinue
      $root = if ($env:ProgramW6432) { $env:ProgramW6432 } else { $env:ProgramFiles }
      $d = Join-Path $root 'LogiDownloadAssistant'
      if (Test-Path -LiteralPath $d) { Get-ChildItem -LiteralPath $d -Force -ErrorAction SilentlyContinue | Remove-Item -Recurse -Force -ErrorAction SilentlyContinue }
      else { New-Item -Path $d -ItemType Directory -Force | Out-Null }
      & icacls.exe $d /deny '*S-1-1-0:(W)' 2>&1 | Out-Null
      $null
  } `
  -Undo {
      $root = if ($env:ProgramW6432) { $env:ProgramW6432 } else { $env:ProgramFiles }
      & icacls.exe (Join-Path $root 'LogiDownloadAssistant') /remove:d '*S-1-1-0' 2>&1 | Out-Null
  } `
  -Test {
      $root = if ($env:ProgramW6432) { $env:ProgramW6432 } else { $env:ProgramFiles }
      Test-FolderWriteDenied (Join-Path $root 'LogiDownloadAssistant')
  }

# ======================================================================= BROWSERS (enterprise policies; browsers show "managed by your organization")
$B = 'Browsers'
$CHR = 'HKLM:\SOFTWARE\Policies\Google\Chrome'
$BRV = 'HKLM:\SOFTWARE\Policies\BraveSoftware\Brave'
$EDG = 'HKLM:\SOFTWARE\Policies\Microsoft\Edge'
$FFX = 'HKLM:\SOFTWARE\Policies\Mozilla\Firefox'
$VIV = 'HKLM:\SOFTWARE\Policies\Vivaldi'
$NOTE = ' Applied as browser policy, so the browser shows "managed by your organization". Undo removes it.'

T -Id 'br-chrome-debloat' -Cat $B -Name 'Chrome: no telemetry, background mode or promos' -Tags privacy, max, deai `
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

T -Id 'br-chrome-ai' -Cat $B -Name 'Chrome: turn off Gemini and generative AI features' -Tags privacy, max, deai `
  -Desc ('Disables Gemini integration, Help me write, tab organizer, AI themes, tab compare and the AI-mode omnibox entry.' + $NOTE) `
  -Reg @(
      Rg $CHR 'GenAiDefaultSettings' 2
      Rg $CHR 'GeminiSettings' 1
      Rg $CHR 'HelpMeWriteSettings' 2
      Rg $CHR 'TabOrganizerSettings' 2
      Rg $CHR 'CreateThemesSettings' 2
      Rg $CHR 'TabCompareSettings' 2
      Rg $CHR 'AIModeSettings' 1
      Rg $CHR 'AutofillPredictionSettings' 2
      Rg $CHR 'ChromeSuggestionsSettings' 1
      Rg $CHR 'DevToolsGenAiSettings' 2
      Rg $CHR 'FindAndFillWithGeminiSettings' 2
      Rg $CHR 'GeminiSparkSettings' 1
      Rg $CHR 'GenAILocalFoundationalModelSettings' 1
      Rg $CHR 'HistorySearchSettings' 2
      Rg $CHR 'SearchContentSharingSettings' 1
      Rg $CHR 'SmartTabSharingSettings' 1
      Rg $CHR 'ThirdPartyAiChatSettings' 1
      Rg $CHR 'VoiceTypingSettings' 2
  )

T -Id 'br-brave-debloat' -Cat $B -Name 'Brave: remove Rewards, Wallet, VPN, News, Talk and AI Chat' -Tags privacy, max, deai `
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
      Rg $BRV 'BraveP3AEnabled' 0
      Rg $BRV 'UrlKeyedAnonymizedDataCollectionEnabled' 0
  )

T -Id 'br-edge-debloat' -Cat $B -Name 'Edge: remove new-tab feed, workspaces, Bing ads and promos' -Tags privacy, max, deai `
  -Desc ('Extra Edge policies on top of "Edge: disable nags": new-tab news feed, promotional tabs, workspaces, Bing ads, default-browser nag, Do Not Track, metrics, and the Microsoft Editor sidebar extension WinUtil blocks.' + $NOTE) `
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
      Rg $EDG 'TabServicesEnabled' 0
      Rg $EDG 'AlternateErrorPagesEnabled' 0
      Rg $EDG 'WalletDonationEnabled' 0
      Rg $EDG 'DefaultBrowserSettingsCampaignEnabled' 0
      Rg $EDG 'NewTabPageHideDefaultTopSites' 1
      Rg $EDG 'EdgeCollectionsEnabled' 0
      Rg $EDG 'EdgeAssetDeliveryServiceEnabled' 0
      Rg $EDG 'DiagnosticData' 0
      Rg $EDG 'HideFirstRunExperience' 1
      Rg $EDG 'ConfigureDoNotTrack' 1
      Rg 'HKLM:\SOFTWARE\Policies\Microsoft\EdgeUpdate' 'CreateDesktopShortcutDefault' 0
      Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Edge\ExtensionInstallBlocklist' '1' 'ofefcgjbeghpigppfmkologfjadafddi' 'String'
  )

T -Id 'br-firefox-debloat' -Cat $B -Name 'Firefox: no telemetry, studies, Pocket, sponsored content or nags' -Tags privacy, max, deai `
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

T -Id 'br-firefox-ai' -Cat $B -Name 'Firefox: turn off AI chatbot, link previews and AI tab groups' -Tags privacy, max, deai `
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

T -Id 'ui-sharetray' -Cat $I -Name 'Disable the Share drag tray' -Tags max `
  -Desc 'Stops the "drag files here to share" tray that appears when dragging files to the top of the screen.' `
  -Reg @( Rg "$CV\CDP" 'DragTrayEnabled' 0 )

T -Id 'ui-phonelink-start' -Cat $I -Name 'Hide Phone Link in the Start menu' -Tags max `
  -Desc 'Removes the Phone Link companion panel from Start.' `
  -Reg @( Rg "$CV\Start\Companions\Microsoft.YourPhone_8wekyb3d8bbwe" 'IsEnabled' 0 )

T -Id 'ui-legacyboot' -Cat $I -Name 'Classic boot menu (F8 advanced options)' -Risk Moderate `
  -Desc 'bcdedit bootmenupolicy legacy: F8 works at boot for Safe Mode. Undo sets it back to standard.' `
  -Apply { bcdedit /set '{default}' bootmenupolicy legacy | Out-Null; $null } `
  -Undo  { bcdedit /set '{default}' bootmenupolicy standard | Out-Null } `
  -Test  { (bcdedit /enum '{current}' | Out-String) -match 'bootmenupolicy\s+Legacy' }

# ---- privacy / debloat
T -Id 'priv-searchhistory' -Cat $V -Name 'Disable device search history' -Tags privacy, max `
  -Desc 'Windows stops remembering what you search for on this device.' `
  -Reg @( Rg "$CV\SearchSettings" 'IsDeviceSearchHistoryEnabled' 0 )

T -Id 'priv-speech' -Cat $V -Name 'Disable online speech recognition' -Tags privacy, max `
  -Desc 'Voice data is no longer sent to Microsoft for cloud speech recognition.' `
  -Reg @( Rg 'HKCU:\Software\Microsoft\Speech_OneCore\Settings\OnlineSpeechPrivacy' 'HasAccepted' 0 )

T -Id 'priv-findmydevice' -Cat $V -Name 'Disable Find My Device' -Tags privacy -Risk Moderate `
  -Desc 'Stops location reporting for Find My Device. You lose the ability to locate this PC from your Microsoft account.' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Policies\Microsoft\FindMyDevice' 'AllowFindMyDevice' 0 )

T -Id 'priv-anon' -Cat $V -Name 'Block anonymous enumeration of accounts and shares' -Tags max `
  -Desc 'Security hardening: anonymous users can no longer list accounts or share names.' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\Lsa' 'RestrictAnonymous' 1; Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\Lsa' 'RestrictAnonymousSAM' 1 )

T -Id 'debloat-devicemeta' -Cat $D -Name 'Do not auto-download device apps from manufacturers' -Tags max, deai `
  -Desc 'Windows stops fetching vendor companion apps and icons when you plug in hardware.' `
  -Reg @( Rg "$POL\Device Metadata" 'PreventDeviceMetadataFromNetwork' 1 )

T -Id 'debloat-settings365' -Cat $D -Name 'Remove Microsoft 365 and account promos from Settings' -Tags max, deai `
  -Desc 'Hides the consumer account-state banners and offers in the Settings app.' `
  -Reg @( Rg "$POL\CloudContent" 'DisableConsumerAccountStateContent' 1 )

T -Id 'debloat-suggestions2' -Cat $D -Name 'Turn off remaining Windows suggestions and nag toasts' -Tags safe, privacy, max, deai `
  -Desc 'The rest of the Win11Debloat suggestions list: Start tips, account notifications, OneDrive ads, backup reminders, suggested toasts and Phone Link nags.' `
  -Reg @(
      Rg $CDM 'SubscribedContent-353698Enabled' 0
      Rg $EXA 'Start_IrisRecommendations' 0
      Rg $EXA 'Start_AccountNotifications' 0
      Rg "$CV\SystemSettings\AccountNotifications" 'EnableAccountNotifications' 0
      Rg $EXA 'ShowSyncProviderNotifications' 0
      Rg "$CV\Notifications\Settings\Windows.SystemToast.Suggested" 'Enabled' 0
      Rg "$CV\Notifications\Settings\Windows.SystemToast.BackupReminder" 'Enabled' 0
      Rg "$CV\Mobility" 'OptedIn' 0
  )

T -Id 'debloat-aiservice' -Cat $D -Name 'AI Fabric service: start on demand only' -Tags max, deai `
  -Desc 'Sets the Windows AI Fabric service (WSAIFabricSvc) to Manual so it does not start with Windows.' `
  -Svc @( Sx 'WSAIFabricSvc' 3 )

T -Id 'wu-asap' -Cat $D -Name 'Windows Update: do not get new features as soon as available' -Tags max `
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

T -Id 'ui-officefiles' -Cat $I -Name 'No cloud/Office files in Quick access' -Tags max `
  -Desc 'Quick access stops listing recent Office/cloud documents.' `
  -Reg @( Rg "$CV\Explorer" 'ShowCloudFilesInQuickAccess' 0 )

T -Id 'ui-autoplay' -Cat $I -Name 'Disable AutoPlay for drives and devices' -Tags max `
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

T -Id 'perf-shortcuts' -Cat $P -Name 'Do not search for missing shortcut targets' -Tags max `
  -Desc 'Windows stops hunting the whole disk when a shortcut is broken, which can freeze Explorer for seconds.' `
  -Reg @( Rg "$CV\Policies\Explorer" 'NoResolveSearch' 1; Rg "$CV\Policies\Explorer" 'NoResolveTrack' 1 )

T -Id 'perf-openwith' -Cat $P -Name 'Do not look up apps online for unknown file types' -Tags max `
  -Desc 'Opening an unknown file no longer queries Microsoft for "Look for an app in the Store".' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\Explorer' 'NoInternetOpenWith' 1 )

T -Id 'perf-nicpower' -Cat $P -Name 'Network adapters: never power down to save energy' -Tags max -DesktopOnly `
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

T -Id 'sec-nullsess' -Cat $N -Name 'Restrict anonymous (null session) access to shares' -Tags max `
  -Desc 'Security hardening: anonymous connections can no longer reach named pipes and shares.' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Services\LanManServer\Parameters' 'RestrictNullSessAccess' 1 )

T -Id 'priv-settingtips' -Cat $V -Name 'Disable online tips in Settings' -Tags safe, privacy, max `
  -Desc 'Settings stops fetching tips and "did you know" banners from Microsoft.' `
  -Reg @(
      Rg 'HKLM:\SOFTWARE\Microsoft\PolicyManager\default\Settings\AllowOnlineTips' 'value' 0
      Rg 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\Explorer' 'AllowOnlineTips' 0
  )

T -Id 'priv-lockcamera' -Cat $V -Name 'Disable camera on the lock screen' -Tags privacy, max `
  -Desc 'Swiping on the lock screen can no longer open the camera.' `
  -Reg @( Rg "$POL\Personalization" 'NoLockScreenCamera' 1 )

T -Id 'priv-speechmodel' -Cat $V -Name 'Stop automatic speech-model downloads' -Tags privacy, max `
  -Desc 'Windows no longer downloads speech data updates in the background.' `
  -Reg @( Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Speech' 'AllowSpeechModelUpdate' 0 )

T -Id 'priv-settingsync' -Cat $V -Name 'Disable settings sync to your Microsoft account' -Tags privacy -Risk Moderate `
  -Desc 'Themes, passwords-metadata and preferences are no longer synced across your devices.' `
  -Reg @( Rg "$POL\SettingSync" 'DisableSettingSync' 2; Rg "$POL\SettingSync" 'DisableSettingSyncUserOverride' 1 )

T -Id 'priv-msgsync' -Cat $V -Name 'Disable message cloud sync' -Tags privacy, max `
  -Desc 'Blocks SMS/message sync to the cloud.' `
  -Reg @( Rg "$POL\Messaging" 'AllowMessageSync' 0 )

T -Id 'priv-nvidia' -Cat $V -Name 'NVIDIA: opt out of telemetry' -Tags privacy, max `
  -Desc 'Turns off NVIDIA Control Panel usage telemetry and the NVIDIA telemetry container service (if installed).' `
  -Reg @( Rg 'HKCU:\Software\NVIDIA Corporation\NVControlPanel2\Client' 'OptInOrOutPreference' 0 ) `
  -Svc @( Sx 'NvTelemetryContainer' 4 )

T -Id 'priv-office' -Cat $V -Name 'Microsoft Office: opt out of telemetry and feedback' -Tags privacy, max `
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

T -Id 'wu-nag' -Cat $D -Name 'Windows Update: remove "update and shut down" nag' -Tags max `
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

T -Id 'audio-loudnesseq' -Cat $P -Name 'Unhide "Loudness Equalization" on audio devices' -Tags optin -Risk Moderate `
  -Desc 'Some audio drivers hide the Loudness Equalization checkbox (Sound > device Properties > Enhancements). Adds the FxProperties value that unhides it on every render endpoint; restarts the audio service. You still enable the checkbox yourself.' `
  -Apply {
      $base = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Render'
      $prop = '{d04e05a6-594b-4fb6-a80d-01af5eed7d1d},3'
      $prev = @()
      foreach ($dev in Get-ChildItem -Path $base -ErrorAction SilentlyContinue) {
          $fx = Join-Path $dev.PSPath 'FxProperties'
          $had = Test-Path -LiteralPath $fx
          $old = if ($had) { (Get-ItemProperty -LiteralPath $fx -Name $prop -ErrorAction SilentlyContinue).$prop } else { $null }
          $prev += @{ Path = $fx; Had = $had; Old = $old }
          try { Set-RegEntry -Path $fx -Name $prop -Value '{5860E1C5-F95C-4a7a-8EC8-8AEF24F379A1}' -Type String } catch {}
      }
      try { Restart-Service audiosrv -Force -ErrorAction Stop } catch {}
      , $prev
  } `
  -Undo {
      param($d)
      $prop = '{d04e05a6-594b-4fb6-a80d-01af5eed7d1d},3'
      foreach ($p in @($d)) {
          if (-not $p.Had) { Remove-RegEntry -Path $p.Path -Name $prop }
          elseif ($null -ne $p.Old) { try { Set-RegEntry -Path $p.Path -Name $prop -Value $p.Old -Type String } catch {} }
      }
      try { Restart-Service audiosrv -Force -ErrorAction Stop } catch {}
  } `
  -Test {
      $base = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Render'
      $prop = '{d04e05a6-594b-4fb6-a80d-01af5eed7d1d},3'
      $any = $false
      foreach ($dev in Get-ChildItem -Path $base -ErrorAction SilentlyContinue) {
          $fx = Join-Path $dev.PSPath 'FxProperties'
          if ((Get-ItemProperty -LiteralPath $fx -Name $prop -ErrorAction SilentlyContinue).$prop -eq '{5860E1C5-F95C-4a7a-8EC8-8AEF24F379A1}') { $any = $true }
      }
      $any
  }

# ======================================================================= 2026 MODULES (WinUtil + Win11Debloat, both MIT)
# Registry values and behavior follow those projects. Original values are snapshotted, so undo restores this PC.

T -Id 'game-barpop' -Cat $G -Name 'Stop Game Bar popups when a game or controller opens' -Tags max `
  -Desc 'Adapted from Win11Debloat: points the ms-gamebar and ms-gamebarservices links at systray so Windows stops opening Game Bar when a game launches or a controller connects. Your previous registration is exported first.' `
  -Apply {
      $done = @()
      foreach ($proto in 'ms-gamebar', 'ms-gamebarservices') {
          $native = "HKEY_CURRENT_USER\Software\Classes\$proto"
          $ps = "HKCU:\Software\Classes\$proto"
          $existed = Test-Path -LiteralPath $ps
          $file = $null
          if ($existed) {
              $file = Join-Path $global:OptiBackupDir ("gamebar-{0}-{1}.reg" -f $proto, (Get-Date -Format 'yyyyMMdd-HHmmss'))
              reg.exe export $native $file /y | Out-Null
              if (-not (Test-Path -LiteralPath $file)) { throw "could not back up $proto, left untouched" }
          }
          if (-not $existed) { New-Item -Path $ps -Force | Out-Null }
          Set-Item -LiteralPath $ps -Value "URL:$proto"
          New-ItemProperty -LiteralPath $ps -Name 'URL Protocol' -Value '' -PropertyType String -Force | Out-Null
          New-ItemProperty -LiteralPath $ps -Name 'NoOpenWith' -Value '' -PropertyType String -Force | Out-Null
          $cmd = Join-Path $ps 'shell\open\command'
          New-Item -Path $cmd -Force | Out-Null
          Set-Item -LiteralPath $cmd -Value '%SystemRoot%/System32/systray.exe'
          $done += @{ Proto = $proto; Existed = $existed; File = $file }
      }
      , $done
  } `
  -Undo {
      param($d)
      foreach ($p in @($d)) {
          $ps = "HKCU:\Software\Classes\$($p.Proto)"
          if ($p.File -and (Test-Path -LiteralPath $p.File)) { reg.exe import $p.File | Out-Null }
          elseif (-not $p.Existed) { Remove-Item -LiteralPath $ps -Recurse -Force -ErrorAction SilentlyContinue }
      }
  } `
  -Test {
      $cmd = 'HKCU:\Software\Classes\ms-gamebar\shell\open\command'
      (Test-Path -LiteralPath $cmd) -and ((Get-Item -LiteralPath $cmd).GetValue('') -match 'systray\.exe')
  }

T -Id 'ui-snapassist' -Cat $I -Name 'No app suggestions when snapping windows' -Tags max -Explorer `
  -Desc 'Adapted from Win11Debloat: turns off Snap Assist, the row of other windows offered after you snap one to a side or corner.' `
  -Reg @( Rg $EXA 'SnapAssist' 0 )

T -Id 'ui-snaplayouts' -Cat $I -Name 'Hide snap layout flyouts' -Tags max -Explorer `
  -Desc 'Adapted from Win11Debloat: hides the snap layout popup on the maximize button and when a window is dragged to the top of the screen. Snapping itself stays on.' `
  -Reg @( Rg $EXA 'EnableSnapBar' 0; Rg $EXA 'EnableSnapAssistFlyout' 0 )

T -Id 'ui-taskbar-labels' -Conflict 'taskbar grouping' -Cat $I -Name 'Taskbar: never combine buttons, show labels' -Tags max -Explorer `
  -Desc 'Adapted from Win11Debloat: the main taskbar keeps a separate button and a label for every window.' `
  -Reg @( Rg $EXA 'TaskbarGlomLevel' 2 )

T -Id 'ui-taskbar-whenfull' -Conflict 'taskbar grouping' -Cat $I -Name 'Taskbar: combine buttons only when full' -Explorer `
  -Desc 'Adapted from Win11Debloat: the main taskbar shows labels until it runs out of room, then combines them.' `
  -Reg @( Rg $EXA 'TaskbarGlomLevel' 1 )

T -Id 'ui-taskbar-labels2' -Cat $I -Name 'Secondary taskbars: never combine buttons' -Tags max -Explorer `
  -Desc 'Adapted from Win11Debloat: taskbars on extra monitors show a separate labeled button for every window.' `
  -Reg @( Rg $EXA 'MMTaskbarGlomLevel' 2 )

T -Id 'ui-mmtaskbar-main' -Cat $I -Name 'Extra monitors: show a window on the main taskbar too' -Explorer `
  -Desc 'Adapted from Win11Debloat: an app appears on the main taskbar and on the monitor where its window is open.' `
  -Reg @( Rg $EXA 'MMTaskbarMode' 1 )

T -Id 'ui-mmtaskbar-active' -Cat $I -Name 'Extra monitors: show a window only where it is open' -Explorer `
  -Desc 'Adapted from Win11Debloat: each window is listed only on the taskbar of the monitor it sits on.' `
  -Reg @( Rg $EXA 'MMTaskbarMode' 2 )

T -Id 'ui-explorer-dl' -Conflict 'Explorer opens to' -Cat $I -Name 'Open Explorer to Downloads' -Explorer `
  -Desc 'Adapted from Win11Debloat: File Explorer starts in Downloads instead of Home or This PC.' `
  -Reg @( Rg $EXA 'LaunchTo' 3 )

T -Id 'ui-letters-first' -Conflict 'drive letters' -Cat $I -Name 'Show drive letters before the drive name' -Explorer `
  -Desc 'Adapted from Win11Debloat: Explorer lists "C: Local Disk" instead of "Local Disk (C:)".' `
  -Reg @( Rg "$CV\Explorer" 'ShowDriveLettersFirst' 4 )

T -Id 'ui-letters-net' -Conflict 'drive letters' -Cat $I -Name 'Show network drive letters before the name' -Explorer `
  -Desc 'Adapted from Win11Debloat: only network drives put the letter first. Local drives stay "name (letter)". Replaces the all-drives letter layout if that was applied.' `
  -Reg @( Rg "$CV\Explorer" 'ShowDriveLettersFirst' 1 )

T -Id 'ui-letters-hide' -Conflict 'drive letters' -Cat $I -Name 'Hide drive letters' -Tags optin -Explorer `
  -Desc 'Adapted from Win11Debloat: Explorer shows drive names without letters. Undo restores the previous layout.' `
  -Reg @( Rg "$CV\Explorer" 'ShowDriveLettersFirst' 2 )

T -Id 'ui-spotlight-learn' -Cat $I -Name 'Hide "Learn about this picture" on the desktop' -Tags max `
  -Desc 'Adapted from Win11Debloat: hides the Spotlight desktop shortcut. Rotating Spotlight wallpaper can stay on. This does not turn Spotlight off.' `
  -Reg @( Rg 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\HideDesktopIcons\NewStartPanel' '{2cc5ca98-6485-489a-920e-b3e88a6ccce3}' 1 )

T -Id 'ui-thispc-folders' -Cat $I -Name 'This PC: show Desktop, Documents, Downloads, Music, Pictures and Videos' -Explorer `
  -Desc 'Adapted from Win11Debloat: puts those folders back on the This PC page. Previous hide flags are saved and restored on undo.' `
  -Apply {
      $base = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\MyComputer\NameSpace'
      $ids = '{B4BFCC3A-DB2C-424C-B029-7FE99A87C641}', '{d3162b92-9365-467a-956b-92703aca08af}', '{088e3905-0323-4b02-9826-5d99428e115f}', '{1CF1260C-4DD0-4ebb-811F-33C572699FDE}', '{3dfdf296-dbec-4fb4-81d1-6a3438bcf4de}', '{24ad3ad4-a569-4530-98e1-ab02f9417aa8}', '{f86fa3ab-70d2-4fc7-9c99-fcbf05467f3a}'
      $prev = @()
      foreach ($id in $ids) {
          $p = Join-Path $base $id
          $existed = Test-Path -LiteralPath $p
          if (-not $existed) { New-Item -Path $p -Force | Out-Null }
          $item = Get-ItemProperty -LiteralPath $p
          $prev += @{ Path = $p; Existed = $existed; HadHide = ($null -ne $item.HideIfEnabled); Hide = $item.HideIfEnabled; HadHidden = ($null -ne $item.HiddenByDefault); Hidden = $item.HiddenByDefault }
          if ($null -ne $item.HideIfEnabled) { Remove-ItemProperty -LiteralPath $p -Name 'HideIfEnabled' -ErrorAction SilentlyContinue }
          Set-RegEntry $p 'HiddenByDefault' 0
      }
      , $prev
  } `
  -Undo {
      param($d)
      foreach ($p in @($d)) {
          if (-not $p.Existed) { Remove-Item -LiteralPath $p.Path -Recurse -Force -ErrorAction SilentlyContinue; continue }
          if ($p.HadHide) { Set-RegEntry $p.Path 'HideIfEnabled' $p.Hide } else { Remove-RegEntry $p.Path 'HideIfEnabled' }
          if ($p.HadHidden) { Set-RegEntry $p.Path 'HiddenByDefault' $p.Hidden } else { Remove-RegEntry $p.Path 'HiddenByDefault' }
      }
  } `
  -Test {
      $p = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\MyComputer\NameSpace\{B4BFCC3A-DB2C-424C-B029-7FE99A87C641}'
      $item = Get-ItemProperty -LiteralPath $p -ErrorAction SilentlyContinue
      ($null -ne $item) -and ($item.HiddenByDefault -eq 0) -and ($null -eq $item.HideIfEnabled)
  }

T -Id 'ui-allapps-grid' -Conflict 'All apps view' -Cat $I -Name 'Start All apps: grid' -Explorer `
  -Desc 'Adapted from Win11Debloat (Windows 11 25H2, build 26200 and newer): the All apps list uses the alphabetical grid.' `
  -Reg @( Rg "$CV\Start" 'AllAppsViewMode' 1 )

T -Id 'ui-allapps-list' -Conflict 'All apps view' -Cat $I -Name 'Start All apps: list' -Explorer `
  -Desc 'Adapted from Win11Debloat (Windows 11 25H2, build 26200 and newer): the All apps list uses the alphabetical list. Category view is the Windows default; undo restores whatever you had.' `
  -Reg @( Rg "$CV\Start" 'AllAppsViewMode' 2 )

T -Id 'ui-allapps-hide' -Conflict 'All apps view' -Cat $I -Name 'Hide All apps in Start' -Tags optin -Risk Moderate -Explorer `
  -Desc 'Adapted from Win11Debloat (build 26200 and newer): removes the All apps section from Start. Installed apps are harder to find. Undo restores the section.' `
  -Reg @( Rg 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Policies\Explorer' 'NoStartMenuMorePrograms' 1 )

T -Id 'ui-search-icon' -Conflict 'search style' -Cat $I -Name 'Taskbar search: icon only' -Explorer `
  -Desc 'Adapted from Win11Debloat: replaces the search box with a search icon. Win+S still opens search.' `
  -Reg @( Rg "$CV\Search" 'SearchboxTaskbarMode' 1 )

T -Id 'ui-search-label' -Conflict 'search style' -Cat $I -Name 'Taskbar search: icon and label' -Explorer `
  -Desc 'Adapted from Win11Debloat: shows a search icon with the word Search, narrower than the full box.' `
  -Reg @( Rg "$CV\Search" 'SearchboxTaskbarMode' 3 )

T -Id 'ui-notifcenter' -Cat $I -Name 'Disable the notification center and calendar flyout' -Tags optin -Risk Moderate `
  -Desc 'Adapted from Chris Titus Tech WinUtil: turns off the notification center, including the calendar flyout. Toast popups are a separate tweak.' `
  -Reg @( Rg 'HKCU:\Software\Policies\Microsoft\Windows\Explorer' 'DisableNotificationCenter' 1 )

T -Id 'priv-storesearch' -Cat $V -Name 'Hide Microsoft Store suggestions in search' -Tags safe, privacy, max, deai `
  -Desc 'Adapted from Chris Titus Tech WinUtil: denies Everyone write access to the Store recommendation database so Start search stops offering Store apps. Undo removes only that deny entry. Needs the Microsoft Store package for this account.' `
  -Apply {
      $db = Join-Path $env:LOCALAPPDATA 'Packages\Microsoft.WindowsStore_8wekyb3d8bbwe\LocalState\store.db'
      if (-not (Test-Path -LiteralPath $db)) { throw 'Microsoft Store database was not found for this account' }
      & icacls.exe $db /deny '*S-1-1-0:F' | Out-Null
      if ($LASTEXITCODE -ne 0) { throw "could not update permissions on the Store database (exit $LASTEXITCODE)" }
      $null
  } `
  -Undo {
      $db = Join-Path $env:LOCALAPPDATA 'Packages\Microsoft.WindowsStore_8wekyb3d8bbwe\LocalState\store.db'
      if (Test-Path -LiteralPath $db) { & icacls.exe $db /remove:d '*S-1-1-0' | Out-Null }
  } `
  -Test {
      $db = Join-Path $env:LOCALAPPDATA 'Packages\Microsoft.WindowsStore_8wekyb3d8bbwe\LocalState\store.db'
      if (-not (Test-Path -LiteralPath $db)) { return $false }
      ((& icacls.exe $db | Out-String) -match 'Everyone:\(DENY\)|Everyone:\(N\)')
  }

T -Id 'perf-s3' -Cat $P -Name 'Prefer classic S3 sleep over Modern Standby' -Tags optin -Risk Advanced -Restart `
  -Desc 'Adapted from Chris Titus Tech WinUtil: sets PlatformAoAcOverride so the PC uses S3 sleep (CPU powered off, memory refreshed) instead of Modern Standby. Some laptops sleep poorly or not at all with this. Undo restores the previous value.' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\Power' 'PlatformAoAcOverride' 0 )

T -Id 'net-rdp-unsigned' -Cat $N -Name 'Do not warn when opening an unsigned Remote Desktop file' -Tags optin -Risk Moderate `
  -Desc 'Adapted from Chris Titus Tech WinUtil: skips the warning added by recent Windows 10 and 11 updates when you open an .rdp file that is not signed. Leave this off unless you open Remote Desktop files you trust.' `
  -Reg @(
      Rg 'HKLM:\SOFTWARE\Policies\Microsoft\Windows NT\Terminal Services\Client' 'RedirectionWarningDialogVersion' 1
      Rg 'HKCU:\SOFTWARE\Microsoft\Terminal Server Client' 'RdpLaunchConsentAccepted' 1
  )

T -Id 'ui-start-old' -Cat $I -Name 'Prefer the Start menu layout from before the 25H2 rollout' -Tags optin -Risk Advanced -Explorer `
  -Desc 'Adapted from Chris Titus Tech WinUtil: sets the feature override that keeps the previous Start layout on builds still honoring it. WinUtil warns this does nothing on newer Windows. Undo restores the previous override value.' `
  -Reg @(
      Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\FeatureManagement\Overrides\8\3036241548' 'EnabledState' 1
      Rg 'HKLM:\SYSTEM\ControlSet001\Control\FeatureManagement\Overrides\8\3036241548' 'EnabledState' 1
  )
