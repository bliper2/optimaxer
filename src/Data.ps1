# Optimaxer data tables: service recommendations, cleanup targets, appx bloat list, DNS providers, Windows features.

# 's' = set to Manual (safe: still starts on demand). 'd' = Disable (maximum profile).
$global:OptiServiceRecs = @'
AJRouter|s|AllJoyn Router (IoT discovery)
ALG|s|Application Layer Gateway (Internet Connection Sharing helper)
AppMgmt|s|Group-policy software installation
AppReadiness|s|App Readiness (first-logon app prep)
AppVClient|d|Microsoft App-V client
AssignedAccessManagerSvc|d|Kiosk / assigned access
AxInstSV|s|ActiveX Installer
BDESVC|s|BitLocker service (still starts on demand)
BTAGService|s|Bluetooth audio gateway
Browser|s|Computer Browser (legacy)
CDPSvc|s|Connected Devices Platform
COMSysApp|s|COM+ System Application
CertPropSvc|s|Certificate propagation (smart cards)
CscService|s|Offline Files
DmEnrollmentSvc|s|Device management enrollment
DsSvc|s|Data Sharing Service
EntAppSvc|s|Enterprise app management
FDResPub|s|Function Discovery Resource Publication
Fax|d|Fax
HvHost|s|Hyper-V host service
IpxlatCfgSvc|s|IP translation configuration
KtmRm|s|Kernel transaction manager for MSDTC
MSDTC|s|Distributed Transaction Coordinator
MSiSCSI|s|iSCSI initiator
MapsBroker|d|Downloaded maps manager
MicrosoftEdgeElevationService|s|Edge elevation service
MixedRealityOpenXRSvc|s|Windows Mixed Reality OpenXR
NaturalAuthentication|s|Natural authentication
NcaSvc|s|Network Connectivity Assistant
NetTcpPortSharing|d|Net.Tcp port sharing
Netlogon|s|Netlogon (domain controllers only)
PNRPAutoReg|s|PNRP machine name publication
PNRPsvc|s|Peer Name Resolution Protocol
p2pimsvc|s|Peer networking identity manager
p2psvc|s|Peer networking grouping
PeerDistSvc|s|BranchCache
PerfHost|s|Performance counter DLL host
PhoneSvc|s|Phone service
PrintNotify|s|Printer extensions and notifications
PushToInstall|s|Windows PushToInstall
QWAVE|s|Quality Windows Audio Video Experience
RasAuto|s|Remote Access auto connection manager
RemoteAccess|d|Routing and Remote Access
RemoteRegistry|d|Remote Registry (security risk)
RetailDemo|d|Retail demo mode
RpcLocator|s|RPC locator
SCPolicySvc|s|Smart card removal policy
SCardSvr|s|Smart card
ScDeviceEnum|s|Smart card device enumeration
SDRSVC|s|Windows Backup
SEMgrSvc|s|Payments and NFC/SE manager
SNMPTRAP|d|SNMP trap
SSDPSRV|s|SSDP discovery (UPnP)
SessionEnv|s|Remote Desktop configuration
SharedAccess|s|Internet Connection Sharing
SmsRouter|s|SMS router
TapiSrv|s|Telephony
TermService|s|Remote Desktop Services
TieringEngineService|s|Storage tiers management
TrkWks|s|Distributed link tracking client
TroubleshootingSvc|s|Recommended troubleshooting service
UevAgentService|d|User Experience Virtualization
UmRdpService|s|Remote Desktop port redirector
WEPHOSTSVC|s|Windows Encryption Provider host
WFDSConMgrSvc|s|Wi-Fi Direct services connection manager
WMPNetworkSvc|d|Windows Media Player network sharing
WManSvc|s|Windows Management Service
WalletService|d|Wallet service
WarpJITSvc|s|Warp JIT service
WdiServiceHost|s|Diagnostic service host
WdiSystemHost|s|Diagnostic system host
WebClient|s|WebDAV client
Wecsvc|s|Windows Event Collector
WinRM|s|Windows Remote Management
WpcMonSvc|s|Parental controls
autotimesvc|s|Cellular time
cloudidsvc|s|Microsoft cloud identity
edgeupdate|s|Microsoft Edge updater
edgeupdatem|s|Microsoft Edge updater (machine)
fdPHost|s|Function Discovery provider host
fhsvc|s|File History
icssvc|s|Mobile hotspot
lfsvc|s|Geolocation service
lltdsvc|s|Link-Layer Topology Discovery mapper
upnphost|s|UPnP device host
wisvc|s|Windows Insider service
workfolderssvc|s|Work Folders
'@ -split "`r?`n" | Where-Object { $_ } | ForEach-Object {
    $p = $_ -split '\|', 3
    [pscustomobject]@{ Name = $p[0]; Start = $(if ($p[1] -eq 'd') { 4 } else { 3 }); Desc = $p[2] }
}

