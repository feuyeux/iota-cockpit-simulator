$ErrorActionPreference = "Stop"

$metadata = cargo metadata --format-version 1 | ConvertFrom-Json
$sympantosPackages = @(
  $metadata.packages | Where-Object {
    $_.source -like "*iota-sympantos*"
  }
)

if ($sympantosPackages.Count -ne 1 -or $sympantosPackages[0].name -ne "iota-core") {
  throw "Expected exactly one iota-sympantos package and it must be iota-core"
}

$agentRuntime = $metadata.packages | Where-Object { $_.name -eq "cockpit-agent-runtime" }
$agentDeps = @($agentRuntime.dependencies | Where-Object { $_.name -eq "iota-core" })
if ($agentDeps.Count -ne 1) {
  throw "cockpit-agent-runtime must directly depend on iota-core exactly once"
}

$forbidden = @("iota-cli", "iota-desktop", "iota-kanban")
foreach ($package in $sympantosPackages) {
  if ($forbidden -contains $package.name) {
    throw "Forbidden iota-sympantos package found: $($package.name)"
  }
}

Write-Output "Dependency audit passed: iota-core is the only iota-sympantos package."
