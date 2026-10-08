<#
 Optimaxer - Windows performance, privacy and cleanup suite.
 Run via Launch.bat (elevates automatically) or: powershell -STA -ExecutionPolicy Bypass -File Optimaxer.ps1
#>
param(
    [switch]$NoElevate,        # developer: skip UAC relaunch
    [string]$Screenshot,       # developer: save a PNG of every page to this folder, then exit
    [switch]$Silent,           # unattended: apply -Preset / -Config without opening the window
    [string]$Preset,           # safe | gaming | privacy | deai | max
    [string]$Config,           # json exported from the Config tab (tweaks and apps)
    [switch]$NoRestorePoint
)

$global:OptiRoot = $PSScriptRoot
$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
$sta = [Threading.Thread]::CurrentThread.ApartmentState -eq 'STA'

$isCore = $PSVersionTable.PSEdition -eq 'Core'   # Appx/DISM cmdlets need Windows PowerShell 5.1
if ((-not $isAdmin -and -not $NoElevate) -or -not $sta -or $isCore) {
    $argLine = "-NoProfile -ExecutionPolicy Bypass -STA -WindowStyle $(if ($Silent) { 'Normal' } else { 'Hidden' }) -File `"$PSCommandPath`""
    if ($Silent) { $argLine += ' -Silent' }
    if ($Preset) { $argLine += " -Preset `"$Preset`"" }
    if ($Config) { $argLine += " -Config `"$Config`"" }
    if ($NoRestorePoint) { $argLine += ' -NoRestorePoint' }
    if ($NoElevate) { $argLine += ' -NoElevate' }
    if ($Screenshot) { $argLine += " -Screenshot `"$Screenshot`"" }
    $verb = if ($isAdmin -or $NoElevate) { 'Open' } else { 'RunAs' }
    try { Start-Process powershell.exe -ArgumentList $argLine -Verb $verb } catch { Write-Warning 'Administrator rights are required.' }
    exit
}

$ErrorActionPreference = 'Continue'
. "$PSScriptRoot\src\Core.ps1"
. "$PSScriptRoot\src\Data.ps1"
. "$PSScriptRoot\src\Catalog.ps1"
if ($Silent) {
    # unattended mode: no window, progress on the console and in the log file
    $global:OptiConsole = $true
    . "$PSScriptRoot\src\Apps.ps1"
    $ids = @(); $apps = @()
    if ($Config) {
        if (-not (Test-Path -LiteralPath $Config)) { Write-Log "Config file not found: $Config" 'ERROR'; exit 2 }
        $cfg = Get-Content -LiteralPath $Config -Raw | ConvertFrom-Json
        $ids = @($cfg.tweaks); $apps = @($cfg.apps)
    }
    if ($Preset) {
        if ($Preset -notin 'safe', 'gaming', 'privacy', 'deai', 'max') { Write-Log "Unknown preset '$Preset' (use safe, gaming, privacy, deai or max)" 'ERROR'; exit 2 }
        $ids += @(Get-PresetTweakIds $Preset (Get-SystemTraits))
    }
    $ids = @($ids | Where-Object { $_ } | Select-Object -Unique)
    if (-not $ids.Count -and -not $apps.Count) { Write-Log 'Nothing to do: pass -Preset and/or -Config.' 'ERROR'; exit 2 }
    Write-Log "Unattended run: $($ids.Count) tweak(s), $($apps.Count) app(s)"
    if ($ids.Count -and -not $NoRestorePoint) { [void](New-OptiRestorePoint 'Optimaxer - unattended run') }
    $ok = 0; $bad = 0
    foreach ($id in $ids) {
        $t = Get-TweakById $id
        if (-not $t) { Write-Log "Unknown tweak id: $id" 'WARN'; $bad++; continue }
        if (Invoke-TweakApply $t) { $ok++ } else { $bad++; Write-Log "  not verified: $($t.Name)" 'WARN' }
    }
    if ($apps.Count) { Invoke-WingetPackages -Action Install -Programs $apps }
    Write-Log "Done: $ok tweak(s) verified, $bad not verified." $(if ($bad) { 'WARN' } else { 'OK' })
    if ($PSCommandPath -and -not $NoElevate) { Start-Sleep -Seconds 4 }
    exit $(if ($bad) { 1 } else { 0 })
}
. "$PSScriptRoot\src\App.ps1"

if ($Screenshot) {
    New-Item -ItemType Directory -Path $Screenshot -Force | Out-Null
    function Wait-UI([double]$Sec) {
        $end = (Get-Date).AddSeconds($Sec)
        while ((Get-Date) -lt $end) { $win.Dispatcher.Invoke([Action]{}, 'Background'); Start-Sleep -Milliseconds 60 }
    }
    function Save-Shot($name) {
        $win.UpdateLayout(); Wait-UI 0.3
        $g = $UI.RootGrid
        $bmp = New-Object Windows.Media.Imaging.RenderTargetBitmap([int]$g.ActualWidth, [int]$g.ActualHeight, 96, 96, [Windows.Media.PixelFormats]::Pbgra32)
        $bmp.Render($g)
        $enc = New-Object Windows.Media.Imaging.PngBitmapEncoder
        $enc.Frames.Add([Windows.Media.Imaging.BitmapFrame]::Create($bmp))
        $fs = [IO.File]::Create((Join-Path $Screenshot "$name.png")); $enc.Save($fs); $fs.Close()
    }
    $win.Show()
    Wait-UI 9
    foreach ($n in 'Dash', 'Install', 'Tweaks', 'Config', 'Services', 'Startup', 'Cleanup', 'Debloat', 'Network', 'Features', 'Tools', 'Rice') {
        $UI["Nav$n"].IsChecked = $true
        Wait-UI 4
        Save-Shot $n.ToLower()
    }
    Write-Host 'screenshots done'
    $win.Close()
    exit
}

[void]$win.ShowDialog()