# Never offered for change on the Services page.
$global:OptiProtectedServices = @(
    'RpcSs','RpcEptMapper','DcomLaunch','LSM','PlugPlay','Power','EventLog','EventSystem','mpssvc','BFE','WinDefend',
    'SecurityHealthService','Wcmsvc','Dhcp','Dnscache','nsi','NlaSvc','LanmanWorkstation','CryptSvc','TrustedInstaller',
    'gpsvc','ProfSvc','UserManager','SamSs','Winmgmt','StateRepository','TimeBrokerSvc','SystemEventsBroker',
    'BrokerInfrastructure','CoreMessagingRegistrar','Schedule','Audiosrv','AudioEndpointBuilder','sppsvc','StorSvc',
    'DeviceInstall','Themes','ShellHWDetection','UsoSvc','wuauserv','BITS','KeyIso','SENS','LanmanServer','netprofm',
    'WlanSvc','hidserv','DispBrokerDesktopSvc','CDPUserSvc','WpnService','Appinfo','CertPropSvc_','AppXSvc','ClipSVC',
    'InstallService','DoSvc','Spooler'
)

# Cleanup targets. Paths are directories (contents are removed) or wildcards (matches are removed).
$global:OptiCleanup = @(
    @{ Id = 'usertemp';  Name = 'User temp files';            Desc = 'Per-user temporary files left by apps and installers.';                       Paths = @($env:TEMP) },
    @{ Id = 'wintemp';   Name = 'Windows temp files';         Desc = 'System-wide temporary files.';                                                  Paths = @("$env:WINDIR\Temp") },
    @{ Id = 'wu';        Name = 'Windows Update download cache'; Desc = 'Already-installed update packages (re-downloaded if ever needed).';     Paths = @("$env:WINDIR\SoftwareDistribution\Download"); Services = @('wuauserv', 'bits') },
    @{ Id = 'do';        Name = 'Delivery Optimization cache'; Desc = 'Cached update files shared with other PCs.';                                 Paths = @("$env:WINDIR\ServiceProfiles\NetworkService\AppData\Local\Microsoft\Windows\DeliveryOptimization\Cache") },
    @{ Id = 'thumbs';    Name = 'Thumbnail and icon cache';   Desc = 'Rebuilt automatically by Explorer.';                                         Paths = @("$env:LOCALAPPDATA\Microsoft\Windows\Explorer\thumbcache_*.db", "$env:LOCALAPPDATA\Microsoft\Windows\Explorer\iconcache_*.db") },
    @{ Id = 'wer';       Name = 'Error reports';              Desc = 'Windows Error Reporting archives and queues.';                                   Paths = @("$env:ProgramData\Microsoft\Windows\WER\ReportArchive", "$env:ProgramData\Microsoft\Windows\WER\ReportQueue", "$env:LOCALAPPDATA\Microsoft\Windows\WER") },
    @{ Id = 'dumps';     Name = 'Crash dumps';                Desc = 'Minidumps, kernel reports and MEMORY.DMP.';                                      Paths = @("$env:WINDIR\Minidump", "$env:WINDIR\LiveKernelReports", "$env:WINDIR\MEMORY.DMP") },
    @{ Id = 'logs';      Name = 'Windows log files';          Desc = 'CBS and DISM servicing logs.';                                                   Paths = @("$env:WINDIR\Logs\CBS\*.log", "$env:WINDIR\Logs\DISM\*.log") },
    @{ Id = 'shader';    Name = 'GPU shader caches';          Desc = 'DirectX/NVIDIA/AMD shader caches. Games recompile shaders on next launch.';      Paths = @("$env:LOCALAPPDATA\D3DSCache", "$env:LOCALAPPDATA\NVIDIA\DXCache", "$env:LOCALAPPDATA\NVIDIA\GLCache", "$env:LOCALAPPDATA\AMD\DxCache", "$env:LOCALAPPDATA\AMD\GLCache"); Risk = 'Moderate' },
    @{ Id = 'chrome';    Name = 'Google Chrome cache';        Desc = 'Close Chrome first. Logins and bookmarks are kept.';                             Paths = @("$env:LOCALAPPDATA\Google\Chrome\User Data\*\Cache", "$env:LOCALAPPDATA\Google\Chrome\User Data\*\Code Cache", "$env:LOCALAPPDATA\Google\Chrome\User Data\*\GPUCache") },
    @{ Id = 'edge';      Name = 'Microsoft Edge cache';       Desc = 'Close Edge first. Logins and bookmarks are kept.';                               Paths = @("$env:LOCALAPPDATA\Microsoft\Edge\User Data\*\Cache", "$env:LOCALAPPDATA\Microsoft\Edge\User Data\*\Code Cache", "$env:LOCALAPPDATA\Microsoft\Edge\User Data\*\GPUCache") },
    @{ Id = 'brave';     Name = 'Brave cache';                Desc = 'Close Brave first.';                                                             Paths = @("$env:LOCALAPPDATA\BraveSoftware\Brave-Browser\User Data\*\Cache", "$env:LOCALAPPDATA\BraveSoftware\Brave-Browser\User Data\*\Code Cache", "$env:LOCALAPPDATA\BraveSoftware\Brave-Browser\User Data\*\GPUCache") },
    @{ Id = 'opera';     Name = 'Opera / Opera GX cache';     Desc = 'Close Opera first.';                                                             Paths = @("$env:LOCALAPPDATA\Opera Software\Opera Stable\Cache", "$env:LOCALAPPDATA\Opera Software\Opera GX Stable\Cache", "$env:APPDATA\Opera Software\Opera Stable\Cache", "$env:APPDATA\Opera Software\Opera Stable\Code Cache", "$env:APPDATA\Opera Software\Opera Stable\GPUCache", "$env:APPDATA\Opera Software\Opera GX Stable\Cache", "$env:APPDATA\Opera Software\Opera GX Stable\Code Cache", "$env:APPDATA\Opera Software\Opera GX Stable\GPUCache") },
    @{ Id = 'vivaldi';   Name = 'Vivaldi cache';              Desc = 'Close Vivaldi first.';                                                           Paths = @("$env:LOCALAPPDATA\Vivaldi\User Data\*\Cache", "$env:LOCALAPPDATA\Vivaldi\User Data\*\Code Cache", "$env:LOCALAPPDATA\Vivaldi\User Data\*\GPUCache") },
    @{ Id = 'firefox';   Name = 'Firefox cache';              Desc = 'Close Firefox first.';                                                           Paths = @("$env:LOCALAPPDATA\Mozilla\Firefox\Profiles\*\cache2") },
    @{ Id = 'discord';   Name = 'Discord cache';              Desc = 'Close Discord first.';                                                           Paths = @("$env:APPDATA\discord\Cache", "$env:APPDATA\discord\Code Cache", "$env:APPDATA\discord\GPUCache") },
    @{ Id = 'spotify';   Name = 'Spotify cache';              Desc = 'Offline downloads are kept.';                                                    Paths = @("$env:LOCALAPPDATA\Spotify\Data") },
    @{ Id = 'steam';     Name = 'Steam web cache';            Desc = 'Steam browser/html cache.';                                                      Paths = @("$env:LOCALAPPDATA\Steam\htmlcache") },
    @{ Id = 'recycle';   Name = 'Recycle Bin';                Desc = 'Permanently deletes everything in the Recycle Bin.';                             Special = 'recycle'; Risk = 'Moderate' },
    @{ Id = 'component'; Name = 'Windows component store';    Desc = 'DISM StartComponentCleanup: removes superseded system components (slow).';       Special = 'component' }
)

