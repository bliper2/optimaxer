# Optimaxer look: themes, wallpaper effects, tiling-style layout, animations, settings. Needs $win and $UI from App.ps1.

function Add-Alpha { param([string]$Hex, [string]$Aa) return '#' + $Aa + $Hex.TrimStart('#') }

function New-Theme {
    param($Label, $Wall1, $Wall2, $Tile, $Surface, $Hi, $Line, $Text, $Muted, $A1, $A2, $Good, $Warn, $Bad, $On)
    @{
        Label = $Label; Wall1 = "#$Wall1"; Wall2 = "#$Wall2"
        TileBg = Add-Alpha $Tile 'D2'; Surface = Add-Alpha $Surface '99'; SurfaceHi = Add-Alpha $Hi 'CC'; Line = Add-Alpha $Line '77'
        Text = "#$Text"; Muted = "#$Muted"; Accent = "#$A1"; Accent2 = "#$A2"; Good = "#$Good"; Warn = "#$Warn"; Bad = "#$Bad"; OnAccent = "#$On"
    }
}

$global:OptiThemes = [ordered]@{
    catppuccin = New-Theme 'catppuccin mocha' '11111b' '1e1e2e' '181825' '313244' '45475a' '585b70' 'cdd6f4' '7f849c' 'cba6f7' '89b4fa' 'a6e3a1' 'f9e2af' 'f38ba8' '11111b'
    tokyonight = New-Theme 'tokyo night'      '16161e' '1a1b26' '1a1b26' '24283b' '2f334d' '3b4261' 'c0caf5' '565f89' '7aa2f7' 'bb9af7' '9ece6a' 'e0af68' 'f7768e' '16161e'
    gruvbox    = New-Theme 'gruvbox dark'     '1d2021' '282828' '282828' '3c3836' '504945' '665c54' 'ebdbb2' '928374' 'fe8019' 'fabd2f' 'b8bb26' 'fabd2f' 'fb4934' '1d2021'
    rosepine   = New-Theme 'rose pine'        '110f1b' '191724' '191724' '1f1d2e' '26233a' '403d52' 'e0def4' '6e6a86' 'ebbcba' 'c4a7e7' '9ccfd8' 'f6c177' 'eb6f92' '191724'
    nord       = New-Theme 'nord'             '242933' '2e3440' '2e3440' '3b4252' '434c5e' '4c566a' 'eceff4' '7b88a1' '88c0d0' '81a1c1' 'a3be8c' 'ebcb8b' 'bf616a' '2e3440'
    dracula    = New-Theme 'dracula'          '1e1f29' '282a36' '282a36' '343746' '44475a' '6272a4' 'f8f8f2' '6272a4' 'bd93f9' 'ff79c6' '50fa7b' 'f1fa8c' 'ff5555' '282a36'
    everforest = New-Theme 'everforest'       '232a2e' '2d353b' '2d353b' '343f44' '3d484d' '475258' 'd3c6aa' '859289' 'a7c080' '83c092' 'a7c080' 'dbbc7f' 'e67e80' '2d353b'
    hyprland   = New-Theme 'hyprland default' '0a0a12' '11111b' '11111b' '1b1b2b' '27273d' '3a3a55' 'e6e6f0' '777790' '33ccff' '00ff99' '00ff99' 'ffd166' 'ff5c5c' '0a0a12'
}

$global:OptiFxModes = [ordered]@{ none = 'none'; aurora = 'aurora'; stars = 'starfield'; rain = 'rain'; embers = 'embers' }

# hyprland-style "rices": theme + wallpaper effect + gaps + rounding + border
$global:OptiRices = [ordered]@{
    'mauve night'  = @{ Theme = 'catppuccin'; Fx = 'aurora'; GapIn = 6;  GapOut = 12; Round = 12; Border = 2 }
    'neon city'    = @{ Theme = 'tokyonight'; Fx = 'rain';   GapIn = 4;  GapOut = 10; Round = 8;  Border = 2 }
    'retro warm'   = @{ Theme = 'gruvbox';    Fx = 'embers'; GapIn = 3;  GapOut = 6;  Round = 4;  Border = 3 }
    'tty minimal'  = @{ Theme = 'nord';       Fx = 'none';   GapIn = 0;  GapOut = 0;  Round = 0;  Border = 1 }
    'forest'       = @{ Theme = 'everforest'; Fx = 'stars';  GapIn = 8;  GapOut = 14; Round = 14; Border = 2 }
    'cyber'        = @{ Theme = 'hyprland';   Fx = 'rain';   GapIn = 5;  GapOut = 8;  Round = 10; Border = 2 }
}

