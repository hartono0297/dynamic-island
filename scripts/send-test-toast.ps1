# Fires a test toast through the WinRT ToastNotificationManager using Windows
# PowerShell's own AppUserModelID. -Long sends a 300-character body with emoji.
param(
    [switch]$Long
)

$ErrorActionPreference = 'Stop'

# WinRT type projection only exists in Windows PowerShell 5.1; hop over from pwsh.
if ($PSVersionTable.PSEdition -eq 'Core') {
    $forward = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $PSCommandPath)
    if ($Long) { $forward += '-Long' }
    & powershell.exe @forward
    exit $LASTEXITCODE
}

[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null
[Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime] | Out-Null

$powershellAumid = '{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe'
$stamp = Get-Date -Format 'HH:mm:ss'
$title = "Vale test $stamp"

if ($Long) {
    # Emoji are built from code points so this file stays ASCII (PowerShell 5.1 reads BOM-less files as ANSI).
    $emoji = [char]::ConvertFromUtf32(0x1F680) + [char]::ConvertFromUtf32(0x1F389) + [char]::ConvertFromUtf32(0x2728)
    $filler = 'This is a long test notification body for the Vale probe, checking how text wraps and truncates. '
    while ($filler.Length -lt 300) { $filler += $filler }
    $prefix = "$emoji "
    $body = $prefix + $filler.Substring(0, 300 - $prefix.Length).TrimEnd()
    $body = $body.PadRight(300, '.')
}
else {
    $body = 'Hello from send-test-toast.ps1'
}

$xml = @"
<toast>
  <visual>
    <binding template="ToastGeneric">
      <text>$([System.Security.SecurityElement]::Escape($title))</text>
      <text>$([System.Security.SecurityElement]::Escape($body))</text>
    </binding>
  </visual>
</toast>
"@

$document = New-Object Windows.Data.Xml.Dom.XmlDocument
$document.LoadXml($xml)
$toast = New-Object Windows.UI.Notifications.ToastNotification $document
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($powershellAumid).Show($toast)
Write-Host "Sent toast '$title' ($($body.Length) characters in body)."
