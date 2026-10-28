# Renders the Vale logo ("Bata": terracotta tile, espresso island,
# cream dot, fixed brand colours) and writes assets\icon.ico (16-256 px) and
# assets\logo-256.png. The vector source is assets\logo.svg; keep both in sync.
# Sizes up to 32 px use hand-snapped geometry so the pill keeps crisp edges;
# larger sizes scale the 256 px design and round to whole pixels.
# Every size is drawn at 8x and reduced, which gives clean anti-aliasing.
# Requires Windows PowerShell 5.1 or later. Per-size PNGs go to -OutDir
# (default target\icon) for inspection.

param([string]$OutDir)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
public static class IconBoxFilter {
    // src: premultiplied BGRA, (size * factor) square. Returns straight BGRA,
    // size square, rows packed.
    public static byte[] Reduce(byte[] src, int stride, int size, int factor) {
        byte[] dst = new byte[size * size * 4];
        int n = factor * factor;
        for (int y = 0; y < size; y++) {
            for (int x = 0; x < size; x++) {
                long b = 0, g = 0, r = 0, a = 0;
                for (int sy = 0; sy < factor; sy++) {
                    int row = (y * factor + sy) * stride + x * factor * 4;
                    for (int sx = 0; sx < factor; sx++) {
                        int i = row + sx * 4;
                        b += src[i]; g += src[i + 1]; r += src[i + 2]; a += src[i + 3];
                    }
                }
                int o = (y * size + x) * 4;
                if (a == 0) continue;
                dst[o] = (byte)System.Math.Min(255, (b * 255 + a / 2) / a);
                dst[o + 1] = (byte)System.Math.Min(255, (g * 255 + a / 2) / a);
                dst[o + 2] = (byte)System.Math.Min(255, (r * 255 + a / 2) / a);
                dst[o + 3] = (byte)((a + n / 2) / n);
            }
        }
        return dst;
    }
}
'@

$root = Split-Path -Parent $PSScriptRoot
$assets = Join-Path $root 'assets'
if (-not $OutDir) { $OutDir = Join-Path $root 'target\icon' }
New-Item -ItemType Directory -Force -Path $assets, $OutDir | Out-Null

$sizes = 16, 20, 24, 32, 40, 48, 64, 128, 256
$supersample = 8

$tileColor = [System.Drawing.Color]::FromArgb(255, 0xE2, 0x68, 0x3C)
$pillColor = [System.Drawing.Color]::FromArgb(255, 0x17, 0x12, 0x0F)
$dotColor = [System.Drawing.Color]::FromArgb(255, 0xF4, 0xED, 0xE4)

# Squircle tile: straight edges joined by one cubic per corner. Each corner
# curve spans 70/232 of the side from the corner and its control points sit
# 0.76 of that span along the edges, which eases the curvature into the
# straight sides (logo.svg uses the same numbers).
$cornerSpan = 70 / 232
$cornerPull = 0.76

# The 256 px design, as fractions of the tile side (tile = 232 px, inset 12).
# The pill sits centred on the tile (104..152 at 256 px).
$design = @{ Inset = 12 / 256; PillW = 140 / 232; PillH = 48 / 232; PillTop = 92 / 232; Dot = 19 / 48 }

# Hand-tuned small sizes: tile inset, pill width, height and top (absolute y),
# dot diameter. The pill is vertically centred on whole pixels and the dot is
# centred on the pill's right end cap. 16 px fills the whole square so the
# terracotta tile reads on dark and light taskbars alike.
$hand = @{
    16 = @{ Inset = 0; PillW = 10; PillH = 4; PillY = 6; Dot = 2 }
    20 = @{ Inset = 1; PillW = 12; PillH = 4; PillY = 8; Dot = 2 }
    24 = @{ Inset = 1; PillW = 14; PillH = 6; PillY = 9; Dot = 2 }
    32 = @{ Inset = 1; PillW = 18; PillH = 6; PillY = 13; Dot = 2 }
}

function Round-Parity([double]$value, [int]$parity) {
    # Nearest integer whose parity matches $parity (0 even, 1 odd).
    $low = [math]::Floor($value)
    if (($low % 2) -ne $parity) { $low -= 1 }
    if (($value - $low) -le 1) { return [int]$low }
    return [int]($low + 2)
}