$global:Cfg = @{ Theme = 'catppuccin'; Fx = 'aurora'; GapIn = 6; GapOut = 12; Round = 12; Border = 2; Anim = $true; Speed = 1.0 }
$global:CfgPath = Join-Path $global:OptiData 'settings.json'

function Import-Cfg {
    if (-not (Test-Path -LiteralPath $global:CfgPath)) { return }
    try {
        $o = Get-Content -LiteralPath $global:CfgPath -Raw | ConvertFrom-Json
        foreach ($p in $o.PSObject.Properties) { if ($global:Cfg.ContainsKey($p.Name)) { $global:Cfg[$p.Name] = $p.Value } }
        if (-not $global:OptiThemes.Contains($global:Cfg.Theme)) { $global:Cfg.Theme = 'catppuccin' }
        if (-not $global:OptiFxModes.Contains($global:Cfg.Fx)) { $global:Cfg.Fx = 'aurora' }
    } catch {}
}
function Save-Cfg { try { $global:Cfg | ConvertTo-Json | Set-Content -LiteralPath $global:CfgPath -Encoding UTF8 } catch {} }

# ---------------------------------------------------------------- animation helpers
function Get-Dur { param([double]$Ms) return [TimeSpan]::FromMilliseconds($Ms / [math]::Max(0.2, [double]$global:Cfg.Speed)) }

function Set-Res {
    # bind a property to a theme resource so it follows theme changes
    param($Element, $Property, [string]$Key)
    $Element.SetResourceReference($Property, $Key)
}

function Swap-Brush {
    # theme brushes are frozen once a Style seals them, so replace the resource with a fresh brush and ease it in
    param([string]$Key, [Windows.Media.Color]$To)
    $old = $win.Resources[$Key]
    $nb = New-Object Windows.Media.SolidColorBrush $To
    $win.Resources[$Key] = [Windows.Media.Brush]$nb
    if ($global:Cfg.Anim -and $old.Color -ne $To) {
        try {
            $a = New-Object Windows.Media.Animation.ColorAnimation($old.Color, $To, (Get-Dur 380))
            $a.FillBehavior = 'Stop'
            $nb.BeginAnimation([Windows.Media.SolidColorBrush]::ColorProperty, $a)
        } catch {}
    }
}

function Swap-Gradient {
    param([string]$Key, [Windows.Media.Color[]]$To)
    $nb = $win.Resources[$Key].CloneCurrentValue()
    for ($i = 0; $i -lt $To.Count; $i++) {
        $stop = $nb.GradientStops[$i]
        $from = $stop.Color
        $stop.Color = $To[$i]
        if ($global:Cfg.Anim -and $from -ne $To[$i]) {
            try {
                $a = New-Object Windows.Media.Animation.ColorAnimation($from, $To[$i], (Get-Dur 380))
                $a.FillBehavior = 'Stop'
                $stop.BeginAnimation([Windows.Media.GradientStop]::ColorProperty, $a)
            } catch {}
        }
    }
    $win.Resources[$Key] = [Windows.Media.Brush]$nb
}

function Set-Bar {
    param($Bar, [double]$Value)
    $old = $Bar.Value
    $Bar.Value = $Value
    if ($global:Cfg.Anim -and $old -ne $Value) {
        $a = New-Object Windows.Media.Animation.DoubleAnimation($old, $Value, (Get-Dur 450))
        $a.FillBehavior = 'Stop'
        $a.EasingFunction = New-Object Windows.Media.Animation.CubicEase
        $Bar.BeginAnimation([Windows.Controls.Primitives.RangeBase]::ValueProperty, $a)
    }
}

function Start-PageIn {
    param($Page)
    if (-not $global:Cfg.Anim) { $Page.Opacity = 1; $Page.RenderTransform = $null; return }
    $tt = New-Object Windows.Media.TranslateTransform(0, 16)
    $Page.RenderTransform = $tt
    $ease = New-Object Windows.Media.Animation.CubicEase
    $ease.EasingMode = 'EaseOut'
    $fade = New-Object Windows.Media.Animation.DoubleAnimation(0, 1, (Get-Dur 220))
    $slide = New-Object Windows.Media.Animation.DoubleAnimation(16, 0, (Get-Dur 260))
    $slide.EasingFunction = $ease
    $Page.BeginAnimation([Windows.UIElement]::OpacityProperty, $fade)
    $tt.BeginAnimation([Windows.Media.TranslateTransform]::YProperty, $slide)
}

