param(
    [Parameter(Mandatory)][string]$Installer,
    [string]$PreviousInstaller,
    [string]$Version
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$installDir = "$env:LOCALAPPDATA\Programs\Glint API"
$desktop = Join-Path ([Environment]::GetFolderPath('Desktop')) 'Glint API.lnk'
$startMenu = Join-Path ([Environment]::GetFolderPath('Programs')) 'Glint API\Glint API.lnk'
$dataDir = "$env:LOCALAPPDATA\Glint API"
New-Item -ItemType Directory -Force $dataDir | Out-Null
$sentinel = Join-Path $dataDir 'installer-test-preserve.txt'
'Preserve user data across upgrades and uninstall.' | Set-Content $sentinel
$shell = New-Object -ComObject WScript.Shell
# Install and reinstall to verify upgrades preserve the same application ID/path.
1..2 | ForEach-Object {
    $process = Start-Process $Installer -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-' -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Setup exited with $($process.ExitCode)." }
    foreach ($link in @($desktop, $startMenu)) {
        if (-not (Test-Path $link)) { throw "Missing shortcut: $link" }
        $shortcut = $shell.CreateShortcut($link)
        if ($shortcut.TargetPath -ne "$installDir\glint-api.exe") { throw "Incorrect shortcut target: $link" }
        if ($shortcut.IconLocation -notlike "$installDir\glint-api.exe,*") { throw "Incorrect shortcut icon: $link" }
    }
    if (-not (Test-Path $sentinel)) { throw 'Installation removed user data.' }
}
$exe = "$installDir\glint-api.exe"
$bytes = [IO.File]::ReadAllBytes($exe)
$pe = [BitConverter]::ToInt32($bytes, 0x3c)
if ([BitConverter]::ToUInt16($bytes, $pe + 24 + 68) -ne 2) { throw 'The release executable is not a Windows GUI application.' }
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class ResourceProbe {
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr LoadLibraryEx(string path, IntPtr file, uint flags);
    [DllImport("kernel32.dll")] public static extern IntPtr FindResource(IntPtr module, IntPtr name, IntPtr type);
    [DllImport("kernel32.dll")] public static extern bool FreeLibrary(IntPtr module);
}
'@
$module = [ResourceProbe]::LoadLibraryEx($exe, [IntPtr]::Zero, 2)
try {
    if ($module -eq [IntPtr]::Zero -or [ResourceProbe]::FindResource($module, [IntPtr]1, [IntPtr]14) -eq [IntPtr]::Zero) {
        throw 'The application icon is not embedded in the executable.'
    }
} finally { if ($module -ne [IntPtr]::Zero) { [void][ResourceProbe]::FreeLibrary($module) } }
if ($PreviousInstaller) {
    & "$PSScriptRoot\test-updater.ps1" -Installer $Installer -PreviousInstaller $PreviousInstaller -Version $Version
}
$uninstall = Start-Process "$installDir\unins000.exe" -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -Wait -PassThru
if ($uninstall.ExitCode -ne 0) { throw "Uninstall exited with $($uninstall.ExitCode)." }
foreach ($path in @($desktop, $startMenu, $exe)) {
    if (Test-Path $path) { throw "Uninstall did not remove: $path" }
}
if (-not (Test-Path $sentinel)) { throw 'Uninstall removed user data.' }
Remove-Item $sentinel
Write-Host 'Silent install, upgrade, shortcut targets/icons, GUI subsystem, embedded icon and uninstall verified.'
