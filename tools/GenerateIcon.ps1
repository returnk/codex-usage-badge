$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$assets = Join-Path (Split-Path $PSScriptRoot -Parent) 'src\CodexBadge\Assets'
New-Item -ItemType Directory -Path $assets -Force | Out-Null
$iconOutput = Join-Path $assets 'CodexBadge.ico'
$pngOutput = Join-Path $assets 'CodexBadge.png'
$sizes = @(16, 20, 24, 32, 40, 48, 64, 256)
$mainBlue = [System.Drawing.Color]::FromArgb(58, 131, 247)
$accentBlue = [System.Drawing.Color]::FromArgb(169, 201, 253)

function New-QuotaRingBitmap([int]$size) {
    $renderScale = 4
    $canvasSize = $size * $renderScale
    $bitmap = [System.Drawing.Bitmap]::new($canvasSize, $canvasSize, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.Clear([System.Drawing.Color]::Transparent)
    $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality

    $bounds = [System.Drawing.RectangleF]::new(
        [single]($size * 0.21 * $renderScale),
        [single]($size * 0.21 * $renderScale),
        [single]($size * 0.58 * $renderScale),
        [single]($size * 0.58 * $renderScale))
    $strokeWidth = [single]($size * 0.15 * $renderScale)
    $mainPen = [System.Drawing.Pen]::new($mainBlue, $strokeWidth)
    $accentPen = [System.Drawing.Pen]::new($accentBlue, $strokeWidth)
    $mainPen.StartCap = $mainPen.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
    $accentPen.StartCap = $accentPen.EndCap = [System.Drawing.Drawing2D.LineCap]::Round

    $graphics.DrawArc($mainPen, $bounds, 0, 280)
    $graphics.DrawArc($accentPen, $bounds, 302, 32)

    $mainPen.Dispose()
    $accentPen.Dispose()
    $graphics.Dispose()

    $result = [System.Drawing.Bitmap]::new($size, $size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $resultGraphics = [System.Drawing.Graphics]::FromImage($result)
    $resultGraphics.Clear([System.Drawing.Color]::Transparent)
    $resultGraphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $resultGraphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $resultGraphics.DrawImage($bitmap, 0, 0, $size, $size)
    $resultGraphics.Dispose()
    $bitmap.Dispose()
    return $result
}

$frames = [System.Collections.Generic.List[byte[]]]::new()
foreach ($size in $sizes) {
    $bitmap = New-QuotaRingBitmap $size
    $memory = [System.IO.MemoryStream]::new()
    $bitmap.Save($memory, [System.Drawing.Imaging.ImageFormat]::Png)
    $frames.Add($memory.ToArray())
    $memory.Dispose()
    $bitmap.Dispose()
}

$preview = New-QuotaRingBitmap 512
$preview.Save($pngOutput, [System.Drawing.Imaging.ImageFormat]::Png)
$preview.Dispose()

$stream = [System.IO.File]::Open($iconOutput, [System.IO.FileMode]::Create)
$writer = [System.IO.BinaryWriter]::new($stream)
try {
    $writer.Write([uint16]0)
    $writer.Write([uint16]1)
    $writer.Write([uint16]$frames.Count)
    $offset = 6 + 16 * $frames.Count
    for ($index = 0; $index -lt $frames.Count; $index++) {
        $size = $sizes[$index]
        $writer.Write([byte]$(if ($size -eq 256) { 0 } else { $size }))
        $writer.Write([byte]$(if ($size -eq 256) { 0 } else { $size }))
        $writer.Write([byte]0)
        $writer.Write([byte]0)
        $writer.Write([uint16]1)
        $writer.Write([uint16]32)
        $writer.Write([uint32]$frames[$index].Length)
        $writer.Write([uint32]$offset)
        $offset += $frames[$index].Length
    }
    foreach ($frame in $frames) { $writer.Write($frame) }
}
finally {
    $writer.Dispose()
    $stream.Dispose()
}

Write-Host "Generated: $iconOutput"
Write-Host "Preview: $pngOutput"