function Start-BorderSpin {
    $rt = $win.Resources['ActiveBorder'].RelativeTransform
    if ($global:Cfg.Anim) {
        $a = New-Object Windows.Media.Animation.DoubleAnimation(0, 360, [TimeSpan]::FromSeconds(9 / [math]::Max(0.2, [double]$global:Cfg.Speed)))
        $a.RepeatBehavior = [Windows.Media.Animation.RepeatBehavior]::Forever
        $rt.BeginAnimation([Windows.Media.RotateTransform]::AngleProperty, $a)
    } else { $rt.BeginAnimation([Windows.Media.RotateTransform]::AngleProperty, $null) }
}

# ---------------------------------------------------------------- theme / fx / layout
function ConvertTo-Color { param([string]$Hex) return [Windows.Media.ColorConverter]::ConvertFromString($Hex) }

function Set-Theme {
    param([string]$Key, [bool]$Animate = $true)
    $t = $global:OptiThemes[$Key]
    $global:Cfg.Theme = $Key
    $keep = $global:Cfg.Anim; if (-not $Animate) { $global:Cfg.Anim = $false }
    foreach ($k in 'TileBg', 'Surface', 'SurfaceHi', 'Line', 'Text', 'Muted', 'Accent', 'Accent2', 'Good', 'Warn', 'Bad', 'OnAccent') { Swap-Brush $k (ConvertTo-Color $t[$k]) }
    $a1 = ConvertTo-Color $t.Accent; $a2 = ConvertTo-Color $t.Accent2
    Swap-Gradient 'WallGrad' @((ConvertTo-Color $t.Wall1), (ConvertTo-Color $t.Wall2))
    Swap-Gradient 'AccentGrad' @($a1, $a2)
    Swap-Gradient 'ActiveBorder' @($a1, $a2, $a1)
    $global:Cfg.Anim = $keep
    $global:Fx.SetColors($a1, $a2)
    Start-BorderSpin
    Set-ActiveTile $global:ActiveTile
    Save-Cfg
    Update-Rice
}

function Set-Fx {
    param([string]$Key)
    $global:Cfg.Fx = $Key
    $global:Fx.SetMode($Key)
    Save-Cfg
    Update-Rice
}

function Set-Layout {
    $gi = [double]$global:Cfg.GapIn; $go = [double]$global:Cfg.GapOut
    $UI.Shell.Margin = [Windows.Thickness][math]::Max(0, $go - $gi / 2)
    foreach ($tile in $global:Tiles) {
        $tile.Margin = [Windows.Thickness]($gi / 2)
        $tile.CornerRadius = [Windows.CornerRadius][double]$global:Cfg.Round
        $tile.BorderThickness = [Windows.Thickness][double]$global:Cfg.Border
    }
}

function Set-ActiveTile {
    param($Tile)
    $global:ActiveTile = $Tile
    foreach ($t in $global:Tiles) { $t.SetResourceReference([Windows.Controls.Border]::BorderBrushProperty, $(if ($t -eq $Tile) { 'ActiveBorder' } else { 'Line' })) }
}

function Set-Rice {
    param([string]$Name)
    $r = $global:OptiRices[$Name]
    foreach ($k in 'GapIn', 'GapOut', 'Round', 'Border') { $global:Cfg[$k] = $r[$k] }
    Set-Layout
    Set-Fx $r.Fx
    Set-Theme $r.Theme
}

function Get-HyprConf {
    $t = $global:OptiThemes[$global:Cfg.Theme]
    $a = $t.Accent.TrimStart('#').ToLower(); $b = $t.Accent2.TrimStart('#').ToLower()
    $l = $t.Line.TrimStart('#').ToLower().Substring(2)
    @"
# theme: $($t.Label)   wallpaper: $($global:OptiFxModes[$global:Cfg.Fx])
general {
    gaps_in = $($global:Cfg.GapIn)
    gaps_out = $($global:Cfg.GapOut)
    border_size = $($global:Cfg.Border)
    col.active_border = rgba(${a}ff) rgba(${b}ff) 45deg
    col.inactive_border = rgba(${l}aa)
}
decoration {
    rounding = $($global:Cfg.Round)
}
animations {
    enabled = $(if ($global:Cfg.Anim) { 'yes' } else { 'no' })
    animation = borderangle, 1, $([int](30 / [math]::Max(0.2, [double]$global:Cfg.Speed))), default, loop
}
"@
}

