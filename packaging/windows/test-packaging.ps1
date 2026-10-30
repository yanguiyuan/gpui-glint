# Safe smoke checks: inspect build plans and optionally compile fixture installers.
# Generated installers contain text fixtures, are never launched, and are not release artifacts.
param([string]$IsccPath)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = (Resolve-Path "$PSScriptRoot\..\..").Path
$api = & "$PSScriptRoot\build-windows.ps1" -Product api -Plan
$workbench = & "$PSScriptRoot\build-windows.ps1" -Product workbench -Plan
if ($api.Id -ne '7D39F166-C38C-45EA-B253-053B92032909' -or
    $workbench.Id -ne '7B7931DA-05B8-4E6B-9FD4-F33D82DD4B23') { throw 'Installer upgrade identities changed.' }
if ($api.Stage -eq $workbench.Stage -or $api.Output -eq $workbench.Output -or $api.ModelId -eq $workbench.ModelId) {
    throw 'Products must have independent staging, output and Windows identities.'
}
foreach ($plan in @($api, $workbench)) {
    if (-not (Test-Path -LiteralPath $plan.Icon)) { throw "Missing product icon: $($plan.Icon)" }
    if ($plan.Executable -ne "$($plan.Package).exe") { throw 'Wrong product executable.' }
    $rejected = $false
    try { & "$PSScriptRoot\build-windows.ps1" -Product $plan.Product -Version '999.0.0' -Plan | Out-Null } catch { $rejected = $true }
    if (-not $rejected) { throw 'Mismatched release versions must be rejected.' }
}
if ($IsccPath) {
    $fixtures = Join-Path $repo "target\windows-packaging-tests\$([guid]::NewGuid())"
    foreach ($plan in @($api, $workbench)) {
        # Spaces in fixture paths exercise compiler argument forwarding.
        $stage = Join-Path $fixtures "$($plan.Product) stage"
        $output = Join-Path $fixtures "$($plan.Product) output"
        New-Item -ItemType Directory -Force $stage, $output | Out-Null
        'Template test fixture, not an executable.' | Set-Content (Join-Path $stage $plan.Executable)
        'Template test fixture, not a bootstrapper.' | Set-Content (Join-Path $stage 'MicrosoftEdgeWebview2Setup.exe')
        Copy-Item -LiteralPath $plan.Icon -Destination (Join-Path $stage $plan.IconName)
        $defines = @($plan.Defines | Where-Object { $_ -notlike '/DStageDir=*' -and $_ -notlike '/DOutputDir=*' })
        $defines += "/DStageDir=$stage", "/DOutputDir=$output"
        & $IsccPath '/Q' @defines $plan.Template
        if ($LASTEXITCODE -ne 0) { throw "Template compilation failed: $($plan.Product)" }
        $installer = Join-Path $output ([IO.Path]::GetFileName($plan.Installer))
        if (-not (Test-Path -LiteralPath $installer)) { throw 'Missing fixture installer.' }
    }
    Write-Host 'Both installer templates compiled with fixture payloads (not release artifacts).'
}
Write-Host 'Product isolation, existing upgrade IDs and version validation passed.'
