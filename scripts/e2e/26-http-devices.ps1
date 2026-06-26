#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 5.0+: Devices & Health HTTP API — pair flow, deep health.

# --- GET /api/health/deep ---
$deepResp = Test-Endpoint -Method GET -Path '/api/health/deep'
Assert-True -Condition ($deepResp.StatusCode -in 200,503) -Msg "/api/health/deep returns 200 or 503 (got $($deepResp.StatusCode))"
if ($deepResp.StatusCode -eq 200) {
    $deepBody = $deepResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
    Assert-Eq -Expected 'ok' -Actual $deepBody.status -Msg 'health/deep status == ok when 200'
}

# --- POST /api/pair/new — create pairing code ---
$pairNewResp = Test-Endpoint -Method POST -Path '/api/pair/new' -Body @{ name = 'e2e-test-device' }
Assert-Eq -Expected 200 -Actual $pairNewResp.StatusCode -Msg 'POST /api/pair/new returns 200'
$pairBody = $pairNewResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
$pairCode = $pairBody.code
Assert-True -Condition ($pairCode.Length -eq 8) -Msg 'pairing code is 8 characters'
Assert-True -Condition ($null -ne $pairBody.token) -Msg 'pairing returns a JWT token'

# --- POST /api/pair/claim — claim the code ---
$claimResp = Test-Endpoint -Method POST -Path '/api/pair/claim' -Body @{ code = $pairCode }
Assert-Eq -Expected 200 -Actual $claimResp.StatusCode -Msg 'POST /api/pair/claim returns 200'
$claimBody = $claimResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-Eq -Expected 'e2e-test-device' -Actual $claimBody.name -Msg 'claim returns the device name'
Assert-True -Condition ($null -ne $claimBody.token) -Msg 'claim returns a JWT token'

# --- Claiming the same code again must fail (single-use) ---
$claimFailResp = Test-Endpoint -Method POST -Path '/api/pair/claim' -Body @{ code = $pairCode }
Assert-Eq -Expected 404 -Actual $claimFailResp.StatusCode -Msg 're-claiming same code returns 404'

# --- GET /api/auth/status ---
$statusResp = Test-Endpoint -Method GET -Path '/api/auth/status'
Assert-Eq -Expected 200 -Actual $statusResp.StatusCode -Msg 'GET /api/auth/status returns 200'
$statusBody = $statusResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-True -Condition ($statusBody -match 'needs_setup|ok|ready') -Msg 'auth status returns recognized state'

Show-Scenario -Sprint '5.0' -Name 'http-devices' -Status pass
