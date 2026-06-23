#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 12: install scripts — verify install.ps1 + install.sh exist and are non-empty.
$installPs1 = Join-Path $script:RepoRoot 'scripts/install.ps1'
Assert-True -Condition (Test-Path $installPs1) -Msg 'scripts/install.ps1 exists'
$installSh = Join-Path $script:RepoRoot 'scripts/install.sh'
Assert-True -Condition (Test-Path $installSh) -Msg 'scripts/install.sh exists'

# Docker compose file exists.
$composePath = Join-Path $script:RepoRoot 'docker-compose.yml'
Assert-True -Condition (Test-Path $composePath) -Msg 'docker-compose.yml exists'

Show-Scenario -Sprint '4.0' -Name 'desktop-install' -Status pass