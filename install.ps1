<#
 Optimaxer installer / launcher.
   irm https://raw.githubusercontent.com/bliper2/optimaxer/main/install.ps1 | iex

 What this script does, in order (each step is logged to the console and to <install dir>\install.log):
   1. Asks the GitHub Releases API for the latest release of the repository.
   2. Downloads the release zip into a staging folder next to the install folder (not %TEMP%).
   3. Verifies the zip's SHA-256 against the digest GitHub publishes for that asset. No digest, or a
      mismatch, aborts the install. Nothing is extracted before the hash matches.
   4. Extracts to the staging folder, rejecting any entry that would land outside it.
   5. Copies the files to %LOCALAPPDATA%\Optimaxer (the old install is kept in a .previous folder).
   6. Optionally creates Start menu / desktop shortcuts and starts the app.
 The installer runs no downloaded binaries. The app is plain PowerShell/XAML source you can read in the repo.
#>
param(
    [string]$Dir = (Join-Path $env:LOCALAPPDATA 'Optimaxer'),
    [switch]$NoLaunch,
    [switch]$NoShortcut,
    [string]$Repo = 'bliper2/optimaxer'
)
$ErrorActionPreference = 'Stop'
# GitHub only accepts TLS 1.2+, and Windows PowerShell 5.1 does not enable it by default.
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$script:LogFile = $null
function Write-Step {
    # Console message plus a line in the install log (the log exists once the install folder does).
    param([Parameter(Mandatory)][string]$Message, [string]$Color = 'Gray')
    Write-Host "[Optimaxer] $Message" -ForegroundColor $Color
    if ($script:LogFile) {
        Add-Content -LiteralPath $script:LogFile -Value ('{0:s}  {1}' -f (Get-Date), $Message) -ErrorAction SilentlyContinue
    }
}

function Get-LatestReleaseInfo {
    # Why: the release metadata carries the download URL and the SHA-256 digest GitHub computed on upload.
    param([Parameter(Mandatory)][string]$Repository)
    $headers = @{ 'User-Agent' = 'Optimaxer-Installer'; 'Accept' = 'application/vnd.github+json' }
    $rel = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repository/releases/latest" -Headers $headers -TimeoutSec 20 -ErrorAction Stop
    $asset = @($rel.assets | Where-Object { $_.name -like 'Optimaxer-v*.zip' })[0]
    if (-not $asset) { throw 'The latest release has no Optimaxer zip.' }
    $hash = $null
    if ([string]$asset.digest -match '^sha256:(?<h>[0-9a-fA-F]{64})$') { $hash = $Matches['h'].ToUpperInvariant() }
    [pscustomobject]@{ Tag = [string]$rel.tag_name; Url = [string]$asset.browser_download_url; Sha256 = $hash; Name = [string]$asset.name }
}

function Invoke-VerifiedDownload {
    # Why: download, then compare SHA-256 before anything else touches the file. A bad file is deleted.
    param(
        [Parameter(Mandatory)][string]$Url,
        [Parameter(Mandatory)][string]$OutFile,
        [Parameter(Mandatory)][AllowNull()][string]$ExpectedSha256
    )
    if (-not $ExpectedSha256) { throw 'This release publishes no SHA-256 digest, so the download cannot be verified. Aborting.' }
    if ($Url -notmatch '^https://') { throw "Refusing a non-HTTPS download URL: $Url" }
    Invoke-WebRequest -Uri $Url -OutFile $OutFile -UseBasicParsing -TimeoutSec 180 -Headers @{ 'User-Agent' = 'Optimaxer-Installer' } -ErrorAction Stop
    $actual = (Get-FileHash -LiteralPath $OutFile -Algorithm SHA256 -ErrorAction Stop).Hash
    if ($actual -ne $ExpectedSha256.ToUpperInvariant()) {
        Remove-Item -LiteralPath $OutFile -Force -ErrorAction SilentlyContinue
        throw "Checksum mismatch (expected $ExpectedSha256, got $actual). Download rejected."
    }
    Write-Step "SHA-256 verified: $actual" 'Green'
}

