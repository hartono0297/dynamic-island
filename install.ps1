# Dynamic Island installer.
#
# Install or update (PowerShell, no admin needed):
#   irm https://raw.githubusercontent.com/hartono0297/dynamic-island/main/install.ps1 | iex
#
# Or run it from an extracted release zip: it then installs the exe next to it
# instead of downloading.

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$Repo = 'hartono0297/dynamic-island'
$RawBase = "https://raw.githubusercontent.com/$Repo/main"
$InstallDir = Join-Path $env:LOCALAPPDATA 'Programs\DynamicIsland'
$Exe = Join-Path $InstallDir 'dynamic-island.exe'
$Shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'Dynamic Island.lnk'
$ClaudeSettings = Join-Path $env:USERPROFILE '.claude\settings.json'

function Say([string]$Message) { Write-Host "  $Message" }

Write-Host ''
Write-Host 'Dynamic Island - install'

if (-not [Environment]::Is64BitOperatingSystem) {
    throw 'Dynamic Island hanya jalan di Windows 64-bit.'
}
if ([Environment]::OSVersion.Version.Build -lt 22000) {
    Say 'Peringatan: Dynamic Island dibuat untuk Windows 11. Di Windows 10 mungkin ada yang tidak jalan.'
}

$work = Join-Path $env:TEMP ('dynamic-island-install-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work | Out-Null
try {
    # 1. Get the exe: next to this script (release zip) or the latest GitHub release.
    $source = $null
    $uninstallSource = $null
    if ($PSScriptRoot) {
        $candidate = Join-Path $PSScriptRoot 'dynamic-island.exe'
        if (Test-Path $candidate) {
            $source = $candidate
            $localUninstall = Join-Path $PSScriptRoot 'uninstall.ps1'
            if (Test-Path $localUninstall) { $uninstallSource = $localUninstall }
            Say "Pakai file lokal: $candidate"
        }
    }
    if (-not $source) {
        Say 'Mencari versi terbaru di GitHub...'
        $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -Headers @{ 'User-Agent' = 'dynamic-island-installer' }
        $exeAsset = $release.assets | Where-Object { $_.name -eq 'dynamic-island.exe' } | Select-Object -First 1
        $shaAsset = $release.assets | Where-Object { $_.name -eq 'dynamic-island.exe.sha256' } | Select-Object -First 1
        if (-not $exeAsset -or -not $shaAsset) {
            throw "Rilis $($release.tag_name) tidak punya dynamic-island.exe dan checksum-nya."
        }
        Say "Mengunduh $($release.tag_name)..."
        $source = Join-Path $work 'dynamic-island.exe'
        $shaFile = Join-Path $work 'dynamic-island.exe.sha256'
        Invoke-WebRequest -Uri $exeAsset.browser_download_url -OutFile $source -UseBasicParsing
        Invoke-WebRequest -Uri $shaAsset.browser_download_url -OutFile $shaFile -UseBasicParsing
        $expected = ((Get-Content -Path $shaFile -Raw).Trim() -split '\s+')[0].ToLowerInvariant()
        $actual = (Get-FileHash -Path $source -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($expected -ne $actual) {
            throw 'Checksum file yang diunduh tidak cocok. Install dibatalkan.'
        }
        Say 'Checksum cocok.'
        $uninstallSource = Join-Path $work 'uninstall.ps1'
        try {
            Invoke-WebRequest -Uri "$RawBase/uninstall.ps1" -OutFile $uninstallSource -UseBasicParsing
        } catch {
            $uninstallSource = $null
        }
    }

    # 2. Stop a running island (an update replaces its exe).
    $running = Get-Process -Name 'dynamic-island' -ErrorAction SilentlyContinue
    if ($running) {
        Say 'Menutup Dynamic Island yang sedang jalan...'
        $running | Stop-Process -Force
        Start-Sleep -Milliseconds 800
    }

    # 3. Copy into %LOCALAPPDATA%\Programs\DynamicIsland.
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    Copy-Item -Path $source -Destination $Exe -Force
    Unblock-File -Path $Exe
    if ($uninstallSource) {
        Copy-Item -Path $uninstallSource -Destination (Join-Path $InstallDir 'uninstall.ps1') -Force
    }
    Say "Terpasang di $InstallDir"

    # 4. Start Menu shortcut.
    $shell = New-Object -ComObject WScript.Shell
    $link = $shell.CreateShortcut($Shortcut)
    $link.TargetPath = $Exe
    $link.WorkingDirectory = $InstallDir
    $link.Description = 'Dynamic Island'
    $link.Save()
    Say 'Shortcut Start Menu dibuat.'

    # 5. Claude Code hooks installed earlier point at the old exe: repoint them.
    if ((Test-Path $ClaudeSettings) -and ((Get-Content -Path $ClaudeSettings -Raw) -match 'dynamic-island\.exe\\?" --hook')) {
        Start-Process -FilePath $Exe -ArgumentList '--install-hooks' -Wait -WindowStyle Hidden
        Say 'Hook Claude Code diarahkan ke lokasi baru.'
    }

    # 6. Start it. The island registers its own autostart (tray > "Jalan saat Windows start").
    Start-Process -FilePath $Exe -WorkingDirectory $InstallDir
    Write-Host ''
    Say 'Selesai. Island sudah jalan di tengah atas layar; ikonnya ada di tray.'
    Say 'Untuk update, jalankan perintah install yang sama lagi.'
    Say "Untuk uninstall: powershell -ExecutionPolicy Bypass -File `"$InstallDir\uninstall.ps1`""
    Write-Host ''
}
finally {
    Remove-Item -Path $work -Recurse -Force -ErrorAction SilentlyContinue
}
