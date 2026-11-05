function Get-Vcvars64Path {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    $vcvarsCandidates = @(
        if ($env:VSINSTALLDIR) { Join-Path $env:VSINSTALLDIR 'VC\Auxiliary\Build\vcvars64.bat' }
        if (Test-Path -LiteralPath $vswhere -PathType Leaf) {
            $vsRoot = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
            if ($LASTEXITCODE -eq 0 -and $vsRoot) { Join-Path $vsRoot 'VC\Auxiliary\Build\vcvars64.bat' }
        }
        # Some provisioned images have the toolchain on disk without an Installer registration.
        foreach ($programFiles in @($env:ProgramFiles, ${env:ProgramFiles(x86)})) {
            Resolve-Path "$programFiles\Microsoft Visual Studio\*\*\VC\Auxiliary\Build\vcvars64.bat" -ErrorAction SilentlyContinue |
                Sort-Object Path -Descending | Select-Object -ExpandProperty Path
        }
    )
    $vcvars = $vcvarsCandidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
    if (-not $vcvars) {
        throw 'MSVC x64 tools not found. Install Visual Studio Build Tools with Desktop development with C++ and a Windows SDK.'
    }
    return $vcvars
}