# ---------------------------------------------------------------- appearance page
$global:RiceButtons = @{ theme = @{}; fx = @{}; anim = @{} }
$global:LayoutValues = @{}

function New-RiceButton {
    param([string]$Text, $Swatches = @())
    $b = New-Object Windows.Controls.Button
    $b.Margin = '0,0,6,6'; $b.Padding = '10,6'
    $sp = New-Object Windows.Controls.StackPanel -Property @{ Orientation = 'Horizontal' }
    foreach ($c in $Swatches) {
        $r = New-Object Windows.Shapes.Rectangle -Property @{ Width = 9; Height = 14; Margin = '0,0,2,0'; RadiusX = 2; RadiusY = 2; Fill = (New-Object Windows.Media.SolidColorBrush (ConvertTo-Color $c)) }
        [void]$sp.Children.Add($r)
    }
    $lead = if ($Swatches.Count) { '6,0,0,0' } else { '0' }
    $tb = New-Object Windows.Controls.TextBlock -Property @{ Text = $Text; Margin = $lead }
    Set-Res $tb ([Windows.Controls.TextBlock]::ForegroundProperty) 'Text'
    [void]$sp.Children.Add($tb)
    $b.Content = $sp
    return $b
}

function Update-Rice {
    $bp = [Windows.Controls.Control]::BorderBrushProperty
    foreach ($k in $global:RiceButtons.theme.Keys) { Set-Res $global:RiceButtons.theme[$k] $bp $(if ($k -eq $global:Cfg.Theme) { 'Accent' } else { 'Line' }) }
    foreach ($k in $global:RiceButtons.fx.Keys)    { Set-Res $global:RiceButtons.fx[$k]    $bp $(if ($k -eq $global:Cfg.Fx) { 'Accent' } else { 'Line' }) }
    $speedKey = if ($global:Cfg.Speed -le 0.7) { 'slow' } elseif ($global:Cfg.Speed -ge 1.5) { 'fast' } else { 'normal' }
    foreach ($k in $global:RiceButtons.anim.Keys) {
        $on = if ($k -eq 'on') { $global:Cfg.Anim } elseif ($k -eq 'off') { -not $global:Cfg.Anim } else { $k -eq $speedKey }
        Set-Res $global:RiceButtons.anim[$k] $bp $(if ($on) { 'Accent' } else { 'Line' })
    }
    foreach ($k in $global:LayoutValues.Keys) { $global:LayoutValues[$k].Text = [string]$global:Cfg[$k] }
    $UI.ConfPreview.Text = Get-HyprConf
}

