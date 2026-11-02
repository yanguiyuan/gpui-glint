# Keep downloads out of the cache until both transfer and signature checks pass.
function Test-WebView2Bootstrapper {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $false }
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    return ($signature.Status -eq 'Valid' -and
        $null -ne $signature.SignerCertificate -and
        $signature.SignerCertificate.Subject -match 'Microsoft Corporation')
}

function Get-WebView2Bootstrapper {
    param([string]$Destination, [string]$SourcePath)
    if ($SourcePath) {
        if (-not (Test-WebView2Bootstrapper $SourcePath)) {
            throw "Invalid Microsoft WebView2 bootstrapper: $SourcePath"
        }
        $source = (Resolve-Path -LiteralPath $SourcePath).Path
        if ($source -ne [IO.Path]::GetFullPath($Destination)) {
            Copy-Item -LiteralPath $source -Destination $Destination -Force
        }
        return
    }
    if (Test-WebView2Bootstrapper $Destination) { return }

    $url = 'https://go.microsoft.com/fwlink/p/?LinkId=2124703'
    $temporary = "$Destination.$([guid]::NewGuid()).download"
    $curl = Get-Command curl.exe -ErrorAction SilentlyContinue
    $previousProtocol = [Net.ServicePointManager]::SecurityProtocol
    try {
        # Windows PowerShell 5.1 can otherwise negotiate obsolete TLS versions.
        [Net.ServicePointManager]::SecurityProtocol = $previousProtocol -bor [Net.SecurityProtocolType]::Tls12
        for ($attempt = 1; $attempt -le 3; $attempt++) {
            try {
                Write-Host "Downloading WebView2 bootstrapper (attempt $attempt/3)..."
                if ($curl -and $attempt -gt 1) {
                    # A separate HTTP stack also handles environments where .NET's request fails.
                    & $curl.Source --fail --location --silent --show-error --connect-timeout 20 --max-time 120 --output $temporary $url
                    if ($LASTEXITCODE -ne 0) { throw "curl.exe failed with exit code $LASTEXITCODE." }
                } else {
                    Invoke-WebRequest $url -UseBasicParsing -OutFile $temporary -TimeoutSec 120
                }
                if (-not (Test-WebView2Bootstrapper $temporary)) {
                    throw 'WebView2 bootstrapper signature validation failed.'
                }
                Move-Item -LiteralPath $temporary -Destination $Destination -Force
                return
            } catch {
                $reason = $_.Exception.Message
                if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
                if ($attempt -eq 3) {
                    throw "Unable to download a verified WebView2 bootstrapper: $reason Download the Evergreen Bootstrapper from https://developer.microsoft.com/microsoft-edge/webview2/ and pass -WebView2BootstrapperPath, or set GLINT_WEBVIEW2_BOOTSTRAPPER to its local path. Check network/proxy access to go.microsoft.com and Microsoft's download servers."
                }
                Write-Warning "WebView2 download failed: $reason Retrying..."
                Start-Sleep -Seconds $attempt
            }
        }
    } finally {
        [Net.ServicePointManager]::SecurityProtocol = $previousProtocol
        if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
    }
}
