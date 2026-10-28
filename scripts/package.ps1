# Builds, signs and registers the sparse MSIX package that gives
# target\release\vale.exe its package identity.
# Safe to re-run: existing certificate, staging and registration are reused or replaced.
# Requires Windows PowerShell 5.1 or later; no elevation.

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$packagingDir = Join-Path $root 'packaging'
$certDir = Join-Path $packagingDir 'dev-cert'
$pfxPath = Join-Path $certDir 'dev.pfx'
$cerPath = Join-Path $certDir 'dev.cer'
$passwordPath = Join-Path $certDir 'password.txt'
$stagingDir = Join-Path $packagingDir 'staging'
$releaseDir = Join-Path $root 'target\release'
$exePath = Join-Path $releaseDir 'vale.exe'
$msixPath = Join-Path $root 'target\Vale.msix'
$importCommand = "Import-Certificate -FilePath `"$cerPath`" -CertStoreLocation Cert:\LocalMachine\TrustedPeople"

function Write-ImportInstructions {
    Write-Host ''
    Write-Host 'Run this ONCE in an elevated (Administrator) PowerShell to trust the dev certificate:' -ForegroundColor Yellow
    Write-Host "  $importCommand" -ForegroundColor Yellow
    Write-Host ''
}

function Invoke-Native([string]$exe, [string[]]$arguments, [string]$what) {
    & $exe @arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$what failed with exit code $LASTEXITCODE."
    }
}

if (-not (Test-Path $exePath)) {
    throw "$exePath not found. Run 'cargo build --release' first."
}

# 1. Newest Windows SDK tools.
$sdkBin = 'C:\Program Files (x86)\Windows Kits\10\bin'
$sdkVersionDir = Get-ChildItem $sdkBin -Directory |
    Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' -and (Test-Path (Join-Path $_.FullName 'x64\makeappx.exe')) -and (Test-Path (Join-Path $_.FullName 'x64\signtool.exe')) } |
    Sort-Object { [version]$_.Name } -Descending |
    Select-Object -First 1
if (-not $sdkVersionDir) {
    throw "makeappx.exe/signtool.exe not found under $sdkBin\*\x64. Install the Windows SDK."
}
$makeappx = Join-Path $sdkVersionDir.FullName 'x64\makeappx.exe'
$signtool = Join-Path $sdkVersionDir.FullName 'x64\signtool.exe'
Write-Host "Using Windows SDK tools $($sdkVersionDir.Name)"

# 2. Self-signed development certificate (CurrentUser store; no elevation).
if (-not (Test-Path $pfxPath)) {
    New-Item -ItemType Directory -Force $certDir | Out-Null
    $cert = New-SelfSignedCertificate -Type Custom -Subject 'CN=DynamicIslandDev' -KeyUsage DigitalSignature `
        -FriendlyName 'DynamicIsland Dev' -CertStoreLocation Cert:\CurrentUser\My `
        -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3', '2.5.29.19={text}')

    $bytes = New-Object byte[] 24
    $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    $rng.GetBytes($bytes)
    $rng.Dispose()
    $password = [Convert]::ToBase64String($bytes)
    [System.IO.File]::WriteAllText($passwordPath, $password)

    $securePassword = ConvertTo-SecureString -String $password -AsPlainText -Force
    Export-PfxCertificate -Cert $cert -FilePath $pfxPath -Password $securePassword | Out-Null
    Export-Certificate -Cert $cert -FilePath $cerPath -Type CERT | Out-Null
    Write-Host "Created development certificate $pfxPath"
    Write-ImportInstructions
}
if (-not (Test-Path $passwordPath)) {
    throw "$passwordPath is missing. Delete $certDir and re-run to create a new certificate."
}
$password = ([System.IO.File]::ReadAllText($passwordPath)).Trim()

# 3. Staging directory: manifest + logo.
if (Test-Path $stagingDir) {
    Remove-Item -Recurse -Force $stagingDir
}
New-Item -ItemType Directory -Force $stagingDir | Out-Null
Copy-Item (Join-Path $packagingDir 'AppxManifest.xml') $stagingDir

Add-Type -AssemblyName System.Drawing
$size = 150
$bitmap = New-Object System.Drawing.Bitmap $size, $size, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
try {
    # The Bata logo, scaled down from the one scripts\make-icon.ps1 renders.
    $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $graphics.Clear([System.Drawing.Color]::Transparent)
    $logo = [System.Drawing.Image]::FromFile((Join-Path $root 'assets\logo-256.png'))
    try {
        $graphics.DrawImage($logo, 0, 0, $size, $size)
    }
    finally {
        $logo.Dispose()
    }
    $bitmap.Save((Join-Path $stagingDir 'logo.png'), [System.Drawing.Imaging.ImageFormat]::Png)
}
finally {
    $graphics.Dispose()
    $bitmap.Dispose()
}

# 4. Pack and sign.
Invoke-Native $makeappx @('pack', '/d', $stagingDir, '/p', $msixPath, '/nv', '/o') 'makeappx pack'
Write-Host 'Signing package...'
& $signtool sign /fd SHA256 /f $pfxPath /p $password $msixPath
if ($LASTEXITCODE -ne 0) {
    throw "signtool sign failed with exit code $LASTEXITCODE."
}

# 5. Register the sparse package against the release build output.
$cer = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2 -ArgumentList $cerPath
$trusted = Get-ChildItem Cert:\LocalMachine\TrustedPeople | Where-Object { $_.Thumbprint -eq $cer.Thumbprint }
if (-not $trusted) {
    Write-Warning 'The development certificate is not in Cert:\LocalMachine\TrustedPeople yet; registration will most likely fail.'
}

# DynamicIsland: the package name before the rename to Vale.
foreach ($existing in Get-AppxPackage -Name Vale, DynamicIsland) {
    Write-Host "Removing existing registration $($existing.PackageFullName)"
    $existing | Remove-AppxPackage
}

try {
    Add-AppxPackage -Path $msixPath -ExternalLocation $releaseDir
}
catch {
    Write-Host ''
    Write-Host 'Add-AppxPackage failed:' -ForegroundColor Red
    Write-Host "  $($_.Exception.Message)" -ForegroundColor Red
    if (-not $trusted) {
        Write-Host 'The certificate that signed the package is not trusted yet.' -ForegroundColor Red
        Write-ImportInstructions
        Write-Host 'Then re-run scripts\package.ps1.'
    }
    exit 1
}

Get-AppxPackage -Name Vale | Format-List Name, PackageFullName, InstallLocation, Status
Write-Host 'Package registered.' -ForegroundColor Green