function Get-Geometry([int]$size) {
    if ($hand.ContainsKey($size)) {
        $g = $hand[$size]
        $inset = $g.Inset; $pillW = $g.PillW; $pillH = $g.PillH; $pillY = $g.PillY; $dot = $g.Dot
    } else {
        $inset = [int][math]::Round($size * $design.Inset)
        $tile = $size - 2 * $inset
        # Same parity as the canvas keeps the pill centred on whole pixels.
        $pillW = Round-Parity ($tile * $design.PillW) ($size % 2)
        $pillH = Round-Parity ($tile * $design.PillH) ($size % 2)
        # Keep the design's pill centre, so the snapped pill stays centred.
        $pillY = $inset + [int][math]::Round($tile * $design.PillTop - ($pillH - $tile * $design.PillH) / 2)
        if ($size -ge 128) {
            # Big enough for the exact dot; anti-aliasing hides the half pixels.
            $dot = $pillH * $design.Dot
        } else {
            # Same parity as the pill height keeps the dot on whole pixels.
            $dot = Round-Parity ($pillH * $design.Dot) ($pillH % 2)
        }
    }
    $pillX = ($size - $pillW) / 2
    return @{
        Inset = $inset
        PillX = $pillX; PillY = $pillY; PillW = $pillW; PillH = $pillH
        DotCx = $pillX + $pillW - $pillH / 2; DotCy = $pillY + $pillH / 2; Dot = $dot
    }
}

function New-SquirclePath([double]$left, [double]$top, [double]$side) {
    $span = $side * $cornerSpan
    $k = $span * $cornerPull
    $right = $left + $side; $b = $top + $side
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $path.AddLine([single]($left + $span), [single]$top, [single]($right - $span), [single]$top)
    $path.AddBezier([single]($right - $span), [single]$top, [single]($right - $span + $k), [single]$top, [single]$right, [single]($top + $span - $k), [single]$right, [single]($top + $span))
    $path.AddLine([single]$right, [single]($top + $span), [single]$right, [single]($b - $span))
    $path.AddBezier([single]$right, [single]($b - $span), [single]$right, [single]($b - $span + $k), [single]($right - $span + $k), [single]$b, [single]($right - $span), [single]$b)
    $path.AddLine([single]($right - $span), [single]$b, [single]($left + $span), [single]$b)
    $path.AddBezier([single]($left + $span), [single]$b, [single]($left + $span - $k), [single]$b, [single]$left, [single]($b - $span + $k), [single]$left, [single]($b - $span))
    $path.AddLine([single]$left, [single]($b - $span), [single]$left, [single]($top + $span))
    $path.AddBezier([single]$left, [single]($top + $span), [single]$left, [single]($top + $span - $k), [single]($left + $span - $k), [single]$top, [single]($left + $span), [single]$top)
    $path.CloseFigure()
    return $path
}

function New-PillPath([double]$x, [double]$y, [double]$w, [double]$h) {
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $path.AddArc([single]$x, [single]$y, [single]$h, [single]$h, 90, 180)
    $path.AddArc([single]($x + $w - $h), [single]$y, [single]$h, [single]$h, 270, 180)
    $path.CloseFigure()
    return $path
}