# Appx bloat. L: R = recommended removal, O = optional, C = caution. Id may contain wildcards.
$global:OptiAppx = @'
Microsoft.549981C3F5F10|Cortana|R
Microsoft.BingNews|Microsoft News|R
Microsoft.BingWeather|Weather|R
Microsoft.BingFinance|Money|R
Microsoft.BingSports|Sports|R
Microsoft.BingSearch|Bing Search|R
Microsoft.GetHelp|Get Help|R
Microsoft.Getstarted|Tips|R
Microsoft.MicrosoftOfficeHub|Office hub (ad launcher)|R
Microsoft.MicrosoftSolitaireCollection|Solitaire Collection|R
Microsoft.People|People|R
Microsoft.PowerAutomateDesktop|Power Automate|R
Microsoft.WindowsFeedbackHub|Feedback Hub|R
Microsoft.MixedReality.Portal|Mixed Reality Portal|R
Microsoft.Microsoft3DViewer|3D Viewer|R
Microsoft.Print3D|Print 3D|R
Microsoft.3DBuilder|3D Builder|R
Microsoft.OneConnect|Mobile Plans|R
Microsoft.SkypeApp|Skype|R
Microsoft.Wallet|Wallet|R
Microsoft.Messaging|Messaging|R
Clipchamp.Clipchamp|Clipchamp|R
MicrosoftTeams|Teams (personal)|R
MSTeams|Teams (personal)|R
Microsoft.Copilot|Copilot app|R
Microsoft.Windows.DevHome|Dev Home|R
Microsoft.MicrosoftJournal|Journal|R
Microsoft.Whiteboard|Whiteboard|R
MicrosoftCorporationII.MicrosoftFamily|Family|R
Microsoft.ZuneMusic|Media Player (Groove)|O
Microsoft.ZuneVideo|Movies and TV|O
Microsoft.YourPhone|Phone Link|O
Microsoft.Todos|Microsoft To Do|O
Microsoft.WindowsMaps|Maps|O
Microsoft.OutlookForWindows|New Outlook|O
Microsoft.WindowsCommunicationsApps|Mail and Calendar|O
Microsoft.MicrosoftStickyNotes|Sticky Notes|O
Microsoft.WindowsAlarms|Clock and Alarms|O
Microsoft.WindowsSoundRecorder|Sound Recorder|O
Microsoft.GamingApp|Xbox app|O
Microsoft.XboxApp|Xbox console companion|O
Microsoft.XboxGamingOverlay|Xbox Game Bar|O
Microsoft.XboxGameOverlay|Xbox Game overlay|O
Microsoft.Xbox.TCUI|Xbox TCUI|O
Microsoft.XboxSpeechToTextOverlay|Xbox speech overlay|O
Microsoft.XboxIdentityProvider|Xbox identity provider (needed for Xbox sign-in)|C
*Copilot*|Copilot (all packages)|R
MicrosoftWindows.Client.WebExperience|Widgets web experience|O
Microsoft.WidgetsPlatformRuntime|Widgets runtime|O
Microsoft.Edge.GameAssist|Edge Game Assist|O
Microsoft.PCManager|PC Manager|R
Microsoft.Windows.AIHub|Copilot+ AI Hub|R
Microsoft.NetworkSpeedTest|Network Speed Test|R
Microsoft.News|Microsoft News app|R
Microsoft.Office.Sway|Sway|R
Microsoft.MicrosoftPowerBIForWindows|Power BI|R
Microsoft.BingFoodAndDrink|Bing Food and Drink|R
Microsoft.BingHealthAndFitness|Bing Health and Fitness|R
Microsoft.BingTranslator|Bing Translator|R
Microsoft.BingTravel|Bing Travel|R
Microsoft.M365Companions|Microsoft 365 Companions|R
Microsoft.Office.OneNote|OneNote (UWP)|O
MicrosoftCorporationII.QuickAssist|Quick Assist|O
MicrosoftWindows.CrossDevice|Cross Device Experience|O
Microsoft.StartExperiencesApp|Start experiences (widgets host)|O
AD2F1837.*|HP preinstalled apps|O
DellInc.*|Dell preinstalled apps|O
E046963F.LenovoCompanion|Lenovo Vantage|O
LenovoCompanyLimited.*|Lenovo services|O
*Asphalt*|Asphalt|R
*FarmVille*|FarmVille|R
*HiddenCity*|Hidden City|R
*MarchofEmpires*|March of Empires|R
*RoyalRevolt*|Royal Revolt|R
*CaesarsSlots*|Caesars Slots|R
*CookingFever*|Cooking Fever|R
*DisneyMagicKingdoms*|Disney Magic Kingdoms|R
*BubbleWitch*|Bubble Witch|R
*Flipboard*|Flipboard|R
*iHeartRadio*|iHeartRadio|R
*TuneInRadio*|TuneIn Radio|R
*PandoraMedia*|Pandora|R
*PicsArt*|PicsArt|R
*Phototastic*|Phototastic Collage|R
*Polarr*|Polarr Photo Editor|R
*SlingTV*|Sling TV|R
*WinZip*|WinZip trial|R
*ACGMediaPlayer*|ACG Media Player|R
*ActiproSoftware*|Actipro Software|R
*AdobePhotoshopExpress*|Photoshop Express|R
*AutodeskSketchBook*|Autodesk SketchBook|R
*CyberLinkMediaSuite*|CyberLink Media Suite|R
*DrawboardPDF*|Drawboard PDF|R
*EclipseManager*|Eclipse Manager|R
*NYTCrossword*|NYT Crossword|R
*OneCalendar*|One Calendar|R
*LiveWallpaper*|Live Wallpaper|R
*Spotify*|Spotify|R
*Disney*|Disney+|R
*Netflix*|Netflix|R
*TikTok*|TikTok|R
*Facebook*|Facebook|R
*Instagram*|Instagram|R
*Twitter*|Twitter / X|R
*LinkedIn*|LinkedIn|R
*Amazon*|Amazon apps|R
*CandyCrush*|Candy Crush|R
*king.com*|King games|R
*Hulu*|Hulu|R
*PrimeVideo*|Prime Video|R
*McAfee*|McAfee trial|R
*Dropbox*|Dropbox promo|R
*Duolingo*|Duolingo|R
*Roblox*|Roblox|R
*Solitaire*|Solitaire|R
'@ -split "`r?`n" | Where-Object { $_ } | ForEach-Object {
    $p = $_ -split '\|'
    [pscustomobject]@{ Id = $p[0]; Name = $p[1]; Level = $p[2] }
}

