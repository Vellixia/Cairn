#Requires -Version 5.1
<#
.SYNOPSIS
    Cairn end-to-end smoke harness — runs every Phase 3.5–5 scenario against
    the live `cairn:dev` container on http://127.0.0.1:7777.

.DESCRIPTION
    Exit 0 if every scenario passed; 1 otherwise. The scenarios live in
    scripts/e2e/*.ps1 and are run in numeric order. Use
    `CAIRN_E2E_FAILFAST=0` to keep going after a failure (default: stop on
    first fail).

.EXAMPLE
    ./scripts/e2e.ps1
    ./scripts/e2e.ps1 03-mcp-tools
    CAIRN_E2E_FAILFAST=0 ./scripts/e2e.ps1
#>
[CmdletBinding()]
param(
    [string] $Only = ''   # if non-empty, run only scenarios whose file name starts with this
)

$ErrorActionPreference = 'Stop'
$lib = Join-Path $PSScriptRoot 'e2e/_lib.ps1'
. $lib

Write-Host "Cairn e2e harness (lib=$lib, fail-fast=$Global:E2E_FailFast, base=$Global:E2E_BaseUrl)" -ForegroundColor Cyan
Write-Host "Binaries:" -ForegroundColor DarkGray
Write-Host "  cairn    = $Global:E2E_BinCairn"   -ForegroundColor DarkGray
Write-Host "Data dir : $Global:E2E_DataDir" -ForegroundColor DarkGray
Write-Host "Log file : $Global:E2E_LogFile" -ForegroundColor DarkGray
Write-Host ""

# Quick preflight — bail if the live cairn isn't reachable.
$preflight = Test-Endpoint -Method GET -Path '/api/health'
if ($preflight.StatusCode -ne 200) {
    Write-Host "FATAL: cairn-server at $Global:E2E_BaseUrl returned $($preflight.StatusCode); is `docker compose up -d cairn` running?" -ForegroundColor Red
    exit 2
}
Write-Host "Preflight OK: $($preflight.Body.Trim())" -ForegroundColor Green

# Optional bootstrap: create an admin so /api/auth/login works.
# The cairn container starts with no admin → /setup is open. We mint
# a session token via the admin/setup endpoint on first run only.
$env:CAIRN_ADMIN_PASSWORD = 'cairn-e2e-admin-password-12345'
$setupProbe = Test-Endpoint -Method GET -Path '/api/setup/health'
$isFresh = ($setupProbe.Body -match '"needs_setup"\s*:\s*true')
if ($isFresh) {
    Write-Host "Fresh install -- running first-time setup to mint an admin..." -ForegroundColor Yellow
    $setupBody = @{
        username = 'admin'
        password = $env:CAIRN_ADMIN_PASSWORD
    } | ConvertTo-Json -Compress
    $tmp = New-TemporaryFile
    try {
        $code = & curl -sS -o $tmp.FullName -w '%{http_code}' `
            -H 'content-type: application/json' -d $setupBody `
            "$Global:E2E_BaseUrl/api/auth/setup"
        Write-Host "  setup HTTP $code"
        $resp = Get-Content $tmp.FullName -Raw
        Write-Host "  setup response: $resp"
    } finally {
        Remove-Item $tmp -ErrorAction SilentlyContinue
    }
}

# Mint a session cookie for HTTP scenarios. If admin doesn't yet exist the
# password above, /api/auth/setup mints it; /api/auth/login then works.
try {
    $cookie = Get-BearerToken -Username 'admin' -Password $env:CAIRN_ADMIN_PASSWORD
    $len = $cookie.Length
    Write-Host ('Session cookie acquired (' + $len + ' chars)') -ForegroundColor Green
} catch {
    Write-Host ('WARN: failed to acquire session cookie (some scenarios will skip auth-gated paths): ' + $_) -ForegroundColor Yellow
    $Global:E2E_Cookie = $null
}

# Locate and run scenario scripts.
$scenariosDir = Join-Path $PSScriptRoot 'e2e'
$scripts = Get-ChildItem -Path $scenariosDir -Filter '*.ps1' -File `
    | Where-Object { $_.Name -match '^\d{2}-' } `
    | Sort-Object Name

if ($Only) {
    $scripts = $scripts | Where-Object { $_.BaseName -like ($Only + '*') }
    if (-not $scripts) {
        Write-Host ('No scenarios match ''' + $Only + '''') -ForegroundColor Red
        exit 2
    }
}

Write-Host ""
Write-Host ('Running ' + $scripts.Count + ' scenario(s)...') -ForegroundColor Cyan
Write-Host ""

foreach ($s in $scripts) {
    Write-Host ('--- ' + $s.BaseName + ' ---') -ForegroundColor Magenta
    try {
        & $s.FullName
        Write-Host ""
    } catch {
        Write-Host ('EXCEPTION in ' + $s.BaseName + ': ' + $_) -ForegroundColor Red
        if ($Global:E2E_FailFast) {
            Show-Summary
            Stop-Mcp
            exit 1
        }
    }
}

Exit-Summary
