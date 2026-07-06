#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 5.0+: Devices & Health HTTP API — deep health, auth status.

# --- GET /api/health/deep ---
$deepResp = Test-Endpoint -Method GET -Path '/api/health/deep'
Assert-True -Condition ($deepResp.StatusCode -in 200,503) -Msg "/api/health/deep returns 200 or 503 (got $($deepResp.StatusCode))"
if ($deepResp.StatusCode -eq 200) {
    $deepBody = $deepResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
    Assert-Eq -Expected 'ok' -Actual $deepBody.status -Msg 'health/deep status == ok when 200'
}

# --- GET /api/auth/status ---
$statusResp = Test-Endpoint -Method GET -Path '/api/auth/status'
Assert-Eq -Expected 200 -Actual $statusResp.StatusCode -Msg 'GET /api/auth/status returns 200'
$statusBody = $statusResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-True -Condition ($statusBody -match 'needs_setup|ok|ready') -Msg 'auth status returns recognized state'

Show-Scenario -Sprint '5.0' -Name 'http-devices' -Status pass
