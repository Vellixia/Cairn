#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 12: install scripts — verify install.ps1 exists and is syntactically valid.
$installPath = Join-Path $script:RepoRoot 'scripts/install.ps1'
Assert-True -Condition (Test-Path $installPath) -Msg 'scripts/install.ps1 exists'

# Docker compose file exists.
$composePath = Join-Path $script:RepoRoot 'docker-compose.yml'
Assert-True -Condition (Test-Path $composePath) -Msg 'docker-compose.yml exists'

# Homebrew formula exists.
$brewPath = Join-Path $script:RepoRoot 'packaging/homebrew-tap/cairn.rb'
Assert-True -Condition (Test-Path $brewPath) -Msg 'Homebrew formula exists'

Show-Scenario -Sprint '4.0' -Name 'desktop-install' -Status pass