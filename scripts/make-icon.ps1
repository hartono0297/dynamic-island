# Renders the Vale logo, the cat head (a flat-based dome, two ears and two plain
# espresso pill eyes, fixed brand colours, the same cat as the one in the
# notch, see src/ui/robot.rs) and writes assets\icon.ico (16-256 px) and
# assets\logo-256.png. The vector source is assets\logo.svg (and its copy
# site\img\logo.svg); the settings window draws the same shapes in
# `fn logo` of src/ui/settings_controls.rs. Keep all of them in sync.
# Sizes up to 32 px snap the eyes to whole pixels so they stay crisp; the head
# and ears scale with the 256 px design.
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

$headColor = [System.Drawing.Color]::FromArgb(255, 0xE2, 0x68, 0x3C)
$eyeColor = [System.Drawing.Color]::FromArgb(255, 0x17, 0x12, 0x0F)

# The 256 px design (logo.svg): the robot's 64 grid scaled by 4.6 about the
# middle of the cat. The head spans x 13..243, y 97..235 (a dome on a flat
# base, bottom corners 18.5); the ears are triangles with a 23 px round
# stroke, (22,157) (36,32.5) (105,106) and mirrored; the eyes are 26 x 46
# pills at x 78 and 152, y 147.5.
$ears = @(
    @(@(22, 157), @(36, 32.5), @(105, 106)),
    @(@(234, 157), @(220, 32.5), @(151, 106))
)
$earStroke = 23
$eyes = @(@(78, 147.5), @(152, 147.5))
$eyeW = 26
$eyeH = 46

# GDI+ draws only cubic curves: a quadratic (p0, c, p1) becomes the cubic with
# controls p0 + 2/3 (c - p0) and p1 + 2/3 (c - p1).
function Add-Quad($path, $p0, $c, $p1, [double]$k) {
    $ax = ($p0[0] + 2 / 3 * ($c[0] - $p0[0])) * $k
    $ay = ($p0[1] + 2 / 3 * ($c[1] - $p0[1])) * $k
    $bx = ($p1[0] + 2 / 3 * ($c[0] - $p1[0])) * $k
    $by = ($p1[1] + 2 / 3 * ($c[1] - $p1[1])) * $k
    $path.AddBezier([single]($p0[0] * $k), [single]($p0[1] * $k), [single]$ax, [single]$ay, [single]$bx, [single]$by, [single]($p1[0] * $k), [single]($p1[1] * $k))
}

function New-HeadPath([double]$k) {
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $path.StartFigure()
    $path.AddLine([single](13 * $k), [single](216.5 * $k), [single](13 * $k), [single](175 * $k))
    Add-Quad $path @(13, 175) @(13, 97) @(128, 97) $k
    Add-Quad $path @(128, 97) @(243, 97) @(243, 175) $k
    $path.AddLine([single](243 * $k), [single](175 * $k), [single](243 * $k), [single](216.5 * $k))
    Add-Quad $path @(243, 216.5) @(243, 235) @(224.6, 235) $k
    $path.AddLine([single](224.6 * $k), [single](235 * $k), [single](31.4 * $k), [single](235 * $k))
    Add-Quad $path @(31.4, 235) @(13, 235) @(13, 216.5) $k
    $path.CloseFigure()
    return $path
}

function New-EarPath($points, [double]$k) {
    $pts = foreach ($p in $points) {
        New-Object System.Drawing.PointF ([single]($p[0] * $k)), ([single]($p[1] * $k))
    }
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $path.AddPolygon([System.Drawing.PointF[]]$pts)
    return $path
}

function New-PillPath([double]$x, [double]$y, [double]$w, [double]$h) {
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    if ($w -le $h) {
        $path.AddArc([single]$x, [single]$y, [single]$w, [single]$w, 180, 180)
        $path.AddArc([single]$x, [single]($y + $h - $w), [single]$w, [single]$w, 0, 180)
    } else {
        $path.AddArc([single]$x, [single]$y, [single]$h, [single]$h, 90, 180)
        $path.AddArc([single]($x + $w - $h), [single]$y, [single]$h, [single]$h, 270, 180)
    }
    $path.CloseFigure()
    return $path
}

function Render-Logo([int]$size) {
    $s = $supersample
    $k = $size / 256.0
    $big = New-Object System.Drawing.Bitmap ($size * $s), ($size * $s), ([System.Drawing.Imaging.PixelFormat]::Format32bppPArgb)
    $gfx = [System.Drawing.Graphics]::FromImage($big)
    $gfx.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $gfx.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $gfx.Clear([System.Drawing.Color]::Transparent)

    $headBrush = New-Object System.Drawing.SolidBrush $headColor
    $earPen = New-Object System.Drawing.Pen $headColor, ([single]($earStroke * $k * $s))
    $earPen.LineJoin = [System.Drawing.Drawing2D.LineJoin]::Round
    foreach ($ear in $ears) {
        $path = New-EarPath $ear ($k * $s)
        $gfx.FillPath($headBrush, $path)
        $gfx.DrawPath($earPen, $path)
    }
    $earPen.Dispose()
    $gfx.FillPath($headBrush, (New-HeadPath ($k * $s)))

    # Eyes: the design's, or snapped to whole pixels at the small sizes.
    $eyeBrush = New-Object System.Drawing.SolidBrush $eyeColor
    foreach ($eye in $eyes) {
        $x = $eye[0] * $k; $y = $eye[1] * $k; $w = $eyeW * $k; $h = $eyeH * $k
        if ($size -le 32) {
            $w2 = [math]::Max(2, [math]::Round($w))
            $h2 = [math]::Max(4, [math]::Round($h))
            $x = [math]::Round($x + ($w - $w2) / 2)
            $y = [math]::Round($y + ($h - $h2) / 2)
            $w = $w2; $h = $h2
        }
        $gfx.FillPath($eyeBrush, (New-PillPath ($x * $s) ($y * $s) ($w * $s) ($h * $s)))
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

$images = @()
foreach ($size in $sizes) {
    $bitmap = Render-Logo $size
    $bitmap.Save((Join-Path $OutDir "logo-$size.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    if ($size -eq 256) {
        $bitmap.Save((Join-Path $assets 'logo-256.png'), [System.Drawing.Imaging.ImageFormat]::Png)
    }
    # PNG payloads at every size (Windows 11 reads them everywhere). The
    # uncompressed 32 bpp DIBs of v3.5.0 cost 43 KB for the seven small sizes;
    # the same pixels as PNG cost under 10 KB (src/ui/icon_frames.rs pins them).
    $bytes = Get-PngBytes $bitmap
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
