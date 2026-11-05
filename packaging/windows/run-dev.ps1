$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

. "$PSScriptRoot\msvc.ps1"
$vcvars = Get-Vcvars64Path
& "$env:SystemRoot\System32\cmd.exe" /d /c "call `"$vcvars`" && cargo run -p glint-workbench"
exit $LASTEXITCODE
