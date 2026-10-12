# Optimaxer look: palettes, accent colours, settings file, small animation helpers. Needs $win and $UI from App.ps1.

$global:OptiModes = [ordered]@{
    dark  = @{ Label = 'Dark';  Bg = '1B1B1D'; Sidebar = '151517'; Card = '232326'; CardHi = '2B2B2F'; Line = '34343A'; Text = 'ECECEF'; Muted = '9B9BA4'; Good = '5FB878'; Warn = 'E0A93B'; Bad = 'E5645F' }
    light = @{ Label = 'Light'; Bg = 'F5F5F7'; Sidebar = 'EDEDF0'; Card = 'FFFFFF'; CardHi = 'F0F0F3'; Line = 'DADADF'; Text = '1C1C1F'; Muted = '62626B'; Good = '2E8B4F'; Warn = '9A6700'; Bad = 'C4352F' }
}

# Each accent has a tone tuned for dark surfaces and one for light surfaces.
$global:OptiAccents = [ordered]@{
    teal   = @{ Label = 'Teal';   Dark = '2FA79A'; Light = '0F7B70' }
    blue   = @{ Label = 'Blue';   Dark = '4C8BF5'; Light = '1F63D6' }
    violet = @{ Label = 'Violet'; Dark = '9683F2'; Light = '6B4FD6' }
    rose   = @{ Label = 'Rose';   Dark = 'E5648B'; Light = 'C23A66' }
    amber  = @{ Label = 'Amber';  Dark = 'E0A93B'; Light = 'A86A00' }
    green  = @{ Label = 'Green';  Dark = '5FB878'; Light = '2E8B4F' }
}

$global:Cfg = @{ Mode = 'system'; Accent = 'teal'; Anim = $true; AutoUpdate = $true; UiVersion = 4 }
$global:CfgPath = Join-Path $global:OptiData 'settings.json'

function Import-Cfg {
    if (-not (Test-Path -LiteralPath $global:CfgPath)) { return }
    try {
        $o = Get-Content -LiteralPath $global:CfgPath -Raw | ConvertFrom-Json
        foreach ($k in 'Anim', 'AutoUpdate') { if ($null -ne $o.$k) { $global:Cfg[$k] = [bool]$o.$k } }
        if ($o.UiVersion -eq 4) {
            if ($global:OptiModes.Contains([string]$o.Mode) -or $o.Mode -eq 'system') { $global:Cfg.Mode = [string]$o.Mode }
            if ($global:OptiAccents.Contains([string]$o.Accent)) { $global:Cfg.Accent = [string]$o.Accent }
        } elseif ($o.Theme -eq 'light') { $global:Cfg.Mode = 'light' }
        elseif ($o.Theme) { $global:Cfg.Mode = 'dark' }
    } catch {}
}
function Save-Cfg { try { $global:Cfg | ConvertTo-Json | Set-Content -LiteralPath $global:CfgPath -Encoding UTF8 } catch {} }

# ---------------------------------------------------------------- helpers
function Get-Dur { param([double]$Ms) return [TimeSpan]::FromMilliseconds($Ms) }

function Set-Res {
    # bind a property to a palette brush so it follows theme changes
    param($Element, $Property, [string]$Key)
    $Element.SetResourceReference($Property, $Key)
}

function ConvertTo-Color { param([string]$Hex) return [Windows.Media.ColorConverter]::ConvertFromString($(if ($Hex.StartsWith('#')) { $Hex } else { "#$Hex" })) }

function Mix-Color {
    # blend A toward B by T (0 = A, 1 = B)
    param([Windows.Media.Color]$A, [Windows.Media.Color]$B, [double]$T)
    [Windows.Media.Color]::FromRgb(
        [byte][math]::Round($A.R + ($B.R - $A.R) * $T),
        [byte][math]::Round($A.G + ($B.G - $A.G) * $T),
        [byte][math]::Round($A.B + ($B.B - $A.B) * $T))
}

function Get-Luminance {
    param([Windows.Media.Color]$C)
    $f = { param($v) $x = $v / 255.0; if ($x -le 0.03928) { $x / 12.92 } else { [math]::Pow(($x + 0.055) / 1.055, 2.4) } }
    0.2126 * (& $f $C.R) + 0.7152 * (& $f $C.G) + 0.0722 * (& $f $C.B)
}