function Initialize-Rice {
    foreach ($n in $global:OptiRices.Keys) {
        $r = $global:OptiRices[$n]; $t = $global:OptiThemes[$r.Theme]
        $b = New-RiceButton $n @($t.Accent, $t.Accent2, $t.Wall2)
        $b.Tag = $n
        $b.Add_Click({ param($s, $e) Set-Rice $s.Tag })
        [void]$UI.RicePanel.Children.Add($b)
    }
    foreach ($k in $global:OptiThemes.Keys) {
        $t = $global:OptiThemes[$k]
        $b = New-RiceButton $t.Label @($t.Accent, $t.Accent2, $t.Wall2)
        $b.Tag = $k
        $b.Add_Click({ param($s, $e) Set-Theme $s.Tag })
        $global:RiceButtons.theme[$k] = $b
        [void]$UI.ThemePanel.Children.Add($b)
    }
    foreach ($k in $global:OptiFxModes.Keys) {
        $b = New-RiceButton $global:OptiFxModes[$k]
        $b.Tag = $k
        $b.Add_Click({ param($s, $e) Set-Fx $s.Tag })
        $global:RiceButtons.fx[$k] = $b
        [void]$UI.FxPanel.Children.Add($b)
    }
    $rows = @(
        @{ Key = 'GapIn';  Label = 'gaps_in';     Min = 0; Max = 20; Step = 2 },
        @{ Key = 'GapOut'; Label = 'gaps_out';    Min = 0; Max = 32; Step = 2 },
        @{ Key = 'Round';  Label = 'rounding';    Min = 0; Max = 24; Step = 2 },
        @{ Key = 'Border'; Label = 'border_size'; Min = 1; Max = 4;  Step = 1 }
    )
    foreach ($row in $rows) {
        $dock = New-Object Windows.Controls.StackPanel -Property @{ Orientation = 'Horizontal'; Margin = '0,0,0,6' }
        $lbl = New-Object Windows.Controls.TextBlock -Property @{ Text = $row.Label; Width = 110; VerticalAlignment = 'Center' }
        Set-Res $lbl ([Windows.Controls.TextBlock]::ForegroundProperty) 'Muted'
        [void]$dock.Children.Add($lbl)
        foreach ($d in -1, 1) {
            if ($d -eq 1) {
                $v = New-Object Windows.Controls.TextBlock -Property @{ Width = 36; TextAlignment = 'Center'; VerticalAlignment = 'Center' }
                Set-Res $v ([Windows.Controls.TextBlock]::ForegroundProperty) 'Accent'
                $global:LayoutValues[$row.Key] = $v
                [void]$dock.Children.Add($v)
            }
            $b = New-Object Windows.Controls.Button -Property @{ Content = $(if ($d -lt 0) { '-' } else { '+' }); Width = 28; Padding = '0,3'; Tag = @{ Row = $row; Dir = $d } }
            $b.Add_Click({
                param($s, $e)
                $r = $s.Tag.Row
                $n = [int]$global:Cfg[$r.Key] + $s.Tag.Dir * $r.Step
                $global:Cfg[$r.Key] = [math]::Min($r.Max, [math]::Max($r.Min, $n))
                Set-Layout; Save-Cfg; Update-Rice
            })
            [void]$dock.Children.Add($b)
        }
        [void]$UI.LayoutPanel.Children.Add($dock)
    }
    foreach ($k in 'on', 'off', 'slow', 'normal', 'fast') {
        $b = New-RiceButton $k
        $b.Tag = $k
        $b.Add_Click({
            param($s, $e)
            switch ($s.Tag) {
                'on' { $global:Cfg.Anim = $true } 'off' { $global:Cfg.Anim = $false }
                'slow' { $global:Cfg.Speed = 0.6 } 'normal' { $global:Cfg.Speed = 1.0 } 'fast' { $global:Cfg.Speed = 1.8 }
            }
            Start-BorderSpin; Save-Cfg; Update-Rice
        })
        $global:RiceButtons.anim[$k] = $b
        [void]$UI.AnimPanel.Children.Add($b)
    }
    $UI.BtnConfCopy.Add_Click({ [Windows.Clipboard]::SetText((Get-HyprConf)) })
}

function Initialize-Look {
    Import-Cfg
    $global:Fx = New-Object OptiFx $UI.FxCanvas
    $global:Tiles = @($UI.TileBar, $UI.TileMain, $UI.TileInfo, $UI.TileLog)
    foreach ($tile in $global:Tiles) { $tile.Add_MouseEnter({ param($s, $e) Set-ActiveTile $s }) }
    Initialize-Rice
    Set-Layout
    $global:ActiveTile = $UI.TileMain
    Set-Fx $global:Cfg.Fx
    Set-Theme $global:Cfg.Theme $false
}

function Start-WindowIn {
    if (-not $global:Cfg.Anim) { return }
    $UI.Shell.RenderTransformOrigin = New-Object Windows.Point(0.5, 0.5)
    $st = New-Object Windows.Media.ScaleTransform(0.97, 0.97)
    $UI.Shell.RenderTransform = $st
    $ease = New-Object Windows.Media.Animation.CubicEase
    $ease.EasingMode = 'EaseOut'
    $sa = New-Object Windows.Media.Animation.DoubleAnimation(0.97, 1, (Get-Dur 420))
    $sa.EasingFunction = $ease
    $st.BeginAnimation([Windows.Media.ScaleTransform]::ScaleXProperty, $sa)
    $st.BeginAnimation([Windows.Media.ScaleTransform]::ScaleYProperty, $sa)
    $UI.Shell.BeginAnimation([Windows.UIElement]::OpacityProperty, (New-Object Windows.Media.Animation.DoubleAnimation(0, 1, (Get-Dur 380))))
}
