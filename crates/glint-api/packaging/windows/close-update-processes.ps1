$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Normalize-ExecutablePath([string]$Path) {
    $normalized = [IO.Path]::GetFullPath($Path)
    if ($normalized.StartsWith('\\?\')) { $normalized = $normalized.Substring(4) }
    return $normalized
}

$target = Normalize-ExecutablePath $env:GLINT_API_UPDATE_EXECUTABLE
$blocked = @()
# A separate stdio proxy per MCP client is normal. Only terminate auxiliary
# modes of this exact installation; never kill applications by display name.
$processes = Get-CimInstance Win32_Process -Filter "Name = 'glint-api.exe'"
foreach ($entry in $processes) {
    if (-not $entry.ExecutablePath -or -not [string]::Equals(
        (Normalize-ExecutablePath $entry.ExecutablePath), $target,
        [StringComparison]::OrdinalIgnoreCase)) { continue }

    try { $process = [Diagnostics.Process]::GetProcessById($entry.ProcessId) }
    catch [ArgumentException] { continue } # Already exited.
    try {
        if (-not [string]::Equals((Normalize-ExecutablePath $process.MainModule.FileName),
            $target, [StringComparison]::OrdinalIgnoreCase)) { continue }
        if ($entry.CommandLine -cmatch '(?:^|\s)"?--(?:mcp|plugin-worker)"?(?:\s|$)') {
            $process.Kill()
            if (-not $process.WaitForExit(10000)) { throw "Auxiliary PID $($entry.ProcessId) did not exit." }
        } else {
            # Another GUI may use a different database. Preserve its unsaved work.
            $blocked += $entry.ProcessId
        }
    } finally { $process.Dispose() }
}
if ($blocked.Count -gt 0) {
    throw "Other workbench processes are still running (PID: $($blocked -join ', ')). Close them before retrying."
}
