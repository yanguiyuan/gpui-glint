# Offline regression checks; fake payloads/signatures never enter release staging.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. "$PSScriptRoot\webview2.ps1"
$root = Join-Path ([IO.Path]::GetTempPath()) "glint-webview2-tests-$([guid]::NewGuid())"
New-Item -ItemType Directory $root | Out-Null
$destination = Join-Path $root 'bootstrapper.exe'
$source = Join-Path $root 'local.exe'
$script:calls = 0
$script:fail = $false
$script:useCurl = $false
$protocol = [Net.ServicePointManager]::SecurityProtocol

function Get-AuthenticodeSignature {
    param([string]$LiteralPath)
    $valid = (Get-Content -LiteralPath $LiteralPath -Raw).Trim() -eq 'signed'
    [pscustomobject]@{
        Status = if ($valid) { 'Valid' } else { 'NotSigned' }
        SignerCertificate = if ($valid) { [pscustomobject]@{ Subject = 'CN=Microsoft Corporation' } } else { $null }
    }
}
function Get-Command {
    param([string]$Name, $ErrorAction)
    if ($script:useCurl) { [pscustomobject]@{ Source = (Join-Path $root 'curl.ps1') } }
}
function Invoke-WebRequest {
    param($Uri, [switch]$UseBasicParsing, [string]$OutFile, [int]$TimeoutSec)
    $script:calls++
    'partial' | Set-Content -LiteralPath $OutFile
    if ($script:fail -or $script:useCurl -or $script:calls -eq 1) { throw 'Simulated connection reset' }
    'signed' | Set-Content -LiteralPath $OutFile
}
function Start-Sleep { param([int]$Seconds) }
function Assert($Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
try {
    # Recover from a poisoned cache and an interrupted first transfer.
    'partial' | Set-Content -LiteralPath $destination
    Get-WebView2Bootstrapper $destination
    Assert ($script:calls -eq 2) 'A failed download must retry.'
    Assert (Test-WebView2Bootstrapper $destination) 'Cache must contain the verified payload.'
    Get-WebView2Bootstrapper $destination
    Assert ($script:calls -eq 2) 'A valid cache must avoid network requests.'

    'signed' | Set-Content -LiteralPath $source
    Get-WebView2Bootstrapper $destination $source
    Get-WebView2Bootstrapper $destination $destination
    Assert ($script:calls -eq 2) 'Local sources must avoid network requests.'
    'partial' | Set-Content -LiteralPath $source
    $rejected = $false
    try { Get-WebView2Bootstrapper $destination $source } catch { $rejected = $true }
    Assert $rejected 'Unsigned local files must be rejected.'
    Assert (Test-WebView2Bootstrapper $destination) 'Invalid input must preserve the valid cache.'

    Remove-Item -LiteralPath $destination
    $script:fail = $true
    $rejected = $false
    try { Get-WebView2Bootstrapper $destination } catch {
        $rejected = $_.Exception.Message -like '*GLINT_WEBVIEW2_BOOTSTRAPPER*'
    }
    Assert $rejected 'Exhausted retries must explain the local-file override.'
    Assert ($script:calls -eq 5) 'Failures must stop after three attempts.'
    Assert (-not (Test-Path -LiteralPath $destination)) 'Failed transfers must not populate the cache.'
    Assert (@(Get-ChildItem -LiteralPath $root -Filter '*.download').Count -eq 0) 'Partial files must be removed.'

    @'
param([Parameter(ValueFromRemainingArguments)]$Arguments)
'called' | Set-Content -LiteralPath (Join-Path $PSScriptRoot 'curl-called')
$outputIndex = [Array]::IndexOf($Arguments, '--output') + 1
'signed' | Set-Content -LiteralPath $Arguments[$outputIndex]
$global:LASTEXITCODE = 0
'@ | Set-Content -LiteralPath (Join-Path $root 'curl.ps1')
    $script:useCurl = $true
    Get-WebView2Bootstrapper $destination
    Assert (Test-Path -LiteralPath (Join-Path $root 'curl-called')) 'curl must recover from a failed .NET request.'
    Assert (Test-WebView2Bootstrapper $destination) 'curl output must pass signature validation.'
    Assert ([Net.ServicePointManager]::SecurityProtocol -eq $protocol) 'TLS settings must be restored.'
    Write-Host 'WebView2 cache, retry, cleanup, local source and curl fallback checks passed.'
} finally {
    # Only remove files created by this test, within its unique temporary directory.
    Get-ChildItem -LiteralPath $root -File | Remove-Item -Force
    Remove-Item -LiteralPath $root
}