function Get-EffectiveMode {
    if ($global:Cfg.Mode -in 'dark', 'light') { return $global:Cfg.Mode }
    try {
        $v = (Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize' -Name AppsUseLightTheme -ErrorAction Stop).AppsUseLightTheme
        if ($v -eq 1) { return 'light' }
    } catch {}
    return 'dark'
}

function Swap-Brush {
    # Palette brushes get frozen once a Style seals them, so replace the resource with a fresh brush and ease it in.
    # The animation must start BEFORE the brush goes into the dictionary, which would otherwise freeze it.
    param([string]$Key, [Windows.Media.Color]$To)
    $old = $win.Resources[$Key]
    $nb = New-Object Windows.Media.SolidColorBrush $To
    if ($global:Cfg.Anim -and $old.Color -ne $To) {
        try {
            $a = New-Object Windows.Media.Animation.ColorAnimation($old.Color, $To, (Get-Dur 220))
            $a.FillBehavior = 'Stop'
            $nb.BeginAnimation([Windows.Media.SolidColorBrush]::ColorProperty, $a)
        } catch {}
    }
    $win.Resources[$Key] = [Windows.Media.Brush]$nb
}

function Set-Bar {
    param($Bar, [double]$Value)
    $old = $Bar.Value
    $Bar.Value = $Value
    if ($global:Cfg.Anim -and $old -ne $Value) {
        $a = New-Object Windows.Media.Animation.DoubleAnimation($old, $Value, (Get-Dur 350))
        $a.FillBehavior = 'Stop'
        $a.EasingFunction = New-Object Windows.Media.Animation.CubicEase
        $Bar.BeginAnimation([Windows.Controls.Primitives.RangeBase]::ValueProperty, $a)
    }
}

function Start-PageIn {
    param($Page)
    if (-not $global:Cfg.Anim) { $Page.Opacity = 1; $Page.RenderTransform = $null; return }
    $tt = New-Object Windows.Media.TranslateTransform(0, 8)
    $Page.RenderTransform = $tt
    $ease = New-Object Windows.Media.Animation.CubicEase
    $ease.EasingMode = 'EaseOut'
    $fade = New-Object Windows.Media.Animation.DoubleAnimation(0, 1, (Get-Dur 160))
    $slide = New-Object Windows.Media.Animation.DoubleAnimation(8, 0, (Get-Dur 200))
    $slide.EasingFunction = $ease
    $Page.BeginAnimation([Windows.UIElement]::OpacityProperty, $fade)
    $tt.BeginAnimation([Windows.Media.TranslateTransform]::YProperty, $slide)
}

function Start-WindowIn {
    if (-not $global:Cfg.Anim) { return }
    $UI.Shell.BeginAnimation([Windows.UIElement]::OpacityProperty, (New-Object Windows.Media.Animation.DoubleAnimation(0, 1, (Get-Dur 200))))
}

# ---------------------------------------------------------------- applying the palette
function Set-Appearance {
    param([bool]$Animate = $true)
    $mode = $global:OptiModes[(Get-EffectiveMode)]
    $isLight = (Get-EffectiveMode) -eq 'light'
    $acc = $global:OptiAccents[$global:Cfg.Accent]
    $accent = ConvertTo-Color $(if ($isLight) { $acc.Light } else { $acc.Dark })
    $bg = ConvertTo-Color $mode.Bg
    $hi = if ($isLight) { Mix-Color $accent ([Windows.Media.Colors]::Black) 0.14 } else { Mix-Color $accent ([Windows.Media.Colors]::White) 0.16 }
    $on = if ((Get-Luminance $accent) -gt 0.4) { ConvertTo-Color '111111' } else { ConvertTo-Color 'FFFFFF' }
    $colors = @{
        Bg = $bg; Sidebar = (ConvertTo-Color $mode.Sidebar); Card = (ConvertTo-Color $mode.Card); CardHi = (ConvertTo-Color $mode.CardHi)
        Line = (ConvertTo-Color $mode.Line); Text = (ConvertTo-Color $mode.Text); Muted = (ConvertTo-Color $mode.Muted)
        Good = (ConvertTo-Color $mode.Good); Warn = (ConvertTo-Color $mode.Warn); Bad = (ConvertTo-Color $mode.Bad)
        Accent = $accent; AccentHi = $hi; OnAccent = $on
        Selected = (Mix-Color $bg $accent $(if ($isLight) { 0.12 } else { 0.18 }))
    }
    $keep = $global:Cfg.Anim; if (-not $Animate) { $global:Cfg.Anim = $false }
    foreach ($k in $colors.Keys) { Swap-Brush $k $colors[$k] }
    $global:Cfg.Anim = $keep
    Update-Appearance
}

# ---------------------------------------------------------------- settings page
$global:ChipSets = @{ mode = @{}; accent = @{}; anim = @{}; upd = @{} }

function New-Chip {
    param([string]$Text, [string]$Swatch = '')
    $b = New-Object Windows.Controls.Button
    $b.Style = $win.FindResource('Chip')
    $b.Margin = '0,0,8,0'
    $sp = New-Object Windows.Controls.StackPanel -Property @{ Orientation = 'Horizontal' }
    if ($Swatch) {
        $dot = New-Object Windows.Shapes.Ellipse -Property @{ Width = 10; Height = 10; Margin = '0,0,8,0'; VerticalAlignment = 'Center'; Fill = (New-Object Windows.Media.SolidColorBrush (ConvertTo-Color $Swatch)) }
        [void]$sp.Children.Add($dot)
    }
    [void]$sp.Children.Add((New-Object Windows.Controls.TextBlock -Property @{ Text = $Text; VerticalAlignment = 'Center' }))
    $b.Content = $sp
    return $b
}

function Set-ChipState {
    param($Chip, [bool]$On)
    Set-Res $Chip ([Windows.Controls.Control]::BorderBrushProperty) $(if ($On) { 'Accent' } else { 'Line' })
    Set-Res $Chip ([Windows.Controls.Control]::BackgroundProperty) $(if ($On) { 'Selected' } else { 'Card' })
}

function Update-Appearance {
    foreach ($k in $global:ChipSets.mode.Keys)   { Set-ChipState $global:ChipSets.mode[$k]   ($k -eq $global:Cfg.Mode) }
    foreach ($k in $global:ChipSets.accent.Keys) { Set-ChipState $global:ChipSets.accent[$k] ($k -eq $global:Cfg.Accent) }
    foreach ($k in $global:ChipSets.anim.Keys)   { Set-ChipState $global:ChipSets.anim[$k]   (($k -eq 'on') -eq [bool]$global:Cfg.Anim) }
    foreach ($k in $global:ChipSets.upd.Keys)    { Set-ChipState $global:ChipSets.upd[$k]    (($k -eq 'on') -eq [bool]$global:Cfg.AutoUpdate) }
}

function Initialize-Settings {
    foreach ($m in 'system', 'light', 'dark') {
        $b = New-Chip $(if ($m -eq 'system') { 'Match Windows' } else { $global:OptiModes[$m].Label })
        $b.Tag = $m
        $b.Add_Click({ param($s, $e) $global:Cfg.Mode = $s.Tag; Save-Cfg; Set-Appearance })
        $global:ChipSets.mode[$m] = $b
        [void]$UI.ThemePanel.Children.Add($b)
    }
    foreach ($k in $global:OptiAccents.Keys) {
        $b = New-Chip $global:OptiAccents[$k].Label $global:OptiAccents[$k].Dark
        $b.Tag = $k
        $b.Add_Click({ param($s, $e) $global:Cfg.Accent = $s.Tag; Save-Cfg; Set-Appearance })
        $global:ChipSets.accent[$k] = $b
        [void]$UI.AccentPanel.Children.Add($b)
    }
    foreach ($k in 'on', 'off') {
        $b = New-Chip $(if ($k -eq 'on') { 'Animate' } else { 'Reduce motion' })
        $b.Tag = $k
        $b.Add_Click({ param($s, $e) $global:Cfg.Anim = ($s.Tag -eq 'on'); Save-Cfg; Update-Appearance })
        $global:ChipSets.anim[$k] = $b
        [void]$UI.AnimPanel.Children.Add($b)
    }
    foreach ($k in 'on', 'off') {
        $b = New-Chip $(if ($k -eq 'on') { 'Check at startup' } else { 'Never check' })
        $b.Tag = $k
        $b.Add_Click({ param($s, $e) $global:Cfg.AutoUpdate = ($s.Tag -eq 'on'); Save-Cfg; Update-Appearance })
        $global:ChipSets.upd[$k] = $b
        [void]$UI.UpdPanel.Children.Add($b)
    }
}

function Initialize-Look {
    Import-Cfg
    $win.SetResourceReference([Windows.Window]::BackgroundProperty, 'Bg')
    Initialize-Settings
    Set-Appearance $false
}
