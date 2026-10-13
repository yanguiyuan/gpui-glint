param(
    [Parameter(Mandatory)][string]$Installer,
    [Parameter(Mandatory)][string]$PreviousInstaller,
    [Parameter(Mandatory)][string]$Version
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$installed = "$env:LOCALAPPDATA\Programs\Glint API\glint-api.exe"
$directory = Join-Path $env:RUNNER_TEMP ("glint-api-updater-" + [guid]::NewGuid())
New-Item -ItemType Directory $directory | Out-Null
$helper = Join-Path $directory 'update-helper.exe'
# Keep the new updater outside the installation directory during the upgrade.
Copy-Item $installed $helper
$old = Start-Process $PreviousInstaller -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-' -Wait -PassThru
if ($old.ExitCode -ne 0) { throw 'Could not install the previous version.' }
$previousVersion = (Get-Item $installed).VersionInfo.ProductVersion
if ($previousVersion -eq $Version) { throw 'Upgrade test requires a different previous version.' }
$dataDir = "$env:LOCALAPPDATA\Glint API"
New-Item -ItemType Directory -Force $dataDir | Out-Null
$sentinel = Join-Path $dataDir 'updater-test-preserve.txt'
'Preserve this workspace data.' | Set-Content $sentinel
$package = Join-Path $directory ([IO.Path]::GetFileName($Installer))
Copy-Item $Installer $package
$report = Join-Path $directory 'result.json'
$jobFile = Join-Path $directory 'job.json'
$job = @{
    installer = $package
    executable = $installed
    parent_pid = [uint32]4294967294
    sha256 = ('0' * 64)
    size = (Get-Item $package).Length
    version = $Version
    report = $report
    relaunch = $false
}
$job | ConvertTo-Json | Set-Content $jobFile -Encoding utf8NoBOM
$bad = Start-Process $helper -ArgumentList '--apply-update', ('"' + $jobFile + '"') -Wait -PassThru
if ($bad.ExitCode -eq 0 -or (Get-Content $report -Raw | ConvertFrom-Json).success) { throw 'Updater accepted an invalid installer checksum.' }
if ((Get-Item $installed).VersionInfo.ProductVersion -ne $previousVersion) { throw 'Failed verification changed the installed application.' }
$job.sha256 = (Get-FileHash $package -Algorithm SHA256).Hash.ToLowerInvariant()
$parent = Start-Process "$PSHOME\pwsh.exe" -ArgumentList '-NoProfile', '-Command', 'Start-Sleep -Seconds 120' -PassThru
try {
    $job.parent_pid = $parent.Id
    $job | ConvertTo-Json | Set-Content $jobFile -Encoding utf8NoBOM
    $process = Start-Process $helper -ArgumentList '--apply-update', ('"' + $jobFile + '"') -PassThru
    Start-Sleep -Seconds 2
    $process.Refresh()
    if ($process.HasExited) { throw 'Updater did not wait for the parent application.' }
    if ((Get-Item $installed).VersionInfo.ProductVersion -ne $previousVersion) { throw 'Updater installed before the application exited.' }
    Stop-Process -Id $parent.Id -Force
    if (-not $process.WaitForExit(180000)) { Stop-Process -Id $process.Id -Force; throw 'Updater timed out.' }
    if ($process.ExitCode -ne 0) { throw "Updater failed: $(Get-Content $report -Raw)" }
    if (-not (Get-Content $report -Raw | ConvertFrom-Json).success) { throw 'Updater reported an installation failure.' }
    if ((Get-Item $installed).VersionInfo.ProductVersion -ne $Version) { throw 'Updater did not install the requested version.' }
    if (Test-Path $package) { throw 'Updater did not remove the successful installer download.' }
    if (-not (Test-Path $sentinel)) { throw 'Updater removed workspace data.' }
} finally {
    $parent.Refresh()
    if (-not $parent.HasExited) { Stop-Process -Id $parent.Id -Force }
    Remove-Item $sentinel -ErrorAction SilentlyContinue
}
Write-Host 'Updater checksum rejection, parent-process wait and previous-version upgrade verified.'
