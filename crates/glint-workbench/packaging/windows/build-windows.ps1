param(
    [string]$Version,
    [string]$IsccPath
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { throw 'Build this installer on Windows.' }
$repo = (Resolve-Path "$PSScriptRoot\..\..\..\..").Path
$manifest = Get-Content "$repo\crates\glint-workbench\Cargo.toml" -Raw
$appVersion = [regex]::Match($manifest, '(?m)^version = "([^"]+)"').Groups[1].Value
if (-not $Version) { $Version = $appVersion }
if ($Version -ne $appVersion) { throw "Release version $Version does not match app version $appVersion." }
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw 'Use a stable major.minor.patch version.' }
if (-not $IsccPath) {
    $compiler = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    $candidates = @(
        if ($compiler) { $compiler.Source }
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
    )
    $IsccPath = $candidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
}
if (-not $IsccPath -or -not (Test-Path -LiteralPath $IsccPath -PathType Leaf)) {
    throw 'Inno Setup compiler not found. Install Inno Setup 6 or pass -IsccPath.'
}
$stage = "$repo\target\windows-workbench-installer\stage"
$output = "$repo\target\windows-workbench-installer\output"
New-Item -ItemType Directory -Force $stage, $output | Out-Null
Push-Location $repo
$previousFlags = $env:RUSTFLAGS
try {
    # Embed the MSVC runtime so users do not need a separate VC++ installer.
    $env:RUSTFLAGS = "$previousFlags -C target-feature=+crt-static".Trim()
    cargo build -p glint-workbench --release --target x86_64-pc-windows-msvc --locked
    if ($LASTEXITCODE -ne 0) { throw 'Windows application build failed.' }
    Copy-Item "$repo\target\x86_64-pc-windows-msvc\release\glint-workbench.exe" $stage
    Copy-Item "$repo\crates\glint-api\assets\glint-api-icon.ico" "$stage\glint-workbench-icon.ico"
    $bootstrapper = "$stage\MicrosoftEdgeWebview2Setup.exe"
    if (-not (Test-Path -LiteralPath $bootstrapper)) {
        Invoke-WebRequest 'https://go.microsoft.com/fwlink/p/?LinkId=2124703' -UseBasicParsing -OutFile $bootstrapper -TimeoutSec 120
    }
    $signature = Get-AuthenticodeSignature $bootstrapper
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'Microsoft Corporation') {
        throw 'WebView2 bootstrapper signature validation failed.'
    }
    & $IsccPath "/DAppVersion=$Version" "/DStageDir=$stage" "/DOutputDir=$output" "$PSScriptRoot\setup.iss"
    if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed.' }
    $installer = "$output\Glint-Workbench-$Version-windows-x64-setup.exe"
    if (-not (Test-Path $installer)) { throw 'Installer output is missing.' }
    $hash = (Get-FileHash $installer -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $([IO.Path]::GetFileName($installer))" | Set-Content "$installer.sha256" -Encoding ascii
    Write-Host "Installer: $installer"
} finally {
    $env:RUSTFLAGS = $previousFlags
    Pop-Location
}
