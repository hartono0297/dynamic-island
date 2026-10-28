# Vale installer.
#
# Install or update (PowerShell, no admin needed):
#   irm https://raw.githubusercontent.com/hartono0297/dynamic-island/main/install.ps1 | iex
#
# Or run it from an extracted release zip: it then installs the exe next to it
# instead of downloading.
#
# Vale was called Dynamic Island before; an older install is replaced and its
# leftovers (program folder, shortcut, autostart entry) are removed. Your data
# is moved by Vale itself on its first start.

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$Repo = 'hartono0297/dynamic-island'
$RawBase = "https://raw.githubusercontent.com/$Repo/main"
$InstallDir = Join-Path $env:LOCALAPPDATA 'Programs\Vale'
$Exe = Join-Path $InstallDir 'vale.exe'
$Programs = [Environment]::GetFolderPath('Programs')
$Shortcut = Join-Path $Programs 'Vale.lnk'
$ClaudeSettings = Join-Path $env:USERPROFILE '.claude\settings.json'
$RunKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
# Before the rename.
$OldInstallDir = Join-Path $env:LOCALAPPDATA 'Programs\DynamicIsland'
$OldShortcut = Join-Path $Programs 'Dynamic Island.lnk'

function Say([string]$Message) { Write-Host "  $Message" }

Write-Host ''
Write-Host 'Vale - install'

if (-not [Environment]::Is64BitOperatingSystem) {
    throw 'Vale hanya jalan di Windows 64-bit.'
}
if ([Environment]::OSVersion.Version.Build -lt 22000) {
    Say 'Peringatan: Vale dibuat untuk Windows 11. Di Windows 10 mungkin ada yang tidak jalan.'
}

$work = Join-Path $env:TEMP ('vale-install-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work | Out-Null
try {
    # 1. Get the exe: next to this script (release zip) or the latest GitHub release.
    $source = $null
    $uninstallSource = $null
    if ($PSScriptRoot) {
        $candidate = Join-Path $PSScriptRoot 'vale.exe'
        if (Test-Path $candidate) {
            $source = $candidate
            $localUninstall = Join-Path $PSScriptRoot 'uninstall.ps1'
            if (Test-Path $localUninstall) { $uninstallSource = $localUninstall }
            Say "Pakai file lokal: $candidate"
        }
    }
    if (-not $source) {
        Say 'Mencari versi terbaru di GitHub...'
        $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -Headers @{ 'User-Agent' = 'vale-installer' }
        $exeAsset = $release.assets | Where-Object { $_.name -eq 'vale.exe' } | Select-Object -First 1
        $shaAsset = $release.assets | Where-Object { $_.name -eq 'vale.exe.sha256' } | Select-Object -First 1
        if (-not $exeAsset -or -not $shaAsset) {
            throw "Rilis $($release.tag_name) tidak punya vale.exe dan checksum-nya."
        }
        Say "Mengunduh $($release.tag_name)..."
        $source = Join-Path $work 'vale.exe'
        $shaFile = Join-Path $work 'vale.exe.sha256'
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

    # 2. Stop a running island, Vale or the old Dynamic Island (an update
    #    replaces its exe, and Vale moves the old data folder on start).
    $running = Get-Process -Name 'vale', 'dynamic-island' -ErrorAction SilentlyContinue
    if ($running) {
        Say 'Menutup island yang sedang jalan...'
        $running | Stop-Process -Force
        Start-Sleep -Milliseconds 800
    }

    # 3. Copy into %LOCALAPPDATA%\Programs\Vale.
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
    $link.Description = 'Vale'
    $link.Save()
    Say 'Shortcut Start Menu dibuat.'

    # 5. Claude Code hooks installed earlier (by Vale or Dynamic Island) point
    #    at the old exe: repoint them.
    if ((Test-Path $ClaudeSettings) -and ((Get-Content -Path $ClaudeSettings -Raw) -match '(dynamic-island|vale)\.exe\\?" --hook')) {
        Start-Process -FilePath $Exe -ArgumentList '--install-hooks' -Wait -WindowStyle Hidden
        Say 'Hook Claude Code diarahkan ke lokasi baru.'
    }

    # 6. Leftovers of Dynamic Island: program folder, shortcut, autostart.
    if (Test-Path $OldShortcut) {
        Remove-Item -Path $OldShortcut -Force
    }
    if (Test-Path $OldInstallDir) {
        Remove-Item -Path $OldInstallDir -Recurse -Force -ErrorAction SilentlyContinue
        Say "Versi lama (Dynamic Island) di $OldInstallDir dihapus."
    }
    $oldRun = Get-ItemProperty -Path $RunKey -Name 'DynamicIsland' -ErrorAction SilentlyContinue
    if ($oldRun -and ($oldRun.DynamicIsland -match 'dynamic-island\.exe')) {
        Remove-ItemProperty -Path $RunKey -Name 'DynamicIsland'
    }

    # 7. Start it. The island registers its own autostart (tray > "Jalan saat Windows start").
    Start-Process -FilePath $Exe -WorkingDirectory $InstallDir
    Write-Host ''
    Say 'Selesai. Vale sudah jalan di tengah atas layar; ikonnya ada di tray.'
    Say 'Untuk update, jalankan perintah install yang sama lagi.'
    Say "Untuk uninstall: powershell -ExecutionPolicy Bypass -File `"$InstallDir\uninstall.ps1`""
    Write-Host ''
}
finally {
    Remove-Item -Path $work -Recurse -Force -ErrorAction SilentlyContinue
}
