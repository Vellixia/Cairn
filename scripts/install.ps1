# Cairn installer (Windows).
#
#   irm https://raw.githubusercontent.com/Vellixia/Cairn/main/scripts/install.ps1 | iex
#
# Cairn runs inside Docker. This script downloads docker-compose.yml and
# a template .env into the install directory and prints the next step.
#
# Honors: $env:CAIRN_REPO, $env:CAIRN_INSTALL_DIR.
$ErrorActionPreference = 'Stop'

$Repo       = if ($env:CAIRN_REPO)         { $env:CAIRN_REPO }         else { 'Vellixia/Cairn' }
$InstallDir = if ($env:CAIRN_INSTALL_DIR) { $env:CAIRN_INSTALL_DIR } else { "$env:LOCALAPPDATA\Cairn" }
$RawBase    = "https://raw.githubusercontent.com/$Repo/main"

function Write-Step($msg) { Write-Host "> $msg" -ForegroundColor Cyan }
function Fail($msg)       { Write-Host "X $msg" -ForegroundColor Red; exit 1 }

if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
    Fail "docker is required. Install Docker Desktop first."
}

Write-Step "Cairn install - Docker-only setup"
Write-Step "Target directory: $InstallDir"

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

try {
    Invoke-WebRequest -Uri "$RawBase/docker-compose.yml" -OutFile "$InstallDir\docker-compose.yml" -ErrorAction Stop
} catch {
    Fail "could not download docker-compose.yml from $RawBase"
}
Write-Step "wrote $InstallDir\docker-compose.yml"

try {
    Invoke-WebRequest -Uri "$RawBase/.env.example" -OutFile "$InstallDir\.env.example" -ErrorAction Stop
} catch {
    Fail "could not download .env.example from $RawBase"
}
Write-Step "wrote $InstallDir\.env.example"

$envFile = Join-Path $InstallDir '.env'
if (-not (Test-Path $envFile)) {
    Copy-Item "$InstallDir\.env.example" $envFile
    Write-Step "created $envFile from template"
    Write-Host ""
    Write-Host "Next steps:" -ForegroundColor Yellow
    Write-Host "  1. Edit $envFile and set:"
    Write-Host "       CAIRN_ADMIN_USERNAME=admin"
    Write-Host "       CAIRN_ADMIN_PASSWORD=<a strong password, 8+ chars>"
    Write-Host "       MINIO_ROOT_USER=<random>"
    Write-Host "       MINIO_ROOT_PASSWORD=<random>"
    Write-Host "  2. cd $InstallDir"
    Write-Host "  3. docker compose up -d"
    Write-Host "  4. Open http://127.0.0.1:7777 and log in"
} else {
    Write-Step "$envFile already exists - leaving it alone"
    Write-Host ""
    Write-Host "Next step: cd $InstallDir; docker compose up -d" -ForegroundColor Yellow
}
