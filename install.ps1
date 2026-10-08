<#
 Optimaxer installer / launcher.
   irm https://raw.githubusercontent.com/bliper2/optimaxer/main/install.ps1 | iex
 Downloads the latest release, verifies its SHA-256 checksum, installs it to %LOCALAPPDATA%\Optimaxer,
 adds Start menu and desktop shortcuts (run as administrator) and starts the app.
 Re-running it updates an existing install (the old files are kept in a .previous folder).
#>
param(
    [string]$Dir = (Join-Path $env:LOCALAPPDATA 'Optimaxer'),
    [switch]$NoLaunch,
    [switch]$NoShortcut,
    [string]$Repo = 'bliper2/optimaxer'
)
$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
function Say($m, $c = 'Gray') { Write-Host "[Optimaxer] $m" -ForegroundColor $c }

if ($PSVersionTable.PSVersion.Major -lt 5) { throw 'Windows PowerShell 5.1 or newer is required.' }

Say 'Looking up the latest release...'
$headers = @{ 'User-Agent' = 'Optimaxer-Installer'; 'Accept' = 'application/vnd.github+json' }
$rel = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -Headers $headers -TimeoutSec 20
$asset = @($rel.assets | Where-Object { $_.name -like 'Optimaxer-v*.zip' })[0]
if (-not $asset) { throw 'The latest release has no Optimaxer zip.' }
Say "Latest is $($rel.tag_name)" 'Cyan'

$work = Join-Path ([IO.Path]::GetTempPath()) ('optimaxer-install-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $work -Force | Out-Null
$zip = Join-Path $work 'optimaxer.zip'
Say 'Downloading...'
Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $zip -UseBasicParsing -TimeoutSec 180
if ([string]$asset.digest -match '^sha256:(?<h>[0-9a-fA-F]{64})$') {
    if ((Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash -ne $Matches['h']) { throw 'Checksum mismatch: download rejected.' }
    Say 'Checksum verified (SHA-256).' 'Green'
} else { Say 'No published checksum for this release; relying on HTTPS.' 'Yellow' }

$stage = Join-Path $work 'stage'
Expand-Archive -LiteralPath $zip -DestinationPath $stage -Force
if (-not (Test-Path (Join-Path $stage 'Optimaxer.ps1'))) { throw 'The package does not contain Optimaxer.ps1.' }

if (Test-Path -LiteralPath $Dir) {
    $prev = "$Dir.previous"
    if (Test-Path -LiteralPath $prev) { Remove-Item -LiteralPath $prev -Recurse -Force }
    Copy-Item -LiteralPath $Dir -Destination $prev -Recurse -Force
    Say "Existing install copied to $prev"
}
New-Item -ItemType Directory -Path $Dir -Force | Out-Null
Copy-Item -Path (Join-Path $stage '*') -Destination $Dir -Recurse -Force
Say "Installed to $Dir" 'Green'

if (-not $NoShortcut) {
    $ps = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $argLine = "-NoProfile -ExecutionPolicy Bypass -STA -WindowStyle Hidden -File `"$(Join-Path $Dir 'Optimaxer.ps1')`""
    $icon = Join-Path $Dir 'assets\optimaxer.ico'
    $targets = @((Join-Path ([Environment]::GetFolderPath('Desktop')) 'Optimaxer.lnk'), (Join-Path ([Environment]::GetFolderPath('Programs')) 'Optimaxer.lnk'))
    foreach ($lnkPath in $targets) {
        $sh = New-Object -ComObject WScript.Shell
        $lnk = $sh.CreateShortcut($lnkPath)
        $lnk.TargetPath = $ps; $lnk.Arguments = $argLine; $lnk.WorkingDirectory = $Dir; $lnk.WindowStyle = 7; $lnk.Description = 'Optimaxer'
        if (Test-Path -LiteralPath $icon) { $lnk.IconLocation = $icon }
        $lnk.Save()
        $bytes = [IO.File]::ReadAllBytes($lnkPath); $bytes[0x15] = $bytes[0x15] -bor 0x20; [IO.File]::WriteAllBytes($lnkPath, $bytes)   # run as administrator
    }
    Say 'Shortcuts created on the desktop and in the Start menu.' 'Green'
}

Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
if (-not $NoLaunch) {
    Say 'Starting Optimaxer (accept the administrator prompt)...'
    Start-Process -FilePath powershell.exe -Verb RunAs -ArgumentList "-NoProfile -ExecutionPolicy Bypass -STA -WindowStyle Hidden -File `"$(Join-Path $Dir 'Optimaxer.ps1')`""
}
