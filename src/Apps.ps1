# Installer catalog: Category|Display name|WinGet package id|Homepage domain|WinUtil product link and an icon-pack reference sh:/di:/si: (selfh.st icons, dashboard-icons, Simple Icons), used for the tile icon. Installed through WinGet (--source winget).
$global:OptiApps = @'
Browsers|Brave|Brave.Brave|brave.com|https://www.brave.com|sh:brave
Browsers|Google Chrome|Google.Chrome|google.com|https://www.google.com/chrome/|sh:google-chrome
Browsers|Chromium|Hibbiki.Chromium|github.com/Hibbiki|https://www.chromium.org/|sh:chromium
Browsers|Microsoft Edge|Microsoft.Edge|microsoft.com|https://www.microsoft.com/edge|sh:microsoft-edge
Browsers|Firefox|Mozilla.Firefox|mozilla.org|https://www.mozilla.org/en-US/firefox/new/|sh:firefox
Browsers|Firefox ESR|Mozilla.Firefox.ESR|mozilla.org|https://www.mozilla.org/en-US/firefox/enterprise/|
Browsers|Floorp|Ablaze.Floorp|floorp.app|https://floorp.app/|di:floorp
Browsers|LibreWolf|LibreWolf.LibreWolf|librewolf.net|https://librewolf.net/|sh:librewolf
Browsers|Mullvad Browser|MullvadVPN.MullvadBrowser|mullvad.net|https://mullvad.net/browser|di:mullvad-browser
Browsers|Opera|Opera.Opera|opera.com||sh:opera
Browsers|Opera GX|Opera.OperaGX|opera.com||sh:opera
Browsers|Tor Browser|TorProject.TorBrowser|torproject.org|https://www.torproject.org/|sh:tor-browser
Browsers|Vivaldi|Vivaldi.Vivaldi|vivaldi.com|https://vivaldi.com/|di:vivaldi
Browsers|Waterfox|Waterfox.Waterfox|waterfox.net|https://www.waterfox.net/|
Browsers|Zen Browser|Zen-Team.Zen-Browser|zen-browser.app|https://zen-browser.app/|di:zen-browser
Communications|Betterbird|Betterbird.Betterbird|betterbird.eu|https://www.betterbird.eu/|
Communications|Discord|Discord.Discord|discord.com|https://discord.com/|sh:discord
Communications|Element|Element.Element|element.io|https://element.io/|sh:element
Communications|Proton Mail|Proton.ProtonMail|proton.me|https://proton.me/mail|sh:proton-mail
Communications|Signal|OpenWhisperSystems.Signal|signal.org|https://signal.org/|sh:signal
Communications|Slack|SlackTechnologies.Slack|slack.com|https://slack.com/|sh:slack
Communications|TeamSpeak 3|TeamSpeakSystems.TeamSpeakClient|teamspeak.com|https://www.teamspeak.com/|sh:teamspeak
Communications|Telegram|Telegram.TelegramDesktop|desktop.telegram.org|https://telegram.org/|sh:telegram
Communications|Thunderbird|Mozilla.Thunderbird|thunderbird.net|https://www.thunderbird.net/|sh:thunderbird
Communications|Vesktop|Vencord.Vesktop|github.com/Vencord|https://vesktop.dev|
Communications|Viber|Rakuten.Viber|viber.com|https://www.viber.com/|di:viber
Communications|Zoom|Zoom.Zoom|zoom.us|https://zoom.us/|sh:zoom
Development|Bruno|Bruno.Bruno|usebruno.com|https://www.usebruno.com/|di:bruno
Development|Claude Desktop|Anthropic.Claude|claude.ai|https://claude.ai/download|sh:claude
Development|CMake|Kitware.CMake|cmake.org|https://cmake.org/|si:cmake:064F8C
Development|Cursor|Anysphere.Cursor|cursor.com|https://cursor.com/|si:cursor:000000
Development|Docker Desktop|Docker.DockerDesktop|docker.com|https://www.docker.com/products/docker-desktop/|sh:docker
Development|Fast Node Manager|Schniz.fnm|github.com/Schniz|https://github.com/Schniz/fnm|
Development|Git|Git.Git|gitforwindows.org|https://git-scm.com/|sh:git
Development|Git Extensions|GitExtensionsTeam.GitExtensions|gitextensions.github.io|https://gitextensions.github.io/|sh:git
Development|GitHub CLI|GitHub.cli|cli.github.com|https://cli.github.com/|sh:github
Development|GitHub Desktop|GitHub.GitHubDesktop|desktop.github.com|https://desktop.github.com/|sh:github
Development|Go|GoLang.Go|go.dev|https://go.dev/|di:go
Development|JetBrains Toolbox|JetBrains.Toolbox|jetbrains.com|https://www.jetbrains.com/toolbox/|di:jetbrains-toolbox
Development|Lazygit|JesseDuffield.lazygit|github.com/jesseduffield|https://github.com/jesseduffield/lazygit/|
Development|Neovim|Neovim.Neovim|neovim.io|https://neovim.io/|di:neovim
Development|NodeJS|OpenJS.NodeJS|nodejs.org|https://nodejs.org/|di:nodejs
Development|NodeJS LTS|OpenJS.NodeJS.LTS|nodejs.org|https://nodejs.org/|di:nodejs
Development|Notepad++|Notepad++.Notepad++|notepad-plus-plus.org|https://notepad-plus-plus.org/|si:notepadplusplus:90E59A
Development|Oh My Posh|JanDeDobbeleer.OhMyPosh|github.com/JanDeDobbeleer|https://ohmyposh.dev/|di:oh-my-posh
Development|pnpm|pnpm.pnpm|pnpm.io|https://pnpm.io/|si:pnpm:F69220
Development|Postman|Postman.Postman|postman.com|https://www.postman.com/downloads/|di:postman
Development|PowerShell 7|Microsoft.PowerShell|microsoft.com|https://github.com/PowerShell/PowerShell|di:powershell
Development|Python 3.13|Python.Python.3.13|python.org||sh:python
Development|Rust (rustup)|Rustlang.Rustup|rust-lang.org||sh:rust
Development|Starship|Starship.Starship|starship.rs|https://starship.rs/|si:starship:DD0B78
Development|Sublime Text|SublimeHQ.SublimeText.4|sublimetext.com|https://www.sublimetext.com/|si:sublimetext:FF9800
Development|Temurin JDK 21|EclipseAdoptium.Temurin.21.JDK|adoptium.net||
Development|Unity Hub|Unity.UnityHub|unity.com|https://unity.com/|di:unity
Development|uv|astral-sh.uv|github.com/astral-sh|https://docs.astral.sh/uv/getting-started/installation/|si:uv:DE5FE9
Development|Visual Studio 2022 Community|Microsoft.VisualStudio.2022.Community|visualstudio.microsoft.com|https://visualstudio.microsoft.com/|
Development|VS Code|Microsoft.VisualStudioCode|code.visualstudio.com|https://code.visualstudio.com/|di:vscode
Development|VSCodium|VSCodium.VSCodium|github.com/VSCodium|https://vscodium.com/|sh:vscodium
Development|WinMerge|WinMerge.WinMerge|winmerge.org||
Development|Windows Terminal|Microsoft.WindowsTerminal|docs.microsoft.com|https://aka.ms/terminal|sh:windows-terminal
Development|Yarn|Yarn.Yarn|github.com/yarnpkg|https://yarnpkg.com/|si:yarn:2C8EBB
Development|Zed|ZedIndustries.Zed|zed.dev|https://zed.dev/|sh:zed
Documents|Adobe Acrobat Reader|Adobe.Acrobat.Reader.64-bit|acrobat.adobe.com|https://www.adobe.com/acrobat/pdf-reader.html|
Documents|Foxit PDF Reader|Foxit.FoxitReader|foxitsoftware.com|https://www.foxit.com/pdf-reader/|di:foxit
Documents|Joplin|Joplin.Joplin|joplinapp.org|https://joplinapp.org/|sh:joplin
Documents|LibreOffice|TheDocumentFoundation.LibreOffice|libreoffice.org|https://www.libreoffice.org/|sh:libreoffice
Documents|Obsidian|Obsidian.Obsidian|obsidian.md|https://obsidian.md/|sh:obsidian
Documents|Okular|KDE.Okular|okular.kde.org|https://okular.kde.org/|
Documents|ONLYOFFICE Desktop|ONLYOFFICE.DesktopEditors|onlyoffice.com|https://www.onlyoffice.com/desktop.aspx|sh:onlyoffice
Documents|PDF24 Creator|geeksoftwareGmbH.PDF24Creator|pdf24.org|https://tools.pdf24.org/en/creator|di:pdf24
Documents|PDFgear|PDFgear.PDFgear|pdfgear.com|https://www.pdfgear.com/|
Documents|Simplenote|Automattic.Simplenote|simplenote.com|https://simplenote.com/|si:simplenote:3361CC
Documents|Sumatra PDF|SumatraPDF.SumatraPDF|github.com/sumatrapdfreader|https://www.sumatrapdfreader.org/free-pdf-reader.html|
Documents|Xournal++|Xournal++.Xournal++|xournalpp.github.io|https://xournalpp.github.io/|
Documents|Zotero|DigitalScholar.Zotero|zotero.org|https://www.zotero.org/|sh:zotero
Games|Battle.net|Blizzard.BattleNet|download.battle.net|https://battle.net|si:battledotnet:4381C3
Games|Cemu|Cemu.Cemu|github.com/cemu-project|https://cemu.info/|
Games|EA App|ElectronicArts.EADesktop|ea.com|https://www.ea.com/ea-app|si:ea:000000
Games|Epic Games Launcher|EpicGames.EpicGamesLauncher|epicgames.com|https://www.epicgames.com/store/en-US/|sh:epic-games
Games|GeForce NOW|Nvidia.GeForceNow|nvidia.com|https://www.nvidia.com/en-us/geforce-now/|
Games|GOG Galaxy|GOG.Galaxy|gog.com|https://www.gog.com/galaxy|
Games|Heroic Games Launcher|HeroicGamesLauncher.HeroicGamesLauncher|github.com/Heroic-Games-Launcher|https://heroicgameslauncher.com/|si:heroicgameslauncher:4B93FF
Games|Modrinth App|Modrinth.ModrinthApp|modrinth.com|https://modrinth.com/app|sh:modrinth
Games|Moonlight|MoonlightGameStreamingProject.Moonlight|moonlight-stream.org|https://moonlight-stream.org/|
Games|Playnite|Playnite.Playnite|playnite.link|https://playnite.link/|
Games|Prism Launcher|PrismLauncher.PrismLauncher|github.com/PrismLauncher|https://prismlauncher.org/|
Games|Roblox|Roblox.Roblox|roblox.com|https://www.roblox.com/|sh:roblox
Games|Steam|Valve.Steam|store.steampowered.com|https://store.steampowered.com/about/|sh:steam
Games|Sunshine|LizardByte.Sunshine|github.com/LizardByte|https://app.lizardbyte.dev/Sunshine/|sh:sunshine
Games|Ubisoft Connect|Ubisoft.Connect|ubisoftconnect.com|https://ubisoftconnect.com/|
Microsoft Tools|.NET Desktop Runtime 8|Microsoft.DotNet.DesktopRuntime.8|dotnet.microsoft.com|https://dotnet.microsoft.com/download/dotnet/8.0|
Microsoft Tools|.NET Desktop Runtime 9|Microsoft.DotNet.DesktopRuntime.9|dotnet.microsoft.com|https://dotnet.microsoft.com/download/dotnet/9.0|
Microsoft Tools|.NET Desktop Runtime 10|Microsoft.DotNet.DesktopRuntime.10|dotnet.microsoft.com|https://dotnet.microsoft.com/download/dotnet/10.0|
Microsoft Tools|Autoruns|Microsoft.Sysinternals.Autoruns|sysinternals.com|https://learn.microsoft.com/en-us/sysinternals/downloads/autoruns|
Microsoft Tools|OneDrive|Microsoft.OneDrive|microsoft.com|https://onedrive.live.com/|
Microsoft Tools|PowerToys|Microsoft.PowerToys|github.com/microsoft|https://github.com/microsoft/PowerToys|
Microsoft Tools|Process Explorer|Microsoft.Sysinternals.ProcessExplorer|sysinternals.com|https://learn.microsoft.com/sysinternals/downloads/process-explorer|
Microsoft Tools|Process Monitor|Microsoft.Sysinternals.ProcessMonitor|sysinternals.com|https://docs.microsoft.com/en-us/sysinternals/downloads/procmon|
Microsoft Tools|RDCMan|Microsoft.Sysinternals.RDCMan|sysinternals.com|https://learn.microsoft.com/en-us/sysinternals/downloads/rdcman|
Microsoft Tools|TCPView|Microsoft.Sysinternals.TCPView|sysinternals.com|https://docs.microsoft.com/en-us/sysinternals/downloads/tcpview|
Microsoft Tools|Visual C++ 2015-2022 x64|Microsoft.VCRedist.2015+.x64|microsoft.com|https://support.microsoft.com/en-us/help/2977003/the-latest-supported-visual-c-downloads|
Microsoft Tools|Visual C++ 2015-2022 x86|Microsoft.VCRedist.2015+.x86|microsoft.com|https://support.microsoft.com/en-us/help/2977003/the-latest-supported-visual-c-downloads|
Multimedia|Audacity|Audacity.Audacity|audacityteam.org|https://www.audacityteam.org/|sh:audacity
Multimedia|Blender|BlenderFoundation.Blender|blender.org|https://www.blender.org/|sh:blender
Multimedia|Calibre|calibre.calibre|calibre-ebook.com|https://calibre-ebook.com/|sh:calibre
Multimedia|foobar2000|PeterPawlowski.foobar2000|foobar2000.org|https://www.foobar2000.org/|si:foobar2000:000000
Multimedia|GIMP|GIMP.GIMP.3|gimp.org|https://www.gimp.org/|sh:gimp
Multimedia|HandBrake|HandBrake.HandBrake|handbrake.fr|https://handbrake.fr/|sh:handbrake
Multimedia|ImageGlass|DuongDieuPhap.ImageGlass|imageglass.org|https://imageglass.org/|
Multimedia|Inkscape|Inkscape.Inkscape|inkscape.org||sh:inkscape
Multimedia|iTunes|Apple.iTunes|apple.com|https://www.apple.com/itunes/|si:itunes:FB5BC5
Multimedia|IrfanView|IrfanSkiljan.IrfanView|irfanview.com|https://irfanview.com/|
Multimedia|K-Lite Codec Pack Mega|CodecGuide.K-LiteCodecPack.Mega|codecguide.com||
Multimedia|Kdenlive|KDE.Kdenlive|kdenlive.org||si:kdenlive:527EB2
Multimedia|Krita|KDE.Krita|krita.org||si:krita:3BABFF
Multimedia|Media Player Classic (MPC-HC)|clsid2.mpc-hc|github.com/clsid2|https://mpc-hc.org/|
Multimedia|OBS Studio|OBSProject.OBSStudio|obsproject.com|https://obsproject.com/|si:obsstudio:302E31
Multimedia|Paint.NET|dotPDN.PaintDotNet|getpaint.net|https://www.getpaint.net/|
Multimedia|ShareX|ShareX.ShareX|getsharex.com|https://getsharex.com/|si:sharex:2885F1
Multimedia|Spotify|Spotify.Spotify|spotify.com||sh:spotify
Multimedia|VLC|VideoLAN.VLC|github.com/videolan|https://www.videolan.org/vlc/|si:vlcmediaplayer:FF8800
Utilities|7-Zip|7zip.7zip|7-zip.org|https://www.7-zip.org/|sh:7-zip
Utilities|AnyDesk|AnyDesk.AnyDesk|anydesk.com|https://anydesk.com/|di:anydesk
Utilities|balenaEtcher|Balena.Etcher|etcher.balena.io||
Utilities|BleachBit|BleachBit.BleachBit|bleachbit.org||
Utilities|Bitwarden|Bitwarden.Bitwarden|bitwarden.com|https://bitwarden.com/|sh:bitwarden
Utilities|Bulk Crap Uninstaller|Klocman.BulkCrapUninstaller|bcuninstaller.com|https://www.bcuninstaller.com/|
Utilities|CrystalDiskInfo|CrystalDewWorld.CrystalDiskInfo|crystalmark.info|https://crystalmark.info/en/software/crystaldiskinfo/|
Utilities|CrystalDiskMark|CrystalDewWorld.CrystalDiskMark|crystalmark.info|https://crystalmark.info/en/software/crystaldiskmark/|
Utilities|Display Driver Uninstaller|Wagnardsoft.DisplayDriverUninstaller|wagnardsoft.com|https://www.wagnardsoft.com/display-driver-uninstaller-DDU-|
Utilities|Everything|voidtools.Everything|voidtools.com|https://www.voidtools.com/|
Utilities|Flow Launcher|Flow-Launcher.Flow-Launcher|github.com/Flow-Launcher||
Utilities|GPU-Z|TechPowerUp.GPU-Z|techpowerup.com|https://www.techpowerup.com/gpuz/|
Utilities|CPU-Z|CPUID.CPU-Z|cpuid.com|https://www.cpuid.com/softwares/cpu-z.html|
Utilities|HWiNFO|REALiX.HWiNFO|hwinfo.com|https://www.hwinfo.com/|
Utilities|HWMonitor|CPUID.HWMonitor|cpuid.com|https://www.cpuid.com/softwares/hwmonitor.html|
Utilities|KeePassXC|KeePassXCTeam.KeePassXC|keepassxc.org|https://keepassxc.org/|sh:keepassxc
Utilities|Malwarebytes|Malwarebytes.Malwarebytes|malwarebytes.com||si:malwarebytes:0D3ECC
Utilities|MSI Afterburner|Guru3D.Afterburner|msi.com|https://www.msi.com/Landing/afterburner|
Utilities|Mullvad VPN|MullvadVPN.MullvadVPN|mullvad.net|https://mullvad.net/|sh:mullvad-vpn
Utilities|Nmap|Insecure.Nmap|nmap.org|https://nmap.org/|
Utilities|OpenVPN Connect|OpenVPNTechnologies.OpenVPNConnect|openvpn.net|https://openvpn.net/|sh:openvpn
Utilities|Proton VPN|Proton.ProtonVPN|protonvpn.com|https://protonvpn.com/|sh:proton-vpn
Utilities|PuTTY|PuTTY.PuTTY|putty.org|https://www.chiark.greenend.org.uk/~sgtatham/putty/|sh:putty
Utilities|qBittorrent|qBittorrent.qBittorrent|qbittorrent.org|https://www.qbittorrent.org/|sh:qbittorrent
Utilities|Revo Uninstaller|RevoUninstaller.RevoUninstaller|revouninstaller.com|https://www.revouninstaller.com/|
Utilities|Rufus|Rufus.Rufus|rufus.ie|https://rufus.ie/|
Utilities|Tailscale|Tailscale.Tailscale|tailscale.com|https://tailscale.com/|sh:tailscale
Utilities|TeamViewer|TeamViewer.TeamViewer|teamviewer.com|https://www.teamviewer.com/|sh:teamviewer
Utilities|TreeSize Free|JAMSoftware.TreeSize.Free|jam-software.com|https://www.jam-software.com/treesize_free/|di:treesize
Utilities|Ventoy|Ventoy.Ventoy|ventoy.net|https://www.ventoy.net/|
Utilities|WinRAR|RARLab.WinRAR|win-rar.com|https://www.win-rar.com/|
Utilities|WinSCP|WinSCP.WinSCP|winscp.net|https://winscp.net/|
Utilities|WireGuard|WireGuard.WireGuard|wireguard.com|https://www.wireguard.com/|sh:wireguard
Utilities|Wireshark|WiresharkFoundation.Wireshark|wireshark.org|https://www.wireshark.org/|di:wireshark
Utilities|WizTree|AntibodySoftware.WizTree|diskanalyzer.com|https://wiztreefree.com/|
Selfhosted|Jellyfin Server|Jellyfin.Server|jellyfin.org|https://jellyfin.org/|sh:jellyfin
Selfhosted|Kodi|XBMCFoundation.Kodi|kodi.tv|https://kodi.tv/|sh:kodi
Selfhosted|Nextcloud Desktop|Nextcloud.NextcloudDesktop|nextcloud.com|https://nextcloud.com/install/#install-clients|sh:nextcloud
Selfhosted|Plex Desktop|Plex.Plex|plex.tv|https://www.plex.tv|sh:plex
Selfhosted|Plex Media Server|Plex.PlexMediaServer|plex.tv|https://www.plex.tv/your-media/|sh:plex
Browsers|Helium|ImputNet.Helium|helium.computer|https://helium.computer|si:helium:0ACF83
Browsers|Ungoogled Chromium|eloston.ungoogled-chromium|github.com/ungoogled-software|https://github.com/Eloston/ungoogled-chromium|
Communications|Chatterino|ChatterinoTeam.Chatterino|github.com/Chatterino|https://www.chatterino.com/|
Communications|Dorion|SpikeHD.Dorion|github.com/SpikeHD|https://spikehd.dev/projects/dorion/|
Communications|QTox|Tox.qTox|github.com/TokTok|https://qtox.github.io/|
Communications|Teams|Microsoft.Teams|microsoft.com|https://www.microsoft.com/en-us/microsoft-teams/group-chat-software|
Communications|TeamSpeak 6|TeamSpeakSystems.TeamSpeakClient.Beta.6|teamspeak.com|https://www.teamspeak.com/|sh:teamspeak
Development|Amazon Corretto 21 (LTS)|Amazon.Corretto.21.JDK|aws.amazon.com|https://aws.amazon.com/corretto|
Development|Amazon Corretto 25 (LTS)|Amazon.Corretto.25.JDK|aws.amazon.com|https://aws.amazon.com/corretto|
Development|Amazon Corretto 8 (LTS)|Amazon.Corretto.8.JDK|aws.amazon.com|https://aws.amazon.com/corretto|
Development|Claude Code|Anthropic.ClaudeCode|claude.ai|https://code.claude.com/|di:anthropic
Development|Codex|OpenAI.Codex|github.com/openai|https://developers.openai.com/codex/cli|di:codex
Development|DBeaver Community|DBeaver.DBeaver.Community|dbeaver.io||si:dbeaver:382923
Development|HeidiSQL|HeidiSQL.HeidiSQL|heidisql.com||
Development|LM Studio|ElementLabs.LMStudio|lmstudio.ai||si:lmstudio:000000
Development|Lua|rjpcomputing.luaforwindows|lua.org|https://github.com/rjpcomputing/luaforwindows|di:lua
Development|MongoDB Compass|MongoDB.Compass.Full|mongodb.com||sh:mongodb
Development|Ollama|Ollama.Ollama|ollama.com||sh:ollama
Development|Python3|Python.Python.3.14|python.org|https://www.python.org/|sh:python
Development|Ruby|RubyInstallerTeam.Ruby.4.0|rubyinstaller.org|https://rubyinstaller.org/|di:ruby
Development|Rust|Rustlang.Rust.MSVC|rust-lang.org|https://www.rust-lang.org/|sh:rust
Development|System Informer|WinsiderSS.SystemInformer|system-informer.com|https://systeminformer.com/|
Development|Vagrant|Hashicorp.Vagrant|developer.hashicorp.com|https://developer.hashicorp.com/vagrant|si:vagrant:1868F2
Development|Visual Studio 2026|Microsoft.VisualStudio.Community|visualstudio.microsoft.com|https://visualstudio.microsoft.com/|
Documents|Figma|Figma.Figma|figma.com||sh:figma
Documents|NAPS2 (Scanner)|Cyanfish.NAPS2|naps2.com|https://www.naps2.com/|
Documents|Notion|Notion.Notion|notion.com||sh:notion
Documents|PDFsam Basic|PDFsam.PDFsam|pdfsam.org|https://pdfsam.org/|
Documents|PDF-XChange Editor|TrackerSoftware.PDF-XChangeEditor|pdf-xchange.com|https://www.pdf-xchange.com/|
Documents|QOwnNotes|pbek.QOwnNotes|github.com/pbek|https://www.qownnotes.org/|
Games|CurseForge|Overwolf.CurseForge|curseforge.overwolf.com|https://www.overwolf.com/app/overwolf-curseforge|si:curseforge:F16436
Games|EmulationStation Desktop Edition|ES-DE.EmulationStation-DE|es-de.org|https://es-de.org/|
Games|Itch.io|ItchIo.Itch|itch.io|https://itch.io/|di:itch
Games|Virtual Desktop Streamer|VirtualDesktop.Streamer|vrdesktop.net|https://www.vrdesktop.net/|
Microsoft Tools|.NET Desktop Runtime 6|Microsoft.DotNet.DesktopRuntime.6|dotnet.microsoft.com|https://dotnet.microsoft.com/download/dotnet/6.0|
Microsoft Tools|DISMTools|CodingWondersSoftware.DISMTools.Stable|github.com/CodingWonders|https://github.com/CodingWonders/DISMTools|
Microsoft Tools|NTLite|Nlitesoft.NTLite|ntlite.com|https://ntlite.com|
Microsoft Tools|NuGet|Microsoft.NuGet|nuget.org|https://www.nuget.org/|si:nuget:004880
Multimedia|AIMP (Music Player)|AIMP.AIMP|aimp.ru|https://www.aimp.ru/|
Multimedia|EarTrumpet (Audio)|File-New-Project.EarTrumpet|github.com/File-New-Project|https://eartrumpet.app/|
Multimedia|File Converter|AdrienAllard.FileConverter|file-converter.io|https://file-converter.io/|
Multimedia|Greenshot|Greenshot.Greenshot|getgreenshot.org||
Multimedia|K-Lite Codec Standard|CodecGuide.K-LiteCodecPack.Standard|codecguide.com|https://www.codecguide.com/|
Multimedia|Lively Wallpaper|rocksdanister.LivelyWallpaper|github.com/rocksdanister||
Multimedia|mpc-qt|mpc-qt.mpc-qt|github.com/mpc-qt|https://mpc-qt.github.io|
Multimedia|mpv|shinchiro.mpv|github.com/shinchiro|https://mpv.io/|si:mpv:691F69
Multimedia|nomacs|nomacs.nomacs|nomacs.org|https://nomacs.org/|
Multimedia|Rainmeter|Rainmeter.Rainmeter|rainmeter.net||si:rainmeter:19519B
Multimedia|Streamlabs|Streamlabs.Streamlabs|streamlabs.com||si:streamlabs:80F5D2
Pro Tools|Advanced IP Scanner|Famatech.AdvancedIPScanner|advanced-ip-scanner.com|https://www.advanced-ip-scanner.com/|
Pro Tools|Angry IP Scanner|angryziber.AngryIPScanner|angryip.org|https://angryip.org/|
Pro Tools|Cinebench R23|Maxon.CinebenchR23|maxon.net|https://www.maxon.net/en/cinebench|
Pro Tools|gsudo|gerardog.gsudo|github.com/gerardog|https://github.com/gerardog/gsudo|
Pro Tools|Simplewall|Henry++.simplewall|github.com/henrypp|https://github.com/henrypp/simplewall|
Selfhosted|Jellyfin Media Player|Jellyfin.JellyfinMediaPlayer|github.com/jellyfin|https://jellyfin.org/|sh:jellyfin
Selfhosted|LocalSend|LocalSend.LocalSend|localsend.org|https://localsend.org/|sh:localsend
Selfhosted|NetBird|Netbird.Netbird|netbird.io|https://netbird.io/|sh:netbird
Selfhosted|Syncthing (CLI / Web UI)|Syncthing.Syncthing|github.com/syncthing|https://syncthing.net/|sh:syncthing
Selfhosted|SyncTrayzor|GermanCoding.SyncTrayzor|github.com/GermanCoding|https://github.com/GermanCoding/SyncTrayzor|
Utilities|1Password|AgileBits.1Password|1password.com|https://1password.com/|sh:1password
Utilities|AB Download Manager|amir1376.ABDownloadManager|github.com/amir1376|https://abdownloadmanager.com/|si:abdownloadmanager:897BFF
Utilities|AutoHotkey|AutoHotkey.AutoHotkey|autohotkey.com|https://www.autohotkey.com/|si:autohotkey:334455
Utilities|BlurAutoClicker|Blur009.BlurAutoClicker|github.com/Blur009|https://blur009.vercel.app/projects/blur-autoclicker/|
Utilities|Cloudflare WARP|Cloudflare.Warp|one.one.one.one|https://one.one.one.one|sh:cloudflare
Utilities|Corsair iCUE|Corsair.iCUE.5|corsair.com||
Utilities|Deskflow|Deskflow.Deskflow|github.com/deskflow|https://github.com/deskflow/deskflow|
Utilities|Ditto Clipboard|Ditto.Ditto|github.com/sabrogden||
Utilities|Dropbox|Dropbox.Dropbox|dropbox.com|https://www.dropbox.com/desktop|sh:dropbox
Utilities|Ente Auth|ente-io.auth-desktop|github.com/ente-io|https://ente.io/auth/|sh:ente-auth
Utilities|F.lux|flux.flux|justgetflux.com|https://justgetflux.com/|
Utilities|Files|FilesCommunity.Files|github.com/files-community|https://files.community|
Utilities|GlazeWM|glzr-io.glazewm|github.com/glzr-io|https://github.com/glzr-io/glazewm|
Utilities|Google Drive|Google.GoogleDrive|workspace.google.com|https://www.google.com/drive/|sh:google-drive
Utilities|Hugo|Hugo.Hugo.Extended|gohugo.io|https://gohugo.io|sh:hugo
Utilities|HxD Hex Editor|MHNexus.HxD|mh-nexus.de|https://mh-nexus.de/en/hxd/|
Utilities|Intel Driver and Support Assistant|Intel.IntelDriverAndSupportAssistant|intel.com||
Utilities|Internet Download Manager|Tonec.InternetDownloadManager|internetdownloadmanager.com|https://www.internetdownloadmanager.com/|
Utilities|JPEG View|sylikc.JPEGView|github.com/sylikc|https://github.com/sylikc/jpegview|
Utilities|Logitech G HUB|Logitech.GHUB|support.logi.com||di:logitech
Utilities|MiniTool Partition Wizard|MiniTool.PartitionWizard.Free|partitionwizard.com|https://www.partitionwizard.com/|
Utilities|MSEdgeRedirect|rcmaehl.MSEdgeRedirect|github.com/rcmaehl|https://github.com/rcmaehl/MSEdgeRedirect|
Utilities|NanaZip|M2Team.NanaZip|github.com/M2Team|https://nanazip.org|
Utilities|Nilesoft Shell|Nilesoft.Shell|nilesoft.org|https://nilesoft.org/|
Utilities|NVCleanstall|TechPowerUp.NVCleanstall|techpowerup.com|https://www.techpowerup.com/nvcleanstall/|
Utilities|OFGB (Oh Frick Go Back)|xM4ddy.OFGB|github.com/xM4ddy|https://github.com/xM4ddy/OFGB|
Utilities|OPAutoClicker|OPAutoClicker.OPAutoClicker|opautoclicker.com|https://www.opautoclicker.com|
Utilities|OpenRGB|OpenRGB.OpenRGB|openrgb.org|https://openrgb.org/|
Utilities|Oracle VirtualBox|Oracle.VirtualBox|virtualbox.org|https://www.virtualbox.org/|sh:virtualbox
Utilities|Parsec|Parsec.Parsec|parsec.app|https://parsec.app/|
Utilities|PeaZip|Giorgiotani.Peazip|peazip.github.io|https://peazip.github.io/|
Utilities|Policy Plus|Fleex255.PolicyPlus|github.com/Fleex255|https://github.com/Fleex255/PolicyPlus|
Utilities|Process Lasso|BitSum.ProcessLasso|bitsum.com|https://bitsum.com/|
Utilities|Proton Authenticator|Proton.ProtonAuthenticator|proton.me|https://proton.me/authenticator|
Utilities|Proton Drive|Proton.ProtonDrive|proton.me|https://proton.me/drive|sh:proton-drive
Utilities|Proton Pass|Proton.ProtonPass|proton.me|https://proton.me/pass|sh:proton-pass
Utilities|SignalRGB|WhirlwindFX.SignalRgb|signalrgb.com|https://www.signalrgb.com/|
Utilities|Snappy Driver Installer Origin|GlennDelahoy.SnappyDriverInstallerOrigin|sdi-tool.org|https://www.glenn.delahoy.com/snappy-driver-installer-origin/|
Utilities|Speccy|Piriform.Speccy|ccleaner.com||
Utilities|StartAllBack|StartIsBack.StartAllBack|startallback.com|https://www.startallback.com/|
Utilities|TightVNC|GlavSoft.TightVNC|tightvnc.com|https://www.tightvnc.com/|
Utilities|Total Commander|Ghisler.TotalCommander|ghisler.com|https://www.ghisler.com/|
Utilities|TranslucentTB|CharlesMilette.TranslucentTB|github.com/TranslucentTB|https://translucenttb.github.io|
Utilities|UniGetUI|Devolutions.UniGetUI|devolutions.net|https://devolutions.net/unigetui/|
Utilities|WinDirStat|WinDirStat.WinDirStat|github.com/windirstat||di:windirstat
Utilities|Wise Program Uninstaller (WiseCleaner)|WiseCleaner.WiseProgramUninstaller|wisecleaner.com|https://www.wisecleaner.com/wise-program-uninstaller.html|
'@ -split "`r?`n" | Where-Object { $_ } | ForEach-Object {
    $p = $_ -split '\|', 6
    [pscustomobject]@{ Cat = $p[0]; Name = $p[1]; Id = $p[2]; Domain = $p[3]; Link = $p[4]; Pack = $p[5] }
}
