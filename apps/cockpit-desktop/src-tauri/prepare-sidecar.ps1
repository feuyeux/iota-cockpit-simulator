# Build the cockpit-simulator and stage it as a Tauri sidecar binary.
#
# Tauri resolves `externalBin` entries by appending the host target triple, so
# the simulator is copied to `binaries/cockpit-simulator-<triple><ext>`. Run this
# before `npm run tauri:build` (or `tauri:dev`) to package the simulator alongside
# the desktop app.

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$WorkspaceRoot = Resolve-Path "$ScriptDir\..\..\.."
$BinDir = "$ScriptDir\binaries"

# Get the host target triple
$TripleLine = rustc -vV | Select-String "host:"
if (-not $TripleLine) {
    Write-Error "could not determine host target triple"
    exit 1
}
$Triple = $TripleLine.ToString() -replace "host:\s*", ""

$Ext = if ($Triple -match "windows") { ".exe" } else { "" }

New-Item -ItemType Directory -Path $BinDir -Force | Out-Null
foreach ($Name in @("cockpit-simulator", "cockpit-evaluator")) {
    $Dst = "$BinDir\$Name-$Triple$Ext"
    $Tmp = "$Dst.tmp"
    Remove-Item $Dst, $Tmp -Force -ErrorAction SilentlyContinue
}

Write-Host "Building cockpit-simulator and cockpit-evaluator (release) for $Triple"
$BuildExitCode = 1
Push-Location $WorkspaceRoot
try {
    & cargo build --release -p cockpit-simulator -p cockpit-evaluator --features cockpit-simulator/live-acp
    $BuildExitCode = $LASTEXITCODE
} finally {
    Pop-Location
}
if ($BuildExitCode -ne 0) {
    Write-Error "sidecar cargo build failed with exit code $BuildExitCode; no stale binary was staged"
    exit $BuildExitCode
}

foreach ($Name in @("cockpit-simulator", "cockpit-evaluator")) {
    $Src = "$WorkspaceRoot\target\release\$Name$Ext"
    $Dst = "$BinDir\$Name-$Triple$Ext"
    $Tmp = "$Dst.tmp"
    if (-not (Test-Path -LiteralPath $Src -PathType Leaf)) {
        Write-Error "sidecar build succeeded but expected artifact is missing: $Src"
        exit 1
    }
    Copy-Item -LiteralPath $Src -Destination $Tmp -Force
    Move-Item -LiteralPath $Tmp -Destination $Dst -Force
    Write-Host "Staged sidecar: $Dst"
}
