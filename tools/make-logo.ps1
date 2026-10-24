# Builds assets\optimaxer.ico from assets\logo.png (PNG frames, 16-256 px).
# The 16 px frame is assets\logo-small.png, the hand-drawn pixel mark.
# Also copies the logo and the ico into docs\ for the site header and favicon.
Add-Type -AssemblyName System.Drawing
$root = Split-Path $PSScriptRoot -Parent
$logoPath = Join-Path $root 'assets\logo.png'
$smallPath = Join-Path $root 'assets\logo-small.png'
$src = [System.Drawing.Bitmap]::FromFile((Resolve-Path -LiteralPath $logoPath))

function Render([System.Drawing.Image]$from, [int]$size) {
    if ($from.Width -eq $size -and $from.Height -eq $size) {
        $ms = New-Object IO.MemoryStream
        $from.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
        return , $ms.ToArray()
    }
    $bmp = New-Object System.Drawing.Bitmap $size, $size, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
    $g.Clear([System.Drawing.Color]::Transparent)
    $g.DrawImage($from, 0, 0, $size, $size)
    $g.Dispose()
    $ms = New-Object IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    return , $ms.ToArray()
}

$small = $null
if (Test-Path -LiteralPath $smallPath) {
    $small = [System.Drawing.Bitmap]::FromFile((Resolve-Path -LiteralPath $smallPath))
}
$sizes = 16, 24, 32, 48, 64, 128, 256
$frames = foreach ($s in $sizes) {
    $from = if ($s -eq 16 -and $small) { $small } else { $src }
    , @{ Size = $s; Data = (Render $from $s) }
}

$ms = New-Object IO.MemoryStream
$w = New-Object IO.BinaryWriter $ms
$w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$frames.Count)
$offset = 6 + 16 * $frames.Count
foreach ($f in $frames) {
    $dim = if ($f.Size -ge 256) { 0 } else { $f.Size }
    $w.Write([byte]$dim); $w.Write([byte]$dim); $w.Write([byte]0); $w.Write([byte]0)
    $w.Write([uint16]1); $w.Write([uint16]32); $w.Write([uint32]$f.Data.Length); $w.Write([uint32]$offset)
    $offset += $f.Data.Length
}
foreach ($f in $frames) { $w.Write($f.Data) }
$ico = Join-Path $root 'assets\optimaxer.ico'
[IO.File]::WriteAllBytes($ico, $ms.ToArray())
Copy-Item -LiteralPath $logoPath -Destination (Join-Path $root 'docs\logo.png') -Force
Copy-Item -LiteralPath $ico -Destination (Join-Path $root 'docs\favicon.ico') -Force
$src.Dispose()
if ($small) { $small.Dispose() }
"wrote logo.png, optimaxer.ico and docs copies ($($frames.Count) sizes)"
