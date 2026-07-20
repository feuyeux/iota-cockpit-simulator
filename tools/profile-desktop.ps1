[CmdletBinding()]
param(
    [ValidateSet('cpu', 'memory', 'all')][string]$Type = 'all',
    [int]$ProcessId,
    [string]$ProcessPattern,
    [ValidateRange(1, 86400)][int]$Duration = 45,
    [string]$OutputDir = (Join-Path $PSScriptRoot '..\profile-results'),
    [switch]$NoUpdate
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Write-Log([string]$Message) { Write-Host "[profile] $Message" }
function Find-Winget {
    $command = Get-Command winget.exe -ErrorAction SilentlyContinue
    if (-not $command) { throw 'winget is required for automatic install/update. Install Microsoft App Installer, or use -NoUpdate after installing Windows Performance Toolkit.' }
}
function Ensure-Wpt {
    $wpr = Get-Command wpr.exe -ErrorAction SilentlyContinue
    if (-not $NoUpdate) {
        Find-Winget
        # The Windows ADK package supplies WPR/WPA. winget upgrade is idempotent when current.
        winget upgrade --id Microsoft.WindowsADK --exact --accept-package-agreements --accept-source-agreements --silent 2>$null
        if ($LASTEXITCODE -ne 0 -and -not $wpr) {
            winget install --id Microsoft.WindowsADK --exact --accept-package-agreements --accept-source-agreements --silent
        }
    }
    $script:Wpr = Get-Command wpr.exe -ErrorAction SilentlyContinue
    if (-not $script:Wpr) {
        $candidate = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\Windows Performance Toolkit\wpr.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($candidate) { $script:Wpr = $candidate }
    }
    if (-not $script:Wpr) { throw 'wpr.exe was not found. Install Windows Performance Toolkit from the Windows ADK.' }
}

if ($env:OS -ne 'Windows_NT') { throw 'Use tools/profile-desktop.sh on Linux or macOS.' }
New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
$OutputDir = (Resolve-Path $OutputDir).Path

if ($ProcessId) {
    $target = Get-Process -Id $ProcessId -ErrorAction Stop
} else {
    if ($ProcessPattern) {
        $targets = @(Get-CimInstance Win32_Process | Where-Object { $_.Name -match $ProcessPattern -or $_.CommandLine -match $ProcessPattern })
    } else {
        $targets = @(Get-CimInstance Win32_Process | Where-Object { $_.Name -match '^(cockpit-desktop|Cockpit Simulation)(\.exe)?$' })
    }
    if ($targets.Count -eq 0 -and -not $ProcessPattern) {
        $targets = @(Get-CimInstance Win32_Process | Where-Object { $_.Name -match '^cockpit-simulator(\.exe)?$' })
    }
    if ($targets.Count -eq 0) { throw 'Cockpit Desktop is not running. Start it first, then retry.' }
    $selected = $targets | Sort-Object CreationDate, ProcessId | Select-Object -Last 1
    $ProcessId = [int]$selected.ProcessId
    $target = Get-Process -Id $ProcessId
    Write-Log "Automatically selected PID=$ProcessId ($($target.ProcessName))."
}

if ($Type -eq 'all') {
    Write-Log "Collecting CPU first, then memory, for Desktop PID $ProcessId."
    $common = @{ ProcessId = $ProcessId; Duration = $Duration; OutputDir = $OutputDir; NoUpdate = $NoUpdate }
    & $PSCommandPath -Type cpu @common
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & $PSCommandPath -Type memory @common
    exit $LASTEXITCODE
}

Ensure-Wpt
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$etl = Join-Path $OutputDir "$Type-$ProcessId-$stamp.etl"
Write-Log "Target PID=$ProcessId ($($target.ProcessName)); type=$Type; duration=${Duration}s"

try {
    if ($Type -eq 'cpu') {
        & $Wpr -start CPU -filemode
    } else {
        # Heap tracing requires an image name and captures allocation stacks for new allocations.
        & $Wpr -start Heap -filemode -Pid $ProcessId
    }
    if ($LASTEXITCODE -ne 0) { throw 'WPR failed to start. Run this terminal as Administrator and ensure no other WPR session is active.' }
    Write-Log 'Reproduce the workload now.'
    Start-Sleep -Seconds $Duration
    & $Wpr -stop $etl
    if ($LASTEXITCODE -ne 0) { throw 'WPR failed to stop or save the trace.' }
} catch {
    & $Wpr -cancel 2>$null
    throw
}

Write-Log "Trace: $etl"
Write-Log 'Open it in WPA; filter by the target PID and select CPU Usage or Heap Allocations, then use the Flame Graph view.'