$global:OptiDns = @(
    @{ Name = 'Cloudflare';        Desc = 'Fastest on average, privacy-focused. 1.1.1.1';           V4 = @('1.1.1.1', '1.0.0.1');                 V6 = @('2606:4700:4700::1111', '2606:4700:4700::1001'); Doh = 'https://cloudflare-dns.com/dns-query' },
    @{ Name = 'Google';            Desc = 'Reliable, global anycast. 8.8.8.8';                      V4 = @('8.8.8.8', '8.8.4.4');                 V6 = @('2001:4860:4860::8888', '2001:4860:4860::8844'); Doh = 'https://dns.google/dns-query' },
    @{ Name = 'Quad9';             Desc = 'Blocks known malicious domains. 9.9.9.9';                V4 = @('9.9.9.9', '149.112.112.112');         V6 = @('2620:fe::fe', '2620:fe::9');                    Doh = 'https://dns.quad9.net/dns-query' },
    @{ Name = 'AdGuard';           Desc = 'Blocks ads and trackers network-wide. 94.140.14.14';     V4 = @('94.140.14.14', '94.140.15.15');       V6 = @('2a10:50c0::ad1:ff', '2a10:50c0::ad2:ff');       Doh = 'https://dns.adguard-dns.com/dns-query' },
    @{ Name = 'OpenDNS';           Desc = 'Cisco OpenDNS. 208.67.222.222';                          V4 = @('208.67.222.222', '208.67.220.220');   V6 = @();                                               Doh = '' },
    @{ Name = 'Automatic (DHCP)';  Desc = 'Reset to the router / ISP default.';                      V4 = @();                                     V6 = @();                                               Doh = '' }
)