function Render-Logo([int]$size) {
    $g = Get-Geometry $size
    $s = $supersample
    $big = New-Object System.Drawing.Bitmap ($size * $s), ($size * $s), ([System.Drawing.Imaging.PixelFormat]::Format32bppPArgb)
    $gfx = [System.Drawing.Graphics]::FromImage($big)
    $gfx.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $gfx.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $gfx.Clear([System.Drawing.Color]::Transparent)

    $left = $g.Inset * $s
    $side = ($size - 2 * $g.Inset) * $s
    $tile = New-SquirclePath $left $left $side
    $tileBrush = New-Object System.Drawing.SolidBrush $tileColor
    $gfx.FillPath($tileBrush, $tile)

    $pillBrush = New-Object System.Drawing.SolidBrush $pillColor
    $gfx.FillPath($pillBrush, (New-PillPath ($g.PillX * $s) ($g.PillY * $s) ($g.PillW * $s) ($g.PillH * $s)))

    $dotBrush = New-Object System.Drawing.SolidBrush $dotColor
    $d = $g.Dot * $s
    $dx = $g.DotCx * $s - $d / 2
    $dy = $g.DotCy * $s - $d / 2
    if ($g.Dot -le 2) {
        # A 2 px circle smears into four pale pixels; a square stays a dot.
        $gfx.FillRectangle($dotBrush, [single]$dx, [single]$dy, [single]$d, [single]$d)
    } else {
        $gfx.FillEllipse($dotBrush, [single]$dx, [single]$dy, [single]$d, [single]$d)
    }
    $gfx.Dispose()

    # Exact box filter: each output pixel averages its 8x8 block, so edges
    # that sit on whole pixels stay sharp (GDI+ scaling would blur them).
    $bigRect = New-Object System.Drawing.Rectangle 0, 0, $big.Width, $big.Height
    $bigData = $big.LockBits($bigRect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppPArgb)
    $src = New-Object byte[] ($bigData.Stride * $big.Height)
    [System.Runtime.InteropServices.Marshal]::Copy($bigData.Scan0, $src, 0, $src.Length)
    $big.UnlockBits($bigData)
    $big.Dispose()
    $dst = [IconBoxFilter]::Reduce($src, $bigData.Stride, $size, $s)

    $out = New-Object System.Drawing.Bitmap $size, $size, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $outRect = New-Object System.Drawing.Rectangle 0, 0, $size, $size
    $outData = $out.LockBits($outRect, [System.Drawing.Imaging.ImageLockMode]::WriteOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    for ($y = 0; $y -lt $size; $y++) {
        [System.Runtime.InteropServices.Marshal]::Copy($dst, $y * $size * 4, [IntPtr]($outData.Scan0.ToInt64() + $y * $outData.Stride), $size * 4)
    }
    $out.UnlockBits($outData)
    return $out
}

function Get-PngBytes([System.Drawing.Bitmap]$bitmap) {
    $stream = New-Object System.IO.MemoryStream
    $bitmap.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
    return , $stream.ToArray()
}

function Get-DibBytes([System.Drawing.Bitmap]$bitmap) {
    # 32 bpp BITMAPINFOHEADER image: double height, bottom-up BGRA rows, then
    # a 1 bpp AND mask (set where fully transparent) padded to 32 bits per row.
    $n = $bitmap.Width
    $rect = New-Object System.Drawing.Rectangle 0, 0, $n, $n
    $data = $bitmap.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $pixels = New-Object byte[] ($data.Stride * $n)
    [System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $pixels, 0, $pixels.Length)
    $bitmap.UnlockBits($data)

    $maskStride = [int]([math]::Floor(($n + 31) / 32) * 4)
    $stream = New-Object System.IO.MemoryStream
    $w = New-Object System.IO.BinaryWriter $stream
    $w.Write([int]40); $w.Write([int]$n); $w.Write([int](2 * $n))
    $w.Write([int16]1); $w.Write([int16]32); $w.Write([int]0)
    $w.Write([int]($n * $n * 4 + $maskStride * $n))
    $w.Write([int]0); $w.Write([int]0); $w.Write([int]0); $w.Write([int]0)
    for ($y = $n - 1; $y -ge 0; $y--) {
        $w.Write($pixels, $y * $data.Stride, $n * 4)
    }
    for ($y = $n - 1; $y -ge 0; $y--) {
        $row = New-Object byte[] $maskStride
        for ($x = 0; $x -lt $n; $x++) {
            if ($pixels[$y * $data.Stride + $x * 4 + 3] -eq 0) {
                $row[[math]::Floor($x / 8)] = $row[[math]::Floor($x / 8)] -bor (0x80 -shr ($x % 8))
            }
        }
        $w.Write($row)
    }
    $w.Flush()
    return , $stream.ToArray()
}

$images = @()
foreach ($size in $sizes) {
    $bitmap = Render-Logo $size
    $bitmap.Save((Join-Path $OutDir "logo-$size.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    if ($size -eq 256) {
        $bitmap.Save((Join-Path $assets 'logo-256.png'), [System.Drawing.Imaging.ImageFormat]::Png)
    }
    # PNG payloads for the big sizes keep the file small; classic DIBs for the
    # rest suit every icon reader.
    if ($size -ge 128) { $bytes = Get-PngBytes $bitmap } else { $bytes = Get-DibBytes $bitmap }
    $images += , @{ Size = $size; Bytes = $bytes }
    $bitmap.Dispose()
}

# ICONDIR, one ICONDIRENTRY per image, then the payloads.
$stream = New-Object System.IO.MemoryStream
$w = New-Object System.IO.BinaryWriter $stream
$w.Write([int16]0); $w.Write([int16]1); $w.Write([int16]$images.Count)
$offset = 6 + 16 * $images.Count
foreach ($image in $images) {
    $dim = if ($image.Size -ge 256) { 0 } else { $image.Size }
    $w.Write([byte]$dim); $w.Write([byte]$dim); $w.Write([byte]0); $w.Write([byte]0)
    $w.Write([int16]1); $w.Write([int16]32)
    $w.Write([int]$image.Bytes.Length); $w.Write([int]$offset)
    $offset += $image.Bytes.Length
}
foreach ($image in $images) { $w.Write($image.Bytes) }
$w.Flush()
[System.IO.File]::WriteAllBytes((Join-Path $assets 'icon.ico'), $stream.ToArray())

Write-Host "Wrote $(Join-Path $assets 'icon.ico') ($($images.Count) sizes), $(Join-Path $assets 'logo-256.png'), PNGs in $OutDir"
