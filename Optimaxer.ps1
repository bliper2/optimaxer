<#
 Optimaxer - Windows performance, privacy and cleanup suite.
 Run via Launch.bat (elevates automatically) or: powershell -STA -ExecutionPolicy Bypass -File Optimaxer.ps1
#>
param(
    [switch]$NoElevate,        # developer: skip UAC relaunch
    [string]$Screenshot        # developer: save a PNG of every page to this folder, then exit
)

$global:OptiRoot = $PSScriptRoot
$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
$sta = [Threading.Thread]::CurrentThread.ApartmentState -eq 'STA'

$isCore = $PSVersionTable.PSEdition -eq 'Core'   # Appx/DISM cmdlets need Windows PowerShell 5.1
if ((-not $isAdmin -and -not $NoElevate) -or -not $sta -or $isCore) {
    $argLine = "-NoProfile -ExecutionPolicy Bypass -STA -WindowStyle Hidden -File `"$PSCommandPath`""
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
    foreach ($n in 'Dash', 'Tweaks', 'Services', 'Startup', 'Cleanup', 'Debloat', 'Network', 'Features', 'Tools', 'Rice') {
        $UI["Nav$n"].IsChecked = $true
        Wait-UI 4
        Save-Shot $n.ToLower()
    }
    Write-Host 'screenshots done'
    $win.Close()
    exit
}

[void]$win.ShowDialog()