$global:OptiFeatures = @(
    @{ Id = 'NetFx3';                              Name = '.NET Framework 3.5';              Desc = 'Needed by older apps and games.' },
    @{ Id = 'Microsoft-Hyper-V-All';               Name = 'Hyper-V';                         Desc = 'Virtual machines (Pro/Enterprise). Can reduce gaming performance.' },
    @{ Id = 'VirtualMachinePlatform';              Name = 'Virtual Machine Platform';        Desc = 'Required for WSL2 and Android subsystems.' },
    @{ Id = 'Microsoft-Windows-Subsystem-Linux';   Name = 'Windows Subsystem for Linux';     Desc = 'Run Linux distributions.' },
    @{ Id = 'Containers-DisposableClientVM';       Name = 'Windows Sandbox';                 Desc = 'Disposable desktop for testing (Pro/Enterprise).' },
    @{ Id = 'HypervisorPlatform';                  Name = 'Windows Hypervisor Platform';     Desc = 'For third-party emulators and VMs.' },
    @{ Id = 'TelnetClient';                        Name = 'Telnet client';                   Desc = 'Legacy telnet command.' },
    @{ Id = 'TFTP';                                Name = 'TFTP client';                     Desc = 'Legacy tftp command.' },
    @{ Id = 'MediaPlayback';                       Name = 'Media features';                  Desc = 'Legacy media playback components.' },
    @{ Id = 'DirectPlay';                          Name = 'DirectPlay';                      Desc = 'Needed by very old games.' },
    @{ Id = 'LegacyComponents';                    Name = 'Legacy components';               Desc = 'Old multimedia components.' },
    @{ Id = 'SMB1Protocol';                        Name = 'SMB 1.0 protocol';                Desc = 'Insecure legacy file sharing. Keep disabled.' },
    @{ Id = 'Printing-PDFServices-Features';       Name = 'Print to PDF services';           Desc = 'Microsoft Print to PDF.' }
)