function Expand-SafeArchive {
    # Why: Expand-Archive in 5.1 does not reject entries like ..\..\x ("zip slip"), so check every entry first.
    param([Parameter(Mandatory)][string]$Zip, [Parameter(Mandatory)][string]$Destination)
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    New-Item -ItemType Directory -Path $Destination -Force -ErrorAction Stop | Out-Null
    $root = [IO.Path]::GetFullPath($Destination).TrimEnd('\') + '\'
    $archive = [IO.Compression.ZipFile]::OpenRead($Zip)
    try {
        foreach ($entry in $archive.Entries) {
            $target = [IO.Path]::GetFullPath((Join-Path $Destination $entry.FullName))
            if (-not $target.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) { throw "Unsafe path in archive: $($entry.FullName)" }
        }
    } finally { $archive.Dispose() }
    Expand-Archive -LiteralPath $Zip -DestinationPath $Destination -Force -ErrorAction Stop
}

function Install-AppFiles {
    # Why: keep one previous copy so a bad update can be rolled back by hand.
    param([Parameter(Mandatory)][string]$Source, [Parameter(Mandatory)][string]$Target)
    if (Test-Path -LiteralPath $Target) {
        $prev = "$Target.previous"
        if (Test-Path -LiteralPath $prev) { Remove-Item -LiteralPath $prev -Recurse -Force -ErrorAction Stop }
        Copy-Item -LiteralPath $Target -Destination $prev -Recurse -Force -ErrorAction Stop
        Write-Step "Existing install copied to $prev"
    }
    New-Item -ItemType Directory -Path $Target -Force -ErrorAction Stop | Out-Null
    Copy-Item -Path (Join-Path $Source '*') -Destination $Target -Recurse -Force -ErrorAction Stop
}

function New-AppShortcut {
    # Why: the app edits system settings, so the shortcut is flagged "run as administrator" (bit 0x20 of byte 0x15 in
    # the .lnk header) and Windows shows its normal UAC prompt. The console window is minimized, not hidden.
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][string]$AppDir)
    $ps = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $argLine = "-NoProfile -ExecutionPolicy RemoteSigned -STA -WindowStyle Minimized -File `"$(Join-Path $AppDir 'Optimaxer.ps1')`""
    $icon = Join-Path $AppDir 'assets\optimaxer.ico'
    $lnk = (New-Object -ComObject WScript.Shell).CreateShortcut($Path)
    $lnk.TargetPath = $ps; $lnk.Arguments = $argLine; $lnk.WorkingDirectory = $AppDir; $lnk.WindowStyle = 7; $lnk.Description = 'Optimaxer'
    if (Test-Path -LiteralPath $icon) { $lnk.IconLocation = $icon }
    $lnk.Save()
    $bytes = [IO.File]::ReadAllBytes($Path); $bytes[0x15] = $bytes[0x15] -bor 0x20; [IO.File]::WriteAllBytes($Path, $bytes)
}

# ---- main ----
if ($PSVersionTable.PSVersion.Major -lt 5) { throw 'Windows PowerShell 5.1 or newer is required.' }

Write-Step 'Looking up the latest release...'
$release = Get-LatestReleaseInfo -Repository $Repo
Write-Step "Latest is $($release.Tag)" 'Cyan'

# Staging lives beside the install folder (user-writable, easy to find), not in %TEMP%.
$staging = "$Dir.staging"
if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force -ErrorAction Stop }
New-Item -ItemType Directory -Path $staging -Force -ErrorAction Stop | Out-Null
Write-Step "Staging folder: $staging"

try {
    $zip = Join-Path $staging 'optimaxer.zip'
    Write-Step "Downloading $($release.Name)..."
    Invoke-VerifiedDownload -Url $release.Url -OutFile $zip -ExpectedSha256 $release.Sha256

    $stage = Join-Path $staging 'files'
    Write-Step 'Extracting...'
    Expand-SafeArchive -Zip $zip -Destination $stage
    if (-not (Test-Path -LiteralPath (Join-Path $stage 'Optimaxer.ps1'))) { throw 'The package does not contain Optimaxer.ps1.' }

    Install-AppFiles -Source $stage -Target $Dir
    $script:LogFile = Join-Path $Dir 'install.log'
    Write-Step "Installed $($release.Tag) to $Dir" 'Green'
} finally {
    Remove-Item -LiteralPath $staging -Recurse -Force -ErrorAction SilentlyContinue
}

if (-not $NoShortcut) {
    foreach ($path in @((Join-Path ([Environment]::GetFolderPath('Desktop')) 'Optimaxer.lnk'), (Join-Path ([Environment]::GetFolderPath('Programs')) 'Optimaxer.lnk'))) {
        New-AppShortcut -Path $path -AppDir $Dir
    }
    Write-Step 'Shortcuts created on the desktop and in the Start menu.' 'Green'
}

if (-not $NoLaunch) {
    Write-Step 'Starting Optimaxer (accept the administrator prompt)...'
    Start-Process -FilePath powershell.exe -Verb RunAs -ArgumentList "-NoProfile -ExecutionPolicy RemoteSigned -STA -WindowStyle Minimized -File `"$(Join-Path $Dir 'Optimaxer.ps1')`""
}
