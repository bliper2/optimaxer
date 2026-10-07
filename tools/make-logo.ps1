# Renders assets\logo.xaml into assets\logo.png (256 px) and assets\optimaxer.ico (16-256 px, PNG frames).
Add-Type -AssemblyName PresentationFramework, PresentationCore, WindowsBase, System.Xaml
$root = Split-Path $PSScriptRoot -Parent
$xaml = Get-Content -LiteralPath (Join-Path $root 'assets\logo.xaml') -Raw

function Render([int]$size) {
    $canvas = [Windows.Markup.XamlReader]::Parse($xaml)
    $vb = New-Object Windows.Controls.Viewbox -Property @{ Width = $size; Height = $size; Stretch = 'Uniform'; Child = $canvas }
    $vb.Measure((New-Object Windows.Size($size, $size))); $vb.Arrange((New-Object Windows.Rect(0, 0, $size, $size))); $vb.UpdateLayout()
    $bmp = New-Object Windows.Media.Imaging.RenderTargetBitmap($size, $size, 96, 96, [Windows.Media.PixelFormats]::Pbgra32)
    $bmp.Render($vb)
    $enc = New-Object Windows.Media.Imaging.PngBitmapEncoder
    $enc.Frames.Add([Windows.Media.Imaging.BitmapFrame]::Create($bmp))
    $ms = New-Object IO.MemoryStream; $enc.Save($ms); return , $ms.ToArray()
}

$sizes = 16, 24, 32, 48, 64, 128, 256
$frames = foreach ($s in $sizes) { , @{ Size = $s; Data = (Render $s) } }
[IO.File]::WriteAllBytes((Join-Path $root 'assets\logo.png'), ($frames | Where-Object { $_.Size -eq 256 }).Data)

$ms = New-Object IO.MemoryStream; $w = New-Object IO.BinaryWriter $ms
$w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$frames.Count)
$offset = 6 + 16 * $frames.Count
foreach ($f in $frames) {
    $dim = if ($f.Size -ge 256) { 0 } else { $f.Size }
    $w.Write([byte]$dim); $w.Write([byte]$dim); $w.Write([byte]0); $w.Write([byte]0)
    $w.Write([uint16]1); $w.Write([uint16]32); $w.Write([uint32]$f.Data.Length); $w.Write([uint32]$offset)
    $offset += $f.Data.Length
}
foreach ($f in $frames) { $w.Write($f.Data) }
[IO.File]::WriteAllBytes((Join-Path $root 'assets\optimaxer.ico'), $ms.ToArray())
"wrote logo.png and optimaxer.ico ($($frames.Count) sizes)"
