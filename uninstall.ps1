# Dynamic Island uninstaller.
#
#   irm https://raw.githubusercontent.com/hartono0297/dynamic-island/main/uninstall.ps1 | iex
#
# Your data (%APPDATA%\DynamicIsland: settings, notes, Google login) is kept.
# To delete it as well, set DI_PURGE=1 first:
#   $env:DI_PURGE = '1'; irm https://raw.githubusercontent.com/hartono0297/dynamic-island/main/uninstall.ps1 | iex

$ErrorActionPreference = 'Stop'

$InstallDir = Join-Path $env:LOCALAPPDATA 'Programs\DynamicIsland'
$Exe = Join-Path $InstallDir 'dynamic-island.exe'
$Shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'Dynamic Island.lnk'
$DataDir = Join-Path $env:APPDATA 'DynamicIsland'
$RunKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'

function Say([string]$Message) { Write-Host "  $Message" }

Write-Host ''
Write-Host 'Dynamic Island - uninstall'

# 1. Remove the island's Claude Code hooks while the exe still exists.
if (Test-Path $Exe) {
    Start-Process -FilePath $Exe -ArgumentList '--uninstall-hooks' -Wait -WindowStyle Hidden
    Say 'Hook Claude Code milik island dihapus (kalau ada).'
}

# 2. Stop the island.
$running = Get-Process -Name 'dynamic-island' -ErrorAction SilentlyContinue
if ($running) {
    $running | Stop-Process -Force
    Start-Sleep -Milliseconds 800
    Say 'Dynamic Island ditutup.'
}

# 3. Autostart entry (only when it points at the island).
$run = Get-ItemProperty -Path $RunKey -Name 'DynamicIsland' -ErrorAction SilentlyContinue
if ($run -and ($run.DynamicIsland -match 'dynamic-island\.exe')) {
    Remove-ItemProperty -Path $RunKey -Name 'DynamicIsland'
    Say 'Autostart dihapus.'
}

# 4. Shortcut and program files.
if (Test-Path $Shortcut) {
    Remove-Item -Path $Shortcut -Force
    Say 'Shortcut Start Menu dihapus.'
}
if (Test-Path $InstallDir) {
    Remove-Item -Path $InstallDir -Recurse -Force
    Say "Folder $InstallDir dihapus."
}

# 5. Data.
if ($env:DI_PURGE -eq '1') {
    if (Test-Path $DataDir) {
        Remove-Item -Path $DataDir -Recurse -Force
        Say "Data di $DataDir dihapus."
    }
} elseif (Test-Path $DataDir) {
    Say "Data (setting, notes, login Google) tetap disimpan di $DataDir"
    Say 'Hapus folder itu kalau tidak dipakai lagi.'
}

Write-Host ''
Say 'Selesai.'
Write-Host ''
